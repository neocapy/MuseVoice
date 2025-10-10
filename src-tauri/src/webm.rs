//! WebM audio writer with Opus encoding
//!
//! This module provides functionality to encode audio to WebM format with Opus codec.
//! It handles the complete WebM container structure including EBML headers, segment info,
//! track definitions, and clustering of audio blocks.
//!
//! # Overview
//!
//! The `WebmWriter` wraps a `BufferedOpusEncoder` and manages the WebM container format.
//! It accepts audio samples in any chunk size (as f32 or i16), encodes them to Opus,
//! and organizes the output into WebM clusters for efficient streaming.
//!
//! # WebM Structure
//!
//! ```text
//! EBML Header
//!   - Version info
//!   - DocType = "webm"
//! Segment
//!   ├─ Info (duration, timecode scale)
//!   ├─ Tracks
//!   │  └─ Track 1 (Audio, CodecID="A_OPUS")
//!   │     └─ CodecPrivate (OpusHead with preskip)
//!   └─ Clusters (one per ~1 second of audio)
//!      ├─ Timestamp
//!      └─ SimpleBlocks (each contains one Opus frame)
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use muse_lib::webm::WebmWriter;
//!
//! fn encode_to_webm(audio_chunks: Vec<Vec<f32>>) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
//!     let mut writer = WebmWriter::new(64000)?;
//!
//!     for chunk in audio_chunks {
//!         writer.add_samples_f32(&chunk)?;
//!     }
//!
//!     let webm_data = writer.finalize()?;
//!     Ok(webm_data)
//! }
//! ```

use crate::ebml::EbmlBuilder;
use crate::opus::{BufferedOpusEncoder, OpusError};

/// Sample rate for audio (48kHz - Opus native rate)
const SAMPLE_RATE: u32 = 48000;

/// Frame duration in milliseconds (20ms)
const FRAME_DURATION_MS: u32 = 20;

/// Cluster duration target in milliseconds (~1 second)
const CLUSTER_DURATION_MS: u32 = 1000;

/// WebM Element IDs
mod ids {
    // Top-level elements
    pub const EBML: u32 = 0x1A45DFA3;
    pub const SEGMENT: u32 = 0x18538067;
    
    // EBML Header elements
    pub const EBML_VERSION: u16 = 0x4286;
    pub const EBML_READ_VERSION: u16 = 0x42F7;
    pub const EBML_MAX_ID_LENGTH: u16 = 0x42F2;
    pub const EBML_MAX_SIZE_LENGTH: u16 = 0x42F3;
    pub const DOC_TYPE: u16 = 0x4282;
    pub const DOC_TYPE_VERSION: u16 = 0x4287;
    pub const DOC_TYPE_READ_VERSION: u16 = 0x4285;
    
    // Segment elements
    pub const INFO: u32 = 0x1549A966;
    pub const TRACKS: u32 = 0x1654AE6B;
    pub const CLUSTER: u32 = 0x1F43B675;
    
    // Info elements
    pub const TIMECODE_SCALE: [u8; 3] = [0x2A, 0xD7, 0xB1];
    pub const DURATION: u16 = 0x4489;
    pub const MUXING_APP: u16 = 0x4D80;
    pub const WRITING_APP: u16 = 0x5741;
    
    // Track elements
    pub const TRACK_ENTRY: u8 = 0xAE;
    pub const TRACK_NUMBER: u8 = 0xD7;
    pub const TRACK_UID: u16 = 0x73C5;
    pub const TRACK_TYPE: u8 = 0x83;
    pub const FLAG_LACING: u8 = 0x9C;
    pub const LANGUAGE: [u8; 3] = [0x22, 0xB5, 0x9C];
    pub const CODEC_ID: u8 = 0x86;
    pub const CODEC_PRIVATE: u16 = 0x63A2;
    pub const CODEC_DELAY: u16 = 0x56AA;
    pub const SEEK_PRE_ROLL: u16 = 0x56BB;
    pub const AUDIO: u8 = 0xE1;
    
