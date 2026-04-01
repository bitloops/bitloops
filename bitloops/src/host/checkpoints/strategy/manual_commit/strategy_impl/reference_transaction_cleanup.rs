use super::*;

#[derive(Debug, Clone, Default)]
pub(crate) struct BranchDeletionTargets {
    pub(super) local_branches: Vec<String>,
    pub(super) remote_branches: Vec<String>,
}

const ZERO_GIT_OID: &str = "0000000000000000000000000000000000000000";

pub(crate) fn collect_reference_transaction_branch_deletions(
    state: &str,
    stdin_lines: &[String],
) -> BranchDeletionTargets {
    if !state.eq_ignore_ascii_case("committed") {
        return BranchDeletionTargets::default();
    }

    let mut local = std::collections::BTreeSet::new();
    let mut remote = std::collections::BTreeSet::new();

    for line in stdin_lines {
        let Some((_, new_sha, ref_name)) = parse_reference_transaction_update_line(line) else {
            continue;
        };
        if !is_zero_git_oid(new_sha) {
            continue;
        }

        if let Some(branch_name) = ref_name.strip_prefix("refs/heads/")
            && !branch_name.trim().is_empty()
        {
            local.insert(branch_name.to_string());
            continue;
        }

        if let Some(branch_name) = ref_name.strip_prefix("refs/remotes/")
            && !branch_name.trim().is_empty()
        {
            remote.insert(branch_name.to_string());
        }
    }

    BranchDeletionTargets {
        local_branches: local.into_iter().collect(),
        remote_branches: remote.into_iter().collect(),
    }
}

fn parse_reference_transaction_update_line(line: &str) -> Option<(&str, &str, &str)> {
    let mut parts = line.split_whitespace();
    let old_sha = parts.next()?;
    let new_sha = parts.next()?;
    let ref_name = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((old_sha, new_sha, ref_name))
}

fn is_zero_git_oid(value: &str) -> bool {
    value.trim() == ZERO_GIT_OID
}

pub(crate) fn run_devql_reference_transaction_cleanup(
    _repo_root: &Path,
    _deletions: &BranchDeletionTargets,
) -> Result<()> {
    Ok(())
}
