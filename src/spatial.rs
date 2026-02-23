//! Spatial state integration for controller-based positioning
//!
//! Integrates controller inputs (sticks, triggers, IMU) into spatial state
//! (position, velocity, orientation) using configurable physics parameters.

use serde::{Deserialize, Serialize};
use spatial_core::{ComplementaryFilter, Quaternion};

use crate::dualsense::ControllerState;

/// Integration modes for the controller
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SpatialMode {
    /// Standard integration: Left stick X/Y planar, Triggers for Z
    Standard,
    /// Heading-based movement: Rotation by gyro, Triggers for forward/back
    Heading,
    /// Accelerometer-based movement: Position changes based on acceleration
    Accelerometer,
    /// AxiDraw 2D Plotter: Right stick X/Y, Triggers for Pen Z
    AxiDraw,
    /// 3D Tool: Standard 3D navigation (Triggers and Sticks)
    ThreeD,
}

impl Default for SpatialMode {
    fn default() -> Self {
        Self::Standard
    }
}

/// Configuration for spatial integration, parsed from JSON config
#[derive(Debug, Clone)]
pub struct IntegrationConfig {
    /// Velocity curve: "linear", "quadratic", "cubic"
    pub velocity_curve: VelocityCurve,

    /// Maximum linear speed in mm/s
    pub max_linear_speed: f32,

    /// Maximum angular speed in rad/s
    pub max_angular_speed: f32,

    /// Linear velocity damping factor (0.0-1.0), applied per frame
    pub linear_damping: f32,

    /// Angular velocity damping factor (0.0-1.0), applied per frame
    pub angular_damping: f32,

    /// Smoothing alpha for low-pass filter (0.0-1.0)
    pub smoothing_alpha: f32,

    /// Gyro weight for complementary filter (0.0-1.0)
    pub gyro_weight: f32,

    /// Deadzone for stick inputs
    pub deadzone: f32,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            velocity_curve: VelocityCurve::Linear,
            max_linear_speed: 200.0,
            max_angular_speed: 6.0,
            linear_damping: 0.92,
            angular_damping: 0.96,
            smoothing_alpha: 0.15,
            gyro_weight: 0.92,
            deadzone: 0.12,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VelocityCurve {
    Linear,
    Quadratic,
    Cubic,
}

impl VelocityCurve {
    /// Apply the velocity curve to a normalized input (-1.0 to 1.0)
    pub fn apply(&self, input: f32) -> f32 {
        let sign = input.signum();
        let magnitude = input.abs();
        let curved = match self {
            VelocityCurve::Linear => magnitude,
            VelocityCurve::Quadratic => magnitude * magnitude,
            VelocityCurve::Cubic => magnitude * magnitude * magnitude,
        };
        sign * curved
    }
}

/// Spatial state tracking position, velocity, and orientation
pub struct SpatialState {
    /// Current integration mode
    pub mode: SpatialMode,

    /// Position in mm (X, Y, Z)
    pub position: [f32; 3],

    /// Velocity in mm/s (X, Y, Z)
    pub velocity: [f32; 3],

    /// Linear acceleration from accelerometer in G (X, Y, Z)
    pub linear_accel: [f32; 3],

    /// Angular velocity from gyroscope in rad/s (X, Y, Z)
    pub angular_velocity: [f32; 3],

    /// Smoothed velocity for output
    smoothed_velocity: [f32; 3],

    /// Orientation filter (complementary filter for gyro+accel fusion)
    orientation_filter: ComplementaryFilter,

    /// Integration config
    config: IntegrationConfig,

    /// Current "force" vector for AxiDraw mode (from D-pad)
    axidraw_force_type: u8,
}

impl std::fmt::Debug for SpatialState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpatialState")
            .field("mode", &self.mode)
            .field("position", &self.position)
            .field("velocity", &self.velocity)
            .field("linear_accel", &self.linear_accel)
            .field("angular_velocity", &self.angular_velocity)
            .field("orientation", &self.orientation_filter.orientation)
            .finish()
    }
}

impl SpatialState {
    pub fn new(config: IntegrationConfig) -> Self {
        Self {
            mode: SpatialMode::Standard,
            position: [0.0; 3],
            velocity: [0.0; 3],
            linear_accel: [0.0; 3],
            angular_velocity: [0.0; 3],
            smoothed_velocity: [0.0; 3],
            orientation_filter: ComplementaryFilter::new(config.gyro_weight),
            config,
            axidraw_force_type: 0,
        }
    }

    /// Get the current orientation quaternion
    pub fn orientation(&self) -> &Quaternion {
        &self.orientation_filter.orientation
    }

    /// Set the orientation directly (for snapshotting)
    pub fn set_orientation(&mut self, quat: Quaternion) {
        self.orientation_filter.orientation = quat;
    }

