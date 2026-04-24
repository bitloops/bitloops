use super::*;

pub(super) fn seed_graphql_monorepo_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("packages/api/src")).expect("create api src dir");
    fs::create_dir_all(repo_root.join("packages/web/src")).expect("create web src dir");
    fs::write(
        repo_root.join("packages/api/src/caller.ts"),
        "import { target } from \"./target\";\nimport { render } from \"../../web/src/page\";\n\nexport function caller() {\n  return target() + render();\n}\n",
    )
    .expect("write api caller.ts");
    fs::write(
        repo_root.join("packages/api/src/target.ts"),
        "export function target() {\n  return 41;\n}\n",
    )
    .expect("write api target.ts");
    fs::write(
        repo_root.join("packages/web/src/page.ts"),
        "export function render() {\n  return 1;\n}\n",
    )
    .expect("write web page.ts");
    git_ok(repo_root, &["add", "."]);
    git_ok(repo_root, &["commit", "-m", "Seed GraphQL monorepo repo"]);
    let commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);

    let sqlite_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("graphql-monorepo.sqlite");
    crate::storage::init::init_database(&sqlite_path, false, &commit_sha)
        .expect("initialise GraphQL monorepo sqlite store");
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
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open GraphQL monorepo sqlite");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
         VALUES (?1, 'local', 'local', ?2, 'main')",
        rusqlite::params![repo_id.as_str(), SEEDED_REPO_NAME],
    )
    .expect("insert repository row");
    conn.execute(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
         VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Seed GraphQL monorepo repo', '2026-03-26T10:00:00Z')",
        rusqlite::params![commit_sha.as_str(), repo_id.as_str()],
    )
    .expect("insert commit row");

    for (path, blob_sha) in [
        ("packages/api/src/caller.ts", "blob-api-caller"),
        ("packages/api/src/target.ts", "blob-api-target"),
        ("packages/web/src/page.ts", "blob-web-page"),
    ] {
        conn.execute(
            "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![repo_id.as_str(), commit_sha.as_str(), path, blob_sha],
        )
        .expect("insert file_state row");
        conn.execute(
            "INSERT INTO current_file_state (
                repo_id, path, language,
                head_content_id, index_content_id, worktree_content_id,
                effective_content_id, effective_source,
                parser_version, extractor_version,
                exists_in_head, exists_in_index, exists_in_worktree,
                last_synced_at
            ) VALUES (?1, ?2, 'typescript', ?3, ?3, ?3, ?3, 'head', 'test', 'test', 1, 1, 1, '2026-03-26T10:00:00Z')",
            rusqlite::params![repo_id.as_str(), path, blob_sha],
        )
        .expect("insert current_file_state row");
    }

    let artefacts = [
        (
            "file::api-caller",
            "artefact::file-api-caller",
            "blob-api-caller",
            "packages/api/src/caller.ts",
            "file",
            "source_file",
            "packages/api/src/caller.ts",
            Option::<&str>::None,
            1_i64,
            6_i64,
        ),
        (
            "sym::api-caller",
            "artefact::api-caller",
            "blob-api-caller",
            "packages/api/src/caller.ts",
            "function",
            "function_declaration",
            "packages/api/src/caller.ts::caller",
            Some("artefact::file-api-caller"),
            4_i64,
            6_i64,
        ),
        (
            "file::api-target",
            "artefact::file-api-target",
            "blob-api-target",
            "packages/api/src/target.ts",
            "file",
            "source_file",
            "packages/api/src/target.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
        ),
        (
            "sym::api-target",
            "artefact::api-target",
            "blob-api-target",
            "packages/api/src/target.ts",
            "function",
            "function_declaration",
            "packages/api/src/target.ts::target",
            Some("artefact::file-api-target"),
            1_i64,
            3_i64,
        ),
        (
            "file::web-page",
            "artefact::file-web-page",
            "blob-web-page",
            "packages/web/src/page.ts",
            "file",
            "source_file",
            "packages/web/src/page.ts",
            Option::<&str>::None,
            1_i64,
            3_i64,
        ),
        (
            "sym::web-render",
            "artefact::web-render",
            "blob-web-page",
            "packages/web/src/page.ts",
            "function",
            "function_declaration",
            "packages/web/src/page.ts::render",
            Some("artefact::file-web-page"),
            1_i64,
            3_i64,
        ),
    ];

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
    ) in artefacts
    {
        let parent_symbol_id = match parent_artefact_id {
            Some("artefact::file-api-caller") => Some("file::api-caller"),
            Some("artefact::file-api-target") => Some("file::api-target"),
            Some("artefact::file-web-page") => Some("file::web-page"),
            _ => None,
        };
        conn.execute(
            "INSERT INTO artefacts (
                artefact_id, symbol_id, repo_id, language, canonical_kind,
                language_kind, symbol_fqn, signature, modifiers, docstring, content_hash, created_at
            ) VALUES (
                ?1, ?2, ?3, 'typescript', ?4, ?5, ?6, NULL, ?7, ?8, ?9, '2026-03-26T10:00:00Z'
            )",
            rusqlite::params![
                artefact_id,
                symbol_id,
                repo_id.as_str(),
                canonical_kind,
                language_kind,
                symbol_fqn,
                if canonical_kind == "file" {
                    "[]"
                } else {
                    "[\"export\"]"
                },
                if canonical_kind == "file" {
                    Option::<&str>::None
                } else {
                    Some("Monorepo docstring")
                },
                format!("hash-{artefact_id}"),
            ],
        )
        .expect("insert artefact metadata row");
        conn.execute(
            "INSERT INTO artefact_snapshots (
                repo_id, blob_sha, path, artefact_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, '2026-03-26T10:00:00Z'
            )",
            rusqlite::params![
                repo_id.as_str(),
                blob_sha,
                path,
                artefact_id,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
            ],
        )
        .expect("insert artefact snapshot row");
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript', ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                0, ?13, NULL, ?14, ?15, '2026-03-26T10:00:00Z'
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
                    Some("Monorepo docstring")
                },
            ],
        )
        .expect("insert artefact current row");
    }

    for (
        edge_id,
        from_symbol_id,
        from_artefact_id,
        to_symbol_id,
        to_artefact_id,
        to_symbol_ref,
        line,
    ) in [
        (
            "edge-api-local",
            "sym::api-caller",
            "artefact::api-caller",
            Some("sym::api-target"),
            Some("artefact::api-target"),
            Some("packages/api/src/target.ts::target"),
            5_i64,
        ),
        (
            "edge-api-cross",
            "sym::api-caller",
            "artefact::api-caller",
            Some("sym::web-render"),
            Some("artefact::web-render"),
            Some("packages/web/src/page.ts::render"),
            5_i64,
        ),
    ] {
        conn.execute(
            "INSERT INTO artefact_edges_current (
                repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?2, ?1, 'packages/api/src/caller.ts', 'blob-api-caller', ?3, ?4,
                ?5, ?6, ?7, 'calls', 'typescript',
                ?8, ?8, '{\"resolution\":\"local\"}', '2026-03-26T10:00:00Z'
            )",
            rusqlite::params![
                edge_id,
                repo_id.as_str(),
                from_symbol_id,
                from_artefact_id,
                to_symbol_id,
                to_artefact_id,
                to_symbol_ref,
                line,
            ],
        )
        .expect("insert edge current row");
    }

    dir
}

