//! Profile management for DualSense controller settings
//!
//! Profiles allow users to save and load controller configurations including:
//! - LED color settings
//! - Adaptive trigger effects
//! - Player LED patterns
//! - Rumble preferences
//!
//! Profiles are stored in `$DUALSENSE_HOME/profiles` or `$HOME/.dualsense-cmd/profiles`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::dualsense::{MuteLedState, OutputState, PlayerLeds, TriggerEffect, TriggerEffectMode};

/// Profile directory environment variable
pub const PROFILE_DIR_ENV: &str = "DUALSENSE_HOME";

/// Default profile directory name under $HOME
pub const DEFAULT_PROFILE_DIR: &str = ".dualsense-cmd";

/// Profile sub-directory
pub const PROFILES_SUBDIR: &str = "profiles";

/// LED color configuration in a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileLedColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Default for ProfileLedColor {
    fn default() -> Self {
        Self {
            r: 226,
            g: 64,
            b: 48,
        } // Default red
    }
}

impl From<ProfileLedColor> for (u8, u8, u8) {
    fn from(c: ProfileLedColor) -> Self {
        (c.r, c.g, c.b)
    }
}

/// Adaptive trigger configuration in a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileTriggerEffect {
    /// Effect type: "off", "continuous", "section", "vibration", "weapon", "bow"
    pub effect_type: String,
    /// Start position (0-255)
    #[serde(default)]
    pub start: u8,
    /// End position (0-255)
    #[serde(default = "default_end")]
    pub end: u8,
    /// Force/strength (0-255)
    #[serde(default)]
    pub force: u8,
    /// Frequency for vibration (0-255 Hz)
    #[serde(default)]
    pub frequency: u8,
}

fn default_end() -> u8 {
    255
}

impl Default for ProfileTriggerEffect {
    fn default() -> Self {
        Self {
            effect_type: "off".to_string(),
            start: 0,
            end: 255,
            force: 0,
            frequency: 0,
        }
    }
}

impl From<ProfileTriggerEffect> for TriggerEffect {
    fn from(p: ProfileTriggerEffect) -> Self {
        match p.effect_type.to_lowercase().as_str() {
            "continuous" => TriggerEffect::continuous(p.force),
            "section" => TriggerEffect::section(p.start, p.end, p.force),
            "vibration" => TriggerEffect::vibration(p.start, p.frequency, p.force),
            "weapon" => TriggerEffect::weapon(p.start, p.end, p.force),
            "bow" => TriggerEffect::bow(p.force),
            _ => TriggerEffect::default(),
        }
    }
}

impl From<TriggerEffect> for ProfileTriggerEffect {
    fn from(e: TriggerEffect) -> Self {
        let effect_type = match e.mode {
            TriggerEffectMode::Off => "off",
            TriggerEffectMode::Continuous => "continuous",
            TriggerEffectMode::SectionResistance => "section",
            TriggerEffectMode::Vibration => "vibration",
            TriggerEffectMode::CombinedRV => "section",
            TriggerEffectMode::Calibration => "off",
        };
        Self {
            effect_type: effect_type.to_string(),
            start: e.start_position,
            end: e.end_position,
            force: e.force,
            frequency: e.frequency,
        }
    }
}

/// Player LED configuration in a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProfilePlayerLeds {
    /// Player number (1-5)
    Number(u8),
    /// Individual LED states
    Custom {
        led1: bool,
        led2: bool,
        led3: bool,
        led4: bool,
        led5: bool,
    },
}

impl Default for ProfilePlayerLeds {
    fn default() -> Self {
        ProfilePlayerLeds::Number(1)
    }
}

impl From<ProfilePlayerLeds> for PlayerLeds {
    fn from(p: ProfilePlayerLeds) -> Self {
        match p {
            ProfilePlayerLeds::Number(n) => PlayerLeds::from_player(n),
            ProfilePlayerLeds::Custom {
                led1,
                led2,
                led3,
                led4,
                led5,
            } => PlayerLeds {
                led1,
                led2,
                led3,
                led4,
                led5,
            },
        }
    }
}

