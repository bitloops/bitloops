use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::adapters::model_providers::llm::{self, LlmProvider};
use crate::config::{
    InferenceConfig, InferenceProfileConfig, InferenceRuntimeConfig, InferenceTask,
};

pub const BITLOOPS_EMBEDDINGS_IPC_DRIVER: &str = "bitloops_embeddings_ipc";
pub const BITLOOPS_EMBEDDINGS_RUNTIME_ID: &str = "bitloops_embeddings";
const PYTHON_EMBEDDINGS_DIMENSION_PROBE_TEXT: &str = "bitloops python embedding dimension probe";
const SHARED_EMBEDDINGS_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SHARED_EMBEDDINGS_SWEEP_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingInputType {
    Document,
    Query,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceSlotDescriptor {
    pub name: &'static str,
    pub task: InferenceTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInferenceSlot {
    pub capability_id: String,
    pub slot_name: String,
    pub profile_name: String,
    pub task: Option<InferenceTask>,
    pub driver: Option<String>,
    pub runtime: Option<String>,
    pub model: Option<String>,
}

pub trait EmbeddingService: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn output_dimension(&self) -> Option<usize>;
    fn cache_key(&self) -> String;
    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>>;
}

pub trait TextGenerationService: Send + Sync {
    fn descriptor(&self) -> String;
    fn cache_key(&self) -> String {
        self.descriptor()
    }
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String>;
}

pub trait InferenceGateway: Send + Sync {
    fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>>;
    fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>>;

    fn has_slot(&self, _slot_name: &str) -> bool {
        false
    }

    fn describe(&self, _slot_name: &str) -> Option<ResolvedInferenceSlot> {
        None
    }
}

pub struct EmptyInferenceGateway;

impl InferenceGateway for EmptyInferenceGateway {
    fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
        bail!("embedding inference is not available for slot `{slot_name}`")
    }

    fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
        bail!("text-generation inference is not available for slot `{slot_name}`")
    }
}

pub struct LocalInferenceGateway {
    #[allow(dead_code)]
    repo_root: PathBuf,
    inference: InferenceConfig,
    slot_bindings: HashMap<String, BTreeMap<String, String>>,
    embedding_cache: Mutex<HashMap<String, Arc<dyn EmbeddingService>>>,
    text_generation_cache: Mutex<HashMap<String, Arc<dyn TextGenerationService>>>,
}

impl LocalInferenceGateway {
    pub fn new(
        repo_root: &Path,
        inference: InferenceConfig,
        slot_bindings: HashMap<String, BTreeMap<String, String>>,
    ) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
            inference,
            slot_bindings,
            embedding_cache: Mutex::new(HashMap::new()),
            text_generation_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn scoped<'a>(&'a self, capability_id: Option<&'a str>) -> ScopedInferenceGateway<'a> {
        ScopedInferenceGateway {
            inner: self,
            capability_id,
        }
    }

    fn bound_profile_name(&self, capability_id: Option<&str>, slot_name: &str) -> Option<String> {
        let capability_id = capability_id?;
        self.slot_bindings
            .get(capability_id)
            .and_then(|slots| slots.get(slot_name))
            .cloned()
    }

    fn describe_slot(
        &self,
        capability_id: Option<&str>,
        slot_name: &str,
    ) -> Option<ResolvedInferenceSlot> {
        let capability_id = capability_id?;
        let profile_name = self.bound_profile_name(Some(capability_id), slot_name)?;
        let profile = self.inference.profiles.get(&profile_name);
        Some(ResolvedInferenceSlot {
            capability_id: capability_id.to_string(),
            slot_name: slot_name.to_string(),
            profile_name,
            task: profile.map(|profile| profile.task),
            driver: profile.map(|profile| profile.driver.clone()),
            runtime: profile.and_then(|profile| profile.runtime.clone()),
            model: profile.and_then(|profile| profile.model.clone()),
        })
    }

