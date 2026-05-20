use super::*;
use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;
use crate::host::runtime_store::RepoSqliteRuntimeStore;
use crate::test_support::process_state::enter_process_state;
use tempfile::TempDir;

fn open_test_stores() -> Result<(
    TempDir,
    RepoSqliteRuntimeStore,
    DaemonSqliteRuntimeStore,
    DaemonSqliteRuntimeStore,
)> {
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let config_root = repo_root.clone();
    let runtime_script_path = repo_root.join("fake-embeddings-runtime.sh");
    std::fs::write(
        &runtime_script_path,
        r#"launch_log="$1"
shift
printf '%s\n' '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}'

while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true}\n' "$request_id"
      exit 0
      ;;
    *'"cmd":"embed"'*)
      printf '{"id":"%s","ok":true,"vectors":[[1.0,2.0]]}\n' "$request_id"
      ;;
  esac
done
"#,
    )
    .expect("write fake embeddings runtime script");
    let config_path = config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).expect("create config parent");
    }
    std::fs::write(
        &config_path,
        format!(
            r#"[semantic_clones]

[semantic_clones.inference]
summary_generation = "summary_remote"
code_embeddings = "code_remote"
summary_embeddings = "summary_embed_remote"

[inference.profiles.summary_remote]
task = "text_generation"
driver = "openai_chat_completions"
base_url = "https://example.invalid"
model = "summary"

[inference.runtimes.external_embeddings]
command = "/bin/sh"
args = [{runtime_script_path:?}]
startup_timeout_secs = 1
request_timeout_secs = 1

[inference.profiles.code_remote]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "external_embeddings"
model = "code"

[inference.profiles.summary_embed_remote]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "external_embeddings"
model = "summary"
"#,
            runtime_script_path = runtime_script_path.display().to_string(),
        ),
    )
    .expect("write mailbox claim test config");

    let repo_store =
        RepoSqliteRuntimeStore::open_for_roots_with_repo_id(&config_root, &repo_root, "repo-1")
            .expect("open repo runtime store");
    let workplane_store = DaemonSqliteRuntimeStore::open_at(repo_store.db_path().to_path_buf())
        .expect("open workplane runtime store");
    let runtime_store = DaemonSqliteRuntimeStore::open_at(temp.path().join("runtime.sqlite"))
        .expect("open daemon runtime store");
    Ok((temp, repo_store, workplane_store, runtime_store))
}

fn insert_embedding_mailbox_item(
    workplane_store: &DaemonSqliteRuntimeStore,
    repo_store: &RepoSqliteRuntimeStore,
    item_id: &str,
    representation_kind: EmbeddingRepresentationKind,
    status: SemanticMailboxItemStatus,
    submitted_at_unix: u64,
) -> Result<()> {
    workplane_store.with_write_connection(|conn| {
        conn.execute(
            "INSERT INTO semantic_embedding_mailbox_items (
                item_id, repo_id, repo_root, config_root, init_session_id,
                representation_kind, item_kind, artefact_id, payload_json,
                dedupe_key, status, attempts, available_at_unix,
                submitted_at_unix, leased_at_unix, lease_expires_at_unix,
                lease_token, updated_at_unix, last_error
             ) VALUES (
                ?1, ?2, ?3, ?4, NULL,
                ?5, ?6, NULL, NULL,
                ?7, ?8, ?9, ?10,
                ?11, ?12, ?13,
                ?14, ?15, NULL
             )",
            params![
                item_id,
                &repo_store.repo_id,
                repo_store.repo_root.to_string_lossy().to_string(),
                repo_store.config_root.to_string_lossy().to_string(),
                representation_kind.to_string(),
                SemanticMailboxItemKind::RepoBackfill.as_str(),
                item_id,
                status.as_str(),
                if status == SemanticMailboxItemStatus::Leased {
                    1_u32
                } else {
                    0_u32
                },
                sql_i64(submitted_at_unix)?,
                sql_i64(submitted_at_unix)?,
                if status == SemanticMailboxItemStatus::Leased {
                    Some(sql_i64(submitted_at_unix)?)
                } else {
                    None
                },
                if status == SemanticMailboxItemStatus::Leased {
                    Some(sql_i64(submitted_at_unix.saturating_add(300))?)
                } else {
                    None
                },
                if status == SemanticMailboxItemStatus::Leased {
                    Some(format!("lease-{item_id}"))
                } else {
                    None
                },
                sql_i64(submitted_at_unix)?,
            ],
        )
        .context("insert embedding mailbox item")?;
        Ok(())
    })
}

fn insert_active_summary_refresh_item(
    workplane_store: &DaemonSqliteRuntimeStore,
    repo_store: &RepoSqliteRuntimeStore,
    item_id: &str,
    submitted_at_unix: u64,
) -> Result<()> {
    workplane_store.with_write_connection(|conn| {
        conn.execute(
            "INSERT INTO semantic_summary_mailbox_items (
                item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
                submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
                updated_at_unix, last_error
             ) VALUES (
                ?1, ?2, ?3, ?4, NULL, ?5,
                NULL, NULL, ?6, ?7, 0, ?8,
                ?9, NULL, NULL, NULL,
                ?10, NULL
             )",
            params![
                item_id,
                &repo_store.repo_id,
                repo_store.repo_root.to_string_lossy().to_string(),
                repo_store.config_root.to_string_lossy().to_string(),
                SemanticMailboxItemKind::RepoBackfill.as_str(),
                item_id,
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(submitted_at_unix)?,
                sql_i64(submitted_at_unix)?,
                sql_i64(submitted_at_unix)?,
            ],
        )
        .context("insert active summary refresh item")?;
        Ok(())
    })
}

