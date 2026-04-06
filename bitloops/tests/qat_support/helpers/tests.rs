use super::*;

#[test]
fn sanitize_name_normalizes_user_input() {
    assert_eq!(
        sanitize_name("BDD Foundation: Stores"),
        "bdd-foundation-stores"
    );
    assert_eq!(sanitize_name(" Already__Slugged "), "already-slugged");
}

#[test]
fn git_date_for_relative_day_uses_stable_noon_timestamp() {
    let today = git_date_for_relative_day(0).expect("today git date");
    let yesterday = git_date_for_relative_day(1).expect("yesterday git date");

    assert!(today.ends_with('Z') || today.contains("+00:00"));
    assert!(yesterday.ends_with('Z') || yesterday.contains("+00:00"));
    assert_ne!(today[..10].to_string(), yesterday[..10].to_string());
    assert!(today.contains("12:00:00"));
    assert!(yesterday.contains("12:00:00"));
}

#[test]
fn offline_vite_scaffold_writes_expected_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    create_offline_vite_react_ts_scaffold(dir.path()).expect("create scaffold");

    assert!(dir.path().join("my-app").join("package.json").exists());
    assert!(dir.path().join("my-app").join("index.html").exists());
    assert!(
        dir.path()
            .join("my-app")
            .join("src")
            .join("App.tsx")
            .exists()
    );
    assert!(
        dir.path()
            .join("my-app")
            .join("src")
            .join("main.tsx")
            .exists()
    );
}

#[test]
fn shell_single_quote_escapes_single_quotes() {
    assert_eq!(shell_single_quote("plain"), "'plain'");
    assert_eq!(shell_single_quote("it's ok"), "'it'\"'\"'s ok'");
}

#[test]
fn parse_timeout_seconds_uses_default_for_invalid_values() {
    assert_eq!(
        parse_timeout_seconds(None, 120).as_secs(),
        120,
        "missing value should use default"
    );
    assert_eq!(
        parse_timeout_seconds(Some(""), 120).as_secs(),
        120,
        "empty value should use default"
    );
    assert_eq!(
        parse_timeout_seconds(Some("0"), 120).as_secs(),
        120,
        "zero should use default"
    );
    assert_eq!(
        parse_timeout_seconds(Some("abc"), 120).as_secs(),
        120,
        "non-numeric should use default"
    );
}

#[test]
fn parse_timeout_seconds_accepts_positive_seconds() {
    assert_eq!(
        parse_timeout_seconds(Some("5"), 120).as_secs(),
        5,
        "positive value should be used"
    );
}

#[test]
fn parse_claude_auth_logged_in_reads_boolean_field() {
    let logged_in = r#"{"loggedIn":true,"authMethod":"oauth"}"#;
    let logged_out = r#"{"loggedIn":false,"authMethod":"none"}"#;

    assert_eq!(parse_claude_auth_logged_in(logged_in), Some(true));
    assert_eq!(parse_claude_auth_logged_in(logged_out), Some(false));
}

#[test]
fn text_has_claude_auth_failure_detects_auth_prompts() {
    assert!(text_has_claude_auth_failure(
        "Not logged in · Please run /login"
    ));
    assert!(text_has_claude_auth_failure("Authentication required"));
    assert!(!text_has_claude_auth_failure("all good"));
}

#[test]
fn build_init_bitloops_args_supports_no_sync_choice() {
    let args = build_init_bitloops_args("claude-code", false, None);
    assert_eq!(args, vec!["init", "--agent", "claude-code"]);
}

#[test]
fn build_init_bitloops_args_supports_sync_false_choice() {
    let args = build_init_bitloops_args("claude-code", false, Some(false));
    assert_eq!(args, vec!["init", "--agent", "claude-code", "--sync=false"]);
}

#[test]
fn build_init_bitloops_args_supports_sync_true_choice_and_force() {
    let args = build_init_bitloops_args("codex", true, Some(true));
    assert_eq!(
        args,
        vec!["init", "--agent", "codex", "--sync=true", "--force"]
    );
}
