use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tempfile::tempdir;

pub(super) async fn sqlite_relational_store_with_sync_schema(
    path: &Path,
) -> crate::host::devql::RelationalStorage {
    crate::host::devql::init_sqlite_schema(path)
        .await
        .expect("initialise sqlite relational schema");
    crate::host::devql::RelationalStorage::local_only(path.to_path_buf())
}

pub(super) async fn seed_sync_repository_catalog_row(
    relational: &crate::host::devql::RelationalStorage,
    cfg: &crate::host::devql::DevqlConfig,
) {
    relational
        .exec(&format!(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
             VALUES ('{}', '{}', '{}', '{}', 'main') \
             ON CONFLICT(repo_id) DO UPDATE SET \
               provider = excluded.provider, \
               organization = excluded.organization, \
               name = excluded.name, \
               default_branch = excluded.default_branch",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.provider),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.organization),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.name),
        ))
        .await
        .expect("seed sync repository catalog row");
}

fn isolated_test_repo_root() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("bitloops-devql-sync-test-{id}"))
}

pub(super) fn sync_test_cfg() -> crate::host::devql::DevqlConfig {
    let repo_root = isolated_test_repo_root();
    crate::host::devql::DevqlConfig {
        daemon_config_root: repo_root.clone(),
        repo_root,
        repo: crate::host::devql::RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "temp2".to_string(),
            identity: "github/bitloops/temp2".to_string(),
            repo_id: crate::host::devql::deterministic_uuid("repo://github/bitloops/temp2"),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
    }
}

pub(super) fn sync_test_cfg_for_repo(repo_root: &Path) -> crate::host::devql::DevqlConfig {
    crate::host::devql::DevqlConfig {
        daemon_config_root: repo_root.to_path_buf(),
        repo_root: repo_root.to_path_buf(),
        repo: crate::host::devql::RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "sync-task-10".to_string(),
            identity: "github/bitloops/sync-task-10".to_string(),
            repo_id: crate::host::devql::deterministic_uuid(&format!(
                "repo://{}",
                repo_root.display()
            )),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
    }
}

pub(super) fn desired_file_state(
    path: &str,
    language: &str,
    content_id: &str,
) -> crate::host::devql::sync::types::DesiredFileState {
    crate::host::devql::sync::types::DesiredFileState {
        path: path.to_string(),
        language: language.to_string(),
        head_content_id: Some(content_id.to_string()),
        index_content_id: Some(content_id.to_string()),
        worktree_content_id: Some(content_id.to_string()),
        effective_content_id: content_id.to_string(),
        effective_source: crate::host::devql::sync::types::EffectiveSource::Head,
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    }
}

pub(super) fn expected_symbol_id_by_fqn(
    items: &[crate::host::language_adapter::LanguageArtefact],
    path: &str,
) -> std::collections::HashMap<String, String> {
    let mut symbol_ids = std::collections::HashMap::from([(
        path.to_string(),
        crate::host::devql::file_symbol_id(path),
    )]);

    for item in items {
        let parent_symbol_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_ids.get(fqn))
            .map(String::as_str);
        let symbol_id =
            crate::host::devql::structural_symbol_id_for_artefact(item, parent_symbol_id);
        symbol_ids.insert(item.symbol_fqn.clone(), symbol_id);
    }

    symbol_ids
}

pub(super) fn seed_workspace_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n",
    )
    .expect("write rust source");
    fs::write(dir.path().join("README.md"), "# ignored\n").expect("write readme");

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}

pub(super) fn seed_full_sync_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::create_dir_all(dir.path().join("web")).expect("create web dir");
    fs::create_dir_all(dir.path().join("scripts")).expect("create scripts dir");

    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n",
    )
    .expect("write rust source");
    fs::write(
        dir.path().join("web/app.ts"),
        "import { helper } from \"./util\";\n\nexport function run(): number {\n  return helper();\n}\n",
    )
    .expect("write TypeScript source");
    fs::write(
        dir.path().join("web/util.js"),
        "export function helper() {\n  return 7;\n}\n",
    )
    .expect("write JavaScript source");
    fs::write(
        dir.path().join("scripts/main.py"),
        "def helper() -> int:\n    return 1\n\n\ndef run() -> int:\n    return helper()\n",
    )
    .expect("write Python source");
    fs::write(dir.path().join("README.md"), "# ignored\n").expect("write readme");

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}

pub(super) fn seed_supported_and_unsupported_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::create_dir_all(dir.path().join("docs")).expect("create docs dir");

    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n",
    )
    .expect("write supported source file");
    fs::write(dir.path().join("docs/notes.foo"), "ignored content\n")
        .expect("write unsupported source file");

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}
