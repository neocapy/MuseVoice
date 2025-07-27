// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

mod audio;
mod encode;
mod transcribe;

use audio::{AudioManager, RecordingStatus};
use transcribe::{TranscriptionManager, TranscriptionStatus};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::State;

// Global state
type AudioManagerState = Arc<Mutex<Option<AudioManager>>>;
type TranscriptionManagerState = Arc<Mutex<Option<TranscriptionManager>>>;

#[derive(Serialize, Deserialize)]
pub struct StatusResponse {
    pub state: RecordingStatus,
    pub samples: Option<usize>,
    pub transcription_status: TranscriptionStatus,
}

#[tauri::command]
fn get_status(
    audio_manager: State<AudioManagerState>,
    transcription_manager: State<TranscriptionManagerState>,
) -> Result<StatusResponse, String> {
    let manager_guard = audio_manager.lock().map_err(|e| format!("Lock error: {}", e))?;
    let transcription_guard = transcription_manager.lock().map_err(|e| format!("Lock error: {}", e))?;
    
    if let Some(manager) = manager_guard.as_ref() {
        let (status, samples) = manager.get_status_info();
        let transcription_status = if let Some(transcription_mgr) = transcription_guard.as_ref() {
            transcription_mgr.get_status().unwrap_or(TranscriptionStatus::Idle)
        } else {
            TranscriptionStatus::Idle
        };
        
        Ok(StatusResponse {
            state: status,
            samples,
            transcription_status,
        })
    } else {
        Err("Audio manager not initialized".to_string())
    }
}

#[tauri::command]
fn start_audio_stream(audio_manager: State<AudioManagerState>) -> Result<String, String> {
    let manager_guard = audio_manager.lock().map_err(|e| format!("Lock error: {}", e))?;
    
    if let Some(manager) = manager_guard.as_ref() {
        // Check current status first
        let (current_status, _) = manager.get_status_info();
        match current_status {
            RecordingStatus::Recording => {
                return Err("Recording is already in progress".to_string());
            }
            RecordingStatus::Transcribing => {
                return Err("Cannot start recording while transcribing".to_string());
            }
            RecordingStatus::Idle => {
                // Proceed with starting
            }
        }
        
        match manager.start() {
            Ok(_) => Ok("Audio recording started successfully".to_string()),
            Err(e) => Err(format!("Failed to start recording: {}", e)),
        }
    } else {
        Err("Audio manager not initialized".to_string())
    }
}

#[tauri::command]
fn stop_audio_stream(
    audio_manager: State<AudioManagerState>,
    transcription_manager: State<TranscriptionManagerState>,
) -> Result<String, String> {
    use std::fs::File;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    use crate::encode::resample_and_encode_wav;

    let manager_guard = audio_manager.lock().map_err(|e| format!("Lock error: {}", e))?;

    if let Some(manager) = manager_guard.as_ref() {
        // Check current status first
        let (current_status, _) = manager.get_status_info();
        match current_status {
            RecordingStatus::Idle => {
                return Err("No recording in progress".to_string());
            }
            RecordingStatus::Transcribing => {
                return Err("Cannot stop recording while transcribing".to_string());
            }
            RecordingStatus::Recording => {
                // Proceed with stopping
            }
        }

        match manager.stop() {
            Ok(recording_data) => {
                // Set status to transcribing (placeholder for actual transcription)
                if let Err(e) = manager.set_transcribing() {
                    eprintln!("Warning: Failed to set transcribing status: {}", e);
                }

                // Calculate RMS for reporting
                let rms = calculate_rms(&recording_data.samples);

                // Encode to WAV using encode.rs
                let wav_bytes = match resample_and_encode_wav(recording_data) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        // Reset to idle on error
                        let _ = manager.set_idle();
                        return Err(format!("Failed to encode WAV: {}", e));
                    }
                };

                // Start transcription with the WAV data
                let transcription_guard = transcription_manager.lock().map_err(|e| format!("Lock error: {}", e))?;
                if let Some(transcription_mgr) = transcription_guard.as_ref() {
                    // Cancel any existing transcription and start a new one
                    transcription_mgr.cancel_transcription();
                    if let Err(e) = transcription_mgr.start_transcription(wav_bytes.clone()) {
                        eprintln!("Failed to start transcription: {}", e);
                    }
                } else {
                    eprintln!("Transcription manager not initialized");
                }

                // Reset audio manager to idle after a short delay
                let manager_clone = Arc::clone(&audio_manager);
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(500));
                    if let Ok(guard) = manager_clone.lock() {
                        if let Some(manager) = guard.as_ref() {
                            let _ = manager.set_idle();
                        }
                    }
                });

                Ok(format!(
                    "Recording stopped. Starting transcription... RMS: {:.6}",
                    rms
                ))
            },
            Err(e) => Err(format!("Failed to stop recording: {}", e)),
        }
    } else {
        Err("Audio manager not initialized".to_string())
    }
}

#[tauri::command]
fn get_transcription_status(transcription_manager: State<TranscriptionManagerState>) -> Result<TranscriptionStatus, String> {
    let transcription_guard = transcription_manager.lock().map_err(|e| format!("Lock error: {}", e))?;
    
    if let Some(transcription_mgr) = transcription_guard.as_ref() {
        transcription_mgr.get_status()
    } else {
        Err("Transcription manager not initialized".to_string())
    }
}

#[tauri::command]
fn cancel_transcription(transcription_manager: State<TranscriptionManagerState>) -> Result<String, String> {
    let transcription_guard = transcription_manager.lock().map_err(|e| format!("Lock error: {}", e))?;
    
    if let Some(transcription_mgr) = transcription_guard.as_ref() {
        transcription_mgr.cancel_transcription();
        Ok("Transcription cancelled".to_string())
    } else {
        Err("Transcription manager not initialized".to_string())
    }
}

fn calculate_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    
    let sum_of_squares: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_of_squares / samples.len() as f32).sqrt()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let audio_manager: AudioManagerState = Arc::new(Mutex::new(None));
    let transcription_manager: TranscriptionManagerState = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(audio_manager.clone())
        .manage(transcription_manager.clone())
        .invoke_handler(tauri::generate_handler![
            get_status,
            start_audio_stream,
            stop_audio_stream,
            get_transcription_status,
            cancel_transcription
        ])
        .setup(move |_app| {
            // Initialize the audio manager
            let mut manager_guard = audio_manager.lock().unwrap();
            let manager = AudioManager::new();
            
            // Initialize the transcription manager
            let mut transcription_guard = transcription_manager.lock().unwrap();
            let transcription_mgr = TranscriptionManager::new();
            
            *manager_guard = Some(manager);
            *transcription_guard = Some(transcription_mgr);
            println!("Audio manager and transcription manager initialized");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
