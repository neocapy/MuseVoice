//! EBML (Extensible Binary Meta Language) builder
//!
//! This module provides utilities for constructing EBML data structures,
//! which are used as the container format for WebM/Matroska files.
//!
//! EBML uses variable-length integers and a hierarchical element structure.
//! Elements consist of an ID, a size, and payload data.

use std::io::Write;

/// Builder for constructing EBML binary data
///
/// Provides a fluent API for building EBML structures with method chaining.
///
/// # Example
///
/// ```rust,no_run
/// use muse_lib::ebml::EbmlBuilder;
///
/// let mut ebml = EbmlBuilder::new();
/// ebml.u4(0x1A45DFA3)  // EBML element ID
///     .size(31)         // Size of payload
///     .u2(0x4286)       // EBMLVersion
///     .size(1)
///     .u1(1);
///
/// let data = ebml.build();
/// ```
#[derive(Debug, Clone)]
pub struct EbmlBuilder {
    data: Vec<u8>,
}

impl EbmlBuilder {
    /// Create a new empty EBML builder
    pub fn new() -> Self {
        Self {
            data: Vec::with_capacity(4096),
        }
    }

    /// Create a new EBML builder with specified capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
        }
    }

    /// Write a variable-length integer (vint)
    ///
    /// EBML uses a special variable-length encoding where the first byte
    /// contains a marker bit indicating the total length.
    ///
    /// # Arguments
    /// * `val` - The value to encode
    /// * `disallow_all_ones` - If true, subtract 1 from the maximum value for each width
    ///
    /// # Returns
    /// Self for method chaining
    pub fn vint(&mut self, val: u64, disallow_all_ones: bool) -> &mut Self {
        let shift = if disallow_all_ones { 1 } else { 0 };

        // 1xxx xxxx
        if val < (1 << 7) - shift {
            self.data.push(0x80 | (val as u8 & 0x7f));
        }
        // 01xx xxxx xxxx xxxx
        else if val < (1 << 14) - shift {
            self.data.push(0x40 | ((val >> 8) as u8 & 0x3f));
            self.data.push((val & 0xff) as u8);
        }
        // 001x xxxx xxxx xxxx xxxx xxxx
        else if val < (1 << 21) - shift {
            self.data.push(0x20 | ((val >> 16) as u8 & 0x1f));
            self.data.push(((val >> 8) & 0xff) as u8);
            self.data.push((val & 0xff) as u8);
        }
        // 0001 xxxx xxxx xxxx xxxx xxxx xxxx xxxx
        else if val < (1 << 28) - shift {
            self.data.push(0x10 | ((val >> 24) as u8 & 0x0f));
            self.data.push(((val >> 16) & 0xff) as u8);
            self.data.push(((val >> 8) & 0xff) as u8);
            self.data.push((val & 0xff) as u8);
        }
        // 0000 1xxx xxxx xxxx xxxx xxxx xxxx xxxx xxxx xxxx
        else if val < (1 << 35) - shift {
            self.data.push(0x08 | ((val >> 32) as u8 & 0x07));
            self.data.push(((val >> 24) & 0xff) as u8);
            self.data.push(((val >> 16) & 0xff) as u8);
            self.data.push(((val >> 8) & 0xff) as u8);
            self.data.push((val & 0xff) as u8);
        }
        // 0000 01xx ... (6 bytes total)
        else if val < (1u64 << 42) - shift {
            self.data.push(0x04 | ((val >> 40) as u8 & 0x03));
            self.data.push(((val >> 32) & 0xff) as u8);
            self.data.push(((val >> 24) & 0xff) as u8);
            self.data.push(((val >> 16) & 0xff) as u8);
            self.data.push(((val >> 8) & 0xff) as u8);
            self.data.push((val & 0xff) as u8);
        }
        // 0000 001x ... (7 bytes total)
        else if val < (1u64 << 49) - shift {
            self.data.push(0x02 | ((val >> 48) as u8 & 0x01));
            self.data.push(((val >> 40) & 0xff) as u8);
            self.data.push(((val >> 32) & 0xff) as u8);
            self.data.push(((val >> 24) & 0xff) as u8);
            self.data.push(((val >> 16) & 0xff) as u8);
            self.data.push(((val >> 8) & 0xff) as u8);
            self.data.push((val & 0xff) as u8);
        }
        // 0000 0001 ... (8 bytes total)
        else if val < (1u64 << 56) - shift {
            self.data.push(0x01);
            self.data.push(((val >> 48) & 0xff) as u8);
            self.data.push(((val >> 40) & 0xff) as u8);
            self.data.push(((val >> 32) & 0xff) as u8);
            self.data.push(((val >> 24) & 0xff) as u8);
            self.data.push(((val >> 16) & 0xff) as u8);
            self.data.push(((val >> 8) & 0xff) as u8);
            self.data.push((val & 0xff) as u8);
        } else {
            panic!("EBML vint overflow: value {} is too large", val);
        }

        self
    }

    /// Write a size value (vint with disallow_all_ones = true)
    ///
    /// This is used for element sizes in EBML.
    pub fn size(&mut self, val: u64) -> &mut Self {
        self.vint(val, true)
    }

    /// Write a single unsigned byte
    pub fn u1(&mut self, val: u8) -> &mut Self {
        self.data.push(val);
        self
    }

    /// Write a 2-byte unsigned integer (big-endian)
    pub fn u2(&mut self, val: u16) -> &mut Self {
        self.data.push((val >> 8) as u8);
        self.data.push((val & 0xff) as u8);
        self
    }

    /// Write a 4-byte unsigned integer (big-endian)
    pub fn u4(&mut self, val: u32) -> &mut Self {
        self.data.push((val >> 24) as u8);
        self.data.push(((val >> 16) & 0xff) as u8);
        self.data.push(((val >> 8) & 0xff) as u8);
        self.data.push((val & 0xff) as u8);
        self
    }

    /// Write an 8-byte unsigned integer (big-endian)
    pub fn u8(&mut self, val: u64) -> &mut Self {
        self.data.push((val >> 56) as u8);
        self.data.push(((val >> 48) & 0xff) as u8);
        self.data.push(((val >> 40) & 0xff) as u8);
        self.data.push(((val >> 32) & 0xff) as u8);
        self.data.push(((val >> 24) & 0xff) as u8);
        self.data.push(((val >> 16) & 0xff) as u8);
        self.data.push(((val >> 8) & 0xff) as u8);
        self.data.push((val & 0xff) as u8);
        self
    }

    /// Write a 4-byte float (IEEE 754, big-endian)
    pub fn f4(&mut self, val: f32) -> &mut Self {
        let bits = val.to_bits();
        self.u4(bits)
    }

    /// Write an 8-byte double (IEEE 754, big-endian)
    pub fn f8(&mut self, val: f64) -> &mut Self {
        let bits = val.to_bits();
        self.u8(bits)
    }

    /// Write raw bytes
    pub fn bytes(&mut self, data: &[u8]) -> &mut Self {
        self.data.extend_from_slice(data);
        self
    }

    /// Write an EBML element with a single child payload
    ///
    /// This writes the size of the payload followed by the payload data.
    pub fn payload(&mut self, payload: &EbmlBuilder) -> &mut Self {
        self.size(payload.len() as u64);
        self.bytes(&payload.data);
        self
    }

    /// Write an EBML element with multiple child payloads
    ///
    /// This calculates the total size of all children, writes that size,
    /// then writes all the children's data.
    pub fn payload_multiple(&mut self, children: &[&EbmlBuilder]) -> &mut Self {
        let total_size: usize = children.iter().map(|c| c.len()).sum();
        self.size(total_size as u64);
        for child in children {
            self.bytes(&child.data);
        }
        self
    }

    /// Get the current length of the data
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if the builder is empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Get a reference to the underlying data
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Build and consume the builder, returning the data
    pub fn build(self) -> Vec<u8> {
        self.data
    }

    /// Clear all data from the builder
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Write the data to a file
    pub fn write_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, &self.data)
    }

    /// Write the data to a writer
    pub fn write_to<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(&self.data)
    }
}