    // Audio elements
    pub const CHANNELS: u8 = 0x9F;
    pub const SAMPLING_FREQUENCY: u8 = 0xB5;
    pub const BIT_DEPTH: u16 = 0x6264;
    
    // Cluster elements
    pub const TIMESTAMP: u8 = 0xE7;
    pub const SIMPLE_BLOCK: u8 = 0xA3;
}

/// WebM writer that encodes audio to Opus and packages it in WebM container
pub struct WebmWriter {
    /// Opus encoder
    encoder: BufferedOpusEncoder,
    
    /// Completed clusters ready to be written
    completed_clusters: Vec<Vec<u8>>,
    
    /// Current cluster being built
    current_cluster_blocks: EbmlBuilder,
    
    /// Timestamp tracking
    current_timestamp_ms: u32,
    cluster_start_timestamp_ms: u32,
    
    /// Total samples encoded (for duration calculation)
    total_samples_encoded: u64,
    
    /// Whether finalize() has been called
    finalized: bool,
}

impl WebmWriter {
    /// Create a new WebM writer with specified bitrate
    ///
    /// # Arguments
    /// * `bitrate` - Target bitrate in bits per second (e.g., 64000 for 64kbps)
    ///
    /// # Returns
    /// A new WebmWriter instance or an error if encoder creation fails
    pub fn new(bitrate: i32) -> Result<Self, OpusError> {
        let encoder = BufferedOpusEncoder::new(bitrate)?;
        
        let mut writer = Self {
            encoder,
            completed_clusters: Vec::new(),
            current_cluster_blocks: EbmlBuilder::with_capacity(32768),
            current_timestamp_ms: 0,
            cluster_start_timestamp_ms: 0,
            total_samples_encoded: 0,
            finalized: false,
        };
        
        // Initialize first cluster with timestamp
        writer.init_cluster();
        
        Ok(writer)
    }
    
    /// Initialize a new cluster with timestamp header
    fn init_cluster(&mut self) {
        self.current_cluster_blocks.clear();
        self.current_cluster_blocks
            .u1(ids::TIMESTAMP)
            .size(4)
            .u4(self.cluster_start_timestamp_ms);
    }
    
    /// Add audio samples to the writer (i16 format)
    ///
    /// # Arguments
    /// * `samples` - Slice of mono i16 audio samples
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if encoding fails
    pub fn add_samples(&mut self, samples: &[i16]) -> Result<(), OpusError> {
        if self.finalized {
            return Err(OpusError::WebmError("Cannot add samples after finalize()".to_string()));
        }
        
        self.encoder.add_samples(samples)?;
        self.process_encoded_frames()?;
        
        Ok(())
    }
    
    /// Add audio samples to the writer (f32 format)
    ///
    /// Converts f32 samples (range -1.0 to 1.0) to i16 format and encodes them.
    ///
    /// # Arguments
    /// * `samples` - Slice of mono f32 audio samples (-1.0 to 1.0 range)
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if encoding fails
    pub fn add_samples_f32(&mut self, samples: &[f32]) -> Result<(), OpusError> {
        if self.finalized {
            return Err(OpusError::WebmError("Cannot add samples after finalize()".to_string()));
        }
        
        self.encoder.add_samples_f32(samples)?;
        self.process_encoded_frames()?;
        
        Ok(())
    }
    
    /// Process any newly encoded frames from the encoder
    fn process_encoded_frames(&mut self) -> Result<(), OpusError> {
        let frames = self.encoder.take_frames();
        
        if !frames.is_empty() {
            println!("[WebmWriter] Processing {} Opus frame(s)", frames.len());
        }

        for frame in frames {
            self.write_opus_frame(&frame)?;
        }

        Ok(())
    }
    
