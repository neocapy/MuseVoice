// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

mod encode;
mod flow;

use flow::{Flow, FlowCallback, FlowEvent, FlowState};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{oneshot, RwLock};
use tauri_plugin_clipboard_manager::ClipboardExt;

// Global state
type FlowManagerState = Arc<RwLock<Option<FlowManager>>>;

pub struct FlowManager {
    current_flow: Option<Arc<Flow>>,
    stop_sender: Option<oneshot::Sender<()>>,
    retry_wav_data: Option<Vec<u8>>, // Store WAV data for retry functionality
}

impl FlowManager {
    pub fn new() -> Self {
        Self {
            current_flow: None,
            stop_sender: None,
            retry_wav_data: None,
        }
    }

    pub async fn start_flow(&mut self, app_handle: AppHandle, flow_manager_state: FlowManagerState) -> Result<(), String> {
        // Cancel any existing flow
        self.cancel_flow().await;

        // Create stop channel
        let (stop_sender, stop_receiver) = oneshot::channel();

        // Create callback for flow events
        let app_handle_clone = app_handle.clone();
        let flow_manager_weak = Arc::downgrade(&flow_manager_state);
        let callback: FlowCallback = Arc::new(move |event| match event {
            FlowEvent::StateChanged(state) => {
                let _ = app_handle_clone.emit("flow-state-changed", &state);
            }
            FlowEvent::SampleCount(count) => {
                let _ = app_handle_clone.emit("sample-count", count);
            }
            FlowEvent::TranscriptionResult(text) => {
                // Clear retry data on successful transcription
                if let Some(manager_arc) = flow_manager_weak.upgrade() {
                    tokio::spawn(async move {
                        let mut manager_guard = manager_arc.write().await;
                        if let Some(manager) = manager_guard.as_mut() {
                            manager.clear_wav_data();
                        }
                    });
                }
                let _ = app_handle_clone.emit("transcription-result", &text);
                let _ = app_handle_clone.emit("retry-available", false);
            }
            FlowEvent::WavFileSaved(path) => {
                let _ = app_handle_clone.emit("wav-file-saved", &path);
            }
            FlowEvent::WavDataReady(wav_data) => {
                // Store WAV data directly in FlowManager
                if let Some(manager_arc) = flow_manager_weak.upgrade() {
                    tokio::spawn(async move {
                        let mut manager_guard = manager_arc.write().await;
                        if let Some(manager) = manager_guard.as_mut() {
                            manager.store_wav_data(wav_data);
                        }
                    });
                }
            }
            FlowEvent::Error(error) => {
                // Emit retry availability when there's an error and we have WAV data
                let app_handle_clone2 = app_handle_clone.clone();
                if let Some(manager_arc) = flow_manager_weak.upgrade() {
                    tokio::spawn(async move {
                        let manager_guard = manager_arc.read().await;
                        if let Some(manager) = manager_guard.as_ref() {
                            let retry_available = manager.has_retry_data();
                            let _ = app_handle_clone2.emit("retry-available", retry_available);
                        }
                    });
                }
                let _ = app_handle_clone.emit("flow-error", &error);
            }
        });

        // Create and start flow
        let flow = Arc::new(Flow::new(callback));
        let flow_clone = Arc::clone(&flow);

        // Store references
        self.current_flow = Some(flow);
        self.stop_sender = Some(stop_sender);

        // Start the flow in a background task
        tokio::spawn(async move {
            if let Err(e) = flow_clone.run(stop_receiver).await {
                eprintln!("Flow error: {}", e);
            }
        });

        Ok(())
    }

    pub async fn stop_flow(&mut self) -> Result<(), String> {
        println!("Flow manager: Stopping flow");
        if let Some(sender) = self.stop_sender.take() {
            println!("Flow manager: Sending stop signal");
            match sender.send(()) {
                Ok(_) => Ok(()),
                Err(e) => {
                    eprintln!("Flow manager: Failed to send stop signal: {:?}", e);
                    Err("Failed to send stop signal".to_string())
                }
            }
        } else {
            Err("No active flow to stop".to_string())
        }
    }

    pub async fn cancel_flow(&mut self) {
        if let Some(flow) = &self.current_flow {
            flow.cancel();
        }
        self.current_flow = None;
        self.stop_sender = None;
    }

    pub async fn get_state(&self) -> FlowState {
        if let Some(flow) = &self.current_flow {
            flow.get_state().await
        } else {
            FlowState::Idle
        }
    }

