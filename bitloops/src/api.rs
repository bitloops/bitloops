mod bundle;
mod bundle_types;
mod db;
mod dto;
mod handlers;
mod hosts;
mod router;
pub mod tls;

use crate::config::dashboard_use_bitloops_local;
use crate::host::checkpoints::strategy::manual_commit::{
    CommittedInfo, list_committed, read_commit_checkpoint_mappings, read_committed_info, run_git,
};
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, bitloops_wordmark, color_hex};
use crate::utils::paths;
use anyhow::{Context, Result, anyhow, bail};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::io::IsTerminal;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

pub const DEFAULT_DASHBOARD_PORT: u16 = 5667;

const PREFERRED_LOCAL_HOST: &str = "bitloops.local";
const FALLBACK_LOCAL_HOST: &str = "127.0.0.1";
const DEFAULT_BUNDLE_RELATIVE_DIR: &str = ".bitloops/dashboard/bundle";
pub(super) const API_GIT_SCAN_LIMIT: usize = 5_000;
pub(super) const API_DEFAULT_PAGE_LIMIT: usize = 100;
const API_MAX_PAGE_LIMIT: usize = 500;
pub(super) const GIT_FIELD_SEPARATOR: char = '\u{1f}';
pub(super) const GIT_RECORD_SEPARATOR: char = '\u{1e}';
pub(super) const DASHBOARD_FALLBACK_INSTALL_HTML: &str =
    include_str!("api/dashboard_fallback_install.html");