    /// Write an Opus frame as a SimpleBlock in the current cluster
    fn write_opus_frame(&mut self, opus_data: &[u8]) -> Result<(), OpusError> {
        // Calculate timestamp offset relative to cluster start
        let timestamp_offset = (self.current_timestamp_ms - self.cluster_start_timestamp_ms) as i16;
        
        // SimpleBlock structure:
        // - Track number (vint)
        // - Timestamp offset (2 bytes, signed)
        // - Flags (1 byte)
        // - Frame data
        
        self.current_cluster_blocks
            .u1(ids::SIMPLE_BLOCK)
            .size(4 + opus_data.len() as u64)
            .u1(0x81)  // Track number = 1 (vint encoded)
            .u2(timestamp_offset as u16)
            .u1(0x80)  // Flags: keyframe
            .bytes(opus_data);
        
        // Update timestamp for next frame
        self.current_timestamp_ms += FRAME_DURATION_MS;
        self.total_samples_encoded += 960; // 960 samples per frame at 48kHz
        
        // Check if we should start a new cluster
        if self.current_timestamp_ms >= self.cluster_start_timestamp_ms + CLUSTER_DURATION_MS {
            self.flush_cluster();
        }
        
        Ok(())
    }
    
    /// Flush the current cluster to completed_clusters and start a new one
    fn flush_cluster(&mut self) {
        // Build the cluster element
        let mut cluster = EbmlBuilder::new();
        cluster
            .u4(ids::CLUSTER)
            .payload(&self.current_cluster_blocks);
        
        // Store completed cluster
        self.completed_clusters.push(cluster.build());
        
        // Start new cluster
        self.cluster_start_timestamp_ms = self.current_timestamp_ms;
        self.init_cluster();
    }
    
    /// Finalize the WebM file and return the complete data
    ///
    /// This flushes any remaining samples through the encoder, completes the
    /// final cluster, and assembles the complete WebM file structure.
    ///
    /// # Returns
    /// The complete WebM file as a Vec<u8>, or an error
    pub fn finalize(mut self) -> Result<Vec<u8>, OpusError> {
        if self.finalized {
            return Err(OpusError::WebmError("finalize() called twice".to_string()));
        }
        
        // Finalize encoder (pads partial frames and flushes)
        self.encoder.finalize()?;
        
        // Process any final encoded frames
        self.process_encoded_frames()?;
        
        // Flush the final cluster if it has content
        if self.current_cluster_blocks.len() > 7 {  // More than just timestamp header
            self.flush_cluster();
        }
        
        // Get preskip from encoder
        let preskip = self.encoder.get_preskip()? as u16;
        
        // Calculate duration
        let duration_ms = (self.total_samples_encoded as f64 / SAMPLE_RATE as f64) * 1000.0;
        
        // Build the complete WebM structure
        let webm = self.build_webm_file(preskip, duration_ms);
        
        self.finalized = true;
        
        Ok(webm)
    }
    
    /// Build the complete WebM file structure
    fn build_webm_file(&self, preskip: u16, duration_ms: f64) -> Vec<u8> {
        let ebml_header = Self::build_ebml_header();
        let segment_info = Self::build_segment_info(duration_ms);
        let tracks = Self::build_tracks(preskip);
        
        // Combine all clusters
        let mut clusters_data = Vec::new();
        for cluster in &self.completed_clusters {
            clusters_data.extend_from_slice(cluster);
        }
        
        // Build segment
        let mut segment_payload = EbmlBuilder::new();
        segment_payload.bytes(segment_info.as_slice());
        segment_payload.bytes(tracks.as_slice());
        segment_payload.bytes(&clusters_data);
        
        // Build final WebM structure
        let mut webm = EbmlBuilder::new();
        webm.u4(ids::EBML)
            .payload(&ebml_header);
        webm.u4(ids::SEGMENT)
            .payload(&segment_payload);
        
        webm.build()
    }
    
