use super::*;
use crate::host::checkpoints::session::state::PendingCheckpointState;

#[test]
pub(crate) fn hash_worktree_id_is_six_chars() {
    for worktree_id in ["", "test-123", "feature/auth-system"] {
        let got = sha256_hex(worktree_id.as_bytes());
        assert_eq!(
            got[..6].len(),
            6,
            "hash prefix should be 6 chars for {worktree_id:?}"
        );
    }
}

#[test]
pub(crate) fn hash_worktree_id_is_deterministic() {
    let id = "test-worktree";
    let h1 = sha256_hex(id.as_bytes());
    let h2 = sha256_hex(id.as_bytes());
    assert_eq!(h1[..6], h2[..6], "hash prefix should be deterministic");
}

#[test]
pub(crate) fn hash_worktree_id_differs_for_different_inputs() {
    let h1 = sha256_hex("worktree-a".as_bytes());
    let h2 = sha256_hex("worktree-b".as_bytes());
    assert_ne!(
        h1[..6],
        h2[..6],
        "different worktrees should hash differently"
    );
}

#[test]
pub(crate) fn shadow_branch_name_for_commit() {
    let cases = [
        (
            "abc1234567890",
            "",
            format!(
                "refs/heads/bitloops/abc1234-{}",
                &sha256_hex("".as_bytes())[..6]
            ),
        ),
        (
            "abc1234567890",
            "test-123",
            format!(
                "refs/heads/bitloops/abc1234-{}",
                &sha256_hex("test-123".as_bytes())[..6]
            ),
        ),
        (
            "abc",
            "wt",
            format!(
                "refs/heads/bitloops/abc-{}",
                &sha256_hex("wt".as_bytes())[..6]
            ),
        ),
    ];

    for (base_commit, worktree_id, expected) in cases {
        let got = shadow_branch_ref(base_commit, worktree_id);
        assert_eq!(
            got, expected,
            "unexpected shadow branch for {base_commit}/{worktree_id}"
        );
    }
}

#[test]
pub(crate) fn parse_shadow_branch_name_cases() {
    let cases = [
        ("bitloops/abc1234-e3b0c4", "abc1234", "e3b0c4", true),
        ("bitloops/abc1234", "abc1234", "", true),
        (
            "bitloops/abcdef1234567890-fedcba",
            "abcdef1234567890",
            "fedcba",
            true,
        ),
        ("main", "", "", false),
        (paths::METADATA_BRANCH_NAME, "checkpoints/v1", "", true),
        ("bitloops/", "", "", true),
    ];

    for (branch, want_commit, want_worktree, want_ok) in cases {
        let (commit, worktree, ok) = parse_shadow_branch_name(branch);
        assert_eq!(ok, want_ok, "ok mismatch for {branch}");
        assert_eq!(commit, want_commit, "commit mismatch for {branch}");
        assert_eq!(worktree, want_worktree, "worktree mismatch for {branch}");
    }
}

#[test]
pub(crate) fn parse_shadow_branch_name_round_trip() {
    for (base_commit, worktree_id) in [
        ("abc1234567890", ""),
        ("abc1234567890", "test-worktree"),
        ("deadbeef", "feature/auth"),
    ] {
        let branch_name = shadow_branch_ref(base_commit, worktree_id);
        let (commit_prefix, worktree_hash, ok) = parse_shadow_branch_name(&branch_name);
        assert!(ok, "parse should succeed for {branch_name}");
        let expected_commit = if base_commit.len() > 7 {
            &base_commit[..7]
        } else {
            base_commit
        };
        assert_eq!(commit_prefix, expected_commit, "commit prefix mismatch");
        assert_eq!(worktree_hash, &sha256_hex(worktree_id.as_bytes())[..6]);
    }
}

#[test]
pub(crate) fn is_shadow_branch_cases() {
    let cases = [
        ("bitloops/abc1234", true),
        ("bitloops/1234567", true),
        ("bitloops/abcdef0123456789abcdef0123456789abcdef01", true),
        ("bitloops/AbCdEf1", true),
        ("bitloops/abc1234-e3b0c4", true),
        ("bitloops/1234567-123456", true),
        ("bitloops/abcdef0123456789-fedcba", true),
        ("bitloops/AbCdEf1-AbCdEf", true),
        ("bitloops/", false),
        ("bitloops/abc123", false),
        ("bitloops/a", false),
        ("bitloops/ghijklm", false),
        (paths::METADATA_BRANCH_NAME, false),
        ("abc1234", false),
        ("feature/abc1234", false),
        ("main", false),
        ("master", false),
        ("", false),
        ("bitloops", false),
        ("bitloops/abc1234-e3b0c", false),
        ("bitloops/abc1234-e3b0c44", false),
        ("bitloops/abc1234-ghijkl", false),
        ("bitloops/-e3b0c4", false),
    ];

    for (branch_name, want) in cases {
        let got = is_shadow_branch(branch_name);
        assert_eq!(got, want, "is_shadow_branch({branch_name:?})");
    }
}

