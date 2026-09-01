//! Hive-bin management and cell allocation.
//!
//! A hive body is a sequence of **hive bins** (HBIN) of 4096 bytes (or a
//! multiple), each prefixed with a 32-byte header. An HBIN holds
//! variable-sized **cells**: `[i32 size][payload]`, the size being negative
//! when the cell is allocated and positive when it is free. The offsets used
//! within a hive ("data offsets") are relative to the start of the hive bins,
//! i.e. the `REGF_HEADER_SIZE` offset within the buffer.

use crate::error::{RegError, Result};
use crate::header::REGF_HEADER_SIZE;

const HBIN_MAGIC: &[u8; 4] = b"hbin";
const HBIN_HEADER: usize = 0x20;
pub const CELL_ALIGN: usize = 8;
pub const HBIN_GRANULARITY: usize = 0x1000;

/// Rounds `n` up to the next multiple of `align`.
#[inline]
fn align_up(n: usize, align: usize) -> usize {
    n.div_ceil(align) * align
}

/// Converts a data offset (relative to the hive bins) into an absolute buffer
/// index.
#[inline]
pub fn abs(data_offset: u32) -> usize {
    REGF_HEADER_SIZE + data_offset as usize
}

/// Converts an absolute buffer index into a data offset.
#[inline]
pub fn rel(abs_index: usize) -> u32 {
    (abs_index - REGF_HEADER_SIZE) as u32
}

/// Raw (signed) value of a cell's size field at the absolute index.
fn cell_raw_size(data: &[u8], abs_index: usize) -> Result<i32> {
    if abs_index + 4 > data.len() {
        return Err(RegError::Truncated { offset: abs_index });
    }
    Ok(i32::from_le_bytes(
        data[abs_index..abs_index + 4].try_into().unwrap(),
    ))
}

/// Allocates a cell able to hold `payload_len` payload bytes and returns its
/// data offset. Reuses a free cell (first fit, splitting the remainder) or
/// grows the hive by a new HBIN.
///
/// The payload contents are left uninitialized: the caller writes them via
/// [`payload_mut`]. The size field is set (negative = allocated).
pub fn allocate(
    data: &mut alloc::vec::Vec<u8>,
    header_hive_bins_size: &mut u32,
    payload_len: usize,
) -> Result<u32> {
    let need = align_up(4 + payload_len, CELL_ALIGN);

    // 1. First-fit search among the free cells.
    let mut pos = REGF_HEADER_SIZE;
    let end = REGF_HEADER_SIZE + *header_hive_bins_size as usize;
    while pos + HBIN_HEADER <= end.min(data.len()) {
        if &data[pos..pos + 4] != HBIN_MAGIC {
            break; // no more coherent HBIN
        }
        let hbin_size = u32::from_le_bytes(data[pos + 8..pos + 12].try_into().unwrap()) as usize;
        if hbin_size == 0 {
            break;
        }
        let hbin_end = (pos + hbin_size).min(data.len());
        let mut cur = pos + HBIN_HEADER;
        while cur + 4 <= hbin_end {
            let raw = cell_raw_size(data, cur)?;
            let size = raw.unsigned_abs() as usize;
            if size < 4 || cur + size > hbin_end {
                break; // inconsistent cell: give up on this HBIN
            }
            if raw > 0 && size >= need {
                // Free cell large enough: occupy it, splitting the remainder.
                split_and_occupy(data, cur, size, need);
                return Ok(rel(cur));
            }
            cur += size;
        }
        pos += hbin_size;
    }

    // 2. No room: append a new HBIN at the end.
    let new_hbin_size = align_up(HBIN_HEADER + need, HBIN_GRANULARITY);
    let hbin_data_offset = *header_hive_bins_size;
    let base = data.len();
    data.resize(base + new_hbin_size, 0);
    // HBIN header
    data[base..base + 4].copy_from_slice(HBIN_MAGIC);
    data[base + 4..base + 8].copy_from_slice(&hbin_data_offset.to_le_bytes());
    data[base + 8..base + 12].copy_from_slice(&(new_hbin_size as u32).to_le_bytes());
    // Allocated cell
    let cell = base + HBIN_HEADER;
    write_size(data, cell, -(need as i32));
    // Optional free remainder
    let leftover = new_hbin_size - HBIN_HEADER - need;
    if leftover >= CELL_ALIGN {
        write_size(data, cell + need, leftover as i32);
    }
    *header_hive_bins_size += new_hbin_size as u32;
    Ok(rel(cell))
}

/// Occupies a free cell of size `size` at `abs_index`, splitting the remainder
/// into a new free cell when it is usable.
fn split_and_occupy(data: &mut [u8], abs_index: usize, size: usize, need: usize) {
    let leftover = size - need;
    if leftover >= CELL_ALIGN {
        write_size(data, abs_index, -(need as i32));
        write_size(data, abs_index + need, leftover as i32);
    } else {
        // Remainder too small: keep the whole cell (allocated).
        write_size(data, abs_index, -(size as i32));
    }
}

/// Frees the cell at the given data offset, merging with the following cell
/// when it is free and belongs to the same HBIN (forward coalescing).
pub fn free(data: &mut [u8], data_offset: u32, header_hive_bins_size: u32) -> Result<()> {
    let idx = abs(data_offset);
    let raw = cell_raw_size(data, idx)?;
    let mut size = raw.unsigned_abs() as usize;
    let hbin_end = enclosing_hbin_end(data, idx, header_hive_bins_size);

    // Forward coalescing: while the following cell is free.
    let mut next = idx + size;
    while next + 4 <= hbin_end {
        let nraw = cell_raw_size(data, next)?;
        if nraw <= 0 {
            break; // occupied
        }
        size += nraw as usize;
        next = idx + size;
    }
    write_size(data, idx, size as i32); // positive = free
    Ok(())
}

/// Absolute end of the HBIN containing the given index.
fn enclosing_hbin_end(data: &[u8], idx: usize, header_hive_bins_size: u32) -> usize {
    let mut pos = REGF_HEADER_SIZE;
    let end = REGF_HEADER_SIZE + header_hive_bins_size as usize;
    while pos + HBIN_HEADER <= end.min(data.len()) {
        if &data[pos..pos + 4] != HBIN_MAGIC {
            break;
        }
        let hbin_size = u32::from_le_bytes(data[pos + 8..pos + 12].try_into().unwrap()) as usize;
        if hbin_size == 0 {
            break;
        }
        if idx >= pos && idx < pos + hbin_size {
            return (pos + hbin_size).min(data.len());
        }
        pos += hbin_size;
    }
    data.len()
}

/// Writes a cell's (signed) size field at the absolute index.
fn write_size(data: &mut [u8], abs_index: usize, size: i32) {
    data[abs_index..abs_index + 4].copy_from_slice(&size.to_le_bytes());
}

/// Mutable access to an allocated cell's payload (excluding the size field).
pub fn payload_mut(data: &mut [u8], data_offset: u32) -> Result<&mut [u8]> {
    let idx = abs(data_offset);
    let raw = cell_raw_size(data, idx)?;
    let size = raw.unsigned_abs() as usize;
    if size < 4 || idx + size > data.len() {
        return Err(RegError::CorruptCell { offset: idx });
    }
    Ok(&mut data[idx + 4..idx + size])
}
