//! Safe Rust wrapper for libopus encoding
//!
//! This module provides a safe interface to the opus encoder, handling
//! buffering of irregularly-sized audio chunks and producing fixed-size
//! opus frames (20ms at 48kHz = 960 samples).
//!
//! # Overview
//!
//! The `BufferedOpusEncoder` handles the complexity of opus encoding by:
//! - Buffering audio samples until a complete 20ms frame is available
//! - Automatically encoding complete frames
//! - Zero-padding the final incomplete frame when finalized
//! - Providing thread-safe access to encoded opus packets
//!
//! # Example
//!
//! ```rust,no_run
//! use muse_lib::opus::{BufferedOpusEncoder, OpusError};
//!
//! fn encode_audio(audio_chunks: Vec<Vec<i16>>) -> Result<Vec<Vec<u8>>, OpusError> {
//!     // Create encoder with 64kbps bitrate
//!     let mut encoder = BufferedOpusEncoder::new(64000)?;
//!
//!     // Add irregularly-sized audio chunks
//!     for chunk in audio_chunks {
//!         encoder.add_samples(&chunk)?;
//!     }
//!
//!     // Finalize to encode any remaining samples
//!     encoder.finalize()?;
//!
//!     // Get all encoded opus frames
//!     Ok(encoder.take_frames())
//! }
//! ```
//!
//! # Technical Details
//!
//! - **Sample Rate**: 48kHz (opus native rate)
//! - **Frame Size**: 960 samples (20ms at 48kHz)
//! - **Channels**: Mono (1 channel)
//! - **Application Type**: OPUS_APPLICATION_AUDIO (general audio)
//!
//! # Usage Pattern
//!
//! 1. Create encoder with desired bitrate
//! 2. Feed audio samples in any chunk size using `add_samples()`
//! 3. Periodically retrieve encoded frames with `take_frames()`
//! 4. Call `finalize()` when done to flush remaining samples
//! 5. Retrieve final frames with `take_frames()`

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

// Include the generated bindings
include!(concat!(env!("OUT_DIR"), "/opus_bindings.rs"));

/// Sample rate for opus encoding (48kHz is the native rate for opus)
const SAMPLE_RATE: i32 = 48000;

/// Frame size for 20ms at 48kHz
const FRAME_SIZE: usize = 960;

/// Maximum packet size for opus (as recommended in the docs)
const MAX_PACKET_SIZE: usize = 4000;

/// Errors that can occur during opus encoding or WebM writing
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpusError {
    /// Invalid arguments passed to opus
    BadArg,
    /// Buffer too small
    BufferTooSmall,
    /// Internal opus error
    InternalError,
    /// Invalid packet
    InvalidPacket,
    /// Unimplemented feature
    Unimplemented,
    /// Invalid state
    InvalidState,
    /// Memory allocation failed
    AllocFail,
    /// Unknown error code from opus
    Unknown(i32),
    /// WebM-specific errors
    WebmError(String),
    /// I/O error
    IoError(String),
}

impl OpusError {
    fn from_code(code: i32) -> Self {
        match code {
            -1 => OpusError::BadArg,
            -2 => OpusError::BufferTooSmall,
            -3 => OpusError::InternalError,
            -4 => OpusError::InvalidPacket,
            -5 => OpusError::Unimplemented,
            -6 => OpusError::InvalidState,
            -7 => OpusError::AllocFail,
            _ => OpusError::Unknown(code),
        }
    }
}

impl std::fmt::Display for OpusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpusError::BadArg => write!(f, "Invalid argument"),
            OpusError::BufferTooSmall => write!(f, "Buffer too small"),
            OpusError::InternalError => write!(f, "Internal error"),
            OpusError::InvalidPacket => write!(f, "Invalid packet"),
            OpusError::Unimplemented => write!(f, "Unimplemented"),
            OpusError::InvalidState => write!(f, "Invalid state"),
            OpusError::AllocFail => write!(f, "Memory allocation failed"),
            OpusError::Unknown(code) => write!(f, "Unknown error: {}", code),
            OpusError::WebmError(msg) => write!(f, "WebM error: {}", msg),
            OpusError::IoError(msg) => write!(f, "I/O error: {}", msg),
        }
    }
}

