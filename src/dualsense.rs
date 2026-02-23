//! DualSense controller communication layer
//!
//! Handles HID communication with the PlayStation DualSense controller,
//! parsing input reports and managing controller state.
//!
//! ## Protocol Notes
//!
//! ### Input (Receiving from Controller)
//! - **Implemented**: Thumbsticks, action buttons, D-pad, bumpers, triggers, stick buttons,
//!   Create/Options/PS/Mute buttons, touchpad (click + multitouch), accelerometer, gyroscope, battery
//! - **Future**: Microphone input, headset jack input
//!
//! ### Output (Sending to Controller)
//! - **Implemented but not tested**: Haptic feedback (rumble motors), Light bar (RGB LED), Player LEDs
//! - **Implemented but not tested**: Adaptive triggers (resistance/vibration effects)
//! - **Future**: Speaker output, headset jack output
//!
//! ### Connection Types
//! - **USB**: Direct HID, no authentication required
//! - **Bluetooth**: Requires CRC32 checksum on output reports - seems to not be applying saves correctly

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crc32fast::Hasher;
use hidapi::{HidApi, HidDevice};
use nalgebra::{Quaternion, UnitQuaternion, Vector3};
use thiserror::Error;
use tracing::{debug, info, trace, warn};

/// Sony vendor ID
pub const SONY_VENDOR_ID: u16 = 0x054C;
/// DualSense product ID
pub const DUALSENSE_PRODUCT_ID: u16 = 0x0CE6;
/// DualSense Edge product ID
pub const DUALSENSE_EDGE_PRODUCT_ID: u16 = 0x0DF2;

/// Report sizes
pub const USB_REPORT_SIZE: usize = 64;
pub const BT_REPORT_SIZE: usize = 78;

/// Input report IDs
pub const USB_INPUT_REPORT_ID: u8 = 0x01;
pub const BT_INPUT_REPORT_ID: u8 = 0x31;

#[derive(Error, Debug)]
pub enum DualSenseError {
    #[error("HID API error: {0}")]
    HidApi(#[from] hidapi::HidError),

    #[error("No DualSense controller found")]
    NotFound,

    #[error("Invalid report received: {0}")]
    InvalidReport(String),

    #[error("Connection lost")]
    ConnectionLost,

    #[error("Read timeout")]
    Timeout,
}

/// DualSense button state
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Buttons {
    // Face buttons
    pub cross: bool,
    pub circle: bool,
    pub square: bool,
    pub triangle: bool,

    // D-pad
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,

    // Shoulder buttons
    pub l1: bool,
    pub r1: bool,
    pub l2_button: bool,
    pub r2_button: bool,

    // Stick buttons
    pub l3: bool,
    pub r3: bool,

    // System buttons
    pub options: bool,
    pub create: bool,
    pub ps: bool,
    pub touchpad: bool,
    pub mute: bool,
}

/// Analog stick state (0-255, center at 128)
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Stick {
    pub x: u8,
    pub y: u8,
}

impl Stick {
    /// Get normalized values (-1.0 to 1.0)
    pub fn normalized(&self) -> (f32, f32) {
        let x = (self.x as f32 - 128.0) / 127.0;
        let y = (self.y as f32 - 128.0) / 127.0;
        (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0))
    }

    /// Get normalized values with deadzone applied
    pub fn normalized_with_deadzone(&self, deadzone: f32) -> (f32, f32) {
        let (x, y) = self.normalized();
        let magnitude = (x * x + y * y).sqrt();
        if magnitude < deadzone {
            (0.0, 0.0)
        } else {
            let scale = (magnitude - deadzone) / (1.0 - deadzone) / magnitude;
            (x * scale, y * scale)
        }
    }
}

/// Trigger state (0-255)
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Triggers {
    pub l2: u8,
    pub r2: u8,
}

impl Triggers {
    /// Get normalized values (0.0 to 1.0)
    pub fn normalized(&self) -> (f32, f32) {
        (self.l2 as f32 / 255.0, self.r2 as f32 / 255.0)
    }
}

/// Touchpad finger state
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct TouchFinger {
    pub active: bool,
    pub id: u8,
    pub x: u16,
    pub y: u16,
}

/// Touchpad state (supports 2 fingers)
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Touchpad {
    pub finger1: TouchFinger,
    pub finger2: TouchFinger,
}

