//! WebSocket client management
//!
//! Handles WebSocket connections with automatic reconnection
//! for real-time command streaming.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, warn};

use crate::config::WebSocketConfig;

/// WebSocket connection manager
#[allow(dead_code)]
pub struct WebSocketManager {
    config: WebSocketConfig,
    running: Arc<AtomicBool>,
    sender: Arc<Mutex<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>>,
    connected: Arc<AtomicBool>,
}

impl WebSocketManager {
    pub fn new(config: WebSocketConfig, running: Arc<AtomicBool>) -> Self {
        Self {
            config,
            running,
            sender: Arc::new(Mutex::new(None)),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Get a clone of the sender for external use
    pub fn get_sender(
        &self,
    ) -> Arc<Mutex<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>> {
        Arc::clone(&self.sender)
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Start the WebSocket connection with automatic reconnection
    pub async fn run(&self, message_handler: mpsc::Sender<String>) -> Result<()> {
        let mut reconnect_attempts = 0;

        while self.running.load(Ordering::SeqCst) {
            match self.connect().await {
                Ok((ws_sender, ws_receiver)) => {
                    info!("WebSocket connected to {}", self.config.url);
                    reconnect_attempts = 0;
                    self.connected.store(true, Ordering::SeqCst);

                    // Store sender
                    {
                        let mut sender = self.sender.lock().await;
                        *sender = Some(ws_sender);
                    }

                    // Handle incoming messages until disconnect
                    self.handle_messages(ws_receiver, message_handler.clone())
                        .await;

                    // Clear sender on disconnect
                    {
                        let mut sender = self.sender.lock().await;
                        *sender = None;
                    }
                    self.connected.store(false, Ordering::SeqCst);

                    if !self.config.reconnect {
                        info!("WebSocket disconnected, reconnect disabled");
                        break;
                    }

                    warn!("WebSocket disconnected, will reconnect...");
                }
                Err(e) => {
                    error!("WebSocket connection failed: {}", e);
                    reconnect_attempts += 1;

                    if self.config.max_reconnect_attempts > 0
                        && reconnect_attempts >= self.config.max_reconnect_attempts
                    {
                        error!(
                            "Max reconnect attempts ({}) reached",
                            self.config.max_reconnect_attempts
                        );
                        break;
                    }
                }
            }

            if !self.running.load(Ordering::SeqCst) {
                break;
            }

            // Wait before reconnecting
            debug!(
                "Waiting {}ms before reconnect...",
                self.config.reconnect_delay_ms
            );
            sleep(Duration::from_millis(self.config.reconnect_delay_ms)).await;
        }

        Ok(())
    }

    async fn connect(
        &self,
    ) -> Result<(
        SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
        SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    )> {
        debug!("Connecting to WebSocket: {}", self.config.url);

        let (ws_stream, _) = connect_async(&self.config.url)
            .await
            .context("Failed to connect to WebSocket")?;

        let (sender, receiver) = ws_stream.split();
        Ok((sender, receiver))
    }

    async fn handle_messages(
        &self,
        mut receiver: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        message_handler: mpsc::Sender<String>,
    ) {
        while self.running.load(Ordering::SeqCst) {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            debug!("Received WebSocket message: {}", text);
                            message_handler.send(text.to_string()).await.ok();
                        }
                        Some(Ok(Message::Binary(data))) => {
                            if let Ok(text) = String::from_utf8(data.to_vec()) {
                                debug!("Received WebSocket binary message: {}", text);
                                message_handler.send(text).await.ok();
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            debug!("Received ping, sending pong");
                            if let Some(sender) = &mut *self.sender.lock().await {
                                sender.send(Message::Pong(data)).await.ok();
                            }
                        }
                        Some(Ok(Message::Pong(_))) => {
                            debug!("Received pong");
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("WebSocket closed by server");
                            break;
                        }
                        Some(Ok(Message::Frame(_))) => {}
                        Some(Err(e)) => {
                            error!("WebSocket error: {}", e);
                            break;
                        }
                        None => {
                            info!("WebSocket stream ended");
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    // Send ping to keep connection alive
                    if let Some(sender) = &mut *self.sender.lock().await {
                        if sender.send(Message::Ping(vec![].into())).await.is_err() {
                            warn!("Failed to send ping");
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Send a message through the WebSocket
    pub async fn send(&self, message: Message) -> Result<()> {
        let mut sender_lock = self.sender.lock().await;
        if let Some(sender) = sender_lock.as_mut() {
            sender
                .send(message)
                .await
                .context("Failed to send WebSocket message")?;
        } else {
            warn!("WebSocket not connected, cannot send message");
        }
        Ok(())
    }

    /// Send a text message
    pub async fn send_text(&self, text: String) -> Result<()> {
        self.send(Message::Text(text.into())).await
    }

    /// Send a binary message
    pub async fn send_binary(&self, data: Vec<u8>) -> Result<()> {
        self.send(Message::Binary(data.into())).await
    }
}
