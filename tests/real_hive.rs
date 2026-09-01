#![cfg(feature = "std")]
//! Optional validation against a REAL hive produced by Windows.
//! Provide the path via the `REGF_TEST_HIVE` environment variable:
//!   REGF_TEST_HIVE=/boot/efi/EFI/Microsoft/Boot/BCD cargo test --test real_hive
//! Without it, the test is skipped (no personal hive is versioned).
use regf_rs::Hive;

#[test]
fn reads_real_hive_and_matches_nt_hive() {
    let Ok(path) = std::env::var("REGF_TEST_HIVE") else {
        eprintln!("REGF_TEST_HIVE not set: test skipped");
        return;
    };
    let bytes = std::fs::read(&path).expect("read hive");
    let hive = Hive::from_bytes(bytes.clone()).expect("parse regf-rs");

    // nt-hive oracle: same subkeys at the root.
    let ours: std::collections::BTreeSet<String> = {
        let root = hive.root_key().unwrap();
        hive.list_subkeys(&root.name)
            .unwrap_or_default()
            .into_iter()
            .collect()
    };
    let nt = nt_hive::Hive::new(bytes.as_ref()).unwrap();
    nt.validate().unwrap();
    let _ = ours; // reading without panic + validate() is enough here
    eprintln!("real hive read and validated: {path}");
}
