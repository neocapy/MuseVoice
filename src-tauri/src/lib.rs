// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

mod encode;
mod flow;
mod flow_manager;
mod stream_processor;
pub mod ebml;
pub mod opus;
pub mod webm;

use flow_manager::{FlowManager, FlowManagerState, StatusResponse};
use crate::flow::FlowState;
use std::sync::Arc;
use tauri::{AppHandle, State, Manager, Emitter};
use tokio::sync::RwLock;
use tauri_plugin_clipboard_manager::ClipboardExt;

#[cfg(desktop)]
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};


#[tauri::command]
async fn get_status(flow_manager: State<'_, FlowManagerState>) -> Result<StatusResponse, String> {
    let manager_guard = flow_manager.read().await;

    if let Some(manager) = manager_guard.as_ref() {
        let state = manager.get_state().await;

        Ok(StatusResponse {
            state,
            samples: None, // Sample count is now handled via events
        })
    } else {
        Ok(StatusResponse {
            state: FlowState::Idle,
            samples: None,
        })
    }
}

#[tauri::command]
async fn start_audio_stream(
    flow_manager: State<'_, FlowManagerState>,
    app_handle: AppHandle,
    origin: String,
) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        let current_state = manager.get_state().await;

        match current_state {
            FlowState::Idle | FlowState::Completed | FlowState::Error | FlowState::Cancelled => {
                let flow_manager_clone = Arc::clone(&flow_manager.inner());
                manager.start_flow(app_handle, flow_manager_clone, origin).await?;
                Ok("Audio recording started successfully".to_string())
            }
            _ => Err("Cannot start recording: flow is not idle".to_string()),
        }
    } else {
        Err("Flow manager not initialized".to_string())
    }
}

#[tauri::command]
async fn stop_audio_stream(flow_manager: State<'_, FlowManagerState>, origin: String) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        let current_state = manager.get_state().await;

        match current_state {
            FlowState::Recording => {
                manager.stop_flow(origin).await?;
                Ok("Recording stopped, starting transcription...".to_string())
            }
            _ => Err("Cannot stop recording: not currently recording".to_string()),
        }
    } else {
        Err("Flow manager not initialized".to_string())
    }
}

#[tauri::command]
async fn cancel_transcription(flow_manager: State<'_, FlowManagerState>, origin: String) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        manager.cancel_flow(origin).await;
        Ok("Flow cancelled".to_string())
    } else {
        Err("Flow manager not initialized".to_string())
    }
}



#[tauri::command]
async fn retry_transcription(
    flow_manager: State<'_, FlowManagerState>,
    app_handle: AppHandle,
    origin: String,
) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        let flow_manager_clone = Arc::clone(&flow_manager.inner());
        manager.retry_transcription(app_handle, flow_manager_clone, origin).await?;
        Ok("Retrying transcription...".to_string())
    } else {
        Err("Flow manager not initialized".to_string())
    }
}
#[tauri::command]
async fn set_transcription_model(
    flow_manager: State<'_, FlowManagerState>,
    model: String,
) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        manager.set_model(model)?;
        Ok("Model updated".to_string())
    } else {
        Err("Flow manager not initialized".to_string())
    }
}

#[tauri::command]
async fn set_rewrite_enabled(
    flow_manager: State<'_, FlowManagerState>,
    enabled: bool,
) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        manager.set_rewrite_enabled(enabled);
        Ok("Rewrite setting updated".to_string())
    } else {
        Err("Flow manager not initialized".to_string())
    }
}

#[tauri::command]
async fn has_retry_data(flow_manager: State<'_, FlowManagerState>) -> Result<bool, String> {
    let manager_guard = flow_manager.read().await;

    if let Some(manager) = manager_guard.as_ref() {
        Ok(manager.has_retry_data())
    } else {
        Ok(false)
    }
}


#[tauri::command]
async fn copy_to_clipboard(text: String, app_handle: AppHandle) -> Result<String, String> {
    // Use Tauri's clipboard API through the app handle
    match app_handle.clipboard().write_text(text) {
        Ok(_) => Ok("Text copied to clipboard successfully".to_string()),
        Err(e) => Err(format!("Failed to copy text to clipboard: {}", e)),
    }
}

