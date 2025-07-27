use reqwest::blocking::multipart;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionStatus {
    Idle,
    InProgress,
    Completed(String),
    Error(String),
}

#[derive(Debug)]
struct TranscriptionState {
    status: TranscriptionStatus,
}

impl TranscriptionState {
    fn new() -> Self {
        Self {
            status: TranscriptionStatus::Idle,
        }
    }
}

pub struct TranscriptionManager {
    state: Arc<Mutex<TranscriptionState>>,
    cancel_flag: Arc<AtomicBool>,
    thread_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    text: String,
}

impl TranscriptionManager {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(TranscriptionState::new())),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            thread_handle: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start_transcription(&self, wav_data: Vec<u8>) -> Result<(), String> {
        // Check if transcription is already in progress
        {
            let state = self.state.lock().map_err(|e| format!("Lock error: {}", e))?;
            if matches!(state.status, TranscriptionStatus::InProgress) {
                return Err("Transcription already in progress".to_string());
            }
        }

        // Cancel any existing transcription first
        self.cancel_transcription();

        // Reset state
        {
            let mut state = self.state.lock().map_err(|e| format!("Lock error: {}", e))?;
            state.status = TranscriptionStatus::InProgress;
        }

        self.cancel_flag.store(false, Ordering::Relaxed);

        // Spawn transcription thread
        let state_clone = Arc::clone(&self.state);
        let cancel_flag_clone = Arc::clone(&self.cancel_flag);
        let thread_handle_arc = Arc::clone(&self.thread_handle);

        let handle = thread::spawn(move || {
            let result = Self::perform_transcription(wav_data, &cancel_flag_clone);
            
            // Update state with result
            if let Ok(mut state) = state_clone.lock() {
                if !cancel_flag_clone.load(Ordering::Relaxed) {
                    state.status = match result {
                        Ok(text) => {
                            println!("Transcription completed: {}", text);
                            TranscriptionStatus::Completed(text)
                        }
                        Err(e) => {
                            println!("Transcription failed: {}", e);
                            TranscriptionStatus::Error(e)
                        }
                    };
                } else {
                    state.status = TranscriptionStatus::Idle;
                    println!("Transcription was cancelled");
                }
            }
        });

        *thread_handle_arc.lock().map_err(|e| format!("Lock error: {}", e))? = Some(handle);

        Ok(())
    }

    pub fn get_status(&self) -> Result<TranscriptionStatus, String> {
        let state = self.state.lock().map_err(|e| format!("Lock error: {}", e))?;
        Ok(state.status.clone())
    }

    pub fn cancel_transcription(&self) {
        // Signal cancellation
        self.cancel_flag.store(true, Ordering::Relaxed);

        // Wait for thread to finish if it's running
        if let Ok(mut handle_guard) = self.thread_handle.lock() {
            if let Some(handle) = handle_guard.take() {
                // Give the thread a moment to respond to cancellation
                thread::sleep(Duration::from_millis(100));
                let _ = handle.join();
            }
        }

        // Reset state to idle
        if let Ok(mut state) = self.state.lock() {
            state.status = TranscriptionStatus::Idle;
        }
    }

    pub fn is_idle(&self) -> bool {
        if let Ok(state) = self.state.lock() {
            matches!(state.status, TranscriptionStatus::Idle)
        } else {
            false
        }
    }

    fn perform_transcription(wav_data: Vec<u8>, cancel_flag: &Arc<AtomicBool>) -> Result<String, String> {
        // Check for API key
        let api_key = env::var("OPENAI_API_KEY")
            .map_err(|_| "OPENAI_API_KEY environment variable not set".to_string())?;

        if api_key.trim().is_empty() {
            return Err("OPENAI_API_KEY is empty".to_string());
        }

        // Check for cancellation before starting
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("Transcription cancelled before starting".to_string());
        }

        // Create HTTP client
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        // Create multipart form
        let form = multipart::Form::new()
            .part(
                "file",
                multipart::Part::bytes(wav_data)
                    .file_name("audio.wav")
                    .mime_str("audio/wav")
                    .map_err(|e| format!("Failed to create file part: {}", e))?,
            )
            .text("model", "gpt-4o-transcribe");

        // Check for cancellation before making request
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("Transcription cancelled before request".to_string());
        }

        println!("Sending transcription request to OpenAI...");

        // Make the request
        let response = client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", api_key))
            .multipart(form)
            .send()
            .map_err(|e| format!("Failed to send request: {}", e))?;

        // Check for cancellation after request
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("Transcription cancelled after request".to_string());
        }

        // Check response status
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("OpenAI API error {}: {}", status, error_text));
        }

        // Parse JSON response
        let openai_response: OpenAIResponse = response
            .json()
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(openai_response.text)
    }
}
