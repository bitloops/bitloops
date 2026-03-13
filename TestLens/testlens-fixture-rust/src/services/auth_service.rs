#[derive(Debug, Default, Clone, Copy)]
pub struct AuthService;

impl AuthService {
    pub fn hash_password(raw: &str) -> String {
        format!("hash::{}", raw.trim().to_lowercase())
    }

    pub fn verify_password(raw: &str, hash: &str) -> bool {
        Self::hash_password(raw) == hash
    }
}
