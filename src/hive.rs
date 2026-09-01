//! API haut niveau : chargement, navigation, lecture et écriture in-place.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::cell::{self, LeafKind};
use crate::error::{RegError, Result};
use crate::hbin;
use crate::header::Header;
use crate::name;
use crate::value::RegValue;

/// Une ruche REGF chargée en mémoire.
#[derive(Clone)]
pub struct Hive {
    data: Vec<u8>,
    header: Header,
}

/// Vue publique d'un nœud de clé.
#[derive(Debug, Clone)]
pub struct KeyNode {
    pub name: String,
    pub subkey_count: u32,
    pub value_count: u32,
    pub(crate) offset: u32,
}

impl KeyNode {
    /// Data offset de ce nœud dans la ruche (relatif aux hive bins).
    pub fn offset(&self) -> u32 {
        self.offset
    }
}

impl Hive {
    // -- Chargement ---------------------------------------------------------

    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let header = Header::parse(&data)?;
        Ok(Hive { data, header })
    }

    /// Crée une ruche vierge valide : base block, un hive bin, un nœud racine
    /// nommé `root_name` et un descripteur de sécurité minimal partageable.
    /// Utile pour bâtir des ruches de test ou neuves sans fichier d'origine.
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

        // Premier hive bin, puis une grande cellule libre.
        let hb = crate::header::REGF_HEADER_SIZE;
        data[hb..hb + 4].copy_from_slice(b"hbin");
        wr(&mut data, hb + 4, 0); // offset du bin
        wr(&mut data, hb + 8, HBIN as u32); // taille du bin
        let free_cell = hb + 0x20;
        let free_size = (HBIN - 0x20) as i32;
        data[free_cell..free_cell + 4].copy_from_slice(&free_size.to_le_bytes());

        let mut header = Header::parse_unchecked(&data);
        let mut hive = Hive {
            data,
            header: header.clone(),
        };

        // Descripteur de sécurité minimal (self-relative, DACL absente).
        let sk = build_min_security();
        let sk_off = hive.alloc_write(&sk).expect("alloc sk");
        // flink/blink auto-référents + refcount = 1.
        {
            let p = hbin::payload_mut(&mut hive.data, sk_off).unwrap();
            p[4..8].copy_from_slice(&sk_off.to_le_bytes());
            p[8..12].copy_from_slice(&sk_off.to_le_bytes());
            p[12..16].copy_from_slice(&1u32.to_le_bytes());
        }

        // Nœud racine.
        let nk = cell::build_key_node(root_name, cell::FREE, sk_off);
        let nk_off = hive.alloc_write(&nk).expect("alloc nk");
        // Marque les drapeaux racine (hive entry | no delete | comp name).
        {
            let p = hbin::payload_mut(&mut hive.data, nk_off).unwrap();
            p[2..4].copy_from_slice(&0x2Cu16.to_le_bytes());
        }

        // root_cell_offset dans l'en-tête + resync de la structure Header.
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

    /// Voir [`Header::is_dirty`].
    pub fn is_dirty(&self) -> bool {
        self.header.is_dirty()
    }

    // -- Navigation ---------------------------------------------------------

    pub fn root_key(&self) -> Result<KeyNode> {
        self.node_at(self.header.root_cell_offset)
    }

    /// Ouvre une clé par chemin (séparateur `\`), insensible à la casse.
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

    // -- Écriture -----------------------------------------------------------

    /// Crée les clés manquantes le long du chemin et renvoie l'offset de la
    /// clé finale. Chaque niveau créé hérite du descripteur de sécurité de son
    /// parent et est inséré en position triée dans la liste de sous-clés.
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

    /// Définit (crée ou remplace) une valeur sous une clé existante.
    pub fn set_value(&mut self, key_path: &str, value_name: &str, value: RegValue) -> Result<()> {
        self.guard_writable()?;
        let nk_off = self.resolve(key_path)?;

        // Encodage de la donnée (owned : aucun emprunt ne traverse une alloc).
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

        // Liste de valeurs existante.
        let nk = cell::read_key_node(&self.data, nk_off)?;
        let existing = cell::value_offsets(&self.data, nk.value_list_offset, nk.value_count)?;

        // Remplacement si le nom existe déjà.
        for (i, &vk_off) in existing.iter().enumerate() {
            let vk = cell::read_value_node(&self.data, vk_off)?;
            if name::eq_name(&vk.name, value_name) {
                // Remplace l'offset dans la liste (taille inchangée).
                let list_payload = hbin::payload_mut(&mut self.data, nk.value_list_offset)?;
                list_payload[i * 4..i * 4 + 4].copy_from_slice(&new_vk.to_le_bytes());
                self.free_value_storage(&vk)?;
                hbin::free(&mut self.data, vk_off, self.header.hive_bins_size)?;
                return Ok(());
            }
        }

        // Ajout : nouvelle liste de valeurs = anciennes + la nouvelle.
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

    /// Supprime une valeur. Erreur si elle n'existe pas.
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

    // -- Sérialisation ------------------------------------------------------

    /// Finalise l'en-tête (séquences + checksum) et renvoie les octets de la
    /// ruche. Consomme un « cran » de séquence : chaque appel produit une
    /// transaction cohérente distincte.
    pub fn to_bytes(&mut self) -> Vec<u8> {
        self.header.finalize(&mut self.data);
        self.data.clone()
    }

    #[cfg(feature = "std")]
    pub fn save<P: AsRef<std::path::Path>>(&mut self, path: P) -> std::io::Result<()> {
        let bytes = self.to_bytes();
        std::fs::write(path, bytes)
    }

    // -- Internes -----------------------------------------------------------

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

    /// Crée une sous-clé `name` sous `parent`, insérée en position triée.
    fn insert_subkey(&mut self, parent: u32, name_new: &str) -> Result<u32> {
        let parent_nk = cell::read_key_node(&self.data, parent)?;

        // Sécurité héritée du parent (compteur de références incrémenté).
        let security = parent_nk.security_offset;
        cell::incr_security_refcount(&mut self.data, security)?;

        // Nouveau nk.
        let nk_payload = cell::build_key_node(name_new, parent, security);
        let new_off = self.alloc_write(&nk_payload)?;

        // Entrées existantes + la nouvelle, triées.
        let mut entries = self.child_entries(parent)?;
        entries.push((name_new.to_string(), new_off));
        entries.sort_by(|a, b| name::cmp_str(&a.0, &b.0));

        // Format feuille préservé (ri aplati en lf).
        let kind =
            if parent_nk.subkey_list_offset == cell::FREE || parent_nk.subkey_list_offset == 0 {
                LeafKind::Lf
            } else {
                cell::leaf_kind(&self.data, parent_nk.subkey_list_offset)?
            };
        let list_payload = cell::build_leaf_list(kind, &entries);
        let new_list = self.alloc_write(&list_payload)?;

        // Libère l'ancienne liste (et ses sous-listes si c'était un ri).
        self.free_subkey_list(parent_nk.subkey_list_offset)?;

        // Met à jour le parent.
        cell::set_nk_field(&mut self.data, parent, cell::NK_SUBKEY_LIST, new_list)?;
        cell::set_nk_field(
            &mut self.data,
            parent,
            cell::NK_SUBKEY_COUNT,
            entries.len() as u32,
        )?;
        Ok(new_off)
    }

    /// Libère une liste de sous-clés. Pour un `ri`, libère aussi chaque
    /// sous-liste feuille qu'il référence.
    fn free_subkey_list(&mut self, list_off: u32) -> Result<()> {
        if list_off == cell::FREE || list_off == 0 {
            return Ok(());
        }
        // Repère les sous-listes ri avant de libérer.
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

    /// Libère la cellule de données d'un vk (si elle n'est pas inline).
    fn free_value_storage(&mut self, vk: &cell::ValueNodeRaw) -> Result<()> {
        if !vk.inline && vk.data_size != 0 && vk.data_offset != cell::FREE && vk.data_offset != 0 {
            hbin::free(&mut self.data, vk.data_offset, self.header.hive_bins_size)?;
        }
        Ok(())
    }

    /// Alloue une cellule et y écrit `payload`.
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

/// Descripteur de sécurité auto-relatif minimal (40 octets de cellule sk).
fn build_min_security() -> Vec<u8> {
    // sk : "sk"(2) rsvd(2) flink(4) blink(4) refcount(4) sd_size(4) sd(20)
    let mut p = alloc::vec![0u8; 2 + 2 + 4 + 4 + 4 + 4 + 20];
    p[0..2].copy_from_slice(b"sk");
    // flink/blink/refcount remplis par l'appelant (offset connu après alloc).
    let sd_size = 20u32;
    p[16..20].copy_from_slice(&sd_size.to_le_bytes());
    // Security descriptor self-relative : revision 1, control SE_SELF_RELATIVE.
    let sd = 20;
    p[sd] = 1; // revision
    p[sd + 2..sd + 4].copy_from_slice(&0x8000u16.to_le_bytes()); // control
                                                                 // owner/group/sacl/dacl offsets = 0 (absents)
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

    /// Une ruche sale (séquences divergentes) refuse toute écriture.
    #[test]
    fn refuses_write_on_dirty_hive() {
        // Ruche vierge valide, rendue « sale » en désynchronisant les séquences
        // puis en recalculant le checksum pour qu'elle reste parsable.
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

    /// Après finalisation, la ruche est propre (séquences égales).
    #[test]
    fn finalize_makes_clean() {
        let mut hive = Hive::new_empty("ROOT");
        let _ = hive.to_bytes();
        assert!(!hive.is_dirty());
        assert_eq!(REGF_HEADER_SIZE, 0x1000);
    }
}
