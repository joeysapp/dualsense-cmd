//! DualSense Command CLI
//!
//! Cross-platform CLI for mapping DualSense controller inputs
//! to shell commands and WebSocket messages.

mod config;
mod dualsense;
mod executor;
mod websocket;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn, Level};
use tracing_subscriber::EnvFilter;

use crate::config::{Config, TemplateContext};
use crate::dualsense::{ConnectionType, ControllerState, DualSense};
use crate::executor::{ControllerCommand, Executor};
use crate::websocket::WebSocketManager;

/// DualSense controller command mapper
#[derive(Parser)]
#[command(name = "dualsense-cmd")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Configuration file or directory
    #[arg(short, long, default_value = "./config")]
    config: PathBuf,

    /// Verbose output
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the controller mapper with a configuration
    Run {
        /// Configuration file to use (overrides -c)
        #[arg(short, long)]
        profile: Option<PathBuf>,

        /// Dry run - show actions without executing
        #[arg(long)]
        dry_run: bool,
    },

    /// List connected DualSense controllers
    List,

    /// Show controller state in real-time
    Monitor {
        /// Show raw values instead of formatted
        #[arg(long)]
        raw: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Generate a sample configuration file
    Init {
        /// Output file path
        #[arg(short, long, default_value = "./config/config.json")]
        output: PathBuf,

        /// Configuration preset
        #[arg(short, long, default_value = "default")]
        preset: String,
    },

    /// Validate a configuration file
    Validate {
        /// Configuration file to validate
        file: PathBuf,
    },

    /// Test WebSocket connection
    TestWs {
        /// WebSocket URL to test
        url: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up logging
    let log_level = match cli.verbose {
        0 => Level::INFO,
        1 => Level::DEBUG,
        _ => Level::TRACE,
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(format!("dualsense_cmd={}", log_level).parse().unwrap()),
        )
        .with_target(false)
        .init();

    match cli.command {
        Commands::Run { profile, dry_run } => {
            let config_path = profile.unwrap_or(cli.config);
            run_mapper(config_path, dry_run).await
        }
        Commands::List => list_controllers().await,
        Commands::Monitor { raw, json } => monitor_controller(raw, json).await,
        Commands::Init { output, preset } => init_config(output, &preset).await,
        Commands::Validate { file } => validate_config(file).await,
        Commands::TestWs { url } => test_websocket(&url).await,
    }
}

async fn run_mapper(config_path: PathBuf, dry_run: bool) -> Result<()> {
    // Load configuration
    let config = Config::load_dir(&config_path)
        .with_context(|| format!("Failed to load config from {:?}", config_path))?;

    info!("Loaded configuration: {}", config.name);
    if dry_run {
        warn!("Dry run mode - actions will not be executed");
    }

    // Set up shutdown signal
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        info!("Received shutdown signal");
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Connect to controller
    println!(
        "{} Searching for DualSense controller...",
        "→".bright_blue()
    );

    let mut controller = DualSense::find_and_connect()
        .context("Failed to connect to DualSense controller")?;

    let connection_type = controller.connection_type();
    println!(
        "{} Connected via {}",
        "✓".bright_green(),
        match connection_type {
            ConnectionType::Usb => "USB".bright_cyan(),
            ConnectionType::Bluetooth => "Bluetooth".bright_magenta(),
        }
    );

    // Set up controller command channel
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ControllerCommand>(32);

    // Set initial LED color
    if let Some(led_config) = &config.led.connected_color {
        controller
            .set_led_color(led_config.r, led_config.g, led_config.b)
            .ok();
    } else {
        controller.set_led_color(0, 128, 255).ok(); // Default blue
    }

    // Set up WebSocket if configured
    let ws_manager = if let Some(ws_config) = &config.websocket {
        println!(
            "{} Connecting to WebSocket: {}",
            "→".bright_blue(),
            ws_config.url
        );

        let manager = WebSocketManager::new(ws_config.clone(), running.clone());
        Some(manager)
    } else {
        None
    };

    // Create executor
    let mut executor = Executor::new(config.clone(), cmd_tx.clone());

    // Start WebSocket connection in background if configured
    let ws_sender = if let Some(manager) = &ws_manager {
        let (msg_tx, mut msg_rx) = mpsc::channel::<String>(32);
        let manager_clone = manager.get_sender();

        // Spawn WebSocket handler
        let ws_running = running.clone();
        let ws_config = config.websocket.clone().unwrap();
        tokio::spawn(async move {
            let manager = WebSocketManager::new(ws_config, ws_running);
            if let Err(e) = manager.run(msg_tx).await {
                error!("WebSocket error: {}", e);
            }
        });

        // Handle incoming WebSocket messages
        tokio::spawn(async move {
            while let Some(msg) = msg_rx.recv().await {
                debug!("WebSocket message received: {}", msg);
                // Could be used for bidirectional communication
            }
        });

        // Wait a bit for connection
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Try to get a connected sender
        let sender = manager_clone;
        Some(sender)
    } else {
        None
    };

    // Set up WebSocket sender in executor if we have one
    if let Some(_sender) = ws_sender {
        // Connect directly via tokio-tungstenite for the executor
        if let Some(ws_config) = &config.websocket {
            match connect_async(&ws_config.url).await {
                Ok((ws_stream, _)) => {
                    println!("{} WebSocket connected", "✓".bright_green());
                    let (ws_sink, _ws_stream) = ws_stream.split();
                    executor.set_ws_sender(Arc::new(Mutex::new(ws_sink)));
                }
                Err(e) => {
                    warn!("Failed to connect WebSocket: {}", e);
                }
            }
        }
    }

    println!(
        "{} Running with config: {}",
        "✓".bright_green(),
        config.name.bright_yellow()
    );
    println!("{}", "Press Ctrl+C to stop".dimmed());
    println!();

    // Calculate poll interval
    let poll_interval = Duration::from_micros(1_000_000 / config.poll_rate as u64);
    let mut last_state_update = Instant::now();
    let state_interval = config
        .websocket
        .as_ref()
        .map(|ws| Duration::from_millis(ws.state_interval_ms))
        .unwrap_or(Duration::from_millis(0));

    // Main loop
    while running.load(Ordering::SeqCst) {
        // Poll controller and extract states by cloning
        let poll_result = controller.poll(poll_interval.as_millis() as i32);

        match poll_result {
            Ok(_) => {
                // Clone states to avoid borrow issues
                let current_state = controller.state().clone();
                let prev_state = controller.prev_state().clone();

                // Process state changes
                if !dry_run {
                    if let Err(e) = executor.process_state_change(&prev_state, &current_state).await {
                        error!("Error processing state change: {}", e);
                    }
                }

                // Send periodic state updates if configured
                if state_interval.as_millis() > 0
                    && last_state_update.elapsed() >= state_interval
                {
                    let ctx = TemplateContext::from(&current_state);
                    if let Err(e) = executor.send_state_update(&ctx).await {
                        debug!("Error sending state update: {}", e);
                    }
                    last_state_update = Instant::now();
                }
            }
            Err(dualsense::DualSenseError::Timeout) => {
                // Normal timeout, continue
            }
            Err(e) => {
                error!("Controller error: {}", e);
                break;
            }
        }

        // Handle controller commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ControllerCommand::SetLed(r, g, b) => {
                    controller.set_led_color(r, g, b).ok();
                }
                ControllerCommand::SetRumble(left, right, duration_ms) => {
                    controller.set_rumble(left, right).ok();
                    if duration_ms > 0 {
                        let _r = running.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(duration_ms)).await;
                            // Can't stop rumble here without controller reference
                            // This is a limitation we'd need to address with Arc<Mutex<>>
                        });
                    }
                }
            }
        }
    }

    // Clean up - explicitly close to ensure device is released
    controller.close();
    drop(controller); // Explicitly drop to release HID device
    println!("\n{} Disconnected", "✓".bright_green());

    Ok(())
}

