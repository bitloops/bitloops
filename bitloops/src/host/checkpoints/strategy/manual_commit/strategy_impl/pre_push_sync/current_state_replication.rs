use super::*;

pub(super) async fn sync_remote_branch_current_state(
    _relational: &crate::host::devql::RelationalStorage,
    _repo_id: &str,
    _source_branch: &str,
    _remote_branch: &str,
) -> Result<()> {
    Ok(())
}
