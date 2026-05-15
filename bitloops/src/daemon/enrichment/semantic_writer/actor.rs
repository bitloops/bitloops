use anyhow::{Context, Result, anyhow};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use tokio::sync::oneshot;

use super::commit::{execute_embedding_commit, execute_summary_commit};
use super::runtime_store::open_semantic_writer_connection;
use super::{
    CommitEmbeddingBatchRequest, CommitSummaryBatchRequest, SummaryCommitFailure,
    SummaryCommitPhase, SummaryCommitPhaseTimings, SummaryCommitReport,
};

#[derive(Debug)]
enum SemanticWriterRequest {
    Summary {
        request: CommitSummaryBatchRequest,
        response: oneshot::Sender<std::result::Result<SummaryCommitReport, SummaryCommitFailure>>,
    },
    Embedding {
        request: CommitEmbeddingBatchRequest,
        response: oneshot::Sender<std::result::Result<(), String>>,
    },
}

#[derive(Debug)]
pub(super) struct RepoSemanticWriterActor {
    sender: Sender<SemanticWriterRequest>,
}

#[derive(Debug, Default)]
struct PendingSemanticWriterRequests {
    summaries: VecDeque<SemanticWriterRequest>,
    embeddings: VecDeque<SemanticWriterRequest>,
    prefer_summary: bool,
}

impl PendingSemanticWriterRequests {
    fn push(&mut self, request: SemanticWriterRequest) {
        match request {
            request @ SemanticWriterRequest::Summary { .. } => self.summaries.push_back(request),
            request @ SemanticWriterRequest::Embedding { .. } => self.embeddings.push_back(request),
        }
    }

    fn pop_next(&mut self) -> Option<SemanticWriterRequest> {
        match (self.summaries.is_empty(), self.embeddings.is_empty()) {
            (true, true) => None,
            (false, true) => {
                self.prefer_summary = false;
                self.summaries.pop_front()
            }
            (true, false) => {
                self.prefer_summary = true;
                self.embeddings.pop_front()
            }
            (false, false) => {
                if self.prefer_summary {
                    self.prefer_summary = false;
                    self.summaries.pop_front()
                } else {
                    self.prefer_summary = true;
                    self.embeddings.pop_front()
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.summaries.is_empty() && self.embeddings.is_empty()
    }
}

impl RepoSemanticWriterActor {
    pub(super) fn shared(
        runtime_db_path: &Path,
        relational_db_path: &Path,
        repo_id: &str,
    ) -> Result<Arc<Self>> {
        static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<RepoSemanticWriterActor>>>> =
            OnceLock::new();
        let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
        let key = format!(
            "{}::{}::{repo_id}",
            runtime_db_path.display(),
            relational_db_path.display()
        );
        let mut registry = registry
            .lock()
            .map_err(|_| anyhow!("locking semantic writer actor registry"))?;
        if let Some(actor) = registry.get(&key) {
            return Ok(Arc::clone(actor));
        }

        let actor = Arc::new(Self::spawn(
            runtime_db_path.to_path_buf(),
            relational_db_path.to_path_buf(),
            repo_id.to_string(),
        )?);
        registry.insert(key, Arc::clone(&actor));
        Ok(actor)
    }

    fn spawn(
        runtime_db_path: PathBuf,
        relational_db_path: PathBuf,
        repo_id: String,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::channel::<SemanticWriterRequest>();
        let thread_name = format!("bitloops-semantic-writer-{repo_id}");
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || writer_loop(runtime_db_path, relational_db_path, receiver))
            .context("spawning semantic writer actor thread")?;
        Ok(Self { sender })
    }

    pub(super) async fn commit_summary(
        &self,
        request: CommitSummaryBatchRequest,
    ) -> std::result::Result<SummaryCommitReport, SummaryCommitFailure> {
        let (response_tx, response_rx) = oneshot::channel();
        self.sender
            .send(SemanticWriterRequest::Summary {
                request,
                response: response_tx,
            })
            .map_err(|_| {
                SummaryCommitFailure::new(
                    SummaryCommitPhase::TransactionStart,
                    SummaryCommitPhaseTimings::default(),
                    false,
                    anyhow!("sending summary batch to semantic writer"),
                )
            })?;
        match response_rx.await {
            Ok(Ok(report)) => Ok(report),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(SummaryCommitFailure::new(
                SummaryCommitPhase::TransactionStart,
                SummaryCommitPhaseTimings::default(),
                false,
                anyhow!("semantic writer dropped the summary response channel"),
            )),
        }
    }

    pub(super) async fn commit_embedding(
        &self,
        request: CommitEmbeddingBatchRequest,
    ) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.sender
            .send(SemanticWriterRequest::Embedding {
                request,
                response: response_tx,
            })
            .map_err(|_| anyhow!("sending embedding batch to semantic writer"))?;
        match response_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(anyhow!(err)),
            Err(_) => Err(anyhow!(
                "semantic writer dropped the embedding response channel"
            )),
        }
    }
}

