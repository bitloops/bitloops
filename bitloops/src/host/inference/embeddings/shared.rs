use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};

use super::runtime::embeddings_runtime_error_is_timeout;
use super::session::{PythonEmbeddingsSession, PythonEmbeddingsSessionConfig};

const SHARED_EMBEDDINGS_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SHARED_EMBEDDINGS_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_REMOTE_EMBEDDINGS_SESSION_POOL_SIZE: usize = 4;

pub(crate) struct SharedBitloopsEmbeddingsSessionRegistry {
    session_pools:
        Mutex<HashMap<PythonEmbeddingsSessionConfig, Arc<SharedBitloopsEmbeddingsSessionPool>>>,
}

pub(crate) struct SharedBitloopsEmbeddingsSessionPool {
    sessions: Vec<Arc<SharedBitloopsEmbeddingsSession>>,
    next_session_index: AtomicUsize,
}

pub(crate) struct SharedBitloopsEmbeddingsSession {
    config: PythonEmbeddingsSessionConfig,
    state: Mutex<SharedBitloopsEmbeddingsSessionState>,
}

struct SharedBitloopsEmbeddingsSessionState {
    session: Option<PythonEmbeddingsSession>,
    output_dimension: Option<usize>,
    last_used_at: Instant,
}

impl SharedBitloopsEmbeddingsSessionRegistry {
    pub(crate) fn get_or_create(
        &self,
        config: &PythonEmbeddingsSessionConfig,
    ) -> Result<Arc<SharedBitloopsEmbeddingsSessionPool>> {
        let mut session_pools = self
            .session_pools
            .lock()
            .map_err(|_| anyhow!("shared embeddings session registry mutex was poisoned"))?;
        let pool_size = embeddings_session_pool_size(config);
        Ok(session_pools
            .entry(config.clone())
            .or_insert_with(|| {
                Arc::new(SharedBitloopsEmbeddingsSessionPool::new(
                    config.clone(),
                    pool_size,
                ))
            })
            .clone())
    }

    fn shutdown_idle_sessions(&self, idle_timeout: Duration) {
        let session_pools = match self.session_pools.lock() {
            Ok(session_pools) => session_pools.values().cloned().collect::<Vec<_>>(),
            Err(_) => return,
        };
        for session_pool in session_pools {
            session_pool.shutdown_idle_sessions(idle_timeout);
        }
    }
}

impl SharedBitloopsEmbeddingsSessionPool {
    fn new(config: PythonEmbeddingsSessionConfig, pool_size: usize) -> Self {
        let pool_size = pool_size.max(1);
        Self {
            sessions: (0..pool_size)
                .map(|_| Arc::new(SharedBitloopsEmbeddingsSession::new(config.clone())))
                .collect(),
            next_session_index: AtomicUsize::new(0),
        }
    }

    pub(crate) fn output_dimension(&self) -> Result<usize> {
        self.first_session().output_dimension()
    }

    pub(crate) fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.next_session().embed(texts)
    }

    fn shutdown_idle_sessions(&self, idle_timeout: Duration) {
        for session in &self.sessions {
            session.shutdown_if_idle(idle_timeout);
        }
    }

    fn first_session(&self) -> &Arc<SharedBitloopsEmbeddingsSession> {
        self.sessions
            .first()
            .expect("embeddings session pool always contains at least one session")
    }

    fn next_session(&self) -> &Arc<SharedBitloopsEmbeddingsSession> {
        let index = self.next_session_index.fetch_add(1, Ordering::Relaxed) % self.sessions.len();
        &self.sessions[index]
    }
}

impl SharedBitloopsEmbeddingsSession {
    fn new(config: PythonEmbeddingsSessionConfig) -> Self {
        Self {
            config,
            state: Mutex::new(SharedBitloopsEmbeddingsSessionState {
                session: None,
                output_dimension: None,
                last_used_at: Instant::now(),
            }),
        }
    }

    pub(crate) fn output_dimension(&self) -> Result<usize> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("shared embeddings runtime session mutex was poisoned"))?;
        if let Some(output_dimension) = state.output_dimension {
            return Ok(output_dimension);
        }
        let session = self.ensure_session_started(&mut state)?;
        let output_dimension = session.probe_dimension()?;
        state.output_dimension = Some(output_dimension);
        state.last_used_at = Instant::now();
        Ok(output_dimension)
    }

    pub(crate) fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("shared embeddings runtime session mutex was poisoned"))?;
        match self.ensure_session_started(&mut state)?.embed(texts) {
            Ok(vectors) => {
                state.last_used_at = Instant::now();
                Ok(vectors)
            }
            Err(first_err) => {
                state.session = None;
                if embeddings_runtime_error_is_timeout(&first_err) {
                    return Err(first_err);
                }
                let restarted = PythonEmbeddingsSession::start(&self.config).context(
                    "restarting standalone `bitloops-local-embeddings` runtime after failure",
                )?;
                state.session = Some(restarted);
                let retry = state
                    .session
                    .as_mut()
                    .expect("session replaced above")
                    .embed(texts)
                    .with_context(|| {
                        format!(
                            "retrying standalone `bitloops-local-embeddings` runtime request after failure: {first_err:#}"
                        )
                    });
                match retry {
                    Ok(vectors) => {
                        state.last_used_at = Instant::now();
                        Ok(vectors)
                    }
                    Err(err) => {
                        state.session = None;
                        Err(err)
                    }
                }
            }
        }
    }

    fn shutdown_if_idle(&self, idle_timeout: Duration) {
        let session = {
            let mut state = match self.state.try_lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            if state.session.is_none() || state.last_used_at.elapsed() < idle_timeout {
                return;
            }
            state.session.take()
        };
        drop(session);
    }

    fn ensure_session_started<'a>(
        &self,
        state: &'a mut SharedBitloopsEmbeddingsSessionState,
    ) -> Result<&'a mut PythonEmbeddingsSession> {
        if state.session.is_none() {
            state.session = Some(PythonEmbeddingsSession::start(&self.config)?);
        }
        Ok(state.session.as_mut().expect("session ensured above"))
    }
}

pub(crate) fn shared_bitloops_embeddings_session_registry()
-> &'static Arc<SharedBitloopsEmbeddingsSessionRegistry> {
    static REGISTRY: OnceLock<Arc<SharedBitloopsEmbeddingsSessionRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry = Arc::new(SharedBitloopsEmbeddingsSessionRegistry {
            session_pools: Mutex::new(HashMap::new()),
        });
        let sweeper_registry = Arc::clone(&registry);
        let _ = thread::Builder::new()
            .name("bitloops-local-embeddings-ipc-sweeper".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(SHARED_EMBEDDINGS_SWEEP_INTERVAL);
                    sweeper_registry.shutdown_idle_sessions(SHARED_EMBEDDINGS_IDLE_TIMEOUT);
                }
            });
        registry
    })
}

#[cfg(test)]
pub(crate) fn evict_idle_embeddings_sessions_for_tests(idle_timeout: Duration) {
    shared_bitloops_embeddings_session_registry().shutdown_idle_sessions(idle_timeout);
}

fn embeddings_session_pool_size(config: &PythonEmbeddingsSessionConfig) -> usize {
    if config.platform_backed {
        DEFAULT_REMOTE_EMBEDDINGS_SESSION_POOL_SIZE
    } else {
        1
    }
}
