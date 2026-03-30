mod classification;
mod claude;
mod constants;
mod repo;
mod storage;
mod worktree;

pub use classification::{is_infrastructure_path, is_protected_path, to_relative_path};
pub use claude::{get_claude_project_dir, sanitize_path_for_claude};
pub use constants::{
    BITLOOPS_BLOB_STORE_DIR, BITLOOPS_DIR, BITLOOPS_EMBEDDING_MODELS_DIR, BITLOOPS_EMBEDDINGS_DIR,
    BITLOOPS_EVENT_STORE_DIR, BITLOOPS_METADATA_DIR, BITLOOPS_RELATIONAL_STORE_DIR,
    BITLOOPS_STORES_DIR, BITLOOPS_TMP_DIR, CHECKPOINT_FILE_NAME, CONTENT_HASH_FILE_NAME,
    CONTEXT_FILE_NAME, EVENTS_DB_FILE_NAME, EXPORT_DATA_FILE_NAME, METADATA_BRANCH_NAME,
    METADATA_FILE_NAME, PROMPT_FILE_NAME, RELATIONAL_DB_FILE_NAME, SETTINGS_FILE_NAME,
    SUMMARY_FILE_NAME, TRANSCRIPT_FILE_NAME, TRANSCRIPT_FILE_NAME_LEGACY,
};
pub use repo::{
    abs_path, bitloops_project_root, clear_repo_root_cache, open_repository, repo_root,
};
pub use storage::{
    default_blob_store_path, default_embedding_model_cache_dir, default_events_db_path,
    default_relational_db_path, default_runtime_state_dir, default_session_tmp_dir,
    extract_session_id_from_transcript_path, session_metadata_dir_from_session_id,
};
pub use worktree::{get_main_repo_root, get_worktree_id, is_inside_worktree};

#[cfg(test)]
mod tests {
    use super::{
        abs_path, bitloops_project_root, clear_repo_root_cache, default_blob_store_path,
        default_embedding_model_cache_dir, default_events_db_path, default_relational_db_path,
        extract_session_id_from_transcript_path, get_claude_project_dir, get_main_repo_root,
        get_worktree_id, is_infrastructure_path, is_inside_worktree, is_protected_path,
        open_repository, repo_root, sanitize_path_for_claude, session_metadata_dir_from_session_id,
        to_relative_path,
    };
    use crate::test_support::process_state::{with_cwd, with_env_var};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use tempfile::tempdir;

