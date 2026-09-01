//! High-level API: loading, navigation, reading and in-place writing.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::cell::{self, LeafKind};
use crate::error::{RegError, Result};
use crate::hbin;
use crate::header::Header;
use crate::name;
use crate::value::RegValue;

/// A REGF hive loaded in memory.
#[derive(Clone)]
pub struct Hive {
    data: Vec<u8>,
    header: Header,
}

/// Public view of a key node.
#[derive(Debug, Clone)]
pub struct KeyNode {
    pub name: String,
    pub subkey_count: u32,
    pub value_count: u32,
    pub(crate) offset: u32,
}

impl KeyNode {
    /// This node's data offset within the hive (relative to the hive bins).
    pub fn offset(&self) -> u32 {
        self.offset
    }
}

impl Hive {
    // -- Loading ------------------------------------------------------------

    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let header = Header::parse(&data)?;
        Ok(Hive { data, header })
    }

    /// Creates a valid empty hive: base block, one hive bin, a root node named
    /// `root_name` and a minimal shareable security descriptor. Useful to build
    /// test or fresh hives without a source file.
    pub fn new_empty(root_name: &str) -> Self {
        const HBIN: usize = 0x1000;
        let mut data = alloc::vec![0u8; crate::header::REGF_HEADER_SIZE + HBIN];

        // Base block.
        data[0..4].copy_from_slice(b"regf");
        wr(&mut data, 0x04, 1); // primary sequence
        wr(&mut data, 0x08, 1); // secondary sequence
        wr(&mut data, 0x14, 1); // major version
        wr(&mut data, 0x18, 5); // minor version
        wr(&mut data, 0x1C, 0); // file type (primary)
        wr(&mut data, 0x20, 1); // file format (direct memory load)
        wr(&mut data, 0x28, HBIN as u32); // hive bins size
        wr(&mut data, 0x2C, 1); // clustering factor

        // First hive bin, then one large free cell.
        let hb = crate::header::REGF_HEADER_SIZE;
        data[hb..hb + 4].copy_from_slice(b"hbin");
        wr(&mut data, hb + 4, 0); // bin offset
        wr(&mut data, hb + 8, HBIN as u32); // bin size
        let free_cell = hb + 0x20;
        let free_size = (HBIN - 0x20) as i32;
        data[free_cell..free_cell + 4].copy_from_slice(&free_size.to_le_bytes());

        let mut header = Header::parse_unchecked(&data);
        let mut hive = Hive {
            data,
            header: header.clone(),
        };

        // Minimal security descriptor (self-relative, no DACL).
        let sk = build_min_security();
        let sk_off = hive.alloc_write(&sk).expect("alloc sk");
        // Self-referential flink/blink + refcount = 1.
        {
            let p = hbin::payload_mut(&mut hive.data, sk_off).unwrap();
            p[4..8].copy_from_slice(&sk_off.to_le_bytes());
            p[8..12].copy_from_slice(&sk_off.to_le_bytes());
            p[12..16].copy_from_slice(&1u32.to_le_bytes());
        }

        // Root node.
        let nk = cell::build_key_node(root_name, cell::FREE, sk_off);
        let nk_off = hive.alloc_write(&nk).expect("alloc nk");
        // Set the root flags (hive entry | no delete | comp name).
        {
            let p = hbin::payload_mut(&mut hive.data, nk_off).unwrap();
            p[2..4].copy_from_slice(&0x2Cu16.to_le_bytes());
        }

        // root_cell_offset in the header + resync of the Header struct.
        wr(&mut hive.data, 0x24, nk_off);
        header = Header::parse_unchecked(&hive.data);
        hive.header = header;
        hive
    }

    #[cfg(feature = "std")]
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Self> {
        let data = std::fs::read(path)?;
        Self::from_bytes(data).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// See [`Header::is_dirty`].
    pub fn is_dirty(&self) -> bool {
        self.header.is_dirty()
    }

    // -- Navigation ---------------------------------------------------------

    pub fn root_key(&self) -> Result<KeyNode> {
        self.node_at(self.header.root_cell_offset)
    }

    /// Opens a key by path (separator `\`), case-insensitive.
    pub fn open(&self, path: &str) -> Result<KeyNode> {
        self.node_at(self.resolve(path)?)
    }

    pub fn list_subkeys(&self, path: &str) -> Result<Vec<String>> {
        let off = self.resolve(path)?;
        self.child_entries(off)?
            .into_iter()
            .map(|(n, _)| Ok(n))
            .collect()
    }

    pub fn list_values(&self, path: &str) -> Result<Vec<(String, RegValue)>> {
        let off = self.resolve(path)?;
        let nk = cell::read_key_node(&self.data, off)?;
        let mut out = Vec::new();
        for vk_off in cell::value_offsets(&self.data, nk.value_list_offset, nk.value_count)? {
            let vk = cell::read_value_node(&self.data, vk_off)?;
            let raw = cell::read_value_data(&self.data, &vk)?;
            out.push((vk.name, RegValue::from_raw(vk.ty, &raw)));
        }
        Ok(out)
    }

    pub fn get_value(&self, path: &str, value_name: &str) -> Result<RegValue> {
        let off = self.resolve(path)?;
        let nk = cell::read_key_node(&self.data, off)?;
        for vk_off in cell::value_offsets(&self.data, nk.value_list_offset, nk.value_count)? {
            let vk = cell::read_value_node(&self.data, vk_off)?;
            if name::eq_name(&vk.name, value_name) {
                let raw = cell::read_value_data(&self.data, &vk)?;
                return Ok(RegValue::from_raw(vk.ty, &raw));
            }
        }
        Err(RegError::ValueNotFound(value_name.to_string()))
    }

    // -- Writing ------------------------------------------------------------

    /// Creates the missing keys along the path and returns the final key's
    /// offset. Each created level inherits its parent's security descriptor and
    /// is inserted in sorted position within the subkey list.
    pub fn create_key(&mut self, path: &str) -> Result<KeyNode> {
        self.guard_writable()?;
        let mut current = self.header.root_cell_offset;
        for part in path.split('\\').filter(|s| !s.is_empty()) {
            current = match self.find_child(current, part)? {
                Some(off) => off,
                None => self.insert_subkey(current, part)?,
            };
        }
        self.node_at(current)
    }

    /// Sets (creates or replaces) a value under an existing key.
    pub fn set_value(&mut self, key_path: &str, value_name: &str, value: RegValue) -> Result<()> {
        self.guard_writable()?;
        let nk_off = self.resolve(key_path)?;

        // Encode the data (owned: no borrow crosses an allocation).
        let bytes = value.to_bytes();
        if bytes.len() > cell::BIG_DATA_LIMIT {
            return Err(RegError::ValueTooLarge {
                size: bytes.len(),
                max: cell::BIG_DATA_LIMIT,
            });
        }
        let ty = value.reg_type();
        let (size, data_field, inline) = if bytes.len() <= 4 {
            let mut buf = [0u8; 4];
            buf[..bytes.len()].copy_from_slice(&bytes);
            (bytes.len() as u32, u32::from_le_bytes(buf), true)
        } else {
            let off = self.alloc_write(&bytes)?;
            (bytes.len() as u32, off, false)
        };

        let vk_payload = cell::build_value_node(value_name, ty, size, data_field, inline);
        let new_vk = self.alloc_write(&vk_payload)?;

        // Existing value list.
        let nk = cell::read_key_node(&self.data, nk_off)?;
        let existing = cell::value_offsets(&self.data, nk.value_list_offset, nk.value_count)?;

        // Replace if the name already exists.
        for (i, &vk_off) in existing.iter().enumerate() {
            let vk = cell::read_value_node(&self.data, vk_off)?;
            if name::eq_name(&vk.name, value_name) {
                // Replace the offset in the list (size unchanged).
                let list_payload = hbin::payload_mut(&mut self.data, nk.value_list_offset)?;
                list_payload[i * 4..i * 4 + 4].copy_from_slice(&new_vk.to_le_bytes());
                self.free_value_storage(&vk)?;
                hbin::free(&mut self.data, vk_off, self.header.hive_bins_size)?;
                return Ok(());
            }
        }

        // Append: new value list = old ones + the new one.
        let mut offs = existing;
        offs.push(new_vk);
        let new_list = self.write_value_list(&offs)?;
        if nk.value_list_offset != cell::FREE && nk.value_list_offset != 0 {
            hbin::free(
                &mut self.data,
                nk.value_list_offset,
                self.header.hive_bins_size,
            )?;
        }
        cell::set_nk_field(&mut self.data, nk_off, cell::NK_VALUE_LIST, new_list)?;
        cell::set_nk_field(
            &mut self.data,
            nk_off,
            cell::NK_VALUE_COUNT,
            offs.len() as u32,
        )?;
        Ok(())
    }

    /// Deletes a value. Errors if it does not exist.
    pub fn delete_value(&mut self, key_path: &str, value_name: &str) -> Result<()> {
        self.guard_writable()?;
        let nk_off = self.resolve(key_path)?;
        let nk = cell::read_key_node(&self.data, nk_off)?;
        let existing = cell::value_offsets(&self.data, nk.value_list_offset, nk.value_count)?;

        let mut kept = Vec::with_capacity(existing.len());
        let mut victim = None;
        for &vk_off in &existing {
            let vk = cell::read_value_node(&self.data, vk_off)?;
            if name::eq_name(&vk.name, value_name) {
                victim = Some((vk_off, vk));
            } else {
                kept.push(vk_off);
            }
        }
        let (vk_off, vk) = victim.ok_or_else(|| RegError::ValueNotFound(value_name.to_string()))?;

        let new_list = if kept.is_empty() {
            cell::FREE
        } else {
            self.write_value_list(&kept)?
        };
        if nk.value_list_offset != cell::FREE && nk.value_list_offset != 0 {
            hbin::free(
                &mut self.data,
                nk.value_list_offset,
                self.header.hive_bins_size,
            )?;
        }
        self.free_value_storage(&vk)?;
        hbin::free(&mut self.data, vk_off, self.header.hive_bins_size)?;
        cell::set_nk_field(&mut self.data, nk_off, cell::NK_VALUE_LIST, new_list)?;
        cell::set_nk_field(
            &mut self.data,
            nk_off,
            cell::NK_VALUE_COUNT,
            kept.len() as u32,
        )?;
        Ok(())
    }

    // -- Serialization ------------------------------------------------------

    /// Finalizes the header (sequences + checksum) and returns the hive bytes.
    /// Consumes one sequence "tick": each call produces a distinct consistent
    /// transaction.
    pub fn to_bytes(&mut self) -> Vec<u8> {
        self.header.finalize(&mut self.data);
        self.data.clone()
    }

    #[cfg(feature = "std")]
    pub fn save<P: AsRef<std::path::Path>>(&mut self, path: P) -> std::io::Result<()> {
        let bytes = self.to_bytes();
        std::fs::write(path, bytes)
    }

    // -- Internals ----------------------------------------------------------

    fn guard_writable(&self) -> Result<()> {
        if self.header.is_dirty() {
            return Err(RegError::DirtyHive);
        }
        Ok(())
    }

    fn resolve(&self, path: &str) -> Result<u32> {
        let mut current = self.header.root_cell_offset;
        for part in path.split('\\').filter(|s| !s.is_empty()) {
            current = self
                .find_child(current, part)?
                .ok_or_else(|| RegError::KeyNotFound(path.to_string()))?;
        }
        Ok(current)
    }

    fn find_child(&self, parent: u32, name_wanted: &str) -> Result<Option<u32>> {
        let nk = cell::read_key_node(&self.data, parent)?;
        for off in cell::subkey_offsets(&self.data, nk.subkey_list_offset)? {
            let child = cell::read_key_node(&self.data, off)?;
            if name::eq_name(&child.name, name_wanted) {
                return Ok(Some(off));
            }
        }
        Ok(None)
    }

    fn child_entries(&self, parent: u32) -> Result<Vec<(String, u32)>> {
        let nk = cell::read_key_node(&self.data, parent)?;
        let mut out = Vec::new();
        for off in cell::subkey_offsets(&self.data, nk.subkey_list_offset)? {
            out.push((cell::read_key_node(&self.data, off)?.name, off));
        }
        Ok(out)
    }

    /// Creates a subkey `name` under `parent`, inserted in sorted position.
    fn insert_subkey(&mut self, parent: u32, name_new: &str) -> Result<u32> {
        let parent_nk = cell::read_key_node(&self.data, parent)?;

        // Security inherited from the parent (reference count incremented).
        let security = parent_nk.security_offset;
        cell::incr_security_refcount(&mut self.data, security)?;

        // New nk.
        let nk_payload = cell::build_key_node(name_new, parent, security);
        let new_off = self.alloc_write(&nk_payload)?;

        // Existing entries + the new one, sorted.
        let mut entries = self.child_entries(parent)?;
        entries.push((name_new.to_string(), new_off));
        entries.sort_by(|a, b| name::cmp_str(&a.0, &b.0));

        // Leaf format preserved (ri flattened to lf).
        let kind =
            if parent_nk.subkey_list_offset == cell::FREE || parent_nk.subkey_list_offset == 0 {
                LeafKind::Lf
            } else {
                cell::leaf_kind(&self.data, parent_nk.subkey_list_offset)?
            };
        let list_payload = cell::build_leaf_list(kind, &entries);
        let new_list = self.alloc_write(&list_payload)?;

        // Free the old list (and its sublists if it was a ri).
        self.free_subkey_list(parent_nk.subkey_list_offset)?;

        // Update the parent.
        cell::set_nk_field(&mut self.data, parent, cell::NK_SUBKEY_LIST, new_list)?;
        cell::set_nk_field(
            &mut self.data,
            parent,
            cell::NK_SUBKEY_COUNT,
            entries.len() as u32,
        )?;
        Ok(new_off)
    }

    /// Frees a subkey list. For a `ri`, also frees each leaf sublist it
    /// references.
    fn free_subkey_list(&mut self, list_off: u32) -> Result<()> {
        if list_off == cell::FREE || list_off == 0 {
            return Ok(());
        }
        // Collect the ri sublists before freeing.
        let sublists: Vec<u32> = {
            let p = cell::cell_payload(&self.data, list_off)?;
            if p.len() >= 4 && &p[0..2] == b"ri" {
                let count = u16::from_le_bytes(p[2..4].try_into().unwrap()) as usize;
                (0..count)
                    .filter_map(|i| {
                        let o = 4 + i * 4;
                        (o + 4 <= p.len())
                            .then(|| u32::from_le_bytes(p[o..o + 4].try_into().unwrap()))
                    })
                    .collect()
            } else {
                Vec::new()
            }
        };
        for sub in sublists {
            hbin::free(&mut self.data, sub, self.header.hive_bins_size)?;
        }
        hbin::free(&mut self.data, list_off, self.header.hive_bins_size)?;
        Ok(())
    }

    fn write_value_list(&mut self, offs: &[u32]) -> Result<u32> {
        let mut payload = Vec::with_capacity(offs.len() * 4);
        for o in offs {
            payload.extend_from_slice(&o.to_le_bytes());
        }
        self.alloc_write(&payload)
    }

    /// Frees a vk's data cell (unless it is inline).
    fn free_value_storage(&mut self, vk: &cell::ValueNodeRaw) -> Result<()> {
        if !vk.inline && vk.data_size != 0 && vk.data_offset != cell::FREE && vk.data_offset != 0 {
            hbin::free(&mut self.data, vk.data_offset, self.header.hive_bins_size)?;
        }
        Ok(())
    }

    /// Allocates a cell and writes `payload` into it.
    fn alloc_write(&mut self, payload: &[u8]) -> Result<u32> {
        let off = hbin::allocate(
            &mut self.data,
            &mut self.header.hive_bins_size,
            payload.len(),
        )?;
        let dst = hbin::payload_mut(&mut self.data, off)?;
        dst[..payload.len()].copy_from_slice(payload);
        Ok(off)
    }

    fn node_at(&self, offset: u32) -> Result<KeyNode> {
        let raw = cell::read_key_node(&self.data, offset)?;
        Ok(KeyNode {
            name: raw.name,
            subkey_count: raw.subkey_count,
            value_count: raw.value_count,
            offset,
        })
    }
}

