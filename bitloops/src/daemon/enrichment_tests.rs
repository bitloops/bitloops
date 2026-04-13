use super::*;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, RepoSqliteRuntimeStore, WorkplaneJobRecord, WorkplaneJobStatus,
};
use crate::test_support::git_fixtures::init_test_repo;
use serde_json::json;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tempfile::TempDir;
use tokio::sync::{Mutex, Notify};

fn sample_input() -> semantic_features::SemanticFeatureInput {
    semantic_features::SemanticFeatureInput {
        artefact_id: "artefact-1".to_string(),
        symbol_id: Some("symbol-1".to_string()),
        repo_id: "repo-1".to_string(),
        blob_sha: "blob-1".to_string(),
        path: "src/service.rs".to_string(),
        language: "rust".to_string(),
        canonical_kind: "function".to_string(),
        language_kind: "function".to_string(),
        symbol_fqn: "src/service.rs::load_user".to_string(),
        name: "load_user".to_string(),
        signature: Some("fn load_user(id: &str)".to_string()),
        modifiers: vec!["pub".to_string()],
        body: "load_user_impl(id)".to_string(),
        docstring: Some("Loads a user.".to_string()),
        parent_kind: None,
        dependency_signals: vec!["calls:user_store::load".to_string()],
        content_hash: Some("content-hash".to_string()),
    }
}

fn sample_input_with_artefact_id(artefact_id: &str) -> semantic_features::SemanticFeatureInput {
    let mut input = sample_input();
    input.artefact_id = artefact_id.to_string();
    input.symbol_id = Some(format!("symbol-{artefact_id}"));
    input.symbol_fqn = format!("src/service.rs::{artefact_id}");
    input.name = artefact_id.to_string();
    input
}

#[test]
fn enrichment_job_kind_serializes_lightweight_artefact_ids() {
    let job = EnrichmentJobKind::SemanticSummaries {
        artefact_ids: vec!["artefact-1".to_string()],
        input_hashes: BTreeMap::from([("artefact-1".to_string(), "hash-1".to_string())]),
        batch_key: "artefact-1".to_string(),
    };

    let value = serde_json::to_value(job).expect("serialize job kind");
    assert_eq!(
        value.get("kind").and_then(|value| value.as_str()),
        Some("semantic_summaries")
    );
    assert_eq!(
        value
            .get("artefact_ids")
            .and_then(|value| value.as_array())
            .map(|values| values.len()),
        Some(1)
    );
    assert!(value.get("inputs").is_none());
}

#[test]
fn enrichment_job_kind_deserializes_legacy_inputs_into_artefact_ids() {
    let input = sample_input();
    let job = serde_json::from_value::<EnrichmentJobKind>(json!({
        "kind": "semantic_summaries",
        "inputs": [input],
        "input_hashes": { "artefact-1": "hash-1" },
        "batch_key": "artefact-1",
        "embedding_mode": "semantic_aware_once"
    }))
    .expect("deserialize legacy job kind");

    match job {
        EnrichmentJobKind::SemanticSummaries { artefact_ids, .. } => {
            assert_eq!(artefact_ids, vec!["artefact-1".to_string()]);
        }
        other => panic!("expected semantic summaries job, got {other:?}"),
    }
}

fn sample_target(config_root: PathBuf, repo_root: PathBuf) -> EnrichmentJobTarget {
    EnrichmentJobTarget::new(config_root, repo_root)
}

