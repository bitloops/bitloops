use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
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
    while let Ok(request) = receiver.recv() {
        match (&mut connection, request) {
            (Some(connection), SemanticWriterRequest::Summary { request, response }) => {
                let result = with_semantic_writer_sqlite_locks_map(
                    &runtime_db_path,
                    &relational_db_path,
                    |err| {
                        SummaryCommitFailure::new(
                            SummaryCommitPhase::TransactionStart,
                            SummaryCommitPhaseTimings::default(),
                            false,
                            err.context("acquiring semantic summary SQLite write lock"),
                        )
                    },
                    || execute_summary_commit(connection, &request),
                )
                .map_err(|err| {
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
                let result = with_semantic_writer_sqlite_locks(
                    &runtime_db_path,
                    &relational_db_path,
                    || execute_embedding_commit(connection, &request),
                )
                .map_err(|err| {
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

pub(super) fn with_semantic_writer_sqlite_locks<T>(
    runtime_db_path: &Path,
    relational_db_path: &Path,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    with_semantic_writer_sqlite_locks_map(runtime_db_path, relational_db_path, |err| err, operation)
}

fn with_semantic_writer_sqlite_locks_map<T, E>(
    runtime_db_path: &Path,
    relational_db_path: &Path,
    map_lock_error: impl Fn(anyhow::Error) -> E,
    operation: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let runtime_key = canonical_lock_order_path(runtime_db_path);
    let relational_key = canonical_lock_order_path(relational_db_path);
    if runtime_key <= relational_key {
        crate::storage::sqlite::with_sqlite_write_lock_map(runtime_db_path, &map_lock_error, || {
            crate::storage::sqlite::with_sqlite_write_lock_map(
                relational_db_path,
                &map_lock_error,
                operation,
            )
        })
    } else {
        crate::storage::sqlite::with_sqlite_write_lock_map(
            relational_db_path,
            &map_lock_error,
            || {
                crate::storage::sqlite::with_sqlite_write_lock_map(
                    runtime_db_path,
                    &map_lock_error,
                    operation,
                )
            },
        )
    }
}

fn canonical_lock_order_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