#[inline]
fn wr(data: &mut [u8], off: usize, v: u32) {
    data[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// Minimal self-relative security descriptor (a 40-byte sk cell).
fn build_min_security() -> Vec<u8> {
    // sk: "sk"(2) rsvd(2) flink(4) blink(4) refcount(4) sd_size(4) sd(20)
    let mut p = alloc::vec![0u8; 2 + 2 + 4 + 4 + 4 + 4 + 20];
    p[0..2].copy_from_slice(b"sk");
    // flink/blink/refcount filled by the caller (offset known after alloc).
    let sd_size = 20u32;
    p[16..20].copy_from_slice(&sd_size.to_le_bytes());
    // Self-relative security descriptor: revision 1, control SE_SELF_RELATIVE.
    let sd = 20;
    p[sd] = 1; // revision
    p[sd + 2..sd + 4].copy_from_slice(&0x8000u16.to_le_bytes()); // control
                                                                 // owner/group/sacl/dacl offsets = 0 (absent)
    p
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::header::{Header, REGF_HEADER_SIZE};

    #[test]
    fn header_dirty_detection() {
        let clean = Header {
            primary_sequence: 5,
            secondary_sequence: 5,
            major_version: 1,
            minor_version: 5,
            root_cell_offset: 0x20,
            hive_bins_size: 0x1000,
        };
        assert!(!clean.is_dirty());
        let dirty = Header {
            primary_sequence: 6,
            secondary_sequence: 5,
            major_version: 1,
            minor_version: 5,
            root_cell_offset: 0x20,
            hive_bins_size: 0x1000,
        };
        assert!(dirty.is_dirty());
    }

    /// A dirty hive (diverging sequences) must reject any write.
    #[test]
    fn refuses_write_on_dirty_hive() {
        // Valid empty hive, made "dirty" by desyncing the sequences, then
        // recomputing the checksum so it stays parsable.
        let mut data = Hive::new_empty("ROOT").to_bytes();
        let primary = u32::from_le_bytes(data[0x04..0x08].try_into().unwrap());
        data[0x08..0x0C].copy_from_slice(&primary.wrapping_add(1).to_le_bytes());
        let sum = Header::checksum(&data);
        data[0x1FC..0x200].copy_from_slice(&sum.to_le_bytes());

        let mut hive = Hive::from_bytes(data).unwrap();
        assert!(hive.is_dirty());
        assert_eq!(
            hive.set_value("ROOT", "x", RegValue::Dword(1)),
            Err(RegError::DirtyHive)
        );
        assert!(matches!(
            hive.create_key("ROOT\\Zzz"),
            Err(RegError::DirtyHive)
        ));
    }

    /// After finalization, the hive is clean (equal sequences).
    #[test]
    fn finalize_makes_clean() {
        let mut hive = Hive::new_empty("ROOT");
        let _ = hive.to_bytes();
        assert!(!hive.is_dirty());
        assert_eq!(REGF_HEADER_SIZE, 0x1000);
    }
}
