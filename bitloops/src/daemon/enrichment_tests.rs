use super::*;
use crate::host::runtime_store::DaemonSqliteRuntimeStore;
use serde_json::json;
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

fn sample_target(repo_id: &str) -> EnrichmentJobTarget {
    EnrichmentJobTarget::new(
        PathBuf::from("/tmp/config"),
        PathBuf::from("/tmp/repo"),
        repo_id.to_string(),
        "main".to_string(),
    )
}

fn sample_embedding_job(
    repo_id: &str,
    status: EnrichmentJobStatus,
    batch_key: &str,
) -> EnrichmentJob {
    EnrichmentJob {
        id: format!("embedding-{batch_key}"),
        repo_id: repo_id.to_string(),
        repo_root: PathBuf::from("/tmp/repo"),
        config_root: PathBuf::from("/tmp/config"),
        branch: "main".to_string(),
        status,
        attempts: 0,
        error: None,
        created_at_unix: 1,
        updated_at_unix: 1,
        job: EnrichmentJobKind::SymbolEmbeddings {
            artefact_ids: vec![format!("artefact-{batch_key}")],
            input_hashes: BTreeMap::from([(format!("artefact-{batch_key}"), "hash".to_string())]),
            batch_key: batch_key.to_string(),
            representation_kind: EmbeddingRepresentationKind::Baseline,
        },
    }
}

fn sample_semantic_job(
    repo_id: &str,
    status: EnrichmentJobStatus,
    batch_key: &str,
) -> EnrichmentJob {
    EnrichmentJob {
        id: format!("semantic-{batch_key}"),
        repo_id: repo_id.to_string(),
        repo_root: PathBuf::from("/tmp/repo"),
        config_root: PathBuf::from("/tmp/config"),
        branch: "main".to_string(),
        status,
        attempts: 0,
        error: None,
        created_at_unix: 1,
        updated_at_unix: 1,
        job: EnrichmentJobKind::SemanticSummaries {
            artefact_ids: vec![format!("artefact-{batch_key}")],
            input_hashes: BTreeMap::from([(format!("artefact-{batch_key}"), "hash".to_string())]),
            batch_key: batch_key.to_string(),
        },
    }
}

fn new_test_coordinator(runtime_db_path: PathBuf) -> EnrichmentCoordinator {
    EnrichmentCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(runtime_db_path)
            .expect("open test daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
    }
}

