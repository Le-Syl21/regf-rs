#![cfg(feature = "std")]

//! Validation de la lecture contre une vraie ruche BCD, avec `nt-hive`
//! (implémentation REGF indépendante) comme oracle croisé.

use regf_rs::Hive;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/bcd_sample.hive"
);

#[test]
fn parse_header_and_root() {
    let hive = Hive::from_file(FIXTURE).expect("chargement BCD");
    let h = hive.header();
    assert_eq!(
        h.primary_sequence, h.secondary_sequence,
        "ruche propre (séquences égales)"
    );
    let root = hive.root_key().expect("racine");
    assert_eq!(root.name, "NewStoreRoot");
}

#[test]
fn lists_objects() {
    let hive = Hive::from_file(FIXTURE).unwrap();
    let objects = hive.list_subkeys("Objects").expect("Objects");
    assert_eq!(objects.len(), 18, "18 objets attendus dans le BCD de test");
}

/// Oracle : la liste des sous-clés produite par regf-rs doit être identique
/// (en ensemble) à celle produite par nt-hive sur la même clé.
#[test]
fn cross_check_with_nt_hive() {
    let buf = std::fs::read(FIXTURE).unwrap();

    // --- regf-rs ---
    let ours: std::collections::BTreeSet<String> = Hive::from_bytes(buf.clone())
        .unwrap()
        .list_subkeys("Objects")
        .unwrap()
        .into_iter()
        .collect();

    // --- nt-hive ---
    let hive = nt_hive::Hive::new(buf.as_ref()).unwrap();
    let root = hive.root_key_node().unwrap();
    let objects = root.subpath("Objects").unwrap().unwrap();
    let theirs: std::collections::BTreeSet<String> = objects
        .subkeys()
        .unwrap()
        .unwrap()
        .filter_map(|k| k.ok())
        .filter_map(|k| k.name().ok().map(|n| n.to_string()))
        .collect();

    assert_eq!(
        ours, theirs,
        "regf-rs et nt-hive doivent voir les mêmes sous-clés"
    );
}
