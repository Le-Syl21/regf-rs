# regf-rs

Lecteur et écrivain in-place de ruches de registre Windows (**format REGF**),
en Rust pur — sans dépendance C, `no_std`-compatible.

REGF est le format des ruches du registre : `SYSTEM`, `SOFTWARE`, `NTUSER.DAT`,
et le **BCD** (`\EFI\Microsoft\Boot\BCD`). Le paysage Rust actuel est lacunaire :
`nt-hive` lit mais n'écrit pas, `regf` ne crée que des ruches neuves, et les
solutions complètes passent par des bindings C (`hivex`). `regf-rs` vise la case
manquante : **lecture + écriture in-place**, en Rust pur et testée en croisé.

## Statut

- [x] **Lecture** : en-tête + checksum, navigation des clés (`nk`), listes de
      sous-clés (`lf`/`lh`/`li`/`ri`), valeurs (`vk`) de tous types `REG_*`,
      données inline et big-data (`db`).
- [x] **Écriture in-place** : allocation premier-ajustement avec scission,
      libération avec coalescence, **insertion triée** dans les listes de
      sous-clés (indispensable à la recherche binaire de Windows), création de
      clés (sécurité héritée du parent), (dé)définition de valeurs, recalcul du
      checksum et des numéros de séquence.
- [x] **Sûreté** : refus d'écrire sur une ruche non réconciliée (transaction
      log en attente) ; les valeurs « big data » sont lues mais refusées en
      écriture (`ValueTooLarge`) plutôt que de produire une ruche douteuse.
- [x] **Tests** : validation croisée avec [`nt-hive`](https://crates.io/crates/nt-hive)
      (implémentation REGF indépendante) sur une vraie ruche BCD, y compris la
      recherche binaire des clés créées.

### Limites assumées

- Le repliage de casse des noms couvre le plan ASCII (suffisant pour les noms
  de registre standard) ; les noms non-ASCII sont comparés unité par unité.
- Les listes `ri` traversées lors d'une insertion sont réécrites en une liste
  feuille `lf` plate (valide jusqu'à 65 535 sous-clés) plutôt que rééquilibrées.
- L'écriture d'une valeur unique est plafonnée à une cellule (< 16 Kio) ;
  au-delà, `ValueTooLarge`.

## `no_std`

Le cœur ne dépend que de `core` + `alloc`. La feature `std` (par défaut) ajoute
`Hive::from_file` et `impl std::error::Error`.

```toml
# usage courant (std)
regf-rs = "0"
# contexte UEFI / embarqué
regf-rs = { version = "0", default-features = false, features = ["alloc"] }
```

## Licence

Sous double licence, au choix :

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- Licence MIT ([LICENSE-MIT](LICENSE-MIT))

Sauf mention contraire explicite, toute contribution soumise pour inclusion
dans ce dépôt est réputée sous cette double licence, sans clause additionnelle.