pub(super) fn seed_graphql_clone_data(repo_root: &Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open clone sqlite");

    conn.execute_batch(
        crate::capability_packs::semantic_clones::schema::semantic_clones_sqlite_schema_sql(),
    )
    .expect("initialise clone sqlite schema");

    for (
        source_symbol_id,
        source_artefact_id,
        target_symbol_id,
        target_artefact_id,
        relation_kind,
        score,
        semantic_score,
        lexical_score,
        structural_score,
        clone_input_hash,
        explanation_json,
    ) in [
        (
            "sym::api-caller",
            "artefact::api-caller",
            "sym::api-target",
            "artefact::api-target",
            "similar_implementation",
            0.93_f64,
            0.91_f64,
            0.84_f64,
            0.72_f64,
            "clone-hash-1",
            r#"{"reason":"shared invoice assembly"}"#,
        ),
        (
            "sym::api-caller",
            "artefact::api-caller",
            "sym::web-render",
            "artefact::web-render",
            "similar_implementation",
            0.71_f64,
            0.68_f64,
            0.64_f64,
            0.58_f64,
            "clone-hash-2",
            r#"{"reason":"shared rendering pattern"}"#,
        ),
        (
            "sym::web-render",
            "artefact::web-render",
            "sym::api-target",
            "artefact::api-target",
            "contextual_neighbor",
            0.68_f64,
            0.66_f64,
            0.52_f64,
            0.61_f64,
            "clone-hash-3",
            r#"{"reason":"cross-package helper overlap"}"#,
        ),
    ] {
        for table in ["symbol_clone_edges", "symbol_clone_edges_current"] {
            let sql = format!(
                "INSERT INTO {table} (
                    repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
                    relation_kind, score, semantic_score, lexical_score, structural_score,
                    clone_input_hash, explanation_json
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12
                )"
            );
            conn.execute(
                &sql,
                rusqlite::params![
                    repo_id.as_str(),
                    source_symbol_id,
                    source_artefact_id,
                    target_symbol_id,
                    target_artefact_id,
                    relation_kind,
                    score,
                    semantic_score,
                    lexical_score,
                    structural_score,
                    clone_input_hash,
                    explanation_json,
                ],
            )
            .expect("insert clone edge");
        }
    }
}

