use std::path::Path;

use rusqlite::params;

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID,
};
use crate::daemon::types::{CapabilityEventRunRecord, CapabilityEventRunStatus};
use crate::host::capability_host::{ChangedFile, ReconcileMode, RemovedArtefact, RemovedFile};

use super::plan::{
    MergedArtefactChange, MergedFileChange, build_execution_plan, determine_reconcile_mode,
    merge_artefact_changes, merge_file_changes,
};
use super::queue::{ArtefactChangeRow, FileChangeRow, GenerationRow};

#[test]
fn merge_file_changes_keeps_latest_change_per_path() {
    let merged = merge_file_changes(&[
        FileChangeRow {
            generation_seq: 1,
            path: "src/lib.rs".to_string(),
            change_kind: "added".to_string(),
            language: Some("rust".to_string()),
            content_id: Some("a".to_string()),
        },
        FileChangeRow {
            generation_seq: 2,
            path: "src/lib.rs".to_string(),
            change_kind: "changed".to_string(),
            language: Some("rust".to_string()),
            content_id: Some("b".to_string()),
        },
        FileChangeRow {
            generation_seq: 3,
            path: "src/old.rs".to_string(),
            change_kind: "removed".to_string(),
            language: None,
            content_id: None,
        },
    ]);

    assert_eq!(
        merged,
        vec![
            MergedFileChange::Upsert(ChangedFile {
                path: "src/lib.rs".to_string(),
                language: "rust".to_string(),
                content_id: "b".to_string(),
            }),
            MergedFileChange::Removed(RemovedFile {
                path: "src/old.rs".to_string(),
            }),
        ]
    );
}

#[test]
fn merge_artefact_changes_keeps_latest_change_per_symbol() {
    let merged = merge_artefact_changes(&[
        ArtefactChangeRow {
            generation_seq: 1,
            symbol_id: "symbol-a".to_string(),
            change_kind: "added".to_string(),
            artefact_id: "artefact-a".to_string(),
            path: "src/lib.rs".to_string(),
            canonical_kind: Some("function".to_string()),
            name: "create_user".to_string(),
        },
        ArtefactChangeRow {
            generation_seq: 2,
            symbol_id: "symbol-a".to_string(),
            change_kind: "removed".to_string(),
            artefact_id: "artefact-a".to_string(),
            path: "src/lib.rs".to_string(),
            canonical_kind: None,
            name: "create_user".to_string(),
        },
    ]);

    assert_eq!(
        merged,
        vec![MergedArtefactChange::Removed(RemovedArtefact {
            artefact_id: "artefact-a".to_string(),
            symbol_id: "symbol-a".to_string(),
            path: "src/lib.rs".to_string(),
        })]
    );
}

#[test]
fn determine_reconcile_mode_promotes_full_reconcile_for_first_run_and_thresholds() {
    let generations = vec![GenerationRow {
        generation_seq: 65,
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        requires_full_reconcile: false,
    }];

    assert_eq!(
        determine_reconcile_mode(None, &generations, 1, 1),
        ReconcileMode::FullReconcile
    );
    assert_eq!(
        determine_reconcile_mode(Some(0), &generations, 1, 1),
        ReconcileMode::FullReconcile
    );
    assert_eq!(
        determine_reconcile_mode(Some(64), &generations, 1, 1),
        ReconcileMode::MergedDelta
    );
    assert_eq!(
        determine_reconcile_mode(Some(0), &generations, 2_001, 1),
        ReconcileMode::FullReconcile
    );
    assert_eq!(
        determine_reconcile_mode(Some(0), &generations, 1, 5_001),
        ReconcileMode::FullReconcile
    );
}