impl std::error::Error for OpusError {}

impl From<std::io::Error> for OpusError {
    fn from(err: std::io::Error) -> Self {
        OpusError::IoError(err.to_string())
    }
}

/// A buffered opus encoder that handles irregularly-sized audio chunks
/// and produces fixed-size opus frames.
pub struct BufferedOpusEncoder {
    /// The raw opus encoder pointer
    encoder: *mut OpusEncoder,
    /// Buffer for accumulating samples until we have a full frame
    sample_buffer: Vec<i16>,
    /// Completed opus frames ready to be retrieved
    encoded_frames: Vec<Vec<u8>>,
    /// Temporary buffer for encoding
    packet_buffer: Vec<u8>,
}

impl BufferedOpusEncoder {
    /// Create a new opus encoder for mono audio at 48kHz
    ///
    /// # Arguments
    /// * `bitrate` - Target bitrate in bits per second (e.g., 64000 for 64kbps)
    ///
    /// # Returns
    /// A new BufferedOpusEncoder instance or an error if creation fails
    pub fn new(bitrate: i32) -> Result<Self, OpusError> {
        let mut error: i32 = 0;

        // Create the encoder (48kHz, mono, audio application)
        let encoder = unsafe {
            opus_encoder_create(
                SAMPLE_RATE,
                1, // mono
                OPUS_APPLICATION_AUDIO as i32,
                &mut error as *mut i32,
            )
        };

        if error != 0 {
            return Err(OpusError::from_code(error));
        }

        if encoder.is_null() {
            return Err(OpusError::AllocFail);
        }

        // Set the bitrate
        let result = unsafe {
            opus_encoder_ctl(encoder, OPUS_SET_BITRATE_REQUEST as i32, bitrate)
        };

        if result != 0 {
            unsafe { opus_encoder_destroy(encoder) };
            return Err(OpusError::from_code(result));
        }

        Ok(Self {
            encoder,
            sample_buffer: Vec::with_capacity(FRAME_SIZE * 2),
            encoded_frames: Vec::new(),
            packet_buffer: vec![0u8; MAX_PACKET_SIZE],
        })
    }

    /// Add audio samples to the encoder (i16 format)
    ///
    /// This method accepts any number of samples. They will be buffered
    /// internally until we have enough for a complete 20ms frame (960 samples at 48kHz),
    /// at which point they will be encoded automatically.
    ///
    /// # Arguments
    /// * `samples` - Slice of mono i16 audio samples
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if encoding fails
    pub fn add_samples(&mut self, samples: &[i16]) -> Result<(), OpusError> {
        // Add samples to our buffer
        self.sample_buffer.extend_from_slice(samples);

        // Encode as many complete frames as we can
        while self.sample_buffer.len() >= FRAME_SIZE {
            // Take exactly FRAME_SIZE samples
            let frame: Vec<i16> = self.sample_buffer.drain(..FRAME_SIZE).collect();

            // Encode this frame
            let encoded_len = unsafe {
                opus_encode(
                    self.encoder,
                    frame.as_ptr(),
                    FRAME_SIZE as i32,
                    self.packet_buffer.as_mut_ptr(),
                    MAX_PACKET_SIZE as i32,
                )
            };

            if encoded_len < 0 {
                return Err(OpusError::from_code(encoded_len));
            }

            // Store the encoded frame (skip DTX frames which are 2 bytes or less)
            if encoded_len > 2 {
                let encoded_frame = self.packet_buffer[..encoded_len as usize].to_vec();
                self.encoded_frames.push(encoded_frame);
            }
        }

        Ok(())
    }

