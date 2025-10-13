// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

mod flow;
mod flow_manager;
mod stream_processor;
pub mod ebml;
pub mod opus;
pub mod webm;

use flow_manager::{FlowManager, FlowManagerState, StatusResponse, Options, OptionsPatch};
use crate::flow::FlowState;
use std::sync::Arc;
use tauri::{AppHandle, State, Emitter, Manager};
use tauri::menu::{Menu, MenuItem, ContextMenu};
use tokio::sync::RwLock;
use tauri_plugin_clipboard_manager::ClipboardExt;
use serde::Serialize;

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
    app_handle: AppHandle,
    model: String,
) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        let applied = manager.update_options(OptionsPatch { model: Some(model), rewrite_enabled: None, omit_final_punctuation: None })?;
        let full = manager.options();
        let _ = app_handle.emit("options-changed", OptionsChangedEvent { full, patch: applied });
        Ok("Model updated".to_string())
    } else {
        Err("Flow manager not initialized".to_string())
    }
}

#[tauri::command]
async fn set_rewrite_enabled(
    flow_manager: State<'_, FlowManagerState>,
    app_handle: AppHandle,
    enabled: bool,
) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        let applied = manager.update_options(OptionsPatch { model: None, rewrite_enabled: Some(enabled), omit_final_punctuation: None })?;
        let full = manager.options();
        let _ = app_handle.emit("options-changed", OptionsChangedEvent { full, patch: applied });
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

#[derive(Serialize, Clone)]
struct OptionsChangedEvent {
    full: Options,
    patch: OptionsPatch,
}

#[tauri::command]
async fn get_options(flow_manager: State<'_, FlowManagerState>) -> Result<Options, String> {
    let manager_guard = flow_manager.read().await;
    if let Some(manager) = manager_guard.as_ref() {
        Ok(manager.options())
    } else {
        Ok(Options { model: "whisper-1".to_string(), rewrite_enabled: false, omit_final_punctuation: false })
    }
}

#[tauri::command]
async fn update_options(
    flow_manager: State<'_, FlowManagerState>,
    app_handle: AppHandle,
    patch: OptionsPatch,
) -> Result<Options, String> {
    let mut manager_guard = flow_manager.write().await;
    if let Some(manager) = manager_guard.as_mut() {
        let applied = manager.update_options(patch)?;
        let full = manager.options();
        let _ = app_handle.emit("options-changed", OptionsChangedEvent { full: full.clone(), patch: applied });
        Ok(full)
    } else {
        Err("Flow manager not initialized".to_string())
    }
}

#[tauri::command]
async fn show_context_menu(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let settings_item = MenuItem::with_id(&app, "settings", "Settings", true, None::<&str>)
        .map_err(|e| format!("Failed to create settings menu item: {}", e))?;
    let minimize_item = MenuItem::with_id(&app, "minimize", "Minimize", true, None::<&str>)
        .map_err(|e| format!("Failed to create minimize menu item: {}", e))?;
    let close_item = MenuItem::with_id(&app, "close", "Close", true, None::<&str>)
        .map_err(|e| format!("Failed to create close menu item: {}", e))?;

    let menu = Menu::with_items(&app, &[
        &settings_item,
        &minimize_item,
        &close_item,
    ]).map_err(|e| format!("Failed to create menu: {}", e))?;

    menu.popup(window)
        .map_err(|e| format!("Failed to show context menu: {}", e))?;

    Ok(())
}

#[tauri::command]
async fn open_settings_window(app: AppHandle) -> Result<(), String> {
    if let Some(_existing) = app.get_webview_window("settings") {
        return Err("Settings window already open".to_string());
    }

    use tauri::{WebviewUrl, WebviewWindowBuilder};
    
    let _window = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Settings")
        .inner_size(400.0, 500.0)
        .resizable(false)
        .center()
        .visible(true)
        .build()
        .map_err(|e| format!("Failed to create settings window: {}", e))?;

    Ok(())
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
            set_rewrite_enabled,
            get_options,
            update_options,
            show_context_menu,
            open_settings_window
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

            // Initialize the flow manager in an async context
            let flow_manager_clone = flow_manager.clone();
            tauri::async_runtime::spawn(async move {
                let mut manager_guard = flow_manager_clone.write().await;
                *manager_guard = Some(FlowManager::new());
                println!("Flow manager initialized");
            });

            let app_handle = app.handle().clone();
            app.on_menu_event(move |app, event| {
                let windows = app.webview_windows();
                let window = windows.get("main");
                match event.id().as_ref() {
                    "minimize" => {
                        if let Some(window) = window {
                            let _ = window.minimize();
                        }
                    }
                    "close" => {
                        if let Some(window) = window {
                            let _ = window.close();
                        }
                    }
                    "settings" => {
                        let app_handle_clone = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = open_settings_window(app_handle_clone).await {
                                if e != "Settings window already open" {
                                    eprintln!("Failed to open settings window: {}", e);
                                }
                            }
                        });
                    }
                    _ => {}
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
