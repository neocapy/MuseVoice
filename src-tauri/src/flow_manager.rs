use crate::flow::{Flow, FlowCallback, FlowEvent, FlowState};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, RwLock};

// Global state
pub type FlowManagerState = Arc<RwLock<Option<FlowManager>>>;

pub struct FlowManager {
    current_flow: Option<Arc<Flow>>,
    stop_sender: Option<oneshot::Sender<()>>,
    retry_wav_data: Option<Vec<u8>>, // Store WAV data for retry functionality
    model: String,
}

impl FlowManager {
    pub fn new() -> Self {
        Self {
            current_flow: None,
            stop_sender: None,
            retry_wav_data: None,
            model: "whisper-1".to_string(),
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
            FlowEvent::WaveformChunk { bins, avg_rms } => {
                let payload = WaveformChunkPayload { bins, avg_rms };
                let _ = app_handle_clone.emit("waveform-chunk", payload);
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
        let flow = Arc::new(Flow::new(callback, self.model.clone()));
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
        let flow = Arc::new(Flow::new(callback, self.model.clone()));
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

    pub fn set_model(&mut self, model: String) -> Result<(), String> {
        // Accept only allowed models
        match model.as_str() {
            "whisper-1" | "gpt-4o-transcribe" => {
                self.model = model;
                Ok(())
            }
            _ => Err("Invalid model".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct StatusResponse {
    pub state: FlowState,
    pub samples: Option<usize>,
}

#[derive(Serialize, Clone)]
pub struct WaveformChunkPayload {
    pub bins: Vec<f32>,
    pub avg_rms: f32,
}