    /// Add audio samples to the encoder (f32 format)
    ///
    /// Converts f32 samples (range -1.0 to 1.0) to i16 format and encodes them.
    /// This is a convenience method for audio backends that provide float samples.
    ///
    /// # Arguments
    /// * `samples` - Slice of mono f32 audio samples (-1.0 to 1.0 range)
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if encoding fails
    pub fn add_samples_f32(&mut self, samples: &[f32]) -> Result<(), OpusError> {
        // Convert f32 to i16
        let i16_samples: Vec<i16> = samples
            .iter()
            .map(|&s| {
                // Clamp to [-1.0, 1.0] and convert to i16 range
                let clamped = s.clamp(-1.0, 1.0);
                (clamped * 32767.0) as i16
            })
            .collect();

        self.add_samples(&i16_samples)
    }

    /// Finalize encoding by padding and encoding any remaining samples
    ///
    /// If there are leftover samples that don't make up a complete frame,
    /// they will be zero-padded on the right to make a full frame and encoded.
    ///
    /// This also pushes two silent frames through the encoder to flush it,
    /// as recommended by the Opus documentation.
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if encoding fails
    pub fn finalize(&mut self) -> Result<(), OpusError> {
        if !self.sample_buffer.is_empty() {
            // Pad with zeros to make a complete frame
            self.sample_buffer.resize(FRAME_SIZE, 0);

            // Encode the final frame
            let encoded_len = unsafe {
                opus_encode(
                    self.encoder,
                    self.sample_buffer.as_ptr(),
                    FRAME_SIZE as i32,
                    self.packet_buffer.as_mut_ptr(),
                    MAX_PACKET_SIZE as i32,
                )
            };

            if encoded_len < 0 {
                return Err(OpusError::from_code(encoded_len));
            }

            // Store the encoded frame (skip DTX frames)
            if encoded_len > 2 {
                let encoded_frame = self.packet_buffer[..encoded_len as usize].to_vec();
                self.encoded_frames.push(encoded_frame);
            }

            // Clear the sample buffer
            self.sample_buffer.clear();
        }

        // Push two silent frames to flush the encoder (as per Opus best practices)
        let silent_frame = vec![0i16; FRAME_SIZE];
        self.add_samples(&silent_frame)?;
        self.add_samples(&silent_frame)?;

        Ok(())
    }

    /// Get the preskip value from the encoder
    ///
    /// Preskip is the number of samples that should be discarded from the
    /// beginning of the decoded audio. This is necessary for proper synchronization
    /// in formats like WebM/Matroska.
    ///
    /// # Returns
    /// The preskip value (lookahead) in samples, or an error
    pub fn get_preskip(&self) -> Result<i32, OpusError> {
        let mut preskip: i32 = 0;
        let result = unsafe {
            opus_encoder_ctl(
                self.encoder,
                OPUS_GET_LOOKAHEAD_REQUEST as i32,
                &mut preskip as *mut i32,
            )
        };

        if result != 0 {
            return Err(OpusError::from_code(result));
        }

        Ok(preskip)
    }

    /// Get all encoded opus frames
    ///
    /// This returns a vector of byte vectors, where each inner vector
    /// is a complete opus frame ready for transmission or storage.
    ///
    /// Note: This consumes the frames, so calling it multiple times
    /// will only return new frames that were encoded since the last call.
    ///
    /// # Returns
    /// A vector of opus frames (each frame is a Vec<u8>)
    pub fn take_frames(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.encoded_frames)
    }

    /// Get the number of frames currently available
    pub fn frame_count(&self) -> usize {
        self.encoded_frames.len()
    }

    /// Get the number of buffered samples (not yet encoded)
    pub fn buffered_samples(&self) -> usize {
        self.sample_buffer.len()
    }

    /// Set the encoder bitrate
    ///
    /// # Arguments
    /// * `bitrate` - Target bitrate in bits per second
    ///
    /// # Returns
    /// Ok(()) if successful, or an error
    pub fn set_bitrate(&mut self, bitrate: i32) -> Result<(), OpusError> {
        let result = unsafe {
            opus_encoder_ctl(self.encoder, OPUS_SET_BITRATE_REQUEST as i32, bitrate)
        };

        if result != 0 {
            return Err(OpusError::from_code(result));
        }

        Ok(())
    }

    /// Set the encoder complexity (0-10)
    ///
    /// Higher complexity means better quality but slower encoding.
    /// Default is 10.
    ///
    /// # Arguments
    /// * `complexity` - Complexity level (0-10)
    ///
    /// # Returns
    /// Ok(()) if successful, or an error
    pub fn set_complexity(&mut self, complexity: i32) -> Result<(), OpusError> {
        let result = unsafe {
            opus_encoder_ctl(self.encoder, OPUS_SET_COMPLEXITY_REQUEST as i32, complexity)
        };

        if result != 0 {
            return Err(OpusError::from_code(result));
        }

        Ok(())
    }
}

