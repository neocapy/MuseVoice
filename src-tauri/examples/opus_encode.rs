//! Example demonstrating Buffered Opus Encoder usage
//!
//! Run with: cargo run --example opus_encode

use muse_lib::opus::{BufferedOpusEncoder, OpusError};

fn main() -> Result<(), OpusError> {
    println!("=== Buffered Opus Encoder Example ===\n");

    // Create an encoder with 24kbps bitrate
    let mut encoder = BufferedOpusEncoder::new(24000)?;
    println!("✓ Created encoder with 24kbps bitrate");

    // Example 1: Encoding exact frames
    println!("\n--- Example 1: Exact Frame Encoding ---");
    let exact_frame = vec![0i16; 480]; // Exactly 480 samples (10ms at 48kHz)
    encoder.add_samples(&exact_frame)?;
    println!("Added 480 samples (1 complete frame)");
    println!("Encoded frames available: {}", encoder.frame_count());

    let frames = encoder.take_frames();
    println!("Retrieved {} opus frame(s)", frames.len());
    for (i, frame) in frames.iter().enumerate() {
        println!("  Frame {}: {} bytes", i, frame.len());
    }

    // Example 2: Irregular chunk sizes
    println!("\n--- Example 2: Irregular Chunk Sizes ---");
    let chunks = vec![
        vec![100i16; 300],   // 300 samples
        vec![200i16; 500],   // 500 samples
        vec![300i16; 700],   // 700 samples
        vec![400i16; 460],   // 460 samples
    ];

    for (i, chunk) in chunks.iter().enumerate() {
        encoder.add_samples(chunk)?;
        println!(
            "Added chunk {}: {} samples | Buffered: {} | Encoded frames: {}",
            i + 1,
            chunk.len(),
            encoder.buffered_samples(),
            encoder.frame_count()
        );
    }

    let frames = encoder.take_frames();
    println!("\nRetrieved {} opus frame(s) from irregular chunks", frames.len());

    // Example 3: Finalizing with remainder
    println!("\n--- Example 3: Finalize with Remainder ---");
    let mut partial_encoder = BufferedOpusEncoder::new(24000)?;
    partial_encoder.add_samples(&vec![500i16; 100])?;
    println!(
        "Added 100 samples | Buffered: {} | Encoded frames: {}",
        partial_encoder.buffered_samples(),
        partial_encoder.frame_count()
    );

    partial_encoder.finalize()?;
    println!(
        "After finalize | Buffered: {} | Encoded frames: {}",
        partial_encoder.buffered_samples(),
        partial_encoder.frame_count()
    );

    let frames = partial_encoder.take_frames();
    println!("Retrieved {} opus frame(s) after finalize (includes 2 flush frames)", frames.len());

    // Example 4: Simulating streaming audio
    println!("\n--- Example 4: Streaming Simulation ---");
    let mut new_encoder = BufferedOpusEncoder::new(24000)?;
    println!("Created new encoder with 24kbps bitrate");

    // Simulate receiving audio in various chunk sizes
    let chunk_sizes = vec![128, 256, 512, 384, 640, 100, 860];
    let mut total_samples = 0;
    let mut total_encoded = 0;

    for chunk_size in chunk_sizes {
        let chunk = vec![0i16; chunk_size];
        new_encoder.add_samples(&chunk)?;
        total_samples += chunk_size;

        let available_frames = new_encoder.frame_count();
        if available_frames > 0 {
            let frames = new_encoder.take_frames();
            total_encoded += frames.len();
            println!(
                "  Added {} samples (total: {}) -> {} new frame(s) ready",
                chunk_size, total_samples, frames.len()
            );
        }
    }

    // Don't forget to finalize!
    new_encoder.finalize()?;
    let final_frames = new_encoder.take_frames();
    if !final_frames.is_empty() {
        total_encoded += final_frames.len();
        println!(
            "  Finalized -> {} final frame(s)",
            final_frames.len()
        );
    }

    println!("\nTotal: {} samples -> {} opus frames", total_samples, total_encoded);
    println!("Expected frames: ~{}", (total_samples + 479) / 480);

    // Example 5: Adjusting encoder settings
    println!("\n--- Example 5: Encoder Settings ---");
    let mut config_encoder = BufferedOpusEncoder::new(24000)?;
    println!("Initial bitrate: 24kbps");

    config_encoder.set_bitrate(128000)?;
    println!("✓ Changed bitrate to 128kbps");

    config_encoder.set_complexity(5)?;
    println!("✓ Changed complexity to 5 (default is 10)");

    // Encode some audio with new settings
    config_encoder.add_samples(&vec![0i16; 480])?;
    let frames = config_encoder.take_frames();
    println!("Encoded {} frame(s) with new settings", frames.len());

    println!("\n=== All examples completed successfully! ===");
    Ok(())
}
