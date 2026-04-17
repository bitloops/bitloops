use super::*;

pub(super) async fn execute_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    query: &str,
    variables: Value,
) -> Result<T> {
    execute_graphql_request(repo_root, "/devql/global", None, query, variables, false).await
}

pub(super) async fn execute_repo_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    query: &str,
    variables: Value,
) -> Result<T> {
    execute_graphql_request(repo_root, "/devql/global", None, query, variables, true).await
}

pub(super) async fn execute_runtime_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    query: &str,
    variables: Value,
) -> Result<T> {
    execute_graphql_request(repo_root, "/devql/runtime", None, query, variables, true).await
}

pub(super) async fn execute_slim_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    scope: &SlimCliRepoScope,
    query: &str,
    variables: Value,
) -> Result<T> {
    execute_graphql_request(repo_root, "/devql", Some(scope), query, variables, true).await
}

async fn execute_graphql_request<T: DeserializeOwned>(
    repo_root: &Path,
    endpoint_path: &str,
    scope: Option<&SlimCliRepoScope>,
    query: &str,
    variables: Value,
    require_binding: bool,
) -> Result<T> {
    let timings_enabled = crate::devql_timing::timings_enabled_from_env();
    let trace = timings_enabled.then(crate::devql_timing::TimingTrace::new);

    let runtime_started = Instant::now();
    let runtime = read_runtime_state(repo_root)?.context(
        "Bitloops daemon is not running for this repository. Start it with `bitloops daemon start`.",
    )?;
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.read_runtime_state",
            runtime_started.elapsed(),
            json!({
                "url": runtime.url,
            }),
        );
    }

    let client_started = Instant::now();
    let client = daemon_http_client(&runtime.url)?;
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.build_http_client",
            client_started.elapsed(),
            Value::Null,
        );
    }

    let endpoint = format!("{}{}", runtime.url.trim_end_matches('/'), endpoint_path);
    let send_started = Instant::now();
    let mut request = client.post(endpoint).json(&json!({
        "query": query,
        "variables": variables,
    }));
    if require_binding {
        request = crate::devql_transport::attach_repo_daemon_binding_headers(request, repo_root)?;
    }
    if let Some(scope) = scope {
        request = attach_slim_cli_scope_headers(request, scope);
    }
    if timings_enabled {
        request = request.header(
            crate::devql_timing::DEVQL_TIMINGS_HEADER,
            crate::devql_timing::timing_header_value(),
        );
    }
    let response = request
        .send()
        .await
        .context("sending DevQL request to Bitloops daemon")?;
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.http_post",
            send_started.elapsed(),
            json!({
                "status": response.status().as_u16(),
            }),
        );
    }

    if response.status() != ReqwestStatusCode::OK {
        emit_query_timing_debug(trace.as_ref(), None);
        bail!("Bitloops daemon returned HTTP {}", response.status());
    }

    let decode_started = Instant::now();
    let payload: GraphqlEnvelope = response
        .json()
        .await
        .context("decoding DevQL response from Bitloops daemon")?;
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.decode_response_json",
            decode_started.elapsed(),
            Value::Null,
        );
    }

    let server_timings = payload
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get(crate::devql_timing::DEVQL_TIMINGS_EXTENSION))
        .cloned();

    if let Some(errors) = payload.errors
        && let Some(error) = errors.first()
    {
        emit_query_timing_debug(trace.as_ref(), server_timings.as_ref());
        bail!("{}", error.message);
    }

    let Some(data) = payload.data else {
        emit_query_timing_debug(trace.as_ref(), server_timings.as_ref());
        bail!("Bitloops daemon returned no GraphQL data payload");
    };

    let decode_graphql_started = Instant::now();
    let decoded = serde_json::from_value(data).context("decoding GraphQL data payload for CLI");
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.decode_graphql_data",
            decode_graphql_started.elapsed(),
            Value::Null,
        );
    }
    emit_query_timing_debug(trace.as_ref(), server_timings.as_ref());
    decoded
}

pub(super) fn choose_dashboard_launch_mode() -> Result<Option<DaemonMode>> {
    use std::io::{self, IsTerminal, Write};

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return Ok(None);
    }

    let mut stdout = io::stdout();
    writeln!(
        stdout,
        "Bitloops daemon is not running. Start it in foreground [f], detached [d], always-on [a], or cancel [c]?"
    )?;
    write!(stdout, "> ")?;
    stdout.flush()?;

    let mut input = String::new();
    stdin
        .read_line(&mut input)
        .context("reading dashboard daemon launch choice")?;
    let choice = match input.trim().to_ascii_lowercase().as_str() {
        "f" | "foreground" => Some(DaemonMode::Foreground),
        "d" | "detached" => Some(DaemonMode::Detached),
        "a" | "always-on" | "always_on" | "service" => Some(DaemonMode::Service),
        "c" | "cancel" | "" => None,
        other => bail!("unsupported dashboard launch choice `{other}`"),
    };
    Ok(choice)
}

pub(super) fn daemon_url() -> Result<Option<String>> {
    Ok(read_runtime_state(Path::new("."))?.map(|state| state.url))
}

#[derive(Debug, Deserialize)]
struct GraphqlEnvelope {
    data: Option<Value>,
    extensions: Option<serde_json::Map<String, Value>>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
}

fn emit_query_timing_debug(
    trace: Option<&crate::devql_timing::TimingTrace>,
    server_timings: Option<&Value>,
) {
    if let Some(server_timings) = server_timings {
        crate::devql_timing::print_summary("server", server_timings);
    }
    if let Some(trace) = trace {
        crate::devql_timing::print_summary("client", &trace.summary_value());
    }
}
