use super::*;

pub async fn run_query(cfg: &DevqlConfig, query: &str, compact: bool) -> Result<()> {
    let output = execute_query_json(cfg, query).await?;
    if compact {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

pub async fn execute_query_json_for_repo_root(repo_root: &Path, query: &str) -> Result<Value> {
    let repo = resolve_repo_identity(repo_root)?;
    let cfg = DevqlConfig::from_env(repo_root.to_path_buf(), repo)?;
    execute_query_json(&cfg, query).await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisteredStageCompositionContext {
    pub(crate) caller_capability_id: String,
    pub(crate) depth: usize,
    pub(crate) max_depth: usize,
}

async fn execute_query_json(cfg: &DevqlConfig, query: &str) -> Result<Value> {
    execute_query_json_with_composition(cfg, query, None).await
}

pub(crate) async fn execute_query_json_with_composition(
    cfg: &DevqlConfig,
    query: &str,
    composition: Option<RegisteredStageCompositionContext>,
) -> Result<Value> {
    let parsed = parse_devql_query(query)?;
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for `devql query`")?;
    let relational = if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        None
    } else {
        Some(RelationalStorage::connect(cfg, &backends.relational, "devql query").await?)
    };
    let mut rows = execute_devql_query(cfg, &parsed, &backends.events, relational.as_ref()).await?;
    rows = execute_registered_stages_with_composition(cfg, &parsed, rows, composition.as_ref())
        .await?;

    if !parsed.select_fields.is_empty() {
        rows = project_rows(rows, &parsed.select_fields);
    }

    Ok(Value::Array(rows))
}
