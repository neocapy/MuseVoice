use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Sample, SampleFormat, SampleRate, StreamConfig};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub enum AudioError {
    NoDeviceFound,
    StreamCreationFailed(cpal::BuildStreamError),
    DeviceEnumerationFailed(cpal::DevicesError),
    UnsupportedFormat,
    StreamPlayFailed(cpal::PlayStreamError),
    AlreadyRecording,
    NotRecording,
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::NoDeviceFound => write!(f, "No audio input device found"),
            AudioError::StreamCreationFailed(e) => write!(f, "Failed to create audio stream: {}", e),
            AudioError::DeviceEnumerationFailed(e) => write!(f, "Failed to enumerate devices: {}", e),
            AudioError::UnsupportedFormat => write!(f, "Unsupported audio format"),
            AudioError::StreamPlayFailed(e) => write!(f, "Failed to play stream: {}", e),
            AudioError::AlreadyRecording => write!(f, "Recording is already in progress"),
            AudioError::NotRecording => write!(f, "No recording in progress"),
        }
    }
}

impl std::error::Error for AudioError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingStatus {
    Idle,
    Recording,
    Transcribing,
}

#[derive(Debug)]
struct AudioState {
    status: RecordingStatus,
    samples: Vec<f32>,
    sample_rate: u32,
}

impl AudioState {
    fn new() -> Self {
        Self {
            status: RecordingStatus::Idle,
            samples: Vec::new(),
            sample_rate: 0,
        }
    }
}

pub struct RecordingData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub struct AudioManager {
    state: Arc<Mutex<AudioState>>,
    should_stop: Arc<AtomicBool>,
    recording_thread: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl AudioManager {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(AudioState::new())),
            should_stop: Arc::new(AtomicBool::new(false)),
            recording_thread: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start(&self) -> Result<(), AudioError> {
        let mut state = self.state.lock().unwrap();
        
        if state.status == RecordingStatus::Recording {
            return Err(AudioError::AlreadyRecording);
        }

        // Reset state for new recording
        state.samples.clear();
        state.status = RecordingStatus::Recording;
        self.should_stop.store(false, Ordering::Relaxed);

        // Get device config before starting thread
        let device = Self::find_input_device()?;
        let config = Self::get_best_config(&device)?;
        state.sample_rate = config.sample_rate.0;
        
        println!("Starting recording: {} channels, {} Hz", 
                 config.channels, config.sample_rate.0);

        // Start recording in a separate thread
        let state_clone = Arc::clone(&self.state);
        let should_stop_clone = Arc::clone(&self.should_stop);
        let recording_thread_handle = Arc::clone(&self.recording_thread);
        
        let handle = thread::spawn(move || {
            if let Err(e) = Self::run_recording_thread(state_clone, should_stop_clone) {
                eprintln!("Recording thread error: {}", e);
            }
        });
        
        *recording_thread_handle.lock().unwrap() = Some(handle);
        
        Ok(())
    }

    pub fn stop(&self) -> Result<RecordingData, AudioError> {
        let mut state = self.state.lock().unwrap();
        
        if state.status != RecordingStatus::Recording {
            return Err(AudioError::NotRecording);
        }

        // Signal the recording thread to stop
        self.should_stop.store(true, Ordering::Relaxed);
        
        // Wait for the recording thread to finish
        if let Some(handle) = self.recording_thread.lock().unwrap().take() {
            drop(state); // Release the lock before joining
            let _ = handle.join();
            state = self.state.lock().unwrap();
        }
        
        // Extract the recorded data
        let samples = std::mem::take(&mut state.samples);
        let sample_rate = state.sample_rate;
        state.status = RecordingStatus::Idle;
        
        println!("Recording stopped. Captured {} samples at {} Hz", samples.len(), sample_rate);
        
        Ok(RecordingData { samples, sample_rate })
    }

    pub fn status(&self) -> Result<usize, AudioError> {
        let state = self.state.lock().unwrap();
        
        if state.status != RecordingStatus::Recording {
            return Err(AudioError::NotRecording);
        }
        
        Ok(state.samples.len())
    }

    pub fn get_status_info(&self) -> (RecordingStatus, Option<usize>) {
        let state = self.state.lock().unwrap();
        
        match state.status {
            RecordingStatus::Recording => (state.status.clone(), Some(state.samples.len())),
            _ => (state.status.clone(), None),
        }
    }

    pub fn set_transcribing(&self) -> Result<(), AudioError> {
        let mut state = self.state.lock().unwrap();
        
        if state.status != RecordingStatus::Idle {
            return Err(AudioError::AlreadyRecording);
        }
        
        state.status = RecordingStatus::Transcribing;
        Ok(())
    }

    pub fn set_idle(&self) -> Result<(), AudioError> {
        let mut state = self.state.lock().unwrap();
        state.status = RecordingStatus::Idle;
        Ok(())
    }