#[derive(Debug, Clone)]
pub struct DashboardServerConfig {
    pub host: Option<String>,
    pub port: u16,
    pub no_open: bool,
    pub bundle_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct DashboardCommitNode {
    pub(super) sha: String,
    pub(super) parents: Vec<String>,
    pub(super) author_name: String,
    pub(super) author_email: String,
    pub(super) timestamp: i64,
    pub(super) message: String,
    pub(super) checkpoint_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct DashboardUser {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) email: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ApiPage {
    pub(super) limit: usize,
    pub(super) offset: usize,
}

impl Default for ApiPage {
    fn default() -> Self {
        Self {
            limit: API_DEFAULT_PAGE_LIMIT,
            offset: 0,
        }
    }
}

impl ApiPage {
    pub(super) fn normalized(self) -> Self {
        let mut limit = self.limit;
        if limit == 0 {
            limit = API_DEFAULT_PAGE_LIMIT;
        }
        if limit > API_MAX_PAGE_LIMIT {
            limit = API_MAX_PAGE_LIMIT;
        }
        Self {
            limit,
            offset: self.offset,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct CommitCheckpointPair {
    pub(super) commit: DashboardCommitNode,
    pub(super) user: DashboardUser,
    pub(super) checkpoint: CommittedInfo,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct CommitCheckpointQuery {
    pub(super) branch: String,
    pub(super) from_unix: Option<i64>,
    pub(super) to_unix: Option<i64>,
    pub(super) user: Option<String>,
    pub(super) agent: Option<String>,
    pub(super) page: ApiPage,
}

#[derive(Debug, Clone)]
pub(super) enum ServeMode {
    HelloWorld,
    Bundle(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardTransport {
    Http,
    Https,
}

#[derive(Debug, Clone)]
pub(super) struct DashboardState {
    pub(super) repo_root: PathBuf,
    pub(super) mode: ServeMode,
    pub(super) db: db::DashboardDbPools,
    pub(super) bundle_dir: PathBuf,
}

/// True when BITLOOPS_DEV is set (show extra info on CLI).
fn is_dev_mode() -> bool {
    env::var("BITLOOPS_DEV").is_ok()
}

pub async fn run(config: DashboardServerConfig) -> Result<()> {
    let db_init = db::init_dashboard_db().await;
    if db_init.startup_health.has_failures() {
        bail!(
            "dashboard database startup health check failed; run `bitloops --connection-status` for details"
        );
    }

    let mut startup_warnings: Vec<String> = Vec::new();

    let selected_host = if let Some(explicit_host) = config.host.as_deref().and_then(normalized_host) {
        explicit_host.to_string()
    } else if dashboard_use_bitloops_local() {
        match hosts::ensure_default_dashboard_host_mapping()? {
            hosts::HostMappingOutcome::AlreadyCorrect | hosts::HostMappingOutcome::Updated => {
                PREFERRED_LOCAL_HOST.to_string()
            }
            hosts::HostMappingOutcome::NeedsFallback { reason } => {
                startup_warnings.push(format!(
                    "Warning: could not map {PREFERRED_LOCAL_HOST} in the hosts file: {reason}\n\
                     Falling back to localhost for this run."
                ));
                FALLBACK_LOCAL_HOST.to_string()
            }
        }
    } else {
        FALLBACK_LOCAL_HOST.to_string()
    };
    let bind_addr = resolve_bind_addr(&selected_host, config.port)?;

    let listener = TcpListener::bind(bind_addr).await.with_context(|| {
        format!(
            "Binding dashboard server to {selected_host}:{}",
            config.port
        )
    })?;
    let local_addr = listener
        .local_addr()
        .context("Reading dashboard listener address")?;
    let browser_host = browser_host_for_url(&selected_host, local_addr);
    let (transport, tls_acceptor) = if !tls::mkcert_on_path() {
        startup_warnings.push(format!(
            "Warning: `mkcert` is not on PATH. Falling back to local HTTP.\n\
             See https://bitloops.com/docs/guides/dashboard-local-https-setup for mkcert and /etc/hosts setup instructions."
        ));
        (DashboardTransport::Http, None)
    } else {
        match tls::ensure_dashboard_tls_material(&browser_host) {
            Ok(tls_material) => {
                log::debug!(
                    "Dashboard TLS: cert={} key={}",
                    tls_material.cert_path.display(),
                    tls_material.key_path.display()
                );
                (
                    DashboardTransport::Https,
                    Some(TlsAcceptor::from(tls_material.server_config.clone())),
                )
            }
            Err(err) => {
                startup_warnings.push(format!(
                    "Warning: Dashboard HTTPS setup failed ({err:#}).\n\
                     Falling back to local HTTP.\n\
                     See https://bitloops.com/docs/guides/dashboard-local-https-setup for mkcert and /etc/hosts setup instructions."
                ));
                (DashboardTransport::Http, None)
            }
        }
    };

    let bundle_dir = resolve_bundle_dir(config.bundle_dir.as_deref());
    let serve_mode = if has_bundle_index(&bundle_dir) {
        ServeMode::Bundle(bundle_dir.clone())
    } else {
        ServeMode::HelloWorld
    };
    let repo_root = paths::repo_root()
        .or_else(|_| env::current_dir().context("Getting current directory for dashboard state"))
        .unwrap_or_else(|_| PathBuf::from("."));

    let url = format_dashboard_url(transport, &browser_host, local_addr.port());

    println!();
    println!("{}", color_hex(&bitloops_wordmark(), BITLOOPS_PURPLE_HEX));
    println!();
    print!("📊 Dashboard ");
    print!("{}", color_hex("ready ", "#22c55e"));
    print!("at ");
    println!("{}", color_hex(&clickable_url(&url), BITLOOPS_PURPLE_HEX));
    if !startup_warnings.is_empty() {
        eprintln!();
    }
    for warning in &startup_warnings {
        print_warning_message(warning);
    }
    match &serve_mode {
        ServeMode::HelloWorld => {
            println!(
                "Bundle not found. Bundle expected at {}",
                bundle_dir.display()
            );
        }
        ServeMode::Bundle(path) => {
            log::debug!("Serving dashboard bundle from {}", path.display());
            if is_dev_mode() {
                println!("Serving dashboard bundle from {}", path.display());
            }
            println!();
            println!("To exit, press Ctrl+C");
        }
    }

    if !config.no_open
        && let Err(err) = open_in_default_browser(&url)
    {
        print_warning_message(&format!("Warning: failed to open default browser: {err:#}"));
    }

    let state = DashboardState {
        repo_root,
        mode: serve_mode,
        db: db_init.pools,
        bundle_dir,
    };

    match (transport, tls_acceptor) {
        (DashboardTransport::Https, Some(acceptor)) => {
            serve_until_ctrl_c_tls(listener, acceptor, state).await
        }
        (DashboardTransport::Http, _) => serve_until_ctrl_c_http(listener, state).await,
        (DashboardTransport::Https, None) => {
            Err(anyhow!("dashboard HTTPS selected without a TLS acceptor"))
        }
    }
}

async fn serve_until_ctrl_c_tls(
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
    state: DashboardState,
) -> Result<()> {
    use hyper::server::conn::http1::Builder as Http1Builder;
    use hyper_util::rt::TokioIo;
    use hyper_util::service::TowerToHyperService;

    let app = router::build_dashboard_router(state);
    serve_until_ctrl_c_with_handler(listener, move |stream| {
        let tls_acceptor = tls_acceptor.clone();
        let app = app.clone();
        async move {
            let tls_stream = match tls_acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => {
                    let detail = e.to_string();
                    if detail.contains("CertificateUnknown")
                        || detail.contains("UnknownIssuer")
                    {
                        log::warn!(
                            "TLS handshake failed: {e} \
                             (the client rejected the certificate — run `mkcert -install` once, quit Chrome fully, retry)"
                        );
                    } else {
                        log::warn!("TLS handshake failed: {e}");
                    }
                    return;
                }
            };
            let io = TokioIo::new(tls_stream);
            let service = TowerToHyperService::new(app);
            if let Err(e) = Http1Builder::new().serve_connection(io, service).await {
                log::warn!("HTTP connection error: {e}");
            }
        }
    })
    .await
}

async fn serve_until_ctrl_c_http(listener: TcpListener, state: DashboardState) -> Result<()> {
    use hyper::server::conn::http1::Builder as Http1Builder;
    use hyper_util::rt::TokioIo;
    use hyper_util::service::TowerToHyperService;

    let app = router::build_dashboard_router(state);
    serve_until_ctrl_c_with_handler(listener, move |stream| {
        let app = app.clone();
        async move {
            let io = TokioIo::new(stream);
            let service = TowerToHyperService::new(app);
            if let Err(e) = Http1Builder::new().serve_connection(io, service).await {
                log::warn!("HTTP connection error: {e}");
            }
        }
    })
    .await
}

async fn serve_until_ctrl_c_with_handler<F, Fut>(listener: TcpListener, mut on_stream: F) -> Result<()>
where
    F: FnMut(tokio::net::TcpStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            accept = listener.accept() => {
                let (stream, _) = accept.context("accepting dashboard TCP connection")?;
                tokio::spawn(on_stream(stream));
            }
        }
    }

    drop(listener);
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("Dashboard server stopped.");
    Ok(())
}

fn clickable_url(url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
}

fn terminal_supports_color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

fn warning_block_lines(warning: &str, use_color: bool) -> Vec<String> {
    let lines: Vec<&str> = warning.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    // Brown background close to terminal selection tone in user screenshot.
    const WARN_STYLE: &str = "30;48;2;107;79;59";
    let max_content_width = lines.iter().map(|line| line.len()).max().unwrap_or(0);
    let total_width = max_content_width + 4; // two spaces left + two spaces right
    let blank = " ".repeat(total_width);

    if use_color {
        let mut out = Vec::with_capacity(lines.len() + 3);
        out.push(format!("\x1b[{WARN_STYLE}m{blank}\x1b[K\x1b[0m"));
        let icon_line = format!("  \x1b[33m⚠\x1b[{WARN_STYLE}m  ");
        out.push(format!("\x1b[{WARN_STYLE}m{icon_line}\x1b[K\x1b[0m"));
        for line in &lines {
            let content = format!("  {line:<width$}  ", width = max_content_width);
            out.push(format!("\x1b[{WARN_STYLE}m{content}\x1b[K\x1b[0m"));
        }
        out.push(format!("\x1b[{WARN_STYLE}m{blank}\x1b[K\x1b[0m"));
        return out;
    }

    let mut out = Vec::with_capacity(lines.len() + 3);
    out.push(String::new());
    out.push("  ⚠".to_string());
    for line in &lines {
        out.push(format!("  {line}"));
    }
    out.push(String::new());
    out
}

fn print_warning_message(warning: &str) {
    for line in warning_block_lines(warning, terminal_supports_color()) {
        eprintln!("{line}");
    }
}

fn canonical_user_key(name: &str, email: &str) -> String {
    let email_normalized = email.trim().to_ascii_lowercase();
    if !email_normalized.is_empty() {
        return email_normalized;
    }

    let name_normalized = name.trim().to_ascii_lowercase();
    if name_normalized.is_empty() {
        return String::new();
    }
    format!("name:{name_normalized}")
}

pub(super) fn dashboard_user(name: &str, email: &str) -> DashboardUser {
    DashboardUser {
        key: canonical_user_key(name, email),
        name: name.trim().to_string(),
        email: email.trim().to_ascii_lowercase(),
    }
}

pub(super) fn canonical_agent_key(agent: &str) -> String {
    let trimmed = agent.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut key = String::with_capacity(trimmed.len());
    let mut last_was_dash = false;

    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            key.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !key.is_empty() && !last_was_dash {
            key.push('-');
            last_was_dash = true;
        }
    }

    while key.ends_with('-') {
        key.pop();
    }

    key
}

fn user_matches_filter(user: &DashboardUser, user_filter: Option<&str>) -> bool {
    let Some(filter) = user_filter else {
        return true;
    };

    let normalized = filter.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }

    user.key == normalized || user.name.to_ascii_lowercase() == normalized
}

fn agent_matches_filter(info: &CommittedInfo, agent_filter: Option<&str>) -> bool {
    let Some(filter) = agent_filter else {
        return true;
    };

    let normalized = canonical_agent_key(filter);
    if normalized.is_empty() {
        return true;
    }

    if !info.agents.is_empty() {
        return info
            .agents
            .iter()
            .map(|agent| canonical_agent_key(agent))
            .any(|agent| agent == normalized);
    }

    canonical_agent_key(&info.agent) == normalized
}

fn normalize_branch_name(branch: &str) -> &str {
    let trimmed = branch.trim().trim_start_matches('*').trim();
    if let Some(short) = trimmed.strip_prefix("refs/heads/") {
        return short;
    }
    if let Some(short) = trimmed.strip_prefix("refs/remotes/") {
        return short;
    }
    trimmed
}

pub(super) fn branch_is_excluded(branch: &str) -> bool {
    let normalized = normalize_branch_name(branch);
    let without_origin = normalized.strip_prefix("origin/").unwrap_or(normalized);

    without_origin == paths::METADATA_BRANCH_NAME || without_origin.starts_with("bitloops/")
}

pub(super) fn list_dashboard_branches(repo_root: &Path) -> Result<Vec<String>> {
    let refs = run_git(
        repo_root,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes/origin",
        ],
    )?;

    let mut branches: HashSet<String> = HashSet::new();
    for branch in refs.lines() {
        let branch = branch.trim();
        if branch.is_empty() || branch.ends_with("/HEAD") {
            continue;
        }
        if branch_is_excluded(branch) {
            continue;
        }
        branches.insert(branch.to_string());
    }

    let mut out: Vec<String> = branches.into_iter().collect();
    out.sort();
    Ok(out)
}

pub(super) fn build_branch_commit_log_args(
    branch_ref: &str,
    from_unix: Option<i64>,
    to_unix: Option<i64>,
    max_count: usize,
) -> Vec<String> {
    let mut args = vec![
        "log".to_string(),
        branch_ref.to_string(),
        "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%ct%x1f%s%x1e".to_string(),
        "--max-count".to_string(),
        max_count.max(1).to_string(),
        "--no-color".to_string(),
    ];

    if let Some(from) = from_unix {
        args.push(format!("--since=@{from}"));
    }
    if let Some(to) = to_unix {
        args.push(format!("--until=@{to}"));
    }
    args
}

pub(super) fn parse_branch_commit_log(raw: &str) -> Vec<DashboardCommitNode> {
    let mut nodes = Vec::new();

    for record in raw.split(GIT_RECORD_SEPARATOR) {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }

        let mut parts = record.split(GIT_FIELD_SEPARATOR);
        let Some(sha) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(parents_raw) = parts.next() else {
            continue;
        };
        let Some(author_name) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(author_email) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(timestamp_raw) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(message) = parts.next().map(str::trim) else {
            continue;
        };

        if sha.is_empty() {
            continue;
        }

        let timestamp = timestamp_raw.parse::<i64>().unwrap_or(0);

        nodes.push(DashboardCommitNode {
            sha: sha.to_string(),
            parents: parents_raw
                .split_whitespace()
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect(),
            author_name: author_name.to_string(),
            author_email: author_email.to_string(),
            timestamp,
            message: message.to_string(),
            checkpoint_id: String::new(),
        });
    }

    nodes
}

pub(super) fn parse_numstat_output(raw: &str) -> HashMap<String, (u64, u64)> {
    let mut stats: HashMap<String, (u64, u64)> = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() != 3 {
            continue;
        }
        let adds = if parts[0] == "-" {
            0u64
        } else {
            parts[0].parse::<u64>().unwrap_or(0)
        };
        let dels = if parts[1] == "-" {
            0u64
        } else {
            parts[1].parse::<u64>().unwrap_or(0)
        };
        let path = parts[2].to_string();
        let entry = stats.entry(path).or_insert((0, 0));
        entry.0 += adds;
        entry.1 += dels;
    }
    stats
}