fn delete_embedding_batch(
    workplane_store: &DaemonSqliteRuntimeStore,
    lease_token: &str,
) -> Result<()> {
    workplane_store.with_write_connection(|conn| {
        conn.execute(
            "DELETE FROM semantic_embedding_mailbox_items WHERE lease_token = ?1",
            params![lease_token],
        )
        .context("delete embedding mailbox batch")?;
        Ok(())
    })
}

fn assert_no_blocked_mailboxes(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
) -> Result<()> {
    let blocked = super::super::readiness::current_workplane_mailbox_blocked_statuses_for_repo(
        workplane_store,
        runtime_store,
        "repo-1",
    )?
    .into_iter()
    .filter(|status| {
        status.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
            || status.mailbox_name == SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
    })
    .collect::<Vec<_>>();
    assert!(
        blocked.is_empty(),
        "embedding mailboxes should be ready for claims in this fixture: {blocked:?}"
    );
    Ok(())
}

#[test]
fn single_worker_summary_overlap_alternates_after_non_summary_claim() -> Result<()> {
    let _guard = enter_process_state(
        None,
        &[("BITLOOPS_SEMANTIC_CLONES_EMBEDDING_WORKERS", Some("1"))],
    );
    let (_temp, repo_store, workplane_store, runtime_store) = open_test_stores()?;

    insert_active_summary_refresh_item(&workplane_store, &repo_store, "summary-refresh-1", 1)?;
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "code-batch-1",
        EmbeddingRepresentationKind::Code,
        SemanticMailboxItemStatus::Pending,
        10,
    )?;
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "code-batch-2",
        EmbeddingRepresentationKind::Code,
        SemanticMailboxItemStatus::Pending,
        11,
    )?;
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "summary-batch-1",
        EmbeddingRepresentationKind::Summary,
        SemanticMailboxItemStatus::Pending,
        20,
    )?;
    assert_no_blocked_mailboxes(&workplane_store, &runtime_store)?;

    let first = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected first embedding claim");
    assert_eq!(first.representation_kind, EmbeddingRepresentationKind::Code);

    delete_embedding_batch(&workplane_store, &first.lease_token)?;

    let state = EnrichmentControlState {
        last_embedding_claim_kind: Some(EmbeddingRepresentationKind::Code),
        ..EnrichmentControlState::default()
    };
    let second = claim_embedding_mailbox_batch(&workplane_store, &runtime_store, &state)?
        .expect("expected second embedding claim");

    assert_eq!(
        second.representation_kind,
        EmbeddingRepresentationKind::Summary
    );
    Ok(())
}

#[test]
fn multi_worker_summary_overlap_leases_summary_code_and_identity_in_parallel() -> Result<()> {
    let _guard = enter_process_state(
        None,
        &[("BITLOOPS_SEMANTIC_CLONES_EMBEDDING_WORKERS", Some("3"))],
    );
    let (_temp, repo_store, workplane_store, runtime_store) = open_test_stores()?;

    insert_active_summary_refresh_item(&workplane_store, &repo_store, "summary-refresh-1", 1)?;
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "code-batch-1",
        EmbeddingRepresentationKind::Code,
        SemanticMailboxItemStatus::Pending,
        10,
    )?;
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "summary-batch-1",
        EmbeddingRepresentationKind::Summary,
        SemanticMailboxItemStatus::Pending,
        20,
    )?;
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "identity-batch-1",
        EmbeddingRepresentationKind::Identity,
        SemanticMailboxItemStatus::Pending,
        30,
    )?;
    assert_no_blocked_mailboxes(&workplane_store, &runtime_store)?;

    let first = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected first embedding claim");
    assert_eq!(
        first.representation_kind,
        EmbeddingRepresentationKind::Summary
    );

    let second = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected second embedding claim");
    assert_eq!(
        second.representation_kind,
        EmbeddingRepresentationKind::Code
    );

    let third = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected third embedding claim");
    assert_eq!(
        third.representation_kind,
        EmbeddingRepresentationKind::Identity
    );
    Ok(())
}

