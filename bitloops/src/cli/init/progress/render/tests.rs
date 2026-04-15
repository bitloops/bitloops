use super::super::{BottomProgressState, InitChecklistState, SummaryProgressState};
use super::frame::InitProgressRenderer;

fn non_interactive_renderer() -> InitProgressRenderer {
    InitProgressRenderer {
        interactive: false,
        terminal_width: None,
        spinner_index: 0,
        last_frame: None,
        wrote_in_place: false,
        rendered_lines: 0,
    }
}

fn sync_task(
    paths_total: i32,
    paths_completed: i32,
) -> crate::cli::devql::graphql::TaskGraphqlRecord {
    serde_json::from_value(serde_json::json!({
        "taskId": "sync-task-1",
        "repoId": "repo-1",
        "repoName": "demo",
        "repoIdentity": "local/demo",
        "kind": "SYNC",
        "source": "init",
        "status": "RUNNING",
        "submittedAtUnix": 1,
        "startedAtUnix": 1,
        "updatedAtUnix": 2,
        "completedAtUnix": null,
        "queuePosition": null,
        "tasksAhead": null,
        "error": null,
        "syncSpec": {
            "mode": "auto",
            "paths": []
        },
        "ingestSpec": null,
        "embeddingsBootstrapSpec": null,
        "syncProgress": {
            "phase": "extracting_paths",
            "currentPath": "src/lib.rs",
            "pathsTotal": paths_total,
            "pathsCompleted": paths_completed,
            "pathsRemaining": paths_total.saturating_sub(paths_completed),
            "pathsUnchanged": 0,
            "pathsAdded": 0,
            "pathsChanged": 0,
            "pathsRemoved": 0,
            "cacheHits": 0,
            "cacheMisses": 0,
            "parseErrors": 0
        },
        "ingestProgress": null,
        "embeddingsBootstrapProgress": null,
        "syncResult": null,
        "ingestResult": null,
        "embeddingsBootstrapResult": null
    }))
    .expect("build sync task")
}

fn ingest_task(
    commits_total: i32,
    commits_processed: i32,
) -> crate::cli::devql::graphql::TaskGraphqlRecord {
    serde_json::from_value(serde_json::json!({
        "taskId": "ingest-task-1",
        "repoId": "repo-1",
        "repoName": "demo",
        "repoIdentity": "local/demo",
        "kind": "INGEST",
        "source": "init",
        "status": "RUNNING",
        "submittedAtUnix": 1,
        "startedAtUnix": 1,
        "updatedAtUnix": 2,
        "completedAtUnix": null,
        "queuePosition": null,
        "tasksAhead": null,
        "error": null,
        "syncSpec": null,
        "ingestSpec": {
            "backfill": 50
        },
        "embeddingsBootstrapSpec": null,
        "syncProgress": null,
        "ingestProgress": {
            "phase": "extracting",
            "commitsTotal": commits_total,
            "commitsProcessed": commits_processed,
            "checkpointCompanionsProcessed": 0,
            "currentCheckpointId": null,
            "currentCommitSha": "abc123",
            "eventsInserted": 0,
            "artefactsUpserted": 0
        },
        "embeddingsBootstrapProgress": null,
        "syncResult": null,
        "ingestResult": null,
        "embeddingsBootstrapResult": null
    }))
    .expect("build ingest task")
}

#[test]
fn render_waiting_lanes_do_not_report_completion_early() {
    let renderer = non_interactive_renderer();
    let frame = renderer.render_frame(
        InitChecklistState {
            show_sync: true,
            show_ingest: true,
            show_embeddings: true,
            show_summaries: true,
            sync_complete: false,
            ingest_complete: false,
        },
        None,
        &BottomProgressState::WaitingForQueue {
            baseline_total: 0,
            completed_floor: 0,
            completed_jobs: 0,
            failed_jobs: 0,
        },
        &SummaryProgressState::WaitingForQueue {
            result: crate::cli::inference::SummarySetupExecutionResult {
                outcome: crate::cli::inference::SummarySetupOutcome::Configured {
                    model_name: "ministral-3:3b".to_string(),
                },
                message: "Configured semantic summaries to use Ollama model `ministral-3:3b`."
                    .to_string(),
            },
            baseline_total: 0,
            completed_floor: 0,
            completed_jobs: 0,
            failed_jobs: 0,
        },
    );

    assert!(frame.contains("waiting for sync and ingest"));
    assert!(!frame.contains("Embedding queue complete"));
    assert!(!frame.contains("Semantic summary queue complete"));
}

#[test]
fn render_ingest_lane_while_sync_is_running() {
    let renderer = non_interactive_renderer();
    let sync_task = sync_task(12, 3);
    let frame = renderer.render_frame(
        InitChecklistState {
            show_sync: true,
            show_ingest: true,
            show_embeddings: false,
            show_summaries: false,
            sync_complete: false,
            ingest_complete: false,
        },
        Some(&sync_task),
        &BottomProgressState::Hidden,
        &SummaryProgressState::Hidden,
    );

    assert!(frame.contains("Waiting for sync to finish before starting ingest"));
    assert!(frame.contains("Analysing your current branch to know what's what"));
    assert!(frame.contains("Analysing your git history because you know... history is important"));
}

#[test]
fn render_completed_sync_lane_while_ingest_continues() {
    let renderer = non_interactive_renderer();
    let ingest_task = ingest_task(20, 5);
    let frame = renderer.render_frame(
        InitChecklistState {
            show_sync: true,
            show_ingest: true,
            show_embeddings: false,
            show_summaries: false,
            sync_complete: true,
            ingest_complete: false,
        },
        Some(&ingest_task),
        &BottomProgressState::Hidden,
        &SummaryProgressState::Hidden,
    );

    assert!(frame.contains("Sync complete"));
    assert!(frame.contains("100% complete"));
    assert!(frame.contains("Ingesting demo"));
    assert!(!frame.contains("Waiting for sync to finish before starting ingest"));
}
