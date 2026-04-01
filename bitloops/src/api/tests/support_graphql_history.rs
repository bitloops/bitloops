use super::*;

pub(super) struct SeededGraphqlTemporalRepo {
    pub(super) repo: TempDir,
    pub(super) first_commit: String,
}

pub(super) fn seed_graphql_temporal_repo() -> SeededGraphqlTemporalRepo {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("packages/api/src")).expect("create api src dir");
    fs::create_dir_all(repo_root.join("packages/web/src")).expect("create web src dir");
    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "import { target } from \"./target\";\n\nexport function caller() {\n  return target();\n}\n",
    )
    .expect("write api caller v1");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function target() {\n  return 41;\n}\n",
    )
    .expect("write api target v1");
    fs::write(
        repo_root.join("packages/web/src/page.ts"),
        "export function render() {\n  return 1;\n}\n",
    )
    .expect("write web page");
    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed temporal GraphQL commit 1"],
    );
    let first_commit = git_ok(repo_root, &["rev-parse", "HEAD"]);

    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "import { render } from \"../../web/src/page\";\n\nexport function callerCurrent() {\n  return render();\n}\n",
    )
    .expect("write api caller v2");
    fs::remove_file(repo_root.join("packages/api/src/target.ts")).expect("remove api target");
    git_ok(repo_root, &["add", "-A"]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed temporal GraphQL commit 2"],
    );
    let second_commit = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let sqlite_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("graphql-temporal.sqlite");
    crate::storage::init::init_database(&sqlite_path, false, &second_commit)
        .expect("initialise GraphQL temporal sqlite store");
    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": sqlite_path.to_string_lossy()
                }
            }
        }),
    );

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open GraphQL temporal sqlite");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', ?2, 'main')",
        rusqlite::params![repo_id.as_str(), SEEDED_REPO_NAME],
    )
    .expect("insert repository row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed temporal GraphQL commit 1', '2026-03-25T09:00:00Z')",
        rusqlite::params![first_commit.as_str(), repo_id.as_str()],
    )
    .expect("insert first commit row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed temporal GraphQL commit 2', '2026-03-26T09:00:00Z')",
        rusqlite::params![second_commit.as_str(), repo_id.as_str()],
    )
    .expect("insert second commit row");

    for (commit_sha, path, blob_sha) in [
        (
            first_commit.as_str(),
            "packages/api/src/caller.ts",
            "blob-api-caller-v1",
        ),
        (
            first_commit.as_str(),
            "packages/api/src/target.ts",
            "blob-api-target-v1",
        ),
        (
            first_commit.as_str(),
            "packages/web/src/page.ts",
            "blob-web-page-v1",
        ),
        (
            second_commit.as_str(),
            "packages/api/src/caller.ts",
            "blob-api-caller-v2",
        ),
        (
            second_commit.as_str(),
            "packages/web/src/page.ts",
            "blob-web-page-v1",
        ),
    ] {
        conn.execute(
            "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![repo_id.as_str(), commit_sha, path, blob_sha],
        )
        .expect("insert file_state row");
    }

    for (path, blob_sha) in [
        ("packages/api/src/caller.ts", "blob-api-caller-v2"),
        ("packages/web/src/page.ts", "blob-web-page-v1"),
    ] {
        conn.execute(
            "INSERT INTO current_file_state (
                repo_id, path, language,
                head_content_id, index_content_id, worktree_content_id,
                effective_content_id, effective_source,
                parser_version, extractor_version,
                exists_in_head, exists_in_index, exists_in_worktree,
                last_synced_at
            ) VALUES (?1, ?2, 'typescript', ?3, ?3, ?3, ?3, 'head', 'test', 'test', 1, 1, 1, '2026-03-26T09:00:00Z')",
            rusqlite::params![repo_id.as_str(), path, blob_sha],
        )
        .expect("insert current_file_state row");
    }

    for (
        symbol_id,
        artefact_id,
        blob_sha,
        path,
        canonical_kind,
        language_kind,
        symbol_fqn,
        parent_artefact_id,
        start_line,
        end_line,
        created_at,
    ) in [
        (
            "file::v1-api-caller",
            "artefact::v1-file-api-caller",
            "blob-api-caller-v1",
            "packages/api/src/caller.ts",
            "file",
            "source_file",
            "packages/api/src/caller.ts",
            Option::<&str>::None,
            1_i64,
            5_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "sym::v1-api-caller",
            "artefact::v1-api-caller",
            "blob-api-caller-v1",
            "packages/api/src/caller.ts",
            "function",
            "function_declaration",
            "packages/api/src/caller.ts::caller",
            Some("artefact::v1-file-api-caller"),
            3_i64,
            5_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "file::v1-api-target",
            "artefact::v1-file-api-target",
            "blob-api-target-v1",
            "packages/api/src/target.ts",
            "file",
            "source_file",
            "packages/api/src/target.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "sym::v1-api-target",
            "artefact::v1-api-target",
            "blob-api-target-v1",
            "packages/api/src/target.ts",
            "function",
            "function_declaration",
            "packages/api/src/target.ts::target",
            Some("artefact::v1-file-api-target"),
            1_i64,
            3_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "file::v1-web-page",
            "artefact::v1-file-web-page",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "file",
            "source_file",
            "packages/web/src/page.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "sym::v1-web-render",
            "artefact::v1-web-render",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "function",
            "function_declaration",
            "packages/web/src/page.ts::render",
            Some("artefact::v1-file-web-page"),
            1_i64,
            3_i64,
            "2026-03-25T09:00:00Z",
        ),
        (
            "file::v2-api-caller",
            "artefact::v2-file-api-caller",
            "blob-api-caller-v2",
            "packages/api/src/caller.ts",
            "file",
            "source_file",
            "packages/api/src/caller.ts",
            Option::<&str>::None,
            1_i64,
            5_i64,
            "2026-03-26T09:00:00Z",
        ),
        (
            "sym::v2-api-caller",
            "artefact::v2-api-caller",
            "blob-api-caller-v2",
            "packages/api/src/caller.ts",
            "function",
            "function_declaration",
            "packages/api/src/caller.ts::callerCurrent",
            Some("artefact::v2-file-api-caller"),
            3_i64,
            5_i64,
            "2026-03-26T09:00:00Z",
        ),
        (
            "file::v2-web-page",
            "artefact::v2-file-web-page",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "file",
            "source_file",
            "packages/web/src/page.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
            "2026-03-26T09:00:00Z",
        ),
        (
            "sym::v2-web-render",
            "artefact::v2-web-render",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "function",
            "function_declaration",
            "packages/web/src/page.ts::render",
            Some("artefact::v2-file-web-page"),
            1_i64,
            3_i64,
            "2026-03-26T09:00:00Z",
        ),
    ] {
        conn.execute(
            "INSERT INTO artefacts (
                artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
                language_kind, symbol_fqn, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, content_hash, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript', ?6,
                ?7, ?8, ?9, ?10, ?11, 0, ?12, NULL, ?13, ?14, ?15, ?16
            )",
            rusqlite::params![
                artefact_id,
                symbol_id,
                repo_id.as_str(),
                blob_sha,
                path,
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
                if canonical_kind == "file" {
                    "[]"
                } else {
                    "[\"export\"]"
                },
                if canonical_kind == "file" {
                    Option::<&str>::None
                } else {
                    Some("Temporal docstring")
                },
                format!("hash-{artefact_id}"),
                created_at,
            ],
        )
        .expect("insert historical artefact row");
    }

    for (
        symbol_id,
        artefact_id,
        blob_sha,
        path,
        canonical_kind,
        language_kind,
        symbol_fqn,
        parent_symbol_id,
        parent_artefact_id,
        start_line,
        end_line,
    ) in [
        (
            "file::v2-api-caller",
            "artefact::v2-file-api-caller",
            "blob-api-caller-v2",
            "packages/api/src/caller.ts",
            "file",
            "source_file",
            "packages/api/src/caller.ts",
            Option::<&str>::None,
            Option::<&str>::None,
            1_i64,
            5_i64,
        ),
        (
            "sym::v2-api-caller",
            "artefact::v2-api-caller",
            "blob-api-caller-v2",
            "packages/api/src/caller.ts",
            "function",
            "function_declaration",
            "packages/api/src/caller.ts::callerCurrent",
            Some("file::v2-api-caller"),
            Some("artefact::v2-file-api-caller"),
            3_i64,
            5_i64,
        ),
        (
            "file::v2-web-page",
            "artefact::v2-file-web-page",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "file",
            "source_file",
            "packages/web/src/page.ts",
            Option::<&str>::None,
            Option::<&str>::None,
            1_i64,
            3_i64,
        ),
        (
            "sym::v2-web-render",
            "artefact::v2-web-render",
            "blob-web-page-v1",
            "packages/web/src/page.ts",
            "function",
            "function_declaration",
            "packages/web/src/page.ts::render",
            Some("file::v2-web-page"),
            Some("artefact::v2-file-web-page"),
            1_i64,
            3_i64,
        ),
    ] {
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript', ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                0, ?13, NULL, ?14, ?15, '2026-03-26T09:00:00Z'
            )",
            rusqlite::params![
                repo_id.as_str(),
                path,
                blob_sha,
                symbol_id,
                artefact_id,
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_symbol_id,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
                if canonical_kind == "file" {
                    "[]"
                } else {
                    "[\"export\"]"
                },
                if canonical_kind == "file" {
                    Option::<&str>::None
                } else {
                    Some("Current temporal docstring")
                },
            ],
        )
        .expect("insert current artefact row");
    }

    conn.execute(
        "INSERT INTO artefact_edges (
            edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref,
            edge_kind, language, start_line, end_line, metadata, created_at
        ) VALUES (
            'edge::v1-api-caller-target', ?1, 'blob-api-caller-v1', 'artefact::v1-api-caller',
            'artefact::v1-api-target', 'packages/api/src/target.ts::target',
            'calls', 'typescript', 4, 4, '{\"resolution\":\"local\"}', '2026-03-25T09:00:00Z'
        )",
        rusqlite::params![repo_id.as_str()],
    )
    .expect("insert historical edge row");

    conn.execute(
        "INSERT INTO artefact_edges_current (
            repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
            to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
            start_line, end_line, metadata, updated_at
        ) VALUES (
            ?1, 'edge::v2-api-caller-web', 'packages/api/src/caller.ts', 'blob-api-caller-v2',
            'sym::v2-api-caller', 'artefact::v2-api-caller',
            'sym::v2-web-render', 'artefact::v2-web-render', 'packages/web/src/page.ts::render', 'calls', 'typescript',
            4, 4, '{\"resolution\":\"local\"}', '2026-03-26T09:00:00Z'
        )",
        rusqlite::params![repo_id.as_str()],
    )
    .expect("insert current edge row");

    SeededGraphqlTemporalRepo {
        repo: dir,
        first_commit,
    }
}

