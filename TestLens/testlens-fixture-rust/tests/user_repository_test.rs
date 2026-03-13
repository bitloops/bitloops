#[cfg(test)]
mod tests {
    use testlens_fixture_rust::models::user::User;
    use testlens_fixture_rust::repositories::user_repository::UserRepository;

    #[test]
    fn finds_user_by_id() {
        let mut repo = UserRepository::new();
        repo.save(User::new(7, "markos@bitloops.com".to_string(), "hash::secret".to_string()));

        let user = repo.find_by_id(7);

        assert!(user.is_some());
        assert_eq!(user.expect("missing user").email, "markos@bitloops.com");
    }
}
