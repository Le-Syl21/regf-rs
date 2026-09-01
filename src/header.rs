use crate::error::{RegError, Result};

/// Taille du base block REGF (et offset de départ des hive bins).
pub const REGF_HEADER_SIZE: usize = 0x1000;

const SIGNATURE: &[u8; 4] = b"regf";
const OFF_PRIMARY_SEQ: usize = 0x04;
const OFF_SECONDARY_SEQ: usize = 0x08;
const OFF_ROOT_CELL: usize = 0x24;
const OFF_HIVE_BINS_SIZE: usize = 0x28;
const OFF_CHECKSUM: usize = 0x1FC;

/// Base block d'une ruche : en-tête de 4096 octets précédant les hive bins.
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

    /// Une ruche est « sale » si ses deux numéros de séquence diffèrent :
    /// elle a été interrompue en cours d'écriture et un transaction log
    /// reste à rejouer. Y écrire sans réconciliation la corromprait.
    pub fn is_dirty(&self) -> bool {
        self.primary_sequence != self.secondary_sequence
    }

    /// Checksum REGF : XOR des 127 premiers mots de 32 bits (octets 0..508).
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

    /// Réécrit dans `data` les champs de l'en-tête susceptibles d'avoir changé
    /// (taille des hive bins, séquences), incrémente le compteur de version
    /// pour marquer une nouvelle transaction cohérente, puis recalcule le
    /// checksum. À appeler après toute modification, avant sérialisation.
    pub fn finalize(&mut self, data: &mut [u8]) {
        let next = self.primary_sequence.wrapping_add(1);
        self.primary_sequence = next;
        self.secondary_sequence = next; // égaux ⇒ ruche propre
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
