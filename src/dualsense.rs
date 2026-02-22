//! DualSense controller communication layer
//!
//! Handles HID communication with the PlayStation DualSense controller,
//! parsing input reports and managing controller state.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use hidapi::{HidApi, HidDevice};
use nalgebra::{Quaternion, UnitQuaternion, Vector3};
use thiserror::Error;
use tracing::{debug, info, trace};

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
    pub level: u8,       // 0-10
    pub charging: bool,
    pub fully_charged: bool,
}

impl Battery {
    pub fn percentage(&self) -> u8 {
        (self.level * 10).min(100)
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

/// DualSense controller connection
pub struct DualSense {
    device: HidDevice,
    connection_type: ConnectionType,
    state: ControllerState,
    prev_state: ControllerState,
    orientation_filter: MadgwickFilter,
    last_update: Instant,
    running: Arc<AtomicBool>,
    // Cached output state
    led_state: std::sync::Mutex<(u8, u8, u8)>,
    rumble_state: std::sync::Mutex<(u8, u8)>,
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

        let product_name = device_info
            .product_string()
            .unwrap_or("DualSense");
        let serial = device_info
            .serial_number()
            .unwrap_or("unknown");

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
            led_state: std::sync::Mutex::new((255, 255, 255)), // Default white
            rumble_state: std::sync::Mutex::new((0, 0)),
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

    /// Set controller LEDs (color)
    pub fn set_led_color(&self, r: u8, g: u8, b: u8) -> Result<(), DualSenseError> {
        {
            let mut led = self.led_state.lock().unwrap();
            *led = (r, g, b);
        }
        self.send_output_report()
    }

    /// Set controller rumble
    pub fn set_rumble(&self, left: u8, right: u8) -> Result<(), DualSenseError> {
        {
            let mut rumble = self.rumble_state.lock().unwrap();
            *rumble = (left, right);
        }
        self.send_output_report()
    }

    /// Internal helper to send output reports
    fn send_output_report(&self) -> Result<(), DualSenseError> {
        let (r, g, b) = *self.led_state.lock().unwrap();
        let (left, right) = *self.rumble_state.lock().unwrap();
        
        match self.connection_type {
            ConnectionType::Usb => {
                let mut report = [0u8; 48];
                report[0] = 0x02; // Output report ID
                
                // Flags: 0x01 (Rumble) | 0x02 (Haptics) | 0x04 (Lightbar)
                report[1] = 0x01 | 0x02 | 0x04 | 0x10 | 0x40;
                
                report[3] = left;
                report[4] = right;
                
                report[45] = r;
                report[46] = g;
                report[47] = b;
                
                self.device.write(&report)?;
            }
            ConnectionType::Bluetooth => {
                let mut report = [0u8; 78];
                report[0] = 0x31; // BT output report ID
                report[1] = 0x02; // Seq/Tag
                
                // Flags: 0x01 (Rumble) | 0x02 (Haptics) | 0x04 (Lightbar)
                report[2] = 0x01 | 0x02 | 0x04 | 0x10 | 0x40;
                
                report[4] = left;
                report[5] = right;
                
                report[46] = r;
                report[47] = g;
                report[48] = b;
                
                self.device.write(&report)?;
            }
        }
        Ok(())
    }

    /// Explicitly close the device connection
    pub fn close(&mut self) {
        // Reset LED to white and stop rumble before closing
        let _ = self.set_rumble(0, 0);
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

    fn update(
        &mut self,
        gyro: Vector3<f32>,
        accel: Vector3<f32>,
        dt: f32,
    ) -> UnitQuaternion<f32> {
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

        let grad_norm = (grad_w * grad_w + grad_x * grad_x + grad_y * grad_y + grad_z * grad_z).sqrt();

        let (grad_w, grad_x, grad_y, grad_z) = if grad_norm > 0.0 {
            (grad_w / grad_norm, grad_x / grad_norm, grad_y / grad_norm, grad_z / grad_norm)
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