impl Default for EbmlBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vint_encoding() {
        // Test 1-byte encoding
        let mut ebml = EbmlBuilder::new();
        ebml.vint(0x7F, false);
        assert_eq!(ebml.as_slice(), &[0xFF]);

        // Test 2-byte encoding
        let mut ebml = EbmlBuilder::new();
        ebml.vint(0x3FFF, false);
        assert_eq!(ebml.as_slice(), &[0x7F, 0xFF]);

        // Test 3-byte encoding
        let mut ebml = EbmlBuilder::new();
        ebml.vint(0x1FFFFF, false);
        assert_eq!(ebml.as_slice(), &[0x3F, 0xFF, 0xFF]);
    }

    #[test]
    fn test_size_encoding() {
        let mut ebml = EbmlBuilder::new();
        ebml.size(126);
        // With disallow_all_ones, max 1-byte is 126
        assert_eq!(ebml.as_slice(), &[0xFE]);

        let mut ebml = EbmlBuilder::new();
        ebml.size(127);
        // 127 exceeds 1-byte max with disallow_all_ones, so uses 2 bytes
        assert_eq!(ebml.as_slice().len(), 2);
        assert_eq!(ebml.as_slice(), &[0x40, 0x7F]);
    }

    #[test]
    fn test_integer_encoding() {
        let mut ebml = EbmlBuilder::new();
        ebml.u1(0x42).u2(0x1234).u4(0xDEADBEEF);

        assert_eq!(
            ebml.as_slice(),
            &[0x42, 0x12, 0x34, 0xDE, 0xAD, 0xBE, 0xEF]
        );
    }

    #[test]
    fn test_float_encoding() {
        let mut ebml = EbmlBuilder::new();
        ebml.f4(1.0f32);
        assert_eq!(ebml.as_slice(), &[0x3F, 0x80, 0x00, 0x00]);

        let mut ebml = EbmlBuilder::new();
        ebml.f8(1.0f64);
        assert_eq!(ebml.as_slice(), &[0x3F, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_bytes() {
        let mut ebml = EbmlBuilder::new();
        ebml.bytes(b"hello");
        assert_eq!(ebml.as_slice(), b"hello");
    }

    #[test]
    fn test_payload() {
        let mut child = EbmlBuilder::new();
        child.u1(0x42).u1(0x43);

        let mut parent = EbmlBuilder::new();
        parent.payload(&child);

        // Should be: size(2) + [0x42, 0x43]
        assert_eq!(parent.as_slice()[0], 0x82); // vint(2) = 0x82
        assert_eq!(&parent.as_slice()[1..], &[0x42, 0x43]);
    }

    #[test]
    fn test_method_chaining() {
        let mut builder = EbmlBuilder::new();
        builder.u1(0x01)
            .u2(0x0203)
            .u4(0x04050607);

        let data = builder.build();
        assert_eq!(data, vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
    }
}