    /// Create a snapshot copy of the spatial state (for sending to renderer)
    pub fn snapshot(&self) -> SpatialState {
        let config = self.config.clone();
        let mut snapshot = SpatialState::new(config);
        snapshot.mode = self.mode;
        snapshot.position = self.position;
        snapshot.velocity = self.velocity;
        snapshot.linear_accel = self.linear_accel;
        snapshot.angular_velocity = self.angular_velocity;
        snapshot.smoothed_velocity = self.smoothed_velocity;
        snapshot.orientation_filter.orientation = self.orientation_filter.orientation;
        snapshot.axidraw_force_type = self.axidraw_force_type;
        snapshot
    }

    /// Reset all spatial state to initial values
    pub fn reset(&mut self) {
        self.position = [0.0; 3];
        self.velocity = [0.0; 3];
        self.linear_accel = [0.0; 3];
        self.angular_velocity = [0.0; 3];
        self.smoothed_velocity = [0.0; 3];
        self.orientation_filter.orientation = spatial_core::Quaternion::IDENTITY;
    }

    /// Reset position to origin (keeps orientation)
    pub fn reset_position(&mut self) {
        self.position = [0.0; 3];
        self.velocity = [0.0; 3];
        self.smoothed_velocity = [0.0; 3];
    }

    /// Reset orientation to identity
    pub fn reset_orientation(&mut self) {
        self.orientation_filter.orientation = Quaternion::IDENTITY;
    }

    /// Set the spatial mode
    pub fn set_mode(&mut self, mode: SpatialMode) {
        self.mode = mode;
        // Reset some state when switching modes if necessary
        self.velocity = [0.0; 3];
    }

    /// [IMPORTANT] This is where we change how the controller's spatial state
    ///             is controlled.
    /// Integrate controller state over time delta
    pub fn integrate(&mut self, state: &ControllerState, dt: f32) {
        // Natural DualSense axes: X=Right, Y=Forward, Z=Up (Touchpad)
        let gyro = state.gyroscope.to_rad_per_sec();
        let accel = state.accelerometer.to_g();

        // Small deadzone to gyro to reduce drift
        let gx = if gyro.x.abs() < 0.005 { 0.0 } else { gyro.x };
        let gy = if gyro.y.abs() < 0.005 { 0.0 } else { gyro.y };
        let gz = if gyro.z.abs() < 0.005 { 0.0 } else { gyro.z };

        // Internal state is Natural (Z-Up)
        self.angular_velocity = [gx, gy, gz];
        self.linear_accel = [accel.x, accel.y, accel.z];

        // Update orientation using complementary filter
        // Assuming the filter expects gravity on the 3rd component (Z)
        self.orientation_filter
            .update([gx, gy, gz], [accel.x, accel.y, accel.z], dt);

        // Check for reset buttons
        if state.buttons.options {
            self.reset();
        }

        // Handle specific modes
        match self.mode {
            SpatialMode::Standard => {
                let (lx, ly) = state.left_stick.normalized();
                let (l2, r2) = state.triggers.normalized();

                let lx = apply_deadzone(lx, self.config.deadzone);
                let ly = apply_deadzone(ly, self.config.deadzone);
                let l2 = apply_deadzone(l2, self.config.deadzone);
                let r2 = apply_deadzone(r2, self.config.deadzone);

                // Natural: X=Right, Y=Forward, Z=Up
                let target_vel = [
                    lx * self.config.max_linear_speed,
                    ly * self.config.max_linear_speed,
                    (r2 - l2) * self.config.max_linear_speed,
                ];

                self.update_velocity_and_position(target_vel, dt);
            }
            SpatialMode::Heading => {
                let (l2, r2) = state.triggers.normalized();
                let l2 = apply_deadzone(l2, self.config.deadzone);
                let r2 = apply_deadzone(r2, self.config.deadzone);

                // Natural Forward is Y+ [0, 1, 0]
                let quat = self.orientation_filter.orientation;
                let forward = quat.rotate_vec3([0.0, 1.0, 0.0]);

                let speed = (r2 - l2) * self.config.max_linear_speed;
                let target_vel = [
                    forward[0] * speed,
                    forward[1] * speed,
                    forward[2] * speed,
                ];

                self.update_velocity_and_position(target_vel, dt);
            }
            SpatialMode::Accelerometer => {
                let g_to_mms2 = 9806.65;
                let quat = self.orientation_filter.orientation;

                // Rotate measured accel to world frame
                let accel_world = quat.rotate_vec3(self.linear_accel);

                // Subtract gravity (1G on Z+ in Natural Z-Up world)
                let mut true_accel = [
                    accel_world[0] * g_to_mms2,
                    accel_world[1] * g_to_mms2,
                    (accel_world[2] - 1.0) * g_to_mms2,
                ];

                let accel_deadzone = 0.08 * g_to_mms2;
                for i in 0..3 {
                    if true_accel[i].abs() < accel_deadzone {
                        true_accel[i] = 0.0;
                    } else {
                        true_accel[i] -= true_accel[i].signum() * accel_deadzone;
                    }
                }

                for i in 0..3 {
                    self.velocity[i] += true_accel[i] * dt;
                    self.velocity[i] *= 0.98; // Aggressive damping for IMU stability
                    if self.velocity[i].abs() < 5.0 {
                        self.velocity[i] = 0.0;
                    }
                    self.position[i] += self.velocity[i] * dt;
                }
            }
            SpatialMode::AxiDraw => {
                let (rx, ry) = state.right_stick.normalized();
                let rx = apply_deadzone(rx, self.config.deadzone);
                let ry = apply_deadzone(ry, self.config.deadzone);

                let (lx, ly) = state.left_stick.normalized();
                let lx = apply_deadzone(lx, self.config.deadzone);
                let ly = apply_deadzone(ly, self.config.deadzone);

                let force_weight = 0.5;
                let combined_x = rx + lx * force_weight;
                let combined_y = ry + ly * force_weight;

                let (l2, r2) = state.triggers.normalized();
                let l2 = apply_deadzone(l2, self.config.deadzone);
                let r2 = apply_deadzone(r2, self.config.deadzone);

                // Pen Z: R2 lowers (incremental), L2 raises (fast)
                let z_vel = (r2 * 0.5 - l2 * 2.0) * self.config.max_linear_speed;

                let target_vel = [
                    combined_x * self.config.max_linear_speed,
                    combined_y * self.config.max_linear_speed,
                    z_vel,
                ];

                if state.buttons.dpad_up { self.axidraw_force_type = 1; }
                if state.buttons.dpad_down { self.axidraw_force_type = 2; }
                if state.buttons.dpad_left { self.axidraw_force_type = 3; }
                if state.buttons.dpad_right { self.axidraw_force_type = 4; }

                self.update_velocity_and_position(target_vel, dt);
            }
            SpatialMode::ThreeD => {
                let (lx, ly) = state.left_stick.normalized();
                let (l2, r2) = state.triggers.normalized();

                let lx = apply_deadzone(lx, self.config.deadzone);
                let ly = apply_deadzone(ly, self.config.deadzone);
                let l2 = apply_deadzone(l2, self.config.deadzone);
                let r2 = apply_deadzone(r2, self.config.deadzone);

                let quat = self.orientation_filter.orientation;
                let forward = quat.rotate_vec3([0.0, 1.0, 0.0]);
                let right = quat.rotate_vec3([1.0, 0.0, 0.0]);

                let move_speed = self.config.max_linear_speed;
                let target_vel = [
                    (right[0] * lx + forward[0] * ly) * move_speed,
                    (right[1] * lx + forward[1] * ly) * move_speed,
                    (r2 - l2) * move_speed + (right[2] * lx + forward[2] * ly) * move_speed,
                ];

                self.update_velocity_and_position(target_vel, dt);
            }
        }

        // Smooth velocity for output (separate from physics velocity)
        for i in 0..3 {
            self.smoothed_velocity[i] = self.smoothed_velocity[i] * 0.8 + self.velocity[i] * 0.2;
        }
    }