/// [TODO] Issue where macOS cannot save profile over Bluetooth
///        Likely related to how we cannot receive messages over
///        bluetooth until we 'Identify' the controller through
///        macOS system settings, which I assume means our messages
///        are either being mitm'd by macOS/kernel or something else.
///        Surely we can bypass the macOS identify step? Or is that
///        part of the bluetooth connection protocol and the Identify
///        is loading saved profiles onto the controller (it is, it changes
///        to a different LED but that was set on Windows through Steam
///        big picture years ago.)
/// Controller profile with all settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Profile name
    pub name: String,

    /// Profile description
    #[serde(default)]
    pub description: String,

    /// LED lightbar color
    #[serde(default)]
    pub led_color: ProfileLedColor,

    /// Whether lightbar is enabled
    #[serde(default = "default_true")]
    pub lightbar_enabled: bool,

    /// L2 trigger effect
    #[serde(default)]
    pub l2_trigger: ProfileTriggerEffect,

    /// R2 trigger effect
    #[serde(default)]
    pub r2_trigger: ProfileTriggerEffect,

    /// Player LED configuration
    #[serde(default)]
    pub player_leds: Option<ProfilePlayerLeds>,

    /// Mute LED state: "off", "on", "breathing"
    #[serde(default)]
    pub mute_led: Option<String>,

    /// Default rumble intensity (0-255) - used as multiplier
    #[serde(default = "default_rumble_intensity")]
    pub rumble_intensity: u8,

    /// Custom metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

fn default_rumble_intensity() -> u8 {
    255
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            description: "Default controller profile".to_string(),
            led_color: ProfileLedColor::default(),
            lightbar_enabled: true,
            l2_trigger: ProfileTriggerEffect::default(),
            r2_trigger: ProfileTriggerEffect::default(),
            player_leds: Some(ProfilePlayerLeds::Number(1)),
            mute_led: None,
            rumble_intensity: 255,
            metadata: HashMap::new(),
        }
    }
}

impl Profile {
    /// Create a new profile with a name
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// Load a profile from a JSON file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read profile: {}", path.display()))?;
        let profile: Profile = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse profile: {}", path.display()))?;
        Ok(profile)
    }

    /// Save the profile to a JSON file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Convert to OutputState for applying to controller
    pub fn to_output_state(&self) -> OutputState {
        let mute_led = match self.mute_led.as_deref() {
            Some("on") => MuteLedState::On,
            Some("breathing") => MuteLedState::Breathing,
            _ => MuteLedState::Off,
        };

        let player_leds = self
            .player_leds
            .clone()
            .map(PlayerLeds::from)
            .unwrap_or_default();

        OutputState {
            led_color: self.led_color.clone().into(),
            rumble: (0, 0),
            l2_effect: self.l2_trigger.clone().into(),
            r2_effect: self.r2_trigger.clone().into(),
            player_leds,
            mute_led,
            lightbar_enabled: self.lightbar_enabled,
            bt_seq: 0,
        }
    }

    /// Create preset profiles
    pub fn preset_default() -> Self {
        Self::default()
    }

    pub fn preset_gaming() -> Self {
        Self {
            name: "Gaming".to_string(),
            description: "Optimized for gaming with trigger feedback".to_string(),
            led_color: ProfileLedColor { r: 255, g: 0, b: 0 },
            l2_trigger: ProfileTriggerEffect {
                effect_type: "section".to_string(),
                start: 70,
                end: 160,
                force: 200,
                frequency: 0,
            },
            r2_trigger: ProfileTriggerEffect {
                effect_type: "weapon".to_string(),
                start: 80,
                end: 120,
                force: 255,
                frequency: 0,
            },
            player_leds: Some(ProfilePlayerLeds::Number(1)),
            ..Default::default()
        }
    }

    pub fn preset_racing() -> Self {
        Self {
            name: "Racing".to_string(),
            description: "Progressive resistance for racing games".to_string(),
            led_color: ProfileLedColor { r: 0, g: 255, b: 0 },
            l2_trigger: ProfileTriggerEffect {
                effect_type: "continuous".to_string(),
                start: 0,
                end: 255,
                force: 150,
                frequency: 0,
            },
            r2_trigger: ProfileTriggerEffect {
                effect_type: "continuous".to_string(),
                start: 0,
                end: 255,
                force: 150,
                frequency: 0,
            },
            player_leds: Some(ProfilePlayerLeds::Number(1)),
            ..Default::default()
        }
    }

    pub fn preset_accessibility() -> Self {
        Self {
            name: "Accessibility".to_string(),
            description: "Reduced resistance for easier use".to_string(),
            led_color: ProfileLedColor {
                r: 255,
                g: 255,
                b: 255,
            },
            l2_trigger: ProfileTriggerEffect::default(),
            r2_trigger: ProfileTriggerEffect::default(),
            rumble_intensity: 128,
            player_leds: Some(ProfilePlayerLeds::Number(1)),
            ..Default::default()
        }
    }
}

