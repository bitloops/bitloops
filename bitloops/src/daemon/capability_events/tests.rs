use crate::host::capability_host::{ChangedFile, ReconcileMode, RemovedArtefact, RemovedFile};

use super::plan::{
    MergedArtefactChange, MergedFileChange, determine_reconcile_mode, merge_artefact_changes,
    merge_file_changes,
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