/// Raw gyroscope data
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Gyroscope {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

impl Gyroscope {
    /// Convert to radians per second (approximate calibration)
    pub fn to_rad_per_sec(&self) -> Vector3<f32> {
        // DualSense gyro scale factor (approximate)
        const SCALE: f32 = 1.0 / 1024.0;
        Vector3::new(
            self.x as f32 * SCALE,
            self.y as f32 * SCALE,
            self.z as f32 * SCALE,
        )
    }
}

/// Raw accelerometer data
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Accelerometer {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

impl Accelerometer {
    /// Convert to G-force units (approximate calibration)
    pub fn to_g(&self) -> Vector3<f32> {
        // DualSense accelerometer scale factor (approximate)
        const SCALE: f32 = 1.0 / 8192.0;
        Vector3::new(
            self.x as f32 * SCALE,
            self.y as f32 * SCALE,
            self.z as f32 * SCALE,
        )
    }
}

/// Battery status
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Battery {
    pub level: u8, // 0-10
    pub charging: bool,
    pub fully_charged: bool,
}

impl Battery {
    pub fn percentage(&self) -> u8 {
        (self.level * 10).min(100)
    }
}

/// Adaptive trigger effect mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TriggerEffectMode {
    /// No effect (trigger operates normally)
    #[default]
    Off = 0x00,
    /// Continuous resistance throughout pull
    Continuous = 0x01,
    /// Resistance within a specific range
    SectionResistance = 0x02,
    /// Vibration effect
    Vibration = 0x06,
    /// Combined resistance and vibration (recommended)
    CombinedRV = 0x26,
    /// Calibration mode
    Calibration = 0xFC,
}

impl TriggerEffectMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => TriggerEffectMode::Off,
            0x01 => TriggerEffectMode::Continuous,
            0x02 => TriggerEffectMode::SectionResistance,
            0x06 => TriggerEffectMode::Vibration,
            0x26 => TriggerEffectMode::CombinedRV,
            0xFC => TriggerEffectMode::Calibration,
            _ => TriggerEffectMode::Off,
        }
    }
}

/// Adaptive trigger effect configuration
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TriggerEffect {
    /// Effect mode
    pub mode: TriggerEffectMode,
    /// Start position (0-255, where 0 is released)
    pub start_position: u8,
    /// End position (0-255, where 255 is fully pressed)
    pub end_position: u8,
    /// Force/strength of the effect (0-255)
    pub force: u8,
    /// Frequency for vibration effects (Hz, 0-255)
    pub frequency: u8,
}

impl Default for TriggerEffect {
    fn default() -> Self {
        Self {
            mode: TriggerEffectMode::Off,
            start_position: 0,
            end_position: 255,
            force: 0,
            frequency: 0,
        }
    }
}

impl TriggerEffect {
    /// Create a continuous resistance effect
    pub fn continuous(force: u8) -> Self {
        Self {
            mode: TriggerEffectMode::Continuous,
            start_position: 0,
            end_position: 255,
            force,
            frequency: 0,
        }
    }

    /// Create a section resistance effect
    pub fn section(start: u8, end: u8, force: u8) -> Self {
        Self {
            mode: TriggerEffectMode::SectionResistance,
            start_position: start,
            end_position: end,
            force,
            frequency: 0,
        }
    }

    /// Create a vibration effect
    pub fn vibration(start: u8, frequency: u8, force: u8) -> Self {
        Self {
            mode: TriggerEffectMode::Vibration,
            start_position: start,
            end_position: 255,
            force,
            frequency,
        }
    }

    /// Create a weapon-like effect (resistance at a point with click)
    pub fn weapon(start: u8, end: u8, force: u8) -> Self {
        Self {
            mode: TriggerEffectMode::SectionResistance,
            start_position: start,
            end_position: end,
            force,
            frequency: 0,
        }
    }

    /// Create a bow-draw effect (increasing resistance)
    pub fn bow(force: u8) -> Self {
        Self {
            mode: TriggerEffectMode::Continuous,
            start_position: 30,
            end_position: 200,
            force,
            frequency: 0,
        }
    }

