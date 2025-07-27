use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Sample, SampleFormat, SampleRate, StreamConfig};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowState {
    Idle,
    Recording,
    Processing,
    Completed,
    Error,
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum FlowEvent {
    StateChanged(FlowState),
    SampleCount(usize),
    TranscriptionResult(String),
    WavFileSaved(String), // Path to the saved WAV file
    Error(String),
}

pub type FlowCallback = Arc<dyn Fn(FlowEvent) + Send + Sync>;

#[derive(Debug)]
pub struct AudioError {
    pub message: String,
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AudioError {}

impl From<cpal::BuildStreamError> for AudioError {
    fn from(e: cpal::BuildStreamError) -> Self {
        AudioError {
            message: format!("Stream build error: {}", e),
        }
    }
}

impl From<cpal::PlayStreamError> for AudioError {
    fn from(e: cpal::PlayStreamError) -> Self {
        AudioError {
            message: format!("Stream play error: {}", e),
        }
    }
}

impl From<cpal::DevicesError> for AudioError {
    fn from(e: cpal::DevicesError) -> Self {
        AudioError {
            message: format!("Device enumeration error: {}", e),
        }
    }
}

impl From<cpal::SupportedStreamConfigsError> for AudioError {
    fn from(e: cpal::SupportedStreamConfigsError) -> Self {
        AudioError {
            message: format!("Stream config error: {}", e),
        }
    }
}

#[derive(Debug)]
pub struct RecordingData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub struct Flow {
    state: Arc<RwLock<FlowState>>,
    callback: FlowCallback,
    cancellation_token: CancellationToken,
}

impl Flow {
    pub fn new(callback: FlowCallback) -> Self {
        Self {
            state: Arc::new(RwLock::new(FlowState::Idle)),
            callback,
            cancellation_token: CancellationToken::new(),
        }
    }

    pub async fn get_state(&self) -> FlowState {
        self.state.read().await.clone()
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    /// Saves WAV data to $HOME/.musevoice/recording-${unixtime}.wav
    /// Returns the full path if successful, or None if it fails gracefully
    fn save_wav_file(&self, wav_data: &[u8]) -> Option<String> {
        // Get home directory
        let home_dir = match env::var("HOME") {
            Ok(path) => PathBuf::from(path),
            Err(_) => {
                eprintln!("Warning: Could not determine home directory, skipping WAV file save");
                return None;
            }
        };

        // Create .musevoice directory
        let musevoice_dir = home_dir.join(".musevoice");
        if let Err(e) = fs::create_dir_all(&musevoice_dir) {
            eprintln!("Warning: Could not create directory {:?}: {}, skipping WAV file save", musevoice_dir, e);
            return None;
        }

        // Generate filename with Unix timestamp
        let unix_time = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs(),
            Err(_) => {
                eprintln!("Warning: Could not get current time, skipping WAV file save");
                return None;
            }
        };

        let filename = format!("recording-{}.wav", unix_time);
        let file_path = musevoice_dir.join(&filename);

        // Save the WAV data
        match fs::write(&file_path, wav_data) {
            Ok(_) => {
                println!("Saved WAV file: {:?}", file_path);
                file_path.to_string_lossy().to_string().into()
            }
            Err(e) => {
                eprintln!("Warning: Could not write WAV file {:?}: {}, skipping WAV file save", file_path, e);
                None
            }
        }
    }

    /// Main flow method: starts recording, waits for stop signal, then transcribes
    pub async fn run(&self, stop_signal: oneshot::Receiver<()>) -> Result<(), AudioError> {
        // Set initial state
        self.set_state(FlowState::Recording).await;

        // Start audio recording
        let recording_data = match self.record_audio(stop_signal).await {
            Ok(data) => data,
            Err(e) => {
                self.set_state(FlowState::Error).await;
                self.emit_event(FlowEvent::Error(e.message.clone()));
                return Err(e);
            }
        };

        // Check if cancelled during recording
        if self.cancellation_token.is_cancelled() {
            self.set_state(FlowState::Cancelled).await;
            return Ok(());
        }

        // Move to processing state
        self.set_state(FlowState::Processing).await;

        // Encode audio (sync operation, but fast)
        let wav_data = match tokio::task::spawn_blocking(move || {
            crate::encode::resample_and_encode_wav(recording_data.into()).map_err(|e| e.to_string())
        })
        .await
        {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => {
                let error = AudioError {
                    message: format!("Encoding error: {}", e),
                };
                self.set_state(FlowState::Error).await;
                self.emit_event(FlowEvent::Error(error.message.clone()));
                return Err(error);
            }
            Err(e) => {
                let error = AudioError {
                    message: format!("Task join error: {}", e),
                };
                self.set_state(FlowState::Error).await;
                self.emit_event(FlowEvent::Error(error.message.clone()));
                return Err(error);
            }
        };

        // Check if cancelled during encoding
        if self.cancellation_token.is_cancelled() {
            self.set_state(FlowState::Cancelled).await;
            return Ok(());
        }

        // Save WAV file to disk (fails gracefully if not possible)
        if let Some(saved_path) = self.save_wav_file(&wav_data) {
            self.emit_event(FlowEvent::WavFileSaved(saved_path));
        }

        // Transcribe with OpenAI
        match self.transcribe_audio(wav_data).await {
            Ok(text) => {
                self.set_state(FlowState::Completed).await;
                self.emit_event(FlowEvent::TranscriptionResult(text));
                Ok(())
            }
            Err(e) => {
                self.set_state(FlowState::Error).await;
                self.emit_event(FlowEvent::Error(e.message.clone()));
                Err(e)
            }
        }
    }

    async fn record_audio(
        &self,
        stop_signal: oneshot::Receiver<()>,
    ) -> Result<RecordingData, AudioError> {
        let device = Self::find_input_device()?;
        let config = Self::get_best_config(&device)?;
        let sample_rate = config.sample_rate.0;

        println!(
            "Starting recording: {} channels, {} Hz",
            config.channels, sample_rate
        );

        // Create channels for communication with the blocking audio thread
        let (sample_sender, mut sample_receiver) = mpsc::unbounded_channel::<Vec<f32>>();
        let (stop_sender, stop_receiver) = oneshot::channel();
        let (result_sender, result_receiver) = oneshot::channel();

        let callback = Arc::clone(&self.callback);
        let cancellation_token = self.cancellation_token.clone();

        // Spawn the audio recording in a blocking task
        let _handle = tokio::task::spawn_blocking(move || {
            Self::run_audio_recording_thread(
                device,
                config,
                sample_sender,
                stop_receiver,
                result_sender,
            )
        });

        // Accumulate samples from the audio thread
        let mut all_samples = Vec::new();
        let mut sample_count = 0;
        let mut stop_signal = Some(stop_signal);
        let mut result_receiver = Some(result_receiver);

        loop {
            tokio::select! {
                // Receive samples from audio thread
                Some(samples) = sample_receiver.recv() => {
                    all_samples.extend_from_slice(&samples);
                    sample_count += samples.len();
                    callback(FlowEvent::SampleCount(sample_count));
                }

                // Stop signal received
                _ = stop_signal.as_mut().unwrap() => {
                    println!("Stop signal received");
                    let _ = stop_sender.send(());
                    break;
                }

                // Cancellation requested
                _ = cancellation_token.cancelled() => {
                    println!("Recording cancelled");
                    return Err(AudioError { message: "Recording cancelled".to_string() });
                }

                // Audio thread finished
                result = result_receiver.as_mut().unwrap() => {
                    match result {
                        Ok(Ok(())) => {
                            println!("Audio thread finished successfully");
                            break;
                        }
                        Ok(Err(e)) => {
                            return Err(AudioError { message: format!("Audio thread error: {}", e) });
                        }
                        Err(_) => {
                            return Err(AudioError { message: "Audio thread communication error".to_string() });
                        }
                    }
                }
            }
        }

        println!(
            "Recording stopped. Captured {} samples at {} Hz",
            all_samples.len(),
            sample_rate
        );

        Ok(RecordingData {
            samples: all_samples,
            sample_rate,
        })
    }

    fn run_audio_recording_thread(
        device: Device,
        config: StreamConfig,
        sample_sender: mpsc::UnboundedSender<Vec<f32>>,
        stop_receiver: oneshot::Receiver<()>,
        result_sender: oneshot::Sender<Result<(), String>>,
    ) {
        let result = (|| -> Result<(), String> {
            let channels = config.channels;

            // Find a supported config that matches our preferred sample formats: f32 > i16 > i32
            let mut selected_config = None;
            let mut selected_format = None;
            for fmt in [SampleFormat::F32, SampleFormat::I16, SampleFormat::I32] {
                for supported_config in device.supported_input_configs().map_err(|e| format!("Failed to get supported configs: {}", e))? {
                    if supported_config.sample_format() == fmt {
                        selected_config = Some(supported_config);
                        selected_format = Some(fmt);
                        break;
                    }
                }
                if selected_config.is_some() {
                    break;
                }
            }
            let sample_format = match selected_format {
                Some(fmt) => fmt,
                None => return Err("No supported sample format (f32, i16, i32) found".to_string()),
            };

            let recording_active = Arc::new(AtomicBool::new(true));
            let recording_active_clone = Arc::clone(&recording_active);

            let stream = match sample_format {
                SampleFormat::F32 => device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if recording_active_clone.load(Ordering::Relaxed) {
                            let mono_data = Self::mix_to_mono(data, channels);
                            let _ = sample_sender.send(mono_data);
                        }
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                ),
                SampleFormat::I16 => device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if recording_active_clone.load(Ordering::Relaxed) {
                            let float_data: Vec<f32> =
                                data.iter().map(|&s| s.to_sample::<f32>()).collect();
                            let mono_data = Self::mix_to_mono(&float_data, channels);
                            let _ = sample_sender.send(mono_data);
                        }
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                ),
                SampleFormat::I32 => device.build_input_stream(
                    &config,
                    move |data: &[i32], _: &cpal::InputCallbackInfo| {
                        if recording_active_clone.load(Ordering::Relaxed) {
                            let float_data: Vec<f32> =
                                data.iter().map(|&s| s.to_sample::<f32>()).collect();
                            let mono_data = Self::mix_to_mono(&float_data, channels);
                            let _ = sample_sender.send(mono_data);
                        }
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                ),
                _ => {
                    return Err("Unsupported sample format".to_string());
                }
            }
            .map_err(|e| format!("Failed to build stream: {}", e))?;

            // Start the stream
            stream
                .play()
                .map_err(|e| format!("Failed to play stream: {}", e))?;

            // Wait for stop signal
            let _ = stop_receiver.blocking_recv();
            println!("Audio thread: Stop signal received");

            // Stop recording
            recording_active.store(false, Ordering::Relaxed);
            drop(stream);

            Ok(())
        })();

        let _ = result_sender.send(result);
    }

    async fn transcribe_audio(&self, wav_data: Vec<u8>) -> Result<String, AudioError> {
        let api_key = env::var("OPENAI_API_KEY").map_err(|_| AudioError {
            message: "OPENAI_API_KEY environment variable not set".to_string(),
        })?;

        if api_key.trim().is_empty() {
            return Err(AudioError {
                message: "OPENAI_API_KEY is empty".to_string(),
            });
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AudioError {
                message: format!("Failed to create HTTP client: {}", e),
            })?;

        let form = reqwest::multipart::Form::new()
            .part(
                "file",
                reqwest::multipart::Part::bytes(wav_data)
                    .file_name("audio.wav")
                    .mime_str("audio/wav")
                    .map_err(|e| AudioError {
                        message: format!("Failed to create file part: {}", e),
                    })?,
            )
            .text("model", "gpt-4o-transcribe");

        println!("Sending transcription request to OpenAI...");

        let request_future = client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", api_key))
            .multipart(form)
            .send();

        // Wait for either response or cancellation
        let response = tokio::select! {
            result = request_future => {
                result.map_err(|e| AudioError { message: format!("Failed to send request: {}", e) })?
            }
            _ = self.cancellation_token.cancelled() => {
                return Err(AudioError { message: "Transcription cancelled".to_string() });
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AudioError {
                message: format!("OpenAI API error {}: {}", status, error_text),
            });
        }

        #[derive(Deserialize)]
        struct OpenAIResponse {
            text: String,
        }

        let openai_response: OpenAIResponse = response.json().await.map_err(|e| AudioError {
            message: format!("Failed to parse response: {}", e),
        })?;

        Ok(openai_response.text)
    }

    async fn set_state(&self, new_state: FlowState) {
        {
            let mut state = self.state.write().await;
            *state = new_state.clone();
        }
        self.emit_event(FlowEvent::StateChanged(new_state));
    }

    fn emit_event(&self, event: FlowEvent) {
        (self.callback)(event);
    }

    fn find_input_device() -> Result<Device, AudioError> {
        let host = cpal::default_host();

        // Check for custom device from environment variable
        if let Ok(device_name) = env::var("MUSE_INPUT_DEVICE") {
            println!("Looking for custom input device: {}", device_name);

            let devices = host.input_devices().map_err(AudioError::from)?;

            for device in devices {
                if let Ok(name) = device.name() {
                    if name.to_lowercase() == device_name.to_lowercase() {
                        println!("Found custom input device: {}", name);
                        return Ok(device);
                    }
                }
            }

            println!(
                "Custom device '{}' not found, falling back to default",
                device_name
            );
        }

        // Fall back to default input device
        let device = host.default_input_device().ok_or_else(|| AudioError {
            message: "No input device found".to_string(),
        })?;

        if let Ok(name) = device.name() {
            println!("Using default input device: {}", name);
        }

        Ok(device)
    }

    fn get_best_config(device: &Device) -> Result<StreamConfig, AudioError> {
        let supported_configs = device.supported_input_configs().map_err(|_| AudioError {
            message: "Unsupported format".to_string(),
        })?;

        // Try to find a configuration that supports 16kHz mono
        let mut best_config = None;
        let mut fallback_config = None;

        for config in supported_configs {
            // Check if 16kHz is supported
            if config.min_sample_rate() <= SampleRate(16000)
                && config.max_sample_rate() >= SampleRate(16000)
            {
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

        let config = best_config
            .or(fallback_config)
            .ok_or_else(|| AudioError {
                message: "Unsupported format".to_string(),
            })?
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
}
