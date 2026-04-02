use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::path::Path;

use crate::devql_transport::SlimCliRepoScope;
use crate::host::devql::{
    IngestionCounters, InitSchemaSummary, SyncSummary, SyncValidationFileDrift,
    SyncValidationSummary, format_ingestion_summary, format_init_schema_summary,
};
use crate::{api::DashboardServerConfig, daemon};

#[cfg(test)]
type IngestDaemonBootstrapHook = dyn Fn(&Path) -> Result<()> + 'static;

#[cfg(test)]
type IngestDaemonBootstrapHookCell =
    std::cell::RefCell<Option<std::rc::Rc<IngestDaemonBootstrapHook>>>;

#[cfg(test)]
thread_local! {
    static INGEST_DAEMON_BOOTSTRAP_HOOK: IngestDaemonBootstrapHookCell =
        std::cell::RefCell::new(None);
}

const INIT_SCHEMA_MUTATION: &str = r#"
    mutation InitSchema {
      initSchema {
        success
        repoIdentity
        repoId
        relationalBackend
        eventsBackend
      }
    }
"#;

const INGEST_MUTATION: &str = r#"
    mutation Ingest($input: IngestInput!) {
      ingest(input: $input) {
        success
        checkpointsProcessed
        eventsInserted
        artefactsUpserted
        checkpointsWithoutCommit
        temporaryRowsPromoted
        semanticFeatureRowsUpserted
        semanticFeatureRowsSkipped
        symbolEmbeddingRowsUpserted
        symbolEmbeddingRowsSkipped
        symbolCloneEdgesUpserted
        symbolCloneSourcesScored
      }
    }
"#;

const SYNC_MUTATION: &str = r#"
    mutation Sync($input: SyncInput!) {
      sync(input: $input) {
        success
        mode
        parserVersion
        extractorVersion
        activeBranch
        headCommitSha
        headTreeSha
        pathsUnchanged
        pathsAdded
        pathsChanged
        pathsRemoved
        cacheHits
        cacheMisses
        parseErrors
        validation {
          valid
          expectedArtefacts
          actualArtefacts
          expectedEdges
          actualEdges
          missingArtefacts
          staleArtefacts
          mismatchedArtefacts
          missingEdges
          staleEdges
          mismatchedEdges
          filesWithDrift {
            path
            missingArtefacts
            staleArtefacts
            mismatchedArtefacts
            missingEdges
            staleEdges
            mismatchedEdges
          }
        }
      }
    }
