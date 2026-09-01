//! `regf-rs` — lecteur et écrivain in-place de ruches de registre Windows
//! (format REGF), en Rust pur.
//!
//! Le cœur ne dépend que de `core` + `alloc` et fonctionne en `no_std`
//! (applications UEFI, bootloaders, embarqué). La feature `std` (activée par
//! défaut) ajoute les conforts nécessitant un OS : lecture/écriture de fichiers
//! et `impl std::error::Error`.
//!
//! # Sécurité des écritures
//! Toute modification est refusée sur une ruche « sale » (transaction log en
//! attente, cf. [`Hive::is_dirty`]). Les valeurs dépassant la limite d'une
//! cellule unique (« big data ») sont lues mais pas produites en écriture :
//! [`RegError::ValueTooLarge`] est renvoyée plutôt qu'une ruche corrompue.
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

mod cell;
mod error;
mod hbin;
mod header;
mod hive;
mod name;
mod value;

pub use error::{RegError, Result};
pub use header::Header;
pub use hive::{Hive, KeyNode};
pub use value::{RegType, RegValue};
