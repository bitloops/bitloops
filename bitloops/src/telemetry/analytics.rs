use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Args;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// PostHog API key for the Bitloops project.
/// This is a public key - it can only send events, not read data.
pub const POSTHOG_API_KEY: &str = "phc_MSoOVb9El27W2DAgq2bTCC8JWbZQMxyRNQ7L0jOjKbZ";

/// PostHog base URL; the REST capture endpoint is derived as `{POSTHOG_ENDPOINT}/capture/`.
pub const POSTHOG_ENDPOINT: &str = "https://eu.i.posthog.com";
const SESSION_STARTED_EVENT: &str = "$session_start";
const SESSION_ENDED_EVENT: &str = "$session_end";

/// If this env var is set to any non-empty value, telemetry is disabled (user opt-out).
pub const TELEMETRY_OPTOUT_ENV: &str = "BITLOOPS_TELEMETRY_OPTOUT";

/// Namespace used when hashing machine id into distinct_id (avoids collisions with other products).
/// Not the PostHog project ID; use your project ID here if you want to namespace by project.
pub const DISTINCT_ID_NAMESPACE: &str = "137911";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPayload {
    pub event: String,
    #[serde(rename = "distinct_id")]
    pub distinct_id: String,
    pub properties: HashMap<String, Value>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Default)]