pub(super) fn seed_graphql_clone_scoring_inputs(repo_root: &Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open clone scoring sqlite");

    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS symbol_semantics_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL NOT NULL,
    source_model TEXT,
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS symbol_features_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    identifier_tokens TEXT NOT NULL DEFAULT '[]',
    normalized_body_tokens TEXT NOT NULL DEFAULT '[]',
    parent_kind TEXT,
    context_tokens TEXT NOT NULL DEFAULT '[]',
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS symbol_embeddings_current (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    setup_fingerprint TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding TEXT NOT NULL,
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (artefact_id, representation_kind, setup_fingerprint)
);
"#,
    )
    .expect("initialise clone scoring tables");
    conn.execute_batch(
        crate::capability_packs::semantic_clones::schema::semantic_clones_sqlite_schema_sql(),
    )
    .expect("initialise clone edge table");

    for (artefact_id, path, content_id, symbol_id, semantic_hash, template_summary, summary) in [
        (
            "artefact::api-caller",
            "packages/api/src/caller.ts",
            "blob-api-caller",
            "sym::api-caller",
            "semantic-hash-api-caller",
            "Caller helper summary",
            "Calls API target and web render helpers to build a response payload.",
        ),
        (
            "artefact::api-target",
            "packages/api/src/target.ts",
            "blob-api-target",
            "sym::api-target",
            "semantic-hash-api-target",
            "Target helper summary",
            "Builds API response payload fields and returns the transformed target result.",
        ),
        (
            "artefact::web-render",
            "packages/web/src/page.ts",
            "blob-web-page",
            "sym::web-render",
            "semantic-hash-web-render",
            "Render helper summary",
            "Renders a web payload fragment used by API caller output assembly.",
        ),
    ] {
        conn.execute(
            "INSERT OR REPLACE INTO symbol_semantics_current (
                artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                template_summary, summary, confidence
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                artefact_id,
                repo_id.as_str(),
                path,
                content_id,
                symbol_id,
                semantic_hash,
                template_summary,
                summary,
                0.92_f64,
            ],
        )
        .expect("insert current clone scoring semantics");
    }

    for (
        artefact_id,
        path,
        content_id,
        symbol_id,
        semantic_hash,
        normalized_name,
        normalized_signature,
        identifier_tokens,
        body_tokens,
        context_tokens,
    ) in [
        (
            "artefact::api-caller",
            "packages/api/src/caller.ts",
            "blob-api-caller",
            "sym::api-caller",
            "semantic-hash-api-caller",
            "caller",
            "function caller()",
            r#"["api","caller","payload","target","render"]"#,
            r#"["compose","payload","target","result"]"#,
            r#"["packages","api","src"]"#,
        ),
        (
            "artefact::api-target",
            "packages/api/src/target.ts",
            "blob-api-target",
            "sym::api-target",
            "semantic-hash-api-target",
            "target",
            "function caller()",
            r#"["api","target","payload","response"]"#,
            r#"["compose","payload","target","result"]"#,
            r#"["packages","api","src"]"#,
        ),
        (
            "artefact::web-render",
            "packages/web/src/page.ts",
            "blob-web-page",
            "sym::web-render",
            "semantic-hash-web-render",
            "render",
            "function render()",
            r#"["web","render","payload","response"]"#,
            r#"["render","payload","fragment","page"]"#,
            r#"["packages","web","src"]"#,
        ),
    ] {
        conn.execute(
            "INSERT OR REPLACE INTO symbol_features_current (
                artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                normalized_name, normalized_signature, modifiers, identifier_tokens,
                normalized_body_tokens, parent_kind, context_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '[]', ?9, ?10, ?11, ?12)",
            rusqlite::params![
                artefact_id,
                repo_id.as_str(),
                path,
                content_id,
                symbol_id,
                semantic_hash,
                normalized_name,
                normalized_signature,
                identifier_tokens,
                body_tokens,
                "module",
                context_tokens,
            ],
        )
        .expect("insert current clone scoring features");
    }

    for (artefact_id, path, content_id, symbol_id, input_hash, embedding) in [
        (
            "artefact::api-caller",
            "packages/api/src/caller.ts",
            "blob-api-caller",
            "sym::api-caller",
            "embed-hash-api-caller",
            "[0.95,0.05,0.0]",
        ),
        (
            "artefact::api-target",
            "packages/api/src/target.ts",
            "blob-api-target",
            "sym::api-target",
            "embed-hash-api-target",
            "[0.93,0.07,0.0]",
        ),
        (
            "artefact::web-render",
            "packages/web/src/page.ts",
            "blob-web-page",
            "sym::web-render",
            "embed-hash-web-render",
            "[0.81,0.19,0.0]",
        ),
    ] {
        conn.execute(
            "INSERT OR REPLACE INTO symbol_embeddings_current (
                artefact_id, repo_id, path, content_id, symbol_id, representation_kind,
                provider, model, dimension, embedding_input_hash, embedding
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'baseline', ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                artefact_id,
                repo_id.as_str(),
                path,
                content_id,
                symbol_id,
                crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER,
                "bge-m3",
                3,
                input_hash,
                embedding,
            ],
        )
        .expect("insert current clone scoring embeddings");
    }
}

