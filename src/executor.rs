//! Action executor
//!
//! Handles execution of shell commands, HTTP requests, and WebSocket messages
//! based on controller input events.

use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use handlebars::Handlebars;
use reqwest::Client as HttpClient;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, trace, warn};

use crate::config::{
    ActionConfig, Config, HttpRequest, RumbleConfig,
    TemplateContext, WebSocketMessage,
};
use crate::dualsense::ControllerState;

/// Event types for action triggering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    Press,
    Release,
    Hold,
    Change,
}

impl EventType {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "release" => EventType::Release,
            "hold" => EventType::Hold,
            "change" => EventType::Change,
            _ => EventType::Press,
        }
    }
}

/// Debounce tracker
struct DebounceState {
    last_trigger: HashMap<String, Instant>,
}

impl DebounceState {
    fn new() -> Self {
        Self {
            last_trigger: HashMap::new(),
        }
    }

    fn can_trigger(&mut self, key: &str, debounce_ms: u64) -> bool {
        if debounce_ms == 0 {
            return true;
        }

        let now = Instant::now();
        if let Some(last) = self.last_trigger.get(key) {
            if now.duration_since(*last) < Duration::from_millis(debounce_ms) {
                return false;
            }
        }
        self.last_trigger.insert(key.to_string(), now);
        true
    }
}

/// Commands to send to the controller
pub enum ControllerCommand {
    SetLed(u8, u8, u8),
    SetRumble(u8, u8, u64), // left, right, duration_ms
}

/// Action executor
pub struct Executor {
    config: Config,
    handlebars: Handlebars<'static>,
    http_client: Option<HttpClient>,
    debounce: DebounceState,
    ws_sender: Option<Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>>,
    controller_cmd_tx: mpsc::Sender<ControllerCommand>,
}

impl Executor {
    pub fn new(config: Config, controller_cmd_tx: mpsc::Sender<ControllerCommand>) -> Self {
        let http_client = config.http.as_ref().map(|_| {
            HttpClient::builder()
                .timeout(Duration::from_millis(
                    config.http.as_ref().map(|h| h.timeout_ms).unwrap_or(5000),
                ))
                .build()
                .expect("Failed to create HTTP client")
        });

        Self {
            config,
            handlebars: Handlebars::new(),
            http_client,
            debounce: DebounceState::new(),
            ws_sender: None,
            controller_cmd_tx,
        }
    }

    /// Set the WebSocket sender
    pub fn set_ws_sender(
        &mut self,
        sender: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    ) {
        self.ws_sender = Some(sender);
    }

    /// Process a state change and execute matching actions
    pub async fn process_state_change(
        &mut self,
        prev: &ControllerState,
        current: &ControllerState,
    ) -> Result<()> {
        let ctx = TemplateContext::from(current);

        // Check button changes
        self.check_button_action("cross", prev.buttons.cross, current.buttons.cross, &ctx)
            .await?;
        self.check_button_action("circle", prev.buttons.circle, current.buttons.circle, &ctx)
            .await?;
        self.check_button_action("square", prev.buttons.square, current.buttons.square, &ctx)
            .await?;
        self.check_button_action(
            "triangle",
            prev.buttons.triangle,
            current.buttons.triangle,
            &ctx,
        )
        .await?;

        self.check_button_action("l1", prev.buttons.l1, current.buttons.l1, &ctx)
            .await?;
        self.check_button_action("r1", prev.buttons.r1, current.buttons.r1, &ctx)
            .await?;
        self.check_button_action(
            "l2_button",
            prev.buttons.l2_button,
            current.buttons.l2_button,
            &ctx,
        )
        .await?;
        self.check_button_action(
            "r2_button",
            prev.buttons.r2_button,
            current.buttons.r2_button,
            &ctx,
        )
        .await?;

        self.check_button_action("dpad_up", prev.buttons.dpad_up, current.buttons.dpad_up, &ctx)
            .await?;
        self.check_button_action(
            "dpad_down",
            prev.buttons.dpad_down,
            current.buttons.dpad_down,
            &ctx,
        )
        .await?;
        self.check_button_action(
            "dpad_left",
            prev.buttons.dpad_left,
            current.buttons.dpad_left,
            &ctx,
        )
        .await?;
        self.check_button_action(
            "dpad_right",
            prev.buttons.dpad_right,
            current.buttons.dpad_right,
            &ctx,
        )
        .await?;

        self.check_button_action("l3", prev.buttons.l3, current.buttons.l3, &ctx)
            .await?;
        self.check_button_action("r3", prev.buttons.r3, current.buttons.r3, &ctx)
            .await?;

        self.check_button_action("options", prev.buttons.options, current.buttons.options, &ctx)
            .await?;
        self.check_button_action("create", prev.buttons.create, current.buttons.create, &ctx)
            .await?;
        self.check_button_action("ps", prev.buttons.ps, current.buttons.ps, &ctx)
            .await?;
        self.check_button_action(
            "touchpad",
            prev.buttons.touchpad,
            current.buttons.touchpad,
            &ctx,
        )
        .await?;
        self.check_button_action("mute", prev.buttons.mute, current.buttons.mute, &ctx)
            .await?;

        // Check analog inputs
        self.check_stick_actions(prev, current, &ctx).await?;
        self.check_trigger_actions(prev, current, &ctx).await?;

        Ok(())
    }