fn new_test_coordinator(temp: &TempDir) -> (EnrichmentCoordinator, EnrichmentJobTarget, String) {
    let config_root = temp.path().join("config");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&config_root).expect("create test config root");
    fs::create_dir_all(&repo_root).expect("create test repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    let repo_store = RepoSqliteRuntimeStore::open_for_roots(&config_root, &repo_root)
        .expect("open repo workplane store");
    let runtime_db_path = repo_store.db_path().to_path_buf();
    let repo_id = repo_store.repo_id().to_string();
    (
        EnrichmentCoordinator {
            runtime_store: DaemonSqliteRuntimeStore::open_at(runtime_db_path.clone())
                .expect("open test daemon runtime store"),
            workplane_store: DaemonSqliteRuntimeStore::open_at(runtime_db_path)
                .expect("open test workplane store"),
            lock: Mutex::new(()),
            notify: Notify::new(),
            state_initialised: AtomicBool::new(false),
            workers_started: AtomicBool::new(false),
        },
        sample_target(config_root, repo_root),
        repo_id,
    )
}

fn load_workplane_jobs(
    coordinator: &EnrichmentCoordinator,
    status: WorkplaneJobStatus,
) -> Vec<WorkplaneJobRecord> {
    coordinator
        .workplane_store
        .with_connection(|conn| super::load_workplane_jobs_by_status(conn, status))
        .expect("load workplane jobs")
}

struct WorkplaneJobFixture<'a> {
    repo_id: &'a str,
    mailbox_name: &'a str,
    status: WorkplaneJobStatus,
    artefact_id: Option<&'a str>,
    job_id: &'a str,
    updated_at_unix: u64,
    attempts: u32,
    last_error: Option<&'a str>,
}

fn insert_workplane_job(
    coordinator: &EnrichmentCoordinator,
    target: &EnrichmentJobTarget,
    fixture: WorkplaneJobFixture<'_>,
) {
    let dedupe_key = match (fixture.mailbox_name, fixture.artefact_id) {
        (SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, _) => {
            Some(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string())
        }
        (_, Some(artefact_id)) => Some(format!("{}:{artefact_id}", fixture.mailbox_name)),
        _ => None,
    };
    let payload = fixture
        .artefact_id
        .map(|artefact_id| serde_json::json!({ "artefact_id": artefact_id }))
        .unwrap_or_else(|| serde_json::json!({}));
    let started_at_unix =
        (fixture.status == WorkplaneJobStatus::Running).then_some(fixture.updated_at_unix);
    let completed_at_unix = matches!(
        fixture.status,
        WorkplaneJobStatus::Completed | WorkplaneJobStatus::Failed
    )
    .then_some(fixture.updated_at_unix);
    coordinator
        .workplane_store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO capability_workplane_jobs (
                     job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                     dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                     started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                     lease_expires_at_unix, last_error
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, NULL, ?16)",
                rusqlite::params![
                    fixture.job_id,
                    fixture.repo_id,
                    target.repo_root.to_string_lossy().to_string(),
                    target.config_root.to_string_lossy().to_string(),
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    fixture.mailbox_name,
                    dedupe_key,
                    payload.to_string(),
                    fixture.status.as_str(),
                    fixture.attempts,
                    sql_i64(fixture.updated_at_unix)?,
                    sql_i64(fixture.updated_at_unix)?,
                    started_at_unix.map(sql_i64).transpose()?,
                    sql_i64(fixture.updated_at_unix)?,
                    completed_at_unix.map(sql_i64).transpose()?,
                    fixture.last_error,
                ],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .expect("insert workplane job");
}

fn insert_pending_artefact_jobs_bulk(
    coordinator: &EnrichmentCoordinator,
    target: &EnrichmentJobTarget,
    repo_id: &str,
    mailbox_name: &str,
    count: usize,
    submitted_at_unix: u64,
) {
    coordinator
        .workplane_store
        .with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO capability_workplane_jobs (
                         job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                         dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                         started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                         lease_expires_at_unix, last_error
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, NULL, ?12, NULL, NULL, NULL, NULL)",
                )?;
                for index in 0..count {
                    let artefact_id = format!("artefact-{index}");
                    stmt.execute(rusqlite::params![
                        format!("bulk-job-{mailbox_name}-{index}"),
                        repo_id,
                        target.repo_root.to_string_lossy().to_string(),
                        target.config_root.to_string_lossy().to_string(),
                        SEMANTIC_CLONES_CAPABILITY_ID,
                        mailbox_name,
                        format!("{mailbox_name}:{artefact_id}"),
                        serde_json::to_string(
                            &crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::Artefact { artefact_id }
                        )
                        .expect("serialize bulk artefact payload"),
                        WorkplaneJobStatus::Pending.as_str(),
                        sql_i64(submitted_at_unix)?,
                        sql_i64(submitted_at_unix)?,
                        sql_i64(submitted_at_unix)?,
                    ])?;
                }
            }
            tx.commit()?;
            Ok::<_, anyhow::Error>(())
        })
        .expect("insert bulk workplane jobs");
}

