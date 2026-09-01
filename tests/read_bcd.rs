#![cfg(feature = "std")]
//! Reading, cross-checked against `nt-hive` on a synthetic hive.
mod common;
use common::{synthetic_bcd, BOOTMGR, OSLOADER};
use regf_rs::RegValue;
use std::collections::BTreeSet;

#[test]
fn parse_header_and_root() {
    let mut h = synthetic_bcd();
    assert!(!h.is_dirty());
    assert_eq!(h.root_key().unwrap().name, "BCD");
    let _ = h.to_bytes();
}

#[test]
fn reads_values_of_all_kinds() {
    let mut h = synthetic_bcd();
    let e = format!("Objects\\{BOOTMGR}\\Elements");
    assert_eq!(
        h.get_value(&format!("{e}\\23000003"), "Element").unwrap(),
        RegValue::Sz(OSLOADER.into())
    );
    assert_eq!(
        h.get_value(&format!("{e}\\24000001"), "Element").unwrap(),
        RegValue::MultiSz(vec![OSLOADER.into()])
    );
    assert!(matches!(
        h.get_value(&format!("{e}\\25000004"), "Element").unwrap(),
        RegValue::Binary(_)
    ));
    let _ = h.to_bytes();
}

/// Oracle: same subkeys seen by regf-rs and nt-hive.
#[test]
fn cross_check_with_nt_hive() {
    let mut h = synthetic_bcd();
    let ours: BTreeSet<String> = h.list_subkeys("Objects").unwrap().into_iter().collect();

    let bytes = h.to_bytes();
    let nt = nt_hive::Hive::new(bytes.as_ref()).unwrap();
    let root = nt.root_key_node().unwrap();
    let objects = root.subpath("Objects").unwrap().unwrap();
    let theirs: BTreeSet<String> = objects
        .subkeys()
        .unwrap()
        .unwrap()
        .filter_map(|k| k.ok())
        .filter_map(|k| k.name().ok().map(|n| n.to_string()))
        .collect();
    assert_eq!(ours, theirs);
}
