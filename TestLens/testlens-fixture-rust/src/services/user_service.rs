use crate::models::user::User;
use crate::repositories::user_repository::UserRepository;
use crate::services::auth_service::AuthService;

#[derive(Debug, Default, Clone, Copy)]
pub struct UserService;

impl UserService {
    pub fn create_user(repo: &mut UserRepository, id: u32, email: &str, raw_password: &str) -> User {
        let password_hash = AuthService::hash_password(raw_password);
        let user = User::new(id, email.to_string(), password_hash);
        repo.save(user.clone());
        user
    }

    pub fn authenticate(repo: &UserRepository, email: &str, raw_password: &str) -> bool {
        if let Some(user) = repo.find_by_email(email) {
            return AuthService::verify_password(raw_password, &user.password_hash);
        }
        false
    }
}