    async fn check_button_action(
        &mut self,
        name: &str,
        prev: bool,
        current: bool,
        ctx: &TemplateContext,
    ) -> Result<()> {
        // Clone the action config to avoid borrow conflicts
        let action_opt: Option<ActionConfig> = match name {
            "cross" => self.config.buttons.cross.clone(),
            "circle" => self.config.buttons.circle.clone(),
            "square" => self.config.buttons.square.clone(),
            "triangle" => self.config.buttons.triangle.clone(),
            "l1" => self.config.buttons.l1.clone(),
            "r1" => self.config.buttons.r1.clone(),
            "l2_button" => self.config.buttons.l2_button.clone(),
            "r2_button" => self.config.buttons.r2_button.clone(),
            "dpad_up" => self.config.buttons.dpad_up.clone(),
            "dpad_down" => self.config.buttons.dpad_down.clone(),
            "dpad_left" => self.config.buttons.dpad_left.clone(),
            "dpad_right" => self.config.buttons.dpad_right.clone(),
            "l3" => self.config.buttons.l3.clone(),
            "r3" => self.config.buttons.r3.clone(),
            "options" => self.config.buttons.options.clone(),
            "create" => self.config.buttons.create.clone(),
            "ps" => self.config.buttons.ps.clone(),
            "touchpad" => self.config.buttons.touchpad.clone(),
            "mute" => self.config.buttons.mute.clone(),
            _ => return Ok(()),
        };

        if let Some(action) = action_opt {
            let event_type = EventType::from_str(&action.trigger);
            let should_trigger = match event_type {
                EventType::Press => !prev && current,
                EventType::Release => prev && !current,
                EventType::Hold => current,
                EventType::Change => prev != current,
            };

            if should_trigger && self.debounce.can_trigger(name, action.debounce_ms) {
                debug!("Triggering action for button: {}", name);
                self.execute_action(&action, ctx).await?;
            }
        }

        Ok(())
    }