    /// Build the EBML header
    fn build_ebml_header() -> EbmlBuilder {
        let mut header = EbmlBuilder::new();
        
        header.u2(ids::EBML_VERSION).size(1).u1(1);
        header.u2(ids::EBML_READ_VERSION).size(1).u1(1);
        header.u2(ids::EBML_MAX_ID_LENGTH).size(1).u1(4);
        header.u2(ids::EBML_MAX_SIZE_LENGTH).size(1).u1(8);
        header.u2(ids::DOC_TYPE).size(4).bytes(b"webm");
        header.u2(ids::DOC_TYPE_VERSION).size(1).u1(4);
        header.u2(ids::DOC_TYPE_READ_VERSION).size(1).u1(2);
        
        header
    }
    
    /// Build the Segment Info element
    fn build_segment_info(duration_ms: f64) -> EbmlBuilder {
        let mut info_children = EbmlBuilder::new();
        
        // TimecodeScale = 1000000 (1ms per tick)
        info_children.bytes(&ids::TIMECODE_SCALE).size(4).u4(1000000);
        
        // Duration
        info_children.u2(ids::DURATION).size(8).f8(duration_ms);
        
        // MuxingApp
        let app_name = b"MuseVoice-0.1.0";
        info_children.u2(ids::MUXING_APP).size(app_name.len() as u64).bytes(app_name);
        
        // WritingApp
        info_children.u2(ids::WRITING_APP).size(app_name.len() as u64).bytes(app_name);
        
        let mut info = EbmlBuilder::new();
        info.u4(ids::INFO).payload(&info_children);
        
        info
    }
    
    /// Build the Tracks element with Opus audio track
    fn build_tracks(preskip: u16) -> EbmlBuilder {
        // Audio element
        let mut audio = EbmlBuilder::new();
        audio.u1(ids::CHANNELS).size(1).u1(1);  // Mono
        audio.u1(ids::SAMPLING_FREQUENCY).size(8).f8(SAMPLE_RATE as f64);
        audio.u2(ids::BIT_DEPTH).size(1).u1(16);
        
        // Build OpusHead structure for CodecPrivate
        let opus_head = Self::build_opus_head(preskip);
        
        // Track Entry
        let mut track_entry = EbmlBuilder::new();
        
        track_entry.u1(ids::TRACK_NUMBER).size(1).u1(1);
        
        // TrackUID
        track_entry.u2(ids::TRACK_UID).size(8)
            .u4(0xDEADBEEF)
            .u4(0x12345678);
        
        track_entry.u1(ids::FLAG_LACING).size(1).u1(0);
        
        track_entry.bytes(&ids::LANGUAGE).size(3).bytes(b"eng");
        
        track_entry.u1(ids::CODEC_ID).size(6).bytes(b"A_OPUS");
        
        // Codec delay derived from preskip (in nanoseconds)
        track_entry.u2(ids::CODEC_DELAY).size(8).u8(((preskip as u64) * 1_000_000_000u64 / SAMPLE_RATE as u64));
        
        // Seek pre-roll (80000000 ns = 80ms)
        track_entry.u2(ids::SEEK_PRE_ROLL).size(4).u4(80000000);
        
        track_entry.u1(ids::TRACK_TYPE).size(1).u1(0x02);  // Audio
        
        track_entry.u1(ids::AUDIO).payload(&audio);
        
        track_entry.u2(ids::CODEC_PRIVATE).size(opus_head.len() as u64).bytes(&opus_head);
        
        // Track element
        let mut track = EbmlBuilder::new();
        track.u1(ids::TRACK_ENTRY).payload(&track_entry);
        
        // Tracks element
        let mut tracks = EbmlBuilder::new();
        tracks.u4(ids::TRACKS).payload(&track);
        
        tracks
    }
    
