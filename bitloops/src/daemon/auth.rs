#[path = "auth/credentials.rs"]
mod credentials;
#[path = "auth/http.rs"]
mod http;
#[path = "auth/session.rs"]
mod session;
#[path = "auth/types.rs"]
mod types;

#[cfg(test)]
#[path = "auth/tests.rs"]
mod tests;

pub(crate) use session::load_workos_session_details_cached;
pub(crate) use session::platform_gateway_bearer_token;
pub use session::{
    complete_workos_device_login, logout_workos_session, prepare_workos_device_login,
    resolve_workos_session_status,
};
pub(crate) use types::{PLATFORM_GATEWAY_TOKEN_ENV, PersistedWorkosAuthSessionState};
pub use types::{WorkosDeviceLoginStart, WorkosLoginStart, WorkosSessionDetails};
