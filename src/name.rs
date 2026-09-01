//! Comparaison et hachage des noms de clés selon les règles Windows.
//!
//! Les noms de clés du registre sont **insensibles à la casse**. Windows
//! ordonne les listes de sous-clés (`lf`/`lh`) selon la casse-repliée, et
//! effectue une recherche binaire dessus : une liste mal ordonnée rend des
//! clés introuvables côté Windows. Ce module fournit l'ordre et le hachage
//! de référence.
//!
//! Le repliage de casse est appliqué unité de code UTF-16 par unité de code.
//! Pour le plan ASCII (couvrant la totalité des noms de registre standard,
//! BCD compris) il reproduit exactement `RtlUpcaseUnicodeString`. Au-delà,
//! les unités de code non-ASCII sont comparées telles quelles ; c'est une
//! limite assumée et documentée, cohérente avec le hachage `lh` ci-dessous.

use core::cmp::Ordering;

/// Replie une unité de code UTF-16 en majuscule (plan ASCII).
#[inline]
fn upcase(u: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&u) {
        u - 0x20
    } else {
        u
    }
}

/// Compare deux noms comme le fait Windows : unité UTF-16 par unité,
/// après repliage de casse. `a` est une chaîne Rust, `b` un itérateur
/// d'unités de code UTF-16 (représentation d'un nom stocké).
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

/// Compare deux noms Rust entre eux (même règle).
pub fn cmp_str(a: &str, b: &str) -> Ordering {
    cmp_name(a, b.encode_utf16())
}

/// Égalité insensible à la casse selon la même règle.
pub fn eq_name(a: &str, b: &str) -> bool {
    cmp_str(a, b) == Ordering::Equal
}

/// Hachage `lh` d'un nom : `hash = hash * 37 + upcase(octet)` sur les octets
/// ASCII majuscules du nom. C'est le hachage stocké dans les entrées `lh`,
/// que Windows compare avant le nom lors de la recherche binaire ; il doit
/// donc être exact.
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
        // prefixe plus court < plus long
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
        // Non nul pour une entrée non vide.
        let _: Vec<u32> = ["a", "bb"].iter().map(|s| lh_hash(s)).collect();
        assert_ne!(lh_hash("Element"), 0);
        assert_eq!("".to_string().len(), 0); // garde alloc utilisé
    }
}