    fn resolve_profile_for_slot(
        &self,
        capability_id: Option<&str>,
        slot_name: &str,
        expected_task: InferenceTask,
    ) -> Result<(String, &InferenceProfileConfig)> {
        let profile_name = if let Some(capability_id) = capability_id {
            let Some(profile_name) = self.bound_profile_name(Some(capability_id), slot_name) else {
                bail!("capability `{capability_id}` does not bind inference slot `{slot_name}`");
            };
            profile_name
        } else if self.inference.profiles.contains_key(slot_name) {
            slot_name.to_string()
        } else {
            bail!("inference slot `{slot_name}` requires an active capability scope");
        };
        let profile = self
            .inference
            .profiles
            .get(&profile_name)
            .ok_or_else(|| anyhow!("inference profile `{profile_name}` is not defined"))?;
        if profile.task != expected_task {
            bail!(
                "inference profile `{profile_name}` is bound to slot `{slot_name}` but has task `{}` instead of `{}`",
                profile.task,
                expected_task
            );
        }
        Ok((profile_name, profile))
    }

    fn build_embedding_service(
        &self,
        profile_name: &str,
        profile: &InferenceProfileConfig,
    ) -> Result<Arc<dyn EmbeddingService>> {
        match profile.driver.as_str() {
            BITLOOPS_EMBEDDINGS_IPC_DRIVER => {
                let runtime_name = profile
                    .runtime
                    .as_deref()
                    .ok_or_else(|| anyhow!("profile `{profile_name}` requires a runtime"))?;
                let runtime = self
                    .inference
                    .runtimes
                    .get(runtime_name)
                    .ok_or_else(|| anyhow!("runtime `{runtime_name}` is not defined"))?;
                let model = profile
                    .model
                    .as_deref()
                    .ok_or_else(|| anyhow!("profile `{profile_name}` requires a model"))?;
                if profile.api_key.is_some() || profile.base_url.is_some() {
                    bail!(
                        "profile `{profile_name}` uses driver `{}` and cannot declare `api_key` or `base_url`",
                        BITLOOPS_EMBEDDINGS_IPC_DRIVER
                    );
                }

                let service = BitloopsEmbeddingsIpcService::new(
                    profile_name,
                    runtime,
                    model,
                    profile.cache_dir.as_deref(),
                )
                .with_context(|| {
                    format!(
                        "building `{BITLOOPS_EMBEDDINGS_IPC_DRIVER}` service for profile `{profile_name}`"
                    )
                })?;
                Ok(Arc::new(service))
            }
            other => bail!("unsupported embeddings driver `{other}`"),
        }
    }

    fn build_text_generation_service(
        &self,
        profile_name: &str,
        profile: &InferenceProfileConfig,
    ) -> Result<Arc<dyn TextGenerationService>> {
        let model = profile
            .model
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("profile `{profile_name}` requires a model"))?
            .trim()
            .to_string();
        let api_key = profile
            .api_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("profile `{profile_name}` requires an api_key"))?
            .trim()
            .to_string();
        let provider =
            llm::build_llm_provider(&profile.driver, model, api_key, profile.base_url.as_deref())
                .with_context(|| {
                format!("building text-generation service for profile `{profile_name}`")
            })?;
        Ok(Arc::new(LlmTextGenerationService { inner: provider }))
    }
}

impl InferenceGateway for LocalInferenceGateway {
    fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
        self.scoped(None).embeddings(slot_name)
    }

    fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
        self.scoped(None).text_generation(slot_name)
    }

    fn has_slot(&self, slot_name: &str) -> bool {
        self.inference.profiles.contains_key(slot_name)
    }
}

pub struct ScopedInferenceGateway<'a> {
    inner: &'a LocalInferenceGateway,
    capability_id: Option<&'a str>,
}

