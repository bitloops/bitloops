use std::collections::HashMap;

use super::super::dto::ApiCommitFileDiffDto;

pub(super) fn api_file_diff_list_from_numstat(
    stats: HashMap<String, (u64, u64)>,
) -> Vec<ApiCommitFileDiffDto> {
    let mut files_touched: Vec<ApiCommitFileDiffDto> = stats
        .into_iter()
        .map(|(filepath, (adds, dels))| ApiCommitFileDiffDto {
            filepath,
            additions_count: adds,
            deletions_count: dels,
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
        })
        .collect();
    files_touched.sort_by(|left, right| left.filepath.cmp(&right.filepath));
    files_touched
}
