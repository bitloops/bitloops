#[cfg(test)]
mod tests {
    use testlens_fixture_rust::repositories::user_repository::UserRepository;
    use testlens_fixture_rust::services::user_service::UserService;

    #[test]
    fn user_signup_and_login_flow() {
        let mut repo = UserRepository::new();

        UserService::create_user(&mut repo, 9, "flow@bitloops.com", "Pass123");

        let reloaded = repo.find_by_id(9);
        let authenticated = UserService::authenticate(&repo, "flow@bitloops.com", "Pass123");

        assert!(reloaded.is_some());
        assert!(authenticated);
    }
}
