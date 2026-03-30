use super::*;

#[test]
fn startup_mode_fast_http_requires_loopback_host() {
    let config = DashboardServerConfig {
        host: None,
        port: 5667,
        no_open: true,
        force_http: true,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    };

    let err = select_startup_mode(&config, None, None).expect_err("must require loopback host");
    assert!(format!("{err:#}").contains("`--http`"));
    assert!(format!("{err:#}").contains("--host 127.0.0.1"));
}

#[test]
fn startup_mode_fast_http_accepts_explicit_loopback_host() {
    let config = DashboardServerConfig {
        host: Some("127.0.0.1".to_string()),
        port: 5667,
        no_open: true,
        force_http: true,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    };

    let mode = select_startup_mode(&config, None, Some("127.0.0.1")).expect("fast HTTP mode");
    assert_eq!(mode, DashboardStartupMode::FastHttpLoopback);
}

#[test]
fn startup_mode_uses_configured_https_fast_path() {
    let config = DashboardServerConfig {
        host: None,
        port: 5667,
        no_open: true,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    };
    let local_dashboard = crate::config::DashboardLocalDashboardConfig { tls: Some(true) };

    let mode =
        select_startup_mode(&config, Some(&local_dashboard), None).expect("configured https mode");
    assert_eq!(mode, DashboardStartupMode::FastConfiguredHttps);
}

#[test]
fn startup_mode_recheck_flag_forces_slow_probe() {
    let config = DashboardServerConfig {
        host: None,
        port: 5667,
        no_open: true,
        force_http: false,
        recheck_local_dashboard_net: true,
        bundle_dir: None,
    };
    let local_dashboard = crate::config::DashboardLocalDashboardConfig { tls: Some(true) };

    let mode = select_startup_mode(&config, Some(&local_dashboard), None).expect("slow probe");
    assert_eq!(mode, DashboardStartupMode::SlowProbe);
}

#[test]
fn default_bundle_dir_uses_cache_directory() {
    let path = default_bundle_dir_from_cache_dir(Some(Path::new("/tmp/cache/bitloops")));
    assert_eq!(path, PathBuf::from("/tmp/cache/bitloops/dashboard/bundle"));
}

#[test]
fn expand_tilde_replaces_user_home_prefix() {
    let expanded = expand_tilde_with_home(Path::new("~/bundle"), Some(Path::new("/tmp/home")));
    assert_eq!(expanded, PathBuf::from("/tmp/home/bundle"));
}

#[test]
fn resolve_bundle_file_rejects_parent_traversal() {
    let root = Path::new("/tmp/root");
    let resolved = resolve_bundle_file(root, "/../../etc/passwd");
    assert!(resolved.is_none());
}

#[test]
fn resolve_bundle_file_maps_root_to_index() {
    let root = Path::new("/tmp/root");
    let resolved = resolve_bundle_file(root, "/").expect("path should resolve");
    assert_eq!(resolved, PathBuf::from("/tmp/root/index.html"));
}

