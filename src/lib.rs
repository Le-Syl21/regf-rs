//! `regf-rs` — a pure-Rust reader and in-place writer for Windows Registry
//! hive files (REGF format).
//!
//! The core depends only on `core` + `alloc`, so it works in `no_std`
//! (UEFI applications, bootloaders, embedded systems). The `std` feature
//! (enabled by default) adds OS conveniences: file I/O and
//! `impl std::error::Error`.
//!
//! # Write safety
//! Any modification is rejected on a "dirty" hive (a pending transaction log,
//! see [`Hive::is_dirty`]). Values larger than a single cell ("big data") are
//! read but not produced on write: [`RegError::ValueTooLarge`] is returned
//! instead of writing a corrupt hive.
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