    /// Convert to bytes for output report
    pub fn to_bytes(&self) -> [u8; 11] {
        let mut bytes = [0u8; 11];
        bytes[0] = self.mode as u8;

        match self.mode {
            TriggerEffectMode::Off => {
                // All zeros
            }
            TriggerEffectMode::Continuous => {
                bytes[1] = self.start_position;
                bytes[2] = self.force;
            }
            TriggerEffectMode::SectionResistance => {
                bytes[1] = self.start_position;
                bytes[2] = self.end_position;
                bytes[3] = self.force;
            }
            TriggerEffectMode::Vibration => {
                bytes[1] = self.start_position;
                bytes[2] = self.force; // strength
                bytes[3] = self.frequency; // frequency
            }
            TriggerEffectMode::CombinedRV => {
                bytes[1] = self.start_position;
                bytes[2] = self.end_position;
                bytes[3] = self.force; // force in resistance zone
                bytes[4] = self.force / 2; // strength near release
                bytes[5] = self.force / 2; // strength near middle
                bytes[6] = self.force; // strength at pressed
                bytes[9] = self.frequency; // vibration frequency
            }
            TriggerEffectMode::Calibration => {
                // Special calibration mode
            }
        }

        bytes
    }
}

/// Player LED configuration (5 LEDs below touchpad)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlayerLeds {
    pub led1: bool,
    pub led2: bool,
    pub led3: bool,
    pub led4: bool,
    pub led5: bool,
}

impl PlayerLeds {
    /// Create from player number (1-5)
    pub fn from_player(player: u8) -> Self {
        match player {
            1 => Self {
                led3: true,
                ..Default::default()
            },
            2 => Self {
                led2: true,
                led4: true,
                ..Default::default()
            },
            3 => Self {
                led1: true,
                led3: true,
                led5: true,
                ..Default::default()
            },
            4 => Self {
                led1: true,
                led2: true,
                led4: true,
                led5: true,
                ..Default::default()
            },
            5 => Self {
                led1: true,
                led2: true,
                led3: true,
                led4: true,
                led5: true,
            },
            _ => Self::default(),
        }
    }

    /// All LEDs on
    pub fn all() -> Self {
        Self {
            led1: true,
            led2: true,
            led3: true,
            led4: true,
            led5: true,
        }
    }

    /// Convert to byte value
    pub fn to_byte(&self) -> u8 {
        let mut byte = 0u8;
        if self.led1 {
            byte |= 0x01;
        }
        if self.led2 {
            byte |= 0x02;
        }
        if self.led3 {
            byte |= 0x04;
        }
        if self.led4 {
            byte |= 0x08;
        }
        if self.led5 {
            byte |= 0x10;
        }
        byte
    }
}

/// Mute LED state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MuteLedState {
    #[default]
    Off,
    On,
    Breathing,
}

impl MuteLedState {
    pub fn to_byte(&self) -> u8 {
        match self {
            MuteLedState::Off => 0,
            MuteLedState::On => 1,
            MuteLedState::Breathing => 2,
        }
    }
}

/// Complete controller state
#[derive(Debug, Clone, Default, Serialize)]
pub struct ControllerState {
    pub buttons: Buttons,
    pub left_stick: Stick,
    pub right_stick: Stick,
    pub triggers: Triggers,
    pub touchpad: Touchpad,
    pub gyroscope: Gyroscope,
    pub accelerometer: Accelerometer,
    pub battery: Battery,
    pub timestamp: u32,

    // Computed orientation from sensor fusion
    #[serde(skip)]
    pub orientation: UnitQuaternion<f32>,
}

impl ControllerState {
    /// Get orientation as Euler angles (roll, pitch, yaw) in radians
    pub fn euler_angles(&self) -> (f32, f32, f32) {
        self.orientation.euler_angles()
    }
}

/// Connection type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionType {
    Usb,
    Bluetooth,
}

/// Complete output state for the controller
#[derive(Debug, Clone)]
pub struct OutputState {
    /// LED color (R, G, B)
    pub led_color: (u8, u8, u8),
    /// Rumble motors (left, right)
    pub rumble: (u8, u8),
    /// L2 trigger effect
    pub l2_effect: TriggerEffect,
    /// R2 trigger effect
    pub r2_effect: TriggerEffect,
    /// Player LEDs
    pub player_leds: PlayerLeds,
    /// Mute LED state
    pub mute_led: MuteLedState,
    /// Whether lightbar is enabled
    pub lightbar_enabled: bool,
    /// Sequence number for Bluetooth (0-15)
    pub bt_seq: u8,
}