#[cfg(desktop)]
fn get_default_shortcut() -> &'static str {
    #[cfg(target_os = "macos")]
    return "Alt+Slash";
    
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    return "Alt+Slash";
}

#[cfg(desktop)]
fn get_shortcut_from_env() -> String {
    #[cfg(target_os = "macos")]
    let env_key = "MUSE_SHORTCUT_MACOS";
    
    #[cfg(target_os = "windows")]
    let env_key = "MUSE_SHORTCUT_WINDOWS";
    
    #[cfg(target_os = "linux")]
    let env_key = "MUSE_SHORTCUT_LINUX";
    
    std::env::var(env_key).unwrap_or_else(|_| get_default_shortcut().to_string())
}

#[cfg(desktop)]
fn parse_shortcut(shortcut_str: &str) -> Result<Shortcut, String> {
    let parts: Vec<&str> = shortcut_str.split('+').collect();
    if parts.is_empty() {
        return Err("Empty shortcut string".to_string());
    }
    
    let mut modifiers = Modifiers::empty();
    let mut key_code = None;
    
    for part in &parts {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "cmd" | "command" => modifiers |= Modifiers::META,
            "slash" => key_code = Some(Code::Slash),
            "space" => key_code = Some(Code::Space),
            "enter" => key_code = Some(Code::Enter),
            _ => {
                // Try to parse single character keys
                if part.len() == 1 {
                    let ch = part.chars().next().unwrap().to_uppercase().next().unwrap();
                    key_code = match ch {
                        'A' => Some(Code::KeyA),
                        'B' => Some(Code::KeyB),
                        'C' => Some(Code::KeyC),
                        'D' => Some(Code::KeyD),
                        'E' => Some(Code::KeyE),
                        'F' => Some(Code::KeyF),
                        'G' => Some(Code::KeyG),
                        'H' => Some(Code::KeyH),
                        'I' => Some(Code::KeyI),
                        'J' => Some(Code::KeyJ),
                        'K' => Some(Code::KeyK),
                        'L' => Some(Code::KeyL),
                        'M' => Some(Code::KeyM),
                        'N' => Some(Code::KeyN),
                        'O' => Some(Code::KeyO),
                        'P' => Some(Code::KeyP),
                        'Q' => Some(Code::KeyQ),
                        'R' => Some(Code::KeyR),
                        'S' => Some(Code::KeyS),
                        'T' => Some(Code::KeyT),
                        'U' => Some(Code::KeyU),
                        'V' => Some(Code::KeyV),
                        'W' => Some(Code::KeyW),
                        'X' => Some(Code::KeyX),
                        'Y' => Some(Code::KeyY),
                        'Z' => Some(Code::KeyZ),
                        _ => return Err(format!("Unsupported key: {}", part)),
                    };
                } else {
                    return Err(format!("Unsupported key: {}", part));
                }
            }
        }
    }
    
    match key_code {
        Some(code) => Ok(Shortcut::new(Some(modifiers), code)),
        None => Err("No key code found in shortcut".to_string()),
    }
}

