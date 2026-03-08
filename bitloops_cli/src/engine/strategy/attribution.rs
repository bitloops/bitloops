//! Attribution helpers for manual-commit condensation.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct TreeSnapshot {
    files: BTreeMap<String, String>,
}

impl TreeSnapshot {
    pub fn from_files(
        files: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        let mut map = BTreeMap::new();
        for (path, content) in files {
            map.insert(path.into(), content.into());
        }
        Self { files: map }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptAttribution {
    pub checkpoint_number: i32,
    pub user_lines_added: i32,
    pub user_lines_removed: i32,
    pub agent_lines_added: i32,
    pub agent_lines_removed: i32,
    pub user_added_per_file: BTreeMap<String, i32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitialAttribution {
    pub agent_lines: i32,
    pub human_added: i32,
    pub human_modified: i32,
    pub human_removed: i32,
    pub total_committed: i32,
    pub agent_percentage: f64,
}

pub fn get_all_changed_files_between_trees(
    tree1: Option<&TreeSnapshot>,
    tree2: Option<&TreeSnapshot>,
) -> Vec<String> {
    if tree1.is_none() && tree2.is_none() {
        return vec![];
    }

    let mut changed = BTreeSet::new();
    let left = tree1.map(|t| &t.files);
    let right = tree2.map(|t| &t.files);

    if let Some(left_map) = left {
        for (path, content_left) in left_map {
            let different = match right.and_then(|m| m.get(path)) {
                Some(content_right) => content_right != content_left,
                None => true,
            };
            if different {
                changed.insert(path.clone());
            }
        }
    }

    if let Some(right_map) = right {
        for path in right_map.keys() {
            if left.is_none_or(|m| !m.contains_key(path)) {
                changed.insert(path.clone());
            }
        }
    }

    changed.into_iter().collect()
}

fn get_file_content(tree: Option<&TreeSnapshot>, path: &str) -> String {
    tree.and_then(|t| t.files.get(path).cloned())
        .unwrap_or_default()
}

fn split_lines(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return vec![];
    }
    let mut parts: Vec<&str> = content.split('\n').collect();
    if content.ends_with('\n') {
        let _ = parts.pop();
    }
    parts
}

pub fn count_lines_str(content: &str) -> i32 {
    split_lines(content).len() as i32
}

fn lcs_line_count(left: &[&str], right: &[&str]) -> usize {
    let m = left.len();
    let n = right.len();
    if m == 0 || n == 0 {
        return 0;
    }

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if left[i - 1] == right[j - 1] {
                dp[i - 1][j - 1] + 1
            } else {
                dp[i - 1][j].max(dp[i][j - 1])
            };
        }
    }
    dp[m][n]
}

pub fn diff_lines(checkpoint_content: &str, committed_content: &str) -> (i32, i32, i32) {
    if checkpoint_content == committed_content {
        return (count_lines_str(committed_content), 0, 0);
    }
    if checkpoint_content.is_empty() {
        return (0, count_lines_str(committed_content), 0);
    }
    if committed_content.is_empty() {
        return (0, 0, count_lines_str(checkpoint_content));
    }

    let a = split_lines(checkpoint_content);
    let b = split_lines(committed_content);
    let unchanged = lcs_line_count(&a, &b) as i32;
    let removed = (a.len() as i32 - unchanged).max(0);
    let added = (b.len() as i32 - unchanged).max(0);
    (unchanged, added, removed)
}

pub fn estimate_user_self_modifications(
    accumulated_user_added_per_file: &BTreeMap<String, i32>,
    post_checkpoint_user_removed_per_file: &BTreeMap<String, i32>,
) -> i32 {
    let mut self_modified = 0;
    for (file_path, removed) in post_checkpoint_user_removed_per_file {
        let user_added = accumulated_user_added_per_file
            .get(file_path)
            .copied()
            .unwrap_or(0);
        self_modified += (*removed).min(user_added);
    }
    self_modified
}

