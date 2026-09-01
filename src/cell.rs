//! Structures REGF : lecture des nœuds de clé (`nk`), des valeurs (`vk`),
//! des listes de sous-clés (`lf`/`lh`/`li`/`ri`) et des données (`db`),
//! ainsi que la construction des charges utiles pour l'écriture.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{RegError, Result};
use crate::hbin::abs;
use crate::value::RegType;

pub const FREE: u32 = 0xFFFF_FFFF;
const FLAG_NK_COMP_NAME: u16 = 0x0020;
const FLAG_VK_COMP_NAME: u16 = 0x0001;
const INLINE_BIT: u32 = 0x8000_0000;
const BIG_DATA_THRESHOLD: usize = 16344;

/// Renvoie la charge utile (hors champ de taille) de la cellule au data offset.
pub fn cell_payload(data: &[u8], data_offset: u32) -> Result<&[u8]> {
    let idx = abs(data_offset);
    if idx + 4 > data.len() {
        return Err(RegError::Truncated { offset: idx });
    }
    let raw = i32::from_le_bytes(data[idx..idx + 4].try_into().unwrap());
    let size = raw.unsigned_abs() as usize;
    if size < 4 || idx + size > data.len() {
        return Err(RegError::CorruptCell { offset: idx });
    }
    Ok(&data[idx + 4..idx + size])
}

// ---------------------------------------------------------------------------
// Nœud de clé (nk)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct KeyNodeRaw {
    pub name: String,
    pub subkey_count: u32,
    pub subkey_list_offset: u32,
    pub value_count: u32,
    pub value_list_offset: u32,
    pub security_offset: u32,
}

pub fn read_key_node(data: &[u8], data_offset: u32) -> Result<KeyNodeRaw> {
    let p = cell_payload(data, data_offset)?;
    if p.len() < 76 || &p[0..2] != b"nk" {
        return Err(RegError::CorruptCell {
            offset: data_offset as usize,
        });
    }
    let flags = u16::from_le_bytes(p[2..4].try_into().unwrap());
    let name_len = u16::from_le_bytes(p[72..74].try_into().unwrap()) as usize;
    let name_bytes = p.get(76..76 + name_len).ok_or(RegError::CorruptCell {
        offset: data_offset as usize,
    })?;
    Ok(KeyNodeRaw {
        name: decode_name(name_bytes, flags & FLAG_NK_COMP_NAME != 0),
        subkey_count: rd(p, 20),
        subkey_list_offset: rd(p, 28),
        value_count: rd(p, 36),
        value_list_offset: rd(p, 40),
        security_offset: rd(p, 44),
    })
}

/// Construit la charge utile d'un nouveau `nk` feuille (sans sous-clé ni valeur).
pub fn build_key_node(name: &str, parent: u32, security: u32) -> Vec<u8> {
    let ascii = name.is_ascii();
    let name_bytes = encode_name(name, ascii);
    let mut p = alloc::vec![0u8; 76 + name_bytes.len()];
    p[0..2].copy_from_slice(b"nk");
    let flags: u16 = if ascii { FLAG_NK_COMP_NAME } else { 0 };
    p[2..4].copy_from_slice(&flags.to_le_bytes());
    wr(&mut p, 16, parent);
    wr(&mut p, 20, 0); // subkey count
    wr(&mut p, 28, FREE); // subkey list
    wr(&mut p, 32, FREE); // volatile subkey list
    wr(&mut p, 36, 0); // value count
    wr(&mut p, 40, FREE); // value list
    wr(&mut p, 44, security);
    wr(&mut p, 48, FREE); // class name
    p[72..74].copy_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    p[76..].copy_from_slice(&name_bytes);
    p
}

/// Modifie en place un champ u32 d'un nk (via son data offset).
pub fn set_nk_field(data: &mut [u8], nk_offset: u32, field_off: usize, value: u32) -> Result<()> {
    let p = crate::hbin::payload_mut(data, nk_offset)?;
    if field_off + 4 > p.len() {
        return Err(RegError::CorruptCell {
            offset: nk_offset as usize,
        });
    }
    p[field_off..field_off + 4].copy_from_slice(&value.to_le_bytes());
    Ok(())
}
pub const NK_SUBKEY_COUNT: usize = 20;
pub const NK_SUBKEY_LIST: usize = 28;
pub const NK_VALUE_COUNT: usize = 36;
pub const NK_VALUE_LIST: usize = 40;

// ---------------------------------------------------------------------------
// Listes de sous-clés (lf / lh / li / ri)
// ---------------------------------------------------------------------------

