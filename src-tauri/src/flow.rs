use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Sample, SampleFormat, SampleRate, StreamConfig};
use crossbeam_channel::RecvTimeoutError;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{oneshot, RwLock};
use tokio_util::sync::CancellationToken;
use crate::stream_processor::AudioStreamProcessor;
use crate::audio_output::AudioOutputManager;

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



/// Controls whether the flow should record audio or use existing audio data
#[derive(Debug)]
pub enum FlowMode {
    /// Record audio, then encode and transcribe it
    RecordAndTranscribe {
        stop_signal: oneshot::Receiver<()>,
    },
    /// Skip recording and encoding, transcribe existing audio data (WebM format)
    TranscribeOnly {
        audio_data: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub enum FlowEvent {
    StateChanged(FlowState),
    SampleCount(usize),
    TranscriptionResult(String),
    AudioFileSaved(String), // Path to the saved audio file (WebM format)
    AudioDataReady(Vec<u8>), // Audio buffer ready for transcription (WebM format, for retry functionality)
    WaveformChunk { bins: Vec<f32>, avg_rms: f32 },
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

pub struct Flow {
    state: Arc<RwLock<FlowState>>,
    callback: FlowCallback,
    cancellation_token: CancellationToken,
    model: String,
    rewrite_enabled: bool,
    omit_final_punctuation: bool,
    audio_manager: Arc<Mutex<AudioOutputManager>>,
    rewrite_prompt: String,
}

impl Flow {
    pub fn new(callback: FlowCallback, model: String, rewrite_enabled: bool, omit_final_punctuation: bool, audio_manager: Arc<Mutex<AudioOutputManager>>, rewrite_prompt: String) -> Self {
        Self {
            state: Arc::new(RwLock::new(FlowState::Idle)),
            callback,
            cancellation_token: CancellationToken::new(),
            model,
            rewrite_enabled,
            omit_final_punctuation,
            audio_manager,
            rewrite_prompt,
        }
    }

    pub async fn get_state(&self) -> FlowState {
        self.state.read().await.clone()
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    /// Saves audio data to $HOME/.musevoice/recording-${unixtime}.webm
    /// Returns the full path if successful, or None if it fails gracefully
    fn save_audio_file(&self, audio_data: &[u8]) -> Option<String> {
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

        let filename = format!("recording-{}.webm", unix_time);
        let file_path = musevoice_dir.join(&filename);

        // Save the WAV data
        match fs::write(&file_path, audio_data) {
            Ok(_) => {
                println!("Saved audio file: {:?}", file_path);
                file_path.to_string_lossy().to_string().into()
            }
            Err(e) => {
                eprintln!("Warning: Could not write audio file {:?}: {}, skipping audio file save", file_path, e);
                None
            }
        }
    }

    /// Main flow method: either records and transcribes, or transcribes existing audio data
    pub async fn run(&self, mode: FlowMode) -> Result<(), AudioError> {
        let audio_data = match mode {
            FlowMode::RecordAndTranscribe { stop_signal } => {
                // Set initial state and emit audio feedback for starting recording
                self.play_sound("boowomp.mp3");
                self.set_state(FlowState::Recording).await;

                // Start streaming audio recording (now includes encoding)
                let audio_data = match self.record_audio(stop_signal).await {
                    Ok(data) => data,
                    Err(e) => {
                        self.play_sound("pipe.mp3");
                        self.set_state(FlowState::Error).await;
                        self.emit_event(FlowEvent::Error(e.message.clone()));
                        return Err(e);
                    }
                };

                // Check if cancelled
                if self.cancellation_token.is_cancelled() {
                    self.play_sound("pipe.mp3");
                    self.set_state(FlowState::Cancelled).await;
                    return Ok(());
                }

                // Recording and encoding complete - emit bamboo hit sound
                self.play_sound("bamboo_hit.mp3");
                self.set_state(FlowState::Processing).await;

                // Emit audio data for potential retry functionality
                self.emit_event(FlowEvent::AudioDataReady(audio_data.clone()));

                // Save audio file to disk (fails gracefully if not possible)
                if let Some(saved_path) = self.save_audio_file(&audio_data) {
                    self.emit_event(FlowEvent::AudioFileSaved(saved_path));
                }

                audio_data
            }
            FlowMode::TranscribeOnly { audio_data } => {
                // Set to processing state
                self.set_state(FlowState::Processing).await;

                // Check if cancelled
                if self.cancellation_token.is_cancelled() {
                    self.play_sound("pipe.mp3");
                    self.set_state(FlowState::Cancelled).await;
                    return Ok(());
                }

                audio_data
            }
        };

        // Transcribe with OpenAI
        let mut transcribed_text = match self.transcribe_audio(audio_data).await {
            Ok(text) => text,
            Err(e) => {
                self.play_sound("pipe.mp3");
                self.set_state(FlowState::Error).await;
                self.emit_event(FlowEvent::Error(e.message.clone()));
                return Err(e);
            }
        };

        // Apply rewriting if enabled
        if self.rewrite_enabled {
            println!("Rewrite enabled, attempting to rewrite transcribed text...");
            match self.rewrite_transcribed_text(&transcribed_text).await {
                Ok(rewritten_text) => {
                    println!("Rewrite successful");
                    transcribed_text = rewritten_text;
                }
                Err(e) => {
                    eprintln!("Rewrite failed, using original transcription: {}", e.message);
                }
            }
        }

        if self.omit_final_punctuation {
            transcribed_text = transcribed_text
                .trim_end_matches(&['.', '!', '?', ';', ','][..])
                .trim_end()
                .to_string();
        }

        self.set_state(FlowState::Completed).await;
        self.play_sound("done.wav");
        self.emit_event(FlowEvent::TranscriptionResult(transcribed_text));
        Ok(())
    }



    async fn record_audio(
        &self,
        stop_signal: oneshot::Receiver<()>,
    ) -> Result<Vec<u8>, AudioError> {
        let device = Self::find_input_device()?;
        let (config, sample_format) = Self::get_best_config(&device)?;
        let sample_rate = config.sample_rate.0;

        println!(
            "Starting streaming recording: {} channels, {} Hz",
            config.channels, sample_rate
        );

        // Create channels for communication between threads
        let (sample_sender, sample_receiver) = crossbeam_channel::unbounded::<Vec<f32>>();
        let (stop_sender, stop_receiver) = oneshot::channel();
        let (audio_result_sender, audio_result_receiver) = oneshot::channel();

        let callback = Arc::clone(&self.callback);
        let cancellation_token = self.cancellation_token.clone();

        // Spawn the audio recording thread
        let _audio_handle = tokio::task::spawn_blocking(move || {
            Self::run_audio_recording_thread(
                device,
                config,
                sample_format,
                sample_sender,
                stop_receiver,
                audio_result_sender,
                callback.clone(),
            )
        });

        // Spawn the processing thread
        let sample_receiver_clone = sample_receiver.clone();
        let processing_handle = tokio::task::spawn_blocking(move || {
            Self::run_processing_thread(
                sample_rate,
                sample_receiver_clone,
            )
        });

        // Main event loop - just wait for stop signal or completion
        let mut stop_signal = Some(stop_signal);
        let mut audio_result_receiver = Some(audio_result_receiver);

        loop {
            tokio::select! {
                // Stop signal received
                _ = stop_signal.as_mut().unwrap() => {
                    println!("Stop signal received, stopping audio recording...");
                    let _ = stop_sender.send(());
                    break;
                }

                // Cancellation requested
                _ = cancellation_token.cancelled() => {
                    println!("Recording cancelled");
                    return Err(AudioError { message: "Recording cancelled".to_string() });
                }

                // Audio thread finished
                result = audio_result_receiver.as_mut().unwrap() => {
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

        // Drop sample_receiver to signal processing thread to finalize
        drop(sample_receiver);

        // Wait for processing thread to complete and return WebM data
        match processing_handle.await {
            Ok(Ok(webm_data)) => {
                println!("[Main Thread] Processing complete, WebM data ready: {} bytes", webm_data.len());
Ok(webm_data)
            }
            Ok(Err(e)) => {
                Err(AudioError { message: format!("Processing thread error: {}", e) })
            }
            Err(e) => {
                Err(AudioError { message: format!("Processing thread join error: {}", e) })
            }
        }
    }

    /// Processing thread that resamples and encodes audio in real-time
    fn run_processing_thread(
        input_sample_rate: u32,
        sample_receiver: crossbeam_channel::Receiver<Vec<f32>>,
    ) -> Result<Vec<u8>, String> {
        (|| -> Result<Vec<u8>, String> {
            // Calculate chunk size: 100ms of audio at input sample rate
            let chunk_size = ((input_sample_rate as f32 * 0.1) as usize).max(960);

            println!("[Processing Thread] Started with chunk size: {}", chunk_size);

            // Create streaming processor
            let mut processor = AudioStreamProcessor::new(
                input_sample_rate,
                48000, // target sample rate for WebM (Opus native rate)
                64000, // 64 kbps bitrate
                chunk_size,
            ).map_err(|e| format!("Failed to create processor: {}", e))?;

            let mut last_stats_print = Instant::now();
            let stats_interval = Duration::from_secs(10);
            let mut total_received = 0usize;
            let mut total_sample_count = 0usize;

            // Process samples as they arrive
            loop {
                match sample_receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(samples) => {
                        let sample_count = samples.len();
                        total_received += sample_count;
                        total_sample_count += sample_count;
                        
                        processor.push_samples(&samples)
                            .map_err(|e| format!("Failed to process samples: {}", e))?;

                        // Print stats periodically
                        if last_stats_print.elapsed() >= stats_interval {
                            let stats = processor.stats();
                            println!(
                                "[Processing Thread] Channel received: {} samples | \
                                 Processor received: {} samples | Resampled: {} samples | \
                                 Chunks: {} | Buffer: {}/{} ({:.1}%) | WebM: {} bytes",
                                total_received,
                                stats.samples_received,
                                stats.samples_resampled,
                                stats.chunks_processed,
                                stats.buffer_fill,
                                stats.buffer_capacity,
                                stats.buffer_fill_pct(),
                                stats.webm_buffer_size,
                            );
                            last_stats_print = Instant::now();
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        // No samples available, continue waiting
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        // Channel closed, audio recording finished
                        println!("[Processing Thread] Channel closed. Total received: {} samples", total_received);
                        println!("[Processing Thread] Finalizing processor...");
                        break;
                    }
                }
            }

            // Finalize and return WebM data
            let webm_data = processor.finalize()
                .map_err(|e| format!("Failed to finalize processor: {}", e))?;
            
            println!("[Processing Thread] Total samples processed: {}", total_sample_count);
            println!("[Processing Thread] Expected duration: {:.2}s at {}Hz", 
                total_sample_count as f64 / input_sample_rate as f64, input_sample_rate);
            
            Ok(webm_data)
        })()
    }

    fn run_audio_recording_thread(
        device: Device,
        config: StreamConfig,
        sample_format: SampleFormat,
        sample_sender: crossbeam_channel::Sender<Vec<f32>>,
        stop_receiver: oneshot::Receiver<()>,
        result_sender: oneshot::Sender<Result<(), String>>,
        callback: FlowCallback,
    ) {
        let result = (|| -> Result<(), String> {
            let channels = config.channels;

            // Track total samples captured
            let total_captured = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let total_captured_clone = total_captured.clone();

            // Find the best supported config according to our preference order:
            // 1. 48000 Hz f32
            // 2. 48000 Hz i16
            // 3. 48000 Hz i32
            // 4. >48000 Hz f32, i16, i32 (lowest above 48000 preferred)
            // 5. Any f32, i16, i32 (lowest sample rate preferred)
            // Otherwise, bail.

            // Use the provided config and sample format; only set buffer size here
            let mut config = config.clone();
            config.buffer_size = cpal::BufferSize::Fixed(2048);

            let sample_format = sample_format;

            let recording_active = Arc::new(AtomicBool::new(true));
            let recording_active_clone = Arc::clone(&recording_active);

            // Waveform computation state
            let waveform_buf = Arc::new(Mutex::new(Vec::<f32>::new()));
            let total_mono_captured = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            const WINDOW_SIZE: usize = 2048;
            const BIN_SIZE: usize = 8;

            let total_cap_f32 = total_captured_clone.clone();
            let stream = match sample_format {
                SampleFormat::F32 => {
                    let first_callback = Arc::new(AtomicBool::new(true));
                    let first_callback_clone = first_callback.clone();
                    // Clones for callback and waveform state
                    let cb = callback.clone();
                    let wf_buf = waveform_buf.clone();
                    let mono_count_f32 = total_mono_captured.clone();
                    device.build_input_stream(
                        &config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if recording_active_clone.load(Ordering::Relaxed) {
                                if first_callback_clone.load(Ordering::Relaxed) {
                                    println!("[Audio Callback] First callback - data.len()={}, channels={}, samples_per_callback={}",
                                        data.len(), channels, data.len() / channels as usize);
                                    first_callback_clone.store(false, Ordering::Relaxed);
                                }
                                total_cap_f32.fetch_add(data.len(), Ordering::Relaxed);
                                let mono_data = Self::mix_to_mono(data, channels);

                                // Update mono sample count and emit
                                mono_count_f32.fetch_add(mono_data.len(), Ordering::Relaxed);
                                let count = mono_count_f32.load(Ordering::Relaxed);
                                (cb)(FlowEvent::SampleCount(count));

                                // Accumulate and emit waveform bins per 2048-sample window
                                {
                                    let mut buf = wf_buf.lock().unwrap();
                                    buf.extend_from_slice(&mono_data);
                                    while buf.len() >= WINDOW_SIZE {
                                        let window = &buf[..WINDOW_SIZE];

                                        // Compute bins (256 bins of 8 samples RMS) and avg RMS
                                        let mut bins: Vec<f32> = Vec::with_capacity(WINDOW_SIZE / BIN_SIZE);
                                        let mut sum_sq_total: f32 = 0.0;
                                        for chunk in window.chunks(BIN_SIZE) {
                                            let mut sum_sq = 0.0f32;
                                            for &s in chunk {
                                                let ss = s * s;
                                                sum_sq += ss;
                                                sum_sq_total += ss;
                                            }
                                            bins.push((sum_sq / BIN_SIZE as f32).sqrt());
                                        }
                                        let avg_rms = (sum_sq_total / WINDOW_SIZE as f32).sqrt();

                                        (cb)(FlowEvent::WaveformChunk { bins, avg_rms });

                                        // Remove processed window
                                        buf.drain(..WINDOW_SIZE);
                                    }
                                }

                                let _ = sample_sender.send(mono_data);
                            }
                        },
                        |err| eprintln!("Audio stream error: {}", err),
                        None,
                    )
                }
                SampleFormat::I16 => {
                    let total_cap_i16 = total_captured_clone.clone();
                    let first_callback = Arc::new(AtomicBool::new(true));
                    let first_callback_clone = first_callback.clone();
                    // Clones for callback and waveform state
                    let cb = callback.clone();
                    let wf_buf = waveform_buf.clone();
                    let mono_count_i16 = total_mono_captured.clone();
                    device.build_input_stream(
                        &config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if recording_active_clone.load(Ordering::Relaxed) {
                                if first_callback_clone.load(Ordering::Relaxed) {
                                    println!("[Audio Callback] First callback - data.len()={}, channels={}, samples_per_callback={}",
                                        data.len(), channels, data.len() / channels as usize);
                                    first_callback_clone.store(false, Ordering::Relaxed);
                                }
                                total_cap_i16.fetch_add(data.len(), Ordering::Relaxed);
                                let float_data: Vec<f32> =
                                    data.iter().map(|&s| s.to_sample::<f32>()).collect();
                                let mono_data = Self::mix_to_mono(&float_data, channels);

                                // Update mono sample count and emit
                                mono_count_i16.fetch_add(mono_data.len(), Ordering::Relaxed);
                                let count = mono_count_i16.load(Ordering::Relaxed);
                                (cb)(FlowEvent::SampleCount(count));

                                // Accumulate and emit waveform bins per 2048-sample window
                                {
                                    let mut buf = wf_buf.lock().unwrap();
                                    buf.extend_from_slice(&mono_data);
                                    while buf.len() >= WINDOW_SIZE {
                                        let window = &buf[..WINDOW_SIZE];

                                        // Compute bins (256 bins of 8 samples RMS) and avg RMS
                                        let mut bins: Vec<f32> = Vec::with_capacity(WINDOW_SIZE / BIN_SIZE);
                                        let mut sum_sq_total: f32 = 0.0;
                                        for chunk in window.chunks(BIN_SIZE) {
                                            let mut sum_sq = 0.0f32;
                                            for &s in chunk {
                                                let ss = s * s;
                                                sum_sq += ss;
                                                sum_sq_total += ss;
                                            }
                                            bins.push((sum_sq / BIN_SIZE as f32).sqrt());
                                        }
                                        let avg_rms = (sum_sq_total / WINDOW_SIZE as f32).sqrt();

                                        (cb)(FlowEvent::WaveformChunk { bins, avg_rms });

                                        // Remove processed window
                                        buf.drain(..WINDOW_SIZE);
                                    }
                                }

                                let _ = sample_sender.send(mono_data);
                            }
                        },
                        |err| eprintln!("Audio stream error: {}", err),
                        None,
                    )
                }
                SampleFormat::I32 => {
                    let total_cap_i32 = total_captured_clone.clone();
                    let first_callback = Arc::new(AtomicBool::new(true));
                    let first_callback_clone = first_callback.clone();
                    // Clones for callback and waveform state
                    let cb = callback.clone();
                    let wf_buf = waveform_buf.clone();
                    let mono_count_i32 = total_mono_captured.clone();
                    device.build_input_stream(
                        &config,
                        move |data: &[i32], _: &cpal::InputCallbackInfo| {
                            if recording_active_clone.load(Ordering::Relaxed) {
                                if first_callback_clone.load(Ordering::Relaxed) {
                                    println!("[Audio Callback] First callback - data.len()={}, channels={}, samples_per_callback={}",
                                        data.len(), channels, data.len() / channels as usize);
                                    first_callback_clone.store(false, Ordering::Relaxed);
                                }
                                total_cap_i32.fetch_add(data.len(), Ordering::Relaxed);
                                let float_data: Vec<f32> =
                                    data.iter().map(|&s| s.to_sample::<f32>()).collect();
                                let mono_data = Self::mix_to_mono(&float_data, channels);

                                // Update mono sample count and emit
                                mono_count_i32.fetch_add(mono_data.len(), Ordering::Relaxed);
                                let count = mono_count_i32.load(Ordering::Relaxed);
                                (cb)(FlowEvent::SampleCount(count));

                                // Accumulate and emit waveform bins per 2048-sample window
                                {
                                    let mut buf = wf_buf.lock().unwrap();
                                    buf.extend_from_slice(&mono_data);
                                    while buf.len() >= WINDOW_SIZE {
                                        let window = &buf[..WINDOW_SIZE];

                                        // Compute bins (256 bins of 8 samples RMS) and avg RMS
                                        let mut bins: Vec<f32> = Vec::with_capacity(WINDOW_SIZE / BIN_SIZE);
                                        let mut sum_sq_total: f32 = 0.0;
                                        for chunk in window.chunks(BIN_SIZE) {
                                            let mut sum_sq = 0.0f32;
                                            for &s in chunk {
                                                let ss = s * s;
                                                sum_sq += ss;
                                                sum_sq_total += ss;
                                            }
                                            bins.push((sum_sq / BIN_SIZE as f32).sqrt());
                                        }
                                        let avg_rms = (sum_sq_total / WINDOW_SIZE as f32).sqrt();

                                        (cb)(FlowEvent::WaveformChunk { bins, avg_rms });

                                        // Remove processed window
                                        buf.drain(..WINDOW_SIZE);
                                    }
                                }

                                let _ = sample_sender.send(mono_data);
                            }
                        },
                        |err| eprintln!("Audio stream error: {}", err),
                        None,
                    )
                }
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

            let final_count = total_captured.load(Ordering::Relaxed);
            println!("[Audio Thread] Total samples captured: {}", final_count);

            Ok(())
        })();

        let _ = result_sender.send(result);
    }

    /// Rewrite transcribed text using GPT-5 to handle dictation issues
    /// (phonetic alphabet, punctuation, formatting commands, etc.)
    async fn rewrite_transcribed_text(&self, transcribed_text: &str) -> Result<String, AudioError> {
        let api_key = env::var("OPENAI_API_KEY").map_err(|_| AudioError {
            message: "OPENAI_API_KEY environment variable not set".to_string(),
        })?;

        if api_key.trim().is_empty() {
            return Err(AudioError {
                message: "OPENAI_API_KEY is empty".to_string(),
            });
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AudioError {
                message: format!("Failed to create HTTP client: {}", e),
            })?;

        let rewrite_prompt = self.rewrite_prompt.replace("{}", transcribed_text);

        let request_body = serde_json::json!({
            "model": "gpt-5",
            "input": rewrite_prompt,
            "reasoning": {
                "effort": "minimal"
            },
            "max_output_tokens": 16384,
            "service_tier": "priority"
        });

        println!("Sending rewrite request to GPT-5...");

        let request_future = client
            .post("https://api.openai.com/v1/responses")
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request_body)
            .send();

        // Wait for either response or cancellation
        let response = tokio::select! {
            result = request_future => {
                result.map_err(|e| AudioError { message: format!("Failed to send rewrite request: {}", e) })?
            }
            _ = self.cancellation_token.cancelled() => {
                return Err(AudioError { message: "Rewrite cancelled".to_string() });
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AudioError {
                message: format!("GPT-5 API error {}: {}", status, error_text),
            });
        }

        #[derive(Deserialize)]
        struct GPTResponse {
            output: Vec<GPTOutputItem>,
        }

        #[derive(Deserialize)]
        struct GPTOutputItem {
            #[serde(rename = "type")]
            output_type: String,
            content: Option<Vec<GPTContentItem>>, // Optional because reasoning items don't have content
        }

        #[derive(Deserialize)]
        struct GPTContentItem {
            text: String,
        }

        // Get the raw response text first for debugging
        let response_text = response.text().await.map_err(|e| AudioError {
            message: format!("Failed to get response text: {}", e),
        })?;

        // Try to parse it
        let gpt_response: GPTResponse = serde_json::from_str(&response_text).map_err(|e| AudioError {
            message: format!("Failed to parse rewrite response: {} | Raw response: {}", e, response_text),
        })?;

        // Extract the rewritten text from the response structure
        // Find the "message" type output item and get its content
        let rewritten_text = gpt_response
            .output
            .iter()
            .find(|item| item.output_type == "message")
            .and_then(|item| item.content.as_ref())
            .and_then(|content| content.first())
            .map(|content| content.text.clone())
            .unwrap_or_else(|| {
                eprintln!("Could not extract text from GPT-5 response, using original");
                transcribed_text.to_string()
            });

        Ok(rewritten_text)
    }

    async fn transcribe_audio(&self, audio_data: Vec<u8>) -> Result<String, AudioError> {
        let api_key = env::var("OPENAI_API_KEY").map_err(|_| AudioError {
            message: "OPENAI_API_KEY environment variable not set".to_string(),
        })?;

        if api_key.trim().is_empty() {
            return Err(AudioError {
                message: "OPENAI_API_KEY is empty".to_string(),
            });
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AudioError {
                message: format!("Failed to create HTTP client: {}", e),
            })?;

        let form = reqwest::multipart::Form::new()
            .part(
                "file",
                reqwest::multipart::Part::bytes(audio_data)
                    .file_name("audio.webm")
                    .mime_str("audio/webm")
                    .map_err(|e| AudioError {
                        message: format!("Failed to create file part: {}", e),
                    })?,
            )
            .text("model", self.model.clone());

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

    fn play_sound(&self, sound_file: &str) {
        if let Ok(mut manager) = self.audio_manager.lock() {
            manager.play_sound(sound_file);
        }
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

    fn get_best_config(device: &Device) -> Result<(StreamConfig, SampleFormat), AudioError> {
        let supported_configs = device.supported_input_configs().map_err(|_| AudioError {
            message: "Unsupported format".to_string(),
        })?;

        // Prefer 48000 Hz, mono if possible. Prefer F32, then I16, then I32.
        let mut best: Option<(u32, bool, SampleFormat, cpal::SupportedStreamConfigRange)> = None;

        for range in supported_configs {
            let supports_48k = range.min_sample_rate() <= SampleRate(48000)
                && range.max_sample_rate() >= SampleRate(48000);
            let fmt = range.sample_format();
            let channels = range.channels();
            let is_mono = channels == 1;

            // Score: lower is better
            let rate_score = if supports_48k { 0 } else { 1 };
            let fmt_score = match fmt {
                SampleFormat::F32 => 0,
                SampleFormat::I16 => 1,
                SampleFormat::I32 => 2,
                _ => 3,
            };
            let mono_score = if is_mono { 0 } else { 1 };

            let score_tuple = (rate_score, mono_score, fmt_score);

            match &best {
                None => best = Some((score_tuple.0 * 100 + score_tuple.1 * 10 + score_tuple.2, is_mono, fmt, range)),
                Some((best_score, _, _, _)) => {
                    let score = score_tuple.0 * 100 + score_tuple.1 * 10 + score_tuple.2;
                    if score < *best_score {
                        best = Some((score, is_mono, fmt, range));
                    }
                }
            }
        }

        let (_score, _mono, fmt, range) = best.ok_or_else(|| AudioError {
            message: "Unsupported format".to_string(),
        })?;

        // Choose 48000 if supported, otherwise use min sample rate in range.
        let picked_rate = if range.min_sample_rate() <= SampleRate(48000)
            && range.max_sample_rate() >= SampleRate(48000)
        {
            SampleRate(48000)
        } else {
            range.min_sample_rate()
        };

        // Build concrete config and convert to StreamConfig
        let cfg = range.with_sample_rate(picked_rate);
        let stream_config: StreamConfig = cfg.clone().into();

        Ok((stream_config, fmt))
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
