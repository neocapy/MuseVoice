use crate::webm::WebmWriter;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::time::Instant;

/// Resamples audio data to the specified sample rate and encodes it as a WebM Opus file in memory
///
/// # Arguments
/// * `samples` - The audio samples as f32 mono data
/// * `original_sample_rate` - The sample rate of the input audio
/// * `target_sample_rate` - The desired output sample rate in Hz (typically 48000 for Opus)
/// * `bitrate` - Target bitrate in bits per second (e.g., 64000 for 64kbps)
///
/// # Returns
/// * `Result<Vec<u8>, Box<dyn std::error::Error>>` - WebM file data as bytes or error
pub fn resample_and_encode_webm(
    samples: Vec<f32>,
    original_sample_rate: u32,
    target_sample_rate: u32,
    bitrate: i32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {

    // If already at target sample rate, skip resampling
    let resampled_samples = if original_sample_rate == target_sample_rate {
        println!("Audio already at target sample rate ({}Hz), skipping resampling", target_sample_rate);
        samples
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
            samples.len(), // chunk_size
            1, // number of channels (mono)
        )?;

        // Prepare input as 2D vector (channels x samples)
        let input = vec![samples];

        // Resample
        let output = resampler.process(&input, None)?;

        // Extract the single channel
        let resampled = output.into_iter().next().unwrap_or_default();

        let resample_duration = resample_start.elapsed();
        println!("Resampling completed in {:.2}ms", resample_duration.as_secs_f64() * 1000.0);

        resampled
    };

    println!("Encoding audio as WebM Opus ({}kbps)...", bitrate / 1000);
    let encode_start = Instant::now();

    // Create WebM writer
    let mut writer = WebmWriter::new(bitrate)?;

    // Add all samples to the writer
    writer.add_samples_f32(&resampled_samples)?;

    // Finalize and get the complete WebM file
    let webm_data = writer.finalize()?;

    let encode_duration = encode_start.elapsed();
    println!("WebM encoding completed in {:.2}ms", encode_duration.as_secs_f64() * 1000.0);
    println!("WebM file size: {} bytes", webm_data.len());

    // Return the WebM data as bytes
    Ok(webm_data)
}