pub(super) fn seed_graphql_historical_summary_inputs(repo_root: &Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open historical summary sqlite");

    conn.execute_batch(
        crate::capability_packs::semantic_clones::semantic_features_sqlite_schema_sql(),
    )
    .expect("initialise historical summary schema");

    conn.execute(
        "INSERT OR REPLACE INTO symbol_semantics (
            artefact_id, repo_id, blob_sha, semantic_features_input_hash,
            template_summary, summary, confidence, source_model
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "artefact::api-target",
            repo_id.as_str(),
            "blob-api-target",
            "semantic-hash-api-target",
            "Target helper summary",
            "Builds API response payload fields and returns the transformed target result.",
            0.92_f64,
            "bitloops:historical-test-model",
        ],
    )
    .expect("insert historical summary row");
}

pub(super) fn seed_graphql_semantic_query_inputs(repo_root: &Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open semantic query sqlite");
    let setup = crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup::new(
        crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER,
        "semantic-query-test-model",
        3,
    );

    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS semantic_embedding_setups (
    setup_fingerprint TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS symbol_embeddings_current (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    representation_kind TEXT NOT NULL DEFAULT 'code',
    setup_fingerprint TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding TEXT NOT NULL,
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (artefact_id, representation_kind, setup_fingerprint)
);

CREATE TABLE IF NOT EXISTS semantic_clone_embedding_setup_state (
    repo_id TEXT NOT NULL,
    representation_kind TEXT NOT NULL DEFAULT 'code',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    setup_fingerprint TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (repo_id, representation_kind)
);
"#,
    )
    .expect("initialise semantic query embedding tables");
    conn.execute(
        "INSERT OR REPLACE INTO semantic_embedding_setups (
            setup_fingerprint, provider, model, dimension
        ) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            setup.setup_fingerprint,
            setup.provider,
            setup.model,
            i64::try_from(setup.dimension).expect("setup dimension fits in i64"),
        ],
    )
    .expect("insert semantic query setup");
    for representation_kind in ["identity", "code", "summary"] {
        conn.execute(
            "INSERT OR REPLACE INTO semantic_clone_embedding_setup_state (
                repo_id, representation_kind, provider, model, dimension, setup_fingerprint
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                repo_id.as_str(),
                representation_kind,
                crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER,
                "semantic-query-test-model",
                3,
                setup.setup_fingerprint,
            ],
        )
        .expect("insert semantic query active setup");
    }

    for (representation_kind, (artefact_id, path, content_id, symbol_id, input_hash, embedding)) in [
        (
            "identity",
            (
                "artefact::api-caller",
                "packages/api/src/caller.ts",
                "blob-api-caller",
                "sym::api-caller",
                "semantic-query-hash-api-caller-identity",
                "[0.0,-1.0,0.0]",
            ),
        ),
        (
            "identity",
            (
                "artefact::api-target",
                "packages/api/src/target.ts",
                "blob-api-target",
                "sym::api-target",
                "semantic-query-hash-api-target-identity",
                "[0.0,0.0,-1.0]",
            ),
        ),
        (
            "identity",
            (
                "artefact::web-render",
                "packages/web/src/page.ts",
                "blob-web-page",
                "sym::web-render",
                "semantic-query-hash-web-render-identity",
                "[0.0,0.0,1.0]",
            ),
        ),
        (
            "code",
            (
                "artefact::api-caller",
                "packages/api/src/caller.ts",
                "blob-api-caller",
                "sym::api-caller",
                "semantic-query-hash-api-caller-code",
                "[1.0,0.0,0.0]",
            ),
        ),
        (
            "code",
            (
                "artefact::web-render",
                "packages/web/src/page.ts",
                "blob-web-page",
                "sym::web-render",
                "semantic-query-hash-web-render-code",
                "[0.0,1.0,0.0]",
            ),
        ),
        (
            "summary",
            (
                "artefact::api-target",
                "packages/api/src/target.ts",
                "blob-api-target",
                "sym::api-target",
                "semantic-query-hash-api-target-summary",
                "[0.98,0.02,0.0]",
            ),
        ),
        (
            "summary",
            (
                "artefact::web-render",
                "packages/web/src/page.ts",
                "blob-web-page",
                "sym::web-render",
                "semantic-query-hash-web-render-summary",
                "[0.0,0.95,0.05]",
            ),
        ),
    ] {
        conn.execute(
            "INSERT OR REPLACE INTO symbol_embeddings_current (
                artefact_id, repo_id, path, content_id, symbol_id, representation_kind,
                setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                artefact_id,
                repo_id.as_str(),
                path,
                content_id,
                symbol_id,
                representation_kind,
                setup.setup_fingerprint,
                crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER,
                "semantic-query-test-model",
                3,
                input_hash,
                embedding,
            ],
        )
        .expect("insert semantic query identity embedding row");
    }
}