#[test]
fn multi_worker_summary_overlap_limits_summary_priority_to_one_worker_slot() -> Result<()> {
    let _guard = enter_process_state(
        None,
        &[("BITLOOPS_SEMANTIC_CLONES_EMBEDDING_WORKERS", Some("4"))],
    );
    let (_temp, repo_store, workplane_store, runtime_store) = open_test_stores()?;

    insert_active_summary_refresh_item(&workplane_store, &repo_store, "summary-refresh-1", 1)?;
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "code-batch-1",
        EmbeddingRepresentationKind::Code,
        SemanticMailboxItemStatus::Pending,
        10,
    )?;
    for index in 0..33 {
        insert_embedding_mailbox_item(
            &workplane_store,
            &repo_store,
            &format!("summary-batch-{index:02}"),
            EmbeddingRepresentationKind::Summary,
            SemanticMailboxItemStatus::Pending,
            20 + index,
        )?;
    }
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "identity-batch-1",
        EmbeddingRepresentationKind::Identity,
        SemanticMailboxItemStatus::Pending,
        60,
    )?;
    assert_no_blocked_mailboxes(&workplane_store, &runtime_store)?;

    let first = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected first embedding claim");
    assert_eq!(
        first.representation_kind,
        EmbeddingRepresentationKind::Summary
    );

    let second = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected second embedding claim");
    assert_eq!(
        second.representation_kind,
        EmbeddingRepresentationKind::Code,
        "only one worker slot should prioritize summary while summary refresh is active",
    );

    let third = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected third embedding claim");
    assert_eq!(
        third.representation_kind,
        EmbeddingRepresentationKind::Identity,
        "remaining overlap workers should move to non-summary embeddings before another summary batch",
    );
    Ok(())
}

#[test]
fn multi_worker_summary_overlap_refills_non_summary_before_reclaiming_summary_slot() -> Result<()> {
    let _guard = enter_process_state(
        None,
        &[("BITLOOPS_SEMANTIC_CLONES_EMBEDDING_WORKERS", Some("4"))],
    );
    let (_temp, repo_store, workplane_store, runtime_store) = open_test_stores()?;

    insert_active_summary_refresh_item(&workplane_store, &repo_store, "summary-refresh-1", 1)?;
    for index in 0..64 {
        insert_embedding_mailbox_item(
            &workplane_store,
            &repo_store,
            &format!("code-batch-{index:02}"),
            EmbeddingRepresentationKind::Code,
            SemanticMailboxItemStatus::Pending,
            10 + index,
        )?;
    }
    for index in 0..32 {
        insert_embedding_mailbox_item(
            &workplane_store,
            &repo_store,
            &format!("identity-batch-{index:02}"),
            EmbeddingRepresentationKind::Identity,
            SemanticMailboxItemStatus::Pending,
            100 + index,
        )?;
    }
    for index in 0..64 {
        insert_embedding_mailbox_item(
            &workplane_store,
            &repo_store,
            &format!("summary-batch-{index:02}"),
            EmbeddingRepresentationKind::Summary,
            SemanticMailboxItemStatus::Pending,
            200 + index,
        )?;
    }
    assert_no_blocked_mailboxes(&workplane_store, &runtime_store)?;

    let first = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected first embedding claim");
    assert_eq!(
        first.representation_kind,
        EmbeddingRepresentationKind::Summary
    );

    let second = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected second embedding claim");
    assert_eq!(
        second.representation_kind,
        EmbeddingRepresentationKind::Code
    );

    let third = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected third embedding claim");
    assert_eq!(
        third.representation_kind,
        EmbeddingRepresentationKind::Identity
    );

    delete_embedding_batch(&workplane_store, &first.lease_token)?;

    let fourth = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected fourth embedding claim");
    assert_eq!(
        fourth.representation_kind,
        EmbeddingRepresentationKind::Code,
        "summary should not reclaim its freed slot while non-summary overlap capacity is still below three leased batches",
    );
    Ok(())
}

#[test]
fn single_worker_without_active_summary_refresh_keeps_fifo_code_backlog() -> Result<()> {
    let _guard = enter_process_state(
        None,
        &[("BITLOOPS_SEMANTIC_CLONES_EMBEDDING_WORKERS", Some("1"))],
    );
    let (_temp, repo_store, workplane_store, runtime_store) = open_test_stores()?;

    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "code-batch-1",
        EmbeddingRepresentationKind::Code,
        SemanticMailboxItemStatus::Pending,
        10,
    )?;
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "code-batch-2",
        EmbeddingRepresentationKind::Code,
        SemanticMailboxItemStatus::Pending,
        11,
    )?;
    insert_embedding_mailbox_item(
        &workplane_store,
        &repo_store,
        "summary-batch-1",
        EmbeddingRepresentationKind::Summary,
        SemanticMailboxItemStatus::Pending,
        20,
    )?;
    assert_no_blocked_mailboxes(&workplane_store, &runtime_store)?;

    let first = claim_embedding_mailbox_batch(
        &workplane_store,
        &runtime_store,
        &EnrichmentControlState::default(),
    )?
    .expect("expected first embedding claim");
    assert_eq!(first.representation_kind, EmbeddingRepresentationKind::Code);

    delete_embedding_batch(&workplane_store, &first.lease_token)?;

    let state = EnrichmentControlState {
        last_embedding_claim_kind: Some(EmbeddingRepresentationKind::Code),
        ..EnrichmentControlState::default()
    };
    let second = claim_embedding_mailbox_batch(&workplane_store, &runtime_store, &state)?
        .expect("expected second embedding claim");

    assert_eq!(
        second.representation_kind,
        EmbeddingRepresentationKind::Code
    );
    Ok(())
}