    async fn check_stick_actions(
        &mut self,
        prev: &ControllerState,
        current: &ControllerState,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let deadzone = self.config.deadzone;
        let mut actions_to_execute: Vec<ActionConfig> = Vec::new();

        // Left stick - collect actions
        if let Some(mapping) = &self.config.analog.left_stick {
            let (px, py) = prev.left_stick.normalized_with_deadzone(deadzone);
            let (cx, cy) = current.left_stick.normalized_with_deadzone(deadzone);
            let threshold = mapping.threshold;
            let rate_limit = mapping.rate_limit_ms;

            // Directional triggers
            if let Some(action) = &mapping.on_right {
                if px <= threshold && cx > threshold {
                    actions_to_execute.push(action.clone());
                }
            }
            if let Some(action) = &mapping.on_left {
                if px >= -threshold && cx < -threshold {
                    actions_to_execute.push(action.clone());
                }
            }
            if let Some(action) = &mapping.on_up {
                if py >= -threshold && cy < -threshold {
                    // Y is inverted
                    actions_to_execute.push(action.clone());
                }
            }
            if let Some(action) = &mapping.on_down {
                if py <= threshold && cy > threshold {
                    actions_to_execute.push(action.clone());
                }
            }

            // Continuous movement - check debounce separately
            if let Some(action) = &mapping.on_move {
                if (cx != 0.0 || cy != 0.0)
                    && self.debounce.can_trigger("left_stick_move", rate_limit)
                {
                    actions_to_execute.push(action.clone());
                }
            }
        }

        // Right stick - collect actions
        if let Some(mapping) = &self.config.analog.right_stick {
            let (px, py) = prev.right_stick.normalized_with_deadzone(deadzone);
            let (cx, cy) = current.right_stick.normalized_with_deadzone(deadzone);
            let threshold = mapping.threshold;
            let rate_limit = mapping.rate_limit_ms;

            if let Some(action) = &mapping.on_right {
                if px <= threshold && cx > threshold {
                    actions_to_execute.push(action.clone());
                }
            }
            if let Some(action) = &mapping.on_left {
                if px >= -threshold && cx < -threshold {
                    actions_to_execute.push(action.clone());
                }
            }
            if let Some(action) = &mapping.on_up {
                if py >= -threshold && cy < -threshold {
                    actions_to_execute.push(action.clone());
                }
            }
            if let Some(action) = &mapping.on_down {
                if py <= threshold && cy > threshold {
                    actions_to_execute.push(action.clone());
                }
            }

            if let Some(action) = &mapping.on_move {
                if (cx != 0.0 || cy != 0.0)
                    && self.debounce.can_trigger("right_stick_move", rate_limit)
                {
                    actions_to_execute.push(action.clone());
                }
            }
        }

        // Execute collected actions
        for action in actions_to_execute {
            self.execute_action(&action, ctx).await?;
        }

        Ok(())
    }

    async fn check_trigger_actions(
        &mut self,
        prev: &ControllerState,
        current: &ControllerState,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let (pl2, pr2) = prev.triggers.normalized();
        let (cl2, cr2) = current.triggers.normalized();

        // Collect actions to execute (to avoid borrow issues)
        let mut actions_to_execute: Vec<ActionConfig> = Vec::new();

        // L2 trigger
        if let Some(mapping) = &self.config.analog.l2_trigger {
            let threshold = mapping.threshold;

            if let Some(action) = &mapping.on_press {
                if pl2 < threshold && cl2 >= threshold {
                    actions_to_execute.push(action.clone());
                }
            }

            if let Some(action) = &mapping.on_change {
                if (pl2 - cl2).abs() > 0.01 {
                    actions_to_execute.push(action.clone());
                }
            }
        }

        // R2 trigger
        if let Some(mapping) = &self.config.analog.r2_trigger {
            let threshold = mapping.threshold;

            if let Some(action) = &mapping.on_press {
                if pr2 < threshold && cr2 >= threshold {
                    actions_to_execute.push(action.clone());
                }
            }

            if let Some(action) = &mapping.on_change {
                if (pr2 - cr2).abs() > 0.01 {
                    actions_to_execute.push(action.clone());
                }
            }
        }

        // Execute collected actions
        for action in actions_to_execute {
            self.execute_action(&action, ctx).await?;
        }

        Ok(())
    }

    /// Execute an action
    async fn execute_action(&mut self, action: &ActionConfig, ctx: &TemplateContext) -> Result<()> {
        // Shell command
        if let Some(cmd_template) = &action.command {
            self.execute_shell_command(cmd_template, ctx).await?;
        }

        // WebSocket message
        if let Some(ws_msg) = &action.websocket {
            self.send_websocket_message(ws_msg, ctx).await?;
        }

        // HTTP request
        if let Some(http_req) = &action.http {
            self.execute_http_request(http_req, ctx).await?;
        }

        // Rumble feedback
        if let Some(rumble) = &action.rumble {
            self.trigger_rumble(rumble).await?;
        }

        // LED feedback
        if let Some(led) = &action.led {
            self.controller_cmd_tx
                .send(ControllerCommand::SetLed(led.r, led.g, led.b))
                .await
                .ok();
        }

        Ok(())
    }

