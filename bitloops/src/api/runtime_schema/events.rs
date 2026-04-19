use async_graphql::{ID, SimpleObject};

use super::util::to_graphql_i64;
use crate::daemon::RuntimeEventRecord;

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeEventObject {
    pub domain: String,
    #[graphql(name = "repoId")]
    pub repo_id: String,
    #[graphql(name = "initSessionId")]
    pub init_session_id: Option<ID>,
    #[graphql(name = "updatedAtUnix")]
    pub updated_at_unix: i64,
    #[graphql(name = "taskId")]
    pub task_id: Option<String>,
    #[graphql(name = "runId")]
    pub run_id: Option<String>,
    #[graphql(name = "mailboxName")]
    pub mailbox_name: Option<String>,
}

impl From<RuntimeEventRecord> for RuntimeEventObject {
    fn from(value: RuntimeEventRecord) -> Self {
        Self {
            domain: value.domain,
            repo_id: value.repo_id,
            init_session_id: value.init_session_id.map(ID::from),
            updated_at_unix: to_graphql_i64(value.updated_at_unix),
            task_id: value.task_id,
            run_id: value.run_id,
            mailbox_name: value.mailbox_name,
        }
    }
}
