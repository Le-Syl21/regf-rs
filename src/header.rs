use crate::error::{RegError, Result};

/// REGF base block size (and start offset of the hive bins).
pub const REGF_HEADER_SIZE: usize = 0x1000;

const SIGNATURE: &[u8; 4] = b"regf";
const OFF_PRIMARY_SEQ: usize = 0x04;
const OFF_SECONDARY_SEQ: usize = 0x08;
const OFF_ROOT_CELL: usize = 0x24;
const OFF_HIVE_BINS_SIZE: usize = 0x28;
const OFF_CHECKSUM: usize = 0x1FC;

/// A hive base block: the 4096-byte header preceding the hive bins.
#[derive(Debug, Clone)]
pub struct Header {
    pub primary_sequence: u32,
    pub secondary_sequence: u32,
    pub major_version: u32,
    pub minor_version: u32,
    pub root_cell_offset: u32,
    pub hive_bins_size: u32,
}

impl Header {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < REGF_HEADER_SIZE {
            return Err(RegError::Truncated {
                offset: REGF_HEADER_SIZE,
            });
        }
        if &data[0..4] != SIGNATURE {
            return Err(RegError::BadSignature);
        }
        let expected = Self::checksum(data);
        let found = rd(data, OFF_CHECKSUM);
        if expected != found {
            return Err(RegError::BadChecksum { expected, found });
        }
        Ok(Header {
            primary_sequence: rd(data, OFF_PRIMARY_SEQ),
            secondary_sequence: rd(data, OFF_SECONDARY_SEQ),
            major_version: rd(data, 0x14),
            minor_version: rd(data, 0x18),
            root_cell_offset: rd(data, OFF_ROOT_CELL),
            hive_bins_size: rd(data, OFF_HIVE_BINS_SIZE),
        })
    }

    /// Parses the header without verifying the checksum. Reserved for the
    /// internal construction of a hive being assembled (checksum not yet
    /// computed).
    pub(crate) fn parse_unchecked(data: &[u8]) -> Self {
        Header {
            primary_sequence: rd(data, OFF_PRIMARY_SEQ),
            secondary_sequence: rd(data, OFF_SECONDARY_SEQ),
            major_version: rd(data, 0x14),
            minor_version: rd(data, 0x18),
            root_cell_offset: rd(data, OFF_ROOT_CELL),
            hive_bins_size: rd(data, OFF_HIVE_BINS_SIZE),
        }
    }

    /// A hive is "dirty" when its two sequence numbers differ: it was
    /// interrupted mid-write and a transaction log remains to be replayed.
    /// Writing to it without reconciliation would corrupt it.
    pub fn is_dirty(&self) -> bool {
        self.primary_sequence != self.secondary_sequence
    }

    /// REGF checksum: XOR of the first 127 32-bit words (bytes 0..508).
    pub fn checksum(data: &[u8]) -> u32 {
        let mut sum = 0u32;
        for i in 0..127 {
            sum ^= rd(data, i * 4);
        }
        match sum {
            0 => 1,
            0xFFFF_FFFF => 0xFFFF_FFFE,
            v => v,
        }
    }

    /// Rewrites into `data` the header fields that may have changed (hive bins
    /// size, sequences), bumps the version counter to mark a new consistent
    /// transaction, then recomputes the checksum. Call after any modification,
    /// before serialization.
    pub fn finalize(&mut self, data: &mut [u8]) {
        let next = self.primary_sequence.wrapping_add(1);
        self.primary_sequence = next;
        self.secondary_sequence = next; // equal ⇒ clean hive
        wr(data, OFF_PRIMARY_SEQ, next);
        wr(data, OFF_SECONDARY_SEQ, next);
        wr(data, OFF_HIVE_BINS_SIZE, self.hive_bins_size);
        let sum = Self::checksum(data);
        wr(data, OFF_CHECKSUM, sum);
    }
}

#[inline]
fn rd(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}
#[inline]
fn wr(data: &mut [u8], off: usize, v: u32) {
    data[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
