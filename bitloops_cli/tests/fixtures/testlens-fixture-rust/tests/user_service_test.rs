#[cfg(test)]
mod tests {
    use testlens_fixture_rust::repositories::user_repository::UserRepository;
    use testlens_fixture_rust::services::user_service::UserService;

    #[test]
    fn creates_and_authenticates_user() {
        let mut repo = UserRepository::new();

        let created = UserService::create_user(&mut repo, 1, "admin@bitloops.com", "Secret123");
        let can_auth = UserService::authenticate(&repo, "admin@bitloops.com", "Secret123");

        assert_eq!(created.id, 1);
        assert!(can_auth);
    }
}