    fn update_velocity_and_position(&mut self, target_vel: [f32; 3], dt: f32) {
        // Smooth velocity transition (low-pass filter)
        let alpha = self.config.smoothing_alpha;
        for i in 0..3 {
            self.velocity[i] = self.velocity[i] * (1.0 - alpha) + target_vel[i] * alpha;
        }

        // Apply damping when no significant input
        let has_input = target_vel.iter().any(|&v| v.abs() > 0.1);
        if !has_input {
            for i in 0..3 {
                self.velocity[i] *= self.config.linear_damping;
                if self.velocity[i].abs() < 0.1 {
                    self.velocity[i] = 0.0;
                }
            }
        }

        // Integrate velocity -> position
        for i in 0..3 {
            self.position[i] += self.velocity[i] * dt;
        }
    }

    /// Get smoothed velocity for output
    pub fn smoothed_velocity(&self) -> [f32; 3] {
        self.smoothed_velocity
    }
}

/// Apply deadzone to an input value
fn apply_deadzone(value: f32, deadzone: f32) -> f32 {
    if value.abs() < deadzone {
        0.0
    } else {
        // Rescale so that edge of deadzone maps to 0
        let sign = value.signum();
        let magnitude = (value.abs() - deadzone) / (1.0 - deadzone);
        sign * magnitude.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_velocity_curve_linear() {
        let curve = VelocityCurve::Linear;
        assert!((curve.apply(0.5) - 0.5).abs() < 1e-6);
        assert!((curve.apply(-0.5) - (-0.5)).abs() < 1e-6);
    }

    #[test]
    fn test_velocity_curve_cubic() {
        let curve = VelocityCurve::Cubic;
        assert!((curve.apply(0.5) - 0.125).abs() < 1e-6);
        assert!((curve.apply(-0.5) - (-0.125)).abs() < 1e-6);
    }

    #[test]
    fn test_deadzone() {
        assert_eq!(apply_deadzone(0.05, 0.1), 0.0);
        assert_eq!(apply_deadzone(-0.05, 0.1), 0.0);
        // Value at edge of deadzone should map to 0
        assert!((apply_deadzone(0.1, 0.1)).abs() < 1e-6);
        // Full value should be close to 1
        assert!((apply_deadzone(1.0, 0.1) - 1.0).abs() < 1e-6);
    }
}