    fn run_recording_thread(
        state: Arc<Mutex<AudioState>>,
        should_stop: Arc<AtomicBool>,
    ) -> Result<(), AudioError> {
        let device = Self::find_input_device()?;
        let config = Self::get_best_config(&device)?;
        let channels = config.channels;

        // Determine sample format
        let supported_configs = device.supported_input_configs()
            .map_err(|_| AudioError::UnsupportedFormat)?;
        
        let mut sample_format = SampleFormat::F32;
        for supported_config in supported_configs {
            sample_format = supported_config.sample_format();
            break;
        }

        let stream = match sample_format {
            SampleFormat::I8 => {
                device.build_input_stream(
                    &config,
                    move |data: &[i8], _: &cpal::InputCallbackInfo| {
                        let float_data: Vec<f32> = data.iter()
                            .map(|&s| s.to_sample::<f32>())
                            .collect();
                        Self::accumulate_audio_data(&float_data, channels, &state);
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
            }
            SampleFormat::I16 => {
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let float_data: Vec<f32> = data.iter()
                            .map(|&s| s.to_sample::<f32>())
                            .collect();
                        Self::accumulate_audio_data(&float_data, channels, &state);
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
            }
            SampleFormat::I32 => {
                device.build_input_stream(
                    &config,
                    move |data: &[i32], _: &cpal::InputCallbackInfo| {
                        let float_data: Vec<f32> = data.iter()
                            .map(|&s| s.to_sample::<f32>())
                            .collect();
                        Self::accumulate_audio_data(&float_data, channels, &state);
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
            }
            SampleFormat::F32 => {
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        Self::accumulate_audio_data(data, channels, &state);
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
            }
            _ => return Err(AudioError::UnsupportedFormat),
        }.map_err(AudioError::StreamCreationFailed)?;

        stream.play().map_err(AudioError::StreamPlayFailed)?;
        
        // Keep the stream alive until stop signal is received
        while !should_stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
        }
        
        drop(stream);
        println!("Recording thread finished");
        
        Ok(())
    }

    fn find_input_device() -> Result<Device, AudioError> {
        let host = cpal::default_host();
        
        // Check for custom device from environment variable
        if let Ok(device_name) = env::var("MUSE_INPUT_DEVICE") {
            println!("Looking for custom input device: {}", device_name);
            
            let devices = host.input_devices()
                .map_err(AudioError::DeviceEnumerationFailed)?;
            
            for device in devices {
                if let Ok(name) = device.name() {
                    if name.to_lowercase() == device_name.to_lowercase() {
                        println!("Found custom input device: {}", name);
                        return Ok(device);
                    }
                }
            }
            
            println!("Custom device '{}' not found, falling back to default", device_name);
        }
        
        // Fall back to default input device
        let device = host.default_input_device()
            .ok_or(AudioError::NoDeviceFound)?;
        
        if let Ok(name) = device.name() {
            println!("Using default input device: {}", name);
        }
        
        Ok(device)
    }

    fn get_best_config(device: &Device) -> Result<StreamConfig, AudioError> {
        let supported_configs = device.supported_input_configs()
            .map_err(|_| AudioError::UnsupportedFormat)?;

        // Try to find a configuration that supports 16kHz mono
        let mut best_config = None;
        let mut fallback_config = None;

        for config in supported_configs {
            // Check if 16kHz is supported
            if config.min_sample_rate() <= SampleRate(16000) && 
               config.max_sample_rate() >= SampleRate(16000) {
                // Prefer mono, but accept any channel count
                if config.channels() == 1 {
                    best_config = Some(config.with_sample_rate(SampleRate(16000)));
                    break;
                } else if best_config.is_none() {
                    best_config = Some(config.with_sample_rate(SampleRate(16000)));
                }
            }
            
            // Keep a fallback config
            if fallback_config.is_none() {
                fallback_config = Some(config.with_max_sample_rate());
            }
        }

        let config = best_config.or(fallback_config)
            .ok_or(AudioError::UnsupportedFormat)?
            .into();

        Ok(config)
    }

    fn mix_to_mono(data: &[f32], channels: u16) -> Vec<f32> {
        if channels == 1 {
            return data.to_vec();
        }

        let samples_per_channel = data.len() / channels as usize;
        let mut mono_data = Vec::with_capacity(samples_per_channel);

        for i in 0..samples_per_channel {
            let mut sum = 0.0f32;
            for ch in 0..channels {
                sum += data[i * channels as usize + ch as usize];
            }
            mono_data.push(sum / channels as f32);
        }

        mono_data
    }

    fn accumulate_audio_data(data: &[f32], channels: u16, state: &Arc<Mutex<AudioState>>) {
        // Mix to mono if necessary
        let mono_data = Self::mix_to_mono(data, channels);
        
        // Accumulate samples
        if let Ok(mut state) = state.lock() {
            if state.status == RecordingStatus::Recording {
                state.samples.extend_from_slice(&mono_data);
            }
        }
    }
} 