//! Binary byte units shared by renderer memory code.

/// Bytes in one mebibyte.
pub const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Bytes in one gibibyte.
pub const BYTES_PER_GIB: u64 = 1024 * BYTES_PER_MIB;

/// Converts mebibytes to bytes, saturating on overflow.
#[must_use]
pub const fn mib_to_bytes(mib: u64) -> u64 {
    mib.saturating_mul(BYTES_PER_MIB)
}

/// Converts gibibytes to bytes, saturating on overflow.
#[must_use]
pub const fn gib_to_bytes(gib: u64) -> u64 {
    gib.saturating_mul(BYTES_PER_GIB)
}

/// Converts bytes to mebibytes.
#[must_use]
pub fn bytes_to_mib(bytes: u64) -> f64 {
    bytes as f64 / BYTES_PER_MIB as f64
}

/// Converts bytes to gibibytes.
#[must_use]
pub fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / BYTES_PER_GIB as f64
}
