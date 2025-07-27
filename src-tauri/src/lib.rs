// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

mod audio;
mod encode;

use audio::{AudioManager, RecordingStatus};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::State;

// Global audio manager state
type AudioManagerState = Arc<Mutex<Option<AudioManager>>>;

#[derive(Serialize, Deserialize)]
pub struct StatusResponse {
    pub state: RecordingStatus,
    pub samples: Option<usize>,
}

#[tauri::command]
fn get_status(audio_manager: State<AudioManagerState>) -> Result<StatusResponse, String> {
    let manager_guard = audio_manager.lock().map_err(|e| format!("Lock error: {}", e))?;
    
    if let Some(manager) = manager_guard.as_ref() {
        let (status, samples) = manager.get_status_info();
        Ok(StatusResponse {
            state: status,
            samples,
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
fn stop_audio_stream(audio_manager: State<AudioManagerState>) -> Result<String, String> {
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

                // Get current unix time for filename
                let now = SystemTime::now().duration_since(UNIX_EPOCH)
                    .map_err(|e| format!("Time error: {}", e))?
                    .as_secs();
                let filename = format!("test-{}.wav", now);

                // Write to file
                match File::create(&filename) {
                    Ok(mut file) => {
                        if let Err(e) = file.write_all(&wav_bytes) {
                            // Reset to idle on error
                            let _ = manager.set_idle();
                            return Err(format!("Failed to write WAV file: {}", e));
                        }
                    }
                    Err(e) => {
                        // Reset to idle on error
                        let _ = manager.set_idle();
                        return Err(format!("Failed to create WAV file: {}", e));
                    }
                }

                // Simulate transcription processing time, then set back to idle
                let manager_clone = Arc::clone(&audio_manager);
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(2)); // Simulate transcription time
                    if let Ok(guard) = manager_clone.lock() {
                        if let Some(manager) = guard.as_ref() {
                            let _ = manager.set_idle();
                        }
                    }
                });

                Ok(format!(
                    "Recording stopped. Captured WAV to '{}'. RMS: {:.6}",
                    filename,
                    rms
                ))
            },
            Err(e) => Err(format!("Failed to stop recording: {}", e)),
        }
    } else {
        Err("Audio manager not initialized".to_string())
    }
}

fn calculate_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    
    let sum_of_squares: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_of_squares / samples.len() as f32).sqrt()
}

fn run_audio_test(manager: &AudioManager) {
    println!("\n=== Audio Test Starting ===");
    
    for cycle in 1..=3 {
        println!("\n--- Recording Cycle {} ---", cycle);
        
        // Start recording
        match manager.start() {
            Ok(_) => println!("✓ Recording started"),
            Err(e) => {
                println!("✗ Failed to start recording: {}", e);
                continue;
            }
        }
        
        // Check status periodically for 3 seconds
        for second in 1..=3 {
            thread::sleep(Duration::from_secs(1));
            match manager.status() {
                Ok(sample_count) => println!("  Status after {}s: {} samples", second, sample_count),
                Err(e) => println!("  Status check error: {}", e),
            }
        }
        
        // Stop recording and analyze results
        match manager.stop() {
            Ok(recording_data) => {
                let rms = calculate_rms(&recording_data.samples);
                let duration_seconds = recording_data.samples.len() as f32 / recording_data.sample_rate as f32;
                
                println!("✓ Recording stopped");
                println!("  Total samples: {}", recording_data.samples.len());
                println!("  Sample rate: {} Hz", recording_data.sample_rate);
                println!("  Duration: {:.2} seconds", duration_seconds);
                println!("  RMS amplitude: {:.6}", rms);

                let encoded_wav = encode::resample_and_encode_wav(recording_data).unwrap();
                println!("  WAV file size: {} bytes", encoded_wav.len());
                
                // Write the encoded WAV to disk as test-N.wav
                let filename = format!("test-{}.wav", cycle);
                match std::fs::write(&filename, &encoded_wav) {
                    Ok(_) => println!("  Saved WAV to '{}'", filename),
                    Err(e) => println!("  Failed to save WAV file: {}", e),
                }
            },
            Err(e) => println!("✗ Failed to stop recording: {}", e),
        }
        
        // Wait between cycles
        if cycle < 3 {
            println!("  Waiting 2 seconds before next cycle...");
            thread::sleep(Duration::from_secs(2));
        }
    }
    
    println!("\n=== Audio Test Complete ===\n");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let audio_manager: AudioManagerState = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(audio_manager.clone())
        .invoke_handler(tauri::generate_handler![
            get_status,
            start_audio_stream,
            stop_audio_stream
        ])
        .setup(move |_app| {
            // Initialize the audio manager
            let mut manager_guard = audio_manager.lock().unwrap();
            let manager = AudioManager::new();
            
            // Run the test in a separate thread
            let manager_for_test = AudioManager::new();
            thread::spawn(move || {
                // Give the app a moment to start up
                thread::sleep(Duration::from_millis(500));
                run_audio_test(&manager_for_test);
            });
            
            *manager_guard = Some(manager);
            println!("Audio manager initialized");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