    async fn execute_shell_command(
        &self,
        cmd_template: &str,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let cmd = self
            .handlebars
            .render_template(cmd_template, ctx)
            .context("Failed to render command template")?;

        debug!("Executing shell command: {}", cmd);

        let shell = self
            .config
            .shell
            .shell
            .as_deref()
            .unwrap_or(if cfg!(windows) { "cmd" } else { "/bin/sh" });

        let shell_arg = if cfg!(windows) { "/C" } else { "-c" };

        let mut command = Command::new(shell);
        command.arg(shell_arg).arg(&cmd);

        if let Some(dir) = &self.config.shell.working_dir {
            command.current_dir(dir);
        }

        for (key, value) in &self.config.shell.env {
            command.env(key, value);
        }

        // Spawn and don't wait (fire and forget for non-blocking)
        match command.spawn() {
            Ok(mut child) => {
                // Optionally wait for short commands
                tokio::spawn(async move {
                    match child.wait() {
                        Ok(status) => {
                            if !status.success() {
                                warn!("Command exited with status: {}", status);
                            }
                        }
                        Err(e) => {
                            error!("Failed to wait for command: {}", e);
                        }
                    }
                });
            }
            Err(e) => {
                error!("Failed to spawn command: {}", e);
            }
        }

        Ok(())
    }

    async fn send_websocket_message(
        &mut self,
        ws_msg: &WebSocketMessage,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let Some(sender) = &self.ws_sender else {
            trace!("WebSocket not connected, skipping message");
            return Ok(());
        };

        let content = self
            .handlebars
            .render_template(&ws_msg.message, ctx)
            .context("Failed to render WebSocket message template")?;

        let message = if ws_msg.binary {
            Message::Binary(content.into_bytes().into())
        } else {
            Message::Text(content.into())
        };

        let mut sender = sender.lock().await;
        sender
            .send(message)
            .await
            .context("Failed to send WebSocket message")?;

        trace!("Sent WebSocket message");
        Ok(())
    }

    async fn execute_http_request(
        &self,
        http_req: &HttpRequest,
        ctx: &TemplateContext,
    ) -> Result<()> {
        let Some(client) = &self.http_client else {
            warn!("HTTP client not configured");
            return Ok(());
        };

        let Some(http_config) = &self.config.http else {
            warn!("HTTP not configured");
            return Ok(());
        };

        let url = format!("{}{}", http_config.base_url, http_req.path);
        let url = self
            .handlebars
            .render_template(&url, ctx)
            .context("Failed to render URL template")?;

        debug!("HTTP {} {}", http_req.method, url);

        let mut request = match http_req.method.to_uppercase().as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "DELETE" => client.delete(&url),
            "PATCH" => client.patch(&url),
            _ => client.post(&url),
        };

        // Add default headers
        for (key, value) in &http_config.headers {
            request = request.header(key, value);
        }

        // Add request-specific headers
        for (key, value) in &http_req.headers {
            request = request.header(key, value);
        }

        // Add body if present
        if let Some(body_template) = &http_req.body {
            let body = self
                .handlebars
                .render_template(body_template, ctx)
                .context("Failed to render body template")?;
            request = request.body(body);
        }

        // Fire and forget
        let request = request.build()?;
        let client = client.clone();
        tokio::spawn(async move {
            match client.execute(request).await {
                Ok(response) => {
                    if !response.status().is_success() {
                        warn!("HTTP request failed with status: {}", response.status());
                    }
                }
                Err(e) => {
                    error!("HTTP request failed: {}", e);
                }
            }
        });

        Ok(())
    }

    async fn trigger_rumble(&self, rumble: &RumbleConfig) -> Result<()> {
        self.controller_cmd_tx
            .send(ControllerCommand::SetRumble(
                rumble.left,
                rumble.right,
                rumble.duration_ms,
            ))
            .await
            .ok();
        Ok(())
    }

    /// Send raw state via WebSocket (for streaming)
    pub async fn send_state_update(&mut self, ctx: &TemplateContext) -> Result<()> {
        let Some(ws_config) = &self.config.websocket else {
            return Ok(());
        };

        let Some(format) = &ws_config.state_format else {
            return Ok(());
        };

        let Some(sender) = &self.ws_sender else {
            return Ok(());
        };

        let content = self
            .handlebars
            .render_template(format, ctx)
            .context("Failed to render state format")?;

        let message = if ws_config.binary {
            Message::Binary(content.into_bytes().into())
        } else {
            Message::Text(content.into())
        };

        let mut sender = sender.lock().await;
        sender.send(message).await.ok();

        Ok(())
    }
}
