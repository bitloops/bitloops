use std::collections::HashMap;

use super::super::dto::ApiCommitFileDiffDto;
use crate::host::devql::checkpoint_provenance::{
    CheckpointFileProvenanceDetailRow, checkpoint_display_path,
};

pub(super) fn api_file_diff_list_from_numstat(
    stats: HashMap<String, (u64, u64)>,
) -> Vec<ApiCommitFileDiffDto> {
    let mut files_touched: Vec<ApiCommitFileDiffDto> = stats
        .into_iter()
        .map(|(filepath, (adds, dels))| ApiCommitFileDiffDto {
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

pub(super) fn api_zeroed_file_diff_list(files_touched: &[String]) -> Vec<ApiCommitFileDiffDto> {
    let mut files_touched: Vec<ApiCommitFileDiffDto> = files_touched
        .iter()
        .cloned()
        .map(|filepath| ApiCommitFileDiffDto {
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

pub(super) fn api_checkpoint_file_diff_list_from_relations(
    relations: &[CheckpointFileProvenanceDetailRow],
    stats: Option<&HashMap<String, (u64, u64)>>,
) -> Vec<ApiCommitFileDiffDto> {
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
            Some(ApiCommitFileDiffDto {
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
