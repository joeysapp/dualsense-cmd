//! Configuration types and loading
//!
//! Defines the configuration schema for mapping controller inputs
//! to shell commands and WebSocket messages.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// Root configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Configuration name/description
    #[serde(default)]
    pub name: String,

    /// Polling rate in Hz (default: 100)
    #[serde(default = "default_poll_rate")]
    pub poll_rate: u32,

    /// Deadzone for analog sticks (0.0 - 1.0)
    #[serde(default = "default_deadzone")]
    pub deadzone: f32,

    /// Shell execution settings
    #[serde(default)]
    pub shell: ShellConfig,

    /// WebSocket connection settings
    #[serde(default)]
    pub websocket: Option<WebSocketConfig>,

    /// HTTP endpoint settings
    #[serde(default)]
    pub http: Option<HttpConfig>,

    /// Button mappings
    #[serde(default)]
    pub buttons: ButtonMappings,

    /// Analog input mappings
    #[serde(default)]
    pub analog: AnalogMappings,

    /// Motion/IMU mappings
    #[serde(default)]
    pub motion: MotionMappings,

    /// LED configuration
    #[serde(default)]
    pub led: LedConfig,
}

fn default_poll_rate() -> u32 {
    100
}

fn default_deadzone() -> f32 {
    0.1
}

/// Shell execution configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShellConfig {
    /// Shell to use (default: /bin/sh on Unix, cmd on Windows)
    #[serde(default)]
    pub shell: Option<String>,

    /// Working directory for commands
    #[serde(default)]
    pub working_dir: Option<String>,

    /// Environment variables to set
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// WebSocket configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketConfig {
    /// WebSocket URL to connect to
    pub url: String,

    /// Reconnect on disconnect
    #[serde(default = "default_true")]
    pub reconnect: bool,

    /// Reconnect delay in milliseconds
    #[serde(default = "default_reconnect_delay")]
    pub reconnect_delay_ms: u64,

    /// Maximum reconnect attempts (0 = infinite)
    #[serde(default)]
    pub max_reconnect_attempts: u32,

    /// Message format for state updates
    #[serde(default)]
    pub state_format: Option<String>,

    /// Interval for state updates in milliseconds (0 = disabled)
    #[serde(default)]
    pub state_interval_ms: u64,

    /// Send binary messages instead of text
    #[serde(default)]
    pub binary: bool,
}

fn default_true() -> bool {
    true
}

fn default_reconnect_delay() -> u64 {
    1000
}

/// HTTP configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// Base URL for HTTP requests
    pub base_url: String,

    /// Default headers
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Request timeout in milliseconds
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    5000
}

/// Button mappings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ButtonMappings {
    // Face buttons
    #[serde(default)]
    pub cross: Option<ActionConfig>,
    #[serde(default)]
    pub circle: Option<ActionConfig>,
    #[serde(default)]
    pub square: Option<ActionConfig>,
    #[serde(default)]
    pub triangle: Option<ActionConfig>,

    // D-pad
    #[serde(default)]
    pub dpad_up: Option<ActionConfig>,
    #[serde(default)]
    pub dpad_down: Option<ActionConfig>,
    #[serde(default)]
    pub dpad_left: Option<ActionConfig>,
    #[serde(default)]
    pub dpad_right: Option<ActionConfig>,

    // Shoulder buttons
    #[serde(default)]
    pub l1: Option<ActionConfig>,
    #[serde(default)]
    pub r1: Option<ActionConfig>,
    #[serde(default)]
    pub l2_button: Option<ActionConfig>,
    #[serde(default)]
    pub r2_button: Option<ActionConfig>,

    // Stick buttons
    #[serde(default)]
    pub l3: Option<ActionConfig>,
    #[serde(default)]
    pub r3: Option<ActionConfig>,

    // System buttons
    #[serde(default)]
    pub options: Option<ActionConfig>,
    #[serde(default)]
    pub create: Option<ActionConfig>,
    #[serde(default)]
    pub ps: Option<ActionConfig>,
    #[serde(default)]
    pub touchpad: Option<ActionConfig>,
    #[serde(default)]
    pub mute: Option<ActionConfig>,
}

