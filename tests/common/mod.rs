//! Shared synthetic fixture: a BCD-like hive built in memory,
//! with no real machine data. Enables self-contained (CI) and
//! reproducible tests, without versioning any personal hive.

#![allow(dead_code)]

use regf_rs::{Hive, RegValue};

/// Fixed Windows Boot Manager GUID (a public Microsoft constant, not tied
/// to any machine).
pub const BOOTMGR: &str = "{9dea862c-5cdd-4e70-acc1-f32b344d4795}";
/// Neutral GUID acting as the default "OS loader" (made up).
pub const OSLOADER: &str = "{11111111-2222-3333-4444-555555555555}";

/// Builds a BCD-like hive: `Objects\{bootmgr}\Elements\{23000003,
/// 24000001, 25000004}` filled as in a real BCD, plus an OS loader
/// object. No identifying value.
pub fn synthetic_bcd() -> Hive {
    let mut h = Hive::new_empty("BCD");

    let bootmgr_elems = format!("Objects\\{BOOTMGR}\\Elements");
    h.create_key(&bootmgr_elems).unwrap();
    // DefaultObject → the OS loader.
    set(
        &mut h,
        &bootmgr_elems,
        "23000003",
        RegValue::Sz(OSLOADER.into()),
    );
    // DisplayOrder.
    set(
        &mut h,
        &bootmgr_elems,
        "24000001",
        RegValue::MultiSz(vec![OSLOADER.into()]),
    );
    // Timeout = 30 s.
    set(
        &mut h,
        &bootmgr_elems,
        "25000004",
        RegValue::Binary(vec![30, 0, 0, 0, 0, 0, 0, 0]),
    );

    // A minimal OS loader object.
    let os_elems = format!("Objects\\{OSLOADER}\\Elements");
    h.create_key(&os_elems).unwrap();
    set(
        &mut h,
        &os_elems,
        "12000004",
        RegValue::Sz("Windows".into()),
    );

    h
}

fn set(h: &mut Hive, elements: &str, code: &str, v: RegValue) {
    let path = format!("{elements}\\{code}");
    h.create_key(&path).unwrap();
    h.set_value(&path, "Element", v).unwrap();
}
