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
            "INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at)
             VALUES (?1, ?2, ?3, ?4, '2026-03-26T10:00:00Z')",
            rusqlite::params![repo_id.as_str(), path, commit_sha.as_str(), blob_sha],
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
                artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
                language_kind, symbol_fqn, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, content_hash, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, 'typescript', ?6,
                ?7, ?8, ?9, ?10, ?11, 0, ?12, NULL, ?13, ?14, ?15, '2026-03-26T10:00:00Z'
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
                    Some("Monorepo docstring")
                },
                format!("hash-{artefact_id}"),
            ],
        )
        .expect("insert artefact row");
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id,
                temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind,
                symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
                start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at
            ) VALUES (
                ?1, 'main', ?2, ?3, ?4, 'commit', ?4,
                NULL, ?5, ?6, 'typescript', ?7, ?8,
                ?9, ?10, ?11, ?12, ?13,
                0, ?14, NULL, ?15, ?16, ?17, '2026-03-26T10:00:00Z'
            )",
            rusqlite::params![
                repo_id.as_str(),
                symbol_id,
                artefact_id,
                commit_sha.as_str(),
                blob_sha,
                path,
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
                format!("hash-{artefact_id}"),
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
                edge_id, repo_id, branch, commit_sha, revision_kind, revision_id,
                temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language,
                start_line, end_line, metadata, updated_at
            ) VALUES (
                ?1, ?2, 'main', ?3, 'commit', ?3,
                NULL, 'blob-api-caller', 'packages/api/src/caller.ts', ?4, ?5,
                ?6, ?7, ?8, 'calls', 'typescript',
                ?9, ?9, '{\"resolution\":\"local\"}', '2026-03-26T10:00:00Z'
            )",
            rusqlite::params![
                edge_id,
                repo_id.as_str(),
                commit_sha.as_str(),
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
        conn.execute(
            "INSERT INTO symbol_clone_edges (
                repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
                relation_kind, score, semantic_score, lexical_score, structural_score,
                clone_input_hash, explanation_json
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10,
                ?11, ?12
            )",
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
        TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord, TestDiscoveryRunRecord,
    };

    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    let mut repository = open_repository_for_repo(repo_root).expect("open test harness repository");
    let discovery_run = TestDiscoveryRunRecord {
        discovery_run_id: format!("discovery:{commit_sha}"),
        repo_id: repo_id.clone(),
        commit_sha: commit_sha.to_string(),
        language: Some("typescript".to_string()),
        started_at: "2026-03-26T11:00:00Z".to_string(),
        finished_at: Some("2026-03-26T11:00:01Z".to_string()),
        status: "complete".to_string(),
        enumeration_status: Some("complete".to_string()),
        notes_json: None,
        stats_json: None,
    };

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
            commit_sha: commit_sha.to_string(),
            blob_sha: format!("test-blob-suite-{index}"),
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
            content_hash: None,
            discovery_source: "static".to_string(),
            revision_kind: "commit".to_string(),
            revision_id: commit_sha.to_string(),
        });

        test_artefacts.push(TestArtefactCurrentRecord {
            artefact_id: test_artefact_id.clone(),
            symbol_id: test_symbol_id.clone(),
            repo_id: repo_id.clone(),
            commit_sha: commit_sha.to_string(),
            blob_sha: format!("test-blob-scenario-{index}"),
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
            content_hash: None,
            discovery_source: "static".to_string(),
            revision_kind: "commit".to_string(),
            revision_id: commit_sha.to_string(),
        });

        test_edges.push(TestArtefactEdgeCurrentRecord {
            edge_id: format!("test-edge-{index}"),
            repo_id: repo_id.clone(),
            commit_sha: commit_sha.to_string(),
            blob_sha: format!("test-blob-edge-{index}"),
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
            revision_kind: "commit".to_string(),
            revision_id: commit_sha.to_string(),
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
        .replace_test_discovery(
            commit_sha,
            &test_artefacts,
            &test_edges,
            &discovery_run,
            &[],
        )
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
