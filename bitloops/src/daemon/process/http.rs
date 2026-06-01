use super::*;

pub(in crate::daemon) async fn daemon_http_ready(state: &DaemonRuntimeState) -> bool {
    let client = match daemon_http_client(&state.url) {
        Ok(client) => client,
        Err(_) => return false,
    };
    let url = format!("{}/devql/sdl", state.url.trim_end_matches('/'));
    client
        .get(url)
        .timeout(DAEMON_READY_REQUEST_TIMEOUT)
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

pub(in crate::daemon) fn daemon_http_client(url: &str) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if should_accept_invalid_daemon_certs(url) {
        builder = builder.danger_accept_invalid_certs(true);
    }
    builder
        .connect_timeout(DAEMON_HTTP_CONNECT_TIMEOUT)
        .timeout(DAEMON_HTTP_REQUEST_TIMEOUT)
        .build()
        .context("building Bitloops daemon HTTP client")
}

pub(in crate::daemon) async fn query_health(
    state: &DaemonRuntimeState,
) -> Result<DaemonHealthSummary> {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct HealthEnvelope {
        health: HealthPayload,
    }

    #[derive(Debug, Deserialize)]
    struct HealthPayload {
        relational: Option<HealthBackend>,
        events: Option<HealthBackend>,
        blob: Option<HealthBackend>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct HealthBackend {
        backend: Option<String>,
        connected: Option<bool>,
    }

    let payload: HealthEnvelope = execute_graphql(
        &state.config_root,
        r#"{ health { relational { backend connected } events { backend connected } blob { backend connected } } }"#,
        json!({}),
    )
    .await?;

    Ok(DaemonHealthSummary {
        relational_backend: payload
            .health
            .relational
            .as_ref()
            .and_then(|value| value.backend.clone()),
        relational_connected: payload.health.relational.and_then(|value| value.connected),
        events_backend: payload
            .health
            .events
            .as_ref()
            .and_then(|value| value.backend.clone()),
        events_connected: payload.health.events.and_then(|value| value.connected),
        blob_backend: payload
            .health
            .blob
            .as_ref()
            .and_then(|value| value.backend.clone()),
        blob_connected: payload.health.blob.and_then(|value| value.connected),
    })
}

pub(super) fn should_accept_invalid_daemon_certs(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }

    matches!(
        parsed.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1") | Some("[::1]")
    )
}
