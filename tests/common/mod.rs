//! Fixture synthétique partagée : une ruche façon BCD construite en mémoire,
//! sans aucune donnée machine réelle. Permet des tests autonomes (CI) et
//! reproductibles, sans versionner de ruche personnelle.

#![allow(dead_code)]

use regf_rs::{Hive, RegValue};

/// GUID fixe du Windows Boot Manager (constante publique Microsoft, non liée
/// à une machine).
pub const BOOTMGR: &str = "{9dea862c-5cdd-4e70-acc1-f32b344d4795}";
/// GUID neutre jouant le rôle d'« OS loader » par défaut (inventé).
pub const OSLOADER: &str = "{11111111-2222-3333-4444-555555555555}";

/// Construit une ruche BCD-like : `Objects\{bootmgr}\Elements\{23000003,
/// 24000001, 25000004}` renseignés comme dans un vrai BCD, plus un objet
/// OS loader. Aucune valeur identifiante.
pub fn synthetic_bcd() -> Hive {
    let mut h = Hive::new_empty("BCD");

    let bootmgr_elems = format!("Objects\\{BOOTMGR}\\Elements");
    h.create_key(&bootmgr_elems).unwrap();
    // DefaultObject → l'OS loader.
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

    // Un objet OS loader minimal.
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
