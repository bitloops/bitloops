use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use bitloops_embeddings_protocol::{
    DescribeRequest, DescribeResponse, EmbedBatchRequest, EmbedBatchResponse, EmbeddingInput,
    ErrorResponse, ProviderDescriptor, Request, Response, ShutdownRequest,
};

const DEFAULT_EMBEDDINGS_RUNTIME_COMMAND: &str = "bitloops-embeddings";
const INTERNAL_EMBEDDINGS_RUNTIME_SUBCOMMAND: &str = "__embeddings-runtime";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingInputType {
    Document,
    Query,
}

impl EmbeddingInputType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Query => "query",
        }
    }
}

impl From<EmbeddingInputType> for bitloops_embeddings_protocol::EmbeddingInputType {
    fn from(value: EmbeddingInputType) -> Self {
        match value {
            EmbeddingInputType::Document => Self::Document,
            EmbeddingInputType::Query => Self::Query,
        }
    }
}

pub trait EmbeddingProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn output_dimension(&self) -> Option<usize>;
    fn cache_key(&self) -> String;
    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>>;
}

#[derive(Debug, Clone)]
pub struct EmbeddingRuntimeClientConfig {
    pub command: String,
    pub args: Vec<String>,
    pub startup_timeout_secs: u64,
    pub request_timeout_secs: u64,
    pub config_path: PathBuf,
    pub profile_name: String,
    pub repo_root: Option<PathBuf>,
}

pub fn build_embedding_provider(
    config: &EmbeddingRuntimeClientConfig,
) -> Result<Box<dyn EmbeddingProvider>> {
    let session = RuntimeSession::spawn(config)?;
    let describe = session.describe(Duration::from_secs(config.startup_timeout_secs))?;
    Ok(Box::new(RuntimeEmbeddingProvider {
        request_timeout: Duration::from_secs(config.request_timeout_secs),
        session,
        runtime: describe.runtime,
    }))
}

#[derive(Debug)]
struct RuntimeEmbeddingProvider {
    request_timeout: Duration,
    session: RuntimeSession,
    runtime: bitloops_embeddings_protocol::RuntimeDescriptor,
}

impl EmbeddingProvider for RuntimeEmbeddingProvider {
    fn provider_name(&self) -> &str {
        &self.runtime.provider.provider_name
    }

    fn model_name(&self) -> &str {
        &self.runtime.provider.model_name
    }

    fn output_dimension(&self) -> Option<usize> {
        self.runtime.provider.output_dimension
    }

    fn cache_key(&self) -> String {
        cache_key_for_runtime(&self.runtime.provider, &self.runtime.profile_name)
    }

    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>> {
        let input = input.trim();
        if input.is_empty() {
            bail!("embedding input cannot be empty");
        }

        let request_id = next_request_id();
        let response = self.session.request(
            Request::EmbedBatch(EmbedBatchRequest {
                request_id,
                inputs: vec![EmbeddingInput {
                    id: None,
                    text: input.to_string(),
                    input_type: input_type.into(),
                }],
            }),
            self.request_timeout,
        )?;
        let Response::EmbedBatch(EmbedBatchResponse { vectors, .. }) = response else {
            bail!("embedding runtime returned an unexpected response type");
        };
        let vector = vectors
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("embedding runtime returned no vectors"))?;
        if vector.values.is_empty() {
            bail!("embedding runtime returned an empty vector");
        }
        Ok(vector.values)
    }
}

#[derive(Debug)]
struct RuntimeSession {
    sender: mpsc::Sender<WorkerCommand>,
}

impl RuntimeSession {
    fn spawn(config: &EmbeddingRuntimeClientConfig) -> Result<Self> {
        let invocation = resolve_runtime_invocation(
            config.command.as_str(),
            std::env::current_exe().ok().as_deref(),
        );
        let mut command = Command::new(&invocation.program);
        command.args(&invocation.prefix_args);
        command.args(&config.args);
        command.arg("--config").arg(&config.config_path);
        command.arg("--profile").arg(&config.profile_name);
        if let Some(repo_root) = config.repo_root.as_ref() {
            command.arg("--repo-root").arg(repo_root);
        }
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::inherit());

