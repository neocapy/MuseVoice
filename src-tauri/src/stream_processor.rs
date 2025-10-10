use crate::webm::WebmWriter;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::error::Error;

/// Streaming audio processor that resamples and encodes audio incrementally
///
/// This processor accepts audio samples in chunks (as they arrive from the audio device),
/// buffers them until enough samples are available for the resampler, processes through
/// rubato resampling, and feeds the resampled output to the WebM encoder.
pub struct AudioStreamProcessor {
    // Resampling state
    resampler: Option<SincFixedIn<f32>>,
    input_buffer: Vec<f32>,
    resampler_chunk_size: usize,
    
    // WebM encoding state
    webm_writer: WebmWriter,
    
    // Configuration
    input_sample_rate: u32,
    target_sample_rate: u32,
    
    // Stats/monitoring
    samples_received: usize,
    samples_resampled: usize,
    chunks_processed: usize,
}

impl AudioStreamProcessor {
    /// Create a new streaming audio processor
    ///
    /// # Arguments
    /// * `input_sample_rate` - Sample rate of incoming audio (e.g., 48000)
    /// * `target_sample_rate` - Target sample rate for output (e.g., 24000)
    /// * `bitrate` - Opus bitrate in bits per second (e.g., 64000)
    /// * `resampler_chunk_size` - Number of input samples per resampling chunk
    pub fn new(
        input_sample_rate: u32,
        target_sample_rate: u32,
        bitrate: i32,
        resampler_chunk_size: usize,
    ) -> Result<Self, Box<dyn Error>> {
        println!(
            "Creating AudioStreamProcessor: {}Hz -> {}Hz, bitrate {}kbps, chunk size {}, resample_ratio(in/out)={:.6}",
            input_sample_rate, target_sample_rate, bitrate / 1000, resampler_chunk_size, input_sample_rate as f64 / target_sample_rate as f64
        );

        // Create high-quality resampler
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };

        let use_bypass = input_sample_rate == target_sample_rate;
        let resampler_opt = if use_bypass {
            println!("[AudioStreamProcessor] Bypassing resampler ({} Hz input matches target)", input_sample_rate);
            None
        } else {
            Some(SincFixedIn::<f32>::new(
                target_sample_rate as f64 / input_sample_rate as f64,
                2.0,
                params,
                resampler_chunk_size,
                1, // mono
            )?)
        };

        // Create WebM writer
        let webm_writer = WebmWriter::new(bitrate)?;