#[cfg(unix)]
fn fake_semantic_query_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-semantic-query-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake semantic query runtime dir");
    }
    fs::write(
        &script_path,
        r#"#!/bin/sh
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*'"texts":["build response payload"]'*)
      vector='[[1.0,0.0,0.0]]'
      ;;
    *'"cmd":"embed"'*'"texts":["render payload fragment"]'*)
      vector='[[0.0,1.0,0.0]]'
      ;;
    *'"cmd":"embed"'*'"texts":["caller in caller ts"]'*)
      vector='[[0.0,-1.0,0.0]]'
      ;;
    *'"cmd":"embed"'*'"texts":["caller"]'*)
      vector='[[0.0,-1.0,0.0]]'
      ;;
    *'"cmd":"embed"'*'"texts":["render in page ts"]'*)
      vector='[[0.0,0.0,1.0]]'
      ;;
    *'"cmd":"embed"'*)
      vector='[[-1.0,0.0,0.0]]'
      ;;
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true,"model":"semantic-query-test-model"}\n' "$req_id"
      exit 0
      ;;
    *)
      printf '{"id":"%s","ok":false,"error":{"message":"unexpected request"}}\n' "$req_id"
      continue
      ;;
  esac
  printf '{"id":"%s","ok":true,"vectors":%s,"model":"semantic-query-test-model"}\n' "$req_id" "$vector"
done
"#,
    )
    .expect("write fake semantic query runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake semantic query runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)
        .expect("chmod fake semantic query runtime script");
    (
        "/bin/sh".to_string(),
        vec![script_path.to_string_lossy().into_owned()],
    )
}

#[cfg(windows)]
fn fake_semantic_query_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-semantic-query-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake semantic query runtime dir");
    }
    fs::write(
        &script_path,
        r#"
$ready = @{ event = "ready"; protocol = 1; capabilities = @("embed", "shutdown") }
$ready | ConvertTo-Json -Compress
while (($line = [Console]::In.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.cmd) {
    "embed" {
      $text = $request.texts[0]
      if ($text -eq "build response payload") {
        $vector = @(@(1.0, 0.0, 0.0))
      } elseif ($text -eq "render payload fragment") {
        $vector = @(@(0.0, 1.0, 0.0))
      } elseif ($text -eq "caller in caller ts") {
        $vector = @(@(0.0, -1.0, 0.0))
      } elseif ($text -eq "caller") {
        $vector = @(@(0.0, -1.0, 0.0))
      } elseif ($text -eq "render in page ts") {
        $vector = @(@(0.0, 0.0, 1.0))
      } else {
        $vector = @(@(-1.0, 0.0, 0.0))
      }
      $response = @{
        id = $request.id
        ok = $true
        vectors = $vector
        model = "semantic-query-test-model"
      }
      $response | ConvertTo-Json -Compress
    }
    "shutdown" {
      $response = @{
        id = $request.id
        ok = $true
        model = "semantic-query-test-model"
      }
      $response | ConvertTo-Json -Compress
      exit 0
    }
    default {
      $response = @{
        id = $request.id
        ok = $false
        error = @{ message = "unexpected request" }
      }
      $response | ConvertTo-Json -Compress
    }
  }
}
"#,
    )
    .expect("write fake semantic query runtime script");
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.to_string_lossy().into_owned(),
        ],
    )
}

