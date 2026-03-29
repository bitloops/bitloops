use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::json;
#[cfg(test)]
use std::path::Path;

use crate::devql_transport::SlimCliRepoScope;
use crate::host::devql::{
    IngestionCounters, InitSchemaSummary, format_ingestion_summary, format_init_schema_summary,
};

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
        initRequested
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
    init: bool,
    max_checkpoints: usize,
) -> Result<()> {
    let response: IngestMutationData = execute_devql_graphql(
        scope,
        INGEST_MUTATION,
        json!({
            "input": {
                "init": init,
                "maxCheckpoints": max_checkpoints,
            }
        }),
    )
    .await?;
    println!("{}", format_ingestion_summary(&response.ingest));
    Ok(())
}

#[cfg(test)]
pub(super) fn with_graphql_executor_hook<T>(
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
