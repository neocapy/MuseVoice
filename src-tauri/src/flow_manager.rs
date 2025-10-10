use crate::flow::{Flow, FlowCallback, FlowEvent, FlowMode, FlowState};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, RwLock};

// Global state
pub type FlowManagerState = Arc<RwLock<Option<FlowManager>>>;

enum CallbackMode {
    Full,      // Handle all events
    RetryOnly, // Handle only essential events for retry
}

pub struct FlowManager {
    current_flow: Option<Arc<Flow>>,
    stop_sender: Option<oneshot::Sender<()>>,
    retry_audio_data: Option<Vec<u8>>, // Store audio data (WebM format) for retry functionality
    model: String,
    rewrite_enabled: bool,
}

impl FlowManager {
    pub fn new() -> Self {
        Self {
            current_flow: None,
            stop_sender: None,
            retry_audio_data: None,
            model: "whisper-1".to_string(),
            rewrite_enabled: false,
        }
    }

    fn create_flow_callback(app_handle: AppHandle, flow_manager_state: FlowManagerState, mode: CallbackMode) -> FlowCallback {
        let app_handle_clone = app_handle.clone();
        let flow_manager_weak = Arc::downgrade(&flow_manager_state);
        Arc::new(move |event| {
            match (&mode, event) {
                // Events always handled
                (_, FlowEvent::StateChanged(state)) => {
                    let _ = app_handle_clone.emit("flow-state-changed", &state);
                }
                (_, FlowEvent::TranscriptionResult(text)) => {
                    // Clear retry data on successful transcription
                    if let Some(manager_arc) = flow_manager_weak.upgrade() {
                        tokio::spawn(async move {
                            let mut manager_guard = manager_arc.write().await;
                            if let Some(manager) = manager_guard.as_mut() {
                                manager.clear_audio_data();
                            }
                        });
                    }
                    let _ = app_handle_clone.emit("transcription-result", &text);
                    let _ = app_handle_clone.emit("retry-available", false);
                }
                (_, FlowEvent::AudioFeedback(sound_file)) => {
                    let _ = app_handle_clone.emit("audio-feedback", &sound_file);
                }
                (_, FlowEvent::Error(error)) => {
                    // Emit retry availability when there's an error and we have audio data
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
                
                // Events only handled in Full mode
                (CallbackMode::Full, FlowEvent::SampleCount(count)) => {
                    let _ = app_handle_clone.emit("sample-count", count);
                }
                (CallbackMode::Full, FlowEvent::WaveformChunk { bins, avg_rms }) => {
                    let payload = WaveformChunkPayload { bins, avg_rms };
                    let _ = app_handle_clone.emit("waveform-chunk", payload);
                }
                (CallbackMode::Full, FlowEvent::AudioFileSaved(path)) => {
                    let _ = app_handle_clone.emit("audio-file-saved", &path);
                }
                (CallbackMode::Full, FlowEvent::AudioDataReady(audio_data)) => {
                    // Store audio data directly in FlowManager
                    if let Some(manager_arc) = flow_manager_weak.upgrade() {
                        tokio::spawn(async move {
                            let mut manager_guard = manager_arc.write().await;
                            if let Some(manager) = manager_guard.as_mut() {
                                manager.store_audio_data(audio_data);
                            }
                        });
                    }
                }
                
                // Ignore other combinations (RetryOnly mode with Full-only events)
                _ => {}
            }
        })
    }

    pub async fn start_flow(&mut self, app_handle: AppHandle, flow_manager_state: FlowManagerState, origin: String) -> Result<(), String> {
        // Cancel any existing flow
        self.cancel_flow(origin.clone()).await;

        // Create stop channel
        let (stop_sender, stop_receiver) = oneshot::channel();

        // Create callback for flow events
        let callback = Self::create_flow_callback(app_handle, flow_manager_state, CallbackMode::Full);

        // Create and start flow
        let flow = Arc::new(Flow::new(callback, self.model.clone(), origin.clone(), self.rewrite_enabled));

        // Store references
        self.current_flow = Some(Arc::clone(&flow));
        self.stop_sender = Some(stop_sender);

        // Start the flow in a background task
        tokio::spawn(async move {
            if let Err(e) = flow.run(FlowMode::RecordAndTranscribe { stop_signal: stop_receiver }).await {
                eprintln!("Flow error: {}", e);
            }
        });

        Ok(())
    }

    pub async fn stop_flow(&mut self, _origin: String) -> Result<(), String> {
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

    pub async fn cancel_flow(&mut self, _origin: String) {
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

    pub fn store_audio_data(&mut self, audio_data: Vec<u8>) {
        self.retry_audio_data = Some(audio_data);
    }

    pub fn clear_audio_data(&mut self) {
        self.retry_audio_data = None;
    }

    pub fn has_retry_data(&self) -> bool {
        self.retry_audio_data.is_some()
    }

    pub async fn retry_transcription(&mut self, app_handle: AppHandle, flow_manager_state: FlowManagerState, origin: String) -> Result<(), String> {
        // Get the stored audio data
        let audio_data = self.retry_audio_data.clone().ok_or_else(|| {
            "No recorded audio available for retry".to_string()
        })?;

        // Cancel any existing flow
        self.cancel_flow(origin.clone()).await;

        // Create callback for flow events
        let callback = Self::create_flow_callback(app_handle, flow_manager_state, CallbackMode::RetryOnly);

        // Create flow and run transcription only
        let flow = Arc::new(Flow::new(callback, self.model.clone(), origin.clone(), self.rewrite_enabled));
        let flow_clone = Arc::clone(&flow);

        // Store reference
        self.current_flow = Some(flow);

        // Start the transcription-only flow in a background task
        tokio::spawn(async move {
            if let Err(e) = flow_clone.run(FlowMode::TranscribeOnly { audio_data }).await {
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

    pub fn set_rewrite_enabled(&mut self, enabled: bool) {
        self.rewrite_enabled = enabled;
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