/// Format d'une liste feuille, préservé lors des réécritures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafKind {
    Lf,
    Lh,
    Li,
}

/// Renvoie les offsets de nk d'une liste (résout `ri` récursivement).
pub fn subkey_offsets(data: &[u8], list_offset: u32) -> Result<Vec<u32>> {
    if list_offset == 0 || list_offset == FREE {
        return Ok(Vec::new());
    }
    let p = cell_payload(data, list_offset)?;
    if p.len() < 4 {
        return Err(RegError::CorruptCell {
            offset: list_offset as usize,
        });
    }
    let magic = &p[0..2];
    let count = u16::from_le_bytes(p[2..4].try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(count);
    match magic {
        b"lf" | b"lh" => {
            for i in 0..count {
                let o = 4 + i * 8;
                if o + 4 <= p.len() {
                    out.push(rd(p, o));
                }
            }
        }
        b"li" => {
            for i in 0..count {
                let o = 4 + i * 4;
                if o + 4 <= p.len() {
                    out.push(rd(p, o));
                }
            }
        }
        b"ri" => {
            for i in 0..count {
                let o = 4 + i * 4;
                if o + 4 <= p.len() {
                    out.extend(subkey_offsets(data, rd(p, o))?);
                }
            }
        }
        _ => {
            return Err(RegError::CorruptCell {
                offset: list_offset as usize,
            })
        }
    }
    Ok(out)
}

/// Détecte le format feuille d'une liste (pour préserver `lf`/`lh`/`li`).
/// Une liste `ri` est aplatie en `lf` par convention (voir `build_leaf_list`).
pub fn leaf_kind(data: &[u8], list_offset: u32) -> Result<LeafKind> {
    if list_offset == 0 || list_offset == FREE {
        return Ok(LeafKind::Lf);
    }
    let p = cell_payload(data, list_offset)?;
    match &p[0..2.min(p.len())] {
        b"lh" => Ok(LeafKind::Lh),
        b"li" => Ok(LeafKind::Li),
        _ => Ok(LeafKind::Lf),
    }
}

/// Construit une liste feuille triée à partir d'entrées `(nom, nk_offset)`.
/// Les entrées doivent déjà être triées selon [`crate::name::cmp_str`].
pub fn build_leaf_list(kind: LeafKind, entries: &[(String, u32)]) -> Vec<u8> {
    let magic: &[u8; 2] = match kind {
        LeafKind::Lf => b"lf",
        LeafKind::Lh => b"lh",
        LeafKind::Li => b"li",
    };
    let entry_size = if kind == LeafKind::Li { 4 } else { 8 };
    let mut p = alloc::vec![0u8; 4 + entries.len() * entry_size];
    p[0..2].copy_from_slice(magic);
    p[2..4].copy_from_slice(&(entries.len() as u16).to_le_bytes());
    for (i, (name, off)) in entries.iter().enumerate() {
        let base = 4 + i * entry_size;
        wr(&mut p, base, *off);
        match kind {
            LeafKind::Lf => {
                // Indice : 4 premiers octets ASCII du nom.
                let hint = name.as_bytes();
                for j in 0..4 {
                    p[base + 4 + j] = *hint.get(j).unwrap_or(&0);
                }
            }
            LeafKind::Lh => {
                wr(&mut p, base + 4, crate::name::lh_hash(name));
            }
            LeafKind::Li => {}
        }
    }
    p
}

// ---------------------------------------------------------------------------
// Valeurs (vk) et données (inline / db)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ValueNodeRaw {
    pub name: String,
    pub ty: RegType,
    pub data_size: u32,
    pub data_offset: u32,
    pub inline: bool,
}

pub fn read_value_node(data: &[u8], vk_offset: u32) -> Result<ValueNodeRaw> {
    let p = cell_payload(data, vk_offset)?;
    if p.len() < 20 || &p[0..2] != b"vk" {
        return Err(RegError::CorruptCell {
            offset: vk_offset as usize,
        });
    }
    let name_len = u16::from_le_bytes(p[2..4].try_into().unwrap()) as usize;
    let raw_size = rd(p, 4);
    let data_field = rd(p, 8);
    let ty = RegType::from_u32(rd(p, 12));
    let flags = u16::from_le_bytes(p[16..18].try_into().unwrap());
    let name = if name_len == 0 {
        String::new()
    } else {
        let nb = p.get(20..20 + name_len).ok_or(RegError::CorruptCell {
            offset: vk_offset as usize,
        })?;
        decode_name(nb, flags & FLAG_VK_COMP_NAME != 0)
    };
    Ok(ValueNodeRaw {
        name,
        ty,
        data_size: raw_size & !INLINE_BIT,
        data_offset: data_field,
        inline: raw_size & INLINE_BIT != 0,
    })
}