        Ok(Self {
            resampler: resampler_opt,
            input_buffer: Vec::with_capacity(resampler_chunk_size * 2),
            resampler_chunk_size,
            webm_writer,
            input_sample_rate,
            target_sample_rate,
            samples_received: 0,
            samples_resampled: 0,
            chunks_processed: 0,
        })
    }

    /// Feed samples from audio device
    ///
    /// Buffers samples and processes complete chunks through the resampler.
    /// Returns the number of samples that were processed (not buffered).
    pub fn push_samples(&mut self, samples: &[f32]) -> Result<usize, Box<dyn Error>> {
        let incoming = samples.len();
        self.samples_received += incoming;
        self.input_buffer.extend_from_slice(samples);

        let mut processed = 0;

        // Process complete chunks
        while self.input_buffer.len() >= self.resampler_chunk_size {
            let chunk: Vec<f32> = self.input_buffer.drain(..self.resampler_chunk_size).collect();
            self.process_chunk(&chunk)?;
            processed += self.resampler_chunk_size;
        }

        Ok(processed)
    }

    /// Process a single chunk through the resampler and encoder
    fn process_chunk(&mut self, chunk: &[f32]) -> Result<(), Box<dyn Error>> {
        let input_size = chunk.len();
        
        // Prepare input as 2D vector (channels x samples)
        let input = vec![chunk.to_vec()];

        // Resample or bypass
        if let Some(resampler) = self.resampler.as_mut() {
            let output = resampler.process(&input, None)?;
            // Extract the single channel and add to WebM writer
            if let Some(resampled) = output.into_iter().next() {
                let output_size = resampled.len();
                self.samples_resampled += output_size;
                self.webm_writer.add_samples_f32(&resampled)?;
        
                println!(
                    "[AudioStreamProcessor] Chunk #{}: {} samples in → {} samples out (ratio: {:.6}, expected: {:.6}, diff: {:+.3}%)",
                    self.chunks_processed + 1,
                    input_size,
                    output_size,
                    output_size as f32 / input_size as f32,
                    self.target_sample_rate as f32 / self.input_sample_rate as f32,
                    ((output_size as f32 / input_size as f32) / (self.target_sample_rate as f32 / self.input_sample_rate as f32) - 1.0) * 100.0
                );
            }
        } else {
            // Bypass resampling: feed input directly
            self.samples_resampled += input_size;
            self.webm_writer.add_samples_f32(chunk)?;
            println!(
                "[AudioStreamProcessor] Chunk #{}: {} samples passthrough (ratio: 1.000000, expected: 1.000000, diff: +0.000%)",
                self.chunks_processed + 1,
                input_size
            );
        }

        self.chunks_processed += 1;

        Ok(())
    }

    /// Finalize the encoder and return the complete WebM file
    ///
    /// Processes any remaining buffered samples (padding if necessary),
    /// finalizes the WebM container, and returns the complete file data.
    pub fn finalize(mut self) -> Result<Vec<u8>, Box<dyn Error>> {
        println!("[AudioStreamProcessor] Finalizing...");
        println!("[AudioStreamProcessor] Summary before final chunk:");
        println!("  - Total samples received: {}", self.samples_received);
        println!("  - Total samples resampled: {}", self.samples_resampled);
        println!("  - Chunks processed: {}", self.chunks_processed);
        println!("  - Remaining in buffer: {}", self.input_buffer.len());
        
        // Process remaining samples in buffer
        if let Some(resampler) = self.resampler.as_mut() {
            if !self.input_buffer.is_empty() {
                let remaining_samples = self.input_buffer.len();
                println!(
                    "[AudioStreamProcessor] Processing final partial chunk: {} samples ({}% of chunk size)",
                    remaining_samples,
                    (remaining_samples * 100) / self.resampler_chunk_size
                );

                // Process remaining samples using resampler.process_partial (no manual zero padding)
                let input = vec![self.input_buffer.clone()];
                let output = resampler.process_partial(Some(&input), None)?;
                if let Some(resampled) = output.into_iter().next() {
                    let output_size = resampled.len();
                    self.samples_resampled += output_size;
                    self.webm_writer.add_samples_f32(&resampled)?;
                    println!(
                        "[AudioStreamProcessor] Final partial: {} samples in → {} samples out (ratio: {:.6}, expected: {:.6}, diff: {:+.3}%)",
                        self.input_buffer.len(),
                        output_size,
                        output_size as f32 / self.input_buffer.len() as f32,
                        self.target_sample_rate as f32 / self.input_sample_rate as f32,
                        ((output_size as f32 / self.input_buffer.len() as f32) / (self.target_sample_rate as f32 / self.input_sample_rate as f32) - 1.0) * 100.0
                    );
                    self.chunks_processed += 1;
                }
            }

            // Flush any delayed samples from the resampler
            for _ in 0..4 {
                let flush_output = resampler.process_partial::<Vec<f32>>(None, None)?;
                if let Some(flushed) = flush_output.into_iter().next() {
                    if flushed.is_empty() {
                        break;
                    }
                    let output_size = flushed.len();
                    self.samples_resampled += output_size;
                    self.webm_writer.add_samples_f32(&flushed)?;
                    println!(
                        "[AudioStreamProcessor] Flushed delayed samples: {} out",
                        output_size
                    );
                } else {
                    break;
                }
            }
        } else {
            // Bypass: no resampler state to flush or partial to process
            if !self.input_buffer.is_empty() {
                let remaining_samples = self.input_buffer.len();
                println!(
                    "[AudioStreamProcessor] Final partial passthrough: {} samples ({}% of chunk size)",
                    remaining_samples,
                    (remaining_samples * 100) / self.resampler_chunk_size
                );
                self.samples_resampled += remaining_samples;
                self.webm_writer.add_samples_f32(&self.input_buffer)?;
                self.chunks_processed += 1;
            }
        }

        println!("[AudioStreamProcessor] Final totals:");
        println!("  - Samples received: {}", self.samples_received);
        println!("  - Samples resampled: {}", self.samples_resampled);
        println!("  - Chunks processed: {}", self.chunks_processed);
        println!("  - Observed ratio: {:.6}", self.samples_resampled as f32 / self.samples_received as f32);
        println!("  - Expected ratio: {:.6}", self.target_sample_rate as f32 / self.input_sample_rate as f32);
        println!("  - Ratio diff: {:+.3}%", (((self.samples_resampled as f32 / self.samples_received as f32) / (self.target_sample_rate as f32 / self.input_sample_rate as f32)) - 1.0) * 100.0);

        // Timestamp diagnostics before finalize
        let writer_ts_ms = self.webm_writer.current_timestamp_ms();
        let duration_by_samples_ms = (self.samples_resampled as f64 / 48000.0) * 1000.0;
        println!(
            "[AudioStreamProcessor] Pre-finalize timestamps: writer_ts_ms={} ms, duration_by_samples={:.2} ms",
            writer_ts_ms, duration_by_samples_ms
        );
        // Finalize WebM
        let webm_data = self.webm_writer.finalize()?;
        
        println!("[AudioStreamProcessor] WebM finalized: {} bytes", webm_data.len());

        Ok(webm_data)
    }

    /// Get current buffer statistics
    ///
    /// Returns (samples_in_buffer, webm_buffered_bytes)
    pub fn buffer_stats(&self) -> (usize, usize) {
        (self.input_buffer.len(), self.webm_writer.buffered_size())
    }

    /// Get processing statistics
    pub fn stats(&self) -> ProcessorStats {
        ProcessorStats {
            samples_received: self.samples_received,
            samples_resampled: self.samples_resampled,
            chunks_processed: self.chunks_processed,
            buffer_fill: self.input_buffer.len(),
            buffer_capacity: self.resampler_chunk_size,
            webm_buffer_size: self.webm_writer.buffered_size(),
        }
    }
}

/// Statistics about the processor's state
#[derive(Debug, Clone)]
pub struct ProcessorStats {
    pub samples_received: usize,
    pub samples_resampled: usize,
    pub chunks_processed: usize,
    pub buffer_fill: usize,
    pub buffer_capacity: usize,
    pub webm_buffer_size: usize,
}

impl ProcessorStats {
    /// Get buffer fill percentage (0-100)
    pub fn buffer_fill_pct(&self) -> f32 {
        if self.buffer_capacity == 0 {
            0.0
        } else {
            (self.buffer_fill as f32 / self.buffer_capacity as f32) * 100.0
        }
    }
}