    /// Build the OpusHead structure for CodecPrivate
    fn build_opus_head(preskip: u16) -> Vec<u8> {
        let mut head = Vec::with_capacity(19);
        
        // Magic signature
        head.extend_from_slice(b"OpusHead");
        
        // Version
        head.push(1);
        
        // Channel count
        head.push(1);  // Mono
        
        // Pre-skip (little-endian u16)
        head.push((preskip & 0xFF) as u8);
        head.push(((preskip >> 8) & 0xFF) as u8);
        
        // Input sample rate (little-endian u32) - use 48000
        head.push((SAMPLE_RATE & 0xFF) as u8);
        head.push(((SAMPLE_RATE >> 8) & 0xFF) as u8);
        head.push(((SAMPLE_RATE >> 16) & 0xFF) as u8);
        head.push(((SAMPLE_RATE >> 24) & 0xFF) as u8);
        
        // Output gain (little-endian i16) - 0
        head.push(0);
        head.push(0);
        
        // Channel mapping family - 0 (mono/stereo)
        head.push(0);
        
        head
    }
    
    /// Get approximate size of buffered data in bytes
    ///
    /// This includes completed clusters and the current cluster being built.
    pub fn buffered_size(&self) -> usize {
        let completed_size: usize = self.completed_clusters.iter().map(|c| c.len()).sum();
        completed_size + self.current_cluster_blocks.len()
    }
    
    /// Get the current timestamp in milliseconds
    pub fn current_timestamp_ms(&self) -> u32 {
        self.current_timestamp_ms
    }
    
    /// Get the number of completed clusters
    pub fn cluster_count(&self) -> usize {
        self.completed_clusters.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_webm_writer_creation() {
        let writer = WebmWriter::new(64000);
        assert!(writer.is_ok());
    }
    
    #[test]
    fn test_add_samples_i16() {
        let mut writer = WebmWriter::new(64000).unwrap();
        let samples = vec![0i16; 960];
        
        let result = writer.add_samples(&samples);
        assert!(result.is_ok());
        assert_eq!(writer.current_timestamp_ms(), 20);
    }
    
    #[test]
    fn test_add_samples_f32() {
        let mut writer = WebmWriter::new(64000).unwrap();
        let samples = vec![0.0f32; 960];
        
        let result = writer.add_samples_f32(&samples);
        assert!(result.is_ok());
        assert_eq!(writer.current_timestamp_ms(), 20);
    }
    
    #[test]
    fn test_clustering() {
        let mut writer = WebmWriter::new(64000).unwrap();
        
        // Add ~2 seconds of audio (should create 2 clusters)
        for _ in 0..100 {
            writer.add_samples(&vec![0i16; 960]).unwrap();
        }
        
        // Should have at least 1 completed cluster
        assert!(writer.cluster_count() >= 1);
    }
    
    #[test]
    fn test_finalize() {
        let mut writer = WebmWriter::new(64000).unwrap();
        writer.add_samples(&vec![0i16; 960]).unwrap();
        
        let webm_data = writer.finalize();
        assert!(webm_data.is_ok());
        
        let data = webm_data.unwrap();
        // Check for EBML header magic
        assert_eq!(&data[0..4], &[0x1A, 0x45, 0xDF, 0xA3]);
    }
    
    #[test]
    fn test_double_finalize_error() {
        let mut writer = WebmWriter::new(64000).unwrap();
        writer.add_samples(&vec![0i16; 960]).unwrap();
        
        let _ = writer.finalize().unwrap();
        // Can't test second finalize because it consumes self
        // This test just ensures finalize works
    }
    
    #[test]
    fn test_opus_head_structure() {
        let preskip = 312u16;
        let head = WebmWriter::build_opus_head(preskip);
        
        assert_eq!(head.len(), 19);
        assert_eq!(&head[0..8], b"OpusHead");
        assert_eq!(head[8], 1); // Version
        assert_eq!(head[9], 1); // Channels
        assert_eq!(head[10], (preskip & 0xFF) as u8);
        assert_eq!(head[11], ((preskip >> 8) & 0xFF) as u8);
    }
}