pub struct ActionDescriptor {
    pub event: String,
    pub surface: &'static str,
    pub properties: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct TelemetryDispatchContext {
    pub strategy: Option<String>,
    pub agent: Option<String>,
}

#[derive(Args)]
pub struct SendAnalyticsArgs {
    pub payload: String,
}

pub fn load_dispatch_context() -> Option<TelemetryDispatchContext> {
    let repo_root = crate::utils::paths::repo_root().ok()?;
    load_dispatch_context_for_repo(&repo_root)
}

pub fn load_global_dispatch_context() -> Option<TelemetryDispatchContext> {
    let daemon = crate::config::load_daemon_settings(None).ok()?;
    if daemon.cli.telemetry != Some(true) {
        return None;
    }

    Some(TelemetryDispatchContext {
        strategy: None,
        agent: None,
    })
}

pub fn load_dispatch_context_for_repo(repo_root: &Path) -> Option<TelemetryDispatchContext> {
    let settings = crate::config::settings::load_settings(repo_root).ok()?;
    if settings.telemetry != Some(true) {
        return None;
    }

    let agent = detect_installed_agents(repo_root).join(",");
    Some(TelemetryDispatchContext {
        strategy: Some(settings.strategy),
        agent: (!agent.is_empty()).then_some(agent),
    })
}

pub fn build_action_payload(
    descriptor: &ActionDescriptor,
    dispatch_context: &TelemetryDispatchContext,
    version: &str,
    success: bool,
    duration_ms: u128,
    session_id: Option<String>,
) -> Option<EventPayload> {
    let machine_id = distinct_machine_id()?;
    let mut properties = descriptor.properties.clone();

    properties.insert(
        "surface".to_string(),
        Value::String(descriptor.surface.to_string()),
    );
    properties.insert(
        "result".to_string(),
        Value::String(if success { "success" } else { "error" }.to_string()),
    );
    properties.insert(
        "duration_ms".to_string(),
        Value::Number(serde_json::Number::from(
            u64::try_from(duration_ms).unwrap_or(u64::MAX),
        )),
    );
    properties.insert(
        "cli_version".to_string(),
        Value::String(version.to_string()),
    );
    properties.insert("os".to_string(), Value::String(env::consts::OS.to_string()));
    properties.insert(
        "arch".to_string(),
        Value::String(env::consts::ARCH.to_string()),
    );

    if let Some(strategy) = dispatch_context.strategy.as_deref() {
        properties.insert("strategy".to_string(), Value::String(strategy.to_string()));
    }
    if let Some(agent) = dispatch_context.agent.as_deref()
        && !agent.is_empty()
    {
        properties.insert("agent".to_string(), Value::String(agent.to_string()));
    }
    if let Some(session_id) = session_id {
        properties.insert("$session_id".to_string(), Value::String(session_id));
    }

    Some(EventPayload {
        event: descriptor.event.clone(),
        distinct_id: machine_id,
        properties,
        timestamp: now_rfc3339(),
    })
}

pub fn track_action_detached(
    descriptor: Option<&ActionDescriptor>,
    dispatch_context: &TelemetryDispatchContext,
    version: &str,
    repo_root: Option<&Path>,
    success: bool,
    duration_ms: u128,
) {
    if env::var(TELEMETRY_OPTOUT_ENV).is_ok_and(|v| !v.is_empty()) {
        return;
    }

    let Some(descriptor) = descriptor else {
        return;
    };

    let session_id = repo_root.and_then(|repo_root| {
        let strategy = dispatch_context.strategy.as_deref()?;
        process_session_activity(repo_root, strategy, descriptor.surface).map(|activity| {
            for payload in activity.lifecycle_events {
                spawn_detached_event_payload(&payload);
            }
            activity.session_id
        })
    });

    let Some(payload) = build_action_payload(
        descriptor,
        dispatch_context,
        version,
        success,
        duration_ms,
        session_id,
    ) else {
        return;
    };

    spawn_detached_event_payload(&payload);
}

pub fn track_session_activity_detached(repo_root: &Path, strategy: &str, source: &str) {
    if env::var(TELEMETRY_OPTOUT_ENV).is_ok_and(|v| !v.is_empty()) {
        return;
    }

    if let Some(activity) = process_session_activity(repo_root, strategy, source) {
        for payload in activity.lifecycle_events {
            spawn_detached_event_payload(&payload);
        }
    }
}

pub fn track_session_end_detached(ended: &crate::telemetry::sessions::EndedSession, source: &str) {
    if env::var(TELEMETRY_OPTOUT_ENV).is_ok_and(|v| !v.is_empty()) {
        return;
    }

    if let Some(payload) = build_session_end_payload(ended, source) {
        spawn_detached_event_payload(&payload);
    }
}

struct SessionActivity {
    session_id: String,
    lifecycle_events: Vec<EventPayload>,
}

fn process_session_activity(
    repo_root: &Path,
    strategy: &str,
    source: &str,
) -> Option<SessionActivity> {
    let Ok(state_dir) = crate::utils::platform_dirs::bitloops_state_dir() else {
        return None;
    };
    let (mut session_store, expired_sessions) =
        crate::telemetry::sessions::SessionStore::load_with_expired(&state_dir);

    let mut lifecycle_events = expired_sessions
        .iter()
        .filter(|ended| load_dispatch_context_for_repo(Path::new(&ended.repo_root)).is_some())
        .filter_map(|ended| build_session_end_payload(ended, source))
        .collect::<Vec<_>>();

    let session_result = session_store.get_or_create_session(repo_root);
    if session_result.is_new_session
        && let Some(payload) =
            build_session_start_payload(&session_result.session_id, strategy, source)
    {
        lifecycle_events.push(payload);
    }

    let _ = session_store.save(&state_dir);

    Some(SessionActivity {
        session_id: session_result.session_id,
        lifecycle_events,
    })
}

fn build_session_start_payload(
    session_id: &str,
    strategy: &str,
    source: &str,
) -> Option<EventPayload> {
    let machine_id = distinct_machine_id()?;
    let properties = HashMap::from([
        (
            "$session_id".to_string(),
            Value::String(session_id.to_string()),
        ),
        (
            "$session_start_timestamp".to_string(),
            Value::String(now_rfc3339()),
        ),
        ("strategy".to_string(), Value::String(strategy.to_string())),
        ("source".to_string(), Value::String(source.to_string())),
        (
            "cli_version".to_string(),
            Value::String(env!("CARGO_PKG_VERSION").to_string()),
        ),
        ("os".to_string(), Value::String(env::consts::OS.to_string())),
        (
            "arch".to_string(),
            Value::String(env::consts::ARCH.to_string()),
        ),
    ]);

    Some(EventPayload {
        event: SESSION_STARTED_EVENT.to_string(),
        distinct_id: machine_id,
        properties,
        timestamp: now_rfc3339(),
    })
}

fn build_session_end_payload(
    ended: &crate::telemetry::sessions::EndedSession,
    source: &str,
) -> Option<EventPayload> {
    let machine_id = distinct_machine_id()?;
    let properties = HashMap::from([
        (
            "$session_id".to_string(),
            Value::String(ended.session_id.clone()),
        ),
        (
            "$session_start_timestamp".to_string(),
            Value::String(rfc3339_from_secs(ended.started_at)),
        ),
        (
            "$session_end_timestamp".to_string(),
            Value::String(rfc3339_from_secs(ended.ended_at)),
        ),
        (
            "$session_duration".to_string(),
            Value::Number(serde_json::Number::from(ended.duration_secs)),
        ),
        ("source".to_string(), Value::String(source.to_string())),
        (
            "cli_version".to_string(),
            Value::String(env!("CARGO_PKG_VERSION").to_string()),
        ),
        ("os".to_string(), Value::String(env::consts::OS.to_string())),
        (
            "arch".to_string(),
            Value::String(env::consts::ARCH.to_string()),
        ),
    ]);

    Some(EventPayload {
        event: SESSION_ENDED_EVENT.to_string(),
        distinct_id: machine_id,
        properties,
        timestamp: now_rfc3339(),
    })
}

pub fn send_session_end(ended: &crate::telemetry::sessions::EndedSession) {
    track_session_end_detached(ended, "daemon");
}

fn spawn_detached_event_payload(payload: &EventPayload) {
    if let Ok(payload_json) = serde_json::to_string(payload) {
        spawn_detached_analytics(&payload_json);
    }
}

pub fn send_event(payload_json: &str) {
    let debug = debug_enabled();

    let Ok(payload) = serde_json::from_str::<EventPayload>(payload_json) else {
        if debug {
            eprintln!("[bitloops telemetry] failed to parse event payload");
        }
        return;
    };

    if payload.event.is_empty() || payload.distinct_id.is_empty() {
        if debug {
            eprintln!(
                "[bitloops telemetry] skipped: empty event or distinct_id (event={}, distinct_id={})",
                payload.event, payload.distinct_id
            );
        }
        return;
    }

    let api_key = posthog_api_key();
    if api_key.is_empty() {
        if debug {
            eprintln!("[bitloops telemetry] skipped: no API key configured");
        }
        return;
    }

    let endpoint = posthog_endpoint();
    if endpoint.is_empty() {
        if debug {
            eprintln!("[bitloops telemetry] skipped: no endpoint configured");
        }
        return;
    }

    let mut properties = payload.properties.clone();
    properties
        .entry("$geoip_disable".to_string())
        .or_insert(Value::Bool(true));

    let capture = json!({
        "api_key": api_key,
        "event": payload.event,
        "distinct_id": payload.distinct_id,
        "properties": properties,
        "timestamp": payload.timestamp,
    });

    let Ok(body) = serde_json::to_string(&capture) else {
        if debug {
            eprintln!("[bitloops telemetry] failed to serialize capture request");
        }
        return;
    };

    let capture_url = format!("{}/capture/", endpoint.trim_end_matches('/'));

    if debug {
        eprintln!(
            "[bitloops telemetry] sending event '{}' to {}",
            payload.event, capture_url
        );
        eprintln!("[bitloops telemetry] payload: {}", body);
    }

    // Best effort and silent.
    match Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--max-time")
        .arg("5")
        .arg("--header")
        .arg("Content-Type: application/json")
        .arg("--request")
        .arg("POST")
        .arg("--data-raw")
        .arg(&body)
        .arg(&capture_url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => {
            let status = output.status;
            if debug {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if status.success() {
                    eprintln!("[bitloops telemetry] HTTP success: {}", status);
                    if !stdout.trim().is_empty() {
                        eprintln!("[bitloops telemetry] response: {}", stdout);
                    }
                } else {
                    eprintln!("[bitloops telemetry] HTTP failure: {}", status);
                    if !stdout.trim().is_empty() {
                        eprintln!("[bitloops telemetry] response body: {}", stdout);
                    }
                    if !stderr.trim().is_empty() {
                        eprintln!("[bitloops telemetry] curl error: {}", stderr);
                    }
                }
            }
        }
        Err(e) => {
            if debug {
                eprintln!("[bitloops telemetry] failed to spawn curl: {}", e);
            }
        }
    }
}

pub fn collect_flag_names_from_argv(args: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for token in args {
        if token == "--" {
            break;
        }

        if let Some(long) = token.strip_prefix("--") {
            if long.is_empty() {
                continue;
            }

            let (name, _) = long.split_once('=').unwrap_or((long, ""));
            if !name.is_empty() && seen.insert(name.to_string()) {
                out.push(name.to_string());
            }
            continue;
        }

        if token.starts_with('-') && token.len() > 1 {
            for short in token[1..].chars() {
                let flag = short.to_string();
                if seen.insert(flag.clone()) {
                    out.push(flag);
                }
            }
        }
    }

    out
}

fn detect_installed_agents(repo_root: &Path) -> Vec<String> {
    let mut installed = Vec::new();

    if repo_root.join(".claude").is_dir() {
        installed.push("claude-code".to_string());
    }
    if repo_root
        .join(".github")
        .join("hooks")
        .join("bitloops.json")
        .is_file()
    {
        installed.push("copilot".to_string());
    }
    if repo_root.join(".codex").is_dir() {
        installed.push("codex".to_string());
    }
    if repo_root.join(".gemini").is_dir() {
        installed.push("gemini".to_string());
    }
    if repo_root.join(".cursor").is_dir() {
        installed.push("cursor".to_string());
    }
    if repo_root.join(".opencode").is_dir() {
        installed.push("opencode".to_string());
    }

    installed
}

fn posthog_api_key() -> String {
    POSTHOG_API_KEY.to_string()
}

fn posthog_endpoint() -> String {
    env::var("BITLOOPS_POSTHOG_ENDPOINT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| POSTHOG_ENDPOINT.to_string())
}

fn debug_enabled() -> bool {
    env::var("BITLOOPS_TELEMETRY_DEBUG")
        .ok()
        .is_some_and(|v| !v.trim().is_empty())
}

fn distinct_machine_id() -> Option<String> {
    if env::var("BITLOOPS_TELEMETRY_FORCE_NO_DISTINCT_ID").is_ok_and(|v| !v.is_empty()) {
        return None;
    }

    if let Ok(value) = env::var("BITLOOPS_TELEMETRY_DISTINCT_ID")
        && !value.trim().is_empty()
    {
        return Some(value);
    }

    let raw = read_machine_id()
        .or_else(read_macos_platform_uuid)
        .or_else(read_host_fallback)?;
    Some(sha256_hex(&format!("{DISTINCT_ID_NAMESPACE}:{raw}")))
}

fn read_machine_id() -> Option<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn read_macos_platform_uuid() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("ioreg")
            .arg("-rd1")
            .arg("-c")
            .arg("IOPlatformExpertDevice")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let marker = "IOPlatformUUID";
            if !line.contains(marker) {
                continue;
            }
            let (_, rest) = line.split_once('=')?;
            let value = rest.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn read_host_fallback() -> Option<String> {
    let hostname = env::var("HOSTNAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            env::var("COMPUTERNAME")
                .ok()
                .filter(|v| !v.trim().is_empty())
        });

    let username = env::var("USER")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| env::var("USERNAME").ok().filter(|v| !v.trim().is_empty()));

