use sha2::{Digest, Sha256};

pub(crate) fn deterministic_uuid(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hex::encode(hasher.finalize());

    let hex = &digest[..32];
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