async fn list_controllers() -> Result<()> {
    use hidapi::HidApi;

    println!("{}", "Searching for DualSense controllers...".dimmed());

    let api = HidApi::new().context("Failed to initialize HID API")?;

    let controllers: Vec<_> = api
        .device_list()
        .filter(|d| {
            d.vendor_id() == 0x054C
                && (d.product_id() == 0x0CE6 || d.product_id() == 0x0DF2)
        })
        .collect();

    if controllers.is_empty() {
        println!("{} No DualSense controllers found", "✗".bright_red());
        println!();
        println!("Make sure your controller is:");
        println!("  • Connected via USB cable, or");
        println!(
            "  • Paired via Bluetooth (hold {} + {} to pair)",
            "Create".bright_cyan(),
            "PS".bright_cyan()
        );
        return Ok(());
    }

    println!(
        "\n{} Found {} controller(s):\n",
        "✓".bright_green(),
        controllers.len()
    );

    for (i, device) in controllers.iter().enumerate() {
        let product = device.product_string().unwrap_or("DualSense");
        let serial = device.serial_number().unwrap_or("Unknown");
        let interface = device.interface_number();
        let connection = if interface == -1 {
            "Bluetooth".bright_magenta()
        } else {
            "USB".bright_cyan()
        };

        println!(
            "  {}. {} ({}) - Serial: {}",
            i + 1,
            product.bright_white(),
            connection,
            serial.dimmed()
        );
    }

    println!();
    Ok(())
}

