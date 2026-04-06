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

use super::documents::SYNC_PROGRESS_SUBSCRIPTION;
use super::progress::{SYNC_RENDER_TICK_INTERVAL, SyncProgressRenderer};
use super::types::{SyncProgressSubscriptionData, SyncTaskGraphqlRecord};
use crate::daemon;
use crate::host::devql::SyncSummary;

pub(super) async fn watch_sync_task_via_subscription(
    task_id: &str,
    renderer: &mut SyncProgressRenderer,
) -> Result<Option<SyncSummary>> {
    let endpoint = devql_global_websocket_endpoint()?;
    let mut request = endpoint
        .as_str()
        .into_client_request()
        .context("building DevQL websocket subscription request")?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static("graphql-transport-ws"),
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
                "id": "sync-progress",
                "type": "subscribe",
                "payload": {
                    "query": SYNC_PROGRESS_SUBSCRIPTION,
                    "variables": {
                        "taskId": task_id,
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .context("sending sync progress subscription")?;

    let mut latest_task = None::<SyncTaskGraphqlRecord>;
    let mut render_tick = tokio::time::interval(SYNC_RENDER_TICK_INTERVAL);
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
                let message = message.context("reading sync progress subscription message")?;
                match message {
                    Message::Text(payload) => {
                        let envelope: serde_json::Value = serde_json::from_str(payload.as_str())
                            .context("decoding sync progress subscription message")?;
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
                                let response: SyncProgressSubscriptionData =
                                    serde_json::from_value(data)
                                        .context("decoding sync progress subscription data")?;
                                let task = response.sync_progress;
                                latest_task = Some(task.clone());
                                renderer.render(&task)?;
                                match task.status.as_str() {
                                    "completed" => return Ok(task.summary.map(Into::into)),
                                    "failed" | "cancelled" => {
                                        if let Some(error) = task.error {
                                            bail!("sync task {task_id} failed: {error}");
                                        }
                                        bail!("sync task {task_id} ended with status {}", task.status);
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
                        bail!("Bitloops daemon closed the websocket sync subscription: {detail}");
                    }
                    _ => {}
                }
            }
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
        Some("localhost") | Some("127.0.0.1") | Some("::1") | Some("[::1]")
    )
}

fn insecure_loopback_websocket_tls_config() -> Result<Arc<rustls::ClientConfig>> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        ensure_rustls_crypto_provider()
            .map_err(|err| err.to_string())
            .map(|_| {
                Arc::new(
                    rustls::ClientConfig::builder_with_provider(Arc::new(
                        rustls::crypto::aws_lc_rs::default_provider(),
                    ))
                    .with_safe_default_protocol_versions()
                    .expect("safe default TLS versions are valid")
                    .dangerous()
                    .with_custom_certificate_verifier(SkipLoopbackServerVerification::new())
                    .with_no_client_auth(),
                )
            })
    });

    config
        .as_ref()
        .map(Arc::clone)
        .map_err(|message| anyhow::anyhow!(message.clone()))
}

fn ensure_rustls_crypto_provider() -> Result<()> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    let init = INIT.get_or_init(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            return rustls::crypto::aws_lc_rs::default_provider()
                .install_default()
                .map_err(|err| format!("install rustls aws_lc_rs crypto provider: {err:?}"));
        }
        Ok(())
    });
    init.as_ref()
        .map(|_| ())
        .map_err(|message| anyhow::anyhow!(message.clone()))
}

#[derive(Debug)]
struct SkipLoopbackServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipLoopbackServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(
            Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        ))
    }
}

impl ServerCertVerifier for SkipLoopbackServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphql_websocket_error_message_prefers_top_level_message() {
        let envelope = serde_json::json!({
            "message": "top-level message",
            "payload": {
                "message": "payload message"
            }
        });
        assert_eq!(
            graphql_websocket_error_message(&envelope).as_deref(),
            Some("top-level message")
        );
    }

    #[test]
    fn graphql_websocket_error_message_falls_back_to_payload_fields() {
        let payload_message = serde_json::json!({
            "payload": {
                "message": "payload message"
            }
        });
        assert_eq!(
            graphql_websocket_error_message(&payload_message).as_deref(),
            Some("\"payload message\"")
        );

        let payload_errors = serde_json::json!({
            "payload": {
                "errors": [{"message": "boom"}]
            }
        });
        assert_eq!(
            graphql_websocket_error_message(&payload_errors).as_deref(),
            Some(r#"[{"message":"boom"}]"#)
        );

        assert!(graphql_websocket_error_message(&serde_json::json!({})).is_none());
    }

    #[test]
    fn insecure_loopback_websocket_tls_config_is_cached() {
        let first = insecure_loopback_websocket_tls_config().expect("first tls config");
        let second = insecure_loopback_websocket_tls_config().expect("second tls config");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn ensure_rustls_crypto_provider_is_idempotent() {
        ensure_rustls_crypto_provider().expect("first install check");
        ensure_rustls_crypto_provider().expect("second install check");
    }
}
