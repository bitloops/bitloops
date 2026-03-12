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

/// PostHog API key. Set at build time via BITLOOPS_POSTHOG_API_KEY (e.g. in GitHub Actions from secrets)
/// or at runtime. If unset, no events are sent.
pub const POSTHOG_API_KEY: &str = "phc_development_key";

/// PostHog base URL; the REST capture endpoint is derived as `{POSTHOG_ENDPOINT}/capture/`.
pub const POSTHOG_ENDPOINT: &str = "https://eu.i.posthog.com";

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
pub struct CommandInfo {
    pub command_path: String,
    pub hidden: bool,
    pub flag_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TelemetryDispatchContext {
    pub strategy: String,
    pub agent: String,
    pub is_bitloops_enabled: bool,
    pub version: String,
}

#[derive(Args)]
pub struct SendAnalyticsArgs {
    pub payload: String,
}

pub fn load_dispatch_context() -> Option<TelemetryDispatchContext> {
    let repo_root = crate::engine::paths::repo_root().ok()?;
    let settings = crate::engine::settings::load_settings(&repo_root).ok()?;

    if settings.telemetry != Some(true) {
        return None;
    }

    Some(TelemetryDispatchContext {
        strategy: settings.strategy,
        agent: detect_installed_agents(&repo_root).join(","),
        is_bitloops_enabled: settings.enabled,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

pub fn build_event_payload(
    cmd: Option<&CommandInfo>,
    strategy: &str,
    agent: &str,
    is_bitloops_enabled: bool,
    version: &str,
) -> Option<EventPayload> {
    let command = cmd?;

    let machine_id = distinct_machine_id()?;
    let selected_agent = if agent.is_empty() { "auto" } else { agent };

    let mut properties = HashMap::from([
        (
            "command".to_string(),
            Value::String(command.command_path.clone()),
        ),
        ("strategy".to_string(), Value::String(strategy.to_string())),
        (
            "agent".to_string(),
            Value::String(selected_agent.to_string()),
        ),
        (
            "isBitloopsEnabled".to_string(),
            Value::Bool(is_bitloops_enabled),
        ),
        (
            "cli_version".to_string(),
            Value::String(version.to_string()),
        ),
        ("os".to_string(), Value::String(env::consts::OS.to_string())),
        (
            "arch".to_string(),
            Value::String(env::consts::ARCH.to_string()),
        ),
    ]);

    if !command.flag_names.is_empty() {
        properties.insert(
            "flags".to_string(),
            Value::String(command.flag_names.join(",")),
        );
    }

    Some(EventPayload {
        event: "cli_command_executed".to_string(),
        distinct_id: machine_id,
        properties,
        timestamp: now_rfc3339(),
    })
}

pub fn track_command_detached(
    cmd: Option<&CommandInfo>,
    strategy: &str,
    agent: &str,
    is_bitloops_enabled: bool,
    version: &str,
) {
    if env::var(TELEMETRY_OPTOUT_ENV).is_ok_and(|v| !v.is_empty()) {
        return;
    }

    let Some(command) = cmd else {
        return;
    };

    if command.hidden {
        return;
    }

    let Some(payload) =
        build_event_payload(Some(command), strategy, agent, is_bitloops_enabled, version)
    else {
        return;
    };

    if let Ok(payload_json) = serde_json::to_string(&payload) {
        spawn_detached_analytics(&payload_json);
    }
}

pub fn send_event(payload_json: &str) {
    let Ok(payload) = serde_json::from_str::<EventPayload>(payload_json) else {
        return;
    };

    if payload.event.is_empty() || payload.distinct_id.is_empty() {
        return;
    }

    let api_key = posthog_api_key();
    if api_key.is_empty() {
        return;
    }

    let endpoint = posthog_endpoint();
    if endpoint.is_empty() {
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
        return;
    };

    let capture_url = format!("{}/capture/", endpoint.trim_end_matches('/'));

    // Best effort and silent.
    let _ = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--max-time")
        .arg("2")
        .arg("--header")
        .arg("Content-Type: application/json")
        .arg("--request")
        .arg("POST")
        .arg("--data-raw")
        .arg(&body)
        .arg(&capture_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
    if repo_root.join(".github").join("hooks").join("bitloops.json").is_file() {
        installed.push("copilot".to_string());
    }
    if repo_root.join(".gemini").is_dir() {
        installed.push("gemini-cli".to_string());
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
    env::var("BITLOOPS_POSTHOG_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            option_env!("BITLOOPS_POSTHOG_API_KEY")
                .map(String::from)
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| POSTHOG_API_KEY.to_string())
}

fn posthog_endpoint() -> String {
    env::var("BITLOOPS_POSTHOG_ENDPOINT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| POSTHOG_ENDPOINT.to_string())
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

    let mut cmd = Command::new(executable);
    cmd.arg("__send_analytics")
        .arg(payload_json)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);

    let _ = cmd.spawn();
}

#[cfg(not(unix))]
fn spawn_detached_analytics(_payload_json: &str) {
    // No-op on non-Unix.
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
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
mod detached_test;
