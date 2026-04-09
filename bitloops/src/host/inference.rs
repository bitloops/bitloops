use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::adapters::model_providers::llm::{self, LlmProvider};
use crate::config::{
    InferenceConfig, InferenceProfileConfig, InferenceRuntimeConfig, InferenceTask,
};

pub const BITLOOPS_EMBEDDINGS_IPC_DRIVER: &str = "bitloops_embeddings_ipc";
pub const BITLOOPS_EMBEDDINGS_RUNTIME_ID: &str = "bitloops_embeddings";
const PYTHON_EMBEDDINGS_DIMENSION_PROBE_TEXT: &str = "bitloops python embedding dimension probe";

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
    session_config: PythonEmbeddingsSessionConfig,
    session: Mutex<PythonEmbeddingsSession>,
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
        };
        let mut session = PythonEmbeddingsSession::start(&session_config)?;
        let output_dimension = session.probe_dimension()?;
        let cache_key = format!(
            "profile={profile_name}::driver={BITLOOPS_EMBEDDINGS_IPC_DRIVER}::model={model}::dimension={output_dimension}"
        );

        Ok(Self {
            profile_name: profile_name.to_string(),
            model_name: model.to_string(),
            output_dimension,
            cache_key,
            session_config,
            session: Mutex::new(session),
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
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("python embeddings session mutex was poisoned"))?;
        match session.embed(&texts) {
            Ok(mut vectors) => {
                let vector = vectors
                    .drain(..)
                    .next()
                    .ok_or_else(|| anyhow!("python embeddings daemon returned no vectors"))?;
                if vector.is_empty() {
                    bail!("python embeddings daemon returned an empty vector");
                }
                Ok(vector)
            }
            Err(first_err) => {
                *session =
                    PythonEmbeddingsSession::start(&self.session_config).with_context(|| {
                        format!(
                            "restarting python embeddings daemon for profile `{}` after failure",
                            self.profile_name
                        )
                    })?;
                let mut vectors = session.embed(&texts).with_context(|| {
                    format!(
                        "retrying python embeddings daemon request for profile `{}`",
                        self.profile_name
                    )
                })?;
                let vector = vectors
                    .drain(..)
                    .next()
                    .ok_or_else(|| anyhow!("python embeddings daemon returned no vectors"))?;
                if vector.is_empty() {
                    bail!("python embeddings daemon returned an empty vector");
                }
                if vector.len() != self.output_dimension {
                    bail!(
                        "python embeddings daemon returned dimension {} but expected {} after restart; initial failure: {first_err:#}",
                        vector.len(),
                        self.output_dimension
                    );
                }
                Ok(vector)
            }
        }
    }
}

#[derive(Debug, Clone)]
struct PythonEmbeddingsSessionConfig {
    command: String,
    args: Vec<String>,
    startup_timeout_secs: u64,
    request_timeout_secs: u64,
    model: String,
    cache_dir: Option<PathBuf>,
}

struct PythonEmbeddingsSession {
    config: PythonEmbeddingsSessionConfig,
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
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
                "spawning python embeddings runtime `{}` for model `{}`",
                config.command, config.model
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .context("capturing python embeddings daemon stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("capturing python embeddings daemon stdout")?;
        let mut session = Self {
            config: config.clone(),
            child,
            stdin,
            reader: BufReader::new(stdout),
        };
        session.wait_for_ready()?;
        Ok(session)
    }

    fn wait_for_ready(&mut self) -> Result<()> {
        let _timeout = self.config.startup_timeout_secs;
        loop {
            let value = self.read_json_line()?;
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
            .ok_or_else(|| anyhow!("python embeddings daemon returned no probe vector"))?;
        if vector.is_empty() {
            bail!("python embeddings daemon returned an empty probe vector");
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
        let _timeout = self.config.request_timeout_secs;
        let value = self.read_json_line()?;
        if value.get("id").and_then(Value::as_str) != Some(request_id.as_str()) {
            bail!("python embeddings daemon returned mismatched request id");
        }
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown python embeddings daemon error");
            bail!("{message}");
        }

        let vectors = value
            .get("vectors")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("python embeddings daemon response did not include vectors"))?;
        let mut out = Vec::with_capacity(vectors.len());
        for vector in vectors {
            let values = vector
                .as_array()
                .ok_or_else(|| anyhow!("python embeddings daemon returned a non-array vector"))?;
            let mut row = Vec::with_capacity(values.len());
            for value in values {
                let Some(number) = value.as_f64() else {
                    bail!("python embeddings daemon returned a non-numeric embedding value");
                };
                if !number.is_finite() {
                    bail!("python embeddings daemon returned a non-finite embedding value");
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
        let _ = self.read_json_line();
        Ok(())
    }

    fn write_json_line(&mut self, value: &Value) -> Result<()> {
        let line =
            serde_json::to_string(value).context("serializing python embeddings daemon request")?;
        writeln!(self.stdin, "{line}").context("writing python embeddings daemon request")?;
        self.stdin
            .flush()
            .context("flushing python embeddings daemon request")
    }

    fn read_json_line(&mut self) -> Result<Value> {
        let mut line = String::new();
        let bytes = self
            .reader
            .read_line(&mut line)
            .context("reading python embeddings daemon response")?;
        if bytes == 0 {
            bail!("python embeddings daemon exited before replying");
        }
        serde_json::from_str(line.trim_end()).context("parsing python embeddings daemon response")
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
}
