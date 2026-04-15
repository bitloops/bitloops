use serde::{Deserialize, Serialize};

pub(super) const WORKOS_HTTP_TIMEOUT_SECS: u64 = 10;
pub(super) const DEFAULT_DEVICE_POLL_INTERVAL_SECS: u64 = 5;
pub(super) const ACCESS_TOKEN_REFRESH_SKEW_SECS: u64 = 60;
pub(super) const WORKOS_SESSION_STATE_VERSION: u8 = 1;
pub(super) const WORKOS_KEYRING_SERVICE: &str = "io.bitloops.workos";
pub(super) const WORKOS_DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub(super) const WORKOS_REFRESH_GRANT_TYPE: &str = "refresh_token";
pub(super) const WORKOS_CLIENT_ID_ENV: &str = "BITLOOPS_WORKOS_CLIENT_ID";
pub(super) const WORKOS_BASE_URL_ENV: &str = "BITLOOPS_WORKOS_BASE_URL";
pub(super) const DEFAULT_WORKOS_CLIENT_ID: &str = "client_01KP838YARZTS6P9572557GVG9";
pub(super) const DEFAULT_WORKOS_BASE_URL: &str = "https://api.workos.com";
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
pub(super) struct StoredWorkosTokens {
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkosCredentialKey {
    pub service: String,
    pub account: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkosAuthSettings {
    pub client_id: String,
    pub base_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct WorkosDeviceAuthorizationResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct WorkosTokenResponse {
    pub user: Option<WorkosUser>,
    pub organization_id: Option<String>,
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: Option<String>,
    pub expires_in: Option<u64>,
    pub authentication_method: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct WorkosUser {
    pub id: Option<String>,
    pub email: Option<String>,
    #[serde(default, alias = "firstName")]
    pub first_name: Option<String>,
    #[serde(default, alias = "lastName")]
    pub last_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct WorkosOAuthError {
    pub error: String,
    pub error_description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct WorkosJwtClaims {
    pub exp: Option<u64>,
    pub sid: Option<String>,
    pub sub: Option<String>,
    pub org_id: Option<String>,
    pub email: Option<String>,
}
