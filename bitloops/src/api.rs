mod bundle;
mod bundle_types;
mod dashboard_git;
mod dashboard_paths;
mod dashboard_runtime;
mod db;
mod dto;
mod handlers;
mod hosts;
mod router;
pub mod tls;

use crate::graphql;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(test)]
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) use self::db::{BackendHealth, BackendHealthKind, DashboardDbPools};

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

pub type DashboardReadyHook =
    Arc<dyn Fn(&DashboardReadyInfo) -> Result<()> + Send + Sync + 'static>;
pub type DashboardShutdownHook = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardServerConfig {
    pub host: Option<String>,
    pub port: u16,
    pub no_open: bool,
    pub force_http: bool,
    pub recheck_local_dashboard_net: bool,
    pub bundle_dir: Option<PathBuf>,
}

pub struct DashboardRuntimeOptions {
    pub ready_subject: String,
    pub print_ready_banner: bool,
    pub open_browser: bool,
    pub shutdown_message: Option<String>,
    pub on_ready: Option<DashboardReadyHook>,
    pub on_shutdown: Option<DashboardShutdownHook>,
}

impl Default for DashboardRuntimeOptions {
    fn default() -> Self {
        Self {
            ready_subject: "Dashboard".to_string(),
            print_ready_banner: true,
            open_browser: true,
            shutdown_message: Some("Dashboard server stopped.".to_string()),
            on_ready: None,
            on_shutdown: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardReadyInfo {
    pub url: String,
    pub host: String,
    pub port: u16,
    pub bundle_dir: PathBuf,
    pub repo_root: PathBuf,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardStartupMode {
    FastHttpLoopback,
    FastConfiguredHttps,
    SlowProbe,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LocalDashboardDiscovery {
    tls: bool,
    bitloops_local: bool,
}

#[derive(Clone)]
pub(crate) struct DashboardState {
    pub(super) repo_root: PathBuf,
    pub(super) mode: ServeMode,
    pub(super) db: db::DashboardDbPools,
    pub(super) bundle_dir: PathBuf,
    pub(super) devql_schema: graphql::DevqlSchema,
}

impl DashboardState {
    pub(crate) fn devql_schema(&self) -> &graphql::DevqlSchema {
        &self.devql_schema
    }
}

#[cfg(test)]
fn branch_is_excluded(branch: &str) -> bool {
    dashboard_git::branch_is_excluded(branch)
}

#[cfg(test)]
fn build_branch_commit_log_args(
    branch_ref: &str,
    from_unix: Option<i64>,
    to_unix: Option<i64>,
    max_count: usize,
) -> Vec<String> {
    dashboard_git::build_branch_commit_log_args(branch_ref, from_unix, to_unix, max_count)
}

pub(super) fn canonical_agent_key(agent: &str) -> String {
    dashboard_git::canonical_agent_key(agent)
}

pub(super) fn dashboard_user(name: &str, email: &str) -> DashboardUser {
    dashboard_git::dashboard_user(name, email)
}

pub(super) fn paginate<T: Clone>(items: &[T], page: ApiPage) -> Vec<T> {
    dashboard_git::paginate(items, page)
}

#[cfg(test)]
fn parse_branch_commit_log(raw: &str) -> Vec<DashboardCommitNode> {
    dashboard_git::parse_branch_commit_log(raw)
}

#[cfg(test)]
fn parse_numstat_output(raw: &str) -> HashMap<String, (u64, u64)> {
    dashboard_git::parse_numstat_output(raw)
}

pub(super) fn read_checkpoint_info_for_filtering(
    repo_root: &Path,
    checkpoint_id: &str,
) -> Result<Option<crate::host::checkpoints::strategy::manual_commit::CommittedInfo>> {
    dashboard_git::read_checkpoint_info_for_filtering(repo_root, checkpoint_id)
}

pub(super) fn read_commit_numstat(
    repo_root: &Path,
    sha: &str,
) -> Result<HashMap<String, (u64, u64)>> {
    dashboard_git::read_commit_numstat(repo_root, sha)
}

pub(super) fn user_matches_filter(user: &DashboardUser, user_filter: Option<&str>) -> bool {
    dashboard_git::user_matches_filter(user, user_filter)
}

pub(super) fn walk_branch_commits_with_checkpoints(
    repo_root: &Path,
    branch_ref: &str,
    from_unix: Option<i64>,
    to_unix: Option<i64>,
    max_count: usize,
) -> Result<Vec<DashboardCommitNode>> {
    dashboard_git::walk_branch_commits_with_checkpoints(
        repo_root, branch_ref, from_unix, to_unix, max_count,
    )
}

pub(super) fn content_type_for_path(path: &Path) -> &'static str {
    dashboard_paths::content_type_for_path(path)
}

#[cfg(test)]
fn default_bundle_dir_from_home(home: Option<&Path>) -> PathBuf {
    dashboard_paths::default_bundle_dir_from_home(home)
}

#[cfg(test)]
fn expand_tilde_with_home(path: &Path, home: Option<&Path>) -> PathBuf {
    dashboard_paths::expand_tilde_with_home(path, home)
}

#[cfg(test)]
fn browser_host_for_url(bind_host: &str, local_addr: SocketAddr) -> String {
    dashboard_paths::browser_host_for_url(bind_host, local_addr)
}

#[cfg(test)]
fn format_dashboard_url(transport: DashboardTransport, host: &str, port: u16) -> String {
    dashboard_paths::format_dashboard_url(transport, host, port)
}

pub(super) fn has_bundle_index(bundle_dir: &Path) -> bool {
    dashboard_paths::has_bundle_index(bundle_dir)
}

pub(super) fn resolve_bundle_file(bundle_dir: &Path, request_path: &str) -> Option<PathBuf> {
    dashboard_paths::resolve_bundle_file(bundle_dir, request_path)
}

#[cfg(test)]
fn select_host_with_dashboard_preference(
    explicit_host: Option<&str>,
    use_bitloops_local: bool,
) -> String {
    dashboard_paths::select_host_with_dashboard_preference(explicit_host, use_bitloops_local)
}

#[cfg(test)]
fn select_startup_mode(
    config: &DashboardServerConfig,
    local_dashboard_cfg: Option<&crate::config::DashboardLocalDashboardConfig>,
    explicit_host: Option<&str>,
) -> Result<DashboardStartupMode> {
    dashboard_runtime::select_startup_mode(config, local_dashboard_cfg, explicit_host)
}

#[cfg(test)]
fn warning_block_lines(warning: &str, use_color: bool) -> Vec<String> {
    dashboard_runtime::warning_block_lines(warning, use_color)
}

pub async fn run(config: DashboardServerConfig) -> Result<()> {
    run_with_options(config, DashboardRuntimeOptions::default()).await
}

pub async fn run_with_options(
    config: DashboardServerConfig,
    options: DashboardRuntimeOptions,
) -> Result<()> {
    dashboard_runtime::run(config, options).await
}

pub fn open_in_default_browser(url: &str) -> Result<()> {
    dashboard_runtime::open_in_default_browser(url)
}

#[cfg(test)]
mod tests;
