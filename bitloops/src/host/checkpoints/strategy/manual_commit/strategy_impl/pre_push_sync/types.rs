#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PrePushRefUpdate {
    pub(super) local_ref: String,
    pub(super) local_sha: String,
    pub(super) remote_ref: String,
    pub(super) remote_sha: String,
    pub(super) local_branch: Option<String>,
    pub(super) remote_branch: String,
}
