use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;

use crate::host::runtime_store::DaemonSqliteRuntimeStore;

use super::credentials::{
    SecureCredentialStore, default_secure_store, delete_tokens, load_tokens, save_tokens,
};
use super::http::{
    authenticate_device_code_with_client, oauth_error_message,
    refresh_session_with_blocking_client, refresh_session_with_client,
    start_device_login_with_client, workos_blocking_http_client, workos_http_client,
};
use super::types::{
    ACCESS_TOKEN_REFRESH_SKEW_SECS, DEFAULT_WORKOS_BASE_URL, DEFAULT_WORKOS_CLIENT_ID,
    PersistedWorkosAuthSessionState, StoredWorkosTokens, WORKOS_BASE_URL_ENV, WORKOS_CLIENT_ID_ENV,
    WORKOS_KEYRING_SERVICE, WORKOS_SESSION_STATE_VERSION, WorkosAuthSettings, WorkosCredentialKey,
    WorkosDeviceLoginStart, WorkosJwtClaims, WorkosLoginStart, WorkosOAuthError,
    WorkosSessionDetails, WorkosTokenResponse,
};

pub async fn prepare_workos_device_login() -> Result<WorkosLoginStart> {
    prepare_workos_device_login_with_store_and_env(default_secure_store(), |key| {
        std::env::var(key).ok()
    })
    .await
}

pub(super) async fn prepare_workos_device_login_with_store_and_env<F>(
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

    let client = workos_http_client()?;
    start_device_login_with_client(&client, &config)
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

pub(crate) fn load_workos_session_details_cached() -> Result<Option<WorkosSessionDetails>> {
    Ok(load_workos_session_state()?.map(|state| session_details_from_state(&state)))
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

pub(super) async fn complete_workos_device_login_with_store(
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

        let response = authenticate_device_code_with_client(&client, start).await?;

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

pub(super) async fn resolve_workos_session_status_with_store(
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
    let response = refresh_session_with_client(
        &client,
        &state.base_url,
        &state.client_id,
        refresh_token.as_str(),
    )
    .await?;

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

pub(super) fn load_workos_session_state() -> Result<Option<PersistedWorkosAuthSessionState>> {
    DaemonSqliteRuntimeStore::open()?.load_workos_auth_session_state()
}

pub(super) fn save_workos_session_state(state: &PersistedWorkosAuthSessionState) -> Result<()> {
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

pub(super) fn credential_key_for_client(client_id: &str) -> WorkosCredentialKey {
    WorkosCredentialKey {
        service: WORKOS_KEYRING_SERVICE.to_string(),
        account: client_id.to_string(),
    }
}

pub(super) fn resolve_workos_auth_settings_with<F>(env_lookup: F) -> WorkosAuthSettings
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

fn refresh_session_tokens_blocking(
    state: &PersistedWorkosAuthSessionState,
    refresh_token: String,
    store: Arc<dyn SecureCredentialStore>,
    key: WorkosCredentialKey,
) -> Result<Option<String>> {
    let client = workos_blocking_http_client()?;
    let response = refresh_session_with_blocking_client(
        &client,
        &state.base_url,
        &state.client_id,
        refresh_token.as_str(),
    )?;

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

pub(super) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