pub(super) fn configure_graphql_semantic_query_runtime(repo_root: &Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let (command, args) = fake_semantic_query_runtime_command_and_args(repo_root);
    let runtime_args = args
        .iter()
        .map(|arg| serde_json::Value::String(arg.clone()))
        .collect::<Vec<_>>();
    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": sqlite_path.to_string_lossy()
                }
            },
            "semantic_clones": {
                "summary_mode": "off",
                "embedding_mode": "deterministic",
                "inference": {
                    "code_embeddings": "semantic_query_test",
                    "summary_embeddings": "semantic_query_test"
                }
            },
            "inference": {
                "runtimes": {
                    "bitloops_local_embeddings": {
                        "command": command,
                        "args": runtime_args,
                        "startup_timeout_secs": 5,
                        "request_timeout_secs": 5
                    }
                },
                "profiles": {
                    "semantic_query_test": {
                        "task": "embeddings",
                        "driver": crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER,
                        "runtime": "bitloops_local_embeddings",
                        "model": "semantic-query-test-model"
                    }
                }
            }
        }),
    );
}

pub(super) fn seed_graphql_same_file_method_clone_data(repo_root: &Path) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open method clone sqlite");

    conn.execute_batch(
        crate::capability_packs::semantic_clones::schema::semantic_clones_sqlite_schema_sql(),
    )
    .expect("initialise clone sqlite schema");

    let path = "packages/api/src/change-path.ts";
    let blob_sha = "blob-api-change-path";

    conn.execute(
        "INSERT INTO current_file_state (
            repo_id, path, language,
            head_content_id, index_content_id, worktree_content_id,
            effective_content_id, effective_source,
            parser_version, extractor_version,
            exists_in_head, exists_in_index, exists_in_worktree,
            last_synced_at
        ) VALUES (?1, ?2, 'typescript', ?3, ?3, ?3, ?3, 'head', 'test', 'test', 1, 1, 1, '2026-03-26T10:00:00Z')",
        rusqlite::params![repo_id.as_str(), path, blob_sha],
    )
    .expect("insert method current_file_state row");

    for (
        symbol_id,
        artefact_id,
        symbol_fqn,
        parent_symbol_id,
        parent_artefact_id,
        start_line,
        end_line,
    ) in [
        (
            "class::change-path",
            "artefact::class-change-path",
            "packages/api/src/change-path.ts::ChangePathOfCodeFileCommandHandler",
            Option::<&str>::None,
            Option::<&str>::None,
            1_i64,
            40_i64,
        ),
        (
            "method::execute",
            "artefact::method-execute",
            "packages/api/src/change-path.ts::ChangePathOfCodeFileCommandHandler::execute",
            Some("class::change-path"),
            Some("artefact::class-change-path"),
            10_i64,
            24_i64,
        ),
        (
            "method::command",
            "artefact::method-command",
            "packages/api/src/change-path.ts::ChangePathOfCodeFileCommandHandler::command",
            Some("class::change-path"),
            Some("artefact::class-change-path"),
            26_i64,
            34_i64,
        ),
    ] {
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript', ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                0, ?13, NULL, '[\"public\"]', 'Method docstring', '2026-03-26T10:00:00Z'
            )",
            rusqlite::params![
                repo_id.as_str(),
                path,
                blob_sha,
                symbol_id,
                artefact_id,
                if symbol_id == "class::change-path" {
                    "type"
                } else {
                    "method"
                },
                if symbol_id == "class::change-path" {
                    "class_declaration"
                } else {
                    "method_definition"
                },
                symbol_fqn,
                parent_symbol_id,
                parent_artefact_id,
                start_line,
                end_line,
                end_line * 10,
            ],
        )
        .expect("insert method artefact current row");
    }

    for table in ["symbol_clone_edges", "symbol_clone_edges_current"] {
        let sql = format!(
            "INSERT INTO {table} (
                repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
                relation_kind, score, semantic_score, lexical_score, structural_score,
                clone_input_hash, explanation_json
            ) VALUES (
                ?1, 'method::execute', 'artefact::method-execute', 'method::command', 'artefact::method-command',
                'weak_clone_candidate', 0.61, 0.58, 0.44, 0.73,
                'clone-hash-method-same-file', '{{\"reason\":\"same file helper overlap\"}}'
            )"
        );
        conn.execute(&sql, rusqlite::params![repo_id.as_str()])
            .expect("insert same-file method clone edge");
    }
}