impl Default for OutputState {
    fn default() -> Self {
        Self {
            led_color: (255, 255, 255), // Default white
            rumble: (0, 0),
            l2_effect: TriggerEffect::default(),
            r2_effect: TriggerEffect::default(),
            player_leds: PlayerLeds::default(),
            mute_led: MuteLedState::Off,
            lightbar_enabled: true,
            bt_seq: 0,
        }
    }
}

/// DualSense controller connection
pub struct DualSense {
    device: HidDevice,
    connection_type: ConnectionType,
    state: ControllerState,
    prev_state: ControllerState,
    orientation_filter: MadgwickFilter,
    last_update: Instant,
    running: Arc<AtomicBool>,
    /// Complete output state
    output_state: std::sync::Mutex<OutputState>,
}

impl DualSense {
    /// Find and connect to a DualSense controller
    pub fn find_and_connect() -> Result<Self, DualSenseError> {
        let api = HidApi::new()?;

        // Try to find DualSense or DualSense Edge
        let device_info = api
            .device_list()
            .find(|d| {
                d.vendor_id() == SONY_VENDOR_ID
                    && (d.product_id() == DUALSENSE_PRODUCT_ID
                        || d.product_id() == DUALSENSE_EDGE_PRODUCT_ID)
            })
            .ok_or(DualSenseError::NotFound)?;

        let product_name = device_info.product_string().unwrap_or("DualSense");
        let serial = device_info.serial_number().unwrap_or("unknown");

        info!(
            "Found {} (serial: {}) via {:?}",
            product_name,
            serial,
            if device_info.interface_number() == -1 {
                "Bluetooth"
            } else {
                "USB"
            }
        );

        let device = device_info.open_device(&api)?;

        // Determine connection type based on interface number
        // USB devices have interface_number >= 0, Bluetooth typically has -1
        let connection_type = if device_info.interface_number() == -1 {
            ConnectionType::Bluetooth
        } else {
            ConnectionType::Usb
        };

        info!("Connected via {:?}", connection_type);

        Ok(Self {
            device,
            connection_type,
            state: ControllerState::default(),
            prev_state: ControllerState::default(),
            orientation_filter: MadgwickFilter::new(0.1),
            last_update: Instant::now(),
            running: Arc::new(AtomicBool::new(true)),
            output_state: std::sync::Mutex::new(OutputState::default()),
        })
    }

