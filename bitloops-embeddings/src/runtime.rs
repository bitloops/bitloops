use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use bitloops_embeddings_protocol::{
    DescribeResponse, EmbedBatchRequest, EmbedBatchResponse, ErrorResponse, PROTOCOL_VERSION,
    Request, Response, RuntimeDescriptor, RUNTIME_NAME, ShutdownResponse,
};

use crate::config::{load_runtime_file_config, select_profile};
use crate::providers::{build_batch_vectors, build_provider, describe_provider};

pub struct RuntimeOptions {
    pub config_path: PathBuf,
    pub selected_profile: String,
    pub repo_root: Option<PathBuf>,
}

pub struct RuntimeState {
    profile_name: String,
    provider: Box<dyn crate::providers::EmbeddingRuntimeProvider>,
}

impl std::fmt::Debug for RuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeState")
            .field("profile_name", &self.profile_name)
            .finish_non_exhaustive()
    }
}

pub fn run_stdio_runtime(options: &RuntimeOptions) -> Result<()> {
    let state = RuntimeState::load(options)?;
    run_stdio_loop(state)
}

impl RuntimeState {
    pub fn load(options: &RuntimeOptions) -> Result<Self> {
        let config = load_runtime_file_config(&options.config_path)
            .with_context(|| format!("loading runtime config {}", options.config_path.display()))?;
        let profile = select_profile(&config, &options.selected_profile)?;
        let provider = build_provider(profile, options.repo_root.as_deref())
            .with_context(|| format!("building embedding profile `{}`", options.selected_profile))?;
        Ok(Self {
            profile_name: options.selected_profile.clone(),
            provider,
        })
    }

    fn describe(&self, request_id: &str) -> Response {
        Response::Describe(DescribeResponse {
            request_id: request_id.to_string(),
            protocol_version: PROTOCOL_VERSION,
            runtime: RuntimeDescriptor {
                protocol_version: PROTOCOL_VERSION,
                runtime_name: RUNTIME_NAME.to_string(),
                runtime_version: env!("CARGO_PKG_VERSION").to_string(),
                profile_name: self.profile_name.clone(),
                provider: describe_provider(self.provider.as_ref()),
            },
        })
    }

    fn embed_batch(&self, request: &EmbedBatchRequest) -> Result<Response> {
        let vectors = build_batch_vectors(self.provider.as_ref(), &request.inputs)?;
        let vectors = vectors
            .into_iter()
            .enumerate()
            .map(|(index, values)| bitloops_embeddings_protocol::EmbeddingVector {
                index,
                id: request.inputs.get(index).and_then(|input| input.id.clone()),
                values,
            })
            .collect::<Vec<_>>();

        Ok(Response::EmbedBatch(EmbedBatchResponse {
            request_id: request.request_id.clone(),
            protocol_version: PROTOCOL_VERSION,
            vectors,
        }))
    }

    fn shutdown(&self, request_id: &str) -> Response {
        Response::Shutdown(ShutdownResponse {
            request_id: request_id.to_string(),
            protocol_version: PROTOCOL_VERSION,
            accepted: true,
        })
    }
}

pub fn run_stdio_loop(state: RuntimeState) -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());

    for line in stdin.lock().lines() {
        let line = line.context("reading runtime request line")?;
        if line.trim().is_empty() {
            continue;
        }

        let mut should_shutdown = false;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(Request::Describe(request)) => state.describe(&request.request_id),
            Ok(Request::EmbedBatch(request)) => match state.embed_batch(&request) {
                Ok(response) => response,
                Err(err) => error_response(Some(&request.request_id), err),
            },
            Ok(Request::Shutdown(request)) => {
                should_shutdown = true;
                state.shutdown(&request.request_id)
            }
            Err(err) => error_response(None, err.into()),
        };

        write_response(&mut stdout, &response)?;
        if should_shutdown {
            break;
        }
    }

    Ok(())
}

fn write_response<W: Write>(writer: &mut W, response: &Response) -> Result<()> {
    let line = serde_json::to_string(response).context("serializing runtime response")?;
    writeln!(writer, "{line}").context("writing runtime response")?;
    writer.flush().context("flushing runtime response")
}

fn error_response(request_id: Option<&str>, err: anyhow::Error) -> Response {
    Response::Error(ErrorResponse {
        request_id: request_id.map(str::to_string),
        code: "runtime_error".to_string(),
        message: err.to_string(),
        details: Some(Value::String(format!("{err:#}"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EmbeddingProfileConfig, RuntimeFileConfig};
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn runtime_state_rejects_missing_profile() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        fs::write(&config_path, "[embeddings.profiles.local]\nkind = \"local_fastembed\"\n")
            .expect("write config");
        let options = RuntimeOptions {
            config_path,
            selected_profile: "missing".to_string(),
            repo_root: Some(temp.path().to_path_buf()),
        };

        let err = RuntimeState::load(&options).expect_err("missing profile should fail");
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn runtime_state_shows_describe_descriptor() {
        let temp = tempdir().expect("tempdir");
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "openai".to_string(),
            EmbeddingProfileConfig::OpenAi {
                model: "text-embedding-3-large".to_string(),
                api_key: "secret".to_string(),
                base_url: None,
            },
        );
        let config = RuntimeFileConfig {
            embeddings: crate::config::EmbeddingsSectionConfig { profiles },
        };
        let config_path = temp.path().join("config.toml");
        fs::write(
            &config_path,
            toml::to_string(&config).expect("serialize config"),
        )
        .expect("write config");
        let options = RuntimeOptions {
            config_path,
            selected_profile: "openai".to_string(),
            repo_root: Some(temp.path().to_path_buf()),
        };

        let state = RuntimeState::load(&options).expect("load runtime state");
        let response = state.describe("req-1");
        match response {
            Response::Describe(describe) => {
                assert_eq!(describe.request_id, "req-1");
                assert_eq!(describe.runtime.profile_name, "openai");
                assert_eq!(describe.runtime.runtime_name, RUNTIME_NAME);
            }
            _ => panic!("expected describe response"),
        }
    }
}
