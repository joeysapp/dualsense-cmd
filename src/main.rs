//! DualSense Command CLI
//!
//! Cross-platform CLI for mapping DualSense controller inputs
//! to shell commands and WebSocket messages.

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

use dualsense_cmd::config::{self, Config, TemplateContext};
use dualsense_cmd::dualsense::{ConnectionType, ControllerState, DualSense, DualSenseError};
use dualsense_cmd::executor::{ControllerCommand, Executor};
use dualsense_cmd::profile::{Profile, ProfileManager};
use dualsense_cmd::spatial::{IntegrationConfig, SpatialState, VelocityCurve};
use dualsense_cmd::websocket::WebSocketManager;
use dualsense_cmd::renderer;

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

    /// Open 3D visualization of controller orientation and motion
    #[command(name = "3d")]
    ThreeD,

    /// Manage controller profiles (LED, triggers, player LEDs)
    Profile {
        #[command(subcommand)]
        action: ProfileCommands,
    },

    /// Show supported protocol features and their status
    Features,
}

#[derive(Subcommand)]
enum ProfileCommands {
    /// List available profiles
    List,

    /// Show profile details
    Show {
        /// Profile name
        name: String,
    },

    /// Apply a profile to the connected controller
    Apply {
        /// Profile name
        name: String,
    },

    /// Create a new profile
    Create {
        /// Profile name
        name: String,

        /// Profile description
        #[arg(short, long)]
        description: Option<String>,

        /// LED color (hex format: #RRGGBB or RRGGBB)
        #[arg(long)]
        led: Option<String>,

        /// L2 trigger effect preset: off, continuous, weapon, bow, vibration
        #[arg(long)]
        l2: Option<String>,

        /// L2 trigger force (0-255)
        #[arg(long)]
        l2_force: Option<u8>,

        /// R2 trigger effect preset: off, continuous, weapon, bow, vibration
        #[arg(long)]
        r2: Option<String>,

        /// R2 trigger force (0-255)
        #[arg(long)]
        r2_force: Option<u8>,

        /// Player number for LEDs (1-5)
        #[arg(long)]
        player: Option<u8>,

        /// Initialize from a preset: default, gaming, racing, accessibility
        #[arg(long)]
        preset: Option<String>,
    },

    /// Delete a profile
    Delete {
        /// Profile name
        name: String,

        /// Skip confirmation
        #[arg(short, long)]
        force: bool,
    },

    /// Initialize default profiles
    InitDefaults,

    /// Show profiles directory
    Dir,
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
        Commands::ThreeD => run_3d_viewer().await,
        Commands::Profile { action } => handle_profile_command(action).await,
        Commands::Features => show_features().await,
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
    let mut last_frame_time = Instant::now();
    let state_interval = config
        .websocket
        .as_ref()
        .map(|ws| Duration::from_millis(ws.state_interval_ms))
        .unwrap_or(Duration::from_millis(0));

    // Set up spatial integration if configured
    let mut spatial_state = config.integration.as_ref().map(|int_config| {
        let velocity_curve = match int_config.velocity_curve.to_lowercase().as_str() {
            "quadratic" => VelocityCurve::Quadratic,
            "cubic" => VelocityCurve::Cubic,
            _ => VelocityCurve::Linear,
        };

        let gyro_weight = int_config
            .orientation_filter
            .as_ref()
            .map(|f| f.gyro_weight)
            .unwrap_or(0.98);

        let spatial_config = IntegrationConfig {
            velocity_curve,
            max_linear_speed: int_config.max_linear_speed,
            max_angular_speed: int_config.max_angular_speed,
            linear_damping: int_config.linear_damping,
            angular_damping: int_config.angular_damping,
            smoothing_alpha: int_config.smoothing_alpha,
            gyro_weight,
            deadzone: config.deadzone,
        };

        info!(
            "Spatial integration enabled: max_speed={} mm/s, damping={}, curve={:?}",
            spatial_config.max_linear_speed,
            spatial_config.linear_damping,
            spatial_config.velocity_curve
        );

        SpatialState::new(spatial_config)
    });

    if spatial_state.is_some() {
        println!(
            "{} Spatial integration enabled",
            "✓".bright_green()
        );
    }

