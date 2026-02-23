// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dualsense_cmd::dualsense::{
    DualSense, TriggerEffect,
    SONY_VENDOR_ID, DUALSENSE_PRODUCT_ID, DUALSENSE_EDGE_PRODUCT_ID
};
use dualsense_cmd::profile::{Profile, ProfileManager, ProfileInfo};
use dualsense_cmd::spatial::{IntegrationConfig, SpatialState, SpatialMode};
use hidapi::HidApi;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};
use tokio::time::Duration;

#[derive(Serialize, Clone)]
struct ControllerInfo {
    index: usize,
    product: String,
    serial: String,
    connection: String,
}

struct AppState {
    controller: Arc<Mutex<Option<DualSense>>>,
    spatial: Arc<Mutex<SpatialState>>,
}

#[tauri::command]
async fn list_controllers() -> Result<Vec<ControllerInfo>, String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let controllers: Vec<_> = api
        .device_list()
        .filter(|d| {
            d.vendor_id() == SONY_VENDOR_ID
                && (d.product_id() == DUALSENSE_PRODUCT_ID || d.product_id() == DUALSENSE_EDGE_PRODUCT_ID)
        })
        .enumerate()
        .map(|(i, d)| {
            let interface = d.interface_number();
            let connection = if interface == -1 {
                "Bluetooth".to_string()
            } else {
                "USB".to_string()
            };
            ControllerInfo {
                index: i,
                product: d.product_string().unwrap_or("DualSense").to_string(),
                serial: d.serial_number().unwrap_or("Unknown").to_string(),
                connection,
            }
        })
        .collect();
    Ok(controllers)
}

#[tauri::command]
async fn connect_controller(state: State<'_, AppState>) -> Result<String, String> {
    let mut controller_guard = state.controller.lock().unwrap();
    if controller_guard.is_some() {
        return Ok("Already connected".to_string());
    }

    let controller = DualSense::find_and_connect().map_err(|e| e.to_string())?;
    *controller_guard = Some(controller);
    Ok("Connected".to_string())
}

#[tauri::command]
async fn set_led(r: u8, g: u8, b: u8, state: State<'_, AppState>) -> Result<(), String> {
    let controller_guard = state.controller.lock().unwrap();
    if let Some(controller) = controller_guard.as_ref() {
        controller.set_led_color(r, g, b).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn ping() -> String {
    "pong".to_string()
}

// [NOTE] Does not work (at least on macOS over bt)
#[tauri::command]
async fn set_rumble(left: u8, right: u8, duration_ms: Option<u64>, state: State<'_, AppState>) -> Result<(), String> {
    let controller_arc = state.controller.clone();
    
    // Set rumble immediately
    {
        let controller_guard = controller_arc.lock().unwrap();
        if let Some(controller) = controller_guard.as_ref() {
            controller.set_rumble(left, right).map_err(|e| e.to_string())?;
        }
    }

    // If duration is provided, spawn a task to stop it
    if let Some(ms) = duration_ms {
        if ms > 0 {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                let controller_guard = controller_arc.lock().unwrap();
                if let Some(controller) = controller_guard.as_ref() {
                    let _ = controller.set_rumble(0, 0);
                }
            });
        }
    }
    
    Ok(())
}

#[tauri::command]
async fn reset_spatial(state: State<'_, AppState>) -> Result<(), String> {
    let mut spatial_guard = state.spatial.lock().unwrap();
    spatial_guard.reset();
    Ok(())
}

#[tauri::command]
async fn set_spatial_mode(mode: SpatialMode, state: State<'_, AppState>) -> Result<(), String> {
    let mut spatial_guard = state.spatial.lock().unwrap();
    spatial_guard.set_mode(mode);
    Ok(())
}

// Profile commands

