use crate::flow::{Flow, FlowCallback, FlowEvent, FlowMode, FlowState};
use crate::audio_output::AudioOutputManager;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, RwLock};
use directories::ProjectDirs;

// Global state
pub type FlowManagerState = Arc<RwLock<Option<FlowManager>>>;

enum CallbackMode {
    Full,      // Handle all events
    RetryOnly, // Handle only essential events for retry
}

const DEFAULT_PROMPT_TEXT: &str = "Please fix and rewrite the following dictated text to handle common speech-to-text issues:\n\
- Convert phonetic alphabet spelling (alpha bravo charlie) to actual letters (\"ABC\"); choose upper or lowercase based on context\n\
- When appropriate, convert spoken numbers to numerals: \"one two three\" → \"123\"\n\
- Replace spoken punctuation words with actual punctuation (\"comma\" → \",\", \"period\" → \".\", \"question mark\" → \"?\", etc.)\n\
- Handle formatting commands (\"camel case whatever\" → \"camelCase\", \"title case whatever\" → \"TitleCase\", etc.)\n\
- \"new paragraph\" → \"\\n\\n\"\n\
- \"new line\" → \"\\n\"\n\
- \"no space hello there\" -> \"hellothere\"\n\
- Fix any other obvious dictation artifacts\n\
\n\
Original text: {}\n\
\n\
Return ONLY the corrected text, no explanations or formatting:";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RewritePrompt {
    pub id: String,
    pub name: String,
    pub text: String,
}

pub struct FlowManager {
    current_flow: Option<Arc<Flow>>,
    stop_sender: Option<oneshot::Sender<()>>,
    retry_audio_data: Option<Vec<u8>>,
    model: String,
    rewrite_enabled: bool,
    omit_final_punctuation: bool,
    selected_prompt_id: String,
    custom_prompts: Vec<RewritePrompt>,
    api_key: String,
    shortcuts: String,
    audio_manager: Arc<Mutex<AudioOutputManager>>,
}

