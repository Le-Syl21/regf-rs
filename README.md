# regf-rs

Pure-Rust reader and in-place writer for Windows Registry hive files
(**REGF format**) — no C dependency, `no_std`-friendly.

REGF is the format of registry hives: `SYSTEM`, `SOFTWARE`, `NTUSER.DAT`, and
the **BCD** (`\EFI\Microsoft\Boot\BCD`). The current Rust landscape has a gap:
`nt-hive` reads but does not write, `regf` only creates fresh hives, and the
complete solutions go through C bindings (`hivex`). `regf-rs` targets the
missing slot: **reading + in-place writing**, in pure Rust and cross-checked.

## Status

- [x] **Reading**: header + checksum, key navigation (`nk`), subkey lists
      (`lf`/`lh`/`li`/`ri`), values (`vk`) of every `REG_*` type, inline data
      and big data (`db`).
- [x] **In-place writing**: first-fit allocation with splitting, freeing with
      coalescing, **sorted insertion** into subkey lists (required by Windows'
      binary search), key creation (security inherited from the parent),
      setting/deleting values, checksum and sequence-number recomputation.
- [x] **Safety**: refuses to write to an unreconciled hive (pending transaction
      log); "big data" values are read but refused on write (`ValueTooLarge`)
      rather than producing a questionable hive.
- [x] **Tests**: cross-checked against
      [`nt-hive`](https://crates.io/crates/nt-hive) (an independent REGF
      implementation), including the binary search of created keys.

### Accepted limitations

- Name case folding covers the ASCII plane (enough for standard registry
  names); non-ASCII names are compared code unit by code unit.
- `ri` lists traversed during an insertion are rewritten as a single flat `lf`
  leaf list (valid up to 65,535 subkeys) rather than rebalanced.
- Writing a single value is capped at one cell (< 16 KiB); beyond that,
  `ValueTooLarge`.

## `no_std`

The core depends only on `core` + `alloc`. The `std` feature (default) adds
`Hive::from_file` and `impl std::error::Error`.

```toml
# common use (std)
regf-rs = "0"
# UEFI / embedded context
regf-rs = { version = "0", default-features = false, features = ["alloc"] }
```

## Tests

The suite is **self-contained**: it builds a synthetic BCD hive in memory
(`Hive::new_empty` + `create_key`/`set_value`) and cross-checks it against
[`nt-hive`](https://crates.io/crates/nt-hive). No personal hive is versioned.

```sh
cargo test                    # full suite (std)
cargo test --no-default-features --features alloc   # no_std core
```

To additionally validate against a real hive produced by Windows, point the
`REGF_TEST_HIVE` variable at a local file (never committed):

```sh
REGF_TEST_HIVE=/boot/efi/EFI/Microsoft/Boot/BCD cargo test --test real_hive
```

## License

Licensed under either of, at your option:

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository shall be dual-licensed as above, without any
additional terms or conditions.