pub fn calculate_attribution_with_accumulated(
    base_tree: Option<&TreeSnapshot>,
    shadow_tree: Option<&TreeSnapshot>,
    head_tree: Option<&TreeSnapshot>,
    files_touched: &[String],
    prompt_attributions: &[PromptAttribution],
) -> Option<InitialAttribution> {
    if files_touched.is_empty() {
        return None;
    }

    let mut accumulated_user_removed = 0;
    let mut accumulated_user_added_per_file: BTreeMap<String, i32> = BTreeMap::new();
    for pa in prompt_attributions {
        accumulated_user_removed += pa.user_lines_removed;
        for (file, added) in &pa.user_added_per_file {
            *accumulated_user_added_per_file
                .entry(file.clone())
                .or_default() += *added;
        }
    }

    let mut total_agent_and_user_work = 0;
    let mut post_checkpoint_user_added = 0;
    let mut post_checkpoint_user_removed = 0;
    let mut post_checkpoint_user_removed_per_file: BTreeMap<String, i32> = BTreeMap::new();

    for file_path in files_touched {
        let base_content = get_file_content(base_tree, file_path);
        let shadow_content = get_file_content(shadow_tree, file_path);
        let head_content = get_file_content(head_tree, file_path);

        let (_, work_added, _) = diff_lines(&base_content, &shadow_content);
        total_agent_and_user_work += work_added;

        let (_, post_user_added, post_user_removed_file) =
            diff_lines(&shadow_content, &head_content);
        post_checkpoint_user_added += post_user_added;
        post_checkpoint_user_removed += post_user_removed_file;
        if post_user_removed_file > 0 {
            post_checkpoint_user_removed_per_file.insert(file_path.clone(), post_user_removed_file);
        }
    }

    let non_agent_files = get_all_changed_files_between_trees(base_tree, head_tree);
    let mut all_user_edits_to_non_agent_files = 0;
    for file_path in &non_agent_files {
        if files_touched.contains(file_path) {
            continue;
        }
        let base_content = get_file_content(base_tree, file_path);
        let head_content = get_file_content(head_tree, file_path);
        let (_, user_added, _) = diff_lines(&base_content, &head_content);
        all_user_edits_to_non_agent_files += user_added;
    }

    let committed_non_agent_set: BTreeSet<String> = non_agent_files
        .into_iter()
        .filter(|f| !files_touched.contains(f))
        .collect();

    let mut accumulated_to_agent_files = 0;
    let mut accumulated_to_committed_non_agent_files = 0;
    for (file_path, added) in &accumulated_user_added_per_file {
        if files_touched.contains(file_path) {
            accumulated_to_agent_files += *added;
        } else if committed_non_agent_set.contains(file_path) {
            accumulated_to_committed_non_agent_files += *added;
        }
    }

    let total_agent_added = (total_agent_and_user_work - accumulated_to_agent_files).max(0);
    let post_to_non_agent_files =
        (all_user_edits_to_non_agent_files - accumulated_to_committed_non_agent_files).max(0);

    let relevant_accumulated_user =
        accumulated_to_agent_files + accumulated_to_committed_non_agent_files;
    let total_user_added =
        relevant_accumulated_user + post_checkpoint_user_added + post_to_non_agent_files;
    let total_user_removed = accumulated_user_removed + post_checkpoint_user_removed;

    let total_human_modified = total_user_added.min(total_user_removed);
    let user_self_modified = estimate_user_self_modifications(
        &accumulated_user_added_per_file,
        &post_checkpoint_user_removed_per_file,
    );
    let human_modified_agent = (total_human_modified - user_self_modified).max(0);

    let pure_user_added = total_user_added - total_human_modified;
    let pure_user_removed = total_user_removed - total_human_modified;

    let mut total_committed = total_agent_added + pure_user_added - pure_user_removed;
    if total_committed <= 0 {
        total_committed = total_agent_added.max(0);
    }

    let agent_lines_in_commit =
        (total_agent_added - pure_user_removed - human_modified_agent).max(0);
    let agent_percentage = if total_committed > 0 {
        (agent_lines_in_commit as f64 / total_committed as f64) * 100.0
    } else {
        0.0
    };

    Some(InitialAttribution {
        agent_lines: agent_lines_in_commit,
        human_added: pure_user_added,
        human_modified: total_human_modified,
        human_removed: pure_user_removed,
        total_committed,
        agent_percentage,
    })
}

