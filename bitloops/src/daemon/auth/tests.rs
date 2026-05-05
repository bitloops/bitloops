use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use base64::Engine as _;
use serde_json::json;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

use crate::test_support::process_state::enter_env_vars;
use crate::utils::platform_dirs::{TestPlatformDirOverrides, with_test_platform_dir_overrides};

use super::credentials::{MemoryCredentialStore, SecureCredentialStore};
use super::session::{
    complete_workos_device_login_with_store, credential_key_for_client, load_workos_session_state,
    now_secs, prepare_workos_device_login_with_store_and_env, resolve_workos_auth_settings_with,
    resolve_workos_session_status_with_store, save_workos_session_state,
};
use super::types::{
    DEFAULT_WORKOS_BASE_URL, DEFAULT_WORKOS_CLIENT_ID, PersistedWorkosAuthSessionState,
    StoredWorkosTokens, WORKOS_BASE_URL_ENV, WORKOS_CLIENT_ID_ENV, WORKOS_DEVICE_GRANT_TYPE,
    WORKOS_REFRESH_GRANT_TYPE, WORKOS_SESSION_STATE_VERSION,
};
use super::{WorkosDeviceLoginStart, WorkosLoginStart};

#[derive(Clone)]
struct AuthServerState {
    poll_count: Arc<Mutex<usize>>,
    refresh_count: Arc<Mutex<usize>>,
}

struct TestAuthServer {
    base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl TestAuthServer {
    async fn shutdown(mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for TestAuthServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

fn localhost_bind_available(test_name: &str) -> bool {
    match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => {
            drop(listener);
            true
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!(
                "skipping {test_name}: loopback sockets are unavailable in this environment ({err})"
            );
            false
        }
        Err(err) => panic!("bind localhost for {test_name}: {err}"),
    }
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
    if !localhost_bind_available("workos_device_login_persists_session_to_runtime_store") {
        return;
    }

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
            let server = runtime.block_on(start_test_auth_server());
            let base_url = server.base_url.clone();
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

            runtime.block_on(server.shutdown());
        },
    );
}

#[test]
fn prepare_workos_login_invalidates_session_for_different_client_id() {
    if !localhost_bind_available("prepare_workos_login_invalidates_session_for_different_client_id")
    {
        return;
    }

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
            let server = runtime.block_on(start_test_auth_server());
            let base_url = server.base_url.clone();
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

            runtime.block_on(server.shutdown());
        },
    );
}

#[test]
fn workos_session_status_refreshes_expired_tokens() {
    if !localhost_bind_available("workos_session_status_refreshes_expired_tokens") {
        return;
    }

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
            let server = runtime.block_on(start_test_auth_server());
            let base_url = server.base_url.clone();
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

            runtime.block_on(server.shutdown());
        },
    );
}

async fn start_test_auth_server() -> TestAuthServer {
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
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
        {
            eprintln!("test auth server stopped with error: {err}");
        }
    });
    TestAuthServer {
        base_url: format!("http://127.0.0.1:{}", addr.port()),
        shutdown_tx: Some(shutdown_tx),
        handle: Some(handle),
    }
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
    let header =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
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
