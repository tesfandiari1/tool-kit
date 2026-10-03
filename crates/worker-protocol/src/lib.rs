//! Wire contracts for the workers the converter spawns.
//!
//! One module per engine, and they share no types. A PDF report carries an
//! inspection and a Vision report does not, and each engine pins its own
//! identity handshake, which is what rejects a stale worker binary. What they
//! share is a home. The audio worker is the third engine, and copies of these
//! conventions drifting apart across one file per engine is the failure this
//! crate prevents.
//!
//! serde is the only dependency, and it stays that way. Every worker binary
//! links this crate, so anything added here is linked into all of them.

pub mod anydoc;
pub mod audio;
pub mod pdf;
pub mod vision;

/// A SHA-256 digest as every party here writes it: 64 lowercase hex digits.
pub fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::is_lowercase_sha256;

    #[test]
    fn only_64_lowercase_hex_digits_are_a_digest() {
        assert!(is_lowercase_sha256(&"0f".repeat(32)));
        assert!(!is_lowercase_sha256(&"0F".repeat(32)));
        assert!(!is_lowercase_sha256(&"0f".repeat(31)));
        assert!(!is_lowercase_sha256(&"0g".repeat(32)));
    }
}