    if hostname.is_none() && username.is_none() {
        return None;
    }

    Some(format!(
        "{}:{}:{}:{}",
        hostname.unwrap_or_default(),
        username.unwrap_or_default(),
        env::consts::OS,
        env::consts::ARCH
    ))
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    hex_encode(&digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(unix)]
fn spawn_detached_analytics(payload_json: &str) {
    use std::os::unix::process::CommandExt;

    let Ok(executable) = env::current_exe() else {
        return;
    };

    let debug = debug_enabled();
    let mut cmd = Command::new(executable);
    cmd.arg("__send_analytics")
        .arg(payload_json)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null());

    if debug {
        // Preserve stderr so debug logs are visible
        cmd.stderr(Stdio::inherit());
    } else {
        cmd.stderr(Stdio::null());
    }

    cmd.process_group(0);

    let _ = cmd.spawn();
}

#[cfg(windows)]
fn spawn_detached_analytics(payload_json: &str) {
    use std::os::windows::process::CommandExt;

    let Ok(executable) = env::current_exe() else {
        return;
    };

    // CREATE_NO_WINDOW (0x08000000) - run without creating a console window
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let debug = debug_enabled();
    let mut cmd = Command::new(executable);
    cmd.arg("__send_analytics")
        .arg(payload_json)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null());

    if debug {
        cmd.stderr(Stdio::inherit());
    } else {
        cmd.stderr(Stdio::null());
    }

    let _ = cmd.spawn();
}

#[cfg(all(not(unix), not(windows)))]
fn spawn_detached_analytics(_payload_json: &str) {
    // No detached analytics transport is implemented for this target yet.
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_secs(secs)
}

fn rfc3339_from_secs(secs: u64) -> String {
    let (year, month, day, hour, minute, second) = unix_to_ymdhms(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn unix_to_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    const SECS_PER_DAY: u64 = 86_400;
    let mut days = secs / SECS_PER_DAY;
    let mut rem = secs % SECS_PER_DAY;

    let hour = rem / 3600;
    rem %= 3600;
    let minute = rem / 60;
    let second = rem % 60;

    let mut year = 1970_u64;
    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if days < year_days {
            break;
        }
        days -= year_days;
        year += 1;
    }

    let month_lengths = if is_leap(year) {
        [31_u64, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31_u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1_u64;
    for ml in month_lengths {
        if days < ml {
            break;
        }
        days -= ml;
        month += 1;
    }

    let day = days + 1;
    (year, month, day, hour, minute, second)
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod analytics_test;
