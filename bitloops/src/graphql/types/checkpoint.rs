use async_graphql::{ComplexObject, Context, ID, Result, SimpleObject};

use crate::graphql::{DevqlGraphqlContext, backend_error};
use crate::host::checkpoints::strategy::manual_commit::CommittedInfo;

use super::{Commit, DateTimeScalar, JsonScalar};

const UNIX_EPOCH_RFC3339: &str = "1970-01-01T00:00:00+00:00";

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct Checkpoint {
    pub id: ID,
    pub session_id: String,
    pub commit_sha: Option<String>,
    pub branch: Option<String>,
    pub agent: Option<String>,
    pub event_time: DateTimeScalar,
    pub strategy: Option<String>,
    pub files_touched: Vec<String>,
    pub payload: Option<JsonScalar>,
}

impl Checkpoint {
    pub fn from_committed(commit_sha: &str, info: &CommittedInfo) -> Self {
        let event_time = DateTimeScalar::from_rfc3339(info.created_at.clone())
            .or_else(|_| DateTimeScalar::from_rfc3339(UNIX_EPOCH_RFC3339))
            .expect("static epoch timestamp must parse");
        Self {
            id: ID(info.checkpoint_id.clone()),
            session_id: info.session_id.clone(),
            commit_sha: Some(commit_sha.to_string()),
            branch: non_empty(info.branch.as_str()),
            agent: non_empty(info.agent.as_str()),
            event_time,
            strategy: non_empty(info.strategy.as_str()),
            files_touched: info.files_touched.clone(),
            payload: None,
        }
    }

    pub fn cursor(&self) -> String {
        self.id.to_string()
    }
}

#[ComplexObject]
impl Checkpoint {
    async fn commit(&self, ctx: &Context<'_>) -> Result<Option<Commit>> {
        let Some(commit_sha) = self.commit_sha.as_deref() else {
            return Ok(None);
        };

        ctx.data_unchecked::<DevqlGraphqlContext>()
            .resolve_commit_by_sha(commit_sha, self.branch.as_deref())
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve commit {} for checkpoint {:?}: {err:#}",
                    commit_sha, self.id
                ))
            })
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