pub fn calculate_prompt_attribution(
    base_tree: Option<&TreeSnapshot>,
    last_checkpoint_tree: Option<&TreeSnapshot>,
    worktree_files: &BTreeMap<String, String>,
    checkpoint_number: i32,
) -> PromptAttribution {
    let mut result = PromptAttribution {
        checkpoint_number,
        user_added_per_file: BTreeMap::new(),
        ..Default::default()
    };

    if worktree_files.is_empty() {
        return result;
    }

    let reference_tree = last_checkpoint_tree.or(base_tree);

    for (file_path, worktree_content) in worktree_files {
        let reference_content = get_file_content(reference_tree, file_path);
        let base_content = get_file_content(base_tree, file_path);

        let (_, user_added, user_removed) = diff_lines(&reference_content, worktree_content);
        result.user_lines_added += user_added;
        result.user_lines_removed += user_removed;
        if user_added > 0 {
            result
                .user_added_per_file
                .insert(file_path.clone(), user_added);
        }

        if let Some(last_checkpoint) = last_checkpoint_tree {
            let checkpoint_content = get_file_content(Some(last_checkpoint), file_path);
            let (_, agent_added, agent_removed) = diff_lines(&base_content, &checkpoint_content);
            result.agent_lines_added += agent_added;
            result.agent_lines_removed += agent_removed;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_THREE_LINES: &str = "line1\nline2\nline3\n";

    fn build_test_tree(files: &[(&str, &str)]) -> TreeSnapshot {
        TreeSnapshot::from_files(
            files
                .iter()
                .map(|(p, c)| ((*p).to_string(), (*c).to_string())),
        )
    }

    #[test]
    fn diff_lines_no_changes() {
        let (unchanged, added, removed) = diff_lines(TEST_THREE_LINES, TEST_THREE_LINES);
        assert_eq!(unchanged, 3);
        assert_eq!(added, 0);
        assert_eq!(removed, 0);
    }

    #[test]
    fn diff_lines_all_added() {
        let (unchanged, added, removed) = diff_lines("", TEST_THREE_LINES);
        assert_eq!(unchanged, 0);
        assert_eq!(added, 3);
        assert_eq!(removed, 0);
    }

    #[test]
    fn diff_lines_all_removed() {
        let (unchanged, added, removed) = diff_lines(TEST_THREE_LINES, "");
        assert_eq!(unchanged, 0);
        assert_eq!(added, 0);
        assert_eq!(removed, 3);
    }

    #[test]
    fn diff_lines_mixed_changes() {
        let checkpoint = TEST_THREE_LINES;
        let committed = "line1\nmodified\nline3\nnew line\n";
        let (unchanged, added, removed) = diff_lines(checkpoint, committed);
        assert_eq!(unchanged, 2);
        assert_eq!(added, 2);
        assert_eq!(removed, 1);
    }

    #[test]
    fn diff_lines_without_trailing_newline() {
        let checkpoint = "line1\nline2";
        let committed = "line1\nline2";
        let (unchanged, added, removed) = diff_lines(checkpoint, committed);
        assert_eq!(unchanged, 2);
        assert_eq!(added, 0);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_count_lines_str() {
        let cases = [
            ("", 0),
            ("hello", 1),
            ("hello\n", 1),
            ("hello\nworld\n", 2),
            ("hello\nworld", 2),
            ("a\nb\nc\n", 3),
        ];
        for (content, expected) in cases {
            assert_eq!(count_lines_str(content), expected);
        }
    }

    #[test]
    fn diff_lines_percentage_calculation() {
        let checkpoint = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\n";
        let committed = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nnew1\nnew2\n";
        let (unchanged, added, removed) = diff_lines(checkpoint, committed);
        assert_eq!(unchanged, 8);
        assert_eq!(added, 2);
        assert_eq!(removed, 0);
        assert_eq!(count_lines_str(committed), 10);
    }

    #[test]
    fn diff_lines_modified_estimation() {
        let checkpoint = "original1\noriginal2\noriginal3\n";
        let committed = "modified1\nmodified2\noriginal3\nnew line\n";
        let (unchanged, added, removed) = diff_lines(checkpoint, committed);
        assert_eq!(unchanged, 1);
        assert_eq!(added, 3);
        assert_eq!(removed, 2);

        let human_modified = added.min(removed);
        let human_added = added - human_modified;
        let human_removed = removed - human_modified;
        assert_eq!(human_modified, 2);
        assert_eq!(human_added, 1);
        assert_eq!(human_removed, 0);
    }

    #[test]
    fn calculate_attribution_with_accumulated_basic_case() {
        let base = build_test_tree(&[("main.rs", "")]);
        let shadow = build_test_tree(&[(
            "main.rs",
            "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\n",
        )]);
        let head = build_test_tree(&[(
            "main.rs",
            "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nuser1\nuser2\n",
        )]);

        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["main.rs".to_string()],
            &[],
        )
        .unwrap();

        assert_eq!(result.agent_lines, 8);
        assert_eq!(result.human_added, 2);
        assert_eq!(result.human_modified, 0);
        assert_eq!(result.human_removed, 0);
        assert_eq!(result.total_committed, 10);
        assert!((result.agent_percentage - 80.0).abs() < 0.2);
    }

    #[test]
    fn calculate_attribution_with_accumulated_bug_scenario() {
        let base = build_test_tree(&[("main.rs", "")]);
        let shadow = build_test_tree(&[(
            "main.rs",
            "agent1\nagent2\nagent3\nagent4\nagent5\nagent6\nagent7\nagent8\nagent9\nagent10\n",
        )]);
        let head = build_test_tree(&[(
            "main.rs",
            "agent1\nagent2\nagent3\nagent4\nagent5\nuser1\nuser2\n",
        )]);

        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["main.rs".to_string()],
            &[],
        )
        .unwrap();

        assert_eq!(result.agent_lines, 5);
        assert_eq!(result.human_added, 0);
        assert_eq!(result.human_modified, 2);
        assert_eq!(result.human_removed, 3);
        assert_eq!(result.total_committed, 7);
        assert!((result.agent_percentage - 71.4).abs() < 1.0);
    }

    #[test]
    fn calculate_attribution_with_accumulated_deletion_only() {
        let base = build_test_tree(&[("main.rs", "line1\nline2\nline3\nline4\nline5\n")]);
        let shadow = build_test_tree(&[("main.rs", "line1\nline2\nline3\n")]);
        let head = build_test_tree(&[("main.rs", "line1\n")]);

        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["main.rs".to_string()],
            &[],
        )
        .unwrap();

        assert_eq!(result.agent_lines, 0);
        assert_eq!(result.human_added, 0);
        assert_eq!(result.human_removed, 2);
        assert_eq!(result.total_committed, 0);
        assert_eq!(result.agent_percentage, 0.0);
    }

    #[test]
    fn calculate_attribution_with_accumulated_no_user_edits() {
        let content = "agent1\nagent2\nagent3\nagent4\nagent5\n";
        let base = build_test_tree(&[("main.rs", "")]);
        let shadow = build_test_tree(&[("main.rs", content)]);
        let head = build_test_tree(&[("main.rs", content)]);
        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["main.rs".to_string()],
            &[],
        )
        .unwrap();
        assert_eq!(result.agent_lines, 5);
        assert_eq!(result.human_added, 0);
        assert_eq!(result.human_modified, 0);
        assert_eq!(result.human_removed, 0);
        assert_eq!(result.total_committed, 5);
        assert_eq!(result.agent_percentage, 100.0);
    }

    #[test]
    fn calculate_attribution_with_accumulated_no_agent_work() {
        let content = "line1\nline2\nline3\n";
        let base = build_test_tree(&[("main.rs", content)]);
        let shadow = build_test_tree(&[("main.rs", content)]);
        let head = build_test_tree(&[("main.rs", "line1\nline2\nline3\nuser1\nuser2\n")]);
        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["main.rs".to_string()],
            &[],
        )
        .unwrap();
        assert_eq!(result.agent_lines, 0);
        assert_eq!(result.human_added, 2);
        assert_eq!(result.total_committed, 2);
        assert_eq!(result.agent_percentage, 0.0);
    }

    #[test]
    fn calculate_attribution_with_accumulated_user_removes_all_agent_lines() {
        let base = build_test_tree(&[("main.rs", "")]);
        let shadow = build_test_tree(&[("main.rs", "agent1\nagent2\nagent3\nagent4\nagent5\n")]);
        let head = build_test_tree(&[("main.rs", "user1\nuser2\nuser3\n")]);
        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["main.rs".to_string()],
            &[],
        )
        .unwrap();

        assert_eq!(result.agent_lines, 0);
        assert_eq!(result.human_added, 0);
        assert_eq!(result.human_modified, 3);
        assert_eq!(result.human_removed, 2);
        assert_eq!(result.total_committed, 3);
        assert_eq!(result.agent_percentage, 0.0);
    }

    #[test]
    fn calculate_attribution_with_accumulated_with_prompt_attributions() {
        let base = build_test_tree(&[("main.rs", "")]);
        let shadow = build_test_tree(&[(
            "main.rs",
            "agent1\nagent2\nuser_between1\nuser_between2\nagent3\nagent4\nagent5\nagent6\nagent7\nagent8\nagent9\nagent10\n",
        )]);
        let head = build_test_tree(&[(
            "main.rs",
            "agent1\nagent2\nuser_between1\nuser_between2\nagent3\nagent4\nagent5\nagent6\nagent7\nagent8\nagent9\nagent10\nuser_after\n",
        )]);
        let prompt = PromptAttribution {
            checkpoint_number: 2,
            user_lines_added: 2,
            user_lines_removed: 0,
            agent_lines_added: 0,
            agent_lines_removed: 0,
            user_added_per_file: BTreeMap::from([("main.rs".to_string(), 2)]),
        };
        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["main.rs".to_string()],
            &[prompt],
        )
        .unwrap();
        assert_eq!(result.agent_lines, 10);
        assert_eq!(result.human_added, 3);
        assert_eq!(result.total_committed, 13);
        assert!((result.agent_percentage - 76.9).abs() < 0.3);
    }

    #[test]
    fn calculate_attribution_with_accumulated_empty_files_touched() {
        let base = build_test_tree(&[]);
        let shadow = build_test_tree(&[]);
        let head = build_test_tree(&[]);
        assert!(
            calculate_attribution_with_accumulated(
                Some(&base),
                Some(&shadow),
                Some(&head),
                &[],
                &[]
            )
            .is_none()
        );
    }

    #[test]
    fn calculate_attribution_with_accumulated_user_edits_non_agent_file() {
        let base = build_test_tree(&[
            ("file1.rs", "package main\n"),
            ("file2.rs", "package util\n"),
        ]);
        let shadow = build_test_tree(&[(
            "file1.rs",
            "package main\n\nfunc agent1() {}\nfunc agent2() {}\n",
        )]);
        let head = build_test_tree(&[
            (
                "file1.rs",
                "package main\n\nfunc agent1() {}\nfunc agent2() {}\n",
            ),
            (
                "file2.rs",
                "package util\n\n// User edit 1\n// User edit 2\n// User edit 3\n",
            ),
        ]);
        let prompt = PromptAttribution {
            checkpoint_number: 1,
            user_lines_added: 2,
            user_lines_removed: 0,
            agent_lines_added: 0,
            agent_lines_removed: 0,
            user_added_per_file: BTreeMap::from([("file2.rs".to_string(), 2)]),
        };
        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["file1.rs".to_string()],
            &[prompt],
        )
        .unwrap();
        assert_eq!(result.agent_lines, 3);
        assert_eq!(result.human_added, 4);
        assert_eq!(result.total_committed, 7);
        assert!((result.agent_percentage - 42.9).abs() < 0.2);
    }

    #[test]
    fn test_get_all_changed_files_between_trees() {
        assert!(get_all_changed_files_between_trees(None, None).is_empty());

        let tree2 = build_test_tree(&[("file1.rs", "content1"), ("file2.rs", "content2")]);
        let mut changed = get_all_changed_files_between_trees(None, Some(&tree2));
        changed.sort();
        assert_eq!(
            changed,
            vec!["file1.rs".to_string(), "file2.rs".to_string()]
        );

        let tree1 = build_test_tree(&[("file1.rs", "content1")]);
        let changed = get_all_changed_files_between_trees(Some(&tree1), None);
        assert_eq!(changed, vec!["file1.rs".to_string()]);

        let tree_a = build_test_tree(&[("file1.rs", "same"), ("file2.rs", "same")]);
        let tree_b = build_test_tree(&[("file1.rs", "same"), ("file2.rs", "same")]);
        assert!(get_all_changed_files_between_trees(Some(&tree_a), Some(&tree_b)).is_empty());

        let tree_mod_a = build_test_tree(&[("file1.rs", "original"), ("unchanged.rs", "same")]);
        let tree_mod_b = build_test_tree(&[("file1.rs", "modified"), ("unchanged.rs", "same")]);
        assert_eq!(
            get_all_changed_files_between_trees(Some(&tree_mod_a), Some(&tree_mod_b)),
            vec!["file1.rs".to_string()]
        );
    }

    #[test]
    fn estimate_user_self_modifications_compat() {
        let cases = vec![
            (
                BTreeMap::from([("file.rs".to_string(), 5)]),
                BTreeMap::<String, i32>::new(),
                0,
            ),
            (
                BTreeMap::from([("file.rs".to_string(), 5)]),
                BTreeMap::from([("file.rs".to_string(), 3)]),
                3,
            ),
            (
                BTreeMap::from([("file.rs".to_string(), 5)]),
                BTreeMap::from([("file.rs".to_string(), 5)]),
                5,
            ),
            (
                BTreeMap::from([("file.rs".to_string(), 3)]),
                BTreeMap::from([("file.rs".to_string(), 5)]),
                3,
            ),
            (
                BTreeMap::<String, i32>::new(),
                BTreeMap::from([("file.rs".to_string(), 5)]),
                0,
            ),
            (
                BTreeMap::from([("a.rs".to_string(), 3), ("b.rs".to_string(), 2)]),
                BTreeMap::from([("a.rs".to_string(), 2), ("b.rs".to_string(), 4)]),
                4,
            ),
        ];
        for (added, removed, want) in cases {
            assert_eq!(estimate_user_self_modifications(&added, &removed), want);
        }
    }

    #[test]
    fn calculate_attribution_with_accumulated_user_self_modification() {
        let base = build_test_tree(&[("main.rs", "")]);
        let shadow = build_test_tree(&[(
            "main.rs",
            "agent1\nagent2\nagent3\nagent4\nagent5\nagent6\nagent7\nagent8\nagent9\nagent10\nuser1\nuser2\nuser3\nuser4\nuser5\n",
        )]);
        let head = build_test_tree(&[(
            "main.rs",
            "agent1\nagent2\nagent3\nagent4\nagent5\nagent6\nagent7\nagent8\nagent9\nagent10\nuser1\nuser2\nnew_user1\nnew_user2\nnew_user3\n",
        )]);
        let prompt = PromptAttribution {
            checkpoint_number: 2,
            user_lines_added: 5,
            user_lines_removed: 0,
            agent_lines_added: 0,
            agent_lines_removed: 0,
            user_added_per_file: BTreeMap::from([("main.rs".to_string(), 5)]),
        };
        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["main.rs".to_string()],
            &[prompt],
        )
        .unwrap();
        assert_eq!(result.agent_lines, 10);
        assert_eq!(result.human_added, 5);
        assert_eq!(result.human_modified, 3);
        assert_eq!(result.total_committed, 15);
        assert!((result.agent_percentage - 66.7).abs() < 0.2);
    }

    #[test]
    fn calculate_attribution_with_accumulated_mixed_modifications() {
        let base = build_test_tree(&[("main.rs", "")]);
        let shadow = build_test_tree(&[(
            "main.rs",
            "agent1\nagent2\nagent3\nagent4\nagent5\nagent6\nagent7\nagent8\nagent9\nagent10\nuser1\nuser2\nuser3\n",
        )]);
        let head = build_test_tree(&[(
            "main.rs",
            "agent1\nagent2\nagent3\nagent4\nagent5\nagent6\nagent7\nagent8\nnew1\nnew2\nnew3\nnew4\nnew5\n",
        )]);
        let prompt = PromptAttribution {
            checkpoint_number: 2,
            user_lines_added: 3,
            user_lines_removed: 0,
            agent_lines_added: 0,
            agent_lines_removed: 0,
            user_added_per_file: BTreeMap::from([("main.rs".to_string(), 3)]),
        };
        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["main.rs".to_string()],
            &[prompt],
        )
        .unwrap();
        assert_eq!(result.agent_lines, 8);
        assert_eq!(result.human_modified, 5);
        assert_eq!(result.total_committed, 13);
        assert!((result.agent_percentage - 61.5).abs() < 0.2);
    }

    #[test]
    fn calculate_attribution_with_accumulated_uncommitted_worktree_files() {
        let base = build_test_tree(&[]);
        let agent_content = "# Software Testing\n\nSoftware testing is a critical part of the development process.\n\n## Types of Testing\n\n- Unit testing\n- Integration testing\n- End-to-end testing\n\n## Best Practices\n\nWrite tests early.\nAutomate where possible.\nTest edge cases.\nReview test coverage.\n";
        let shadow = build_test_tree(&[("example.md", agent_content)]);
        let head = build_test_tree(&[("example.md", agent_content)]);
        let prompt = PromptAttribution {
            checkpoint_number: 1,
            user_lines_added: 84,
            user_lines_removed: 0,
            agent_lines_added: 0,
            agent_lines_removed: 0,
            user_added_per_file: BTreeMap::from([(".claude/settings.json".to_string(), 84)]),
        };
        let result = calculate_attribution_with_accumulated(
            Some(&base),
            Some(&shadow),
            Some(&head),
            &["example.md".to_string()],
            &[prompt],
        )
        .unwrap();
        let agent_lines = count_lines_str(agent_content);
        assert_eq!(result.agent_lines, agent_lines);
        assert_eq!(result.human_added, 0);
        assert_eq!(result.total_committed, agent_lines);
        assert_eq!(result.agent_percentage, 100.0);
    }

    #[test]
    fn calculate_prompt_attribution_populates_per_file() {
        let base = build_test_tree(&[("a.rs", "line1\n"), ("b.rs", "line1\n")]);
        let checkpoint = build_test_tree(&[
            ("a.rs", "line1\nagent1\n"),
            ("b.rs", "line1\nagent1\nagent2\n"),
        ]);
        let worktree = BTreeMap::from([
            (
                "a.rs".to_string(),
                "line1\nagent1\nuser1\nuser2\nuser3\n".to_string(),
            ),
            (
                "b.rs".to_string(),
                "line1\nagent1\nagent2\nuser1\n".to_string(),
            ),
        ]);
        let result = calculate_prompt_attribution(Some(&base), Some(&checkpoint), &worktree, 2);
        assert_eq!(result.user_lines_added, 4);
        assert_eq!(result.user_added_per_file.get("a.rs").copied(), Some(3));
        assert_eq!(result.user_added_per_file.get("b.rs").copied(), Some(1));
    }

    #[test]
    fn prompt_attribution_uses_worktree_not_staging_area() {
        let base = build_test_tree(&[("test.rs", "package main\n")]);
        let checkpoint1 = build_test_tree(&[(
            "test.rs",
            "package main\n\nfunc agentFunc() {\n\tprintln(\"agent\")\n}\n",
        )]);
        let worktree = BTreeMap::from([(
            "test.rs".to_string(),
            "package main\n\nfunc agentFunc() {\n\tprintln(\"agent\")\n}\n// User added line 1\n// User added line 2\n// User added line 3\n// User added line 4\n// User added line 5\n// User added line 6\n// User added line 7\n// User added line 8\n// User added line 9\n// User added line 10\n"
                .to_string(),
        )]);
        let result = calculate_prompt_attribution(Some(&base), Some(&checkpoint1), &worktree, 2);
        assert_eq!(result.user_lines_added, 10);
        assert_eq!(result.checkpoint_number, 2);
    }

    #[test]
    fn prompt_attribution_unstaged_changes() {
        let base = build_test_tree(&[("test.rs", "package main\n")]);
        let checkpoint1 = build_test_tree(&[(
            "test.rs",
            "package main\n\nfunc agentFunc() {\n\tprintln(\"agent\")\n}\n",
        )]);
        let worktree = BTreeMap::from([(
            "test.rs".to_string(),
            "package main\n\nfunc agentFunc() {\n\tprintln(\"agent\")\n}\n// User added line 1\n// User added line 2\n// User added line 3\n"
                .to_string(),
        )]);
        let result = calculate_prompt_attribution(Some(&base), Some(&checkpoint1), &worktree, 2);
        assert_eq!(result.user_lines_added, 3);
    }

    #[test]
    fn prompt_attribution_always_stored() {
        let base = build_test_tree(&[("test.rs", "package main\n")]);
        let checkpoint1 = build_test_tree(&[(
            "test.rs",
            "package main\n\nfunc agentFunc() {\n\tprintln(\"agent\")\n}\n",
        )]);
        let worktree = BTreeMap::from([(
            "test.rs".to_string(),
            "package main\n\nfunc agentFunc() {\n\tprintln(\"agent\")\n}\n".to_string(),
        )]);
        let result = calculate_prompt_attribution(Some(&base), Some(&checkpoint1), &worktree, 2);
        assert_eq!(result.user_lines_added, 0);
        assert_eq!(result.user_lines_removed, 0);
        assert_eq!(result.checkpoint_number, 2);
    }

    #[test]
    fn prompt_attribution_captures_pre_prompt_edits() {
        let base = build_test_tree(&[("test.rs", "package main\n")]);
        let worktree = BTreeMap::from([(
            "test.rs".to_string(),
            "package main\n// User added line 1\n// User added line 2\n".to_string(),
        )]);
        let result = calculate_prompt_attribution(Some(&base), None, &worktree, 1);
        assert_eq!(result.user_lines_added, 2);
        assert_eq!(result.checkpoint_number, 1);
        assert_eq!(result.agent_lines_added, 0);
    }
}