pub(super) struct SeededGraphqlEventBackedRepo {
    pub(super) repo: TempDir,
    pub(super) first_commit: String,
}

pub(super) fn seed_graphql_event_backed_repo() -> SeededGraphqlEventBackedRepo {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("packages/api/src")).expect("create api src dir");
    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "export function callerV1() {\n  return 1;\n}\n",
    )
    .expect("write caller v1");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function targetV1() {\n  return 2;\n}\n",
    )
    .expect("write target v1");
    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed event-backed GraphQL commit 1"],
    );
    let first_commit = git_ok(repo_root, &["rev-parse", "HEAD"]);

    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "export function callerCurrent() {\n  return 10;\n}\n",
    )
    .expect("write caller current");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function targetCurrent() {\n  return 20;\n}\n",
    )
    .expect("write target current");
    fs::write(
        repo_root.join("packages/api/src/copy.ts"),
        "export function copyCurrent() {\n  return 20;\n}\n",
    )
    .expect("write copy current");
    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &["commit", "-m", "Seed event-backed GraphQL commit 2"],
    );
    let second_commit = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let sqlite_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("graphql-event-backed.sqlite");
    crate::storage::init::init_database(&sqlite_path, false, &second_commit)
        .expect("initialise GraphQL event-backed sqlite store");
    crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
        .expect("connect GraphQL event-backed sqlite store")
        .initialise_devql_schema()
        .expect("initialise DevQL schema for event-backed GraphQL sqlite store");
    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": sqlite_path.to_string_lossy()
                }
            }
        }),
    );

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open GraphQL event-backed sqlite");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', ?2, 'main')",
        rusqlite::params![repo_id.as_str(), SEEDED_REPO_NAME],
    )
    .expect("insert repository row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed event-backed GraphQL commit 1', '2026-03-25T09:00:00Z')",
        rusqlite::params![first_commit.as_str(), repo_id.as_str()],
    )
    .expect("insert first commit row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed event-backed GraphQL commit 2', '2026-03-26T09:00:00Z')",
        rusqlite::params![second_commit.as_str(), repo_id.as_str()],
    )
    .expect("insert second commit row");

    for (commit_sha, path, blob_sha) in [
        (
            first_commit.as_str(),
            "packages/api/src/caller.ts",
            "blob-caller-v1",
        ),
        (
            first_commit.as_str(),
            "packages/api/src/target.ts",
            "blob-target-v1",
        ),
        (
            second_commit.as_str(),
            "packages/api/src/caller.ts",
            "blob-caller-v2",
        ),
        (
            second_commit.as_str(),
            "packages/api/src/target.ts",
            "blob-target-v2",
        ),
        (
            second_commit.as_str(),
            "packages/api/src/copy.ts",
            "blob-target-v2",
        ),
    ] {
        insert_file_state_row(&conn, repo_id.as_str(), commit_sha, path, blob_sha);
    }

    for (path, blob_sha) in [
        ("packages/api/src/caller.ts", "blob-caller-v2"),
        ("packages/api/src/target.ts", "blob-target-v2"),
        ("packages/api/src/copy.ts", "blob-target-v2"),
    ] {
        insert_current_file_state_row(
            &conn,
            repo_id.as_str(),
            path,
            "typescript",
            blob_sha,
            "2026-03-26T09:00:00Z",
        );
    }

    insert_historical_function_artefact(
        &conn,
        repo_id.as_str(),
        "artefact::caller-v1",
        "sym::caller-v1",
        "blob-caller-v1",
        "packages/api/src/caller.ts",
        "packages/api/src/caller.ts::callerV1",
        1,
        3,
        "2026-03-25T09:00:00Z",
    );
    insert_historical_function_artefact(
        &conn,
        repo_id.as_str(),
        "artefact::target-v1",
        "sym::target-v1",
        "blob-target-v1",
        "packages/api/src/target.ts",
        "packages/api/src/target.ts::targetV1",
        1,
        3,
        "2026-03-25T09:00:00Z",
    );

    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "artefact::caller-current",
        "sym::caller-current",
        "blob-caller-v2",
        "packages/api/src/caller.ts",
        "packages/api/src/caller.ts::callerCurrent",
        1,
        3,
        "2026-03-26T09:00:00Z",
    );
    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "artefact::target-current",
        "sym::target-current",
        "blob-target-v2",
        "packages/api/src/target.ts",
        "packages/api/src/target.ts::targetCurrent",
        1,
        3,
        "2026-03-26T09:00:00Z",
    );
    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "artefact::copy-current",
        "sym::copy-current",
        "blob-target-v2",
        "packages/api/src/copy.ts",
        "packages/api/src/copy.ts::copyCurrent",
        1,
        3,
        "2026-03-26T09:00:00Z",
    );

    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-v1-caller",
        "session-v1",
        "2026-03-25T09:30:00Z",
        "codex",
        "main",
        "manual",
        first_commit.as_str(),
        "packages/api/src/caller.ts",
        "blob-caller-v1",
    );
    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-v2-caller",
        "session-v2",
        "2026-03-26T09:30:00Z",
        "codex",
        "main",
        "manual",
        second_commit.as_str(),
        "packages/api/src/caller.ts",
        "blob-caller-v2",
    );
    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-v2-target",
        "session-v2",
        "2026-03-26T09:45:00Z",
        "codex",
        "main",
        "manual",
        second_commit.as_str(),
        "packages/api/src/target.ts",
        "blob-target-v2",
    );

    SeededGraphqlEventBackedRepo {
        repo: dir,
        first_commit,
    }
}

