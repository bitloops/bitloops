use super::{DEFAULT_BUNDLE_RELATIVE_DIR, DashboardTransport};
#[cfg(test)]
use super::{FALLBACK_LOCAL_HOST, PREFERRED_LOCAL_HOST};
use anyhow::{Context, Result, anyhow};
use std::env;
use std::ffi::OsStr;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};

pub(super) fn resolve_bundle_file(bundle_dir: &Path, request_path: &str) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    let trimmed = request_path.trim_start_matches('/');

    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(segment) => relative.push(segment),
            Component::CurDir => {}
            Component::RootDir | Component::ParentDir | Component::Prefix(_) => return None,
        }
    }

    let mut candidate = if relative.as_os_str().is_empty() {
        bundle_dir.join("index.html")
    } else {
        bundle_dir.join(relative)
    };

    if candidate.is_dir() {
        candidate = candidate.join("index.html");
    }

    Some(candidate)
}

pub(super) fn content_type_for_path(path: &Path) -> &'static str {
    let Some(extension) = path.extension().and_then(OsStr::to_str) else {
        return "application/octet-stream";
    };

    match extension.to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "map" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub(super) fn resolve_bundle_dir(bundle_arg: Option<&Path>) -> PathBuf {
    bundle_arg
        .map(expand_tilde)
        .unwrap_or_else(default_bundle_dir)
}

fn default_bundle_dir() -> PathBuf {
    let home = env::var_os("HOME").map(PathBuf::from);
    default_bundle_dir_from_home(home.as_deref())
}

pub(super) fn default_bundle_dir_from_home(home: Option<&Path>) -> PathBuf {
    if let Some(home) = home {
        home.join(DEFAULT_BUNDLE_RELATIVE_DIR)
    } else {
        PathBuf::from(DEFAULT_BUNDLE_RELATIVE_DIR)
    }
}

pub(super) fn has_bundle_index(bundle_dir: &Path) -> bool {
    bundle_dir.join("index.html").is_file()
}

fn expand_tilde(path: &Path) -> PathBuf {
    let home = env::var_os("HOME").map(PathBuf::from);
    expand_tilde_with_home(path, home.as_deref())
}

pub(super) fn expand_tilde_with_home(path: &Path, home: Option<&Path>) -> PathBuf {
    let rendered = path.to_string_lossy();
    if rendered == "~" {
        return home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(suffix) = rendered.strip_prefix("~/")
        && let Some(home) = home
    {
        return home.join(suffix);
    }
    path.to_path_buf()
}

#[cfg(test)]
pub(super) fn select_host_with_dashboard_preference(
    explicit_host: Option<&str>,
    use_bitloops_local: bool,
) -> String {
    if let Some(host) = explicit_host.and_then(normalized_host) {
        return host.to_string();
    }

    if use_bitloops_local {
        PREFERRED_LOCAL_HOST.to_string()
    } else {
        FALLBACK_LOCAL_HOST.to_string()
    }
}

pub(super) fn normalized_host(input: &str) -> Option<&str> {
    let host = input.trim();
    if host.is_empty() { None } else { Some(host) }
}

pub(super) fn resolve_bind_addr(host: &str, port: u16) -> Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("Resolving host {host}:{port}"))?
        .collect();

    if addrs.is_empty() {
        return Err(anyhow!("Resolved no addresses for {host}:{port}"));
    }

    if let Some(addr) = addrs
        .iter()
        .copied()
        .find(|addr| addr.ip().is_loopback() || addr.ip().is_unspecified())
    {
        return Ok(addr);
    }

    Ok(addrs[0])
}

pub(super) fn browser_host_for_url(bind_host: &str, local_addr: SocketAddr) -> String {
    match bind_host {
        "0.0.0.0" => "127.0.0.1".to_string(),
        "::" | "[::]" => "localhost".to_string(),
        _ if local_addr.ip().is_unspecified() => {
            if local_addr.is_ipv4() {
                "127.0.0.1".to_string()
            } else {
                "localhost".to_string()
            }
        }
        _ => bind_host.to_string(),
    }
}

fn dashboard_scheme(transport: DashboardTransport) -> &'static str {
    match transport {
        DashboardTransport::Http => "http",
        DashboardTransport::Https => "https",
    }
}

pub(super) fn format_dashboard_url(transport: DashboardTransport, host: &str, port: u16) -> String {
    let scheme = dashboard_scheme(transport);
    if host.contains(':') && !host.starts_with('[') {
        format!("{scheme}://[{host}]:{port}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}