pub(super) fn seed_graphql_test_harness_stage_data(
    repo_root: &Path,
    commit_sha: &str,
    rows: &[(&str, &str, &str, &str)],
) {
    use crate::capability_packs::test_harness::storage::{
        TestHarnessRepository, open_repository_for_repo,
    };
    use crate::models::{
        CoverageCaptureRecord, CoverageFormat, CoverageHitRecord, ScopeKind,
        TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord,
    };

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let mut repository = open_repository_for_repo(repo_root).expect("open test harness repository");
    let mut test_artefacts = Vec::<TestArtefactCurrentRecord>::new();
    let mut test_edges = Vec::<TestArtefactEdgeCurrentRecord>::new();
    let mut coverage_captures = Vec::<CoverageCaptureRecord>::new();
    let mut coverage_hits = Vec::<CoverageHitRecord>::new();

    for (index, (production_symbol_id, production_artefact_id, production_path, test_name)) in
        rows.iter().enumerate()
    {
        let suite_symbol_id = format!("test-suite-symbol-{index}");
        let suite_artefact_id = format!("test-suite-artefact-{index}");
        let test_symbol_id = format!("test-scenario-symbol-{index}");
        let test_artefact_id = format!("test-scenario-artefact-{index}");
        let test_path = format!("tests/generated_{index}.ts");

        test_artefacts.push(TestArtefactCurrentRecord {
            artefact_id: suite_artefact_id.clone(),
            symbol_id: suite_symbol_id.clone(),
            repo_id: repo_id.clone(),
            content_id: format!("test-blob-suite-{index}"),
            path: test_path.clone(),
            language: "typescript".to_string(),
            canonical_kind: "test_suite".to_string(),
            language_kind: Some("describe".to_string()),
            symbol_fqn: Some(format!("tests::suite::{index}")),
            name: format!("suite_{index}"),
            parent_artefact_id: None,
            parent_symbol_id: None,
            start_line: 1,
            end_line: 20,
            start_byte: Some(0),
            end_byte: Some(200),
            signature: None,
            modifiers: "[]".to_string(),
            docstring: None,
            discovery_source: "static".to_string(),
        });

        test_artefacts.push(TestArtefactCurrentRecord {
            artefact_id: test_artefact_id.clone(),
            symbol_id: test_symbol_id.clone(),
            repo_id: repo_id.clone(),
            content_id: format!("test-blob-scenario-{index}"),
            path: test_path.clone(),
            language: "typescript".to_string(),
            canonical_kind: "test_scenario".to_string(),
            language_kind: Some("it".to_string()),
            symbol_fqn: Some(format!("tests::scenario::{index}")),
            name: (*test_name).to_string(),
            parent_artefact_id: Some(suite_artefact_id.clone()),
            parent_symbol_id: Some(suite_symbol_id.clone()),
            start_line: 2,
            end_line: 10,
            start_byte: Some(10),
            end_byte: Some(100),
            signature: None,
            modifiers: "[]".to_string(),
            docstring: None,
            discovery_source: "static".to_string(),
        });

        test_edges.push(TestArtefactEdgeCurrentRecord {
            edge_id: format!("test-edge-{index}"),
            repo_id: repo_id.clone(),
            content_id: format!("test-blob-edge-{index}"),
            path: test_path,
            from_artefact_id: test_artefact_id.clone(),
            from_symbol_id: test_symbol_id.clone(),
            to_artefact_id: Some((*production_artefact_id).to_string()),
            to_symbol_id: Some((*production_symbol_id).to_string()),
            to_symbol_ref: None,
            edge_kind: "covers".to_string(),
            language: "typescript".to_string(),
            start_line: Some(3),
            end_line: Some(8),
            metadata: json!({
                "confidence": 0.91,
                "link_source": "static_analysis",
                "linkage_status": "linked"
            })
            .to_string(),
        });

        coverage_captures.push(CoverageCaptureRecord {
            capture_id: format!("capture-{index}"),
            repo_id: repo_id.clone(),
            commit_sha: commit_sha.to_string(),
            tool: "lcov".to_string(),
            format: CoverageFormat::Lcov,
            scope_kind: ScopeKind::TestScenario,
            subject_test_symbol_id: Some(test_symbol_id),
            line_truth: true,
            branch_truth: true,
            captured_at: format!("2026-03-26T11:00:0{}Z", index + 2),
            status: "complete".to_string(),
            metadata_json: None,
        });

        coverage_hits.push(CoverageHitRecord {
            capture_id: format!("capture-{index}"),
            production_symbol_id: (*production_symbol_id).to_string(),
            file_path: (*production_path).to_string(),
            line: 4,
            branch_id: -1,
            covered: true,
            hit_count: 2,
        });
        coverage_hits.push(CoverageHitRecord {
            capture_id: format!("capture-{index}"),
            production_symbol_id: (*production_symbol_id).to_string(),
            file_path: (*production_path).to_string(),
            line: 5,
            branch_id: -1,
            covered: false,
            hit_count: 0,
        });
        coverage_hits.push(CoverageHitRecord {
            capture_id: format!("capture-{index}"),
            production_symbol_id: (*production_symbol_id).to_string(),
            file_path: (*production_path).to_string(),
            line: 4,
            branch_id: 0,
            covered: true,
            hit_count: 2,
        });
    }

    repository
        .replace_test_discovery(commit_sha, &test_artefacts, &test_edges)
        .expect("replace test discovery");
    for capture in &coverage_captures {
        repository
            .insert_coverage_capture(capture)
            .expect("insert coverage capture");
    }
    repository
        .insert_coverage_hits(&coverage_hits)
        .expect("insert coverage hits");
    repository
        .rebuild_classifications_from_coverage(commit_sha)
        .expect("rebuild classifications from coverage");
}

