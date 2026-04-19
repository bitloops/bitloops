use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::auth::{
    ensure_platform_runtime_auth_environment_available, platform_runtime_auth_environment,
};
use super::runtime::{
    HF_HUB_OFFLINE_ENV, TRANSFORMERS_OFFLINE_ENV, cache_contains_requested_model,
    embeddings_runtime_request_timeout_secs, resolve_effective_embeddings_cache_dir,
};

const PYTHON_EMBEDDINGS_DIMENSION_PROBE_TEXT: &str = "bitloops python embedding dimension probe";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PythonEmbeddingsSessionConfig {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) startup_timeout_secs: u64,
    pub(crate) request_timeout_secs: u64,
    pub(crate) model: String,
    pub(crate) cache_dir: Option<PathBuf>,
    pub(crate) platform_backed: bool,
    pub(crate) launch_artifact_fingerprint: String,
    pub(crate) process_environment_fingerprint: String,
}

pub(crate) struct PythonEmbeddingsSession {
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

impl PythonEmbeddingsSession {
    pub(crate) fn start(config: &PythonEmbeddingsSessionConfig) -> Result<Self> {
        ensure_platform_runtime_auth_environment_available(config)?;
        let effective_cache_dir =
            resolve_effective_embeddings_cache_dir(config.cache_dir.as_deref());
        let mut command = Command::new(&config.command);
        command.args(&config.args);
        command.arg("daemon");
        command.arg("--model").arg(&config.model);
        command.envs(platform_runtime_auth_environment(config));
        if let Some(cache_dir) = effective_cache_dir.as_ref() {
            command.arg("--cache-dir").arg(cache_dir);
        }
        if effective_cache_dir
            .as_deref()
            .is_some_and(|cache_dir| cache_contains_requested_model(cache_dir, &config.model))
        {
            command.env(HF_HUB_OFFLINE_ENV, "1");
            command.env(TRANSFORMERS_OFFLINE_ENV, "1");
        }
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::inherit());

        let mut child = command.spawn().with_context(|| {
            format!(
                "spawning standalone `bitloops-local-embeddings` runtime `{}` for model `{}`",
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

    pub(crate) fn probe_dimension(&mut self) -> Result<usize> {
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

    pub(crate) fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let request_id = next_request_id();
        let request = json!({
            "id": request_id,
            "cmd": "embed",
            "model": self.config.model,
            "texts": texts,
        });
        self.write_json_line(&request)?;
        let value = self.read_json_response(
            embeddings_runtime_request_timeout_secs(self.config.request_timeout_secs),
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
        match self.child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(_) => {}
        }
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
