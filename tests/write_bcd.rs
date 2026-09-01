#![cfg(feature = "std")]
//! Écriture in-place, validée en croisé avec `nt-hive` sur une ruche synthétique.
mod common;
use common::{synthetic_bcd, BOOTMGR};
use regf_rs::{Hive, RegValue};
use std::collections::BTreeSet;

fn nt_lookup(bytes: &[u8], key_path: &str) -> Option<String> {
    let hive = nt_hive::Hive::new(bytes).unwrap();
    let root = hive.root_key_node().unwrap();
    let node = root.subpath(key_path)?.ok()?;
    node.value("Element")?.ok()?.string_data().ok()
}

fn nt_subkeys(bytes: &[u8], key_path: &str) -> Vec<String> {
    let hive = nt_hive::Hive::new(bytes).unwrap();
    let root = hive.root_key_node().unwrap();
    let node = root.subpath(key_path).unwrap().unwrap();
    node.subkeys()
        .unwrap()
        .unwrap()
        .filter_map(|k| k.ok())
        .filter_map(|k| k.name().ok().map(|n| n.to_string()))
        .collect()
}

fn validate(bytes: &[u8]) {
    nt_hive::Hive::new(bytes).unwrap().validate().unwrap();
}

#[test]
fn modify_default_object() {
    let mut h = synthetic_bcd();
    let path = format!("Objects\\{BOOTMGR}\\Elements\\23000003");
    let target = "{aabbccdd-1122-3344-5566-778899aabbcc}";
    h.set_value(&path, "Element", RegValue::Sz(target.into()))
        .unwrap();
    let bytes = h.to_bytes();
    validate(&bytes);
    assert_eq!(nt_lookup(&bytes, &path).as_deref(), Some(target));
}

/// LE cas qui piégeait viva-uefi-regf : créer une clé absente et vérifier que
/// nt-hive la TROUVE par recherche binaire (⇒ insérée en position triée).
#[test]
fn create_boot_sequence_is_findable() {
    let mut h = synthetic_bcd();
    let elements = format!("Objects\\{BOOTMGR}\\Elements");
    let seq = format!("{elements}\\24000002");
    let win = "{c54212ab-0000-0000-0000-000000000000}";

    h.create_key(&seq).unwrap();
    h.set_value(&seq, "Element", RegValue::MultiSz(vec![win.into()]))
        .unwrap();
    let bytes = h.to_bytes();

    validate(&bytes);
    let nt = nt_hive::Hive::new(bytes.as_ref()).unwrap();
    assert!(
        nt.root_key_node().unwrap().subpath(&seq).is_some(),
        "clé trouvable par recherche"
    );

    let subkeys = nt_subkeys(&bytes, &elements);
    let mut sorted = subkeys.clone();
    sorted.sort();
    assert_eq!(subkeys, sorted, "sous-clés ordonnées");
    assert!(subkeys.contains(&"24000002".to_string()));
}

#[test]
fn grow_value_reallocates() {
    let mut h = synthetic_bcd();
    let path = format!("Objects\\{BOOTMGR}\\Elements\\23000003");
    let long = "{aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}".repeat(30);
    h.set_value(&path, "Element", RegValue::Sz(long.clone()))
        .unwrap();
    let bytes = h.to_bytes();
    validate(&bytes);
    assert_eq!(nt_lookup(&bytes, &path).as_deref(), Some(long.as_str()));
}

#[test]
fn delete_value_works() {
    let mut h = synthetic_bcd();
    let path = format!("Objects\\{BOOTMGR}\\Elements\\25000004");
    assert!(h.get_value(&path, "Element").is_ok());
    h.delete_value(&path, "Element").unwrap();
    assert!(matches!(
        h.get_value(&path, "Element"),
        Err(regf_rs::RegError::ValueNotFound(_))
    ));
    validate(&h.to_bytes());
}

#[test]
fn arm_then_clear_oneshot() {
    let mut h = synthetic_bcd();
    let seq = format!("Objects\\{BOOTMGR}\\Elements\\24000002");
    let win = "{c54212ab-0000-0000-0000-000000000000}";
    h.create_key(&seq).unwrap();
    h.set_value(&seq, "Element", RegValue::MultiSz(vec![win.into()]))
        .unwrap();
    assert!(h.get_value(&seq, "Element").is_ok());
    h.delete_value(&seq, "Element").unwrap();
    validate(&h.to_bytes());
}

#[test]
fn write_preserves_unrelated_data() {
    let mut h = synthetic_bcd();
    let witness_path = format!("Objects\\{BOOTMGR}\\Elements\\24000001");
    let before = h.get_value(&witness_path, "Element").unwrap();

    let seq = format!("Objects\\{BOOTMGR}\\Elements\\24000002");
    h.create_key(&seq).unwrap();
    h.set_value(&seq, "Element", RegValue::MultiSz(vec!["{x}".into()]))
        .unwrap();

    let after = h.get_value(&witness_path, "Element").unwrap();
    assert_eq!(before, after);

    let bytes = h.to_bytes();
    validate(&bytes);
    let objects: BTreeSet<String> = nt_subkeys(&bytes, "Objects").into_iter().collect();
    assert_eq!(objects.len(), 2); // bootmgr + os loader intacts
}

#[test]
fn self_roundtrip() {
    let mut h = synthetic_bcd();
    let path = format!("Objects\\{BOOTMGR}\\Elements\\23000003");
    let target = "{deadbeef-0000-1111-2222-333344445555}";
    h.set_value(&path, "Element", RegValue::Sz(target.into()))
        .unwrap();
    let bytes = h.to_bytes();
    let reloaded = Hive::from_bytes(bytes).unwrap();
    assert!(!reloaded.is_dirty());
    assert_eq!(
        reloaded.get_value(&path, "Element").unwrap(),
        RegValue::Sz(target.into())
    );
}