pub(super) fn seed_graphql_monorepo_repo_with_duckdb_events() -> TempDir {
    let repo = seed_graphql_monorepo_repo();
    let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    seed_checkpoint_storage_for_dashboard(
        repo.path(),
        SeedCheckpointStorage {
            commit_sha: &commit_sha,
            checkpoint_id: "checkpoint-api",
            branch: "main",
            files_touched: &["packages/api/src/caller.ts", "packages/api/src/target.ts"],
            checkpoints_count: 1,
            token_usage: json!({
                "input_tokens": 50,
                "output_tokens": 20,
                "cache_creation_tokens": 0,
                "cache_read_tokens": 0,
                "api_call_count": 1
            }),
            sessions: &[SeedCheckpointSession {
                session_index: 0,
                session_id: "session-api",
                agent: "codex",
                created_at: "2026-03-26T10:20:00Z",
                checkpoints_count: 1,
                transcript: "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"API checkpoint\"}]}}\n",
                prompts: "API checkpoint",
                context: "API checkpoint context",
            }],
            insert_mapping: true,
        },
    );
    seed_checkpoint_storage_for_dashboard(
        repo.path(),
        SeedCheckpointStorage {
            commit_sha: &commit_sha,
            checkpoint_id: "checkpoint-web",
            branch: "main",
            files_touched: &["packages/web/src/page.ts"],
            checkpoints_count: 1,
            token_usage: json!({
                "input_tokens": 25,
                "output_tokens": 10,
                "cache_creation_tokens": 0,
                "cache_read_tokens": 0,
                "api_call_count": 1
            }),
            sessions: &[SeedCheckpointSession {
                session_index: 0,
                session_id: "session-web",
                agent: "codex",
                created_at: "2026-03-26T10:25:00Z",
                checkpoints_count: 1,
                transcript: "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Web checkpoint\"}]}}\n",
                prompts: "Web checkpoint",
                context: "Web checkpoint context",
            }],
            insert_mapping: true,
        },
    );

    seed_duckdb_events(
        repo.path(),
        &[
            SeedGraphqlEvent {
                event_id: "evt-checkpoint-api",
                event_time: "2026-03-26T10:20:00Z",
                checkpoint_id: "checkpoint-api",
                session_id: "session-api",
                commit_sha: &commit_sha,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "codex",
                strategy: "manual-commit",
                files_touched: &["packages/api/src/caller.ts", "packages/api/src/target.ts"],
                payload: json!({"scope": "api"}),
            },
            SeedGraphqlEvent {
                event_id: "evt-checkpoint-web",
                event_time: "2026-03-26T10:25:00Z",
                checkpoint_id: "checkpoint-web",
                session_id: "session-web",
                commit_sha: &commit_sha,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "codex",
                strategy: "manual-commit",
                files_touched: &["packages/web/src/page.ts"],
                payload: json!({"scope": "web"}),
            },
            SeedGraphqlEvent {
                event_id: "evt-telemetry-tool",
                event_time: "2026-03-26T10:30:00Z",
                checkpoint_id: "",
                session_id: "session-api",
                commit_sha: &commit_sha,
                branch: "main",
                event_type: "tool_invocation",
                agent: "codex",
                strategy: "",
                files_touched: &["packages/api/src/caller.ts"],
                payload: json!({"tool": "Edit", "path": "packages/api/src/caller.ts"}),
            },
        ],
    );

    repo
}