/// Lit tous les vk d'une liste de valeurs.
pub fn value_offsets(data: &[u8], list_offset: u32, count: u32) -> Result<Vec<u32>> {
    if count == 0 || list_offset == 0 || list_offset == FREE {
        return Ok(Vec::new());
    }
    let p = cell_payload(data, list_offset)?;
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let o = i * 4;
        if o + 4 <= p.len() {
            out.push(rd(p, o));
        }
    }
    Ok(out)
}

/// Reconstitue les octets bruts d'une valeur (gère inline et big-data `db`).
pub fn read_value_data(data: &[u8], vk: &ValueNodeRaw) -> Result<Vec<u8>> {
    let size = vk.data_size as usize;
    if vk.inline {
        return Ok(vk.data_offset.to_le_bytes()[..size.min(4)].to_vec());
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    let cell = cell_payload(data, vk.data_offset)?;
    if size > BIG_DATA_THRESHOLD && cell.len() >= 4 && &cell[0..2] == b"db" {
        // Big data : "db", nb segments, offset de la liste des segments.
        let segments = u16::from_le_bytes(cell[2..4].try_into().unwrap()) as usize;
        let list_off = rd(cell, 4);
        let list = cell_payload(data, list_off)?;
        let mut out = Vec::with_capacity(size);
        for i in 0..segments {
            let seg_off = rd(list, i * 4);
            let seg = cell_payload(data, seg_off)?;
            let take = (size - out.len()).min(seg.len());
            out.extend_from_slice(&seg[..take]);
        }
        out.truncate(size);
        Ok(out)
    } else {
        Ok(cell[..size.min(cell.len())].to_vec())
    }
}

/// Construit la charge utile d'un `vk`. `data_field` vaut soit un data offset
/// (données en cellule), soit les octets inline ; `inline`/`size` distinguent.
pub fn build_value_node(
    name: &str,
    ty: RegType,
    size: u32,
    data_field: u32,
    inline: bool,
) -> Vec<u8> {
    let ascii = name.is_ascii();
    let name_bytes = encode_name(name, ascii);
    let mut p = alloc::vec![0u8; 20 + name_bytes.len()];
    p[0..2].copy_from_slice(b"vk");
    p[2..4].copy_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    let raw_size = if inline { size | INLINE_BIT } else { size };
    wr(&mut p, 4, raw_size);
    wr(&mut p, 8, data_field);
    wr(&mut p, 12, ty.to_u32());
    let flags: u16 = if ascii && !name.is_empty() {
        FLAG_VK_COMP_NAME
    } else {
        0
    };
    p[16..18].copy_from_slice(&flags.to_le_bytes());
    if !name_bytes.is_empty() {
        p[20..].copy_from_slice(&name_bytes);
    }
    p
}

pub const BIG_DATA_LIMIT: usize = BIG_DATA_THRESHOLD;

// ---------------------------------------------------------------------------
// Sécurité (sk) : incrément de compteur de références lors du partage
// ---------------------------------------------------------------------------

/// Incrémente le compteur de références d'une cellule `sk` partagée.
pub fn incr_security_refcount(data: &mut [u8], sk_offset: u32) -> Result<()> {
    let p = crate::hbin::payload_mut(data, sk_offset)?;
    if p.len() < 16 || &p[0..2] != b"sk" {
        return Err(RegError::CorruptCell {
            offset: sk_offset as usize,
        });
    }
    let rc = u32::from_le_bytes(p[12..16].try_into().unwrap()).wrapping_add(1);
    p[12..16].copy_from_slice(&rc.to_le_bytes());
    Ok(())
}

// ---------------------------------------------------------------------------
// Utilitaires noms / lecture
// ---------------------------------------------------------------------------

fn decode_name(bytes: &[u8], ascii: bool) -> String {
    if ascii {
        bytes.iter().map(|&b| b as char).collect()
    } else {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        char::decode_utf16(units)
            .map(|r| r.unwrap_or('\u{FFFD}'))
            .collect()
    }
}

fn encode_name(name: &str, ascii: bool) -> Vec<u8> {
    if ascii {
        name.bytes().collect()
    } else {
        let mut out = Vec::with_capacity(name.len() * 2);
        for u in name.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out
    }
}

#[inline]
fn rd(p: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(p[off..off + 4].try_into().unwrap())
}
#[inline]
fn wr(p: &mut [u8], off: usize, v: u32) {
    p[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