impl FlowManager {
    pub fn new(audio_manager: Arc<Mutex<AudioOutputManager>>) -> Self {
        let settings = Self::load_settings();
        Self {
            current_flow: None,
            stop_sender: None,
            retry_audio_data: None,
            model: settings.model,
            rewrite_enabled: settings.rewrite_enabled,
            omit_final_punctuation: settings.omit_final_punctuation,
            selected_prompt_id: settings.selected_prompt_id,
            custom_prompts: settings.custom_prompts,
            api_key: settings.api_key,
            shortcuts: settings.shortcuts,
            audio_manager,
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

    pub async fn start_flow(&mut self, app_handle: AppHandle, flow_manager_state: FlowManagerState) -> Result<(), String> {
        if !self.has_valid_api_key() {
            return Err("OpenAI API key is required. Please set it in Settings or via OPENAI_API_KEY environment variable.".to_string());
        }

        self.cancel_flow().await;

        let (stop_sender, stop_receiver) = oneshot::channel();

        let callback = Self::create_flow_callback(app_handle, flow_manager_state, CallbackMode::Full);

        let prompt_text = self.get_selected_prompt_text();
        let api_key = self.get_effective_api_key();
        let flow = Arc::new(Flow::new(
            callback,
            self.model.clone(),
            self.rewrite_enabled,
            self.omit_final_punctuation,
            Arc::clone(&self.audio_manager),
            prompt_text,
            api_key,
        ));

        self.current_flow = Some(Arc::clone(&flow));
        self.stop_sender = Some(stop_sender);

        tokio::spawn(async move {
            if let Err(e) = flow.run(FlowMode::RecordAndTranscribe { stop_signal: stop_receiver }).await {
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

    pub fn store_audio_data(&mut self, audio_data: Vec<u8>) {
        self.retry_audio_data = Some(audio_data);
    }

    pub fn clear_audio_data(&mut self) {
        self.retry_audio_data = None;
    }

    pub fn has_retry_data(&self) -> bool {
        self.retry_audio_data.is_some()
    }

    pub async fn retry_transcription(&mut self, app_handle: AppHandle, flow_manager_state: FlowManagerState) -> Result<(), String> {
        let audio_data = self.retry_audio_data.clone().ok_or_else(|| {
            "No recorded audio available for retry".to_string()
        })?;

        self.cancel_flow().await;

        let callback = Self::create_flow_callback(app_handle, flow_manager_state, CallbackMode::RetryOnly);

        let prompt_text = self.get_selected_prompt_text();
        let api_key = self.get_effective_api_key();
        let flow = Arc::new(Flow::new(
            callback,
            self.model.clone(),
            self.rewrite_enabled,
            self.omit_final_punctuation,
            Arc::clone(&self.audio_manager),
            prompt_text,
            api_key,
        ));
        let flow_clone = Arc::clone(&flow);

        self.current_flow = Some(flow);

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

    fn get_config_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "muse", "app")
            .map(|proj_dirs| proj_dirs.config_dir().join("settings.json"))
    }

    fn load_settings() -> PersistedSettings {
        if let Some(config_path) = Self::get_config_path() {
            if config_path.exists() {
                match fs::read_to_string(&config_path) {
                    Ok(content) => {
                        match serde_json::from_str::<PersistedSettings>(&content) {
                            Ok(settings) => {
                                println!("Loaded settings from {:?}", config_path);
                                return settings;
                            }
                            Err(e) => {
                                eprintln!("Failed to parse settings file: {}, using defaults", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to read settings file: {}, using defaults", e);
                    }
                }
            }
        }
        PersistedSettings::default()
    }

    fn save_settings(&self) -> Result<(), String> {
        let settings = PersistedSettings {
            model: self.model.clone(),
            rewrite_enabled: self.rewrite_enabled,
            omit_final_punctuation: self.omit_final_punctuation,
            selected_prompt_id: self.selected_prompt_id.clone(),
            custom_prompts: self.custom_prompts.clone(),
            api_key: self.api_key.clone(),
            shortcuts: self.shortcuts.clone(),
        };

        let config_path = Self::get_config_path()
            .ok_or_else(|| "Could not determine config directory".to_string())?;

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!("Failed to create config directory: {}", e)
            })?;
        }

        let json = serde_json::to_string_pretty(&settings)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        fs::write(&config_path, json)
            .map_err(|e| format!("Failed to write settings file: {}", e))?;

        println!("Saved settings to {:?}", config_path);
        Ok(())
    }

    fn get_selected_prompt_text(&self) -> String {
        if self.selected_prompt_id == "default" {
            return DEFAULT_PROMPT_TEXT.to_string();
        }

        self.custom_prompts
            .iter()
            .find(|p| p.id == self.selected_prompt_id)
            .map(|p| p.text.clone())
            .unwrap_or_else(|| {
                eprintln!("Selected prompt '{}' not found, using default", self.selected_prompt_id);
                DEFAULT_PROMPT_TEXT.to_string()
            })
    }

    pub fn options(&self) -> Options {
        let mut all_prompts = vec![RewritePrompt {
            id: "default".to_string(),
            name: "Default (Built-in)".to_string(),
            text: DEFAULT_PROMPT_TEXT.to_string(),
        }];
        all_prompts.extend(self.custom_prompts.clone());

        let api_key_from_env = std::env::var("OPENAI_API_KEY").is_ok();

        Options {
            model: self.model.clone(),
            rewrite_enabled: self.rewrite_enabled,
            omit_final_punctuation: self.omit_final_punctuation,
            selected_prompt_id: self.selected_prompt_id.clone(),
            custom_prompts: all_prompts,
            api_key: self.api_key.clone(),
            api_key_from_env,
            shortcuts: self.shortcuts.clone(),
        }
    }

    fn has_valid_api_key(&self) -> bool {
        if let Ok(env_key) = std::env::var("OPENAI_API_KEY") {
            return !env_key.trim().is_empty();
        }
        !self.api_key.trim().is_empty()
    }

    pub fn get_effective_api_key(&self) -> String {
        std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| self.api_key.clone())
    }

    pub fn get_config_dir() -> Option<PathBuf> {
        ProjectDirs::from("com", "muse", "app")
            .map(|proj_dirs| proj_dirs.config_dir().to_path_buf())
    }

    pub fn update_options(&mut self, patch: OptionsPatch) -> Result<OptionsPatch, String> {
        let mut applied = OptionsPatch::default();

        if let Some(model) = patch.model {
            self.set_model(model.clone())?;
            applied.model = Some(model);
        }
        if let Some(enabled) = patch.rewrite_enabled {
            self.set_rewrite_enabled(enabled);
            applied.rewrite_enabled = Some(enabled);
        }
        if let Some(omit) = patch.omit_final_punctuation {
            self.omit_final_punctuation = omit;
            applied.omit_final_punctuation = Some(omit);
        }
        if let Some(selected_id) = patch.selected_prompt_id {
            if selected_id == "default" || self.custom_prompts.iter().any(|p| p.id == selected_id) {
                self.selected_prompt_id = selected_id.clone();
                applied.selected_prompt_id = Some(selected_id);
            } else {
                return Err(format!("Invalid prompt ID: {}", selected_id));
            }
        }
        if let Some(prompts) = patch.custom_prompts {
            let filtered_prompts: Vec<RewritePrompt> = prompts
                .into_iter()
                .filter(|p| p.id != "default")
                .collect();
            self.custom_prompts = filtered_prompts.clone();
            applied.custom_prompts = Some(filtered_prompts);
        }
        if let Some(api_key) = patch.api_key {
            self.api_key = api_key.clone();
            applied.api_key = Some(api_key);
        }
        if let Some(shortcuts) = patch.shortcuts {
            self.shortcuts = shortcuts.clone();
            applied.shortcuts = Some(shortcuts);
        }

        self.save_settings()?;
        Ok(applied)
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

#[derive(Serialize, Deserialize, Clone)]
pub struct Options {
    pub model: String,
    pub rewrite_enabled: bool,
    pub omit_final_punctuation: bool,
    pub selected_prompt_id: String,
    pub custom_prompts: Vec<RewritePrompt>,
    pub api_key: String,
    pub api_key_from_env: bool,
    pub shortcuts: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct OptionsPatch {
    pub model: Option<String>,
    pub rewrite_enabled: Option<bool>,
    pub omit_final_punctuation: Option<bool>,
    pub selected_prompt_id: Option<String>,
    pub custom_prompts: Option<Vec<RewritePrompt>>,
    pub api_key: Option<String>,
    pub shortcuts: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct PersistedSettings {
    pub model: String,
    pub rewrite_enabled: bool,
    pub omit_final_punctuation: bool,
    pub selected_prompt_id: String,
    pub custom_prompts: Vec<RewritePrompt>,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_shortcuts")]
    pub shortcuts: String,
}

fn default_shortcuts() -> String {
    "Alt+Slash".to_string()
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            model: "whisper-1".to_string(),
            rewrite_enabled: false,
            omit_final_punctuation: false,
            selected_prompt_id: "default".to_string(),
            custom_prompts: Vec::new(),
            api_key: String::new(),
            shortcuts: default_shortcuts(),
        }
    }
}