    fn init_git_repo(path: &Path) {
        let status = Command::new("git")
            .arg("init")
            .current_dir(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run git init");
        assert!(status.success(), "git init should succeed");
    }

    fn run_git(path: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("run git command");

        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_git_repo_with_commit(path: &Path) {
        run_git(path, &["init"]);
        run_git(path, &["config", "user.email", "test@example.com"]);
        run_git(path, &["config", "user.name", "Test User"]);
        fs::write(path.join("test.txt"), "test content\n").expect("write test file");
        run_git(path, &["add", "test.txt"]);
        run_git(path, &["commit", "-m", "Initial commit"]);
    }

    fn canonical(path: &Path) -> PathBuf {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    #[test]
    fn test_is_infrastructure_path() {
        let tests = [
            (".bitloops/metadata/test", true),
            (".bitloops", true),
            (".bitloops\\metadata\\test", true),
            (".bitloopsfile", false),
        ];

        for (path, want) in tests {
            let got = is_infrastructure_path(path);
            assert_eq!(
                got, want,
                "is_infrastructure_path({path:?}) = {got}, want {want}"
            );
        }
    }

    #[test]
    fn test_to_relative_path() {
        let cwd = "/repo";
        let tests = [
            ("/repo/src/main.rs", "src/main.rs"),
            ("/repo", "."),
            ("/other/place/file.txt", ""),
            ("relative/file.txt", "relative/file.txt"),
        ];

        for (input, want) in tests {
            let got = to_relative_path(input, cwd);
            assert_eq!(
                got, want,
                "to_relative_path({input:?}, {cwd:?}) = {got:?}, want {want:?}"
            );
        }
    }

    #[test]
    fn test_sanitize_path_for_claude() {
        let tests = [
            ("/Users/test/myrepo", "-Users-test-myrepo"),
            ("/home/user/project", "-home-user-project"),
            ("simple", "simple"),
            ("/path/with spaces/here", "-path-with-spaces-here"),
            ("/path.with.dots/file", "-path-with-dots-file"),
        ];

        for (input, want) in tests {
            let got = sanitize_path_for_claude(input);
            assert_eq!(
                got, want,
                "sanitize_path_for_claude({input:?}) = {got:?}, want {want:?}"
            );
        }
    }

    #[test]
    fn test_session_metadata_dir_from_session_id() {
        let got = session_metadata_dir_from_session_id("sess-123");
        assert_eq!(got, ".bitloops/metadata/sess-123");
    }

    #[test]
    fn test_default_bitloops_storage_paths_live_under_bitloops_directory() {
        let repo_root = Path::new("/repo");

        assert_eq!(
            default_relational_db_path(repo_root),
            PathBuf::from("/repo/.bitloops/stores/relational/relational.db")
        );
        assert_eq!(
            default_events_db_path(repo_root),
            PathBuf::from("/repo/.bitloops/stores/event/events.duckdb")
        );
        assert_eq!(
            default_blob_store_path(repo_root),
            PathBuf::from("/repo/.bitloops/stores/blob")
        );
        assert_eq!(
            default_embedding_model_cache_dir(repo_root),
            PathBuf::from("/repo/.bitloops/embeddings/models")
        );
    }

    #[test]
    fn test_extract_session_id_from_transcript_path() {
        let tests = [
            (
                "/Users/me/.claude/projects/repo/sessions/abc123.jsonl",
                "abc123",
            ),
            (
                r"C:\Users\me\.claude\projects\repo\sessions\xyz789.jsonl",
                "xyz789",
            ),
            ("/tmp/sessions/raw-id", "raw-id"),
            ("/tmp/no-session-here/file.jsonl", ""),
        ];

        for (input, want) in tests {
            let got = extract_session_id_from_transcript_path(input);
            assert_eq!(
                got, want,
                "extract_session_id_from_transcript_path({input:?}) = {got:?}, want {want:?}"
            );
        }
    }

    #[test]
    fn test_get_claude_project_dir_override() {
        let key = "BITLOOPS_TEST_CLAUDE_PROJECT_DIR";
        with_env_var(key, Some("/tmp/test-claude-project"), || {
            let result = get_claude_project_dir("/some/repo/path");
            let result = result.expect("get_claude_project_dir should return override path");
            assert_eq!(
                result,
                Path::new("/tmp/test-claude-project"),
                "get_claude_project_dir() = {:?}, want {:?}",
                result,
                Path::new("/tmp/test-claude-project")
            );
        });
    }

    #[test]
    fn test_get_claude_project_dir_default() {
        let key = "BITLOOPS_TEST_CLAUDE_PROJECT_DIR";
        with_env_var(key, Some(""), || {
            let result = get_claude_project_dir("/Users/test/myrepo");
            let result = result.expect("get_claude_project_dir should return default path");

            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .expect("home directory env var should be set");
            let expected = Path::new(&home)
                .join(".claude")
                .join("projects")
                .join("-Users-test-myrepo");

            assert_eq!(
                result, expected,
                "get_claude_project_dir() = {:?}, want {:?}",
                result, expected
            );
        });
    }

    #[test]
    fn test_get_worktree_id() {
        struct Case {
            name: &'static str,
            setup: fn(&Path),
            want_id: &'static str,
            want_err: bool,
            err_contains: &'static str,
        }

        fn setup_main_worktree(dir: &Path) {
            fs::create_dir_all(dir.join(".git")).expect("create .git directory");
        }

        fn setup_linked_simple(dir: &Path) {
            let content = "gitdir: /some/repo/.git/worktrees/test-wt\n";
            fs::write(dir.join(".git"), content).expect("write .git file");
        }

        fn setup_linked_nested(dir: &Path) {
            let content = "gitdir: /repo/.git/worktrees/feature/auth-system\n";
            fs::write(dir.join(".git"), content).expect("write .git file");
        }

        fn setup_no_git(_dir: &Path) {}

        fn setup_invalid_format(dir: &Path) {
            fs::write(dir.join(".git"), "invalid content").expect("write .git file");
        }

        fn setup_no_worktrees_marker(dir: &Path) {
            let content = "gitdir: /some/repo/.git\n";
            fs::write(dir.join(".git"), content).expect("write .git file");
        }

        let cases = [
            Case {
                name: "main worktree (git directory)",
                setup: setup_main_worktree,
                want_id: "",
                want_err: false,
                err_contains: "",
            },
            Case {
                name: "linked worktree simple name",
                setup: setup_linked_simple,
                want_id: "test-wt",
                want_err: false,
                err_contains: "",
            },
            Case {
                name: "linked worktree with subdirectory name",
                setup: setup_linked_nested,
                want_id: "feature/auth-system",
                want_err: false,
                err_contains: "",
            },
            Case {
                name: "no .git exists",
                setup: setup_no_git,
                want_id: "",
                want_err: true,
                err_contains: "failed to stat .git",
            },
            Case {
                name: "invalid .git file format",
                setup: setup_invalid_format,
                want_id: "",
                want_err: true,
                err_contains: "invalid .git file format",
            },
            Case {
                name: "gitdir without worktrees path",
                setup: setup_no_worktrees_marker,
                want_id: "",
                want_err: true,
                err_contains: "unexpected gitdir format",
            },
        ];

        for case in cases {
            let dir = tempdir().expect("create temp dir");
            (case.setup)(dir.path());

            let result = get_worktree_id(dir.path());

            if case.want_err {
                let err = result.expect_err("expected error");
                if !case.err_contains.is_empty() {
                    assert!(
                        err.to_string().contains(case.err_contains),
                        "get_worktree_id() error = {err}, want error containing {:?}",
                        case.err_contains
                    );
                }
                continue;
            }

            let id = result.expect("expected no error");
            assert_eq!(
                id, case.want_id,
                "get_worktree_id() = {id:?}, want {:?} (case: {})",
                case.want_id, case.name
            );
        }
    }

    #[test]
    fn test_open_repository() {
        let repo = tempdir().expect("create temp dir");
        init_git_repo_with_commit(repo.path());
        with_cwd(repo.path(), || {
            clear_repo_root_cache();

            let opened = open_repository().expect("open_repository should succeed");
            assert_eq!(
                canonical(&opened),
                canonical(repo.path()),
                "open_repository() should resolve current repo root"
            );

            let commit_message = run_git(&opened, &["log", "-1", "--pretty=%s"]);
            assert_eq!(
                commit_message, "Initial commit",
                "latest commit should remain readable after opening repo"
            );
        });
    }

    #[test]
    fn test_open_repository_error() {
        let non_repo = tempdir().expect("create temp dir");
        with_cwd(non_repo.path(), || {
            clear_repo_root_cache();

            let err = open_repository().expect_err("open_repository should fail outside git repo");
            assert!(
                err.to_string()
                    .contains("failed to get git repository root"),
                "open_repository() error = {err}, want git-root failure"
            );
        });
    }

    #[test]
    fn test_is_inside_worktree() {
        let main_repo = tempdir().expect("create temp dir");
        init_git_repo_with_commit(main_repo.path());
        with_cwd(main_repo.path(), || {
            assert!(
                !is_inside_worktree(),
                "main checkout should not be a worktree"
            );
        });

        let worktree_dir = main_repo.path().join("worktree");
        run_git(
            main_repo.path(),
            &[
                "worktree",
                "add",
                worktree_dir.to_string_lossy().as_ref(),
                "-b",
                "test-branch",
            ],
        );
        with_cwd(&worktree_dir, || {
            assert!(is_inside_worktree(), "linked checkout should be a worktree");
        });

        let non_repo = tempdir().expect("create non-repo dir");
        with_cwd(non_repo.path(), || {
            assert!(!is_inside_worktree(), "non-repo should not be a worktree");
        });
    }

    #[test]
    fn test_get_main_repo_root() {
        let main_repo = tempdir().expect("create temp dir");
        init_git_repo_with_commit(main_repo.path());
        with_cwd(main_repo.path(), || {
            let root = get_main_repo_root().expect("get_main_repo_root should work in main repo");
            assert_eq!(canonical(&root), canonical(main_repo.path()));
        });

        let worktree_dir = main_repo.path().join("worktree");
        run_git(
            main_repo.path(),
            &[
                "worktree",
                "add",
                worktree_dir.to_string_lossy().as_ref(),
                "-b",
                "test-branch",
            ],
        );
        with_cwd(&worktree_dir, || {
            let root =
                get_main_repo_root().expect("get_main_repo_root should resolve worktree parent");
            assert_eq!(canonical(&root), canonical(main_repo.path()));
        });
    }

    #[test]
    fn test_is_protected_path() {
        let cases = [
            (".git", true),
            (".git/objects", true),
            (".bitloops", true),
            (".bitloops/metadata/session.json", true),
            (".claude", true),
            (".claude/settings.json", true),
            (".github/hooks", true),
            (".github/hooks/bitloops.json", true),
            (".codex", true),
            (".codex/hooks.json", true),
            (".cursor", true),
            (".cursor/hooks.json", true),
            (".gemini", true),
            (".gemini/settings.json", true),
            ("src/main.rs", false),
            ("README.md", false),
            (".gitignore", false),
            (".github/workflows/ci.yml", false),
        ];

        for (path, want) in cases {
            let got = is_protected_path(path);
            assert_eq!(
                got, want,
                "is_protected_path({path:?}) = {got}, want {want}"
            );
        }
    }

    #[test]
    fn test_repo_root_and_abs_path() {
        let repo = tempdir().expect("create temp dir");
        init_git_repo(repo.path());
        with_cwd(repo.path(), || {
            clear_repo_root_cache();

            let root = repo_root().expect("resolve repo root");
            assert_eq!(
                canonical(&root),
                canonical(repo.path()),
                "repo_root() should return git top-level"
            );

            let joined = abs_path(".bitloops/settings.json").expect("resolve relative path");
            assert_eq!(
                joined,
                root.join(".bitloops/settings.json"),
                "abs_path should join with repo root"
            );

            let already_abs = repo.path().join("already-absolute.txt");
            let passthrough =
                abs_path(&already_abs.to_string_lossy()).expect("pass through absolute");
            assert_eq!(
                passthrough, already_abs,
                "abs_path should return absolute input unchanged"
            );
        });
    }

    #[test]
    fn test_repo_root_cache_is_per_cwd() {
        let repo1 = tempdir().expect("create repo1");
        let repo2 = tempdir().expect("create repo2");
        init_git_repo(repo1.path());
        init_git_repo(repo2.path());
        clear_repo_root_cache();

        let root1 = with_cwd(repo1.path(), || repo_root().expect("resolve repo1 root"));
        assert_eq!(canonical(&root1), canonical(repo1.path()));

        let root2 = with_cwd(repo2.path(), || repo_root().expect("resolve repo2 root"));
        assert_eq!(canonical(&root2), canonical(repo2.path()));
    }

    #[test]
    fn test_repo_root_errors_outside_git_repository() {
        let non_repo = tempdir().expect("create temp dir");
        with_cwd(non_repo.path(), || {
            clear_repo_root_cache();

            let err = repo_root().expect_err("repo_root should fail outside git repo");
            assert!(
                err.to_string()
                    .contains("failed to get git repository root"),
                "repo_root() error = {err}, want actionable git-root message"
            );

            let err = abs_path("relative/file.txt")
                .expect_err("abs_path should fail outside git repo for relative inputs");
            assert!(
                err.to_string()
                    .contains("failed to get git repository root"),
                "abs_path() error = {err}, want actionable git-root message"
            );
        });
    }

    #[test]
    fn monorepo_bitloops_project_root_finds_nearest_ancestor() {
        let root = tempdir().expect("create monorepo root");
        init_git_repo(root.path());

        let app_dir = root.path().join("packages/app");
        fs::create_dir_all(app_dir.join(".bitloops")).unwrap();

        let result = bitloops_project_root(&app_dir).unwrap();
        assert_eq!(
            canonical(&result),
            canonical(&app_dir),
            "should find nearest .bitloops at packages/app, not git root"
        );
    }

    #[test]
    fn monorepo_bitloops_project_root_falls_back_to_git_root() {
        let root = tempdir().expect("create monorepo root");
        init_git_repo(root.path());

        let lib_dir = root.path().join("packages/lib");
        fs::create_dir_all(&lib_dir).unwrap();

        let result = bitloops_project_root(&lib_dir).unwrap();
        assert_eq!(
            canonical(&result),
            canonical(root.path()),
            "should fall back to git root when no .bitloops marker"
        );
    }

    #[test]
    fn monorepo_bitloops_project_root_resolves_from_deep_subdirectory() {
        let root = tempdir().expect("create monorepo root");
        init_git_repo(root.path());

        let app_dir = root.path().join("packages/app");
        fs::create_dir_all(app_dir.join(".bitloops")).unwrap();
        let deep_dir = app_dir.join("src/components");
        fs::create_dir_all(&deep_dir).unwrap();

        let result = bitloops_project_root(&deep_dir).unwrap();
        assert_eq!(
            canonical(&result),
            canonical(&app_dir),
            "should resolve to packages/app from deep subdirectory"
        );
    }

    #[test]
    fn monorepo_bitloops_project_root_prefers_nearest_over_git_root() {
        let root = tempdir().expect("create monorepo root");
        init_git_repo(root.path());

        fs::create_dir_all(root.path().join(".bitloops")).unwrap();
        let app_dir = root.path().join("packages/app");
        fs::create_dir_all(app_dir.join(".bitloops")).unwrap();

        let result = bitloops_project_root(&app_dir).unwrap();
        assert_eq!(
            canonical(&result),
            canonical(&app_dir),
            "should prefer nearest .bitloops over git-root .bitloops"
        );
    }

    #[test]
    fn monorepo_git_root_unchanged_for_git_operations() {
        let root = tempdir().expect("create monorepo root");
        init_git_repo(root.path());

        let app_dir = root.path().join("packages/app");
        fs::create_dir_all(app_dir.join(".bitloops")).unwrap();

        with_cwd(&app_dir, || {
            clear_repo_root_cache();
            let git_root = repo_root().unwrap();
            assert_eq!(
                canonical(&git_root),
                canonical(root.path()),
                "repo_root() must still return git root, not bitloops project root"
            );
        });
    }

    #[test]
    fn monorepo_bitloops_project_root_single_repo_matches_git_root() {
        let root = tempdir().expect("create single repo");
        init_git_repo(root.path());

        fs::create_dir_all(root.path().join(".bitloops")).unwrap();

        let result = bitloops_project_root(root.path()).unwrap();
        assert_eq!(
            canonical(&result),
            canonical(root.path()),
            "in a single-package repo, bitloops project root equals git root"
        );
    }

    #[test]
    fn monorepo_bitloops_project_root_errors_outside_git_repo() {
        let non_repo = tempdir().expect("create non-repo dir");
        let err = bitloops_project_root(non_repo.path())
            .expect_err("should fail outside a git repository");
        assert!(
            err.to_string().contains("git repository"),
            "error should mention git repository: {err}"
        );
    }
}