#[tokio::test]
async fn enqueue_clone_edges_rebuild_waits_for_embedding_and_semantic_jobs_to_drain() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-semantic-a"),
            job_id: "semantic-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-embedding-a"),
            job_id: "embedding-a",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-embedding-b"),
            job_id: "embedding-b",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );

    coordinator
        .enqueue_clone_edges_rebuild(target.clone())
        .await
        .expect("enqueue coalesced clone rebuild request");

    let enqueued_state = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        enqueued_state
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1
    );

    coordinator
        .enqueue_clone_edges_rebuild(target)
        .await
        .expect("dedupe clone rebuild jobs");

    let deduped_state = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(
        deduped_state
            .iter()
            .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            .count(),
        1
    );
}

#[tokio::test]
async fn enqueue_symbol_embeddings_splits_large_batches_into_smaller_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, _repo_id) = new_test_coordinator(&temp);
    let inputs = (0..(MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS + 1))
        .map(|index| sample_input_with_artefact_id(&format!("artefact-{index}")))
        .collect::<Vec<_>>();
    let input_count = inputs.len();
    let input_hashes = inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                format!("hash-{}", input.artefact_id),
            )
        })
        .collect::<BTreeMap<_, _>>();

    coordinator
        .enqueue_symbol_embeddings(
            target,
            inputs,
            input_hashes,
            EmbeddingRepresentationKind::Code,
        )
        .await
        .expect("enqueue embedding jobs");

    let embedding_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .map(|job| {
            job.payload["artefact_id"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(embedding_jobs.len(), input_count);
    assert!(
        embedding_jobs
            .iter()
            .all(|artefact_id| !artefact_id.is_empty())
    );
}

#[tokio::test]
async fn enqueue_semantic_summaries_keeps_larger_semantic_batches() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, _repo_id) = new_test_coordinator(&temp);
    let inputs = (0..(MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS + 1))
        .map(|index| sample_input_with_artefact_id(&format!("artefact-{index}")))
        .collect::<Vec<_>>();
    let input_hashes = inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                format!("hash-{}", input.artefact_id),
            )
        })
        .collect::<BTreeMap<_, _>>();

    coordinator
        .enqueue_semantic_summaries(target, inputs, input_hashes)
        .await
        .expect("enqueue semantic jobs");

    let semantic_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        .map(|job| {
            job.payload["artefact_id"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        semantic_jobs.len(),
        MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS + 1
    );
    assert!(
        semantic_jobs
            .iter()
            .all(|artefact_id| !artefact_id.is_empty())
    );
}

#[test]
fn requeue_running_jobs_moves_stale_running_jobs_back_to_pending() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);

    coordinator
        .runtime_store
        .save_enrichment_queue_state(&default_state())
        .expect("write initial control state");
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-semantic-a"),
            job_id: "semantic-a",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-embedding-a"),
            job_id: "embedding-a",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-embedding-b"),
            job_id: "embedding-b",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    coordinator.requeue_running_jobs();

    let recovered_running = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Running);
    let recovered_pending = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(recovered_running.len(), 0);
    assert_eq!(recovered_pending.len(), 3);
    assert_eq!(
        coordinator
            .runtime_store
            .load_enrichment_queue_state()
            .expect("read recovered control state")
            .expect("state exists")
            .last_action
            .as_deref(),
        Some("requeue_running")
    );
}

