//! Version check logic.
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const GLOBAL_CONFIG_DIR_NAME: &str = ".config/bitloops";
pub const CACHE_FILE_NAME: &str = "version_check.json";
pub const DEFAULT_GITHUB_API_URL: &str =
    "https://storage.googleapis.com/wwwbitloopscom/cli/latest.json";
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCache {
    #[serde(rename = "last_check_time", alias = "last_check_time_secs")]
    pub last_check_time_secs: u64,
}

fn config_dir_override() -> &'static Mutex<Option<PathBuf>> {
    static OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

fn github_api_url_override() -> &'static Mutex<Option<String>> {
    static OVERRIDE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn set_config_dir_override_for_tests(path: Option<PathBuf>) {
    *config_dir_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = path;
}

#[cfg(test)]
fn set_github_api_url_for_tests(url: Option<String>) {
    *github_api_url_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = url;
}

fn github_api_url() -> String {
    github_api_url_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .unwrap_or_else(|| DEFAULT_GITHUB_API_URL.to_string())
}

pub fn global_config_dir_path() -> io::Result<PathBuf> {
    if let Some(path) = config_dir_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    {
        return Ok(path);
    }

    let home = std::env::var_os("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(GLOBAL_CONFIG_DIR_NAME))
}

pub fn ensure_global_config_dir() -> io::Result<()> {
    let config_dir = global_config_dir_path()?;
    fs::create_dir_all(&config_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

pub fn cache_file_path() -> io::Result<PathBuf> {
    Ok(global_config_dir_path()?.join(CACHE_FILE_NAME))
}

pub fn load_cache() -> io::Result<VersionCache> {
    let file_path = cache_file_path()?;
    let data = fs::read_to_string(&file_path)
        .map_err(|e| io::Error::new(e.kind(), format!("reading cache file: {e}")))?;
    serde_json::from_str::<VersionCache>(&data)
        .map_err(|e| io::Error::other(format!("parsing cache: {e}")))
}

pub fn save_cache(cache: &VersionCache) -> io::Result<()> {
    let file_path = cache_file_path()?;
    let parent = file_path.parent().ok_or_else(|| {
        io::Error::other(format!(
            "cache file path has no parent: {}",
            file_path.display()
        ))
    })?;

    let data = serde_json::to_string_pretty(cache)
        .map_err(|e| io::Error::other(format!("marshaling cache: {e}")))?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_path = parent.join(format!(
        ".version_check_tmp_{}_{}",
        std::process::id(),
        nanos
    ));

    if let Err(e) = fs::write(&temp_path, data.as_bytes()) {
        let _ = fs::remove_file(&temp_path);
        return Err(io::Error::new(e.kind(), format!("writing cache: {e}")));
    }

    if let Err(e) = fs::rename(&temp_path, &file_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(io::Error::new(
            e.kind(),
            format!("renaming cache file: {e}"),
        ));
    }

    Ok(())
}

/// Returns true when `latest` should be considered newer than `current`.
///
/// This is intentionally a placeholder to enable TDD red tests first.
pub fn is_outdated(current: &str, latest: &str) -> bool {
    let Some(current_parsed) = ParsedVersion::parse(current) else {
        return false;
    };
    let Some(latest_parsed) = ParsedVersion::parse(latest) else {
        return false;
    };

    current_parsed < latest_parsed
}

pub fn parse_github_release(body: &[u8]) -> Result<String, String> {
    let payload: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("parsing JSON: {e}"))?;

    if payload
        .get("prerelease")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err("only prerelease versions available".to_string());
    }

    for key in ["tag_name", "version", "latest"] {
        if let Some(version) = payload
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(version.to_string());
        }
    }

    Err("empty tag name".to_string())
}

