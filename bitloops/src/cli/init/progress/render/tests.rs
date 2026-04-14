use super::super::{BottomProgressState, InitChecklistState, SummaryProgressState};
use super::frame::InitProgressRenderer;

#[test]
fn render_waiting_lanes_do_not_report_completion_early() {
    let renderer = InitProgressRenderer {
        interactive: false,
        terminal_width: None,
        spinner_index: 0,
        last_frame: None,
        wrote_in_place: false,
        rendered_lines: 0,
    };
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