impl InferenceGateway for ScopedInferenceGateway<'_> {
    fn embeddings(&self, slot_name: &str) -> Result<Arc<dyn EmbeddingService>> {
        let (profile_name, profile) = self.inner.resolve_profile_for_slot(
            self.capability_id,
            slot_name,
            InferenceTask::Embeddings,
        )?;
        if let Some(service) = self
            .inner
            .embedding_cache
            .lock()
            .map_err(|_| anyhow!("embedding inference cache mutex was poisoned"))?
            .get(&profile_name)
            .cloned()
        {
            return Ok(service);
        }

        let service = self.inner.build_embedding_service(&profile_name, profile)?;
        let mut cache = self
            .inner
            .embedding_cache
            .lock()
            .map_err(|_| anyhow!("embedding inference cache mutex was poisoned"))?;
        Ok(cache
            .entry(profile_name)
            .or_insert_with(|| Arc::clone(&service))
            .clone())
    }

    fn text_generation(&self, slot_name: &str) -> Result<Arc<dyn TextGenerationService>> {
        let (profile_name, profile) = self.inner.resolve_profile_for_slot(
            self.capability_id,
            slot_name,
            InferenceTask::TextGeneration,
        )?;
        if let Some(service) = self
            .inner
            .text_generation_cache
            .lock()
            .map_err(|_| anyhow!("text-generation inference cache mutex was poisoned"))?
            .get(&profile_name)
            .cloned()
        {
            return Ok(service);
        }

        let service = self
            .inner
            .build_text_generation_service(&profile_name, profile)?;
        let mut cache = self
            .inner
            .text_generation_cache
            .lock()
            .map_err(|_| anyhow!("text-generation inference cache mutex was poisoned"))?;
        Ok(cache
            .entry(profile_name)
            .or_insert_with(|| Arc::clone(&service))
            .clone())
    }

    fn has_slot(&self, slot_name: &str) -> bool {
        self.inner
            .bound_profile_name(self.capability_id, slot_name)
            .is_some()
    }

    fn describe(&self, slot_name: &str) -> Option<ResolvedInferenceSlot> {
        self.inner.describe_slot(self.capability_id, slot_name)
    }
}

struct LlmTextGenerationService {
    inner: Box<dyn LlmProvider>,
}

impl TextGenerationService for LlmTextGenerationService {
    fn descriptor(&self) -> String {
        self.inner.descriptor()
    }

    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.inner
            .complete(system_prompt, user_prompt)
            .ok_or_else(|| {
                anyhow!(
                    "text-generation provider `{}` returned no content",
                    self.descriptor()
                )
            })
    }
}

struct BitloopsEmbeddingsIpcService {
    profile_name: String,
    model_name: String,
    output_dimension: usize,
    cache_key: String,
    shared_session: Arc<SharedBitloopsEmbeddingsSession>,
}

impl BitloopsEmbeddingsIpcService {
    fn new(
        profile_name: &str,
        runtime: &InferenceRuntimeConfig,
        model: &str,
        cache_dir: Option<&Path>,
    ) -> Result<Self> {
        let session_config = PythonEmbeddingsSessionConfig {
            command: runtime.command.clone(),
            args: runtime.args.clone(),
            startup_timeout_secs: runtime.startup_timeout_secs,
            request_timeout_secs: runtime.request_timeout_secs,
            model: model.to_string(),
            cache_dir: cache_dir.map(Path::to_path_buf),
            launch_artifact_fingerprint: embeddings_runtime_launch_artifact_fingerprint(
                &runtime.command,
                &runtime.args,
            ),
            process_environment_fingerprint: process_environment_fingerprint(),
        };
        let shared_session =
            shared_bitloops_embeddings_session_registry().get_or_create(&session_config)?;
        let output_dimension = shared_session.output_dimension()?;
        let cache_key = format!(
            "profile={profile_name}::driver={BITLOOPS_EMBEDDINGS_IPC_DRIVER}::model={model}::dimension={output_dimension}"
        );

        Ok(Self {
            profile_name: profile_name.to_string(),
            model_name: model.to_string(),
            output_dimension,
            cache_key,
            shared_session,
        })
    }
}

