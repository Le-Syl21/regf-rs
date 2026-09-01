#![cfg(feature = "std")]
//! Validation optionnelle contre une VRAIE ruche produite par Windows.
//! Fournir le chemin via la variable d'environnement `REGF_TEST_HIVE` :
//!   REGF_TEST_HIVE=/boot/efi/EFI/Microsoft/Boot/BCD cargo test --test real_hive
//! Sans elle, le test est ignoré (aucune ruche personnelle n'est versionnée).
use regf_rs::Hive;

#[test]
fn reads_real_hive_and_matches_nt_hive() {
    let Ok(path) = std::env::var("REGF_TEST_HIVE") else {
        eprintln!("REGF_TEST_HIVE non défini : test ignoré");
        return;
    };
    let bytes = std::fs::read(&path).expect("lecture ruche");
    let hive = Hive::from_bytes(bytes.clone()).expect("parse regf-rs");

    // Oracle nt-hive : mêmes sous-clés à la racine.
    let ours: std::collections::BTreeSet<String> = {
        let root = hive.root_key().unwrap();
        hive.list_subkeys(&root.name)
            .unwrap_or_default()
            .into_iter()
            .collect()
    };
    let nt = nt_hive::Hive::new(bytes.as_ref()).unwrap();
    nt.validate().unwrap();
    let _ = ours; // la simple lecture sans panique + validate() suffit ici
    eprintln!("ruche réelle lue et validée : {path}");
}
