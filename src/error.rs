use alloc::string::String;
use core::fmt;

/// Résultat spécialisé du crate.
pub type Result<T> = core::result::Result<T, RegError>;

/// Erreurs de lecture/écriture d'une ruche REGF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegError {
    /// Signature de base block absente ("regf" attendu).
    BadSignature,
    /// Checksum d'en-tête invalide.
    BadChecksum { expected: u32, found: u32 },
    /// Buffer tronqué : lecture hors limites à l'offset donné.
    Truncated { offset: usize },
    /// Cellule incohérente (taille ou signature).
    CorruptCell { offset: usize },
    /// Clé absente au chemin demandé.
    KeyNotFound(String),
    /// Valeur absente sous la clé demandée.
    ValueNotFound(String),
    /// Écriture refusée : la ruche n'a pas été proprement fermée
    /// (numéros de séquence divergents, transaction log en attente).
    DirtyHive,
    /// Donnée de valeur trop grande pour l'implémentation d'écriture actuelle
    /// (les segments « big data » ne sont pas produits en écriture).
    ValueTooLarge { size: usize, max: usize },
}

impl fmt::Display for RegError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegError::BadSignature => write!(f, "signature REGF absente (\"regf\" attendu)"),
            RegError::BadChecksum { expected, found } => {
                write!(
                    f,
                    "checksum d'en-tête invalide : {expected:#010x} attendu, {found:#010x} lu"
                )
            }
            RegError::Truncated { offset } => write!(f, "buffer tronqué à l'offset {offset}"),
            RegError::CorruptCell { offset } => write!(f, "cellule corrompue à l'offset {offset}"),
            RegError::KeyNotFound(p) => write!(f, "clé introuvable : {p}"),
            RegError::ValueNotFound(v) => write!(f, "valeur introuvable : {v}"),
            RegError::DirtyHive => write!(
                f,
                "ruche non réconciliée (transaction log en attente) : écriture refusée"
            ),
            RegError::ValueTooLarge { size, max } => {
                write!(
                    f,
                    "valeur de {size} octets trop grande (max {max} en écriture)"
                )
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for RegError {}