pub fn fetch_latest_version() -> Result<String, String> {
    let url = github_api_url();

    let output = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--max-time")
        .arg(HTTP_TIMEOUT.as_secs().to_string())
        .arg("--header")
        .arg("Accept: application/vnd.github+json")
        .arg("--header")
        .arg("User-Agent: bitloops-cli")
        .arg("--write-out")
        .arg("\n%{http_code}")
        .arg(url)
        .output()
        .map_err(|e| format!("fetching release info: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "fetching release info: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let Some(status_sep) = output.stdout.iter().rposition(|&b| b == b'\n') else {
        return Err("reading response: missing status code".to_string());
    };

    let status = std::str::from_utf8(&output.stdout[status_sep + 1..])
        .map_err(|e| format!("reading response status: {e}"))?
        .trim()
        .parse::<u16>()
        .map_err(|e| format!("parsing response status: {e}"))?;

    if status != 200 {
        return Err(format!("unexpected status code: {status}"));
    }

    let body = &output.stdout[..status_sep];
    if body.len() > (1 << 20) {
        return Err("reading response: body exceeds 1MB limit".to_string());
    }

    parse_github_release(body).map_err(|e| format!("parsing release: {e}"))
}

pub fn update_command() -> String {
    let fallback = "curl -fsSL https://bitloops.io/install.sh | bash".to_string();
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return fallback,
    };
    let resolved = fs::canonicalize(&exe).unwrap_or(exe);
    let resolved = resolved.to_string_lossy();

    if resolved.contains("/Cellar/") || resolved.contains("/homebrew/") {
        return "brew upgrade bitloops".to_string();
    }

    fallback
}

pub fn check_and_notify(w: &mut dyn Write, current_version: &str) {
    if current_version == "dev" || current_version.is_empty() {
        return;
    }

    if ensure_global_config_dir().is_err() {
        return;
    }

    let mut cache = load_cache().unwrap_or(VersionCache {
        last_check_time_secs: 0,
    });

    let now_secs = current_time_secs();
    if now_secs.saturating_sub(cache.last_check_time_secs) < CHECK_INTERVAL.as_secs() {
        return;
    }

    let latest = fetch_latest_version();

    cache.last_check_time_secs = now_secs;
    let _ = save_cache(&cache);

    let Ok(latest_version) = latest else {
        return;
    };

    if is_outdated(current_version, &latest_version) {
        print_notification(w, current_version, &latest_version);
    }
}

fn print_notification(w: &mut dyn Write, current: &str, latest: &str) {
    let _ = write!(
        w,
        "\nA newer version of Bitloops CLI is available: {latest} (current: {current})\nRun '{}' to update.\n",
        update_command()
    );
}

fn current_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ParsedVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<Vec<Identifier>>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum Identifier {
    Numeric(u64),
    AlphaNumeric(String),
}

impl ParsedVersion {
    fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
        let core = raw.split('+').next()?;
        let (base, prerelease) = match core.split_once('-') {
            Some((b, p)) => (b, Some(p)),
            None => (core, None),
        };

        let mut parts = base.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next()?.parse::<u64>().ok()?;
        let patch = parts.next()?.parse::<u64>().ok()?;
        if parts.next().is_some() {
            return None;
        }

        let prerelease = match prerelease {
            Some("") => return None,
            Some(value) => {
                let mut out = Vec::new();
                for piece in value.split('.') {
                    if piece.is_empty() {
                        return None;
                    }
                    if piece.chars().all(|c| c.is_ascii_digit()) {
                        out.push(Identifier::Numeric(piece.parse::<u64>().ok()?));
                    } else {
                        out.push(Identifier::AlphaNumeric(piece.to_string()));
                    }
                }
                Some(out)
            }
            None => None,
        };

        Some(Self {
            major,
            minor,
            patch,
            prerelease,
        })
    }
}

impl Ord for ParsedVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        let base_cmp =
            (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch));
        if base_cmp != Ordering::Equal {
            return base_cmp;
        }

        match (&self.prerelease, &other.prerelease) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(lhs), Some(rhs)) => {
                for (l, r) in lhs.iter().zip(rhs.iter()) {
                    let cmp = match (l, r) {
                        (Identifier::Numeric(a), Identifier::Numeric(b)) => a.cmp(b),
                        (Identifier::Numeric(_), Identifier::AlphaNumeric(_)) => Ordering::Less,
                        (Identifier::AlphaNumeric(_), Identifier::Numeric(_)) => Ordering::Greater,
                        (Identifier::AlphaNumeric(a), Identifier::AlphaNumeric(b)) => a.cmp(b),
                    };
                    if cmp != Ordering::Equal {
                        return cmp;
                    }
                }
                lhs.len().cmp(&rhs.len())
            }
        }
    }
}

impl PartialOrd for ParsedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
#[path = "versioncheck_tests.rs"]
mod tests;
