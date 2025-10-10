//! Example demonstrating WebM audio writer with Opus encoding
//!
//! This example shows how to use the WebmWriter to encode audio samples
//! to a WebM file with Opus codec. It demonstrates various use cases including
//! exact frames, irregular chunks, f32 samples, and saving to disk.
//!
//! Run with: cargo run --example webm_encode

use muse_lib::webm::WebmWriter;
use muse_lib::opus::OpusError;
use std::f32::consts::PI;

fn main() -> Result<(), OpusError> {
    println!("=== WebM Audio Writer Example ===\n");

    // Example 1: Simple encoding with silence
    println!("--- Example 1: Encode Silent Audio ---");
    {
        let mut writer = WebmWriter::new(64000)?;
        println!("✓ Created WebM writer with 64kbps bitrate");

        // Add 1 second of silence (24000 samples at 24kHz)
        let silent_samples = vec![0i16; 24000];
        writer.add_samples(&silent_samples)?;
        
        println!("Added 24000 samples (1 second)");
        println!("Current timestamp: {}ms", writer.current_timestamp_ms());
        println!("Clusters created: {}", writer.cluster_count());

        let webm_data = writer.finalize()?;
        println!("✓ Finalized WebM file: {} bytes", webm_data.len());
        
        // Save to file
        std::fs::write("silent_1s.webm", &webm_data)?;
        println!("✓ Saved to silent_1s.webm");
    }

    // Example 2: Encoding with f32 samples (sine wave)
    println!("\n--- Example 2: Encode Sine Wave (f32) ---");
    {
        let mut writer = WebmWriter::new(96000)?;
        println!("✓ Created WebM writer with 96kbps bitrate");

        // Generate 2 seconds of 440Hz sine wave
        let sample_rate = 24000.0;
        let frequency = 440.0; // A4 note
        let duration_samples = (2.0 * sample_rate) as usize;
        
        let mut sine_wave = Vec::with_capacity(duration_samples);
        for i in 0..duration_samples {
            let t = i as f32 / sample_rate;
            let sample = 0.5 * (2.0 * PI * frequency * t).sin();
            sine_wave.push(sample);
        }
        
        println!("Generated {}Hz sine wave: {} samples", frequency, sine_wave.len());
        
        writer.add_samples_f32(&sine_wave)?;
        println!("Current timestamp: {}ms", writer.current_timestamp_ms());
        println!("Clusters created: {}", writer.cluster_count());

        let webm_data = writer.finalize()?;
        println!("✓ Finalized WebM file: {} bytes", webm_data.len());
        
        std::fs::write("sine_440hz_2s.webm", &webm_data)?;
        println!("✓ Saved to sine_440hz_2s.webm");
    }

    // Example 3: Irregular chunk sizes (simulating streaming)
    println!("\n--- Example 3: Streaming with Irregular Chunks ---");
    {
        let mut writer = WebmWriter::new(64000)?;
        println!("✓ Created WebM writer for streaming");

        // Simulate receiving audio in various chunk sizes
        let chunk_sizes = vec![128, 256, 512, 384, 640, 100, 860, 1024, 333];
        let mut total_samples = 0;

        for (i, chunk_size) in chunk_sizes.iter().enumerate() {
            // Generate some noise for variety
            let chunk: Vec<f32> = (0..*chunk_size)
                .map(|_| (rand::random::<f32>() - 0.5) * 0.1)
                .collect();
            
            writer.add_samples_f32(&chunk)?;
            total_samples += chunk_size;
            
            if (i + 1) % 3 == 0 {
                println!(
                    "  Processed {} chunks ({} samples) | timestamp: {}ms | clusters: {}",
                    i + 1,
                    total_samples,
                    writer.current_timestamp_ms(),
                    writer.cluster_count()
                );
            }
        }

        println!("Total samples processed: {}", total_samples);
        
        let webm_data = writer.finalize()?;
        println!("✓ Finalized WebM file: {} bytes", webm_data.len());
        
        std::fs::write("streaming_irregular.webm", &webm_data)?;
        println!("✓ Saved to streaming_irregular.webm");
    }

    // Example 4: Multi-second encoding with clustering
    println!("\n--- Example 4: Long Recording with Clustering ---");
    {
        let mut writer = WebmWriter::new(48000)?;
        println!("✓ Created WebM writer with 48kbps bitrate");

        // Generate 5 seconds of audio with varying tones
        let sample_rate = 24000.0;
        let duration_seconds = 5.0;
        let samples_per_chunk = 4800; // 200ms chunks
        let total_samples = (duration_seconds * sample_rate) as usize;
        
        println!("Encoding {} seconds of audio in {}ms chunks...", 
                 duration_seconds, 
                 (samples_per_chunk as f32 / sample_rate * 1000.0) as u32);

        let mut samples_encoded = 0;
        while samples_encoded < total_samples {
            let chunk_size = (total_samples - samples_encoded).min(samples_per_chunk);
            
            // Generate a chirp (frequency increases over time)
            let mut chunk = Vec::with_capacity(chunk_size);
            for i in 0..chunk_size {
                let global_idx = samples_encoded + i;
                let t = global_idx as f32 / sample_rate;
                // Frequency sweeps from 200Hz to 800Hz
                let freq = 200.0 + (t / duration_seconds as f32) * 600.0;
                let sample = 0.3 * (2.0 * PI * freq * t).sin();
                chunk.push(sample);
            }
            
            writer.add_samples_f32(&chunk)?;
            samples_encoded += chunk_size;
            
            if samples_encoded % 24000 == 0 {
                println!(
                    "  Progress: {:.1}s / {:.1}s | clusters: {} | buffered: {} bytes",
                    samples_encoded as f32 / sample_rate,
                    duration_seconds,
                    writer.cluster_count(),
                    writer.buffered_size()
                );
            }
        }

        let final_cluster_count = writer.cluster_count();
        let webm_data = writer.finalize()?;
        println!("✓ Finalized WebM file: {} bytes", webm_data.len());
        println!("  Final clusters: {}", final_cluster_count);
        println!("  Compression ratio: {:.2}x", 
                 (total_samples * 2) as f32 / webm_data.len() as f32);
        
        std::fs::write("chirp_5s.webm", &webm_data)?;
        println!("✓ Saved to chirp_5s.webm");
    }

    // Example 5: Minimal file (testing edge cases)
    println!("\n--- Example 5: Minimal Audio (Edge Cases) ---");
    {
        let mut writer = WebmWriter::new(32000)?;
        println!("✓ Created WebM writer with 32kbps bitrate");

        // Add just a tiny bit of audio
        let tiny_samples = vec![0.1f32; 100]; // Less than one frame
        writer.add_samples_f32(&tiny_samples)?;
        
        println!("Added only {} samples (partial frame)", tiny_samples.len());

        let webm_data = writer.finalize()?;
        println!("✓ Finalized minimal WebM file: {} bytes", webm_data.len());
        
        std::fs::write("minimal.webm", &webm_data)?;
        println!("✓ Saved to minimal.webm");
    }

    // Summary and verification hints
    println!("\n=== All Examples Completed Successfully! ===");
    println!("\nGenerated files:");
    println!("  • silent_1s.webm          - 1 second of silence");
    println!("  • sine_440hz_2s.webm      - 2 second 440Hz tone");
    println!("  • streaming_irregular.webm - Irregular chunks");
    println!("  • chirp_5s.webm           - 5 second frequency sweep");
    println!("  • minimal.webm            - Minimal partial frame");
    
    println!("\n💡 Verify files with:");
    println!("   ffprobe <filename>");
    println!("   ffplay <filename>");
    println!("   mediainfo <filename>");
    
    Ok(())
}

// Simple random number generator for the example
mod rand {
    use std::cell::Cell;
    
    thread_local! {
        static SEED: Cell<u64> = Cell::new(0x123456789ABCDEF0);
    }
    
    pub fn random<T>() -> T 
    where
        T: From<f32>
    {
        SEED.with(|seed| {
            let mut s = seed.get();
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            seed.set(s);
            T::from(s as f32 / u64::MAX as f32)
        })
    }
}