#[test]
fn same_init_session_follow_up_architecture_role_run_uses_merged_delta() -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE capability_workplane_cursor_generations (
            repo_id TEXT NOT NULL,
            generation_seq INTEGER NOT NULL,
            source_task_id TEXT,
            sync_mode TEXT NOT NULL,
            active_branch TEXT,
            head_commit_sha TEXT,
            requires_full_reconcile INTEGER NOT NULL DEFAULT 0,
            created_at_unix INTEGER NOT NULL,
            PRIMARY KEY (repo_id, generation_seq)
        );
        CREATE TABLE capability_workplane_cursor_file_changes (
            repo_id TEXT NOT NULL,
            generation_seq INTEGER NOT NULL,
            path TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            language TEXT,
            content_id TEXT
        );
        CREATE TABLE capability_workplane_cursor_artefact_changes (
            repo_id TEXT NOT NULL,
            generation_seq INTEGER NOT NULL,
            symbol_id TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            artefact_id TEXT NOT NULL,
            path TEXT NOT NULL,
            canonical_kind TEXT,
            name TEXT NOT NULL
        );
        CREATE TABLE capability_workplane_cursor_mailboxes (
            repo_id TEXT NOT NULL,
            capability_id TEXT NOT NULL,
            mailbox_name TEXT NOT NULL,
            last_applied_generation_seq INTEGER,
            last_error TEXT,
            updated_at_unix INTEGER NOT NULL,
            PRIMARY KEY (repo_id, capability_id, mailbox_name)
        );
        CREATE TABLE capability_workplane_cursor_runs (
            run_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            repo_root TEXT NOT NULL,
            capability_id TEXT NOT NULL,
            mailbox_name TEXT NOT NULL,
            init_session_id TEXT,
            from_generation_seq INTEGER NOT NULL,
            to_generation_seq INTEGER NOT NULL,
            reconcile_mode TEXT NOT NULL,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            submitted_at_unix INTEGER NOT NULL,
            started_at_unix INTEGER,
            updated_at_unix INTEGER NOT NULL,
            completed_at_unix INTEGER,
            error TEXT
        );",
    )?;
    let repo_id = "repo-architecture-role-init-follow-up";
    let capability_id =
        crate::capability_packs::architecture_graph::types::ARCHITECTURE_GRAPH_CAPABILITY_ID;
    let consumer_id = crate::capability_packs::architecture_graph::types::ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID;

    conn.execute(
        "INSERT INTO capability_workplane_cursor_mailboxes (
            repo_id, capability_id, mailbox_name, last_applied_generation_seq, last_error,
            updated_at_unix
         ) VALUES (?1, ?2, ?3, 10, NULL, 10)",
        params![repo_id, capability_id, consumer_id],
    )?;
    conn.execute(
        "INSERT INTO capability_workplane_cursor_generations (
            repo_id, generation_seq, source_task_id, sync_mode, active_branch, head_commit_sha,
            requires_full_reconcile, created_at_unix
         ) VALUES (?1, 11, 'sync-11', 'full', 'main', 'abc123', 1, 11)",
        params![repo_id],
    )?;
    conn.execute(
        "INSERT INTO capability_workplane_cursor_file_changes (
            repo_id, generation_seq, path, change_kind, language, content_id
         ) VALUES (?1, 11, 'src/api.rs', 'changed', 'rust', 'content-11')",
        params![repo_id],
    )?;
    let run = CapabilityEventRunRecord {
        run_id: "run-role-follow-up".to_string(),
        repo_id: repo_id.to_string(),
        capability_id: capability_id.to_string(),
        init_session_id: Some("init-1".to_string()),
        consumer_id: consumer_id.to_string(),
        handler_id: consumer_id.to_string(),
        from_generation_seq: 10,
        to_generation_seq: 11,
        reconcile_mode: "merged_delta".to_string(),
        event_kind: "current_state_consumer".to_string(),
        lane_key: format!("{repo_id}:{consumer_id}"),
        event_payload_json: String::new(),
        status: CapabilityEventRunStatus::Queued,
        attempts: 0,
        submitted_at_unix: 11,
        started_at_unix: None,
        updated_at_unix: 11,
        completed_at_unix: None,
        error: None,
    };

    let plan = build_execution_plan(&conn, &run, Path::new("/tmp/repo-architecture-role"))?
        .expect("execution plan");

    assert_eq!(plan.request.reconcile_mode, ReconcileMode::MergedDelta);
    assert_eq!(plan.request.file_upserts.len(), 1);
    assert!(plan.request.affected_paths.is_empty());
    Ok(())
}