    pub fn store_wav_data(&mut self, wav_data: Vec<u8>) {
        self.retry_wav_data = Some(wav_data);
    }

    pub fn clear_wav_data(&mut self) {
        self.retry_wav_data = None;
    }

    pub fn has_retry_data(&self) -> bool {
        self.retry_wav_data.is_some()
    }

    pub async fn retry_transcription(&mut self, app_handle: AppHandle, flow_manager_state: FlowManagerState) -> Result<(), String> {
        // Get the stored WAV data
        let wav_data = self.retry_wav_data.clone().ok_or_else(|| {
            "No recorded audio available for retry".to_string()
        })?;

        // Cancel any existing flow
        self.cancel_flow().await;

        // Create callback for flow events
        let app_handle_clone = app_handle.clone();
        let flow_manager_weak = Arc::downgrade(&flow_manager_state);
        let callback: FlowCallback = Arc::new(move |event| match event {
            FlowEvent::StateChanged(state) => {
                let _ = app_handle_clone.emit("flow-state-changed", &state);
            }
            FlowEvent::TranscriptionResult(text) => {
                // Clear retry data on successful transcription
                if let Some(manager_arc) = flow_manager_weak.upgrade() {
                    tokio::spawn(async move {
                        let mut manager_guard = manager_arc.write().await;
                        if let Some(manager) = manager_guard.as_mut() {
                            manager.clear_wav_data();
                        }
                    });
                }
                let _ = app_handle_clone.emit("transcription-result", &text);
                let _ = app_handle_clone.emit("retry-available", false);
            }
            FlowEvent::Error(error) => {
                // Emit retry availability when there's an error and we have WAV data
                let app_handle_clone2 = app_handle_clone.clone();
                if let Some(manager_arc) = flow_manager_weak.upgrade() {
                    tokio::spawn(async move {
                        let manager_guard = manager_arc.read().await;
                        if let Some(manager) = manager_guard.as_ref() {
                            let retry_available = manager.has_retry_data();
                            let _ = app_handle_clone2.emit("retry-available", retry_available);
                        }
                    });
                }
                let _ = app_handle_clone.emit("flow-error", &error);
            }
            _ => {} // Ignore other events for retry
        });

        // Create flow and run transcription only
        let flow = Arc::new(Flow::new(callback));
        let flow_clone = Arc::clone(&flow);

        // Store reference
        self.current_flow = Some(flow);

        // Start the transcription-only flow in a background task
        tokio::spawn(async move {
            if let Err(e) = flow_clone.run_transcription_only(wav_data).await {
                eprintln!("Retry transcription error: {}", e);
            }
        });

        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct StatusResponse {
    pub state: FlowState,
    pub samples: Option<usize>,
}

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
) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        let current_state = manager.get_state().await;

        match current_state {
            FlowState::Idle | FlowState::Completed | FlowState::Error | FlowState::Cancelled => {
                let flow_manager_clone = Arc::clone(&flow_manager.inner());
                manager.start_flow(app_handle, flow_manager_clone).await?;
                Ok("Audio recording started successfully".to_string())
            }
            _ => Err("Cannot start recording: flow is not idle".to_string()),
        }
    } else {
        Err("Flow manager not initialized".to_string())
    }
}

#[tauri::command]
async fn stop_audio_stream(flow_manager: State<'_, FlowManagerState>) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        let current_state = manager.get_state().await;

        match current_state {
            FlowState::Recording => {
                manager.stop_flow().await?;
                Ok("Recording stopped, starting transcription...".to_string())
            }
            _ => Err("Cannot stop recording: not currently recording".to_string()),
        }
    } else {
        Err("Flow manager not initialized".to_string())
    }
}

#[tauri::command]
async fn cancel_transcription(flow_manager: State<'_, FlowManagerState>) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        manager.cancel_flow().await;
        Ok("Flow cancelled".to_string())
    } else {
        Err("Flow manager not initialized".to_string())
    }
}



#[tauri::command]
async fn retry_transcription(
    flow_manager: State<'_, FlowManagerState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let mut manager_guard = flow_manager.write().await;

    if let Some(manager) = manager_guard.as_mut() {
        let flow_manager_clone = Arc::clone(&flow_manager.inner());
        manager.retry_transcription(app_handle, flow_manager_clone).await?;
        Ok("Retrying transcription...".to_string())
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
            copy_to_clipboard
        ])
        .setup(move |_app| {
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
