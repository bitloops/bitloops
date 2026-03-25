pub(super) const ZERO_GIT_OID: &str = "0000000000000000000000000000000000000000";

pub(super) const PRE_PUSH_RETENTION_COMMITS: usize = 50;
pub(super) const PRE_PUSH_SYNC_WATERMARK_KEY: &str = "last_synced_commit_sha";
pub(super) const PRE_PUSH_SYNC_PENDING_KEY_PREFIX: &str = "pending_remote_sync_sha";
pub(super) const PRE_PUSH_BATCH_SIZE: usize = 200;