async fn monitor_controller(raw: bool, json: bool) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    println!(
        "{} Searching for DualSense controller...",
        "→".bright_blue()
    );

    let mut controller = DualSense::find_and_connect()
        .context("Failed to connect to DualSense controller")?;

    println!("{} Connected! Monitoring inputs...", "✓".bright_green());
    println!("{}", "Press Ctrl+C to stop".dimmed());
    println!();

    // Set LED to indicate monitoring
    controller.set_led_color(0, 255, 0).ok();

    while running.load(Ordering::SeqCst) {
        match controller.poll(16) {
            Ok(state) => {
                if json {
                    print_state_json(state);
                } else if raw {
                    print_state_raw(state);
                } else {
                    print_state_pretty(state);
                }
            }
            Err(dualsense::DualSenseError::Timeout) => {}
            Err(e) => {
                error!("Controller error: {}", e);
                break;
            }
        }
    }

    controller.close();
    drop(controller);
    println!("\n{} Monitoring stopped", "✓".bright_green());
    Ok(())
}

fn print_state_json(state: &ControllerState) {
    let ctx = TemplateContext::from(state);
    if let Ok(json) = serde_json::to_string(&ctx) {
        println!("{}", json);
    }
}

fn print_state_raw(state: &ControllerState) {
    print!("\x1B[2J\x1B[1;1H"); // Clear screen
    println!("DualSense Raw State");
    println!("==================");
    println!(
        "Left Stick:  ({:3}, {:3})",
        state.left_stick.x, state.left_stick.y
    );
    println!(
        "Right Stick: ({:3}, {:3})",
        state.right_stick.x, state.right_stick.y
    );
    println!(
        "Triggers:    L2={:3} R2={:3}",
        state.triggers.l2, state.triggers.r2
    );
    println!(
        "Gyro:        ({:6}, {:6}, {:6})",
        state.gyroscope.x, state.gyroscope.y, state.gyroscope.z
    );
    println!(
        "Accel:       ({:6}, {:6}, {:6})",
        state.accelerometer.x, state.accelerometer.y, state.accelerometer.z
    );
    println!("Buttons:     {:?}", state.buttons);
}