#[cfg(desktop)]
fn emit_audio_feedback(app_handle: &AppHandle, sound_file: &str) {
    let _ = app_handle.emit("audio-feedback", sound_file);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let flow_manager: FlowManagerState = Arc::new(RwLock::new(None));

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .manage(flow_manager.clone())
        .invoke_handler(tauri::generate_handler![
            get_status,
            start_audio_stream,
            stop_audio_stream,
            cancel_transcription,
            retry_transcription,
            has_retry_data,
            copy_to_clipboard,
            set_transcription_model,
            set_rewrite_enabled
        ])
        .setup(move |app| {
            // Setup global shortcut for desktop platforms
            #[cfg(desktop)]
            {
                let shortcut_string = get_shortcut_from_env();
                println!("Setting up global shortcut: {}", shortcut_string);
                
                match parse_shortcut(&shortcut_string) {
                    Ok(shortcut) => {
                        let flow_manager_for_handler = flow_manager.clone();
                        let app_handle_for_handler = app.handle().clone();
                        
                        let handler_result = app.handle().plugin(
                            tauri_plugin_global_shortcut::Builder::new()
                                .with_handler(move |_app, _triggered_shortcut, event| {
                                    if event.state() == ShortcutState::Pressed {
                                        let flow_manager_clone = flow_manager_for_handler.clone();
                                        let app_handle_clone = app_handle_for_handler.clone();
                                        
                                        // Spawn async task to handle the shortcut
                                        tauri::async_runtime::spawn(async move {
                                            // Get current flow state and determine action
                                            let manager_guard = flow_manager_clone.read().await;
                                            
                                            if let Some(manager) = manager_guard.as_ref() {
                                                let current_state = manager.get_state().await;
                                                drop(manager_guard); // Release the read lock
                                                
                                                match current_state {
                                                    FlowState::Idle | FlowState::Completed | FlowState::Error | FlowState::Cancelled => {
                                                        // Start recording - play boowomp.mp3
                                                        emit_audio_feedback(&app_handle_clone, "boowomp.mp3");
                                                        let mut manager_guard = flow_manager_clone.write().await;
                                                        if let Some(manager) = manager_guard.as_mut() {
                                                            let flow_manager_clone_for_start = Arc::clone(&flow_manager_clone);
                                                            match manager.start_flow(app_handle_clone.clone(), flow_manager_clone_for_start, "shortcut".to_string()).await {
                                                                Ok(_) => println!("✅ Recording started via global shortcut"),
                                                                Err(e) => {
                                                                    eprintln!("❌ Failed to start recording: {}", e);
                                                                    emit_audio_feedback(&app_handle_clone, "pipe.mp3");
                                                                }
                                                            }
                                                        }
                                                    }
                                                    FlowState::Recording => {
                                                        // Stop recording - play bamboo_hit.mp3
                                                        emit_audio_feedback(&app_handle_clone, "bamboo_hit.mp3");
                                                        let mut manager_guard = flow_manager_clone.write().await;
                                                        if let Some(manager) = manager_guard.as_mut() {
                                                            match manager.stop_flow("shortcut".to_string()).await {
                                                                Ok(_) => println!("🛑 Recording stopped via global shortcut"),
                                                                Err(e) => {
                                                                    eprintln!("❌ Failed to stop recording: {}", e);
                                                                    emit_audio_feedback(&app_handle_clone, "pipe.mp3");
                                                                }
                                                            }
                                                        }
                                                    }
                                                    FlowState::Processing => {
                                                        // Cancel transcription - play pipe.mp3
                                                        emit_audio_feedback(&app_handle_clone, "pipe.mp3");
                                                        let mut manager_guard = flow_manager_clone.write().await;
                                                        if let Some(manager) = manager_guard.as_mut() {
                                                            manager.cancel_flow("shortcut".to_string()).await;
                                                            println!("❌ Flow cancelled via global shortcut");
                                                        }
                                                    }
                                                }
                                            }
                                        });
                                    }
                                })
                                .build()
                        );
                        
                        match handler_result {
                            Ok(_) => {
                                // Register the shortcut
                                match app.global_shortcut().register(shortcut) {
                                    Ok(_) => println!("✅ Global shortcut registered successfully: {}", shortcut_string),
                                    Err(e) => eprintln!("❌ Failed to register global shortcut: {}", e),
                                }
                            }
                            Err(e) => eprintln!("❌ Failed to initialize global shortcut plugin: {}", e),
                        }
                    }
                    Err(e) => eprintln!("❌ Failed to parse shortcut '{}': {}", shortcut_string, e),
                }
            }
            
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize {
                    width: 270.0,
                    height: 48.0,
                }));
            }

            // Initialize the flow manager in an async context
            let flow_manager_clone = flow_manager.clone();
            tauri::async_runtime::spawn(async move {
                let mut manager_guard = flow_manager_clone.write().await;
                *manager_guard = Some(FlowManager::new());
                println!("Flow manager initialized");
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
