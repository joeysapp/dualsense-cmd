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

    /// Spatial integration settings
    #[serde(default)]
    pub integration: Option<IntegrationConfig>,
}

/// Spatial integration configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationConfig {
    /// Velocity curve: "linear", "quadratic", "cubic"
    #[serde(default = "default_velocity_curve")]
    pub velocity_curve: String,

    /// Maximum linear speed in mm/s
    #[serde(default = "default_max_linear_speed")]
    pub max_linear_speed: f32,

    /// Maximum angular speed in rad/s
    #[serde(default = "default_max_angular_speed")]
    pub max_angular_speed: f32,

    /// Linear velocity damping (0.0-1.0)
    #[serde(default = "default_linear_damping")]
    pub linear_damping: f32,

    /// Angular velocity damping (0.0-1.0)
    #[serde(default = "default_angular_damping")]
    pub angular_damping: f32,

    /// Low-pass filter smoothing alpha (0.0-1.0)
    #[serde(default = "default_smoothing_alpha")]
    pub smoothing_alpha: f32,

    /// Position units (for documentation)
    #[serde(default = "default_position_units")]
    pub position_units: String,

    /// Orientation filter settings
    #[serde(default)]
    pub orientation_filter: Option<OrientationFilterConfig>,
}

/// Orientation filter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrientationFilterConfig {
    /// Filter type: "complementary", "madgwick"
    #[serde(default = "default_filter_type")]
    pub r#type: String,

    /// Gyro weight for complementary filter (0.0-1.0)
    #[serde(default = "default_gyro_weight")]
    pub gyro_weight: f32,
}

fn default_velocity_curve() -> String {
    "linear".to_string()
}

fn default_max_linear_speed() -> f32 {
    200.0
}

fn default_max_angular_speed() -> f32 {
    6.0
}

fn default_linear_damping() -> f32 {
    0.92
}

fn default_angular_damping() -> f32 {
    0.96
}

fn default_smoothing_alpha() -> f32 {
    0.15
}

fn default_position_units() -> String {
    "mm".to_string()
}

fn default_filter_type() -> String {
    "complementary".to_string()
}

fn default_gyro_weight() -> f32 {
    0.98
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
            integration: None,
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

    // Orientation (quaternion) - from spatial integration
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

    // === Spatial state (integrated) ===

    // Position in mm
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,

    // Velocity in mm/s
    pub vel_x: f32,
    pub vel_y: f32,
    pub vel_z: f32,

    // Angular velocity in rad/s (from spatial state)
    pub angvel_x: f32,
    pub angvel_y: f32,
    pub angvel_z: f32,

    // Linear acceleration in G (from spatial state)
    pub linacc_x: f32,
    pub linacc_y: f32,
    pub linacc_z: f32,

    // Buttons as JSON string for WebSocket messages
    pub buttons_json: String,
}

impl From<&crate::dualsense::ControllerState> for TemplateContext {
    fn from(state: &crate::dualsense::ControllerState) -> Self {
        Self::from_controller(state, None)
    }
}

impl TemplateContext {
    /// Create a template context from controller state and optional spatial state
    pub fn from_controller(
        state: &crate::dualsense::ControllerState,
        spatial: Option<&crate::spatial::SpatialState>,
    ) -> Self {
        let (lx, ly) = state.left_stick.normalized();
        let (rx, ry) = state.right_stick.normalized();
        let (l2, r2) = state.triggers.normalized();
        let gyro = state.gyroscope.to_rad_per_sec();
        let accel = state.accelerometer.to_g();

        // Use spatial orientation if available, otherwise fall back to controller's
        let (quat_w, quat_x, quat_y, quat_z, roll, pitch, yaw) = if let Some(spatial) = spatial {
            let q = spatial.orientation();
            // Convert spatial-core quaternion to euler angles
            // Using ZYX convention (yaw-pitch-roll)
            let sinr_cosp = 2.0 * (q.w * q.x + q.y * q.z);
            let cosr_cosp = 1.0 - 2.0 * (q.x * q.x + q.y * q.y);
            let roll = sinr_cosp.atan2(cosr_cosp);

            let sinp = 2.0 * (q.w * q.y - q.z * q.x);
            let pitch = if sinp.abs() >= 1.0 {
                std::f32::consts::FRAC_PI_2.copysign(sinp)
            } else {
                sinp.asin()
            };

            let siny_cosp = 2.0 * (q.w * q.z + q.x * q.y);
            let cosy_cosp = 1.0 - 2.0 * (q.y * q.y + q.z * q.z);
            let yaw = siny_cosp.atan2(cosy_cosp);

            (q.w, q.x, q.y, q.z, roll, pitch, yaw)
        } else {
            let (roll, pitch, yaw) = state.euler_angles();
            let q = state.orientation.quaternion();
            (q.w, q.i, q.j, q.k, roll, pitch, yaw)
        };

        // Spatial state values (zeros if not available)
        let (pos_x, pos_y, pos_z) = spatial
            .map(|s| (s.position[0], s.position[1], s.position[2]))
            .unwrap_or((0.0, 0.0, 0.0));

        let (vel_x, vel_y, vel_z) = spatial
            .map(|s| {
                let v = s.smoothed_velocity();
                (v[0], v[1], v[2])
            })
            .unwrap_or((0.0, 0.0, 0.0));

        let (angvel_x, angvel_y, angvel_z) = spatial
            .map(|s| (s.angular_velocity[0], s.angular_velocity[1], s.angular_velocity[2]))
            .unwrap_or((gyro.x, gyro.y, gyro.z));

        let (linacc_x, linacc_y, linacc_z) = spatial
            .map(|s| (s.linear_accel[0], s.linear_accel[1], s.linear_accel[2]))
            .unwrap_or((accel.x, accel.y, accel.z));

        // Build buttons JSON
        let buttons_json = serde_json::json!({
            "cross": state.buttons.cross,
            "circle": state.buttons.circle,
            "square": state.buttons.square,
            "triangle": state.buttons.triangle,
            "l1": state.buttons.l1,
            "r1": state.buttons.r1,
            "l2": state.buttons.l2_button,
            "r2": state.buttons.r2_button,
            "dpad_up": state.buttons.dpad_up,
            "dpad_down": state.buttons.dpad_down,
            "dpad_left": state.buttons.dpad_left,
            "dpad_right": state.buttons.dpad_right,
            "l3": state.buttons.l3,
            "r3": state.buttons.r3,
            "options": state.buttons.options,
            "create": state.buttons.create,
            "ps": state.buttons.ps,
            "touchpad": state.buttons.touchpad,
            "mute": state.buttons.mute
        })
        .to_string();

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

            quat_w,
            quat_x,
            quat_y,
            quat_z,

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

            // Spatial state
            pos_x,
            pos_y,
            pos_z,
            vel_x,
            vel_y,
            vel_z,
            angvel_x,
            angvel_y,
            angvel_z,
            linacc_x,
            linacc_y,
            linacc_z,
            buttons_json,
        }
    }
}
