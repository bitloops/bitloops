use std::collections::HashMap;

use crate::host::devql::checkpoint_provenance::{
    CheckpointFileProvenanceDetailRow, checkpoint_display_path,
};

use super::dashboard_types::DashboardCommitFileDiff;

pub(crate) fn dashboard_file_diff_list_from_numstat(
    stats: HashMap<String, (u64, u64)>,
) -> Vec<DashboardCommitFileDiff> {
    let mut files_touched: Vec<DashboardCommitFileDiff> = stats
        .into_iter()
        .map(|(filepath, (adds, dels))| DashboardCommitFileDiff {
            filepath,
            additions_count: adds,
            deletions_count: dels,
            change_kind: None,
            copied_from_path: None,
            copied_from_blob_sha: None,
        })
        .collect();
    files_touched.sort_by(|left, right| left.filepath.cmp(&right.filepath));
    files_touched
}

pub(crate) fn dashboard_zeroed_file_diff_list(
    files_touched: &[String],
) -> Vec<DashboardCommitFileDiff> {
    let mut files_touched: Vec<DashboardCommitFileDiff> = files_touched
        .iter()
        .cloned()
        .map(|filepath| DashboardCommitFileDiff {
            filepath,
            additions_count: 0,
            deletions_count: 0,
            change_kind: None,
            copied_from_path: None,
            copied_from_blob_sha: None,
        })
        .collect();
    files_touched.sort_by(|left, right| left.filepath.cmp(&right.filepath));
    files_touched
}

pub(crate) fn dashboard_checkpoint_file_diff_list_from_relations(
    relations: &[CheckpointFileProvenanceDetailRow],
    stats: Option<&HashMap<String, (u64, u64)>>,
) -> Vec<DashboardCommitFileDiff> {
    let mut files_touched = relations
        .iter()
        .filter_map(|relation| {
            let filepath = checkpoint_display_path(
                relation.path_before.as_deref(),
                relation.path_after.as_deref(),
            );
            if filepath.is_empty() {
                return None;
            }
            let (additions_count, deletions_count) = stats
                .and_then(|values| values.get(&filepath).copied())
                .unwrap_or((0, 0));
            Some(DashboardCommitFileDiff {
                filepath,
                additions_count,
                deletions_count,
                change_kind: Some(relation.change_kind.as_str().to_string()),
                copied_from_path: relation.copy_source_path.clone(),
                copied_from_blob_sha: relation.copy_source_blob_sha.clone(),
            })
        })
        .collect::<Vec<_>>();
    files_touched.sort_by(|left, right| left.filepath.cmp(&right.filepath));
    files_touched
}