pub(super) fn read_commit_numstat(
    repo_root: &Path,
    sha: &str,
) -> Result<HashMap<String, (u64, u64)>> {
    let raw = run_git(
        repo_root,
        &[
            "show",
            "--numstat",
            "--format=",
            "--no-color",
            "--find-renames",
            "--find-copies",
            sha,
        ],
    )?;
    Ok(parse_numstat_output(&raw))
}

pub(super) fn walk_branch_commits_with_checkpoints(
    repo_root: &Path,
    branch_ref: &str,
    from_unix: Option<i64>,
    to_unix: Option<i64>,
    max_count: usize,
) -> Result<Vec<DashboardCommitNode>> {
    let args = build_branch_commit_log_args(branch_ref, from_unix, to_unix, max_count);
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    let raw = run_git(repo_root, &args_ref)?;
    let mut commits = parse_branch_commit_log(&raw);
    attach_checkpoint_ids_from_db(repo_root, &mut commits)?;
    Ok(commits)
}

fn attach_checkpoint_ids_from_db(
    repo_root: &Path,
    commits: &mut [DashboardCommitNode],
) -> Result<()> {
    let mappings = read_commit_checkpoint_mappings(repo_root)
        .context("reading commit_checkpoints mappings for dashboard commit walk")?;
    if mappings.is_empty() {
        return Ok(());
    }

    for commit in commits {
        if let Some(checkpoint_id) = mappings.get(&commit.sha) {
            commit.checkpoint_id = checkpoint_id.clone();
        }
    }
    Ok(())
}