/// Analog input mappings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalogMappings {
    #[serde(default)]
    pub left_stick: Option<StickMapping>,
    #[serde(default)]
    pub right_stick: Option<StickMapping>,
    #[serde(default)]
    pub l2_trigger: Option<TriggerMapping>,
    #[serde(default)]
    pub r2_trigger: Option<TriggerMapping>,
}

/// Stick mapping configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickMapping {
    /// Action when stick is moved (continuous)
    #[serde(default)]
    pub on_move: Option<ActionConfig>,

    /// Action when stick crosses threshold in positive X
    #[serde(default)]
    pub on_right: Option<ActionConfig>,
    #[serde(default)]
    pub on_left: Option<ActionConfig>,
    #[serde(default)]
    pub on_up: Option<ActionConfig>,
    #[serde(default)]
    pub on_down: Option<ActionConfig>,

    /// Threshold for directional triggers (0.0 - 1.0)
    #[serde(default = "default_threshold")]
    pub threshold: f32,

    /// Update rate limiting in milliseconds
    #[serde(default)]
    pub rate_limit_ms: u64,
}

fn default_threshold() -> f32 {
    0.5
}

/// Trigger mapping configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerMapping {
    /// Action when trigger value changes
    #[serde(default)]
    pub on_change: Option<ActionConfig>,

    /// Action when trigger crosses threshold
    #[serde(default)]
    pub on_press: Option<ActionConfig>,

    /// Threshold for press detection (0.0 - 1.0)
    #[serde(default = "default_threshold")]
    pub threshold: f32,
}

/// Motion/IMU mappings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MotionMappings {
    /// Action on orientation change
    #[serde(default)]
    pub on_orientation_change: Option<ActionConfig>,

    /// Action on significant motion
    #[serde(default)]
    pub on_shake: Option<ActionConfig>,

    /// Shake detection threshold (in G)
    #[serde(default = "default_shake_threshold")]
    pub shake_threshold: f32,

    /// Update rate for orientation in milliseconds
    #[serde(default)]
    pub orientation_rate_ms: u64,
}

fn default_shake_threshold() -> f32 {
    2.0
}

/// Action configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionConfig {
    /// When to trigger: "press", "release", "hold", "change"
    #[serde(default = "default_trigger")]
    pub trigger: String,

    /// Shell command to execute (supports templates)
    #[serde(default)]
    pub command: Option<String>,

    /// WebSocket message to send (supports templates)
    #[serde(default)]
    pub websocket: Option<WebSocketMessage>,

    /// HTTP request to make
    #[serde(default)]
    pub http: Option<HttpRequest>,

    /// Rumble feedback
    #[serde(default)]
    pub rumble: Option<RumbleConfig>,

    /// LED feedback
    #[serde(default)]
    pub led: Option<LedColorConfig>,

    /// Minimum interval between triggers (debounce) in ms
    #[serde(default)]
    pub debounce_ms: u64,

    /// Only trigger if button held for this duration (ms)
    #[serde(default)]
    pub hold_time_ms: u64,
}

fn default_trigger() -> String {
    "press".to_string()
}

/// WebSocket message configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketMessage {
    /// Message content (template string)
    pub message: String,

    /// Send as binary
    #[serde(default)]
    pub binary: bool,
}

/// HTTP request configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    /// HTTP method
    #[serde(default = "default_method")]
    pub method: String,

    /// Path (appended to base_url)
    pub path: String,

    /// Request body (template string)
    #[serde(default)]
    pub body: Option<String>,

    /// Additional headers
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_method() -> String {
    "POST".to_string()
}