#[test]
pub(crate) fn list_shadow_branches_filters_expected_refs() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);

    run_git(dir.path(), &["branch", "bitloops/abc1234-e3b0c4"]).unwrap();
    run_git(dir.path(), &["branch", "bitloops/def5678-f1e2d3"]).unwrap();
    run_git(dir.path(), &["branch", paths::METADATA_BRANCH_NAME]).unwrap();
    run_git(dir.path(), &["branch", "feature/foo"]).unwrap();

    let branches = list_shadow_branches(dir.path()).unwrap();
    assert_eq!(
        branches.len(),
        2,
        "unexpected shadow branches: {branches:?}"
    );
    assert!(branches.contains(&"bitloops/abc1234-e3b0c4".to_string()));
    assert!(branches.contains(&"bitloops/def5678-f1e2d3".to_string()));
    assert!(
        !branches.contains(&paths::METADATA_BRANCH_NAME.to_string()),
        "metadata branch must be excluded"
    );
}

#[test]
pub(crate) fn list_shadow_branches_empty() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);

    let branches = list_shadow_branches(dir.path()).unwrap();
    assert!(branches.is_empty(), "expected empty list, got {branches:?}");
}

#[test]
pub(crate) fn delete_shadow_branches_existing() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);
    run_git(dir.path(), &["branch", "bitloops/abc1234-e3b0c4"]).unwrap();
    run_git(dir.path(), &["branch", "bitloops/def5678-f1e2d3"]).unwrap();

    let input = vec![
        "bitloops/abc1234-e3b0c4".to_string(),
        "bitloops/def5678-f1e2d3".to_string(),
    ];
    let (deleted, failed) = delete_shadow_branches(dir.path(), &input);
    assert_eq!(deleted.len(), 2);
    assert!(failed.is_empty(), "failed branches: {failed:?}");

    let listed_a = run_git(dir.path(), &["branch", "--list", "bitloops/abc1234-e3b0c4"]).unwrap();
    let listed_b = run_git(dir.path(), &["branch", "--list", "bitloops/def5678-f1e2d3"]).unwrap();
    assert!(listed_a.is_empty(), "branch still exists: {listed_a:?}");
    assert!(listed_b.is_empty(), "branch still exists: {listed_b:?}");
}

#[test]
pub(crate) fn delete_shadow_branches_non_existent() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);

    let input = vec!["bitloops/doesnotexist-abc123".to_string()];
    let (deleted, failed) = delete_shadow_branches(dir.path(), &input);
    assert!(
        deleted.is_empty(),
        "deleted unexpected branches: {deleted:?}"
    );
    assert_eq!(failed.len(), 1, "failed branches: {failed:?}");
}

#[test]
pub(crate) fn delete_shadow_branches_empty() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);

    let (deleted, failed) = delete_shadow_branches(dir.path(), &[]);
    assert!(deleted.is_empty());
    assert!(failed.is_empty());
}

#[test]
pub(crate) fn list_orphaned_session_states_recent_session_not_orphaned() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    backend
        .save_session(&SessionState {
            session_id: "recent-session-123".to_string(),
            base_commit: head,
            started_at: now_secs.to_string(),
            pending: PendingCheckpointState::default(),
            ..Default::default()
        })
        .unwrap();

    let orphaned = list_orphaned_session_states(dir.path()).unwrap();
    assert!(
        !orphaned.iter().any(|item| item.id == "recent-session-123"),
        "recent session should not be marked orphaned: {orphaned:?}"
    );
}

#[test]
pub(crate) fn list_orphaned_session_states_shadow_branch_matching() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let old_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(3600);

    backend
        .save_session(&SessionState {
            session_id: "session-with-shadow-branch".to_string(),
            base_commit: head.clone(),
            worktree_id: "".to_string(),
            started_at: old_secs.to_string(),
            pending: PendingCheckpointState {
                step_count: 1,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();

    let shadow_ref = shadow_branch_ref(&head, "");
    run_git(dir.path(), &["update-ref", &shadow_ref, &head]).unwrap();

    let shadow_branches = list_shadow_branches(dir.path()).unwrap();
    let expected_short = shadow_ref.strip_prefix("refs/heads/").unwrap().to_string();
    assert!(
        shadow_branches.contains(&expected_short),
        "expected shadow branch not listed: {shadow_branches:?}"
    );

    let orphaned = list_orphaned_session_states(dir.path()).unwrap();
    assert!(
        !orphaned
            .iter()
            .any(|item| item.id == "session-with-shadow-branch"),
        "session with matching shadow branch should not be orphaned: {orphaned:?}"
    );
}