    /// Get the running flag for external shutdown control
    pub fn running_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.running)
    }

    /// Stop the controller polling
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Get current controller state
    pub fn state(&self) -> &ControllerState {
        &self.state
    }

    /// Get previous controller state (for change detection)
    pub fn prev_state(&self) -> &ControllerState {
        &self.prev_state
    }

    /// Get connection type
    pub fn connection_type(&self) -> ConnectionType {
        self.connection_type
    }

    /// Read and parse the next input report
    pub fn poll(&mut self, timeout_ms: i32) -> Result<&ControllerState, DualSenseError> {
        let mut buf = [0u8; BT_REPORT_SIZE];

        let bytes_read = self.device.read_timeout(&mut buf, timeout_ms)?;

        if bytes_read == 0 {
            return Err(DualSenseError::Timeout);
        }

        // Store previous state
        self.prev_state = self.state.clone();

        // Parse based on connection type and report ID
        match self.connection_type {
            ConnectionType::Usb => {
                if bytes_read >= USB_REPORT_SIZE && buf[0] == USB_INPUT_REPORT_ID {
                    self.parse_usb_report(&buf[1..])?;
                } else {
                    trace!("Unexpected USB report: id={}, len={}", buf[0], bytes_read);
                }
            }
            ConnectionType::Bluetooth => {
                if bytes_read >= BT_REPORT_SIZE && buf[0] == BT_INPUT_REPORT_ID {
                    self.parse_bt_report(&buf[1..])?;
                } else {
                    trace!("Unexpected BT report: id={}, len={}", buf[0], bytes_read);
                }
            }
        }

        // Update orientation using sensor fusion
        let now = Instant::now();
        let dt = now.duration_since(self.last_update).as_secs_f32();
        self.last_update = now;

        if dt > 0.0 && dt < 1.0 {
            let gyro = self.state.gyroscope.to_rad_per_sec();
            let accel = self.state.accelerometer.to_g();
            self.state.orientation = self.orientation_filter.update(gyro, accel, dt);
        }

        Ok(&self.state)
    }

    /// Parse USB input report (offset by 1 byte for report ID)
    fn parse_usb_report(&mut self, data: &[u8]) -> Result<(), DualSenseError> {
        if data.len() < 63 {
            return Err(DualSenseError::InvalidReport(format!(
                "USB report too short: {} bytes",
                data.len()
            )));
        }

        self.parse_common_input(data, 0)
    }

    /// Parse Bluetooth input report
    fn parse_bt_report(&mut self, data: &[u8]) -> Result<(), DualSenseError> {
        if data.len() < 77 {
            return Err(DualSenseError::InvalidReport(format!(
                "BT report too short: {} bytes",
                data.len()
            )));
        }

        // Bluetooth reports have a 1-byte offset for the feature flags
        self.parse_common_input(data, 1)
    }

    /// Parse common input data (shared between USB and BT)
    fn parse_common_input(&mut self, data: &[u8], offset: usize) -> Result<(), DualSenseError> {
        let d = &data[offset..];

        // Sticks (bytes 0-3)
        self.state.left_stick = Stick { x: d[0], y: d[1] };
        self.state.right_stick = Stick { x: d[2], y: d[3] };

        // Triggers (bytes 4-5)
        self.state.triggers = Triggers { l2: d[4], r2: d[5] };

        // Timestamp (byte 6, or counter)
        self.state.timestamp = d[6] as u32;

        // Buttons (bytes 7-9)
        let btns1 = d[7];
        let btns2 = d[8];
        let btns3 = d[9];

        // D-pad is encoded in lower 4 bits of btns1
        let dpad = btns1 & 0x0F;
        self.state.buttons.dpad_up = matches!(dpad, 0 | 1 | 7);
        self.state.buttons.dpad_right = matches!(dpad, 1 | 2 | 3);
        self.state.buttons.dpad_down = matches!(dpad, 3 | 4 | 5);
        self.state.buttons.dpad_left = matches!(dpad, 5 | 6 | 7);

        // Face buttons (upper 4 bits of btns1)
        self.state.buttons.square = (btns1 & 0x10) != 0;
        self.state.buttons.cross = (btns1 & 0x20) != 0;
        self.state.buttons.circle = (btns1 & 0x40) != 0;
        self.state.buttons.triangle = (btns1 & 0x80) != 0;

        // Shoulder buttons and sticks (btns2)
        self.state.buttons.l1 = (btns2 & 0x01) != 0;
        self.state.buttons.r1 = (btns2 & 0x02) != 0;
        self.state.buttons.l2_button = (btns2 & 0x04) != 0;
        self.state.buttons.r2_button = (btns2 & 0x08) != 0;
        self.state.buttons.create = (btns2 & 0x10) != 0;
        self.state.buttons.options = (btns2 & 0x20) != 0;
        self.state.buttons.l3 = (btns2 & 0x40) != 0;
        self.state.buttons.r3 = (btns2 & 0x80) != 0;

        // System buttons (btns3)
        self.state.buttons.ps = (btns3 & 0x01) != 0;
        self.state.buttons.touchpad = (btns3 & 0x02) != 0;
        self.state.buttons.mute = (btns3 & 0x04) != 0;

        // Gyroscope (bytes 15-20, little-endian i16)
        self.state.gyroscope = Gyroscope {
            x: i16::from_le_bytes([d[15], d[16]]),
            y: i16::from_le_bytes([d[17], d[18]]),
            z: i16::from_le_bytes([d[19], d[20]]),
        };

        // Accelerometer (bytes 21-26, little-endian i16)
        self.state.accelerometer = Accelerometer {
            x: i16::from_le_bytes([d[21], d[22]]),
            y: i16::from_le_bytes([d[23], d[24]]),
            z: i16::from_le_bytes([d[25], d[26]]),
        };

        // Touchpad (bytes 32-40)
        // Each touch point: 4 bytes
        // Byte 0: id (7 bits) + inactive flag (1 bit)
        // Bytes 1-3: x (12 bits) + y (12 bits)
        if d.len() > 40 {
            self.state.touchpad.finger1 = Self::parse_touch_point(&d[32..36]);
            self.state.touchpad.finger2 = Self::parse_touch_point(&d[36..40]);
        }

        // Battery (byte 52)
        if d.len() > 52 {
            let battery_byte = d[52];
            self.state.battery = Battery {
                level: battery_byte & 0x0F,
                charging: (battery_byte & 0x10) != 0,
                fully_charged: (battery_byte & 0x20) != 0,
            };
        }

        Ok(())
    }

    fn parse_touch_point(data: &[u8]) -> TouchFinger {
        TouchFinger {
            active: (data[0] & 0x80) == 0,
            id: data[0] & 0x7F,
            x: ((data[2] & 0x0F) as u16) << 8 | data[1] as u16,
            y: (data[3] as u16) << 4 | ((data[2] & 0xF0) >> 4) as u16,
        }
    }
    /// [TODO] Doesn't seem to work on macOS
    /// Set controller LEDs (color)
    pub fn set_led_color(&self, r: u8, g: u8, b: u8) -> Result<(), DualSenseError> {
        {
            let mut output = self.output_state.lock().unwrap();
            output.led_color = (r, g, b);
            output.lightbar_enabled = true;
        }
        self.send_output_report()
    }
    /// [TODO] Doesn't seem to work on macOS
    /// Set controller rumble
    pub fn set_rumble(&self, left: u8, right: u8) -> Result<(), DualSenseError> {
        {
            let mut output = self.output_state.lock().unwrap();
            output.rumble = (left, right);
        }
        self.send_output_report()
    }
    /// [TODO] Doesn't seem to work on macOS
    /// Set L2 adaptive trigger effect
    pub fn set_l2_trigger_effect(&self, effect: TriggerEffect) -> Result<(), DualSenseError> {
        {
            let mut output = self.output_state.lock().unwrap();
            output.l2_effect = effect;
        }
        self.send_output_report()
    }
    /// [TODO] Doesn't seem to work on macOS
    /// Set R2 adaptive trigger effect
    pub fn set_r2_trigger_effect(&self, effect: TriggerEffect) -> Result<(), DualSenseError> {
        {
            let mut output = self.output_state.lock().unwrap();
            output.r2_effect = effect;
        }
        self.send_output_report()
    }
    /// [TODO] Doesn't seem to work on macOS
    /// Set both trigger effects at once
    pub fn set_trigger_effects(
        &self,
        l2: TriggerEffect,
        r2: TriggerEffect,
    ) -> Result<(), DualSenseError> {
        {
            let mut output = self.output_state.lock().unwrap();
            output.l2_effect = l2;
            output.r2_effect = r2;
        }
        self.send_output_report()
    }
    /// [TODO] Doesn't seem to work on macOS
    /// Set player LEDs
    pub fn set_player_leds(&self, leds: PlayerLeds) -> Result<(), DualSenseError> {
        {
            let mut output = self.output_state.lock().unwrap();
            output.player_leds = leds;
        }
        self.send_output_report()
    }
    /// [TODO] Doesn't seem to work on macOS
    /// Set player number (1-5) using standard LED patterns
    pub fn set_player_number(&self, player: u8) -> Result<(), DualSenseError> {
        self.set_player_leds(PlayerLeds::from_player(player))
    }
    /// [TODO] Doesn't seem to work on macOS
    /// Set mute LED state
    pub fn set_mute_led(&self, state: MuteLedState) -> Result<(), DualSenseError> {
        {
            let mut output = self.output_state.lock().unwrap();
            output.mute_led = state;
        }
        self.send_output_report()
    }

    /// Apply complete output state at once
    pub fn apply_output_state(&self, new_state: OutputState) -> Result<(), DualSenseError> {
        {
            let mut output = self.output_state.lock().unwrap();
            *output = new_state;
        }
        self.send_output_report()
    }

    /// Get current output state
    pub fn get_output_state(&self) -> OutputState {
        self.output_state.lock().unwrap().clone()
    }

    /// Internal helper to compute CRC32 for Bluetooth reports
    fn compute_bt_crc32(data: &[u8]) -> u32 {
        // Bluetooth CRC32 is computed with seed [0xa2, report_id] prepended
        let mut hasher = Hasher::new();
        hasher.update(&[0xa2, 0x31]); // Prefix for BT output report
        hasher.update(data);
        hasher.finalize()
    }

    /// Internal helper to send output reports
    fn send_output_report(&self) -> Result<(), DualSenseError> {
        let mut output = self.output_state.lock().unwrap();
        let (r, g, b) = output.led_color;
        let (left, right) = output.rumble;
        let l2_effect = output.l2_effect.to_bytes();
        let r2_effect = output.r2_effect.to_bytes();
        let player_leds = output.player_leds.to_byte();
        let mute_led = output.mute_led.to_byte();

        match self.connection_type {
            ConnectionType::Usb => {
                let mut report = [0u8; 48];
                report[0] = 0x02; // Output report ID

                // valid_flag0: bit0=rumble, bit1=haptics_select
                report[1] = 0x03; // Enable rumble/haptics
                                  // valid_flag1: bit0=mic_mute_led, bit1=power_save, bit2=lightbar, bit4=player_led
                report[2] = 0x15; // Enable mic LED, lightbar, player LEDs

                // Rumble motors (bytes 3-4)
                report[3] = right; // Right motor (high frequency)
                report[4] = left; // Left motor (low frequency)

                // Mute LED (byte 9)
                report[9] = mute_led;

                // R2 trigger effect (bytes 11-21)
                report[11..22].copy_from_slice(&r2_effect);

                // L2 trigger effect (bytes 22-32)
                report[22..33].copy_from_slice(&l2_effect);

                // valid_flag2 (byte 39): bit1=lightbar_setup_control
                report[39] = 0x02;

                // Lightbar setup (byte 41): 2=enable, 1=disable
                report[41] = if output.lightbar_enabled { 0x02 } else { 0x01 };

                // Player LEDs (byte 44)
                report[44] = player_leds;

                // Lightbar RGB (bytes 45-47)
                report[45] = r;
                report[46] = g;
                report[47] = b;

                self.device.write(&report)?;
            }
            ConnectionType::Bluetooth => {
                let mut report = [0u8; 78];
                report[0] = 0x31; // BT output report ID

                // Sequence tag (upper nibble) | 0x10 (DS_OUTPUT_TAG)
                report[1] = (output.bt_seq << 4) | 0x02;
                output.bt_seq = (output.bt_seq + 1) & 0x0F;

                // valid_flag0 (byte 2): bit0=rumble, bit1=haptics_select
                report[2] = 0x03;
                // valid_flag1 (byte 3): bit0=mic_mute_led, bit2=lightbar, bit4=player_led
                report[3] = 0x15;

                // Rumble motors (bytes 4-5)
                report[4] = right;
                report[5] = left;

                // Mute LED (byte 10)
                report[10] = mute_led;

                // R2 trigger effect (bytes 12-22)
                report[12..23].copy_from_slice(&r2_effect);

                // L2 trigger effect (bytes 23-33)
                report[23..34].copy_from_slice(&l2_effect);

                // valid_flag2 (byte 40): bit1=lightbar_setup_control
                report[40] = 0x02;

                // Lightbar setup (byte 42)
                report[42] = if output.lightbar_enabled { 0x02 } else { 0x01 };

                // Player LEDs (byte 45)
                report[45] = player_leds;

                // Lightbar RGB (bytes 46-48)
                report[46] = r;
                report[47] = g;
                report[48] = b;

                // [TODO] Is this correct?
                // Compute CRC32 and append to last 4 bytes (74-77)
                let crc = Self::compute_bt_crc32(&report[..74]);
                report[74..78].copy_from_slice(&crc.to_le_bytes());

                match self.device.write(&report) {
                    Ok(_) => {}
                    Err(e) => {
                        warn!(
                            "Bluetooth output failed (controller may need identification): {}",
                            e
                        );
                        return Err(DualSenseError::HidApi(e));
                    }
                }
            }
        }
        Ok(())
    }

    /// Explicitly close the device connection
    pub fn close(&mut self) {
        // Reset to default state before closing
        let _ = self.set_rumble(0, 0);
        let _ = self.set_trigger_effects(TriggerEffect::default(), TriggerEffect::default());
        let _ = self.set_led_color(255, 255, 255);
        self.running.store(false, Ordering::SeqCst);
        // HidDevice will be dropped when self is dropped
        debug!("DualSense connection closed");
    }
}

