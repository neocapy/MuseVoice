use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleFormat, SampleRate, StreamConfig,
};
use crossbeam_channel::{bounded, Sender, Receiver};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const OUTPUT_TIMEOUT: Duration = Duration::from_secs(180);
const PLAYBACK_SAMPLE_RATE: u32 = 48000;

const BOOWOMP: &[u8] = include_bytes!("../sounds/boowomp.mp3");
const BAMBOO_HIT: &[u8] = include_bytes!("../sounds/bamboo_hit.mp3");
const PIPE: &[u8] = include_bytes!("../sounds/pipe.mp3");
const DONE: &[u8] = include_bytes!("../sounds/done.wav");

pub enum AudioPlaybackCommand {
    PlaySound(Vec<f32>),
    Stop,
}

struct ActiveOutputStream {
    device_name: String,
    sender: Sender<AudioPlaybackCommand>,
}

pub struct AudioOutputManager {
    active_stream: Option<ActiveOutputStream>,
    last_used: Instant,
    preloaded_sounds: HashMap<String, Vec<f32>>,
}

impl AudioOutputManager {
    pub fn new() -> Arc<Mutex<Self>> {
        let mut preloaded_sounds = HashMap::new();
        
        if let Ok(samples) = decode_audio(BOOWOMP, "mp3") {
            preloaded_sounds.insert("boowomp.mp3".to_string(), samples);
        }
        if let Ok(samples) = decode_audio(BAMBOO_HIT, "mp3") {
            preloaded_sounds.insert("bamboo_hit.mp3".to_string(), samples);
        }
        if let Ok(samples) = decode_audio(PIPE, "mp3") {
            preloaded_sounds.insert("pipe.mp3".to_string(), samples);
        }
        if let Ok(samples) = decode_audio(DONE, "wav") {
            preloaded_sounds.insert("done.wav".to_string(), samples);
        }

        println!("Preloaded {} sound files", preloaded_sounds.len());

        Arc::new(Mutex::new(Self {
            active_stream: None,
            last_used: Instant::now(),
            preloaded_sounds,
        }))
    }

    pub fn start_cleanup_task(manager: Arc<Mutex<Self>>) {
        tokio::spawn(async move {
            cleanup_task(manager).await;
        });
    }

    pub fn play_sound(&mut self, sound_name: &str) {
        let samples = match self.preloaded_sounds.get(sound_name) {
            Some(s) => s.clone(),
            None => {
                eprintln!("Sound '{}' not found in preloaded sounds", sound_name);
                return;
            }
        };

        if self.should_refresh_stream() {
            if let Err(e) = self.refresh_stream() {
                eprintln!("Failed to refresh audio output stream: {}", e);
                return;
            }
        }

        if let Some(ref stream) = self.active_stream {
            if let Err(e) = stream.sender.send(AudioPlaybackCommand::PlaySound(samples)) {
                eprintln!("Failed to send playback command: {}", e);
                self.active_stream = None;
            } else {
                self.last_used = Instant::now();
            }
        }
    }

    fn should_refresh_stream(&self) -> bool {
        if self.active_stream.is_none() {
            return true;
        }

        if let Some(ref stream) = self.active_stream {
            if let Ok(current_device_name) = get_default_output_device_name() {
                if stream.device_name != current_device_name {
                    println!("Output device changed from '{}' to '{}', refreshing stream",
                        stream.device_name, current_device_name);
                    return true;
                }
            }
        }

        false
    }

    fn refresh_stream(&mut self) -> Result<(), String> {
        if let Some(old_stream) = self.active_stream.take() {
            let _ = old_stream.sender.send(AudioPlaybackCommand::Stop);
        }

        let device = find_output_device()?;
        let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        
        let config = get_output_config(&device)?;
        
        let (sender, receiver) = bounded(16);

        create_output_stream(device, config, receiver)?;
        
        self.active_stream = Some(ActiveOutputStream {
            device_name,
            sender,
        });

        Ok(())
    }

    pub fn cleanup_if_idle(&mut self) {
        if self.active_stream.is_some() && self.last_used.elapsed() > OUTPUT_TIMEOUT {
            println!("Closing idle audio output stream");
            if let Some(stream) = self.active_stream.take() {
                let _ = stream.sender.send(AudioPlaybackCommand::Stop);
            }
        }
    }
}

async fn cleanup_task(manager: Arc<Mutex<AudioOutputManager>>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        if let Ok(mut mgr) = manager.lock() {
            mgr.cleanup_if_idle();
        }
    }
}