/// Profile manager for loading, saving, and listing profiles
pub struct ProfileManager {
    profiles_dir: PathBuf,
}

impl ProfileManager {
    /// Create a new profile manager
    pub fn new() -> Result<Self> {
        let profiles_dir = Self::get_profiles_dir()?;

        // Ensure directory exists
        if !profiles_dir.exists() {
            fs::create_dir_all(&profiles_dir).with_context(|| {
                format!(
                    "Failed to create profiles directory: {}",
                    profiles_dir.display()
                )
            })?;
        }

        Ok(Self { profiles_dir })
    }

    /// Get the profiles directory path
    pub fn get_profiles_dir() -> Result<PathBuf> {
        // Check DUALSENSE_HOME first
        if let Ok(home) = std::env::var(PROFILE_DIR_ENV) {
            let path = PathBuf::from(home).join(PROFILES_SUBDIR);
            return Ok(path);
        }

        // Fall back to ~/.dualsense-cmd/profiles
        let home = dirs::home_dir().context("Could not determine home directory")?;

        Ok(home.join(DEFAULT_PROFILE_DIR).join(PROFILES_SUBDIR))
    }

    /// List all available profiles
    pub fn list(&self) -> Result<Vec<ProfileInfo>> {
        let mut profiles = Vec::new();

        if !self.profiles_dir.exists() {
            return Ok(profiles);
        }

        for entry in fs::read_dir(&self.profiles_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(profile) = Profile::load(&path) {
                    let file_name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    profiles.push(ProfileInfo {
                        id: file_name,
                        name: profile.name,
                        description: profile.description,
                        path,
                    });
                }
            }
        }

        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    /// Get a profile by name/ID
    pub fn get(&self, name: &str) -> Result<Profile> {
        let path = self.profile_path(name);
        Profile::load(&path)
    }

    /// Save a profile
    pub fn save(&self, profile: &Profile) -> Result<PathBuf> {
        let id = Self::name_to_id(&profile.name);
        let path = self.profile_path(&id);
        profile.save(&path)?;
        Ok(path)
    }

    /// Delete a profile
    pub fn delete(&self, name: &str) -> Result<()> {
        let path = self.profile_path(name);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to delete profile: {}", path.display()))?;
        }
        Ok(())
    }

    /// Check if a profile exists
    pub fn exists(&self, name: &str) -> bool {
        self.profile_path(name).exists()
    }

    /// Get the path for a profile
    fn profile_path(&self, name: &str) -> PathBuf {
        let id = Self::name_to_id(name);
        self.profiles_dir.join(format!("{}.json", id))
    }

    /// Convert profile name to file ID (lowercase, no spaces)
    fn name_to_id(name: &str) -> String {
        name.to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string()
    }

    /// Get profiles directory
    pub fn profiles_dir(&self) -> &Path {
        &self.profiles_dir
    }

    /// Create default profiles if none exist
    pub fn init_defaults(&self) -> Result<()> {
        if self.list()?.is_empty() {
            self.save(&Profile::preset_default())?;
            self.save(&Profile::preset_gaming())?;
            self.save(&Profile::preset_racing())?;
            self.save(&Profile::preset_accessibility())?;
        }
        Ok(())
    }
}

impl Default for ProfileManager {
    fn default() -> Self {
        Self::new().expect("Failed to create profile manager")
    }
}

/// Basic profile info for listing
#[derive(Debug, Clone, Serialize)]
pub struct ProfileInfo {
    /// Profile file ID (filename without extension)
    pub id: String,
    /// Profile display name
    pub name: String,
    /// Profile description
    pub description: String,
    /// Full path to profile file
    #[serde(skip)]
    pub path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_serialization() {
        let profile = Profile::preset_gaming();
        let json = serde_json::to_string_pretty(&profile).unwrap();
        let loaded: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(profile.name, loaded.name);
    }

    #[test]
    fn test_name_to_id() {
        assert_eq!(ProfileManager::name_to_id("My Profile"), "my-profile");
        assert_eq!(ProfileManager::name_to_id("Gaming"), "gaming");
        assert_eq!(ProfileManager::name_to_id("test_profile"), "test_profile");
    }

    #[test]
    fn test_trigger_effect_conversion() {
        let profile_effect = ProfileTriggerEffect {
            effect_type: "weapon".to_string(),
            start: 80,
            end: 120,
            force: 200,
            frequency: 0,
        };
        let effect: TriggerEffect = profile_effect.into();
        assert_eq!(effect.mode, TriggerEffectMode::SectionResistance);
    }
}