pub(super) struct SeededGraphqlSaveRevisionEventBackedRepo {
    pub(super) repo: TempDir,
    pub(super) save_revision: String,
}

pub(super) fn seed_graphql_save_revision_event_backed_repo()
-> SeededGraphqlSaveRevisionEventBackedRepo {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();
    let save_revision = "temp:42".to_string();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("packages/api/src")).expect("create api src dir");
    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "export function callerBase() {\n  return 1;\n}\n",
    )
    .expect("write caller base");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function targetBase() {\n  return 2;\n}\n",
    )
    .expect("write target base");
    git_ok(repo_root, &["add", "."]);
    git_ok(
        repo_root,
        &[
            "commit",
            "-m",
            "Seed saveRevision event-backed GraphQL commit",
        ],
    );
    let commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let sqlite_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("graphql-save-revision.sqlite");
    crate::storage::init::init_database(&sqlite_path, false, &commit_sha)
        .expect("initialise GraphQL saveRevision sqlite store");
    crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())
        .expect("connect GraphQL saveRevision sqlite store")
        .initialise_devql_schema()
        .expect("initialise DevQL schema for saveRevision GraphQL sqlite store");
    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": sqlite_path.to_string_lossy()
                }
            }
        }),
    );

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open GraphQL saveRevision sqlite");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', ?2, 'main')",
        rusqlite::params![repo_id.as_str(), SEEDED_REPO_NAME],
    )
    .expect("insert repository row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed saveRevision event-backed GraphQL commit', '2026-03-26T11:00:00Z')",
        rusqlite::params![commit_sha.as_str(), repo_id.as_str()],
    )
    .expect("insert commit row");

    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "artefact::caller-temp",
        "sym::caller-temp",
        "blob-caller-temp",
        "packages/api/src/caller.ts",
        "packages/api/src/caller.ts::callerTemp",
        1,
        3,
        "2026-03-26T11:15:00Z",
    );
    insert_current_function_artefact(
        &conn,
        repo_id.as_str(),
        "artefact::target-temp",
        "sym::target-temp",
        "blob-target-temp",
        "packages/api/src/target.ts",
        "packages/api/src/target.ts::targetTemp",
        1,
        3,
        "2026-03-26T11:15:00Z",
    );

    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-temp-caller",
        "session-temp",
        "2026-03-26T11:20:00Z",
        "codex",
        "main",
        "manual",
        commit_sha.as_str(),
        "packages/api/src/caller.ts",
        "blob-caller-temp",
    );
    insert_checkpoint_file_snapshot_row(
        &conn,
        repo_id.as_str(),
        "checkpoint-temp-target",
        "session-temp",
        "2026-03-26T11:25:00Z",
        "codex",
        "main",
        "manual",
        commit_sha.as_str(),
        "packages/api/src/target.ts",
        "blob-target-temp",
    );

    SeededGraphqlSaveRevisionEventBackedRepo {
        repo: dir,
        save_revision,
    }
}
