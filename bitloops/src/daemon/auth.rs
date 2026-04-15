use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use reqwest::Client;
use reqwest::blocking::Client as BlockingClient;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

use crate::host::runtime_store::DaemonSqliteRuntimeStore;

const WORKOS_HTTP_TIMEOUT_SECS: u64 = 10;
const DEFAULT_DEVICE_POLL_INTERVAL_SECS: u64 = 5;
const ACCESS_TOKEN_REFRESH_SKEW_SECS: u64 = 60;
const WORKOS_SESSION_STATE_VERSION: u8 = 1;
const WORKOS_KEYRING_SERVICE: &str = "io.bitloops.workos";
const WORKOS_DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const WORKOS_REFRESH_GRANT_TYPE: &str = "refresh_token";
const WORKOS_CLIENT_ID_ENV: &str = "BITLOOPS_WORKOS_CLIENT_ID";
const WORKOS_BASE_URL_ENV: &str = "BITLOOPS_WORKOS_BASE_URL";
const DEFAULT_WORKOS_CLIENT_ID: &str = "client_01KP838YARZTS6P9572557GVG9";
const DEFAULT_WORKOS_BASE_URL: &str = "https://api.workos.com";
pub(crate) const PLATFORM_GATEWAY_TOKEN_ENV: &str = "BITLOOPS_PLATFORM_GATEWAY_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkosLoginStart {
    AlreadyLoggedIn(WorkosSessionDetails),
    Pending(WorkosDeviceLoginStart),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkosDeviceLoginStart {
    pub verification_url: String,
    pub verification_url_complete: Option<String>,
    pub user_code: String,
    pub expires_in_secs: u64,
    pub poll_interval_secs: u64,
    pub client_id: String,
    pub base_url: String,
    pub device_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkosSessionDetails {
    pub client_id: String,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub user_first_name: Option<String>,
    pub user_last_name: Option<String>,
    pub organisation_id: Option<String>,
    pub authentication_method: Option<String>,
    pub access_token_expires_at_unix: Option<u64>,
    pub authenticated_at_unix: u64,
    pub updated_at_unix: u64,
}

impl WorkosSessionDetails {
    pub fn display_label(&self) -> String {
        let first = self.user_first_name.as_deref().unwrap_or("").trim();
        let last = self.user_last_name.as_deref().unwrap_or("").trim();
        let full_name = format!("{first} {last}").trim().to_string();
        if !full_name.is_empty() {
            full_name
        } else if let Some(email) = self.user_email.as_ref() {
            email.clone()
        } else if let Some(user_id) = self.user_id.as_ref() {
            user_id.clone()
        } else {
            "unknown user".to_string()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PersistedWorkosAuthSessionState {
    pub version: u8,
    pub client_id: String,
    pub base_url: String,
    pub keyring_service: String,
    pub keyring_account: String,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub user_first_name: Option<String>,
    pub user_last_name: Option<String>,
    pub organisation_id: Option<String>,
    pub authentication_method: Option<String>,
    pub token_type: Option<String>,
    pub session_id: Option<String>,
    pub subject: Option<String>,
    pub access_token_expires_at_unix: Option<u64>,
    pub authenticated_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StoredWorkosTokens {
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkosCredentialKey {
    service: String,
    account: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkosAuthSettings {
    client_id: String,
    base_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkosDeviceAuthorizationResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkosTokenResponse {
    user: Option<WorkosUser>,
    organization_id: Option<String>,
    access_token: String,
    refresh_token: String,
    token_type: Option<String>,
    expires_in: Option<u64>,
    authentication_method: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkosUser {
    id: Option<String>,
    email: Option<String>,
    #[serde(default, alias = "firstName")]
    first_name: Option<String>,
    #[serde(default, alias = "lastName")]
    last_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkosOAuthError {
    error: String,
    error_description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkosJwtClaims {
    exp: Option<u64>,
    sid: Option<String>,
    sub: Option<String>,
    org_id: Option<String>,
    email: Option<String>,
}

trait SecureCredentialStore: Send + Sync {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>>;
    fn save_tokens(&self, key: &WorkosCredentialKey, tokens: &StoredWorkosTokens) -> Result<()>;
    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()>;
}

struct KeyringCredentialStore;

impl SecureCredentialStore for KeyringCredentialStore {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>> {
        let entry = keyring::Entry::new(&key.service, &key.account)
            .context("opening secure credential entry")?;
        let secret = match entry.get_secret() {
            Ok(secret) => secret,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(err) => return Err(err).context("reading secure credentials"),
        };
        serde_json::from_slice(&secret)
            .context("parsing secure credential payload")
            .map(Some)
    }

    fn save_tokens(&self, key: &WorkosCredentialKey, tokens: &StoredWorkosTokens) -> Result<()> {
        let entry = keyring::Entry::new(&key.service, &key.account)
            .context("opening secure credential entry")?;
        let payload =
            serde_json::to_vec(tokens).context("serialising secure credential payload")?;
        entry
            .set_secret(&payload)
            .context("writing secure credentials")
    }

    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()> {
        let entry = keyring::Entry::new(&key.service, &key.account)
            .context("opening secure credential entry")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).context("deleting secure credentials"),
        }
    }
}

pub async fn prepare_workos_device_login() -> Result<WorkosLoginStart> {
    prepare_workos_device_login_with_store_and_env(default_secure_store(), |key| {
        std::env::var(key).ok()
    })
    .await
}

async fn prepare_workos_device_login_with_store_and_env<F>(
    store: Arc<dyn SecureCredentialStore>,
    env_lookup: F,
) -> Result<WorkosLoginStart>
where
    F: Fn(&str) -> Option<String>,
{
    let config = resolve_workos_auth_settings_with(env_lookup);
    if let Some(session) = resolve_workos_session_status_with_store(store.clone()).await? {
        if session.client_id == config.client_id {
            return Ok(WorkosLoginStart::AlreadyLoggedIn(session));
        }
        logout_workos_session_with_store(store).await?;
    }

    start_device_login_with_config(&config)
        .await
        .map(WorkosLoginStart::Pending)
}

pub async fn complete_workos_device_login(
    start: &WorkosDeviceLoginStart,
) -> Result<WorkosSessionDetails> {
    complete_workos_device_login_with_store(start, default_secure_store()).await
}

pub async fn resolve_workos_session_status() -> Result<Option<WorkosSessionDetails>> {
    resolve_workos_session_status_with_store(default_secure_store()).await
}

pub async fn logout_workos_session() -> Result<bool> {
    logout_workos_session_with_store(default_secure_store()).await
}

pub(crate) fn platform_gateway_bearer_token() -> Result<Option<String>> {
    let store = default_secure_store();
    let Some(state) = load_workos_session_state()? else {
        return Ok(None);
    };
    let key = WorkosCredentialKey {
        service: state.keyring_service.clone(),
        account: state.keyring_account.clone(),
    };
    let Some(tokens) = store.load_tokens(&key)? else {
        clear_workos_session_state_blocking(store, &key)?;
        return Ok(None);
    };

    if !access_token_needs_refresh(state.access_token_expires_at_unix) {
        return Ok(Some(tokens.access_token));
    }

    refresh_session_tokens_blocking(&state, tokens.refresh_token, store, key)
}

async fn start_device_login_with_config(
    config: &WorkosAuthSettings,
) -> Result<WorkosDeviceLoginStart> {
    let client = workos_http_client()?;
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

async fn complete_workos_device_login_with_store(
    start: &WorkosDeviceLoginStart,
    store: Arc<dyn SecureCredentialStore>,
) -> Result<WorkosSessionDetails> {
    let client = workos_http_client()?;
    let deadline = now_secs().saturating_add(start.expires_in_secs);
    let mut poll_interval_secs = start.poll_interval_secs.max(1);

    loop {
        if now_secs() > deadline {
            bail!("WorkOS device authorisation expired before login completed");
        }

        let response = client
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
            .context("polling WorkOS device login")?;

        if response.status().is_success() {
            let tokens = response
                .json::<WorkosTokenResponse>()
                .await
                .context("parsing WorkOS login response")?;
            return persist_authenticated_session(
                store,
                &start.client_id,
                &start.base_url,
                None,
                tokens,
            )
            .await;
        }

        let status = response.status();
        let error = response
            .json::<WorkosOAuthError>()
            .await
            .unwrap_or(WorkosOAuthError {
                error: "unknown_error".to_string(),
                error_description: Some(format!("HTTP {status}")),
            });
        match error.error.as_str() {
            "authorization_pending" => {}
            "slow_down" => {
                poll_interval_secs = poll_interval_secs.saturating_add(5);
            }
            "access_denied" => bail!("WorkOS login was denied by the user"),
            "expired_token" => {
                bail!("WorkOS device authorisation expired; run `bitloops login` again")
            }
            _ => bail!(oauth_error_message("WorkOS login failed", &error)),
        }

        tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
    }
}

async fn resolve_workos_session_status_with_store(
    store: Arc<dyn SecureCredentialStore>,
) -> Result<Option<WorkosSessionDetails>> {
    let Some(state) = load_workos_session_state()? else {
        return Ok(None);
    };

    let key = WorkosCredentialKey {
        service: state.keyring_service.clone(),
        account: state.keyring_account.clone(),
    };
    let Some(tokens) = load_tokens(store.clone(), key.clone()).await? else {
        clear_workos_session_state(store, &state, &key).await?;
        return Ok(None);
    };

    if !access_token_needs_refresh(state.access_token_expires_at_unix) {
        return Ok(Some(session_details_from_state(&state)));
    }

    match refresh_session_tokens(&state, tokens.refresh_token, store, key).await? {
        Some(session) => Ok(Some(session)),
        None => Ok(None),
    }
}

async fn refresh_session_tokens(
    state: &PersistedWorkosAuthSessionState,
    refresh_token: String,
    store: Arc<dyn SecureCredentialStore>,
    key: WorkosCredentialKey,
) -> Result<Option<WorkosSessionDetails>> {
    let client = workos_http_client()?;
    let response = client
        .post(format!(
            "{}/user_management/authenticate",
            state.base_url.trim_end_matches('/')
        ))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form_urlencoded_body(&[
            ("grant_type", WORKOS_REFRESH_GRANT_TYPE),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", state.client_id.as_str()),
        ]))
        .send()
        .await
        .context("refreshing WorkOS session")?;

    if response.status().is_success() {
        let tokens = response
            .json::<WorkosTokenResponse>()
            .await
            .context("parsing refreshed WorkOS session")?;
        let session = persist_authenticated_session(
            store,
            &state.client_id,
            &state.base_url,
            Some(state),
            tokens,
        )
        .await?;
        return Ok(Some(session));
    }

    let status = response.status();
    let error = response
        .json::<WorkosOAuthError>()
        .await
        .unwrap_or(WorkosOAuthError {
            error: "unknown_error".to_string(),
            error_description: Some(format!("HTTP {status}")),
        });

    if matches!(
        error.error.as_str(),
        "access_denied" | "expired_token" | "invalid_client" | "invalid_grant"
    ) {
        clear_workos_session_state(store, state, &key).await?;
        return Ok(None);
    }

    Err(anyhow!(oauth_error_message(
        "refreshing WorkOS session failed",
        &error
    )))
}

async fn persist_authenticated_session(
    store: Arc<dyn SecureCredentialStore>,
    client_id: &str,
    base_url: &str,
    previous: Option<&PersistedWorkosAuthSessionState>,
    response: WorkosTokenResponse,
) -> Result<WorkosSessionDetails> {
    let key = credential_key_for_client(client_id);
    let now = now_secs();
    let claims = decode_jwt_claims(&response.access_token);
    let expires_at_unix = response
        .expires_in
        .map(|expires_in| now.saturating_add(expires_in))
        .or_else(|| claims.as_ref().and_then(|claims| claims.exp));
    let user = response.user.as_ref();

    let state = PersistedWorkosAuthSessionState {
        version: WORKOS_SESSION_STATE_VERSION,
        client_id: client_id.to_string(),
        base_url: base_url.trim_end_matches('/').to_string(),
        keyring_service: key.service.clone(),
        keyring_account: key.account.clone(),
        user_id: user
            .and_then(|user| user.id.clone())
            .or_else(|| previous.and_then(|state| state.user_id.clone()))
            .or_else(|| claims.as_ref().and_then(|claims| claims.sub.clone())),
        user_email: user
            .and_then(|user| user.email.clone())
            .or_else(|| previous.and_then(|state| state.user_email.clone()))
            .or_else(|| claims.as_ref().and_then(|claims| claims.email.clone())),
        user_first_name: user
            .and_then(|user| user.first_name.clone())
            .or_else(|| previous.and_then(|state| state.user_first_name.clone())),
        user_last_name: user
            .and_then(|user| user.last_name.clone())
            .or_else(|| previous.and_then(|state| state.user_last_name.clone())),
        organisation_id: response
            .organization_id
            .clone()
            .or_else(|| previous.and_then(|state| state.organisation_id.clone()))
            .or_else(|| claims.as_ref().and_then(|claims| claims.org_id.clone())),
        authentication_method: response
            .authentication_method
            .clone()
            .or_else(|| previous.and_then(|state| state.authentication_method.clone())),
        token_type: response
            .token_type
            .clone()
            .or_else(|| previous.and_then(|state| state.token_type.clone())),
        session_id: claims
            .as_ref()
            .and_then(|claims| claims.sid.clone())
            .or_else(|| previous.and_then(|state| state.session_id.clone())),
        subject: claims
            .as_ref()
            .and_then(|claims| claims.sub.clone())
            .or_else(|| previous.and_then(|state| state.subject.clone())),
        access_token_expires_at_unix: expires_at_unix,
        authenticated_at_unix: previous
            .map(|state| state.authenticated_at_unix)
            .unwrap_or(now),
        updated_at_unix: now,
    };

    save_tokens(
        store,
        key,
        StoredWorkosTokens {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
        },
    )
    .await?;
    save_workos_session_state(&state)?;
    Ok(session_details_from_state(&state))
}

async fn logout_workos_session_with_store(store: Arc<dyn SecureCredentialStore>) -> Result<bool> {
    let Some(state) = load_workos_session_state()? else {
        return Ok(false);
    };
    let key = WorkosCredentialKey {
        service: state.keyring_service.clone(),
        account: state.keyring_account.clone(),
    };
    clear_workos_session_state(store, &state, &key).await?;
    Ok(true)
}

async fn clear_workos_session_state(
    store: Arc<dyn SecureCredentialStore>,
    _state: &PersistedWorkosAuthSessionState,
    key: &WorkosCredentialKey,
) -> Result<()> {
    delete_tokens(store, key.clone()).await?;
    delete_workos_session_state()
}

fn clear_workos_session_state_blocking(
    store: Arc<dyn SecureCredentialStore>,
    key: &WorkosCredentialKey,
) -> Result<()> {
    store.delete_tokens(key)?;
    delete_workos_session_state()
}

fn load_workos_session_state() -> Result<Option<PersistedWorkosAuthSessionState>> {
    DaemonSqliteRuntimeStore::open()?.load_workos_auth_session_state()
}

fn save_workos_session_state(state: &PersistedWorkosAuthSessionState) -> Result<()> {
    DaemonSqliteRuntimeStore::open()?.save_workos_auth_session_state(state)
}

fn delete_workos_session_state() -> Result<()> {
    DaemonSqliteRuntimeStore::open()?.delete_workos_auth_session_state()
}

fn session_details_from_state(state: &PersistedWorkosAuthSessionState) -> WorkosSessionDetails {
    WorkosSessionDetails {
        client_id: state.client_id.clone(),
        user_id: state.user_id.clone(),
        user_email: state.user_email.clone(),
        user_first_name: state.user_first_name.clone(),
        user_last_name: state.user_last_name.clone(),
        organisation_id: state.organisation_id.clone(),
        authentication_method: state.authentication_method.clone(),
        access_token_expires_at_unix: state.access_token_expires_at_unix,
        authenticated_at_unix: state.authenticated_at_unix,
        updated_at_unix: state.updated_at_unix,
    }
}

fn access_token_needs_refresh(expires_at_unix: Option<u64>) -> bool {
    let Some(expires_at_unix) = expires_at_unix else {
        return true;
    };
    now_secs().saturating_add(ACCESS_TOKEN_REFRESH_SKEW_SECS) >= expires_at_unix
}

fn decode_jwt_claims(token: &str) -> Option<WorkosJwtClaims> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice::<WorkosJwtClaims>(&decoded).ok()
}

fn credential_key_for_client(client_id: &str) -> WorkosCredentialKey {
    WorkosCredentialKey {
        service: WORKOS_KEYRING_SERVICE.to_string(),
        account: client_id.to_string(),
    }
}

fn resolve_workos_auth_settings_with<F>(env_lookup: F) -> WorkosAuthSettings
where
    F: Fn(&str) -> Option<String>,
{
    let client_id = read_non_empty_env_value(&env_lookup, WORKOS_CLIENT_ID_ENV)
        .unwrap_or_else(|| DEFAULT_WORKOS_CLIENT_ID.to_string());
    let base_url = read_non_empty_env_value(&env_lookup, WORKOS_BASE_URL_ENV)
        .unwrap_or_else(|| DEFAULT_WORKOS_BASE_URL.to_string());

    WorkosAuthSettings {
        client_id,
        base_url: base_url.trim_end_matches('/').to_string(),
    }
}

fn read_non_empty_env_value<F>(env_lookup: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    env_lookup(key).and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn workos_http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(WORKOS_HTTP_TIMEOUT_SECS))
        .user_agent("bitloops-cli")
        .build()
        .context("building WorkOS HTTP client")
}

fn workos_blocking_http_client() -> Result<BlockingClient> {
    BlockingClient::builder()
        .timeout(Duration::from_secs(WORKOS_HTTP_TIMEOUT_SECS))
        .user_agent("bitloops-cli")
        .build()
        .context("building WorkOS HTTP client")
}

fn default_secure_store() -> Arc<dyn SecureCredentialStore> {
    Arc::new(KeyringCredentialStore)
}

async fn load_tokens(
    store: Arc<dyn SecureCredentialStore>,
    key: WorkosCredentialKey,
) -> Result<Option<StoredWorkosTokens>> {
    tokio::task::spawn_blocking(move || store.load_tokens(&key))
        .await
        .context("joining secure credential read task")?
}

async fn save_tokens(
    store: Arc<dyn SecureCredentialStore>,
    key: WorkosCredentialKey,
    tokens: StoredWorkosTokens,
) -> Result<()> {
    tokio::task::spawn_blocking(move || store.save_tokens(&key, &tokens))
        .await
        .context("joining secure credential write task")?
}

async fn delete_tokens(
    store: Arc<dyn SecureCredentialStore>,
    key: WorkosCredentialKey,
) -> Result<()> {
    tokio::task::spawn_blocking(move || store.delete_tokens(&key))
        .await
        .context("joining secure credential delete task")?
}

fn oauth_error_message(prefix: &str, error: &WorkosOAuthError) -> String {
    match error.error_description.as_deref().map(str::trim) {
        Some(description) if !description.is_empty() => {
            format!("{prefix}: {} ({description})", error.error)
        }
        _ => format!("{prefix}: {}", error.error),
    }
}

fn refresh_session_tokens_blocking(
    state: &PersistedWorkosAuthSessionState,
    refresh_token: String,
    store: Arc<dyn SecureCredentialStore>,
    key: WorkosCredentialKey,
) -> Result<Option<String>> {
    let client = workos_blocking_http_client()?;
    let response = client
        .post(format!(
            "{}/user_management/authenticate",
            state.base_url.trim_end_matches('/')
        ))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form_urlencoded_body(&[
            ("grant_type", WORKOS_REFRESH_GRANT_TYPE),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", state.client_id.as_str()),
        ]))
        .send()
        .context("refreshing WorkOS session")?;

    if response.status().is_success() {
        let tokens = response
            .json::<WorkosTokenResponse>()
            .context("parsing refreshed WorkOS session")?;
        let access_token = tokens.access_token.clone();
        persist_authenticated_session_blocking(
            store,
            &state.client_id,
            &state.base_url,
            Some(state),
            tokens,
        )?;
        return Ok(Some(access_token));
    }

    let status = response.status();
    let error = response
        .json::<WorkosOAuthError>()
        .unwrap_or(WorkosOAuthError {
            error: "unknown_error".to_string(),
            error_description: Some(format!("HTTP {status}")),
        });

    if matches!(
        error.error.as_str(),
        "access_denied" | "expired_token" | "invalid_client" | "invalid_grant"
    ) {
        clear_workos_session_state_blocking(store, &key)?;
        return Ok(None);
    }

    Err(anyhow!(oauth_error_message(
        "refreshing WorkOS session failed",
        &error
    )))
}

fn persist_authenticated_session_blocking(
    store: Arc<dyn SecureCredentialStore>,
    client_id: &str,
    base_url: &str,
    previous: Option<&PersistedWorkosAuthSessionState>,
    response: WorkosTokenResponse,
) -> Result<WorkosSessionDetails> {
    let key = credential_key_for_client(client_id);
    let now = now_secs();
    let claims = decode_jwt_claims(&response.access_token);
    let expires_at_unix = response
        .expires_in
        .map(|expires_in| now.saturating_add(expires_in))
        .or_else(|| claims.as_ref().and_then(|claims| claims.exp));
    let user = response.user.as_ref();

    let state = PersistedWorkosAuthSessionState {
        version: WORKOS_SESSION_STATE_VERSION,
        client_id: client_id.to_string(),
        base_url: base_url.trim_end_matches('/').to_string(),
        keyring_service: key.service.clone(),
        keyring_account: key.account.clone(),
        user_id: user
            .and_then(|user| user.id.clone())
            .or_else(|| previous.and_then(|state| state.user_id.clone()))
            .or_else(|| claims.as_ref().and_then(|claims| claims.sub.clone())),
        user_email: user
            .and_then(|user| user.email.clone())
            .or_else(|| previous.and_then(|state| state.user_email.clone()))
            .or_else(|| claims.as_ref().and_then(|claims| claims.email.clone())),
        user_first_name: user
            .and_then(|user| user.first_name.clone())
            .or_else(|| previous.and_then(|state| state.user_first_name.clone())),
        user_last_name: user
            .and_then(|user| user.last_name.clone())
            .or_else(|| previous.and_then(|state| state.user_last_name.clone())),
        organisation_id: response
            .organization_id
            .clone()
            .or_else(|| previous.and_then(|state| state.organisation_id.clone()))
            .or_else(|| claims.as_ref().and_then(|claims| claims.org_id.clone())),
        authentication_method: response
            .authentication_method
            .clone()
            .or_else(|| previous.and_then(|state| state.authentication_method.clone())),
        token_type: response
            .token_type
            .clone()
            .or_else(|| previous.and_then(|state| state.token_type.clone())),
        session_id: claims
            .as_ref()
            .and_then(|claims| claims.sid.clone())
            .or_else(|| previous.and_then(|state| state.session_id.clone())),
        subject: claims
            .as_ref()
            .and_then(|claims| claims.sub.clone())
            .or_else(|| previous.and_then(|state| state.subject.clone())),
        access_token_expires_at_unix: expires_at_unix,
        authenticated_at_unix: previous
            .map(|state| state.authenticated_at_unix)
            .unwrap_or(now),
        updated_at_unix: now,
    };

    store.save_tokens(
        &key,
        &StoredWorkosTokens {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
        },
    )?;
    save_workos_session_state(&state)?;
    Ok(session_details_from_state(&state))
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[derive(Default)]
struct MemoryCredentialStore {
    inner: Mutex<HashMap<(String, String), StoredWorkosTokens>>,
}

#[cfg(test)]
impl SecureCredentialStore for MemoryCredentialStore {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>> {
        Ok(self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&(key.service.clone(), key.account.clone()))
            .cloned())
    }

    fn save_tokens(&self, key: &WorkosCredentialKey, tokens: &StoredWorkosTokens) -> Result<()> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert((key.service.clone(), key.account.clone()), tokens.clone());
        Ok(())
    }

    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&(key.service.clone(), key.account.clone()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        routing::{get, post},
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tokio::net::TcpListener;

    use crate::test_support::process_state::enter_env_vars;
    use crate::utils::platform_dirs::{TestPlatformDirOverrides, with_test_platform_dir_overrides};

    #[derive(Clone)]
    struct AuthServerState {
        poll_count: Arc<Mutex<usize>>,
        refresh_count: Arc<Mutex<usize>>,
    }

    #[test]
    fn workos_auth_settings_default_to_built_in_values() {
        let _guard = enter_env_vars(&[(WORKOS_CLIENT_ID_ENV, None), (WORKOS_BASE_URL_ENV, None)]);
        let settings = resolve_workos_auth_settings_with(|key| std::env::var(key).ok());
        assert_eq!(settings.client_id, DEFAULT_WORKOS_CLIENT_ID);
        assert_eq!(settings.base_url, DEFAULT_WORKOS_BASE_URL);
    }

    #[test]
    fn workos_auth_settings_allow_client_id_override() {
        let settings = resolve_workos_auth_settings_with(|key| match key {
            WORKOS_CLIENT_ID_ENV => Some("client_override".to_string()),
            _ => None,
        });
        assert_eq!(settings.client_id, "client_override");
        assert_eq!(settings.base_url, DEFAULT_WORKOS_BASE_URL);
    }

    #[test]
    fn workos_auth_settings_allow_base_url_override() {
        let settings = resolve_workos_auth_settings_with(|key| match key {
            WORKOS_BASE_URL_ENV => Some("https://workos.example.test///".to_string()),
            _ => None,
        });
        assert_eq!(settings.client_id, DEFAULT_WORKOS_CLIENT_ID);
        assert_eq!(settings.base_url, "https://workos.example.test");
    }

    #[test]
    fn workos_device_login_persists_session_to_runtime_store() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(MemoryCredentialStore::default());

        with_test_platform_dir_overrides(
            TestPlatformDirOverrides {
                config_root: Some(temp.path().join("config")),
                data_root: Some(temp.path().join("data")),
                cache_root: Some(temp.path().join("cache")),
                state_root: Some(temp.path().join("state")),
            },
            || {
                let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
                let (base_url, _shutdown) = runtime.block_on(start_test_auth_server());
                let start = WorkosDeviceLoginStart {
                    verification_url: format!("{base_url}/verify"),
                    verification_url_complete: Some(format!("{base_url}/verify?code=TEST-CODE")),
                    user_code: "TEST-CODE".to_string(),
                    expires_in_secs: 30,
                    poll_interval_secs: 0,
                    client_id: "client_test".to_string(),
                    base_url: base_url.clone(),
                    device_code: "device_123".to_string(),
                };

                let session = runtime
                    .block_on(complete_workos_device_login_with_store(
                        &start,
                        store.clone(),
                    ))
                    .expect("complete login");

                assert_eq!(session.user_email.as_deref(), Some("cli@example.com"));
                let persisted = load_workos_session_state()
                    .expect("load state")
                    .expect("state should exist");
                assert_eq!(persisted.client_id, "client_test");
                assert_eq!(persisted.user_email.as_deref(), Some("cli@example.com"));

                let tokens = store
                    .load_tokens(&credential_key_for_client("client_test"))
                    .expect("load tokens")
                    .expect("tokens should exist");
                assert!(tokens.access_token.starts_with("ey"));
            },
        );
    }

    #[test]
    fn prepare_workos_login_invalidates_session_for_different_client_id() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(MemoryCredentialStore::default());

        with_test_platform_dir_overrides(
            TestPlatformDirOverrides {
                config_root: Some(temp.path().join("config")),
                data_root: Some(temp.path().join("data")),
                cache_root: Some(temp.path().join("cache")),
                state_root: Some(temp.path().join("state")),
            },
            || {
                let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
                let (base_url, _shutdown) = runtime.block_on(start_test_auth_server());
                let key = credential_key_for_client("client_old");
                store
                    .save_tokens(
                        &key,
                        &StoredWorkosTokens {
                            access_token: fake_jwt(
                                now_secs().saturating_add(600),
                                "sid-old",
                                "user_old",
                                Some("old@example.com"),
                            ),
                            refresh_token: "refresh_old".to_string(),
                        },
                    )
                    .expect("seed tokens");
                save_workos_session_state(&PersistedWorkosAuthSessionState {
                    version: WORKOS_SESSION_STATE_VERSION,
                    client_id: "client_old".to_string(),
                    base_url: DEFAULT_WORKOS_BASE_URL.to_string(),
                    keyring_service: key.service.clone(),
                    keyring_account: key.account.clone(),
                    user_id: Some("user_old".to_string()),
                    user_email: Some("old@example.com".to_string()),
                    user_first_name: Some("Old".to_string()),
                    user_last_name: Some("User".to_string()),
                    organisation_id: None,
                    authentication_method: Some("Password".to_string()),
                    token_type: Some("Bearer".to_string()),
                    session_id: Some("sid-old".to_string()),
                    subject: Some("user_old".to_string()),
                    access_token_expires_at_unix: Some(now_secs().saturating_add(600)),
                    authenticated_at_unix: now_secs().saturating_sub(60),
                    updated_at_unix: now_secs().saturating_sub(60),
                })
                .expect("seed session state");

                let pending = runtime
                    .block_on(prepare_workos_device_login_with_store_and_env(
                        store.clone(),
                        |key| match key {
                            WORKOS_CLIENT_ID_ENV => Some("client_new".to_string()),
                            WORKOS_BASE_URL_ENV => Some(format!("{base_url}/")),
                            _ => None,
                        },
                    ))
                    .expect("prepare login");

                let pending = match pending {
                    WorkosLoginStart::Pending(pending) => pending,
                    WorkosLoginStart::AlreadyLoggedIn(_) => {
                        panic!("expected login to restart for a different client id")
                    }
                };

                assert_eq!(pending.client_id, "client_new");
                assert_eq!(pending.base_url, base_url);
                assert!(load_workos_session_state().expect("load state").is_none());
                assert!(
                    store
                        .load_tokens(&credential_key_for_client("client_old"))
                        .expect("load old tokens")
                        .is_none()
                );
            },
        );
    }

    #[test]
    fn workos_session_status_refreshes_expired_tokens() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(MemoryCredentialStore::default());

        with_test_platform_dir_overrides(
            TestPlatformDirOverrides {
                config_root: Some(temp.path().join("config")),
                data_root: Some(temp.path().join("data")),
                cache_root: Some(temp.path().join("cache")),
                state_root: Some(temp.path().join("state")),
            },
            || {
                let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
                let (base_url, _shutdown) = runtime.block_on(start_test_auth_server());
                let old_exp = now_secs().saturating_sub(30);
                let key = credential_key_for_client("client_test");
                store
                    .save_tokens(
                        &key,
                        &StoredWorkosTokens {
                            access_token: fake_jwt(old_exp, "sid-old", "user_old", None),
                            refresh_token: "refresh_old".to_string(),
                        },
                    )
                    .expect("seed tokens");
                save_workos_session_state(&PersistedWorkosAuthSessionState {
                    version: WORKOS_SESSION_STATE_VERSION,
                    client_id: "client_test".to_string(),
                    base_url: base_url.clone(),
                    keyring_service: key.service.clone(),
                    keyring_account: key.account.clone(),
                    user_id: Some("user_old".to_string()),
                    user_email: Some("old@example.com".to_string()),
                    user_first_name: Some("Old".to_string()),
                    user_last_name: Some("User".to_string()),
                    organisation_id: None,
                    authentication_method: Some("Password".to_string()),
                    token_type: Some("Bearer".to_string()),
                    session_id: Some("sid-old".to_string()),
                    subject: Some("user_old".to_string()),
                    access_token_expires_at_unix: Some(old_exp),
                    authenticated_at_unix: now_secs().saturating_sub(60),
                    updated_at_unix: now_secs().saturating_sub(60),
                })
                .expect("seed session state");

                let session = runtime
                    .block_on(resolve_workos_session_status_with_store(store.clone()))
                    .expect("resolve status")
                    .expect("session should exist");

                assert_eq!(session.user_email.as_deref(), Some("refreshed@example.com"));
                assert!(
                    session
                        .access_token_expires_at_unix
                        .is_some_and(|value| value > now_secs())
                );
            },
        );
    }

    async fn start_test_auth_server() -> (String, tokio::task::JoinHandle<()>) {
        let state = AuthServerState {
            poll_count: Arc::new(Mutex::new(0)),
            refresh_count: Arc::new(Mutex::new(0)),
        };
        let app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route(
                "/user_management/authorize/device",
                post(handle_test_authorize_device),
            )
            .route(
                "/user_management/authenticate",
                post(handle_test_authenticate),
            )
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve auth router");
        });
        (format!("http://127.0.0.1:{}", addr.port()), handle)
    }

    async fn handle_test_authorize_device() -> Json<serde_json::Value> {
        Json(json!({
            "device_code": "device_test",
            "user_code": "TEST-CODE",
            "verification_uri": "https://workos.example.test/activate",
            "verification_uri_complete": "https://workos.example.test/activate?code=TEST-CODE",
            "expires_in": 600,
            "interval": 1
        }))
    }

    async fn handle_test_authenticate(
        State(state): State<AuthServerState>,
        form: axum::extract::Form<HashMap<String, String>>,
    ) -> std::result::Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
        match form.get("grant_type").map(String::as_str) {
            Some(WORKOS_DEVICE_GRANT_TYPE) => {
                let mut poll_count = state
                    .poll_count
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                *poll_count += 1;
                if *poll_count == 1 {
                    return Err((
                        axum::http::StatusCode::BAD_REQUEST,
                        json!({
                            "error": "authorization_pending",
                            "error_description": "Pending"
                        })
                        .to_string(),
                    ));
                }

                let exp = now_secs().saturating_add(600);
                Ok(Json(json!({
                    "user": {
                        "id": "user_cli",
                        "email": "cli@example.com",
                        "first_name": "CLI",
                        "last_name": "User"
                    },
                    "organization_id": "org_cli",
                    "access_token": fake_jwt(exp, "sid-cli", "user_cli", Some("cli@example.com")),
                    "refresh_token": "refresh_cli",
                    "token_type": "Bearer",
                    "expires_in": 600,
                    "authentication_method": "Password"
                })))
            }
            Some(WORKOS_REFRESH_GRANT_TYPE) => {
                let mut refresh_count = state
                    .refresh_count
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                *refresh_count += 1;
                let exp = now_secs().saturating_add(900);
                Ok(Json(json!({
                    "user": {
                        "id": "user_refreshed",
                        "email": "refreshed@example.com",
                        "first_name": "Refreshed",
                        "last_name": "User"
                    },
                    "organization_id": "org_refreshed",
                    "access_token": fake_jwt(exp, "sid-refreshed", "user_refreshed", Some("refreshed@example.com")),
                    "refresh_token": "refresh_new",
                    "token_type": "Bearer",
                    "expires_in": 900,
                    "authentication_method": "Password"
                })))
            }
            _ => Err((
                axum::http::StatusCode::BAD_REQUEST,
                json!({
                    "error": "invalid_request",
                    "error_description": "missing grant type"
                })
                .to_string(),
            )),
        }
    }

    fn fake_jwt(exp: u64, sid: &str, sub: &str, email: Option<&str>) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            json!({
                "exp": exp,
                "sid": sid,
                "sub": sub,
                "email": email
            })
            .to_string(),
        );
        format!("{header}.{payload}.signature")
    }
}