impl Drop for DualSense {
    fn drop(&mut self) {
        // Ensure clean state on drop
        let _ = self.set_rumble(0, 0);
        let _ = self.set_trigger_effects(TriggerEffect::default(), TriggerEffect::default());
        debug!("DualSense dropped, device released");
    }
}

/// Madgwick AHRS filter for orientation estimation
struct MadgwickFilter {
    q: UnitQuaternion<f32>,
    beta: f32,
}

impl MadgwickFilter {
    fn new(beta: f32) -> Self {
        Self {
            q: UnitQuaternion::identity(),
            beta,
        }
    }

    fn update(&mut self, gyro: Vector3<f32>, accel: Vector3<f32>, dt: f32) -> UnitQuaternion<f32> {
        let q = self.q;

        // Normalize accelerometer
        let accel_norm = accel.norm();
        if accel_norm < 0.01 {
            // If accelerometer magnitude is too small, skip correction
            let gyro_quat = Quaternion::new(0.0, gyro.x, gyro.y, gyro.z);
            let q_dot = q.quaternion() * gyro_quat * 0.5;
            let new_q = Quaternion::new(
                q.w + q_dot.w * dt,
                q.i + q_dot.i * dt,
                q.j + q_dot.j * dt,
                q.k + q_dot.k * dt,
            );
            self.q = UnitQuaternion::from_quaternion(new_q);
            return self.q;
        }

        let a = accel / accel_norm;

        // Gradient descent step
        let f1 = 2.0 * (q.i * q.k - q.w * q.j) - a.x;
        let f2 = 2.0 * (q.w * q.i + q.j * q.k) - a.y;
        let f3 = 2.0 * (0.5 - q.i * q.i - q.j * q.j) - a.z;

        let j11 = -2.0 * q.j;
        let j12 = 2.0 * q.k;
        let j13 = -2.0 * q.w;
        let j14 = 2.0 * q.i;
        let j21 = 2.0 * q.i;
        let j22 = 2.0 * q.w;
        let j23 = 2.0 * q.k;
        let j24 = 2.0 * q.j;
        let j31 = 0.0;
        let j32 = -4.0 * q.i;
        let j33 = -4.0 * q.j;
        let j34 = 0.0;

        let grad_w = j11 * f1 + j21 * f2 + j31 * f3;
        let grad_x = j12 * f1 + j22 * f2 + j32 * f3;
        let grad_y = j13 * f1 + j23 * f2 + j33 * f3;
        let grad_z = j14 * f1 + j24 * f2 + j34 * f3;

        let grad_norm =
            (grad_w * grad_w + grad_x * grad_x + grad_y * grad_y + grad_z * grad_z).sqrt();

        let (grad_w, grad_x, grad_y, grad_z) = if grad_norm > 0.0 {
            (
                grad_w / grad_norm,
                grad_x / grad_norm,
                grad_y / grad_norm,
                grad_z / grad_norm,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };

        // Gyroscope quaternion derivative
        let gyro_quat = Quaternion::new(0.0, gyro.x, gyro.y, gyro.z);
        let q_dot = q.quaternion() * gyro_quat * 0.5;

        // Apply gradient descent correction
        let new_q = Quaternion::new(
            q.w + (q_dot.w - self.beta * grad_w) * dt,
            q.i + (q_dot.i - self.beta * grad_x) * dt,
            q.j + (q_dot.j - self.beta * grad_y) * dt,
            q.k + (q_dot.k - self.beta * grad_z) * dt,
        );

        self.q = UnitQuaternion::from_quaternion(new_q);
        self.q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stick_normalized() {
        let stick = Stick { x: 128, y: 128 };
        let (x, y) = stick.normalized();
        assert!(x.abs() < 0.01);
        assert!(y.abs() < 0.01);

        let stick = Stick { x: 255, y: 0 };
        let (x, y) = stick.normalized();
        assert!((x - 1.0).abs() < 0.01);
        assert!((y + 1.0).abs() < 0.01);
    }

    #[test]
    fn test_trigger_normalized() {
        let triggers = Triggers { l2: 0, r2: 255 };
        let (l, r) = triggers.normalized();
        assert!(l.abs() < 0.01);
        assert!((r - 1.0).abs() < 0.01);
    }
}