"#;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitSchemaMutationData {
    init_schema: InitSchemaSummary,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngestMutationData {
    ingest: IngestionCounters,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncMutationData {
    sync: SyncMutationResult,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncMutationResult {
    success: bool,
    mode: String,
    parser_version: String,
    extractor_version: String,
    active_branch: Option<String>,
    head_commit_sha: Option<String>,
    head_tree_sha: Option<String>,
    paths_unchanged: usize,
    paths_added: usize,
    paths_changed: usize,
    paths_removed: usize,
    cache_hits: usize,
    cache_misses: usize,
    parse_errors: usize,
    validation: Option<SyncValidationMutationResult>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncValidationMutationResult {
    valid: bool,
    expected_artefacts: usize,
    actual_artefacts: usize,
    expected_edges: usize,
    actual_edges: usize,
    missing_artefacts: usize,
    stale_artefacts: usize,
    mismatched_artefacts: usize,
    missing_edges: usize,
    stale_edges: usize,
    mismatched_edges: usize,
    files_with_drift: Vec<SyncValidationFileDriftMutationResult>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncValidationFileDriftMutationResult {
    path: String,
    missing_artefacts: usize,
    stale_artefacts: usize,
    mismatched_artefacts: usize,
    missing_edges: usize,
    stale_edges: usize,
    mismatched_edges: usize,
}

impl From<SyncMutationResult> for SyncSummary {
    fn from(value: SyncMutationResult) -> Self {
        Self {
            success: value.success,
            mode: value.mode,
            parser_version: value.parser_version,
            extractor_version: value.extractor_version,
            active_branch: value.active_branch,
            head_commit_sha: value.head_commit_sha,
            head_tree_sha: value.head_tree_sha,
            paths_unchanged: value.paths_unchanged,
            paths_added: value.paths_added,
            paths_changed: value.paths_changed,
            paths_removed: value.paths_removed,
            cache_hits: value.cache_hits,
            cache_misses: value.cache_misses,
            parse_errors: value.parse_errors,
            validation: value.validation.map(|validation| SyncValidationSummary {
                valid: validation.valid,
                expected_artefacts: validation.expected_artefacts,
                actual_artefacts: validation.actual_artefacts,
                expected_edges: validation.expected_edges,
                actual_edges: validation.actual_edges,
                missing_artefacts: validation.missing_artefacts,
                stale_artefacts: validation.stale_artefacts,
                mismatched_artefacts: validation.mismatched_artefacts,
                missing_edges: validation.missing_edges,
                stale_edges: validation.stale_edges,
                mismatched_edges: validation.mismatched_edges,
                files_with_drift: validation
                    .files_with_drift
                    .into_iter()
                    .map(|file| SyncValidationFileDrift {
                        path: file.path,
                        missing_artefacts: file.missing_artefacts,
                        stale_artefacts: file.stale_artefacts,
                        mismatched_artefacts: file.mismatched_artefacts,
                        missing_edges: file.missing_edges,
                        stale_edges: file.stale_edges,
                        mismatched_edges: file.mismatched_edges,
                    })
                    .collect(),
            }),
        }
    }
}

#[cfg(test)]
thread_local! {
    static GRAPHQL_EXECUTOR_HOOK: std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
type GraphqlExecutorHook =
    dyn Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static;

pub(super) async fn execute_devql_graphql<T: DeserializeOwned>(
    scope: &SlimCliRepoScope,
    query: &str,
    variables: serde_json::Value,
) -> Result<T> {
    #[cfg(test)]
    if let Some(data) =
        maybe_execute_devql_graphql_via_hook(scope.repo_root.as_path(), query, &variables)
    {
        return Ok(serde_json::from_value(data?)?);
    }

    crate::daemon::execute_slim_graphql(scope.repo_root.as_path(), scope, query, variables).await
}

pub(super) async fn run_init_via_graphql(scope: &SlimCliRepoScope) -> Result<()> {
    let response: InitSchemaMutationData =
        execute_devql_graphql(scope, INIT_SCHEMA_MUTATION, json!({})).await?;
    println!("{}", format_init_schema_summary(&response.init_schema));
    Ok(())
}

pub(super) async fn run_ingest_via_graphql(
    scope: &SlimCliRepoScope,
    max_checkpoints: usize,
) -> Result<()> {
    ensure_daemon_available_for_ingest(scope.repo_root.as_path()).await?;
    let response: IngestMutationData = execute_devql_graphql(
        scope,
        INGEST_MUTATION,
        json!({
            "input": {
                "maxCheckpoints": max_checkpoints,
            }
        }),
    )
    .await?;
    println!("{}", format_ingestion_summary(&response.ingest));
    Ok(())
}

pub(super) async fn run_sync_via_graphql(
    scope: &SlimCliRepoScope,
    full: bool,
    paths: Option<Vec<String>>,
    repair: bool,
    validate: bool,
) -> Result<SyncSummary> {
    ensure_daemon_available_for_ingest(scope.repo_root.as_path()).await?;
    let response: SyncMutationData = execute_devql_graphql(
        scope,
        SYNC_MUTATION,
        json!({
            "input": {
                "full": full,
                "paths": paths,
                "repair": repair,
                "validate": validate,
            }
        }),
    )
    .await?;
    Ok(response.sync.into())
}

async fn ensure_daemon_available_for_ingest(_repo_root: &Path) -> Result<()> {
    #[cfg(test)]
    if let Some(result) = maybe_bootstrap_daemon_via_hook(_repo_root) {
        return result;
    }

    if daemon::daemon_url()?.is_some() {
        return Ok(());
    }

    let report = daemon::status().await?;
    let daemon_config = daemon::resolve_daemon_config(None)?;
    let config = DashboardServerConfig {
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        no_open: true,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    };

    if report.service.is_some() {
        let _ = daemon::start_service(&daemon_config, config, None).await?;
    } else {
        let _ = daemon::start_detached(&daemon_config, config, None).await?;
    }

    Ok(())
}

#[cfg(test)]
pub(super) fn with_ingest_daemon_bootstrap_hook<T>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    INGEST_DAEMON_BOOTSTRAP_HOOK.with(|cell: &IngestDaemonBootstrapHookCell| {
        assert!(
            cell.borrow().is_none(),
            "ingest daemon hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let result = f();
    INGEST_DAEMON_BOOTSTRAP_HOOK.with(|cell: &IngestDaemonBootstrapHookCell| {
        *cell.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
fn maybe_bootstrap_daemon_via_hook(repo_root: &Path) -> Option<Result<()>> {
    INGEST_DAEMON_BOOTSTRAP_HOOK.with(|hook: &IngestDaemonBootstrapHookCell| {
        hook.borrow().as_ref().map(|hook| hook(repo_root))
    })
}

#[cfg(test)]
pub(crate) fn with_graphql_executor_hook<T>(
    hook: impl Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            assert!(
                cell.borrow().is_none(),
                "graphql executor hook already installed"
            );
            *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
        },
    );
    let result = f();
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            *cell.borrow_mut() = None;
        },
    );
    result
}

#[cfg(test)]
fn maybe_execute_devql_graphql_via_hook(
    repo_root: &Path,
    query: &str,
    variables: &serde_json::Value,
) -> Option<Result<serde_json::Value>> {
    GRAPHQL_EXECUTOR_HOOK.with(
        |hook: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            hook.borrow()
                .as_ref()
                .map(|hook| hook(repo_root, query, variables))
        },
    )
}
