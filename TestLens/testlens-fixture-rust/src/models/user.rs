#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: u32,
    pub email: String,
    pub password_hash: String,
}

impl User {
    pub fn new(id: u32, email: String, password_hash: String) -> Self {
        Self {
            id,
            email,
            password_hash,
        }
    }
}