    // Main loop
    while running.load(Ordering::SeqCst) {
        // Calculate delta time
        let dt = last_frame_time.elapsed().as_secs_f32();
        last_frame_time = Instant::now();

        // Poll controller and extract states by cloning
        let poll_result = controller.poll(poll_interval.as_millis() as i32);

        match poll_result {
            Ok(_) => {
                // Clone states to avoid borrow issues
                let current_state = controller.state().clone();
                let prev_state = controller.prev_state().clone();

                // Update spatial integration if enabled
                if let Some(ref mut spatial) = spatial_state {
                    spatial.integrate(&current_state, dt);
                }

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
                    let ctx = TemplateContext::from_controller(
                        &current_state,
                        spatial_state.as_ref(),
                    );
                    if let Err(e) = executor.send_state_update(&ctx).await {
                        debug!("Error sending state update: {}", e);
                    }
                    last_state_update = Instant::now();
                }
            }
            Err(DualSenseError::Timeout) => {
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
            Err(DualSenseError::Timeout) => {}
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
    use dualsense_cmd::config::*;

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

async fn run_3d_viewer() -> Result<()> {
    use std::sync::mpsc;
    use std::thread;

    println!(
        "{} Starting 3D visualization...",
        "→".bright_blue()
    );

    // Set up shutdown signal
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
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

    // Set LED to indicate 3D mode (purple)
    controller.set_led_color(128, 0, 255).ok();

    // Create channel for sending spatial state to renderer
    let (tx, rx) = mpsc::channel::<SpatialState>();

    println!("{} Opening 3D window...", "→".bright_blue());
    println!("{}", "Close the window or press Ctrl+C to stop".dimmed());

    // On macOS, winit requires the event loop to run on the main thread.
    // So we spawn the controller polling in a background thread instead.
    let controller_running = running.clone();
    let controller_handle = thread::spawn(move || {
        let spatial_config = IntegrationConfig::default();
        let mut spatial_state = SpatialState::new(spatial_config);
        let mut last_frame = std::time::Instant::now();

        while controller_running.load(Ordering::SeqCst) {
            let dt = last_frame.elapsed().as_secs_f32();
            last_frame = std::time::Instant::now();

            match controller.poll(8) {
                Ok(state) => {
                    // Update spatial state with controller data
                    spatial_state.integrate(state, dt);

                    // Send snapshot of spatial state to renderer
                    if tx.send(spatial_state.snapshot()).is_err() {
                        // Receiver dropped, exit
                        break;
                    }
                }
                Err(DualSenseError::Timeout) => {
                    // Normal timeout, continue
                }
                Err(e) => {
                    eprintln!("Controller error: {}", e);
                    break;
                }
            }

            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // Clean up
        controller.close();
    });

    // Run renderer on main thread (required by macOS)
    if let Err(e) = renderer::run_3d_visualization(rx) {
        eprintln!("Renderer error: {}", e);
    }

    // Signal controller thread to stop
    running.store(false, Ordering::SeqCst);

    // Wait for controller thread
    let _ = controller_handle.join();

    println!("\n{} 3D visualization stopped", "✓".bright_green());

    Ok(())
}

async fn handle_profile_command(action: ProfileCommands) -> Result<()> {
    use dualsense_cmd::profile::{ProfileLedColor, ProfileTriggerEffect, ProfilePlayerLeds};

    let manager = ProfileManager::new()?;

    match action {
        ProfileCommands::List => {
            let profiles = manager.list()?;

            if profiles.is_empty() {
                println!("{} No profiles found", "!".bright_yellow());
                println!("\nCreate default profiles with: {} profile init-defaults", "dualsense-cmd".bright_cyan());
                println!("Or create a new profile with: {} profile create <name>", "dualsense-cmd".bright_cyan());
            } else {
                println!("{}", "Available Profiles".bright_white().bold());
                println!("{}", "══════════════════════════════════════".dimmed());
                for profile in profiles {
                    println!(
                        "  {} - {}",
                        profile.name.bright_cyan(),
                        profile.description.dimmed()
                    );
                }
            }
        }

        ProfileCommands::Show { name } => {
            let profile = manager.get(&name)?;

            println!("{}", "Profile Details".bright_white().bold());
            println!("{}", "══════════════════════════════════════".dimmed());
            println!("  Name:        {}", profile.name.bright_cyan());
            println!("  Description: {}", profile.description);
            println!(
                "  LED Color:   #{:02X}{:02X}{:02X}",
                profile.led_color.r, profile.led_color.g, profile.led_color.b
            );
            println!("  L2 Trigger:  {} (force: {})",
                profile.l2_trigger.effect_type.bright_yellow(),
                profile.l2_trigger.force
            );
            println!("  R2 Trigger:  {} (force: {})",
                profile.r2_trigger.effect_type.bright_yellow(),
                profile.r2_trigger.force
            );
            if let Some(ref leds) = profile.player_leds {
                match leds {
                    ProfilePlayerLeds::Number(n) => println!("  Player LEDs: Player {}", n),
                    ProfilePlayerLeds::Custom { led1, led2, led3, led4, led5 } => {
                        let pattern: String = [led1, led2, led3, led4, led5]
                            .iter()
                            .map(|&b| if *b { "●" } else { "○" })
                            .collect::<Vec<_>>()
                            .join(" ");
                        println!("  Player LEDs: {}", pattern);
                    }
                }
            }
        }

        ProfileCommands::Apply { name } => {
            let profile = manager.get(&name)?;

            println!(
                "{} Applying profile: {}",
                "→".bright_blue(),
                profile.name.bright_cyan()
            );

            let controller = DualSense::find_and_connect()
                .context("Failed to connect to DualSense controller")?;

            // Apply the output state from profile
            let output_state = profile.to_output_state();
            controller.apply_output_state(output_state)?;

            println!("{} Profile applied successfully", "✓".bright_green());
            println!(
                "  LED: #{:02X}{:02X}{:02X}",
                profile.led_color.r, profile.led_color.g, profile.led_color.b
            );
            println!("  L2:  {}", profile.l2_trigger.effect_type);
            println!("  R2:  {}", profile.r2_trigger.effect_type);

            // Keep controller alive briefly to let effects take
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        ProfileCommands::Create {
            name,
            description,
            led,
            l2,
            l2_force,
            r2,
            r2_force,
            player,
            preset,
        } => {
            // Start from preset or default
            let mut profile = match preset.as_deref() {
                Some("gaming") => Profile::preset_gaming(),
                Some("racing") => Profile::preset_racing(),
                Some("accessibility") => Profile::preset_accessibility(),
                _ => Profile::preset_default(),
            };

            // Override with provided values
            profile.name = name.clone();
            if let Some(desc) = description {
                profile.description = desc;
            }

            if let Some(led_hex) = led {
                let hex = led_hex.trim_start_matches('#');
                if hex.len() == 6 {
                    if let (Ok(r), Ok(g), Ok(b)) = (
                        u8::from_str_radix(&hex[0..2], 16),
                        u8::from_str_radix(&hex[2..4], 16),
                        u8::from_str_radix(&hex[4..6], 16),
                    ) {
                        profile.led_color = ProfileLedColor { r, g, b };
                    }
                }
            }

            if let Some(l2_type) = l2 {
                profile.l2_trigger = ProfileTriggerEffect {
                    effect_type: l2_type,
                    force: l2_force.unwrap_or(200),
                    start: 70,
                    end: 160,
                    frequency: 10,
                };
            } else if let Some(force) = l2_force {
                profile.l2_trigger.force = force;
            }

            if let Some(r2_type) = r2 {
                profile.r2_trigger = ProfileTriggerEffect {
                    effect_type: r2_type,
                    force: r2_force.unwrap_or(200),
                    start: 70,
                    end: 160,
                    frequency: 10,
                };
            } else if let Some(force) = r2_force {
                profile.r2_trigger.force = force;
            }

            if let Some(p) = player {
                profile.player_leds = Some(ProfilePlayerLeds::Number(p.clamp(1, 5)));
            }

            let path = manager.save(&profile)?;
            println!("{} Created profile: {}", "✓".bright_green(), profile.name.bright_cyan());
            println!("  Saved to: {}", path.display().to_string().dimmed());
        }

        ProfileCommands::Delete { name, force } => {
            if !manager.exists(&name) {
                println!("{} Profile not found: {}", "✗".bright_red(), name);
                return Ok(());
            }

            if !force {
                println!("Delete profile '{}'? [y/N] ", name.bright_yellow());
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Cancelled");
                    return Ok(());
                }
            }

            manager.delete(&name)?;
            println!("{} Deleted profile: {}", "✓".bright_green(), name);
        }

        ProfileCommands::InitDefaults => {
            manager.init_defaults()?;
            println!("{} Created default profiles", "✓".bright_green());
            for profile in manager.list()? {
                println!("  • {}", profile.name.bright_cyan());
            }
        }

        ProfileCommands::Dir => {
            let dir = ProfileManager::get_profiles_dir()?;
            println!("Profiles directory: {}", dir.display().to_string().bright_cyan());
            println!("\nSet {} environment variable to change location", "DUALSENSE_HOME".bright_yellow());
        }
    }

    Ok(())
}

async fn show_features() -> Result<()> {
    println!("{}", "DualSense Protocol Features".bright_white().bold());
    println!("{}", "═══════════════════════════════════════════════════════════════════".dimmed());
    println!();

    // Input features
    println!("{}", "INPUT (Receiving from Controller)".bright_cyan().bold());
    println!("{}", "─────────────────────────────────────────────────────────────────".dimmed());

    let input_features = [
        ("✓", "Thumbsticks (analog)", "Left stick, Right stick - X/Y axes with deadzone support"),
        ("✓", "Action buttons", "Cross, Circle, Square, Triangle - digital press/release"),
        ("✓", "Directional buttons", "D-pad Up, Down, Left, Right - 8-way hat switch"),
        ("✓", "Bumpers", "L1, R1 - digital shoulder buttons"),
        ("✓", "Triggers (pressure-sensitive)", "L2, R2 - 8-bit analog (0-255)"),
        ("✓", "Thumbstick buttons", "L3, R3 - click detection"),
        ("✓", "Create button", "Front-left system button"),
        ("✓", "Options button", "Front-right menu button"),
        ("✓", "PS button", "Central PlayStation button"),
        ("✓", "Mute button", "Microphone mute toggle with LED"),
        ("✓", "Touch pad button", "Clickable touchpad surface"),
        ("✓", "Touch pad multitouch", "2-finger tracking with X/Y coordinates"),
        ("✓", "Accelerometer", "3-axis acceleration (X, Y, Z) in G"),
        ("✓", "Gyroscope", "3-axis rotation (X, Y, Z) in rad/s"),
        ("✓", "Battery status", "Level (0-100%), charging state"),
        ("◐", "Microphone", "Audio input - requires OS-level access"),
        ("◐", "Headset jack input", "Audio input - requires OS-level access"),
    ];

    for (status, name, desc) in input_features {
        let status_colored = match status {
            "✓" => status.bright_green(),
            "◐" => status.bright_yellow(),
            _ => status.dimmed(),
        };
        println!("  {} {} - {}", status_colored, name.bright_white(), desc.dimmed());
    }

    println!();
    println!("{}", "OUTPUT (Sending to Controller)".bright_magenta().bold());
    println!("{}", "─────────────────────────────────────────────────────────────────".dimmed());

    let output_features = [
        ("✓", "Haptic feedback", "Dual rumble motors (left: low-freq, right: high-freq)"),
        ("✓", "Adaptive triggers", "Programmable L2/R2 resistance and vibration effects"),
        ("✓", "Light bar (RGB LED)", "Full color control with brightness"),
        ("✓", "Player LEDs", "5 indicator LEDs below touchpad"),
        ("✓", "Mute LED", "Mic mute indicator control (on/off/breathing)"),
        ("◐", "Speaker", "Audio output - requires OS-level access"),
        ("◐", "Headset jack output", "Audio output - requires OS-level access"),
    ];

    for (status, name, desc) in output_features {
        let status_colored = match status {
            "✓" => status.bright_green(),
            "◐" => status.bright_yellow(),
            _ => status.dimmed(),
        };
        println!("  {} {} - {}", status_colored, name.bright_white(), desc.dimmed());
    }

    println!();
    println!("{}", "CONNECTION".bright_blue().bold());
    println!("{}", "─────────────────────────────────────────────────────────────────".dimmed());

    let connection_features = [
        ("✓", "USB", "Direct HID connection, no authentication required"),
        ("✓", "Bluetooth", "Wireless with CRC32 checksum validation"),
    ];

    for (status, name, desc) in connection_features {
        let status_colored = match status {
            "✓" => status.bright_green(),
            "◐" => status.bright_yellow(),
            _ => status.dimmed(),
        };
        println!("  {} {} - {}", status_colored, name.bright_white(), desc.dimmed());
    }

    println!();
    println!("{}", "Legend:".dimmed());
    println!("  {} Implemented     {} OS/Future", "✓".bright_green(), "◐".bright_yellow());

    println!();
    println!("{}", "Note: Bluetooth output features may require 'identifying' the controller".dimmed());
    println!("{}", "      through System Settings on macOS before they work properly.".dimmed());

    Ok(())
}

