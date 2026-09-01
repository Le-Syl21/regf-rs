use alloc::string::String;
use core::fmt;

/// Crate-specific result type.
pub type Result<T> = core::result::Result<T, RegError>;

/// Errors when reading or writing a REGF hive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegError {
    /// Base block signature missing ("regf" expected).
    BadSignature,
    /// Invalid header checksum.
    BadChecksum { expected: u32, found: u32 },
    /// Truncated buffer: out-of-bounds read at the given offset.
    Truncated { offset: usize },
    /// Inconsistent cell (size or signature).
    CorruptCell { offset: usize },
    /// Key missing at the requested path.
    KeyNotFound(String),
    /// Value missing under the requested key.
    ValueNotFound(String),
    /// Write rejected: the hive was not cleanly closed
    /// (diverging sequence numbers, pending transaction log).
    DirtyHive,
    /// Value data too large for the current write implementation
    /// ("big data" segments are not produced on write).
    ValueTooLarge { size: usize, max: usize },
}

impl fmt::Display for RegError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegError::BadSignature => write!(f, "missing REGF signature (expected \"regf\")"),
            RegError::BadChecksum { expected, found } => {
                write!(
                    f,
                    "invalid header checksum: expected {expected:#010x}, found {found:#010x}"
                )
            }
            RegError::Truncated { offset } => write!(f, "buffer truncated at offset {offset}"),
            RegError::CorruptCell { offset } => write!(f, "corrupt cell at offset {offset}"),
            RegError::KeyNotFound(p) => write!(f, "key not found: {p}"),
            RegError::ValueNotFound(v) => write!(f, "value not found: {v}"),
            RegError::DirtyHive => {
                write!(
                    f,
                    "unreconciled hive (pending transaction log): write rejected"
                )
            }
            RegError::ValueTooLarge { size, max } => {
                write!(f, "value of {size} bytes is too large (max {max} on write)")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for RegError {}