fn print_state_pretty(state: &ControllerState) {
    print!("\x1B[2J\x1B[1;1H"); // Clear screen

    let (lx, ly) = state.left_stick.normalized();
    let (rx, ry) = state.right_stick.normalized();
    let (l2, r2) = state.triggers.normalized();
    let (roll, pitch, yaw) = state.euler_angles();

    println!("{}", "DualSense Controller State".bright_white().bold());
    println!("{}", "══════════════════════════════════════".dimmed());

    // Sticks
    println!("\n{}", "Analog Sticks".bright_cyan());
    println!(
        "  Left:  X {:+.2}  Y {:+.2}",
        format_value(lx),
        format_value(ly)
    );
    println!(
        "  Right: X {:+.2}  Y {:+.2}",
        format_value(rx),
        format_value(ry)
    );

    // Triggers
    println!("\n{}", "Triggers".bright_cyan());
    println!("  L2: {}  R2: {}", format_bar(l2), format_bar(r2));

    // D-Pad
    println!("\n{}", "D-Pad".bright_cyan());
    let dpad = format!(
        "  {} {} {} {}",
        if state.buttons.dpad_up { "↑" } else { "·" },
        if state.buttons.dpad_down { "↓" } else { "·" },
        if state.buttons.dpad_left { "←" } else { "·" },
        if state.buttons.dpad_right { "→" } else { "·" }
    );
    println!("{}", dpad);

    // Face buttons
    println!("\n{}", "Face Buttons".bright_cyan());
    let face = format!(
        "  △:{}  ○:{}  ✕:{}  □:{}",
        if state.buttons.triangle { "●" } else { "○" },
        if state.buttons.circle { "●" } else { "○" },
        if state.buttons.cross { "●" } else { "○" },
        if state.buttons.square { "●" } else { "○" }
    );
    println!("{}", face);

    // Shoulder buttons
    println!("\n{}", "Shoulder".bright_cyan());
    println!(
        "  L1:{}  R1:{}  L3:{}  R3:{}",
        if state.buttons.l1 { "●" } else { "○" },
        if state.buttons.r1 { "●" } else { "○" },
        if state.buttons.l3 { "●" } else { "○" },
        if state.buttons.r3 { "●" } else { "○" }
    );

    // Orientation
    println!("\n{}", "Orientation (Euler)".bright_cyan());
    println!(
        "  Roll: {:+.1}°  Pitch: {:+.1}°  Yaw: {:+.1}°",
        roll.to_degrees(),
        pitch.to_degrees(),
        yaw.to_degrees()
    );

    // Battery
    println!("\n{}", "Battery".bright_cyan());
    let battery_icon = if state.battery.charging {
        "⚡"
    } else {
        "🔋"
    };
    println!("{} {}%", battery_icon, state.battery.percentage());

    // Touchpad
    if state.touchpad.finger1.active || state.touchpad.finger2.active {
        println!("\n{}", "Touchpad".bright_cyan());
        if state.touchpad.finger1.active {
            println!(
                "  Touch 1: ({}, {})",
                state.touchpad.finger1.x, state.touchpad.finger1.y
            );
        }
        if state.touchpad.finger2.active {
            println!(
                "  Touch 2: ({}, {})",
                state.touchpad.finger2.x, state.touchpad.finger2.y
            );
        }
    }

    println!("\n{}", "Press Ctrl+C to stop".dimmed());
}

fn format_value(v: f32) -> String {
    if v > 0.1 {
        format!("{:+.2}", v).bright_green().to_string()
    } else if v < -0.1 {
        format!("{:+.2}", v).bright_red().to_string()
    } else {
        format!("{:+.2}", v).dimmed().to_string()
    }
}

fn format_bar(v: f32) -> String {
    let filled = (v * 10.0) as usize;
    let bar: String = (0..10)
        .map(|i| if i < filled { '█' } else { '░' })
        .collect();
    if v > 0.5 {
        bar.bright_green().to_string()
    } else if v > 0.1 {
        bar.bright_yellow().to_string()
    } else {
        bar.dimmed().to_string()
    }
}

