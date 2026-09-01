//! Typing and (de)serialization of registry values (`REG_*`).

use alloc::string::String;
use alloc::vec::Vec;

/// A registry value's type (the `type` field of a `vk` cell).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum RegType {
    None = 0,
    Sz = 1,
    ExpandSz = 2,
    Binary = 3,
    Dword = 4,
    DwordBigEndian = 5,
    Link = 6,
    MultiSz = 7,
    Qword = 11,
    /// Unlisted type: preserved as-is.
    Other(u32),
}

impl RegType {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => RegType::None,
            1 => RegType::Sz,
            2 => RegType::ExpandSz,
            3 => RegType::Binary,
            4 => RegType::Dword,
            5 => RegType::DwordBigEndian,
            6 => RegType::Link,
            7 => RegType::MultiSz,
            11 => RegType::Qword,
            other => RegType::Other(other),
        }
    }
    pub fn to_u32(self) -> u32 {
        match self {
            RegType::None => 0,
            RegType::Sz => 1,
            RegType::ExpandSz => 2,
            RegType::Binary => 3,
            RegType::Dword => 4,
            RegType::DwordBigEndian => 5,
            RegType::Link => 6,
            RegType::MultiSz => 7,
            RegType::Qword => 11,
            RegType::Other(v) => v,
        }
    }
}

/// A decoded registry value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegValue {
    None,
    Sz(String),
    ExpandSz(String),
    Binary(Vec<u8>),
    Dword(u32),
    DwordBigEndian(u32),
    MultiSz(Vec<String>),
    Qword(u64),
    /// Non-standard type: raw bytes + type code.
    Other {
        ty: u32,
        data: Vec<u8>,
    },
}

impl RegValue {
    /// Corresponding REGF type.
    pub fn reg_type(&self) -> RegType {
        match self {
            RegValue::None => RegType::None,
            RegValue::Sz(_) => RegType::Sz,
            RegValue::ExpandSz(_) => RegType::ExpandSz,
            RegValue::Binary(_) => RegType::Binary,
            RegValue::Dword(_) => RegType::Dword,
            RegValue::DwordBigEndian(_) => RegType::DwordBigEndian,
            RegValue::MultiSz(_) => RegType::MultiSz,
            RegValue::Qword(_) => RegType::Qword,
            RegValue::Other { ty, .. } => RegType::from_u32(*ty),
        }
    }

    /// Encodes the value to raw bytes (a data cell's content).
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            RegValue::None => Vec::new(),
            RegValue::Sz(s) | RegValue::ExpandSz(s) => encode_utf16z(s),
            RegValue::Binary(b) => b.clone(),
            RegValue::Dword(v) => v.to_le_bytes().to_vec(),
            RegValue::DwordBigEndian(v) => v.to_be_bytes().to_vec(),
            RegValue::Qword(v) => v.to_le_bytes().to_vec(),
            RegValue::MultiSz(items) => {
                let mut out = Vec::new();
                for s in items {
                    out.extend(encode_utf16z(s));
                }
                out.extend_from_slice(&[0, 0]); // final terminator
                out
            }
            RegValue::Other { data, .. } => data.clone(),
        }
    }

    /// Decodes a value from its type and raw bytes.
    pub fn from_raw(ty: RegType, data: &[u8]) -> Self {
        match ty {
            RegType::None => RegValue::None,
            RegType::Sz => RegValue::Sz(decode_utf16z(data)),
            RegType::ExpandSz => RegValue::ExpandSz(decode_utf16z(data)),
            RegType::Binary => RegValue::Binary(data.to_vec()),
            RegType::Dword => RegValue::Dword(read_u32_le(data)),
            RegType::DwordBigEndian => {
                let mut b = [0u8; 4];
                b.copy_from_slice(&data[..4.min(data.len())]);
                RegValue::DwordBigEndian(u32::from_be_bytes(b))
            }
            RegType::Link => RegValue::Sz(decode_utf16z(data)),
            RegType::MultiSz => RegValue::MultiSz(decode_multi_sz(data)),
            RegType::Qword => {
                let mut b = [0u8; 8];
                b[..data.len().min(8)].copy_from_slice(&data[..data.len().min(8)]);
                RegValue::Qword(u64::from_le_bytes(b))
            }
            RegType::Other(v) => RegValue::Other {
                ty: v,
                data: data.to_vec(),
            },
        }
    }
}

fn encode_utf16z(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2 + 2);
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]); // trailing NUL
    out
}

fn decode_utf16z(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn decode_multi_sz(data: &[u8]) -> Vec<String> {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for u in units {
        if u == 0 {
            if cur.is_empty() {
                break; // double NUL = end
            }
            out.push(String::from_utf16_lossy(&cur));
            cur.clear();
        } else {
            cur.push(u);
        }
    }
    out
}

fn read_u32_le(data: &[u8]) -> u32 {
    let mut b = [0u8; 4];
    b[..data.len().min(4)].copy_from_slice(&data[..data.len().min(4)]);
    u32::from_le_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn roundtrip(v: RegValue) {
        let ty = v.reg_type();
        let bytes = v.to_bytes();
        let back = RegValue::from_raw(ty, &bytes);
        assert_eq!(v, back, "round-trip failed for {v:?}");
    }

    #[test]
    fn roundtrip_all_types() {
        roundtrip(RegValue::Sz("Hello".to_string()));
        roundtrip(RegValue::ExpandSz("%PATH%".to_string()));
        roundtrip(RegValue::Dword(0xDEAD_BEEF));
        roundtrip(RegValue::Qword(0x0123_4567_89AB_CDEF));
        roundtrip(RegValue::Binary(vec![1, 2, 3, 4, 5]));
        roundtrip(RegValue::MultiSz(vec!["a".to_string(), "bb".to_string()]));
        roundtrip(RegValue::MultiSz(vec![]));
        roundtrip(RegValue::None);
    }

    #[test]
    fn sz_is_null_terminated_utf16() {
        let b = RegValue::Sz("Hi".to_string()).to_bytes();
        assert_eq!(b, vec![b'H', 0, b'i', 0, 0, 0]);
    }
}
