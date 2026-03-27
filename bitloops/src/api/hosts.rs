//! OS hosts file: ensure `bitloops.local` resolves to loopback for the default dashboard path.

use anyhow::Result;
use std::fs;
use std::io;
use std::io::Write as _;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const BITLOOPS_HOST: &str = "bitloops.local";

/// Result of attempting to make `bitloops.local` usable for the default dashboard bind.
#[derive(Debug)]
pub enum HostMappingOutcome {
    /// Name already resolves to a loopback address.
    AlreadyCorrect,
    /// Hosts file was updated successfully.
    Updated,
    /// Cannot apply mapping; caller should fall back to `localhost`.
    NeedsFallback { reason: String },
}

fn hosts_file_path() -> PathBuf {
    #[cfg(windows)]
    {
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        PathBuf::from(windir)
            .join("System32")
            .join("drivers")
            .join("etc")
            .join("hosts")
    }
    #[cfg(not(windows))]
    PathBuf::from("/etc/hosts")
}

fn bitloops_resolves_to_loopback() -> bool {
    let Ok(addrs) = format!("{BITLOOPS_HOST}:80").to_socket_addrs() else {
        return false;
    };
    addrs.into_iter().any(|a| a.ip().is_loopback())
}

fn hosts_line_for_bitloops() -> String {
    format!("127.0.0.1\t{BITLOOPS_HOST}\n::1\t{BITLOOPS_HOST}\n")
}

/// Remove non-comment lines that mention `bitloops.local`, then append a managed mapping line.
fn merge_hosts_content(existing: &str) -> String {
    let mut out = String::new();
    for line in existing.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if line.to_ascii_lowercase().contains(BITLOOPS_HOST) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out.push_str("# bitloops dashboard (managed)\n");
    out.push_str(&hosts_line_for_bitloops());
    out
}

fn write_hosts_file_atomic(path: &Path, content: &str) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("hosts file has no parent directory: {}", path.display()),
        )
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("hosts file has no file name: {}", path.display()),
        )
    })?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp_name = format!(
        ".{}.bitloops.tmp.{}.{}",
        file_name.to_string_lossy(),
        std::process::id(),
        nanos
    );
    let temp_path = parent.join(temp_name);

    let mut temp_file = fs::File::create(&temp_path)?;
    temp_file.write_all(content.as_bytes())?;
    temp_file.sync_all()?;

    if let Ok(metadata) = fs::metadata(path) {
        let _ = fs::set_permissions(&temp_path, metadata.permissions());
    }

    let rename_result = fs::rename(&temp_path, path);
    if rename_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    rename_result
}

/// Run **before** host probing for `bitloops.local` when using the default dashboard (no explicit `--host`).
pub fn ensure_default_dashboard_host_mapping() -> Result<HostMappingOutcome> {
    if bitloops_resolves_to_loopback() {
        return Ok(HostMappingOutcome::AlreadyCorrect);
    }

    let path = hosts_file_path();
    let existing = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return Ok(HostMappingOutcome::NeedsFallback {
                reason: format!("cannot read hosts file {}: {e}", path.display()),
            });
        }
    };

    let merged = merge_hosts_content(&existing);

    match write_hosts_file_atomic(&path, &merged) {
        Ok(()) => {
            if bitloops_resolves_to_loopback() {
                Ok(HostMappingOutcome::Updated)
            } else {
                Ok(HostMappingOutcome::NeedsFallback {
                    reason: format!(
                        "updated {} but {BITLOOPS_HOST} still does not resolve to loopback",
                        path.display()
                    ),
                })
            }
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            Ok(HostMappingOutcome::NeedsFallback {
                reason: format!(
                    "permission denied writing hosts file {}: {e}",
                    path.display()
                ),
            })
        }
        Err(e) => Ok(HostMappingOutcome::NeedsFallback {
            reason: format!("cannot write hosts file {}: {e}", path.display()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::merge_hosts_content;

    #[test]
    fn merge_hosts_content_appends_dual_loopback_entries() {
        let merged = merge_hosts_content("127.0.0.1 localhost\n");
        assert!(merged.contains("# bitloops dashboard (managed)\n"));
        assert!(merged.contains("127.0.0.1\tbitloops.local\n"));
        assert!(merged.contains("::1\tbitloops.local\n"));
    }

    #[test]
    fn merge_hosts_content_replaces_existing_bitloops_lines() {
        let existing = "\
127.0.0.1 localhost
127.0.0.1 bitloops.local
::1 BITLOOPS.LOCAL
# bitloops.local in comment should remain
";
        let merged = merge_hosts_content(existing);

        assert!(merged.contains("127.0.0.1 localhost\n"));
        assert!(merged.contains("# bitloops.local in comment should remain\n"));
        assert_eq!(merged.matches("127.0.0.1\tbitloops.local\n").count(), 1);
        assert_eq!(merged.matches("::1\tbitloops.local\n").count(), 1);
    }
}