impl EmbeddingService for BitloopsEmbeddingsIpcService {
    fn provider_name(&self) -> &str {
        BITLOOPS_EMBEDDINGS_IPC_DRIVER
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn output_dimension(&self) -> Option<usize> {
        Some(self.output_dimension)
    }

    fn cache_key(&self) -> String {
        self.cache_key.clone()
    }

    fn embed(&self, input: &str, _input_type: EmbeddingInputType) -> Result<Vec<f32>> {
        let input = input.trim();
        if input.is_empty() {
            bail!("embedding input cannot be empty");
        }

        let texts = vec![input.to_string()];
        let mut vectors = self.shared_session.embed(&texts).with_context(|| {
            format!(
                "requesting standalone `bitloops-embeddings` runtime for profile `{}`",
                self.profile_name
            )
        })?;
        let vector = vectors
            .drain(..)
            .next()
            .ok_or_else(|| anyhow!("standalone embeddings runtime returned no vectors"))?;
        if vector.is_empty() {
            bail!("standalone embeddings runtime returned an empty vector");
        }
        if vector.len() != self.output_dimension {
            bail!(
                "standalone embeddings runtime returned dimension {} but expected {}",
                vector.len(),
                self.output_dimension
            );
        }
        Ok(vector)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PythonEmbeddingsSessionConfig {
    command: String,
    args: Vec<String>,
    startup_timeout_secs: u64,
    request_timeout_secs: u64,
    model: String,
    cache_dir: Option<PathBuf>,
    launch_artifact_fingerprint: String,
    process_environment_fingerprint: String,
}

struct PythonEmbeddingsSession {
    config: PythonEmbeddingsSessionConfig,
    child: Child,
    stdin: ChildStdin,
    response_rx: Receiver<PythonEmbeddingsSessionOutput>,
}

enum PythonEmbeddingsSessionOutput {
    Json(Value),
    ReadError(String),
    Closed,
}

struct SharedBitloopsEmbeddingsSessionRegistry {
    sessions: Mutex<HashMap<PythonEmbeddingsSessionConfig, Arc<SharedBitloopsEmbeddingsSession>>>,
}

struct SharedBitloopsEmbeddingsSession {
    config: PythonEmbeddingsSessionConfig,
    state: Mutex<SharedBitloopsEmbeddingsSessionState>,
}

struct SharedBitloopsEmbeddingsSessionState {
    session: Option<PythonEmbeddingsSession>,
    output_dimension: Option<usize>,
    last_used_at: Instant,
}

impl SharedBitloopsEmbeddingsSessionRegistry {
    fn get_or_create(
        &self,
        config: &PythonEmbeddingsSessionConfig,
    ) -> Result<Arc<SharedBitloopsEmbeddingsSession>> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("shared embeddings session registry mutex was poisoned"))?;
        Ok(sessions
            .entry(config.clone())
            .or_insert_with(|| Arc::new(SharedBitloopsEmbeddingsSession::new(config.clone())))
            .clone())
    }

    fn shutdown_idle_sessions(&self, idle_timeout: Duration) {
        let sessions = match self.sessions.lock() {
            Ok(sessions) => sessions.values().cloned().collect::<Vec<_>>(),
            Err(_) => return,
        };
        for session in sessions {
            session.shutdown_if_idle(idle_timeout);
        }
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

    fn output_dimension(&self) -> Result<usize> {
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

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
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
                let restarted = PythonEmbeddingsSession::start(&self.config)
                    .context("restarting standalone `bitloops-embeddings` runtime after failure")?;
                state.session = Some(restarted);
                let retry = state
                    .session
                    .as_mut()
                    .expect("session replaced above")
                    .embed(texts)
                    .with_context(|| {
                        format!(
                            "retrying standalone `bitloops-embeddings` runtime request after failure: {first_err:#}"
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

fn shared_bitloops_embeddings_session_registry()
-> &'static Arc<SharedBitloopsEmbeddingsSessionRegistry> {
    static REGISTRY: OnceLock<Arc<SharedBitloopsEmbeddingsSessionRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry = Arc::new(SharedBitloopsEmbeddingsSessionRegistry {
            sessions: Mutex::new(HashMap::new()),
        });
        let sweeper_registry = Arc::clone(&registry);
        let _ = thread::Builder::new()
            .name("bitloops-embeddings-ipc-sweeper".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(SHARED_EMBEDDINGS_SWEEP_INTERVAL);
                    sweeper_registry.shutdown_idle_sessions(SHARED_EMBEDDINGS_IDLE_TIMEOUT);
                }
            });
        registry
    })
}

fn process_environment_fingerprint() -> String {
    let mut vars = std::env::vars_os()
        .map(|(key, value)| format!("{}={}", key.to_string_lossy(), value.to_string_lossy()))
        .collect::<Vec<_>>();
    vars.sort();
    sha256_hex(vars.join("\n").as_bytes())
}

fn embeddings_runtime_launch_artifact_fingerprint(command: &str, args: &[String]) -> String {
    let command_path = Path::new(command);
    let mut candidates = vec![command];
    if runtime_command_uses_script_argument(command_path)
        && let Some(script_path) = args.first()
    {
        candidates.push(script_path.as_str());
    }

    let mut artefacts = candidates
        .into_iter()
        .filter_map(|candidate| {
            let path = Path::new(candidate);
            if !path.is_file() {
                return None;
            }
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let metadata = std::fs::metadata(&canonical).ok()?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|timestamp| timestamp.duration_since(UNIX_EPOCH).ok())
                .map(|duration| format!("{}:{}", duration.as_secs(), duration.subsec_nanos()))
                .unwrap_or_else(|| "unknown".to_string());
            Some(format!(
                "{}|{}|{}",
                canonical.display(),
                metadata.len(),
                modified
            ))
        })
        .collect::<Vec<_>>();
    artefacts.sort();
    sha256_hex(artefacts.join("\n").as_bytes())
}

fn runtime_command_uses_script_argument(command: &Path) -> bool {
    command
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "sh" | "bash"
                    | "zsh"
                    | "python"
                    | "python3"
                    | "python3.11"
                    | "python3.12"
                    | "python3.13"
                    | "node"
                    | "ruby"
                    | "perl"
                    | "pwsh"
                    | "powershell"
            )
        })
        .unwrap_or(false)
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
fn evict_idle_embeddings_sessions_for_tests(idle_timeout: Duration) {
    shared_bitloops_embeddings_session_registry().shutdown_idle_sessions(idle_timeout);
}

