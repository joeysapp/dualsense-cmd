// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dualsense_cmd::dualsense::{ConnectionType, DualSense, SONY_VENDOR_ID, DUALSENSE_PRODUCT_ID, DUALSENSE_EDGE_PRODUCT_ID};
use dualsense_cmd::spatial::{IntegrationConfig, SpatialState};
use hidapi::HidApi;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};
use tokio::time::{self, Duration};

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
                                
                                // Emit spatial event
                                // We need to create a serializable version of spatial state
                                // or just emit the parts we need.
                                #[derive(Serialize, Clone)]
                                struct SpatialEvent {
                                    position: [f32; 3],
                                    velocity: [f32; 3],
                                    orientation: [f32; 4], // w, x, y, z
                                }
                                let quat = spatial_guard.orientation();
                                handle.emit_all("spatial-state", SpatialEvent {
                                    position: spatial_guard.position,
                                    velocity: spatial_guard.velocity,
                                    orientation: [quat.w, quat.x, quat.y, quat.z],
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
            reset_spatial
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
