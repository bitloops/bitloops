use std::fs;
use std::path::PathBuf;

use tempfile::tempdir;

use super::{REPO_POLICY_FILE_NAME, REPO_POLICY_LOCAL_FILE_NAME, discover_repo_policy};

#[test]
fn discover_repo_policy_reads_local_daemon_binding() {
    let repo = tempdir().expect("temp dir");
    let local_policy = repo.path().join(REPO_POLICY_LOCAL_FILE_NAME);
    fs::write(
        &local_policy,
        r#"
[daemon]
config_path = "/tmp/daemon/config.toml"
"#,
    )
    .expect("write local repo policy");

    let snapshot = discover_repo_policy(repo.path()).expect("discover repo policy");

    assert_eq!(
        snapshot.daemon_config_path,
        Some(PathBuf::from("/tmp/daemon/config.toml"))
    );
}

#[test]
fn discover_repo_policy_rejects_shared_daemon_binding() {
    let repo = tempdir().expect("temp dir");
    let shared_policy = repo.path().join(REPO_POLICY_FILE_NAME);
    fs::write(
        &shared_policy,
        r#"
[daemon]
config_path = "/tmp/daemon/config.toml"
"#,
    )
    .expect("write shared repo policy");

    let err = discover_repo_policy(repo.path()).expect_err("shared daemon binding must fail");

    assert!(
        err.to_string()
            .contains("Bitloops daemon binding must be local-only"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn daemon_binding_does_not_change_repo_policy_fingerprint() {
    let repo = tempdir().expect("temp dir");
    let shared_policy = repo.path().join(REPO_POLICY_FILE_NAME);
    let local_policy = repo.path().join(REPO_POLICY_LOCAL_FILE_NAME);
    fs::write(
        &shared_policy,
        r#"
[capture]
enabled = true
"#,
    )
    .expect("write shared repo policy");
    fs::write(
        &local_policy,
        r#"
[daemon]
config_path = "/tmp/daemon-a/config.toml"
"#,
    )
    .expect("write first local repo policy");

    let first = discover_repo_policy(repo.path())
        .expect("discover first repo policy")
        .fingerprint;

    fs::write(
        &local_policy,
        r#"
[daemon]
config_path = "/tmp/daemon-b/config.toml"
"#,
    )
    .expect("write second local repo policy");

    let second = discover_repo_policy(repo.path())
        .expect("discover second repo policy")
        .fingerprint;

    assert_eq!(first, second);
}

#[test]
fn local_scope_exclusions_replace_shared_values() {
    let temp = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
    std::fs::write(
        temp.path().join(REPO_POLICY_FILE_NAME),
        r#"
[scope]
project_root = "packages/api"
include = ["src/**"]
exclude = ["dist/**"]
exclude_from = ["shared.ignore"]
"#,
    )
    .expect("write shared policy");
    std::fs::write(
        temp.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[scope]
exclude_from = ["local.ignore"]
"#,
    )
    .expect("write local policy");
    std::fs::write(temp.path().join("shared.ignore"), "vendor/**\n").expect("write shared");
    std::fs::write(temp.path().join("local.ignore"), "tmp/**\n").expect("write local");

    let snapshot = discover_repo_policy(temp.path()).expect("discover policy");
    let scope = snapshot.scope.as_object().expect("scope object");
    assert_eq!(
        scope.get("include"),
        Some(&serde_json::json!(["src/**"])),
        "non-exclusion keys should still inherit from shared"
    );
    assert!(
        scope.get("exclude").is_none(),
        "shared scope.exclude should be cleared when local defines exclusion keys"
    );
    assert_eq!(
        scope.get("exclude_from"),
        Some(&serde_json::json!(["local.ignore"]))
    );
}

#[test]
fn shared_scope_exclusions_apply_when_local_exclusion_keys_absent() {
    let temp = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
    std::fs::write(
        temp.path().join(REPO_POLICY_FILE_NAME),
        r#"
[scope]
exclude = ["dist/**"]
exclude_from = ["shared.ignore"]
"#,
    )
    .expect("write shared policy");
    std::fs::write(
        temp.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[scope]
project_root = "packages/app"
"#,
    )
    .expect("write local policy");
    std::fs::write(temp.path().join("shared.ignore"), "vendor/**\n").expect("write shared");

    let snapshot = discover_repo_policy(temp.path()).expect("discover policy");
    let scope = snapshot.scope.as_object().expect("scope object");
    assert_eq!(scope.get("exclude"), Some(&serde_json::json!(["dist/**"])));
    assert_eq!(
        scope.get("exclude_from"),
        Some(&serde_json::json!(["shared.ignore"]))
    );
}

#[test]
fn policy_fingerprint_changes_when_exclude_from_file_content_changes() {
    let temp = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
    std::fs::write(
        temp.path().join(REPO_POLICY_FILE_NAME),
        r#"
[scope]
exclude_from = [".bitloopsignore"]
"#,
    )
    .expect("write shared policy");
    let ignore_path = temp.path().join(".bitloopsignore");
    std::fs::write(&ignore_path, "vendor/**\n").expect("write ignore");
    let first = discover_repo_policy(temp.path())
        .expect("discover policy")
        .fingerprint;

    std::fs::write(&ignore_path, "vendor/**\nbuild/**\n").expect("rewrite ignore");
    let second = discover_repo_policy(temp.path())
        .expect("discover policy")
        .fingerprint;
    assert_ne!(first, second);
}

#[test]
fn exclude_from_paths_outside_policy_root_are_rejected() {
    let temp = tempfile::tempdir().expect("temp dir");
    let outside = tempfile::tempdir().expect("outside temp dir");
    std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
    std::fs::write(outside.path().join("outside.ignore"), "vendor/**\n")
        .expect("write outside ignore");
    std::fs::write(
        temp.path().join(REPO_POLICY_FILE_NAME),
        format!(
            r#"
[scope]
exclude_from = ["{}"]
"#,
            outside.path().join("outside.ignore").display()
        ),
    )
    .expect("write policy");

    let err = discover_repo_policy(temp.path()).expect_err("outside-root paths should fail");
    assert!(
        err.to_string().contains("outside repo-policy root"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn discover_policy_accepts_unquoted_scope_exclusion_array_values() {
    let temp = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(temp.path().join(".git")).expect("create .git");
    std::fs::write(
        temp.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[scope]
exclude = [docs/**]
exclude_from = [.bitloopsignore]
"#,
    )
    .expect("write local policy");
    std::fs::write(temp.path().join(".bitloopsignore"), "vendor/**\n").expect("write ignore file");

    let snapshot = discover_repo_policy(temp.path()).expect("discover policy");
    let scope = snapshot.scope.as_object().expect("scope object");
    assert_eq!(scope.get("exclude"), Some(&serde_json::json!(["docs/**"])));
    assert_eq!(
        scope.get("exclude_from"),
        Some(&serde_json::json!([".bitloopsignore"]))
    );
}
