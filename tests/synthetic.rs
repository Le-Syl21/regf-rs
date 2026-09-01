#![cfg(feature = "std")]
mod common;
use common::{synthetic_bcd, BOOTMGR, OSLOADER};
use regf_rs::{Hive, RegValue};

/// Une ruche vierge doit être valide selon nt-hive.
#[test]
fn new_empty_is_valid() {
    let mut h = Hive::new_empty("BCD");
    let bytes = h.to_bytes();
    let nt = nt_hive::Hive::new(bytes.as_ref()).expect("nt-hive parse");
    nt.validate().expect("structure valide");
    assert_eq!(
        nt.root_key_node().unwrap().name().unwrap().to_string(),
        "BCD"
    );
}

/// La fixture synthétique se relit correctement (nous + nt-hive).
#[test]
fn synthetic_bcd_roundtrips() {
    let mut h = synthetic_bcd();
    let path = format!("Objects\\{BOOTMGR}\\Elements\\23000003");
    assert_eq!(
        h.get_value(&path, "Element").unwrap(),
        RegValue::Sz(OSLOADER.into())
    );

    let bytes = h.to_bytes();
    let nt = nt_hive::Hive::new(bytes.as_ref()).unwrap();
    nt.validate().unwrap();
    // Recherche binaire côté nt-hive (⇒ listes triées correctement).
    let root = nt.root_key_node().unwrap();
    assert!(root.subpath(&path).is_some());
}