impl PythonEmbeddingsSession {
    fn start(config: &PythonEmbeddingsSessionConfig) -> Result<Self> {
        let mut command = Command::new(&config.command);
        command.args(&config.args);
        command.arg("daemon");
        command.arg("--model").arg(&config.model);
        if let Some(cache_dir) = config.cache_dir.as_ref() {
            command.arg("--cache-dir").arg(cache_dir);
        }
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::inherit());

        let mut child = command.spawn().with_context(|| {
            format!(
                "spawning standalone `bitloops-embeddings` runtime `{}` for model `{}`",
                config.command, config.model
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .context("capturing standalone embeddings runtime stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("capturing standalone embeddings runtime stdout")?;
        let mut session = Self {
            config: config.clone(),
            child,
            stdin,
            response_rx: Self::spawn_stdout_reader(stdout),
        };
        session.wait_for_ready()?;
        Ok(session)
    }

    fn wait_for_ready(&mut self) -> Result<()> {
        loop {
            let value = self.read_json_response(
                self.config.startup_timeout_secs,
                "waiting for standalone embeddings runtime readiness",
            )?;
            if value.get("event").and_then(Value::as_str) == Some("ready") {
                return Ok(());
            }
        }
    }

    fn probe_dimension(&mut self) -> Result<usize> {
        let texts = vec![PYTHON_EMBEDDINGS_DIMENSION_PROBE_TEXT.to_string()];
        let vectors = self.embed(&texts)?;
        let vector = vectors
            .first()
            .ok_or_else(|| anyhow!("standalone embeddings runtime returned no probe vector"))?;
        if vector.is_empty() {
            bail!("standalone embeddings runtime returned an empty probe vector");
        }
        Ok(vector.len())
    }

    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let request_id = next_request_id();
        let request = json!({
            "id": request_id,
            "cmd": "embed",
            "model": self.config.model,
            "texts": texts,
        });
        self.write_json_line(&request)?;
        let value = self.read_json_response(
            self.config.request_timeout_secs,
            "waiting for standalone embeddings runtime response",
        )?;
        if value.get("id").and_then(Value::as_str) != Some(request_id.as_str()) {
            self.terminate_child();
            bail!("standalone embeddings runtime returned mismatched request id");
        }
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            self.terminate_child();
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown standalone embeddings runtime error");
            bail!("{message}");
        }

        let vectors = value
            .get("vectors")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow!("standalone embeddings runtime response did not include vectors")
            })?;
        let mut out = Vec::with_capacity(vectors.len());
        for vector in vectors {
            let values = vector.as_array().ok_or_else(|| {
                anyhow!("standalone embeddings runtime returned a non-array vector")
            })?;
            let mut row = Vec::with_capacity(values.len());
            for value in values {
                let Some(number) = value.as_f64() else {
                    bail!("standalone embeddings runtime returned a non-numeric embedding value");
                };
                if !number.is_finite() {
                    bail!("standalone embeddings runtime returned a non-finite embedding value");
                }
                row.push(number as f32);
            }
            out.push(row);
        }
        Ok(out)
    }

    fn shutdown(&mut self) -> Result<()> {
        let request = json!({
            "id": next_request_id(),
            "cmd": "shutdown",
            "model": self.config.model,
        });
        self.write_json_line(&request)?;
        let _ = self.read_json_response(1, "waiting for standalone embeddings runtime shutdown");
        self.terminate_child();
        Ok(())
    }

    fn write_json_line(&mut self, value: &Value) -> Result<()> {
        let line = serde_json::to_string(value)
            .context("serializing standalone embeddings runtime request")?;
        writeln!(self.stdin, "{line}").context("writing standalone embeddings runtime request")?;
        self.stdin
            .flush()
            .context("flushing standalone embeddings runtime request")
    }

    fn spawn_stdout_reader(stdout: ChildStdout) -> Receiver<PythonEmbeddingsSessionOutput> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = tx.send(PythonEmbeddingsSessionOutput::Closed);
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim_end();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str(trimmed) {
                            Ok(value) => {
                                if tx.send(PythonEmbeddingsSessionOutput::Json(value)).is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                let _ = tx.send(PythonEmbeddingsSessionOutput::ReadError(
                                    anyhow!(
                                        "parsing standalone embeddings runtime response `{trimmed}`: {err}"
                                    )
                                    .to_string(),
                                ));
                                break;
                            }
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(PythonEmbeddingsSessionOutput::ReadError(
                            anyhow!(err)
                                .context("reading standalone embeddings runtime response")
                                .to_string(),
                        ));
                        break;
                    }
                }
            }
        });
        rx
    }

    fn read_json_response(&mut self, timeout_secs: u64, operation: &str) -> Result<Value> {
        let next = if timeout_secs == 0 {
            self.response_rx
                .recv()
                .map_err(|_| anyhow!("standalone embeddings runtime exited before replying"))
        } else {
            self.response_rx
                .recv_timeout(Duration::from_secs(timeout_secs))
                .map_err(|err| match err {
                    RecvTimeoutError::Timeout => {
                        anyhow!("{operation} timed out after {timeout_secs}s")
                    }
                    RecvTimeoutError::Disconnected => {
                        anyhow!("standalone embeddings runtime exited before replying")
                    }
                })
        };
        match next {
            Ok(PythonEmbeddingsSessionOutput::Json(value)) => Ok(value),
            Ok(PythonEmbeddingsSessionOutput::ReadError(message)) => {
                self.terminate_child();
                Err(anyhow!(message))
            }
            Ok(PythonEmbeddingsSessionOutput::Closed) => {
                self.terminate_child();
                Err(anyhow!(
                    "standalone embeddings runtime exited before replying"
                ))
            }
            Err(err) => {
                self.terminate_child();
                Err(err)
            }
        }
    }

    fn terminate_child(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PythonEmbeddingsSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn next_request_id() -> String {
    static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
    format!(
        "inference-{}",
        NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use tempfile::TempDir;

    fn write_fake_runtime_script(script_path: &Path, timeout_marker: Option<&Path>) {
        let timeout_branch = timeout_marker
            .map(|path| {
                format!(
                    r#"
          if [ ! -f "{path}" ]; then
            : > "{path}"
            sleep 2
          fi
"#,
                    path = path.display()
                )
            })
            .unwrap_or_default();
        fs::write(
            script_path,
            format!(
                r#"launch_log="$1"
shift
printf '%s\n' "$$" >> "$launch_log"
printf '%s\n' '{{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}}'

while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"shutdown"'*)
      printf '{{"id":"%s","ok":true}}\n' "$request_id"
      exit 0
      ;;
    *'"cmd":"embed"'*)
      case "$line" in
        *'bitloops python embedding dimension probe'*)
          printf '{{"id":"%s","ok":true,"vectors":[[1.0,2.0]]}}\n' "$request_id"
          ;;
        *)
{timeout_branch}          printf '{{"id":"%s","ok":true,"vectors":[[1.0,2.0]]}}\n' "$request_id"
          ;;
      esac
      ;;
  esac
done
"#,
                timeout_branch = timeout_branch,
            ),
        )
        .expect("write fake runtime script");
    }

    fn fake_runtime_config(script_path: &Path, launch_log: &Path) -> InferenceRuntimeConfig {
        InferenceRuntimeConfig {
            command: "/bin/sh".to_string(),
            args: vec![
                script_path.to_string_lossy().into_owned(),
                launch_log.to_string_lossy().into_owned(),
            ],
            startup_timeout_secs: 1,
            request_timeout_secs: 1,
        }
    }

    #[test]
    fn empty_gateway_rejects_unknown_slots() {
        let gateway = EmptyInferenceGateway;
        let err = match gateway.embeddings("code_embeddings") {
            Ok(_) => panic!("missing slot must fail"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("code_embeddings"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn scoped_gateway_reports_bound_slots() {
        let gateway = LocalInferenceGateway::new(
            Path::new("/repo"),
            InferenceConfig::default(),
            HashMap::from([(
                "semantic_clones".to_string(),
                BTreeMap::from([("code_embeddings".to_string(), "local".to_string())]),
            )]),
        );
        let scoped = gateway.scoped(Some("semantic_clones"));

        assert!(scoped.has_slot("code_embeddings"));
        assert!(!scoped.has_slot("summary_embeddings"));
        let description = scoped
            .describe("code_embeddings")
            .expect("slot description");
        assert_eq!(description.profile_name, "local");
    }

    #[test]
    fn ipc_service_restarts_after_request_timeout() {
        let temp = TempDir::new().expect("temp dir");
        let script_path = temp.path().join("fake_embeddings_runtime.sh");
        let launch_log = temp.path().join("launches.log");
        let timeout_marker = temp.path().join("first-request-timed-out");
        write_fake_runtime_script(&script_path, Some(&timeout_marker));

        let runtime = fake_runtime_config(&script_path, &launch_log);
        let service = BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None)
            .expect("build ipc service");

        let vector = service
            .embed("hello world", EmbeddingInputType::Document)
            .expect("embedding request should recover after timeout");

        assert_eq!(vector, vec![1.0, 2.0]);
        assert!(
            timeout_marker.exists(),
            "first request should have timed out"
        );
    }

    #[test]
    fn ipc_service_reuses_hot_runtime_across_service_instances() {
        let temp = TempDir::new().expect("temp dir");
        let script_path = temp.path().join("fake_embeddings_runtime.sh");
        let launch_log = temp.path().join("launches.log");
        write_fake_runtime_script(&script_path, None);

        let runtime = fake_runtime_config(&script_path, &launch_log);
        let first = BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None)
            .expect("build first ipc service");
        assert_eq!(
            first
                .embed("hello world", EmbeddingInputType::Document)
                .expect("first embed"),
            vec![1.0, 2.0]
        );
        drop(first);

        let second = BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None)
            .expect("build second ipc service");
        assert_eq!(
            second
                .embed("goodbye world", EmbeddingInputType::Document)
                .expect("second embed"),
            vec![1.0, 2.0]
        );

        let launches = fs::read_to_string(&launch_log).expect("read launch log");
        assert_eq!(
            launches.lines().count(),
            1,
            "expected one shared runtime launch, got: {launches}"
        );
    }

    #[test]
    fn ipc_service_shuts_down_after_idle_eviction() {
        let temp = TempDir::new().expect("temp dir");
        let script_path = temp.path().join("fake_embeddings_runtime.sh");
        let launch_log = temp.path().join("launches.log");
        write_fake_runtime_script(&script_path, None);

        let runtime = fake_runtime_config(&script_path, &launch_log);
        let first = BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None)
            .expect("build first ipc service");
        assert_eq!(
            first
                .embed("hello world", EmbeddingInputType::Document)
                .expect("first embed"),
            vec![1.0, 2.0]
        );

        evict_idle_embeddings_sessions_for_tests(Duration::ZERO);

        let second = BitloopsEmbeddingsIpcService::new("local_code", &runtime, "test-model", None)
            .expect("build second ipc service");
        assert_eq!(
            second
                .embed("goodbye world", EmbeddingInputType::Document)
                .expect("second embed"),
            vec![1.0, 2.0]
        );

        let launches = fs::read_to_string(&launch_log).expect("read launch log");
        assert_eq!(
            launches.lines().count(),
            2,
            "expected idle eviction to force a second runtime launch, got: {launches}"
        );
    }
}