#[tokio::test]
async fn enqueue_clone_edges_rebuild_waits_for_embedding_and_semantic_jobs_to_drain() {
    let temp = TempDir::new().expect("temp dir");
    let runtime_db_path = temp.path().join("runtime.sqlite");
    let coordinator = new_test_coordinator(runtime_db_path);
    let repo_id = "repo-1";

    let mut initial_state = default_state();
    initial_state.jobs = vec![
        sample_semantic_job(repo_id, EnrichmentJobStatus::Pending, "semantic-a"),
        sample_embedding_job(repo_id, EnrichmentJobStatus::Pending, "embedding-a"),
        sample_embedding_job(repo_id, EnrichmentJobStatus::Running, "embedding-b"),
    ];
    coordinator
        .runtime_store
        .save_enrichment_queue_state(&initial_state)
        .expect("write initial enrichment state");

    coordinator
        .enqueue_clone_edges_rebuild(sample_target(repo_id))
        .await
        .expect("defer clone rebuild while embedding producers remain");

    let deferred_state = coordinator
        .runtime_store
        .load_enrichment_queue_state()
        .expect("read deferred state")
        .expect("state exists");
    assert!(
        deferred_state
            .jobs
            .iter()
            .all(|job| !matches!(job.job, EnrichmentJobKind::CloneEdgesRebuild { .. }))
    );
    assert_eq!(
        deferred_state.last_action.as_deref(),
        Some("defer_clone_edges_rebuild")
    );

    let mut drained_state = deferred_state.clone();
    for job in &mut drained_state.jobs {
        job.status = EnrichmentJobStatus::Completed;
    }
    coordinator
        .runtime_store
        .save_enrichment_queue_state(&drained_state)
        .expect("write drained state");

    coordinator
        .enqueue_clone_edges_rebuild(sample_target(repo_id))
        .await
        .expect("enqueue clone rebuild after producers drain");

    let enqueued_state = coordinator
        .runtime_store
        .load_enrichment_queue_state()
        .expect("read enqueued state")
        .expect("state exists");
    assert_eq!(
        enqueued_state
            .jobs
            .iter()
            .filter(|job| matches!(job.job, EnrichmentJobKind::CloneEdgesRebuild { .. }))
            .count(),
        1
    );

    coordinator
        .enqueue_clone_edges_rebuild(sample_target(repo_id))
        .await
        .expect("dedupe clone rebuild jobs");

    let deduped_state = coordinator
        .runtime_store
        .load_enrichment_queue_state()
        .expect("read deduped state")
        .expect("state exists");
    assert_eq!(
        deduped_state
            .jobs
            .iter()
            .filter(|job| matches!(job.job, EnrichmentJobKind::CloneEdgesRebuild { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn enqueue_symbol_embeddings_splits_large_batches_into_smaller_jobs() {
    let temp = TempDir::new().expect("temp dir");
    let runtime_db_path = temp.path().join("runtime.sqlite");
    let coordinator = new_test_coordinator(runtime_db_path);
    let repo_id = "repo-1";
    let inputs = (0..(MAX_ENRICHMENT_JOB_ARTEFACTS + 1))
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
        .enqueue_symbol_embeddings(
            sample_target(repo_id),
            inputs,
            input_hashes,
            EmbeddingRepresentationKind::Baseline,
        )
        .await
        .expect("enqueue embedding jobs");

    let state = coordinator
        .runtime_store
        .load_enrichment_queue_state()
        .expect("read enrichment state")
        .expect("state exists");
    let embedding_jobs = state
        .jobs
        .iter()
        .filter_map(|job| match &job.job {
            EnrichmentJobKind::SymbolEmbeddings { artefact_ids, .. } => Some(artefact_ids),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(embedding_jobs.len(), 2);
    assert_eq!(embedding_jobs[0].len(), MAX_ENRICHMENT_JOB_ARTEFACTS);
    assert_eq!(embedding_jobs[1].len(), 1);
}

#[test]
fn requeue_running_jobs_moves_stale_running_jobs_back_to_pending() {
    let temp = TempDir::new().expect("temp dir");
    let runtime_db_path = temp.path().join("runtime.sqlite");
    let coordinator = new_test_coordinator(runtime_db_path);
    let repo_id = "repo-1";

    let mut initial_state = default_state();
    initial_state.jobs = vec![
        sample_semantic_job(repo_id, EnrichmentJobStatus::Running, "semantic-a"),
        sample_embedding_job(repo_id, EnrichmentJobStatus::Running, "embedding-a"),
        sample_embedding_job(repo_id, EnrichmentJobStatus::Pending, "embedding-b"),
    ];
    coordinator
        .runtime_store
        .save_enrichment_queue_state(&initial_state)
        .expect("write initial enrichment state");

    coordinator.requeue_running_jobs();

    let recovered_state = coordinator
        .runtime_store
        .load_enrichment_queue_state()
        .expect("read recovered state")
        .expect("state exists");
    assert_eq!(
        recovered_state
            .jobs
            .iter()
            .filter(|job| job.status == EnrichmentJobStatus::Running)
            .count(),
        0
    );
    assert_eq!(
        recovered_state
            .jobs
            .iter()
            .filter(|job| job.status == EnrichmentJobStatus::Pending)
            .count(),
        3
    );
    assert_eq!(
        recovered_state.last_action.as_deref(),
        Some("requeue_running")
    );
}
