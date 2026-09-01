#![cfg(feature = "std")]

//! Tests d'écriture in-place, validés en croisé avec `nt-hive` (implémentation
//! REGF indépendante) sur une vraie ruche BCD.

use regf_rs::{Hive, RegValue};
use std::collections::BTreeSet;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/bcd_sample.hive"
);
const BOOTMGR: &str = "{9dea862c-5cdd-4e70-acc1-f32b344d4795}";

fn load() -> Hive {
    Hive::from_bytes(std::fs::read(FIXTURE).unwrap()).unwrap()
}

/// Ouvre une ruche via nt-hive et renvoie la valeur `Element` d'un chemin,
/// en s'appuyant sur la RECHERCHE (subpath), pas l'énumération : c'est ce que
/// fait Windows, et c'est ce que la position triée doit rendre possible.
fn nt_lookup(bytes: &[u8], key_path: &str) -> Option<String> {
    let hive = nt_hive::Hive::new(bytes).unwrap();
    let root = hive.root_key_node().unwrap();
    let node = root.subpath(key_path)?.ok()?;
    let v = node.value("Element")?.ok()?;
    v.string_data().ok()
}

fn nt_subkeys(bytes: &[u8], key_path: &str) -> BTreeSet<String> {
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

/// Modifier une valeur existante (DefaultObject).
#[test]
fn modify_default_object() {
    let mut hive = load();
    let path = format!("Objects\\{BOOTMGR}\\Elements\\23000003");
    let target = "{aabbccdd-1122-3344-5566-778899aabbcc}";
    hive.set_value(&path, "Element", RegValue::Sz(target.into()))
        .unwrap();

    let bytes = hive.to_bytes();
    // Oracle nt-hive par recherche.
    assert_eq!(nt_lookup(&bytes, &path).as_deref(), Some(target));
    // En-tête revalidé de zéro (checksum recalculé).
    assert!(nt_hive::Hive::new(bytes.as_ref())
        .unwrap()
        .validate()
        .is_ok());
}

/// LE cas qui piégeait viva-uefi-regf : CRÉER BootSequence (24000002), une clé
/// qui n'existe pas, et vérifier que nt-hive la TROUVE par recherche (⇒ elle
/// est insérée en position triée, pas ajoutée en fin de liste).
#[test]
fn create_boot_sequence_is_findable() {
    let mut hive = load();
    let elements = format!("Objects\\{BOOTMGR}\\Elements");
    let seq = format!("{elements}\\24000002");
    let win = "{c54212ab-98d4-11f1-98b1-b88584af3547}";

    hive.create_key(&seq).unwrap();
    hive.set_value(&seq, "Element", RegValue::MultiSz(vec![win.into()]))
        .unwrap();

    let bytes = hive.to_bytes();

    // 1. structure globale toujours valide selon nt-hive
    assert!(nt_hive::Hive::new(bytes.as_ref())
        .unwrap()
        .validate()
        .is_ok());

    // 2. la clé est TROUVABLE par recherche (position triée correcte)
    let hive_nt = nt_hive::Hive::new(bytes.as_ref()).unwrap();
    let root = hive_nt.root_key_node().unwrap();
    assert!(
        root.subpath(&seq).is_some(),
        "24000002 doit être trouvable par recherche binaire (insertion triée)"
    );

    // 3. et sa valeur est lisible
    let hive_nt = nt_hive::Hive::new(bytes.as_ref()).unwrap();
    let root = hive_nt.root_key_node().unwrap();
    let node = root.subpath(&seq).unwrap().unwrap();
    let v = node.value("Element").unwrap().unwrap();
    let got: Vec<String> = v
        .multi_string_data()
        .unwrap()
        .filter_map(|r| r.ok())
        .map(|s| s.to_string())
        .collect();
    assert_eq!(got, vec![win.to_string()]);

    // 4. l'ordre vu par nt-hive est effectivement trié
    let subkeys: Vec<String> = nt_subkeys(&bytes, &elements).into_iter().collect();
    let mut sorted = subkeys.clone();
    sorted.sort();
    assert_eq!(subkeys, sorted, "les sous-clés doivent être ordonnées");
    assert!(subkeys.contains(&"24000002".to_string()));
}

/// Remplacer une valeur par une nettement plus longue (réallocation de cellule).
#[test]
fn grow_value_reallocates() {
    let mut hive = load();
    let path = format!("Objects\\{BOOTMGR}\\Elements\\23000003");
    let long = "{aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}".repeat(30); // ~1100 caractères
    hive.set_value(&path, "Element", RegValue::Sz(long.clone()))
        .unwrap();

    let bytes = hive.to_bytes();
    assert!(nt_hive::Hive::new(bytes.as_ref())
        .unwrap()
        .validate()
        .is_ok());
    assert_eq!(nt_lookup(&bytes, &path).as_deref(), Some(long.as_str()));
}

/// Supprimer une valeur : elle disparaît, le reste survit.
#[test]
fn delete_value_works() {
    let mut hive = load();
    let path = format!("Objects\\{BOOTMGR}\\Elements\\25000004"); // Timeout
    assert!(hive.get_value(&path, "Element").is_ok());
    hive.delete_value(&path, "Element").unwrap();
    assert!(matches!(
        hive.get_value(&path, "Element"),
        Err(regf_rs::RegError::ValueNotFound(_))
    ));
    let bytes = hive.to_bytes();
    assert!(nt_hive::Hive::new(bytes.as_ref())
        .unwrap()
        .validate()
        .is_ok());
}

/// Cycle complet : armer un one-shot puis l'annuler, façon os-switcher.
#[test]
fn arm_then_clear_oneshot() {
    let mut hive = load();
    let elements = format!("Objects\\{BOOTMGR}\\Elements");
    let seq = format!("{elements}\\24000002");
    let win = "{c54212ab-98d4-11f1-98b1-b88584af3547}";

    hive.create_key(&seq).unwrap();
    hive.set_value(&seq, "Element", RegValue::MultiSz(vec![win.into()]))
        .unwrap();
    assert!(hive.get_value(&seq, "Element").is_ok());

    hive.delete_value(&seq, "Element").unwrap();
    let bytes = hive.to_bytes();
    assert!(nt_hive::Hive::new(bytes.as_ref())
        .unwrap()
        .validate()
        .is_ok());
}

/// Intégrité globale : après une écriture, TOUS les objets et une valeur
/// témoin d'un autre objet restent lisibles (aucune corruption collatérale).
#[test]
fn write_preserves_unrelated_data() {
    let mut hive = load();

    // Valeur témoin dans un autre objet (le Boot Manager) avant modif.
    let default_before = hive
        .get_value(
            &format!("Objects\\{BOOTMGR}\\Elements\\24000001"),
            "Element",
        )
        .unwrap();

    // Modification ailleurs.
    let seq = format!("Objects\\{BOOTMGR}\\Elements\\24000002");
    hive.create_key(&seq).unwrap();
    hive.set_value(&seq, "Element", RegValue::MultiSz(vec!["{x}".into()]))
        .unwrap();

    let bytes = hive.to_bytes();
    let nt = nt_hive::Hive::new(bytes.as_ref()).unwrap();
    let root = nt.root_key_node().unwrap();

    // Les 18 objets d'origine + rien perdu.
    let objects = root.subpath("Objects").unwrap().unwrap();
    let count = objects
        .subkeys()
        .unwrap()
        .unwrap()
        .filter_map(|k| k.ok())
        .count();
    assert_eq!(count, 18);

    // La valeur témoin est intacte.
    let default_after = hive
        .get_value(
            &format!("Objects\\{BOOTMGR}\\Elements\\24000001"),
            "Element",
        )
        .unwrap();
    assert_eq!(default_before, default_after);
}

/// Round-trip par notre propre implémentation : ce qu'on écrit se relit.
#[test]
fn self_roundtrip() {
    let mut hive = load();
    let path = format!("Objects\\{BOOTMGR}\\Elements\\23000003");
    let target = "{deadbeef-0000-1111-2222-333344445555}";
    hive.set_value(&path, "Element", RegValue::Sz(target.into()))
        .unwrap();

    let bytes = hive.to_bytes();
    let reloaded = Hive::from_bytes(bytes).unwrap();
    assert!(!reloaded.is_dirty(), "ruche propre après finalisation");
    assert_eq!(
        reloaded.get_value(&path, "Element").unwrap(),
        RegValue::Sz(target.into())
    );
}
