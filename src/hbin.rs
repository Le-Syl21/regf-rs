//! Gestion des hive bins et allocation de cellules.
//!
//! Le corps d'une ruche est une suite de **hive bins** (HBIN) de 4096 octets
//! (ou multiple), chacun préfixé d'un en-tête de 32 octets. Un HBIN contient
//! des **cellules** de taille variable : `[i32 taille][charge utile]`, la
//! taille étant négative si la cellule est allouée, positive si elle est libre.
//! Les offsets manipulés dans la ruche (« data offsets ») sont relatifs au
//! début des hive bins, soit le décalage `REGF_HEADER_SIZE` dans le buffer.

use crate::error::{RegError, Result};
use crate::header::REGF_HEADER_SIZE;

const HBIN_MAGIC: &[u8; 4] = b"hbin";
const HBIN_HEADER: usize = 0x20;
pub const CELL_ALIGN: usize = 8;
pub const HBIN_GRANULARITY: usize = 0x1000;

/// Arrondit `n` au multiple supérieur de `align`.
#[inline]
fn align_up(n: usize, align: usize) -> usize {
    n.div_ceil(align) * align
}

/// Convertit un data offset (relatif aux hive bins) en index absolu du buffer.
#[inline]
pub fn abs(data_offset: u32) -> usize {
    REGF_HEADER_SIZE + data_offset as usize
}

/// Convertit un index absolu du buffer en data offset.
#[inline]
pub fn rel(abs_index: usize) -> u32 {
    (abs_index - REGF_HEADER_SIZE) as u32
}

/// Taille brute (signée) du champ de taille d'une cellule à l'index absolu.
fn cell_raw_size(data: &[u8], abs_index: usize) -> Result<i32> {
    if abs_index + 4 > data.len() {
        return Err(RegError::Truncated { offset: abs_index });
    }
    Ok(i32::from_le_bytes(
        data[abs_index..abs_index + 4].try_into().unwrap(),
    ))
}

/// Alloue une cellule pouvant contenir `payload_len` octets de charge utile
/// et renvoie son data offset. Réutilise une cellule libre (premier ajustement,
/// avec scission du reliquat) ou étend la ruche d'un nouveau HBIN.
///
/// Le contenu de la charge utile n'est pas initialisé : l'appelant l'écrit via
/// [`payload_mut`]. Le champ de taille est positionné (négatif = alloué).
pub fn allocate(
    data: &mut alloc::vec::Vec<u8>,
    header_hive_bins_size: &mut u32,
    payload_len: usize,
) -> Result<u32> {
    let need = align_up(4 + payload_len, CELL_ALIGN);

    // 1. Recherche premier-ajustement parmi les cellules libres.
    let mut pos = REGF_HEADER_SIZE;
    let end = REGF_HEADER_SIZE + *header_hive_bins_size as usize;
    while pos + HBIN_HEADER <= end.min(data.len()) {
        if &data[pos..pos + 4] != HBIN_MAGIC {
            break; // plus de HBIN cohérent
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
                break; // cellule incohérente : on abandonne ce HBIN
            }
            if raw > 0 && size >= need {
                // Cellule libre assez grande : on l'occupe, avec scission.
                split_and_occupy(data, cur, size, need);
                return Ok(rel(cur));
            }
            cur += size;
        }
        pos += hbin_size;
    }

    // 2. Aucune place : nouveau HBIN à la fin.
    let new_hbin_size = align_up(HBIN_HEADER + need, HBIN_GRANULARITY);
    let hbin_data_offset = *header_hive_bins_size;
    let base = data.len();
    data.resize(base + new_hbin_size, 0);
    // En-tête HBIN
    data[base..base + 4].copy_from_slice(HBIN_MAGIC);
    data[base + 4..base + 8].copy_from_slice(&hbin_data_offset.to_le_bytes());
    data[base + 8..base + 12].copy_from_slice(&(new_hbin_size as u32).to_le_bytes());
    // Cellule allouée
    let cell = base + HBIN_HEADER;
    write_size(data, cell, -(need as i32));
    // Reliquat libre éventuel
    let leftover = new_hbin_size - HBIN_HEADER - need;
    if leftover >= CELL_ALIGN {
        write_size(data, cell + need, leftover as i32);
    }
    *header_hive_bins_size += new_hbin_size as u32;
    Ok(rel(cell))
}

/// Occupe une cellule libre de taille `size` à `abs_index`, en scindant le
/// reliquat en une nouvelle cellule libre si celui-ci est exploitable.
fn split_and_occupy(data: &mut [u8], abs_index: usize, size: usize, need: usize) {
    let leftover = size - need;
    if leftover >= CELL_ALIGN {
        write_size(data, abs_index, -(need as i32));
        write_size(data, abs_index + need, leftover as i32);
    } else {
        // Reliquat trop petit : on garde la cellule entière (allouée).
        write_size(data, abs_index, -(size as i32));
    }
}

/// Libère la cellule au data offset donné, en fusionnant avec la cellule
/// suivante si elle est libre et appartient au même HBIN (coalescence avant).
pub fn free(data: &mut [u8], data_offset: u32, header_hive_bins_size: u32) -> Result<()> {
    let idx = abs(data_offset);
    let raw = cell_raw_size(data, idx)?;
    let mut size = raw.unsigned_abs() as usize;
    let hbin_end = enclosing_hbin_end(data, idx, header_hive_bins_size);

    // Coalescence avant : tant que la cellule suivante est libre.
    let mut next = idx + size;
    while next + 4 <= hbin_end {
        let nraw = cell_raw_size(data, next)?;
        if nraw <= 0 {
            break; // occupée
        }
        size += nraw as usize;
        next = idx + size;
    }
    write_size(data, idx, size as i32); // positif = libre
    Ok(())
}

/// Fin absolue du HBIN contenant l'index donné.
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

/// Écrit le champ de taille (signé) d'une cellule à l'index absolu.
fn write_size(data: &mut [u8], abs_index: usize, size: i32) {
    data[abs_index..abs_index + 4].copy_from_slice(&size.to_le_bytes());
}

/// Accès mutable à la charge utile d'une cellule allouée (hors champ taille).
pub fn payload_mut(data: &mut [u8], data_offset: u32) -> Result<&mut [u8]> {
    let idx = abs(data_offset);
    let raw = cell_raw_size(data, idx)?;
    let size = raw.unsigned_abs() as usize;
    if size < 4 || idx + size > data.len() {
        return Err(RegError::CorruptCell { offset: idx });
    }
    Ok(&mut data[idx + 4..idx + size])
}
