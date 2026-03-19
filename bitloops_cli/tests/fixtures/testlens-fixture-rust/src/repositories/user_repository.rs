use crate::models::user::User;

#[derive(Debug, Default, Clone)]
pub struct UserRepository {
    users: Vec<User>,
}

impl UserRepository {
    pub fn new() -> Self {
        Self { users: Vec::new() }
    }

    pub fn save(&mut self, user: User) {
        self.users.push(user);
    }

    pub fn find_by_id(&self, id: u32) -> Option<User> {
        self.users.iter().find(|user| user.id == id).cloned()
    }

    pub fn find_by_email(&self, email: &str) -> Option<User> {
        self.users
            .iter()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .cloned()
    }
}