impl Drop for BufferedOpusEncoder {
    fn drop(&mut self) {
        if !self.encoder.is_null() {
            unsafe {
                opus_encoder_destroy(self.encoder);
            }
        }
    }
}

// BufferedOpusEncoder is safe to send between threads
unsafe impl Send for BufferedOpusEncoder {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let encoder = BufferedOpusEncoder::new(64000);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_encode_exact_frame() {
        let mut encoder = BufferedOpusEncoder::new(64000).unwrap();
        let samples = vec![0i16; FRAME_SIZE];

        assert_eq!(encoder.frame_count(), 0);
        encoder.add_samples(&samples).unwrap();
        assert_eq!(encoder.frame_count(), 1);

        let frames = encoder.take_frames();
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn test_add_samples_f32() {
        let mut encoder = BufferedOpusEncoder::new(64000).unwrap();
        let samples = vec![0.5f32; FRAME_SIZE];

        encoder.add_samples_f32(&samples).unwrap();
        assert_eq!(encoder.frame_count(), 1);

        let frames = encoder.take_frames();
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn test_get_preskip() {
        let encoder = BufferedOpusEncoder::new(64000).unwrap();
        let preskip = encoder.get_preskip().unwrap();
        // Preskip should be positive and reasonable (typically 312 for 48kHz)
        assert!(preskip > 0);
        assert!(preskip < 1000);
    }

    #[test]
    fn test_encode_irregular_chunks() {
        let mut encoder = BufferedOpusEncoder::new(64000).unwrap();

        // Add various sized chunks with non-zero values to avoid DTX
        encoder.add_samples(&vec![100i16; 300]).unwrap();
        assert_eq!(encoder.frame_count(), 0); // Not enough for a frame

        encoder.add_samples(&vec![200i16; 700]).unwrap();
        assert_eq!(encoder.frame_count(), 2); // Now we have two frames (1000 samples = 2 frames)

        encoder.add_samples(&vec![300i16; 500]).unwrap();
        assert_eq!(encoder.frame_count(), 3); // Still 3 frames (20 buffered)

        encoder.add_samples(&vec![400i16; 500]).unwrap();
        assert_eq!(encoder.frame_count(), 4); // Now 4 frames (40 buffered)
    }

    #[test]
    fn test_finalize_with_remainder() {
        let mut encoder = BufferedOpusEncoder::new(64000).unwrap();

        // Add samples that don't make a complete frame
        encoder.add_samples(&vec![0i16; 100]).unwrap();
        assert_eq!(encoder.frame_count(), 0);

        // Finalize should pad and encode the partial frame + 2 silent frames
        encoder.finalize().unwrap();
        assert_eq!(encoder.frame_count(), 3); // 1 partial + 2 silent
        assert_eq!(encoder.buffered_samples(), 0);
    }

    #[test]
    fn test_take_frames_clears() {
        let mut encoder = BufferedOpusEncoder::new(64000).unwrap();
        encoder.add_samples(&vec![0i16; FRAME_SIZE * 2]).unwrap();

        assert_eq!(encoder.frame_count(), 2);
        let frames = encoder.take_frames();
        assert_eq!(frames.len(), 2);
        assert_eq!(encoder.frame_count(), 0);
    }
}