        let mut child = command.spawn().with_context(|| {
            format!(
                "spawning embeddings runtime `{}` (resolved to `{}`) for profile `{}`",
                config.command,
                invocation.program.display(),
                config.profile_name
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .context("capturing embeddings runtime stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("capturing embeddings runtime stdout")?;

        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || runtime_worker_loop(child, stdin, stdout, receiver));

        Ok(Self { sender })
    }

    fn describe(&self, timeout: Duration) -> Result<DescribeResponse> {
        let response = self.request(
            Request::Describe(DescribeRequest {
                request_id: next_request_id(),
            }),
            timeout,
        )?;
        let Response::Describe(describe) = response else {
            bail!("embedding runtime returned an unexpected response to describe");
        };
        Ok(describe)
    }

    fn request(&self, request: Request, timeout: Duration) -> Result<Response> {
        let (response_tx, response_rx) = mpsc::channel();
        self.sender
            .send(WorkerCommand {
                request,
                response_tx,
            })
            .map_err(|_| anyhow!("embedding runtime worker is not available"))?;
        response_rx
            .recv_timeout(timeout)
            .map_err(|_| anyhow!("embedding runtime request timed out after {:?}", timeout))?
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeInvocation {
    program: PathBuf,
    prefix_args: Vec<String>,
}

fn resolve_runtime_invocation(
    configured_command: &str,
    current_exe: Option<&std::path::Path>,
) -> RuntimeInvocation {
    if configured_command == DEFAULT_EMBEDDINGS_RUNTIME_COMMAND
        && let Some(current_exe) = current_exe
    {
        return RuntimeInvocation {
            program: current_exe.to_path_buf(),
            prefix_args: vec![INTERNAL_EMBEDDINGS_RUNTIME_SUBCOMMAND.to_string()],
        };
    }

    RuntimeInvocation {
        program: PathBuf::from(configured_command),
        prefix_args: Vec::new(),
    }
}

#[derive(Debug)]
struct WorkerCommand {
    request: Request,
    response_tx: mpsc::Sender<Result<Response>>,
}

fn runtime_worker_loop(
    mut child: Child,
    mut stdin: ChildStdin,
    stdout: ChildStdout,
    receiver: mpsc::Receiver<WorkerCommand>,
) {
    let mut reader = BufReader::new(stdout);
    while let Ok(command) = receiver.recv() {
        let response = handle_worker_request(&mut stdin, &mut reader, &command.request);
        let _ = command.response_tx.send(response);
    }

    let _ = write_request(
        &mut stdin,
        &Request::Shutdown(ShutdownRequest {
            request_id: next_request_id(),
        }),
    );
    let _ = child.kill();
    let _ = child.wait();
}

fn handle_worker_request(
    stdin: &mut ChildStdin,
    reader: &mut BufReader<ChildStdout>,
    request: &Request,
) -> Result<Response> {
    write_request(stdin, request)?;
    let response = read_response(reader)?;
    match &response {
        Response::Error(ErrorResponse { message, .. }) => bail!("{message}"),
        _ => Ok(response),
    }
}

fn write_request(stdin: &mut ChildStdin, request: &Request) -> Result<()> {
    let line = serde_json::to_string(request).context("serializing embeddings runtime request")?;
    writeln!(stdin, "{line}").context("writing embeddings runtime request")?;
    stdin.flush().context("flushing embeddings runtime request")
}

fn read_response(reader: &mut BufReader<ChildStdout>) -> Result<Response> {
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .context("reading embeddings runtime response")?;
    if bytes == 0 {
        bail!("embeddings runtime exited before replying");
    }
    serde_json::from_str(line.trim_end()).context("parsing embeddings runtime response")
}

fn cache_key_for_runtime(provider: &ProviderDescriptor, profile_name: &str) -> String {
    match provider.output_dimension {
        Some(output_dimension) => format!(
            "runtime_profile={profile_name}::provider={}::model={}::dimension={output_dimension}",
            provider.provider_name, provider.model_name
        ),
        None => format!(
            "runtime_profile={profile_name}::provider={}::model={}",
            provider.provider_name, provider.model_name
        ),
    }
}

fn next_request_id() -> String {
    static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
    format!(
        "embeddings-{}",
        NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_input_type_maps_to_protocol_shape() {
        assert_eq!(
            bitloops_embeddings_protocol::EmbeddingInputType::from(EmbeddingInputType::Document)
                .as_str(),
            "document"
        );
        assert_eq!(
            bitloops_embeddings_protocol::EmbeddingInputType::from(EmbeddingInputType::Query)
                .as_str(),
            "query"
        );
    }

    #[test]
    fn cache_key_includes_profile_provider_and_model() {
        let key = cache_key_for_runtime(
            &ProviderDescriptor {
                kind: "openai".to_string(),
                provider_name: "openai".to_string(),
                model_name: "text-embedding-3-large".to_string(),
                output_dimension: Some(3072),
                cache_dir: None,
            },
            "prod",
        );

        assert!(key.contains("runtime_profile=prod"));
        assert!(key.contains("provider=openai"));
        assert!(key.contains("model=text-embedding-3-large"));
        assert!(key.contains("dimension=3072"));
    }

    #[test]
    fn default_runtime_command_uses_current_executable() {
        let invocation = resolve_runtime_invocation(
            DEFAULT_EMBEDDINGS_RUNTIME_COMMAND,
            Some(std::path::Path::new("/tmp/bitloops")),
        );

        assert_eq!(invocation.program, PathBuf::from("/tmp/bitloops"));
        assert_eq!(
            invocation.prefix_args,
            vec![INTERNAL_EMBEDDINGS_RUNTIME_SUBCOMMAND.to_string()]
        );
    }

    #[test]
    fn explicit_runtime_command_is_left_unchanged() {
        let invocation = resolve_runtime_invocation("/usr/local/bin/custom-embeddings", None);

        assert_eq!(
            invocation.program,
            PathBuf::from("/usr/local/bin/custom-embeddings")
        );
        assert!(invocation.prefix_args.is_empty());
    }
}