#[test]
fn same_init_session_architecture_snapshot_falls_back_to_touched_paths() -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE capability_workplane_cursor_generations (
            repo_id TEXT NOT NULL,
            generation_seq INTEGER NOT NULL,
            source_task_id TEXT,
            sync_mode TEXT NOT NULL,
            active_branch TEXT,
            head_commit_sha TEXT,
            requires_full_reconcile INTEGER NOT NULL DEFAULT 0,
            created_at_unix INTEGER NOT NULL,
            PRIMARY KEY (repo_id, generation_seq)
        );
        CREATE TABLE capability_workplane_cursor_file_changes (
            repo_id TEXT NOT NULL,
            generation_seq INTEGER NOT NULL,
            path TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            language TEXT,
            content_id TEXT
        );
        CREATE TABLE capability_workplane_cursor_artefact_changes (
            repo_id TEXT NOT NULL,
            generation_seq INTEGER NOT NULL,
            symbol_id TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            artefact_id TEXT NOT NULL,
            path TEXT NOT NULL,
            canonical_kind TEXT,
            name TEXT NOT NULL
        );
        CREATE TABLE capability_workplane_cursor_mailboxes (
            repo_id TEXT NOT NULL,
            capability_id TEXT NOT NULL,
            mailbox_name TEXT NOT NULL,
            last_applied_generation_seq INTEGER,
            last_error TEXT,
            updated_at_unix INTEGER NOT NULL,
            PRIMARY KEY (repo_id, capability_id, mailbox_name)
        );
        CREATE TABLE capability_workplane_cursor_runs (
            run_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            repo_root TEXT NOT NULL,
            capability_id TEXT NOT NULL,
            mailbox_name TEXT NOT NULL,
            init_session_id TEXT,
            from_generation_seq INTEGER NOT NULL,
            to_generation_seq INTEGER NOT NULL,
            reconcile_mode TEXT NOT NULL,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            submitted_at_unix INTEGER NOT NULL,
            started_at_unix INTEGER,
            updated_at_unix INTEGER NOT NULL,
            completed_at_unix INTEGER,
            error TEXT
        );",
    )?;
    let repo_id = "repo-architecture-snapshot-init-follow-up";
    let capability_id =
        crate::capability_packs::architecture_graph::types::ARCHITECTURE_GRAPH_CAPABILITY_ID;
    let consumer_id =
        crate::capability_packs::architecture_graph::types::ARCHITECTURE_GRAPH_CONSUMER_ID;

    conn.execute(
        "INSERT INTO capability_workplane_cursor_mailboxes (
            repo_id, capability_id, mailbox_name, last_applied_generation_seq, last_error,
            updated_at_unix
         ) VALUES (?1, ?2, ?3, 20, NULL, 20)",
        params![repo_id, capability_id, consumer_id],
    )?;
    conn.execute(
        "INSERT INTO capability_workplane_cursor_generations (
            repo_id, generation_seq, source_task_id, sync_mode, active_branch, head_commit_sha,
            requires_full_reconcile, created_at_unix
         ) VALUES (?1, 21, 'sync-21', 'full', 'main', 'abc123', 1, 21)",
        params![repo_id],
    )?;
    for index in 0..2_001 {
        conn.execute(
            "INSERT INTO capability_workplane_cursor_file_changes (
                repo_id, generation_seq, path, change_kind, language, content_id
             ) VALUES (?1, 21, ?2, 'changed', 'rust', ?3)",
            params![
                repo_id,
                format!("src/generated_{index}.rs"),
                format!("content-{index}"),
            ],
        )?;
    }
    let run = CapabilityEventRunRecord {
        run_id: "run-snapshot-follow-up".to_string(),
        repo_id: repo_id.to_string(),
        capability_id: capability_id.to_string(),
        init_session_id: Some("init-1".to_string()),
        consumer_id: consumer_id.to_string(),
        handler_id: consumer_id.to_string(),
        from_generation_seq: 20,
        to_generation_seq: 21,
        reconcile_mode: "merged_delta".to_string(),
        event_kind: "current_state_consumer".to_string(),
        lane_key: format!("{repo_id}:{consumer_id}"),
        event_payload_json: String::new(),
        status: CapabilityEventRunStatus::Queued,
        attempts: 0,
        submitted_at_unix: 21,
        started_at_unix: None,
        updated_at_unix: 21,
        completed_at_unix: None,
        error: None,
    };

    let plan = build_execution_plan(&conn, &run, Path::new("/tmp/repo-architecture-snapshot"))?
        .expect("execution plan");

    assert_eq!(plan.request.reconcile_mode, ReconcileMode::MergedDelta);
    assert!(plan.request.file_upserts.is_empty());
    assert_eq!(plan.request.affected_paths.len(), 2_001);
    Ok(())
}