pub(super) fn paginate<T: Clone>(items: &[T], page: ApiPage) -> Vec<T> {
    let page = page.normalized();
    let start = page.offset.min(items.len());
    let end = start.saturating_add(page.limit).min(items.len());
    items[start..end].to_vec()
}

pub(super) fn build_committed_info_map(repo_root: &Path) -> Result<HashMap<String, CommittedInfo>> {
    Ok(list_committed(repo_root)?
        .into_iter()
        .map(|info| (info.checkpoint_id.clone(), info))
        .collect())
}

pub(super) fn query_commit_checkpoint_pairs(
    repo_root: &Path,
    query: &CommitCheckpointQuery,
) -> Result<Vec<CommitCheckpointPair>> {
    let pairs = query_commit_checkpoint_pairs_all(repo_root, query)?;
    Ok(paginate(&pairs, query.page))
}

pub(super) fn query_commit_checkpoint_pairs_all(
    repo_root: &Path,
    query: &CommitCheckpointQuery,
) -> Result<Vec<CommitCheckpointPair>> {
    let commits = walk_branch_commits_with_checkpoints(
        repo_root,
        &query.branch,
        query.from_unix,
        query.to_unix,
        API_GIT_SCAN_LIMIT,
    )?;
    let committed_map = build_committed_info_map(repo_root)?;

    let mut pairs = Vec::new();
    for commit in commits {
        if commit.checkpoint_id.is_empty() {
            continue;
        }

        let Some(info) = committed_map.get(&commit.checkpoint_id) else {
            continue;
        };

        let user = dashboard_user(&commit.author_name, &commit.author_email);
        if !user_matches_filter(&user, query.user.as_deref()) {
            continue;
        }
        if !agent_matches_filter(info, query.agent.as_deref()) {
            continue;
        }

        pairs.push(CommitCheckpointPair {
            commit,
            user,
            checkpoint: info.clone(),
        });
    }

    pairs.sort_by(|left, right| {
        right
            .commit
            .timestamp
            .cmp(&left.commit.timestamp)
            .then_with(|| right.commit.sha.cmp(&left.commit.sha))
    });

    Ok(pairs)
}

pub(super) fn read_checkpoint_info_for_filtering(
    repo_root: &Path,
    checkpoint_id: &str,
) -> Result<Option<CommittedInfo>> {
    read_committed_info(repo_root, checkpoint_id)
}

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

fn resolve_bundle_dir(bundle_arg: Option<&Path>) -> PathBuf {
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

fn normalized_host(input: &str) -> Option<&str> {
    let host = input.trim();
    if host.is_empty() { None } else { Some(host) }
}

fn resolve_bind_addr(host: &str, port: u16) -> Result<SocketAddr> {
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

fn format_dashboard_url(transport: DashboardTransport, host: &str, port: u16) -> String {
    let scheme = dashboard_scheme(transport);
    if host.contains(':') && !host.starts_with('[') {
        format!("{scheme}://[{host}]:{port}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}

fn open_in_default_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command.arg(url);
        command
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        return Err(anyhow!(
            "opening the browser is not supported on this platform"
        ));
    }

    let status = command
        .status()
        .with_context(|| format!("Running browser opener for {url}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Browser opener exited with status {status}"))
    }
}

#[cfg(test)]
mod tests;