fn find_output_device() -> Result<Device, String> {
    let host = cpal::default_host();
    
    host.default_output_device()
        .ok_or_else(|| "No output device found".to_string())
}

fn get_default_output_device_name() -> Result<String, String> {
    let device = find_output_device()?;
    device.name().map_err(|e| format!("Failed to get device name: {}", e))
}

fn get_output_config(device: &Device) -> Result<StreamConfig, String> {
    let supported_configs = device.supported_output_configs()
        .map_err(|e| format!("Failed to get output configs: {}", e))?;

    let mut best: Option<(u32, SampleFormat, cpal::SupportedStreamConfigRange)> = None;

    for range in supported_configs {
        let supports_48k = range.min_sample_rate() <= SampleRate(PLAYBACK_SAMPLE_RATE)
            && range.max_sample_rate() >= SampleRate(PLAYBACK_SAMPLE_RATE);
        let fmt = range.sample_format();

        let rate_score = if supports_48k { 0 } else { 1 };
        let fmt_score = match fmt {
            SampleFormat::F32 => 0,
            SampleFormat::I16 => 1,
            SampleFormat::I32 => 2,
            _ => 3,
        };

        let score = rate_score * 10 + fmt_score;

        match &best {
            None => best = Some((score, fmt, range)),
            Some((best_score, _, _)) => {
                if score < *best_score {
                    best = Some((score, fmt, range));
                }
            }
        }
    }

    let (_score, fmt, range) = best.ok_or_else(|| "No suitable output format found".to_string())?;

    let picked_rate = if range.min_sample_rate() <= SampleRate(PLAYBACK_SAMPLE_RATE)
        && range.max_sample_rate() >= SampleRate(PLAYBACK_SAMPLE_RATE)
    {
        PLAYBACK_SAMPLE_RATE
    } else {
        range.min_sample_rate().0
    };

    let config = StreamConfig {
        channels: 2,
        sample_rate: SampleRate(picked_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    println!("Output stream config: {} Hz, {} channels, {:?}", picked_rate, config.channels, fmt);

    Ok(config)
}

fn create_output_stream(
    device: Device,
    config: StreamConfig,
    receiver: Receiver<AudioPlaybackCommand>,
) -> Result<(), String> {
    let channels = config.channels as usize;

    let playback_buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
    let playback_buffer_clone = playback_buffer.clone();

    std::thread::spawn(move || {
        loop {
            match receiver.recv() {
                Ok(AudioPlaybackCommand::PlaySound(samples)) => {
                    let mut buf = playback_buffer_clone.lock().unwrap();
                    buf.extend_from_slice(&samples);
                }
                Ok(AudioPlaybackCommand::Stop) => break,
                Err(_) => break,
            }
        }
    });

    std::thread::spawn(move || {
        let err_fn = |err| eprintln!("Audio output stream error: {}", err);

        let stream = match device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut buf = playback_buffer.lock().unwrap();
                
                let frames_needed = data.len() / channels;
                let frames_available = buf.len().min(frames_needed);

                for i in 0..frames_available {
                    let sample = buf[i];
                    for c in 0..channels {
                        data[i * channels + c] = sample;
                    }
                }

                for i in frames_available * channels..data.len() {
                    data[i] = 0.0;
                }

                buf.drain(0..frames_available);
            },
            err_fn,
            None,
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build output stream: {}", e);
                return;
            }
        };

        if let Err(e) = stream.play() {
            eprintln!("Failed to play output stream: {}", e);
            return;
        }

        loop {
            std::thread::park();
        }
    });

    Ok(())
}

fn decode_audio(data: &[u8], format_hint: &str) -> Result<Vec<f32>, String> {
    let data_vec = data.to_vec();
    let cursor = std::io::Cursor::new(data_vec);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    hint.with_extension(format_hint);

    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(|e| format!("Failed to probe audio format: {}", e))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "No supported audio track found".to_string())?;

    let dec_opts: DecoderOptions = Default::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    let track_id = track.id;
    let mut all_samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let duration = decoded.capacity() as u64;

                let mut sample_buf = SampleBuffer::<f32>::new(duration, spec);
                sample_buf.copy_interleaved_ref(decoded);

                let samples = sample_buf.samples();
                
                let mono_samples: Vec<f32> = if spec.channels.count() == 1 {
                    samples.to_vec()
                } else {
                    samples.chunks(spec.channels.count())
                        .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
                        .collect()
                };

                all_samples.extend_from_slice(&mono_samples);
            }
            Err(e) => {
                eprintln!("Decode error: {}", e);
            }
        }
    }

    Ok(all_samples)
}

