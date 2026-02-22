//! Spatial state integration for controller-based positioning
//!
//! Integrates controller inputs (sticks, triggers, IMU) into spatial state
//! (position, velocity, orientation) using configurable physics parameters.

use spatial_core::{ComplementaryFilter, Quaternion};

use crate::dualsense::ControllerState;

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
            gyro_weight: 0.98,
            deadzone: 0.08,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
}

impl std::fmt::Debug for SpatialState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpatialState")
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
            position: [0.0; 3],
            velocity: [0.0; 3],
            linear_accel: [0.0; 3],
            angular_velocity: [0.0; 3],
            smoothed_velocity: [0.0; 3],
            orientation_filter: ComplementaryFilter::new(config.gyro_weight),
            config,
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
        snapshot.position = self.position;
        snapshot.velocity = self.velocity;
        snapshot.linear_accel = self.linear_accel;
        snapshot.angular_velocity = self.angular_velocity;
        snapshot.smoothed_velocity = self.smoothed_velocity;
        snapshot.orientation_filter.orientation = self.orientation_filter.orientation;
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

    /// Integrate controller state over time delta
    pub fn integrate(&mut self, state: &ControllerState, dt: f32) {
        // Extract normalized inputs
        let (left_x, left_y) = state.left_stick.normalized();
        let (_right_x, _right_y) = state.right_stick.normalized();
        let (_, r2) = state.triggers.normalized();

        // Apply deadzone
        let left_x = apply_deadzone(left_x, self.config.deadzone);
        let left_y = apply_deadzone(left_y, self.config.deadzone);
        let r2 = apply_deadzone(r2, self.config.deadzone);

        // Apply velocity curve
        let curved_x = self.config.velocity_curve.apply(left_x);
        let curved_y = self.config.velocity_curve.apply(left_y);
        let curved_z = self.config.velocity_curve.apply(r2);

        // Map to target velocity (left stick = X/Y planar, R2 = Z)
        let target_vel = [
            curved_x * self.config.max_linear_speed,
            curved_y * self.config.max_linear_speed,
            curved_z * self.config.max_linear_speed,
        ];

        // Smooth velocity transition (low-pass filter)
        let alpha = self.config.smoothing_alpha;
        for i in 0..3 {
            self.velocity[i] = self.velocity[i] * (1.0 - alpha) + target_vel[i] * alpha;
        }

        // Apply damping when no input (stick released)
        let has_input = left_x.abs() > 0.0 || left_y.abs() > 0.0 || r2.abs() > 0.0;
        if !has_input {
            for i in 0..3 {
                self.velocity[i] *= self.config.linear_damping;
            }
            // Zero out very small velocities
            for i in 0..3 {
                if self.velocity[i].abs() < 0.1 {
                    self.velocity[i] = 0.0;
                }
            }
        }

        // Integrate velocity -> position
        for i in 0..3 {
            self.position[i] += self.velocity[i] * dt;
        }

        // Smooth velocity for output (separate from physics velocity)
        for i in 0..3 {
            self.smoothed_velocity[i] =
                self.smoothed_velocity[i] * 0.8 + self.velocity[i] * 0.2;
        }

        // Extract IMU data
        let gyro = state.gyroscope.to_rad_per_sec();
        let accel = state.accelerometer.to_g();

        self.angular_velocity = [gyro.x, gyro.y, gyro.z];
        self.linear_accel = [accel.x, accel.y, accel.z];

        // Update orientation using complementary filter
        self.orientation_filter.update(
            [gyro.x, gyro.y, gyro.z],
            [accel.x, accel.y, accel.z],
            dt,
        );
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
