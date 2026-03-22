//! Checkpoint ID validation and constants.

/// Canonical key name used in `CommitNode.checkpoints` HashMaps for checkpoint identification.
/// This is NOT a git commit message element — checkpoint↔commit mappings live in the `commit_checkpoints`
/// relational table.
pub const CHECKPOINT_TRAILER_KEY: &str = "Bitloops-Checkpoint";
pub const CHECKPOINT_ID_PATTERN: &str = "[0-9a-f]{12}";
pub const SHORT_ID_LENGTH: usize = 12;

pub fn is_valid_checkpoint_id(value: &str) -> bool {
    if value.len() != SHORT_ID_LENGTH {
        return false;
    }
    value
        .as_bytes()
        .iter()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_checkpoint_id() {
        assert!(is_valid_checkpoint_id("a1b2c3d4e5f6"));
    }

    #[test]
    fn rejects_too_short() {
        assert!(!is_valid_checkpoint_id("abc123"));
    }

    #[test]
    fn rejects_uppercase() {
        assert!(!is_valid_checkpoint_id("A1B2C3D4E5F6"));
    }

    #[test]
    fn rejects_invalid_chars() {
        assert!(!is_valid_checkpoint_id("a1b2c3d4e5gg"));
    }
}