#[tauri::command]
async fn list_profiles() -> Result<Vec<ProfileInfo>, String> {
    let manager = ProfileManager::new().map_err(|e| e.to_string())?;
    manager.list().map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_profile(name: String) -> Result<Profile, String> {
    let manager = ProfileManager::new().map_err(|e| e.to_string())?;
    manager.get(&name).map_err(|e| e.to_string())
}

#[tauri::command]
async fn apply_profile(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let manager = ProfileManager::new().map_err(|e| e.to_string())?;
    let profile = manager.get(&name).map_err(|e| e.to_string())?;

    let controller_guard = state.controller.lock().unwrap();
    if let Some(controller) = controller_guard.as_ref() {
        let output_state = profile.to_output_state();
        controller.apply_output_state(output_state).map_err(|e| e.to_string())?;
    } else {
        return Err("No controller connected".to_string());
    }
    Ok(())
}

#[tauri::command]
async fn save_profile(profile: Profile) -> Result<String, String> {
    let manager = ProfileManager::new().map_err(|e| e.to_string())?;
    let path = manager.save(&profile).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
async fn delete_profile(name: String) -> Result<(), String> {
    let manager = ProfileManager::new().map_err(|e| e.to_string())?;
    manager.delete(&name).map_err(|e| e.to_string())
}

#[tauri::command]
async fn init_default_profiles() -> Result<(), String> {
    let manager = ProfileManager::new().map_err(|e| e.to_string())?;
    manager.init_defaults().map_err(|e| e.to_string())
}

// Adaptive trigger commands

#[derive(Deserialize)]
pub struct TriggerConfig {
    effect_type: String,
    start: Option<u8>,
    end: Option<u8>,
    force: Option<u8>,
    frequency: Option<u8>,
}

impl From<TriggerConfig> for TriggerEffect {
    fn from(c: TriggerConfig) -> Self {
        let force = c.force.unwrap_or(200);
        let start = c.start.unwrap_or(70);
        let end = c.end.unwrap_or(160);
        let freq = c.frequency.unwrap_or(10);

        match c.effect_type.to_lowercase().as_str() {
            "continuous" => TriggerEffect::continuous(force),
            "section" => TriggerEffect::section(start, end, force),
            "vibration" => TriggerEffect::vibration(start, freq, force),
            "weapon" => TriggerEffect::weapon(start, end, force),
            "bow" => TriggerEffect::bow(force),
            _ => TriggerEffect::default(),
        }
    }
}

#[tauri::command]
async fn set_l2_trigger(config: TriggerConfig, state: State<'_, AppState>) -> Result<(), String> {
    let controller_guard = state.controller.lock().unwrap();
    if let Some(controller) = controller_guard.as_ref() {
        let effect: TriggerEffect = config.into();
        controller.set_l2_trigger_effect(effect).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn set_r2_trigger(config: TriggerConfig, state: State<'_, AppState>) -> Result<(), String> {
    let controller_guard = state.controller.lock().unwrap();
    if let Some(controller) = controller_guard.as_ref() {
        let effect: TriggerEffect = config.into();
        controller.set_r2_trigger_effect(effect).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn set_player_leds(player: u8, state: State<'_, AppState>) -> Result<(), String> {
    let controller_guard = state.controller.lock().unwrap();
    if let Some(controller) = controller_guard.as_ref() {
        controller.set_player_number(player).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Features info

#[derive(Serialize)]
pub struct FeatureInfo {
    name: String,
    category: String,
    status: String, // "implemented", "future", "partial"
    description: String,
}

#[tauri::command]
async fn get_features() -> Vec<FeatureInfo> {
    vec![
        // Input - Implemented
        FeatureInfo { name: "Thumbsticks".into(), category: "input".into(), status: "implemented".into(), description: "Left/right analog sticks with X/Y axes".into() },
        FeatureInfo { name: "Action Buttons".into(), category: "input".into(), status: "implemented".into(), description: "Cross, Circle, Square, Triangle".into() },
        FeatureInfo { name: "D-Pad".into(), category: "input".into(), status: "implemented".into(), description: "Directional buttons (8-way)".into() },
        FeatureInfo { name: "Bumpers".into(), category: "input".into(), status: "implemented".into(), description: "L1, R1 shoulder buttons".into() },
        FeatureInfo { name: "Triggers".into(), category: "input".into(), status: "implemented".into(), description: "L2, R2 pressure-sensitive".into() },
        FeatureInfo { name: "Stick Buttons".into(), category: "input".into(), status: "implemented".into(), description: "L3, R3 click".into() },
        FeatureInfo { name: "Create Button".into(), category: "input".into(), status: "implemented".into(), description: "Capture/share button".into() },
        FeatureInfo { name: "Options Button".into(), category: "input".into(), status: "implemented".into(), description: "Menu button".into() },
        FeatureInfo { name: "PS Button".into(), category: "input".into(), status: "implemented".into(), description: "Central system button".into() },
        FeatureInfo { name: "Mute Button".into(), category: "input".into(), status: "partial".into(), description: "Microphone mute".into() },
        FeatureInfo { name: "Touchpad Multitouch".into(), category: "input".into(), status: "partial".into(), description: "Touchpad 2-finger multitouch".into() },
        FeatureInfo { name: "Touchpad Click".into(), category: "input".into(), status: "partial".into(), description: "Touchpad 1-finger click".into() },        
        FeatureInfo { name: "Accelerometer".into(), category: "input".into(), status: "implemented".into(), description: "3-axis motion sensing".into() },
        FeatureInfo { name: "Gyroscope".into(), category: "input".into(), status: "implemented".into(), description: "3-axis rotation sensing".into() },
        FeatureInfo { name: "Battery".into(), category: "input".into(), status: "implemented".into(), description: "Level and charging status".into() },
        FeatureInfo { name: "Microphone".into(), category: "input".into(), status: "future".into(), description: "Audio input (OS-level)".into() },
        FeatureInfo { name: "Headset Input".into(), category: "input".into(), status: "future".into(), description: "Audio jack input (OS-level)".into() },

        // Output - Implemented
        FeatureInfo { name: "Haptic Feedback".into(), category: "output".into(), status: "partial".into(), description: "Dual rumble motors".into() },
        FeatureInfo { name: "Adaptive Triggers".into(), category: "output".into(), status: "partial".into(), description: "L2/R2 resistance and vibration".into() },
        FeatureInfo { name: "Light Bar".into(), category: "output".into(), status: "partial".into(), description: "RGB LED control".into() },
        FeatureInfo { name: "Player LEDs".into(), category: "output".into(), status: "partial".into(), description: "5 indicator LEDs".into() },
        FeatureInfo { name: "Mute LED".into(), category: "output".into(), status: "partial".into(), description: "Mic mute indicator state".into() },
        FeatureInfo { name: "Speaker".into(), category: "output".into(), status: "future".into(), description: "Audio output (OS-level)".into() },
        FeatureInfo { name: "Headset Output".into(), category: "output".into(), status: "future".into(), description: "Audio jack output (OS-level)".into() },
    ]
}

fn main() {
    let app_state = AppState {
        controller: Arc::new(Mutex::new(None)),
        spatial: Arc::new(Mutex::new(SpatialState::new(IntegrationConfig::default()))),
    };

    let controller_clone = app_state.controller.clone();
    let spatial_clone = app_state.spatial.clone();

    tauri::Builder::default()
        .manage(app_state)
        .setup(|app| {
            let handle = app.handle();
            
            // Spawn polling thread
            std::thread::spawn(move || {
                let mut last_update = std::time::Instant::now();
                loop {
                    let dt = last_update.elapsed().as_secs_f32();
                    last_update = std::time::Instant::now();

                    let mut controller_guard = controller_clone.lock().unwrap();
                    if let Some(controller) = controller_guard.as_mut() {
                        match controller.poll(16) {
                            Ok(state) => {
                                // Emit state event
                                handle.emit_all("controller-state", state).unwrap();

                                // Update spatial
                                let mut spatial_guard = spatial_clone.lock().unwrap();
                                spatial_guard.integrate(state, dt);
                                
                                // Share button (Create) resets camera state in frontend
                                if state.buttons.create {
                                    handle.emit_all("reset-camera", ()).unwrap();
                                }

                                // Emit spatial event
                                #[derive(Serialize, Clone)]
                                struct SpatialEvent {
                                    mode: SpatialMode,
                                    position: [f32; 3],
                                    velocity: [f32; 3],
                                    linear_accel: [f32; 3],
                                    angular_velocity: [f32; 3],
                                    orientation: [f32; 4], // w, x, y, z
                                }
                                let quat = spatial_guard.orientation();
                                let p = spatial_guard.position;
                                let v = spatial_guard.velocity;
                                let a = spatial_guard.linear_accel;
                                let g = spatial_guard.angular_velocity;

                                // Remap Natural (Z-Up) to Three.js (Y-Up)
                                // X -> X, Y -> -Z, Z -> Y
                                handle.emit_all("spatial-state", SpatialEvent {
                                    mode: spatial_guard.mode,
                                    position: [p[0], p[2], -p[1]],
                                    velocity: [v[0], v[2], -v[1]],
                                    linear_accel: [a[0], a[2], -a[1]],
                                    angular_velocity: [g[0], g[2], -g[1]],
                                    orientation: [quat.w, quat.x, quat.z, -quat.y],
                                }).unwrap();
                            }
                            Err(dualsense_cmd::dualsense::DualSenseError::Timeout) => {}
                            Err(_) => {
                                // Connection likely lost
                                *controller_guard = None;
                                handle.emit_all("controller-disconnected", ()).unwrap();
                            }
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(8));
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            list_controllers,
            connect_controller,
            set_led,
            set_rumble,
            reset_spatial,
            set_spatial_mode,
            // Profile commands
            list_profiles,
            get_profile,
            apply_profile,
            save_profile,
            delete_profile,
            init_default_profiles,
            // Trigger commands
            set_l2_trigger,
            set_r2_trigger,
            set_player_leds,
            // Features
            get_features
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
