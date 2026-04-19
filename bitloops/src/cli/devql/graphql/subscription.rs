use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{Connector, connect_async, connect_async_tls_with_config};

use super::documents::{RUNTIME_EVENTS_SUBSCRIPTION, TASK_PROGRESS_SUBSCRIPTION};
use super::progress::{TASK_RENDER_TICK_INTERVAL, TaskProgressRenderer};
use super::types::{
    RuntimeEventGraphqlRecord, RuntimeEventsSubscriptionData, TaskGraphqlRecord,
    TaskProgressSubscriptionData,
};
use crate::daemon;
use crate::devql_transport::SlimCliRepoScope;

pub(super) async fn watch_task_via_subscription(
    scope: &SlimCliRepoScope,
    task_id: &str,
    renderer: &mut TaskProgressRenderer,
) -> Result<Option<TaskGraphqlRecord>> {
    let endpoint = devql_global_websocket_endpoint()?;
    let mut request = endpoint
        .as_str()
        .into_client_request()
        .context("building DevQL websocket subscription request")?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static("graphql-transport-ws"),
    );
    request.headers_mut().insert(
        crate::devql_transport::HEADER_SCOPE_REPO_ROOT,
        HeaderValue::from_str(&crate::devql_transport::encode_scope_header_value(
            &scope.repo_root.to_string_lossy(),
        ))
        .context("encoding DevQL websocket repo root header")?,
    );
    request.headers_mut().insert(
        crate::devql_transport::HEADER_DAEMON_BINDING,
        HeaderValue::from_str(&crate::devql_transport::repo_daemon_binding_identifier(
            scope.repo_root.as_path(),
        ))
        .context("encoding DevQL websocket daemon binding header")?,
    );

    let (mut websocket, _) = connect_devql_websocket(request, &endpoint)
        .await
        .context("connecting to Bitloops daemon websocket")?;
    websocket
        .send(Message::Text(
            json!({
                "type": "connection_init",
                "payload": {},
            })
            .to_string()
            .into(),
        ))
        .await
        .context("sending GraphQL websocket connection init")?;

    loop {
        let message = websocket
            .next()
            .await
            .transpose()
            .context("waiting for GraphQL websocket connection ack")?
            .context(
                "Bitloops daemon closed the websocket before acknowledging the subscription",
            )?;
        match message {
            Message::Text(payload) => {
                let envelope: serde_json::Value = serde_json::from_str(payload.as_str())
                    .context("decoding GraphQL websocket connection message")?;
                match envelope
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                {
                    "connection_ack" => break,
                    "ping" => {
                        websocket
                            .send(Message::Text(json!({ "type": "pong" }).to_string().into()))
                            .await
                            .context("sending GraphQL websocket pong")?;
                    }
                    "error" | "connection_error" => {
                        bail!(
                            "{}",
                            graphql_websocket_error_message(&envelope).unwrap_or_else(|| {
                                "Bitloops daemon rejected the websocket subscription".to_string()
                            })
                        );
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => {
                websocket
                    .send(Message::Pong(payload))
                    .await
                    .context("replying to websocket ping")?;
            }
            Message::Close(frame) => {
                let detail = frame
                    .as_ref()
                    .map(|frame| frame.reason.to_string())
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "no close reason".to_string());
                bail!(
                    "Bitloops daemon closed the websocket before acknowledging the subscription: {detail}"
                );
            }
            _ => {}
        }
    }

    websocket
        .send(Message::Text(
            json!({
                "id": "task-progress",
                "type": "subscribe",
                "payload": {
                    "query": TASK_PROGRESS_SUBSCRIPTION,
                    "variables": {
                        "taskId": task_id,
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .context("sending task progress subscription")?;

    let mut latest_task = None::<TaskGraphqlRecord>;
    let mut render_tick = tokio::time::interval(TASK_RENDER_TICK_INTERVAL);
    loop {
        tokio::select! {
            _ = render_tick.tick(), if renderer.is_interactive() => {
                if let Some(task) = latest_task.as_ref() {
                    renderer.tick(task)?;
                }
            }
            message = websocket.next() => {
                let Some(message) = message else {
                    return Ok(None);
                };
                let message = message.context("reading task progress subscription message")?;
                match message {
                    Message::Text(payload) => {
                        let envelope: serde_json::Value = serde_json::from_str(payload.as_str())
                            .context("decoding task progress subscription message")?;
                        match envelope
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                        {
                            "next" => {
                                let payload = envelope
                                    .get("payload")
                                    .cloned()
                                    .context("subscription event missing payload")?;
                                if let Some(errors) = payload.get("errors") {
                                    bail!("Bitloops daemon returned subscription errors: {errors}");
                                }
                                let data = payload
                                    .get("data")
                                    .cloned()
                                    .context("subscription event missing data")?;
                                let response: TaskProgressSubscriptionData =
                                    serde_json::from_value(data)
                                        .context("decoding task progress subscription data")?;
                                let task = response.task_progress.task;
                                latest_task = Some(task.clone());
                                renderer.render(&task)?;
                                match task.status.to_ascii_lowercase().as_str() {
                                    "completed" => return Ok(Some(task)),
                                    "failed" | "cancelled" => {
                                        if let Some(error) = task.error {
                                            bail!("task {task_id} failed: {error}");
                                        }
                                        bail!("task {task_id} ended with status {}", task.status);
                                    }
                                    _ => {}
                                }
                            }
                            "complete" => return Ok(None),
                            "ping" => {
                                websocket
                                    .send(Message::Text(json!({ "type": "pong" }).to_string().into()))
                                    .await
                                    .context("sending GraphQL websocket pong")?;
                            }
                            "error" => {
                                bail!(
                                    "{}",
                                    graphql_websocket_error_message(&envelope).unwrap_or_else(|| {
                                        "Bitloops daemon returned a websocket subscription error"
                                            .to_string()
                                    })
                                );
                            }
                            _ => {}
                        }
                    }
                    Message::Ping(payload) => {
                        websocket
                            .send(Message::Pong(payload))
                            .await
                            .context("replying to websocket ping")?;
                    }
                    Message::Close(frame) => {
                        let detail = frame
                            .as_ref()
                            .map(|frame| frame.reason.to_string())
                            .filter(|reason| !reason.is_empty())
                            .unwrap_or_else(|| "no close reason".to_string());
                        bail!("Bitloops daemon closed the websocket task subscription: {detail}");
                    }
                    _ => {}
                }
            }
        }
    }
}

pub(crate) async fn next_runtime_event_via_subscription(
    scope: &SlimCliRepoScope,
    repo_id: &str,
    init_session_id: Option<&str>,
) -> Result<RuntimeEventGraphqlRecord> {
    let endpoint = devql_runtime_websocket_endpoint()?;
    let mut request = endpoint
        .as_str()
        .into_client_request()
        .context("building runtime websocket subscription request")?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static("graphql-transport-ws"),
    );
    request.headers_mut().insert(
        crate::devql_transport::HEADER_SCOPE_REPO_ROOT,
        HeaderValue::from_str(&crate::devql_transport::encode_scope_header_value(
            &scope.repo_root.to_string_lossy(),
        ))
        .context("encoding runtime websocket repo root header")?,
    );
    request.headers_mut().insert(
        crate::devql_transport::HEADER_DAEMON_BINDING,
        HeaderValue::from_str(&crate::devql_transport::repo_daemon_binding_identifier(
            scope.repo_root.as_path(),
        ))
        .context("encoding runtime websocket daemon binding header")?,
    );

    let (mut websocket, _) = connect_devql_websocket(request, &endpoint)
        .await
        .context("connecting to Bitloops runtime websocket")?;
    websocket
        .send(Message::Text(
            json!({
                "type": "connection_init",
                "payload": {},
            })
            .to_string()
            .into(),
        ))
        .await
        .context("sending runtime GraphQL websocket connection init")?;

    loop {
        let message = websocket
            .next()
            .await
            .transpose()
            .context("waiting for runtime GraphQL websocket connection ack")?
            .context(
                "Bitloops daemon closed the runtime websocket before acknowledging the subscription",
            )?;
        match message {
            Message::Text(payload) => {
                let envelope: serde_json::Value = serde_json::from_str(payload.as_str())
                    .context("decoding runtime GraphQL websocket connection message")?;
                match envelope
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                {
                    "connection_ack" => break,
                    "ping" => {
                        websocket
                            .send(Message::Text(json!({ "type": "pong" }).to_string().into()))
                            .await
                            .context("sending runtime GraphQL websocket pong")?;
                    }
                    "error" | "connection_error" => {
                        bail!(
                            "{}",
                            graphql_websocket_error_message(&envelope).unwrap_or_else(|| {
                                "Bitloops daemon rejected the runtime websocket subscription"
                                    .to_string()
                            })
                        );
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => {
                websocket
                    .send(Message::Pong(payload))
                    .await
                    .context("replying to runtime websocket ping")?;
            }
            Message::Close(frame) => {
                let detail = frame
                    .as_ref()
                    .map(|frame| frame.reason.to_string())
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "no close reason".to_string());
                bail!(
                    "Bitloops daemon closed the runtime websocket before acknowledging the subscription: {detail}"
                );
            }
            _ => {}
        }
    }

    websocket
        .send(Message::Text(
            json!({
                "id": "runtime-events",
                "type": "subscribe",
                "payload": {
                    "query": RUNTIME_EVENTS_SUBSCRIPTION,
                    "variables": {
                        "repoId": repo_id,
                        "initSessionId": init_session_id,
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .context("sending runtime events subscription")?;

    loop {
        let Some(message) = websocket.next().await else {
            bail!("Bitloops daemon closed the runtime websocket subscription");
        };
        let message = message.context("reading runtime subscription message")?;
        match message {
            Message::Text(payload) => {
                let envelope: serde_json::Value = serde_json::from_str(payload.as_str())
                    .context("decoding runtime subscription message")?;
                match envelope
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                {
                    "next" => {
                        let payload = envelope
                            .get("payload")
                            .cloned()
                            .context("runtime subscription event missing payload")?;
                        if let Some(errors) = payload.get("errors") {
                            bail!("Bitloops daemon returned runtime subscription errors: {errors}");
                        }
                        let data = payload
                            .get("data")
                            .cloned()
                            .context("runtime subscription event missing data")?;
                        let response: RuntimeEventsSubscriptionData = serde_json::from_value(data)
                            .context("decoding runtime subscription data")?;
                        return Ok(response.runtime_events);
                    }
                    "ping" => {
                        websocket
                            .send(Message::Text(json!({ "type": "pong" }).to_string().into()))
                            .await
                            .context("sending runtime GraphQL websocket pong")?;
                    }
                    "complete" => {
                        bail!("Bitloops daemon completed the runtime subscription unexpectedly");
                    }
                    "error" => {
                        bail!(
                            "{}",
                            graphql_websocket_error_message(&envelope).unwrap_or_else(|| {
                                "Bitloops daemon returned a runtime websocket subscription error"
                                    .to_string()
                            })
                        );
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => {
                websocket
                    .send(Message::Pong(payload))
                    .await
                    .context("replying to runtime websocket ping")?;
            }
            Message::Close(frame) => {
                let detail = frame
                    .as_ref()
                    .map(|frame| frame.reason.to_string())
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "no close reason".to_string());
                bail!("Bitloops daemon closed the runtime websocket subscription: {detail}");
            }
            _ => {}
        }
    }
}

fn devql_global_websocket_endpoint() -> Result<String> {
    let runtime_url = daemon::daemon_url()?.context(
        "Bitloops daemon is not running for this repository. Start it with `bitloops daemon start`.",
    )?;
    let base = runtime_url.trim_end_matches('/');
    if let Some(rest) = base.strip_prefix("https://") {
        return Ok(format!("wss://{rest}/devql/global"));
    }
    if let Some(rest) = base.strip_prefix("http://") {
        return Ok(format!("ws://{rest}/devql/global"));
    }
    bail!("unsupported Bitloops daemon url `{runtime_url}`");
}

fn devql_runtime_websocket_endpoint() -> Result<String> {
    let runtime_url = daemon::daemon_url()?.context(
        "Bitloops daemon is not running for this repository. Start it with `bitloops daemon start`.",
    )?;
    let base = runtime_url.trim_end_matches('/');
    if let Some(rest) = base.strip_prefix("https://") {
        return Ok(format!("wss://{rest}/devql/runtime/ws"));
    }
    if let Some(rest) = base.strip_prefix("http://") {
        return Ok(format!("ws://{rest}/devql/runtime/ws"));
    }
    bail!("unsupported Bitloops daemon url `{runtime_url}`");
}

fn graphql_websocket_error_message(envelope: &serde_json::Value) -> Option<String> {
    if let Some(message) = envelope.get("message").and_then(serde_json::Value::as_str) {
        return Some(message.to_string());
    }
    envelope
        .get("payload")
        .and_then(|payload| payload.get("message").or_else(|| payload.get("errors")))
        .map(|value| value.to_string())
}

async fn connect_devql_websocket(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    endpoint: &str,
) -> Result<(
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::handshake::client::Response,
)> {
    if should_accept_invalid_daemon_websocket_certs(endpoint) {
        let connector = Connector::Rustls(insecure_loopback_websocket_tls_config()?);
        return connect_async_tls_with_config(request, None, false, Some(connector))
            .await
            .map_err(Into::into);
    }

    connect_async(request).await.map_err(Into::into)
}

pub(super) fn should_accept_invalid_daemon_websocket_certs(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "wss" {
        return false;
    }
    matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
    )
}

fn insecure_loopback_websocket_tls_config() -> Result<Arc<rustls::ClientConfig>> {
    static CONFIG: OnceLock<std::result::Result<Arc<rustls::ClientConfig>, String>> =
        OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let provider = crate::api::tls::rustls_crypto_provider().map_err(|err| err.to_string())?;
        let verifier = Arc::new(LoopbackCertVerifier::new(
            provider.signature_verification_algorithms,
        ));
        Ok(Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("safe default TLS versions are valid")
                .dangerous()
                .with_custom_certificate_verifier(verifier)
                .with_no_client_auth(),
        ))
    });

    config
        .as_ref()
        .map(Arc::clone)
        .map_err(|message| anyhow::anyhow!(message.clone()))
}

#[derive(Debug)]
struct LoopbackCertVerifier {
    supported_algorithms: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl LoopbackCertVerifier {
    fn new(supported_algorithms: rustls::crypto::WebPkiSupportedAlgorithms) -> Self {
        Self {
            supported_algorithms,
        }
    }
}

impl ServerCertVerifier for LoopbackCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let hostname = match server_name {
            ServerName::DnsName(dns) => dns.as_ref(),
            ServerName::IpAddress(_) => return Ok(ServerCertVerified::assertion()),
            _ => return Err(rustls::Error::General("unsupported server name".into())),
        };

        match hostname {
            "localhost" | "127.0.0.1" | "::1" => Ok(ServerCertVerified::assertion()),
            _ => Err(rustls::Error::General(format!(
                "refusing insecure websocket TLS for non-loopback host `{hostname}`"
            ))),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.supported_algorithms.supported_schemes()
    }
}
