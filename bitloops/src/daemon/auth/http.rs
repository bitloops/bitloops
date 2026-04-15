use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client as BlockingClient, Response as BlockingResponse};
use reqwest::{Client, Response};

use super::types::{
    DEFAULT_DEVICE_POLL_INTERVAL_SECS, WORKOS_DEVICE_GRANT_TYPE, WORKOS_HTTP_TIMEOUT_SECS,
    WORKOS_REFRESH_GRANT_TYPE, WorkosAuthSettings, WorkosDeviceAuthorizationResponse,
    WorkosDeviceLoginStart, WorkosOAuthError,
};

pub(super) async fn start_device_login_with_client(
    client: &Client,
    config: &WorkosAuthSettings,
) -> Result<WorkosDeviceLoginStart> {
    let response = client
        .post(format!(
            "{}/user_management/authorize/device",
            config.base_url.trim_end_matches('/')
        ))
        .json(&serde_json::json!({ "client_id": config.client_id }))
        .send()
        .await
        .context("requesting WorkOS device authorisation")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!(
            "WorkOS device authorisation failed (HTTP {status}){}",
            format_body_suffix(&body)
        );
    }

    let payload = response
        .json::<WorkosDeviceAuthorizationResponse>()
        .await
        .context("parsing WorkOS device authorisation response")?;

    Ok(WorkosDeviceLoginStart {
        verification_url: payload.verification_uri,
        verification_url_complete: payload.verification_uri_complete,
        user_code: payload.user_code,
        expires_in_secs: payload.expires_in,
        poll_interval_secs: payload
            .interval
            .unwrap_or(DEFAULT_DEVICE_POLL_INTERVAL_SECS),
        client_id: config.client_id.clone(),
        base_url: config.base_url.clone(),
        device_code: payload.device_code,
    })
}

pub(super) async fn authenticate_device_code_with_client(
    client: &Client,
    start: &WorkosDeviceLoginStart,
) -> Result<Response> {
    client
        .post(format!(
            "{}/user_management/authenticate",
            start.base_url.trim_end_matches('/')
        ))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form_urlencoded_body(&[
            ("grant_type", WORKOS_DEVICE_GRANT_TYPE),
            ("device_code", start.device_code.as_str()),
            ("client_id", start.client_id.as_str()),
        ]))
        .send()
        .await
        .context("polling WorkOS device login")
}

pub(super) async fn refresh_session_with_client(
    client: &Client,
    base_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<Response> {
    client
        .post(format!(
            "{}/user_management/authenticate",
            base_url.trim_end_matches('/')
        ))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form_urlencoded_body(&[
            ("grant_type", WORKOS_REFRESH_GRANT_TYPE),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ]))
        .send()
        .await
        .context("refreshing WorkOS session")
}

pub(super) fn refresh_session_with_blocking_client(
    client: &BlockingClient,
    base_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<BlockingResponse> {
    client
        .post(format!(
            "{}/user_management/authenticate",
            base_url.trim_end_matches('/')
        ))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form_urlencoded_body(&[
            ("grant_type", WORKOS_REFRESH_GRANT_TYPE),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ]))
        .send()
        .context("refreshing WorkOS session")
}

pub(super) fn workos_http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(WORKOS_HTTP_TIMEOUT_SECS))
        .user_agent("bitloops-cli")
        .build()
        .context("building WorkOS HTTP client")
}

pub(super) fn workos_blocking_http_client() -> Result<BlockingClient> {
    BlockingClient::builder()
        .timeout(Duration::from_secs(WORKOS_HTTP_TIMEOUT_SECS))
        .user_agent("bitloops-cli")
        .build()
        .context("building WorkOS HTTP client")
}

pub(super) fn oauth_error_message(prefix: &str, error: &WorkosOAuthError) -> String {
    match error.error_description.as_deref().map(str::trim) {
        Some(description) if !description.is_empty() => {
            format!("{prefix}: {} ({description})", error.error)
        }
        _ => format!("{prefix}: {}", error.error),
    }
}

fn format_body_suffix(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!(": {trimmed}")
    }
}

fn form_urlencoded_body(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}