#[cfg(unix)]
#[tokio::test]
async fn bundle_request_does_not_follow_symlink_outside_bundle() {
    let bundle_dir = TempDir::new().expect("bundle temp dir");
    let outside_dir = TempDir::new().expect("outside temp dir");

    let secret = outside_dir.path().join("secret.txt");
    fs::write(&secret, "secret").expect("write secret");
    fs::write(bundle_dir.path().join("index.html"), "safe index").expect("write index");
    std::os::unix::fs::symlink(&secret, bundle_dir.path().join("leak.txt")).expect("symlink");

    let app = build_dashboard_router(test_state(
        bundle_dir.path().to_path_buf(),
        ServeMode::Bundle(bundle_dir.path().to_path_buf()),
        bundle_dir.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/leak.txt").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("safe index"));
    assert!(!body.contains("secret"));
}

#[cfg(unix)]
#[tokio::test]
async fn bundle_request_rejects_symlinked_index_outside_bundle() {
    let bundle_dir = TempDir::new().expect("bundle temp dir");
    let outside_dir = TempDir::new().expect("outside temp dir");

    let secret = outside_dir.path().join("secret.html");
    fs::write(&secret, "secret").expect("write secret");
    std::os::unix::fs::symlink(&secret, bundle_dir.path().join("index.html")).expect("symlink");

    let app = build_dashboard_router(test_state(
        bundle_dir.path().to_path_buf(),
        ServeMode::Bundle(bundle_dir.path().to_path_buf()),
        bundle_dir.path().to_path_buf(),
    ));

    let (status, body) = request_text(app, "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, "Bundle not found.\n");
}

#[test]
fn has_bundle_index_true_when_index_exists() {
    let temp = TempDir::new().expect("temp dir");
    std::fs::write(temp.path().join("index.html"), "ok").expect("write file");
    assert!(has_bundle_index(temp.path()));
}

#[test]
fn browser_host_uses_loopback_for_unspecified_ipv4_bind() {
    let host = browser_host_for_url(
        "0.0.0.0",
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 5667),
    );
    assert_eq!(host, "127.0.0.1");
}

#[test]
fn browser_host_uses_localhost_for_unspecified_ipv6_bind() {
    let host = browser_host_for_url(
        "::",
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 5667),
    );
    assert_eq!(host, "localhost");
}

#[test]
fn format_dashboard_url_wraps_ipv6_hosts() {
    assert_eq!(
        format_dashboard_url(DashboardTransport::Https, "::1", 5667),
        "https://[::1]:5667"
    );
    assert_eq!(
        format_dashboard_url(DashboardTransport::Http, "::1", 5667),
        "http://[::1]:5667"
    );
}

#[test]
fn warning_block_lines_plain_keeps_multiline_text_and_icon() {
    let rendered = warning_block_lines("Warning: first line\nsecond line", false);
    assert_eq!(rendered.first(), Some(&String::new()));
    assert_eq!(rendered.get(1).map(String::as_str), Some("  ⚠"));
    assert_eq!(
        rendered.get(2).map(String::as_str),
        Some("  Warning: first line")
    );
    assert_eq!(rendered.get(3).map(String::as_str), Some("  second line"));
    assert_eq!(rendered.last(), Some(&String::new()));
}

#[test]
fn warning_block_lines_colored_renders_padded_block_rows() {
    let rendered = warning_block_lines("abc\nde", true);
    assert_eq!(rendered.len(), 5, "top + icon + 2 text lines + bottom");
    assert!(rendered[0].contains("\x1b[30;48;2;107;79;59m"));
    assert!(rendered[1].contains("\x1b[33m⚠"));
    assert!(rendered[2].contains("  abc  "));
    assert!(rendered[3].contains("  de   "));
    assert!(rendered.iter().all(|line| line.contains("\x1b[K")));
}

#[test]
fn dashboard_user_uses_email_as_canonical_key() {
    let user = dashboard_user("Alice", "ALICE@Example.com");
    assert_eq!(user.key, "alice@example.com");
    assert_eq!(user.name, "Alice");
    assert_eq!(user.email, "alice@example.com");
}

#[test]
fn dashboard_user_falls_back_to_name_key_when_email_missing() {
    let user = dashboard_user("Alice Example", "");
    assert_eq!(user.key, "name:alice example");
    assert_eq!(user.name, "Alice Example");
    assert_eq!(user.email, "");
}

#[test]
fn canonical_agent_key_normalizes_to_kebab_case() {
    assert_eq!(canonical_agent_key("Claude Code"), "claude-code");
    assert_eq!(canonical_agent_key("Codex"), "codex");
    assert_eq!(canonical_agent_key("Gemini"), "gemini");
    assert_eq!(canonical_agent_key("cursor"), "cursor");
    assert_eq!(canonical_agent_key(""), "");
}

#[test]
fn branch_filter_excludes_internal_branches() {
    assert!(branch_is_excluded("bitloops/checkpoints/v1"));
    assert!(branch_is_excluded("bitloops/feature-shadow"));
    assert!(branch_is_excluded("origin/bitloops/feature-shadow"));
    assert!(branch_is_excluded(
        "refs/remotes/origin/bitloops/feature-shadow"
    ));
    assert!(branch_is_excluded("bitloops/legacy-shadow"));
    assert!(!branch_is_excluded("main"));
    assert!(!branch_is_excluded("origin/release/1.0"));
}

#[test]
fn build_branch_commit_log_args_uses_commit_time_range() {
    let args = build_branch_commit_log_args("main", Some(1700000000), Some(1700001000), 0);
    assert!(args.iter().any(|arg| arg == "--since=@1700000000"));
    assert!(args.iter().any(|arg| arg == "--until=@1700001000"));
    assert!(args.iter().any(|arg| arg == "main"));
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--max-count" && pair[1] == "1")
    );
}

#[test]
fn parse_branch_commit_log_skips_malformed_records_without_crashing() {
    let raw = format!(
        "abcd{f}parent{f}Alice{f}alice@example.com{f}1700000000{f}msg{f}aabbccddeeff{r}broken{r}",
        f = GIT_FIELD_SEPARATOR,
        r = GIT_RECORD_SEPARATOR
    );
    let parsed = parse_branch_commit_log(&raw);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].sha, "abcd");
    assert_eq!(parsed[0].checkpoint_id, "");
}

#[test]
fn parse_branch_commit_log_never_extracts_checkpoint_ids_from_git_log_records() {
    let raw = format!(
        "abcd{f}parent{f}Alice{f}alice@example.com{f}1700000000{f}msg{f}invalid-checkpoint{r}",
        f = GIT_FIELD_SEPARATOR,
        r = GIT_RECORD_SEPARATOR
    );
    let parsed = parse_branch_commit_log(&raw);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].checkpoint_id, "");
}

#[test]
fn paginate_clamps_limit_and_offset() {
    let page = ApiPage {
        limit: usize::MAX,
        offset: 3,
    };
    let items = vec![1, 2, 3, 4, 5, 6];
    let paged = paginate(&items, page);
    assert_eq!(paged, vec![4, 5, 6]);
}
