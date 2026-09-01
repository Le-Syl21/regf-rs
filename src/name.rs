//! Key-name comparison and hashing following Windows rules.
//!
//! Registry key names are **case-insensitive**. Windows orders subkey lists
//! (`lf`/`lh`) by case-folded name and binary-searches them: a mis-ordered
//! list makes keys unfindable on the Windows side. This module provides the
//! reference ordering and hashing.
//!
//! Case folding is applied per UTF-16 code unit. For the ASCII plane (which
//! covers every standard registry name, BCD included) it exactly matches
//! `RtlUpcaseUnicodeString`. Beyond that, non-ASCII code units are compared
//! as-is; this is a documented, accepted limitation, consistent with the `lh`
//! hash below.

use core::cmp::Ordering;

/// Folds a UTF-16 code unit to uppercase (ASCII plane).
#[inline]
fn upcase(u: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&u) {
        u - 0x20
    } else {
        u
    }
}

/// Compares two names the way Windows does: UTF-16 code unit by code unit,
/// after case folding. `a` is a Rust string, `b` an iterator of UTF-16 code
/// units (a stored name's representation).
pub fn cmp_name(a: &str, b_units: impl Iterator<Item = u16>) -> Ordering {
    let mut ai = a.encode_utf16();
    let mut bi = b_units;
    loop {
        match (ai.next(), bi.next()) {
            (Some(x), Some(y)) => {
                let (x, y) = (upcase(x), upcase(y));
                if x != y {
                    return x.cmp(&y);
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => return Ordering::Equal,
        }
    }
}

/// Compares two Rust names with each other (same rule).
pub fn cmp_str(a: &str, b: &str) -> Ordering {
    cmp_name(a, b.encode_utf16())
}

/// Case-insensitive equality under the same rule.
pub fn eq_name(a: &str, b: &str) -> bool {
    cmp_str(a, b) == Ordering::Equal
}

/// `lh` hash of a name: `hash = hash * 37 + upcase(byte)` over the name's
/// uppercased ASCII bytes. This is the hash stored in `lh` entries, which
/// Windows compares before the name during binary search; it must therefore
/// be exact.
pub fn lh_hash(name: &str) -> u32 {
    let mut hash: u32 = 0;
    for &b in name.as_bytes() {
        hash = hash
            .wrapping_mul(37)
            .wrapping_add(b.to_ascii_uppercase() as u32);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec::Vec;

    #[test]
    fn case_insensitive_equality() {
        assert!(eq_name("Elements", "elements"));
        assert!(eq_name("24000002", "24000002"));
        assert!(!eq_name("Foo", "Bar"));
    }

    #[test]
    fn ordering_matches_ascii_upcase() {
        assert_eq!(cmp_str("A", "b"), core::cmp::Ordering::Less);
        assert_eq!(cmp_str("a", "B"), core::cmp::Ordering::Less);
        // shorter prefix < longer
        assert_eq!(cmp_str("ab", "abc"), core::cmp::Ordering::Less);
    }

    #[test]
    fn sort_is_stable_and_correct() {
        let mut v = ["24000010", "23000003", "24000001", "24000002"].to_vec();
        v.sort_by(|a, b| cmp_str(a, b));
        assert_eq!(v, ["23000003", "24000001", "24000002", "24000010"]);
    }

    #[test]
    fn hash_is_case_insensitive() {
        assert_eq!(lh_hash("abc"), lh_hash("ABC"));
        assert_ne!(lh_hash("abc"), lh_hash("abd"));
        // Non-zero for a non-empty entry.
        let _: Vec<u32> = ["a", "bb"].iter().map(|s| lh_hash(s)).collect();
        assert_ne!(lh_hash("Element"), 0);
        assert_eq!("".to_string().len(), 0); // keeps alloc used
    }
}