async fn init_config(output: PathBuf, preset: &str) -> Result<()> {
    use crate::config::*;

    let config = match preset {
        "axidraw" => Config {
            name: "AxiDraw Controller".to_string(),
            poll_rate: 60,
            deadzone: 0.15,
            http: Some(HttpConfig {
                base_url: "http://localhost:9700".to_string(),
                headers: [("Content-Type".to_string(), "application/json".to_string())]
                    .into_iter()
                    .collect(),
                timeout_ms: 5000,
            }),
            buttons: ButtonMappings {
                cross: Some(ActionConfig {
                    trigger: "press".to_string(),
                    http: Some(HttpRequest {
                        method: "POST".to_string(),
                        path: "/pen/down".to_string(),
                        body: None,
                        headers: Default::default(),
                    }),
                    rumble: Some(RumbleConfig {
                        left: 50,
                        right: 50,
                        duration_ms: 100,
                    }),
                    ..Default::default()
                }),
                circle: Some(ActionConfig {
                    trigger: "press".to_string(),
                    http: Some(HttpRequest {
                        method: "POST".to_string(),
                        path: "/pen/up".to_string(),
                        body: None,
                        headers: Default::default(),
                    }),
                    ..Default::default()
                }),
                triangle: Some(ActionConfig {
                    trigger: "press".to_string(),
                    http: Some(HttpRequest {
                        method: "POST".to_string(),
                        path: "/home".to_string(),
                        body: None,
                        headers: Default::default(),
                    }),
                    ..Default::default()
                }),
                dpad_up: Some(ActionConfig {
                    trigger: "press".to_string(),
                    http: Some(HttpRequest {
                        method: "POST".to_string(),
                        path: "/move".to_string(),
                        body: Some(r#"{"dx": 0, "dy": -5, "units": "mm"}"#.to_string()),
                        headers: Default::default(),
                    }),
                    debounce_ms: 100,
                    ..Default::default()
                }),
                dpad_down: Some(ActionConfig {
                    trigger: "press".to_string(),
                    http: Some(HttpRequest {
                        method: "POST".to_string(),
                        path: "/move".to_string(),
                        body: Some(r#"{"dx": 0, "dy": 5, "units": "mm"}"#.to_string()),
                        headers: Default::default(),
                    }),
                    debounce_ms: 100,
                    ..Default::default()
                }),
                dpad_left: Some(ActionConfig {
                    trigger: "press".to_string(),
                    http: Some(HttpRequest {
                        method: "POST".to_string(),
                        path: "/move".to_string(),
                        body: Some(r#"{"dx": -5, "dy": 0, "units": "mm"}"#.to_string()),
                        headers: Default::default(),
                    }),
                    debounce_ms: 100,
                    ..Default::default()
                }),
                dpad_right: Some(ActionConfig {
                    trigger: "press".to_string(),
                    http: Some(HttpRequest {
                        method: "POST".to_string(),
                        path: "/move".to_string(),
                        body: Some(r#"{"dx": 5, "dy": 0, "units": "mm"}"#.to_string()),
                        headers: Default::default(),
                    }),
                    debounce_ms: 100,
                    ..Default::default()
                }),
                options: Some(ActionConfig {
                    trigger: "press".to_string(),
                    http: Some(HttpRequest {
                        method: "POST".to_string(),
                        path: "/stop".to_string(),
                        body: None,
                        headers: Default::default(),
                    }),
                    rumble: Some(RumbleConfig {
                        left: 255,
                        right: 255,
                        duration_ms: 200,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            analog: AnalogMappings {
                left_stick: Some(StickMapping {
                    on_move: Some(ActionConfig {
                        trigger: "change".to_string(),
                        http: Some(HttpRequest {
                            method: "POST".to_string(),
                            path: "/move".to_string(),
                            body: Some(
                                r#"{"dx": {{left_stick_x}}, "dy": {{left_stick_y}}, "units": "mm"}"#
                                    .to_string(),
                            ),
                            headers: Default::default(),
                        }),
                        ..Default::default()
                    }),
                    rate_limit_ms: 50,
                    threshold: 0.5,
                    ..Default::default()
                }),
                ..Default::default()
            },
            led: LedConfig {
                connected_color: Some(LedColorConfig { r: 0, g: 128, b: 255 }),
                error_color: Some(LedColorConfig { r: 255, g: 0, b: 0 }),
                ..Default::default()
            },
            ..Default::default()
        },
        "websocket" => Config {
            name: "WebSocket Streaming".to_string(),
            poll_rate: 120,
            deadzone: 0.1,
            websocket: Some(WebSocketConfig {
                url: "ws://localhost:8080/controller".to_string(),
                reconnect: true,
                reconnect_delay_ms: 1000,
                max_reconnect_attempts: 0,
                state_format: Some(
                    r#"{"type":"state","data":{"lx":{{left_stick_x}},"ly":{{left_stick_y}},"rx":{{right_stick_x}},"ry":{{right_stick_y}},"l2":{{l2_trigger}},"r2":{{r2_trigger}},"roll":{{roll}},"pitch":{{pitch}},"yaw":{{yaw}}}}"#.to_string()
                ),
                state_interval_ms: 16, // ~60fps
                binary: false,
            }),
            buttons: ButtonMappings {
                cross: Some(ActionConfig {
                    trigger: "press".to_string(),
                    websocket: Some(WebSocketMessage {
                        message: r#"{"type":"button","button":"cross","state":"pressed"}"#.to_string(),
                        binary: false,
                    }),
                    ..Default::default()
                }),
                circle: Some(ActionConfig {
                    trigger: "press".to_string(),
                    websocket: Some(WebSocketMessage {
                        message: r#"{"type":"button","button":"circle","state":"pressed"}"#.to_string(),
                        binary: false,
                    }),
                    ..Default::default()
                }),
                square: Some(ActionConfig {
                    trigger: "press".to_string(),
                    websocket: Some(WebSocketMessage {
                        message: r#"{"type":"button","button":"square","state":"pressed"}"#.to_string(),
                        binary: false,
                    }),
                    ..Default::default()
                }),
                triangle: Some(ActionConfig {
                    trigger: "press".to_string(),
                    websocket: Some(WebSocketMessage {
                        message: r#"{"type":"button","button":"triangle","state":"pressed"}"#.to_string(),
                        binary: false,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            led: LedConfig {
                connected_color: Some(LedColorConfig { r: 0, g: 255, b: 128 }),
                ..Default::default()
            },
            ..Default::default()
        },
        "shell" => Config {
            name: "Shell Commands".to_string(),
            poll_rate: 60,
            deadzone: 0.15,
            shell: ShellConfig {
                shell: None,
                working_dir: None,
                env: Default::default(),
            },
            buttons: ButtonMappings {
                cross: Some(ActionConfig {
                    trigger: "press".to_string(),
                    command: Some("echo 'Cross pressed'".to_string()),
                    ..Default::default()
                }),
                circle: Some(ActionConfig {
                    trigger: "press".to_string(),
                    command: Some("echo 'Circle pressed'".to_string()),
                    ..Default::default()
                }),
                dpad_up: Some(ActionConfig {
                    trigger: "press".to_string(),
                    command: Some("echo 'Up'".to_string()),
                    debounce_ms: 200,
                    ..Default::default()
                }),
                dpad_down: Some(ActionConfig {
                    trigger: "press".to_string(),
                    command: Some("echo 'Down'".to_string()),
                    debounce_ms: 200,
                    ..Default::default()
                }),
                dpad_left: Some(ActionConfig {
                    trigger: "press".to_string(),
                    command: Some("echo 'Left'".to_string()),
                    debounce_ms: 200,
                    ..Default::default()
                }),
                dpad_right: Some(ActionConfig {
                    trigger: "press".to_string(),
                    command: Some("echo 'Right'".to_string()),
                    debounce_ms: 200,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        },
        _ => Config::default(),
    };

    // Create parent directory if needed
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    config.save(&output)?;

    println!(
        "{} Created configuration: {}",
        "✓".bright_green(),
        output.display()
    );
    println!(
        "\nRun with: {} run -c {}",
        "dualsense-cmd".bright_cyan(),
        output.display()
    );

    Ok(())
}

async fn validate_config(file: PathBuf) -> Result<()> {
    print!("Validating {}... ", file.display());

    match Config::load(&file) {
        Ok(config) => {
            println!("{}", "OK".bright_green());
            println!("\nConfiguration: {}", config.name.bright_yellow());
            println!("  Poll rate: {} Hz", config.poll_rate);
            println!("  Deadzone: {:.2}", config.deadzone);

            if config.websocket.is_some() {
                println!(
                    "  WebSocket: {}",
                    config.websocket.as_ref().unwrap().url.bright_cyan()
                );
            }
            if config.http.is_some() {
                println!(
                    "  HTTP: {}",
                    config.http.as_ref().unwrap().base_url.bright_cyan()
                );
            }

            // Count configured buttons
            let button_count = count_configured_buttons(&config.buttons);
            println!("  Buttons configured: {}", button_count);

            Ok(())
        }
        Err(e) => {
            println!("{}", "FAILED".bright_red());
            println!("\nError: {}", e);
            Err(e)
        }
    }
}

fn count_configured_buttons(buttons: &config::ButtonMappings) -> usize {
    let mut count = 0;
    if buttons.cross.is_some() { count += 1; }
    if buttons.circle.is_some() { count += 1; }
    if buttons.square.is_some() { count += 1; }
    if buttons.triangle.is_some() { count += 1; }
    if buttons.dpad_up.is_some() { count += 1; }
    if buttons.dpad_down.is_some() { count += 1; }
    if buttons.dpad_left.is_some() { count += 1; }
    if buttons.dpad_right.is_some() { count += 1; }
    if buttons.l1.is_some() { count += 1; }
    if buttons.r1.is_some() { count += 1; }
    if buttons.l2_button.is_some() { count += 1; }
    if buttons.r2_button.is_some() { count += 1; }
    if buttons.l3.is_some() { count += 1; }
    if buttons.r3.is_some() { count += 1; }
    if buttons.options.is_some() { count += 1; }
    if buttons.create.is_some() { count += 1; }
    if buttons.ps.is_some() { count += 1; }
    if buttons.touchpad.is_some() { count += 1; }
    if buttons.mute.is_some() { count += 1; }
    count
}

async fn test_websocket(url: &str) -> Result<()> {
    println!("Testing WebSocket connection to: {}", url.bright_cyan());

    match connect_async(url).await {
        Ok((mut ws_stream, response)) => {
            println!("{} Connected!", "✓".bright_green());
            println!("  Response: {:?}", response.status());

            // Try to receive a message
            println!("\nWaiting for messages (5 seconds)...");

            let timeout = tokio::time::timeout(Duration::from_secs(5), async {
                while let Some(msg) = ws_stream.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            println!("  Received: {}", text.bright_yellow());
                        }
                        Ok(Message::Binary(data)) => {
                            println!("  Received binary: {} bytes", data.len());
                        }
                        Ok(Message::Ping(_)) => {
                            println!("  Received ping");
                        }
                        Ok(Message::Close(_)) => {
                            println!("  Connection closed by server");
                            break;
                        }
                        Err(e) => {
                            println!("  Error: {}", e);
                            break;
                        }
                        _ => {}
                    }
                }
            })
            .await;

            if timeout.is_err() {
                println!("  (timeout - no messages received)");
            }

            println!("\n{} WebSocket test complete", "✓".bright_green());
            Ok(())
        }
        Err(e) => {
            println!("{} Connection failed: {}", "✗".bright_red(), e);
            Err(e.into())
        }
    }
}

impl Default for config::ActionConfig {
    fn default() -> Self {
        Self {
            trigger: "press".to_string(),
            command: None,
            websocket: None,
            http: None,
            rumble: None,
            led: None,
            debounce_ms: 0,
            hold_time_ms: 0,
        }
    }
}

impl Default for config::StickMapping {
    fn default() -> Self {
        Self {
            on_move: None,
            on_right: None,
            on_left: None,
            on_up: None,
            on_down: None,
            threshold: 0.5,
            rate_limit_ms: 0,
        }
    }
}