/// Rumble configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RumbleConfig {
    /// Left motor intensity (0-255)
    pub left: u8,
    /// Right motor intensity (0-255)
    pub right: u8,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// LED color configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedColorConfig {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// LED configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LedConfig {
    /// Default/idle color
    #[serde(default)]
    pub default_color: Option<LedColorConfig>,

    /// Color when connected
    #[serde(default)]
    pub connected_color: Option<LedColorConfig>,

    /// Color on error
    #[serde(default)]
    pub error_color: Option<LedColorConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "Default Configuration".to_string(),
            poll_rate: default_poll_rate(),
            deadzone: default_deadzone(),
            shell: ShellConfig::default(),
            websocket: None,
            http: None,
            buttons: ButtonMappings::default(),
            analog: AnalogMappings::default(),
            motion: MotionMappings::default(),
            led: LedConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from a JSON file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        Ok(config)
    }

    /// Load configuration from a directory (merges all JSON files)
    pub fn load_dir<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let mut config = Config::default();

        if !path.is_dir() {
            return Self::load(path);
        }

        // Look for main config file
        let main_config = path.join("config.json");
        if main_config.exists() {
            config = Self::load(&main_config)?;
        }

        Ok(config)
    }

    /// Save configuration to a JSON file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

/// Template context for action commands
#[derive(Debug, Clone, Serialize)]
pub struct TemplateContext {
    // Button states
    pub cross: bool,
    pub circle: bool,
    pub square: bool,
    pub triangle: bool,
    pub l1: bool,
    pub r1: bool,
    pub l2_button: bool,
    pub r2_button: bool,
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,

    // Analog values (normalized -1.0 to 1.0 for sticks, 0.0 to 1.0 for triggers)
    pub left_stick_x: f32,
    pub left_stick_y: f32,
    pub right_stick_x: f32,
    pub right_stick_y: f32,
    pub l2_trigger: f32,
    pub r2_trigger: f32,

    // Orientation (quaternion)
    pub quat_w: f32,
    pub quat_x: f32,
    pub quat_y: f32,
    pub quat_z: f32,

    // Euler angles (radians)
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,

    // Gyroscope (rad/s)
    pub gyro_x: f32,
    pub gyro_y: f32,
    pub gyro_z: f32,

    // Accelerometer (G)
    pub accel_x: f32,
    pub accel_y: f32,
    pub accel_z: f32,

    // Battery
    pub battery_percent: u8,
    pub battery_charging: bool,

    // Touchpad
    pub touch1_active: bool,
    pub touch1_x: u16,
    pub touch1_y: u16,
    pub touch2_active: bool,
    pub touch2_x: u16,
    pub touch2_y: u16,

    // Timestamp
    pub timestamp: u32,
}

impl From<&crate::dualsense::ControllerState> for TemplateContext {
    fn from(state: &crate::dualsense::ControllerState) -> Self {
        let (lx, ly) = state.left_stick.normalized();
        let (rx, ry) = state.right_stick.normalized();
        let (l2, r2) = state.triggers.normalized();
        let gyro = state.gyroscope.to_rad_per_sec();
        let accel = state.accelerometer.to_g();
        let (roll, pitch, yaw) = state.euler_angles();
        let q = state.orientation.quaternion();

        Self {
            cross: state.buttons.cross,
            circle: state.buttons.circle,
            square: state.buttons.square,
            triangle: state.buttons.triangle,
            l1: state.buttons.l1,
            r1: state.buttons.r1,
            l2_button: state.buttons.l2_button,
            r2_button: state.buttons.r2_button,
            dpad_up: state.buttons.dpad_up,
            dpad_down: state.buttons.dpad_down,
            dpad_left: state.buttons.dpad_left,
            dpad_right: state.buttons.dpad_right,

            left_stick_x: lx,
            left_stick_y: ly,
            right_stick_x: rx,
            right_stick_y: ry,
            l2_trigger: l2,
            r2_trigger: r2,

            quat_w: q.w,
            quat_x: q.i,
            quat_y: q.j,
            quat_z: q.k,

            roll,
            pitch,
            yaw,

            gyro_x: gyro.x,
            gyro_y: gyro.y,
            gyro_z: gyro.z,

            accel_x: accel.x,
            accel_y: accel.y,
            accel_z: accel.z,

            battery_percent: state.battery.percentage(),
            battery_charging: state.battery.charging,

            touch1_active: state.touchpad.finger1.active,
            touch1_x: state.touchpad.finger1.x,
            touch1_y: state.touchpad.finger1.y,
            touch2_active: state.touchpad.finger2.active,
            touch2_x: state.touchpad.finger2.x,
            touch2_y: state.touchpad.finger2.y,

            timestamp: state.timestamp,
        }
    }
}
