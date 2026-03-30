#![allow(clippy::await_holding_lock)]

use super::router::build_dashboard_router;
use super::{
    ApiPage, DashboardServerConfig, DashboardStartupMode, DashboardState, DashboardTransport,
    GIT_FIELD_SEPARATOR, GIT_RECORD_SEPARATOR, ServeMode, branch_is_excluded, browser_host_for_url,
    build_branch_commit_log_args, canonical_agent_key, dashboard_user,
    default_bundle_dir_from_cache_dir, expand_tilde_with_home, format_dashboard_url,
    has_bundle_index, paginate, parse_branch_commit_log, parse_numstat_output, resolve_bundle_file,
    select_startup_mode, warning_block_lines,
};
use crate::test_support::git_fixtures::{git_ok, init_test_repo, repo_local_blob_root};
use crate::test_support::process_state::{ProcessStateGuard, enter_env_vars, enter_process_state};
use async_graphql::futures_util::StreamExt;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::thread;
use tempfile::TempDir;
use tower::util::ServiceExt;

mod support_api_harness;
mod support_dashboard;
mod support_graphql_history;
mod support_graphql_knowledge;
mod support_graphql_monorepo;
mod support_storage;

use self::support_api_harness::*;
use self::support_dashboard::*;
use self::support_graphql_history::*;
use self::support_graphql_knowledge::*;
use self::support_graphql_monorepo::*;
use self::support_storage::*;

mod dashboard_api_bundle;
mod dashboard_config_utils;
mod devql_knowledge_clone_events;
mod devql_mutations_and_health;
mod devql_repository_graph;
mod devql_routes_subscriptions;
mod numstat_output;
