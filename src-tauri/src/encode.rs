use crate::flow::RecordingData;
use hound::{SampleFormat, WavSpec, WavWriter};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::io::Cursor;
use std::time::Instant;

/// Resamples audio data to 16kHz and encodes it as a WAV file in memory
///
/// # Arguments
/// * `recording` - The RecordingData containing samples and original sample rate
///
/// # Returns
/// * `Result<Vec<u8>, Box<dyn std::error::Error>>` - WAV file data as bytes or error
pub fn resample_and_encode_wav(
    recording: RecordingData,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let target_sample_rate = 24000u32;
    let original_sample_rate = recording.sample_rate;

    // If already at target sample rate, skip resampling
    let resampled_samples = if original_sample_rate == target_sample_rate {
        println!("Audio already at target sample rate (24kHz), skipping resampling");
        recording.samples
    } else {
        println!("Resampling from {}Hz to {}Hz...", original_sample_rate, target_sample_rate);
        let resample_start = Instant::now();
        
        // Create high-quality resampler parameters
        let params = SincInterpolationParameters {
            sinc_len: 256, // Higher for better quality (default is 256)
            f_cutoff: 0.95, // Good tradeoff between aliasing and bandwidth
            interpolation: SincInterpolationType::Linear, // Cubic is high quality
            oversampling_factor: 128, // High oversampling for quality
            window: WindowFunction::BlackmanHarris2, // Good window for audio
        };

        let mut resampler = SincFixedIn::<f32>::new(
            target_sample_rate as f64 / original_sample_rate as f64,
            2.0, // max_resample_ratio_relative
            params,
            recording.samples.len(), // chunk_size: 256 is recommended for quality and performance
            1, // number of channels (mono)
        )?;

        // Prepare input as 2D vector (channels x samples)
        let input = vec![recording.samples];

        // Resample
        let output = resampler.process(&input, None)?;

        // Extract the single channel
        let resampled = output.into_iter().next().unwrap_or_default();
        
        let resample_duration = resample_start.elapsed();
        println!("Resampling completed in {:.2}ms", resample_duration.as_secs_f64() * 1000.0);
        
        resampled
    };

    println!("Encoding audio as WAV...");
    let encode_start = Instant::now();

    // Create WAV specification for 16-bit PCM, mono, 24kHz
    let spec = WavSpec {
        channels: 1,
        sample_rate: target_sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    // Create a cursor to write WAV data to memory
    let mut cursor = Cursor::new(Vec::new());

    {
        // Create WAV writer
        let mut writer = WavWriter::new(&mut cursor, spec)?;

        // Convert f32 samples to i16 and write them
        for sample in resampled_samples {
            // Clamp sample to [-1.0, 1.0] range and convert to i16
            let clamped_sample = sample.clamp(-1.0, 1.0);
            let i16_sample = (clamped_sample * i16::MAX as f32) as i16;
            writer.write_sample(i16_sample)?;
        }

        // Finalize the WAV file
        writer.finalize()?;
    }

    let encode_duration = encode_start.elapsed();
    println!("WAV encoding completed in {:.2}ms", encode_duration.as_secs_f64() * 1000.0);

    // Return the WAV data as bytes
    Ok(cursor.into_inner())
}