#[test]
fn ensure_started_recovers_stale_running_jobs_on_startup() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let coordinator = Arc::new(coordinator);

    coordinator
        .runtime_store
        .save_enrichment_queue_state(&default_state())
        .expect("write initial control state");
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-semantic-a"),
            job_id: "semantic-a",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Running,
            artefact_id: Some("artefact-embedding-a"),
            job_id: "embedding-a",
            updated_at_unix: 1,
            attempts: 1,
            last_error: None,
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Pending,
            artefact_id: Some("artefact-embedding-b"),
            job_id: "embedding-b",
            updated_at_unix: 1,
            attempts: 0,
            last_error: None,
        },
    );

    coordinator.ensure_started();

    let recovered_running = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Running);
    let recovered_pending = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending);
    assert_eq!(recovered_running.len(), 0);
    assert_eq!(recovered_pending.len(), 3);
    assert_eq!(
        coordinator
            .runtime_store
            .load_enrichment_queue_state()
            .expect("read recovered control state")
            .expect("state exists")
            .last_action
            .as_deref(),
        Some("requeue_running")
    );
}

#[test]
fn snapshot_projects_last_failed_embedding_job_details() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: Some("artefact-older"),
            job_id: "embedding-older",
            updated_at_unix: 10,
            attempts: 1,
            last_error: Some("older failure"),
        },
    );
    insert_workplane_job(
        &coordinator,
        &target,
        WorkplaneJobFixture {
            repo_id: &repo_id,
            mailbox_name: SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            status: WorkplaneJobStatus::Failed,
            artefact_id: Some("artefact-newer"),
            job_id: "embedding-newer",
            updated_at_unix: 20,
            attempts: 3,
            last_error: Some("[capability_host:timeout] capability ingester timed out after 300s"),
        },
    );

    let summary = super::last_failed_embedding_job_from_workplane(&coordinator.workplane_store)
        .expect("read failed embedding summary")
        .expect("failed embedding summary");
    assert_eq!(summary.job_id, "embedding-newer");
    assert_eq!(summary.repo_id, repo_id);
    assert_eq!(summary.branch, "unknown");
    assert_eq!(summary.representation_kind, "code");
    assert_eq!(summary.artefact_count, 1);
    assert_eq!(summary.attempts, 3);
    assert_eq!(
        summary.error.as_deref(),
        Some("[capability_host:timeout] capability ingester timed out after 300s")
    );
    assert_eq!(summary.updated_at_unix, 20);
}

#[test]
fn compaction_replaces_large_old_pending_embedding_backlog_with_repo_backfill_job() {
    let temp = TempDir::new().expect("temp dir");
    let (coordinator, target, repo_id) = new_test_coordinator(&temp);
    let pending_count = usize::try_from(WORKPLANE_PENDING_COMPACTION_MIN_COUNT)
        .expect("pending compaction threshold fits usize");
    insert_pending_artefact_jobs_bulk(
        &coordinator,
        &target,
        &repo_id,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        pending_count,
        1,
    );

    super::compact_and_prune_workplane_jobs(&coordinator.workplane_store)
        .expect("compact pending workplane backlog");

    let pending_jobs = load_workplane_jobs(&coordinator, WorkplaneJobStatus::Pending)
        .into_iter()
        .filter(|job| job.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        .collect::<Vec<_>>();
    let expected_dedupe_key =
        crate::capability_packs::semantic_clones::workplane::repo_backfill_dedupe_key(
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        );

    assert_eq!(
        pending_jobs.len(),
        1,
        "artefact backlog should compact to a single repo backfill job"
    );
    assert_eq!(
        pending_jobs[0].dedupe_key.as_deref(),
        Some(expected_dedupe_key.as_str())
    );
    assert!(
        crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
            &pending_jobs[0].payload
        ),
        "pending job should be converted to a repo backfill payload"
    );
}