#[test]
fn full_reconcile_plan_normalizes_role_scope() -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE capability_workplane_cursor_generations (
            repo_id TEXT NOT NULL,
            generation_seq INTEGER NOT NULL,
            source_task_id TEXT,
            sync_mode TEXT NOT NULL,
            active_branch TEXT,
            head_commit_sha TEXT,
            requires_full_reconcile INTEGER NOT NULL DEFAULT 0,
            created_at_unix INTEGER NOT NULL,
            PRIMARY KEY (repo_id, generation_seq)
        );
        CREATE TABLE capability_workplane_cursor_file_changes (
            repo_id TEXT NOT NULL,
            generation_seq INTEGER NOT NULL,
            path TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            language TEXT,
            content_id TEXT
        );
        CREATE TABLE capability_workplane_cursor_artefact_changes (
            repo_id TEXT NOT NULL,
            generation_seq INTEGER NOT NULL,
            symbol_id TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            artefact_id TEXT NOT NULL,
            path TEXT NOT NULL,
            canonical_kind TEXT,
            name TEXT NOT NULL
        );
        CREATE TABLE capability_workplane_cursor_mailboxes (
            repo_id TEXT NOT NULL,
            capability_id TEXT NOT NULL,
            mailbox_name TEXT NOT NULL,
            last_applied_generation_seq INTEGER,
            last_error TEXT,
            updated_at_unix INTEGER NOT NULL,
            PRIMARY KEY (repo_id, capability_id, mailbox_name)
        );
        CREATE TABLE capability_workplane_cursor_runs (
            run_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            repo_root TEXT NOT NULL,
            capability_id TEXT NOT NULL,
            mailbox_name TEXT NOT NULL,
            init_session_id TEXT,
            from_generation_seq INTEGER NOT NULL,
            to_generation_seq INTEGER NOT NULL,
            reconcile_mode TEXT NOT NULL,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            submitted_at_unix INTEGER NOT NULL,
            started_at_unix INTEGER,
            updated_at_unix INTEGER NOT NULL,
            completed_at_unix INTEGER,
            error TEXT
        );",
    )?;
    let repo_id = "repo-fast-plan";
    conn.execute(
        "INSERT INTO capability_workplane_cursor_mailboxes (
            repo_id, capability_id, mailbox_name, last_applied_generation_seq, last_error,
            updated_at_unix
         ) VALUES (?1, ?2, ?3, 1, NULL, 1)",
        params![
            repo_id,
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID,
        ],
    )?;
    conn.execute(
        "INSERT INTO capability_workplane_cursor_generations (
            repo_id, generation_seq, source_task_id, sync_mode, active_branch, head_commit_sha,
            requires_full_reconcile, created_at_unix
         ) VALUES (?1, 1, 'sync-1', 'merged_delta', 'main', 'abc123', 0, 1)",
        params![repo_id],
    )?;
    for index in 0..10 {
        let path = if index % 2 == 0 {
            "src/b.rs"
        } else {
            "src/a.rs"
        };
        conn.execute(
            "INSERT INTO capability_workplane_cursor_artefact_changes (
                repo_id, generation_seq, symbol_id, change_kind, artefact_id, path,
                canonical_kind, name
             ) VALUES (?1, 1, ?2, 'changed', ?3, ?4, 'function', ?5)",
            params![
                repo_id,
                format!("symbol-{index}"),
                format!("artefact-{index}"),
                path,
                format!("function_{index}"),
            ],
        )?;
    }
    let run = CapabilityEventRunRecord {
        run_id: "run-fast-plan".to_string(),
        repo_id: repo_id.to_string(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        init_session_id: None,
        consumer_id: SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID.to_string(),
        handler_id: SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID.to_string(),
        from_generation_seq: 0,
        to_generation_seq: 1,
        reconcile_mode: "full_reconcile".to_string(),
        event_kind: "current_state_consumer".to_string(),
        lane_key: format!("{repo_id}:{SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID}"),
        event_payload_json: String::new(),
        status: CapabilityEventRunStatus::Queued,
        attempts: 0,
        submitted_at_unix: 1,
        started_at_unix: None,
        updated_at_unix: 1,
        completed_at_unix: None,
        error: None,
    };

    let plan = build_execution_plan(&conn, &run, Path::new("/tmp/repo-fast-plan"))?
        .expect("execution plan");

    assert_eq!(plan.request.reconcile_mode, ReconcileMode::FullReconcile);
    assert_eq!(plan.request.from_generation_seq_exclusive, 0);
    assert_eq!(plan.request.to_generation_seq_inclusive, 1);
    assert_eq!(plan.request.artefact_upserts.len(), 0);
    assert_eq!(plan.request.file_upserts.len(), 0);
    assert_eq!(
        plan.request.affected_paths,
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
    );
    let role_scope =
        crate::capability_packs::architecture_graph::roles::classifier::role_classification_scope_from_request(
            &plan.request,
        );
    assert!(role_scope.full_reconcile);
    assert!(role_scope.affected_paths.is_empty());
    Ok(())
}