fn writer_loop(
    runtime_db_path: PathBuf,
    relational_db_path: PathBuf,
    receiver: Receiver<SemanticWriterRequest>,
) {
    let mut connection = open_semantic_writer_connection(&runtime_db_path, &relational_db_path)
        .map_err(|err| format!("{err:#}"))
        .ok();
    let mut pending = PendingSemanticWriterRequests::default();
    loop {
        if pending.is_empty() {
            let Ok(request) = receiver.recv() else {
                break;
            };
            pending.push(request);
        }
        while let Ok(request) = receiver.try_recv() {
            pending.push(request);
        }
        let Some(request) = pending.pop_next() else {
            continue;
        };
        match (&mut connection, request) {
            (Some(connection), SemanticWriterRequest::Summary { request, response }) => {
                let result = execute_summary_commit(connection, &request).map_err(|err| {
                    SummaryCommitFailure::new(
                        err.phase(),
                        err.timings(),
                        err.runtime_store_writes_succeeded_in_tx(),
                        anyhow!(
                            "committing semantic summary batch for repo `{}`: {:#}",
                            request.repo.repo_id,
                            err
                        ),
                    )
                });
                let _ = response.send(result);
            }
            (Some(connection), SemanticWriterRequest::Embedding { request, response }) => {
                let result = execute_embedding_commit(connection, &request).map_err(|err| {
                    format!(
                        "committing semantic embedding batch for repo `{}`: {err:#}",
                        request.repo.repo_id
                    )
                });
                let _ = response.send(result);
            }
            (None, SemanticWriterRequest::Summary { response, .. }) => {
                let _ = response.send(Err(SummaryCommitFailure::new(
                    SummaryCommitPhase::TransactionStart,
                    SummaryCommitPhaseTimings::default(),
                    false,
                    anyhow!("opening semantic writer connection failed"),
                )));
            }
            (None, SemanticWriterRequest::Embedding { response, .. }) => {
                let _ = response.send(Err("opening semantic writer connection failed".to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PendingSemanticWriterRequests, SemanticWriterRequest};
    use crate::daemon::enrichment::semantic_writer::{
        CommitEmbeddingBatchRequest, CommitSummaryBatchRequest, SemanticBatchRepoContext,
    };
    use tokio::sync::oneshot;

    fn repo() -> SemanticBatchRepoContext {
        SemanticBatchRepoContext {
            repo_id: "repo-1".to_string(),
            repo_root: std::path::PathBuf::from("/tmp/repo"),
            config_root: std::path::PathBuf::from("/tmp/config"),
        }
    }

    fn summary_request() -> SemanticWriterRequest {
        let (response, _rx) = oneshot::channel();
        SemanticWriterRequest::Summary {
            request: CommitSummaryBatchRequest {
                repo: repo(),
                lease_token: "summary-lease".to_string(),
                semantic_statements: Vec::new(),
                embedding_follow_ups: Vec::new(),
                replacement_backfill_item: None,
                acked_item_ids: Vec::new(),
            },
            response,
        }
    }

    fn embedding_request() -> SemanticWriterRequest {
        let (response, _rx) = oneshot::channel();
        SemanticWriterRequest::Embedding {
            request: CommitEmbeddingBatchRequest {
                repo: repo(),
                lease_token: "embedding-lease".to_string(),
                embedding_statements: Vec::new(),
                setup_statements: Vec::new(),
                remote_embedding_statements: Vec::new(),
                remote_setup_statements: Vec::new(),
                clone_rebuild_signal: None,
                replacement_backfill_item: None,
                acked_item_ids: Vec::new(),
            },
            response,
        }
    }

    #[test]
    fn pending_requests_alternate_between_embeddings_and_summaries() {
        let mut pending = PendingSemanticWriterRequests::default();
        pending.push(summary_request());
        pending.push(embedding_request());
        pending.push(summary_request());
        pending.push(embedding_request());

        assert!(matches!(
            pending.pop_next(),
            Some(SemanticWriterRequest::Embedding { .. })
        ));
        assert!(matches!(
            pending.pop_next(),
            Some(SemanticWriterRequest::Summary { .. })
        ));
        assert!(matches!(
            pending.pop_next(),
            Some(SemanticWriterRequest::Embedding { .. })
        ));
        assert!(matches!(
            pending.pop_next(),
            Some(SemanticWriterRequest::Summary { .. })
        ));
        assert!(pending.pop_next().is_none());
    }

    #[test]
    fn pending_requests_drain_summaries_when_no_embeddings_are_waiting() {
        let mut pending = PendingSemanticWriterRequests::default();
        pending.push(summary_request());
        pending.push(summary_request());

        assert!(matches!(
            pending.pop_next(),
            Some(SemanticWriterRequest::Summary { .. })
        ));
        assert!(matches!(
            pending.pop_next(),
            Some(SemanticWriterRequest::Summary { .. })
        ));
        assert!(pending.pop_next().is_none());
    }
}
