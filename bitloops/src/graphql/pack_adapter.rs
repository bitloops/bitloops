use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use super::{DevqlGraphqlContext, ResolverScope};

#[derive(Clone)]
pub(crate) struct StageResolverAdapter {
    context: DevqlGraphqlContext,
    stage_name: String,
}

impl StageResolverAdapter {
    pub(crate) fn new(context: DevqlGraphqlContext, stage_name: impl Into<String>) -> Self {
        Self {
            context,
            stage_name: stage_name.into(),
        }
    }

    pub(crate) async fn resolve(
        &self,
        scope: &ResolverScope,
        input_rows: Vec<Value>,
        args: Option<Value>,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let args = normalise_stage_args(args)?;
        let host = self.context.capability_host_arc()?;
        let capability_id = resolve_stage_owner(&host, &self.stage_name)?.to_string();
        let query_context = build_query_context(scope, self.context.repo_id());
        let response = host
            .invoke_stage(
                capability_id.as_str(),
                &self.stage_name,
                json!({
                    "input_rows": input_rows,
                    "args": args,
                    "limit": limit.max(1),
                    "query_context": query_context,
                }),
            )
            .await?;

        if response.payload.get("status").and_then(Value::as_str) == Some("failed") {
            bail!(response.human_output);
        }

        Ok(match response.payload {
            Value::Array(rows) => rows,
            value => vec![value],
        })
    }
}

fn resolve_stage_owner<'a>(
    host: &'a crate::host::capability_host::DevqlCapabilityHost,
    stage_name: &str,
) -> Result<&'a str> {
    let owners = host
        .descriptors()
        .filter_map(|descriptor| {
            host.has_stage(descriptor.id, stage_name)
                .then_some(descriptor.id)
        })
        .collect::<Vec<_>>();

    match owners.as_slice() {
        [] => bail!("unsupported DevQL stage: {}()", stage_name),
        [capability_id] => Ok(*capability_id),
        _ => bail!(
            "ambiguous DevQL stage: {}() is registered by multiple capabilities ({})",
            stage_name,
            owners.join(", ")
        ),
    }
}

fn build_query_context(scope: &ResolverScope, repo_id: &str) -> Value {
    let mut query_context = Map::new();
    let repo_id = scope
        .repository()
        .map(|repository| repository.repo_id())
        .unwrap_or(repo_id);
    query_context.insert(
        "resolved_commit_sha".to_string(),
        scope
            .temporal_scope()
            .map(|temporal_scope| Value::String(temporal_scope.resolved_commit().to_string()))
            .unwrap_or(Value::Null),
    );
    query_context.insert(
        "project_path".to_string(),
        scope
            .project_path()
            .map(|project_path| Value::String(project_path.to_string()))
            .unwrap_or(Value::Null),
    );
    query_context.insert("repo_id".to_string(), Value::String(repo_id.to_string()));
    Value::Object(query_context)
}

fn normalise_stage_args(args: Option<Value>) -> Result<Value> {
    let Some(args) = args else {
        return Ok(Value::Object(Map::new()));
    };

    match args {
        Value::Null => Ok(Value::Object(Map::new())),
        Value::Object(entries) => {
            let mut normalised = Map::new();
            for (key, value) in entries {
                normalised.insert(key, normalise_stage_arg_value(value)?);
            }
            Ok(Value::Object(normalised))
        }
        _ => bail!("extension args must be a JSON object"),
    }
}

fn normalise_stage_arg_value(value: Value) -> Result<Value> {
    match value {
        Value::Null | Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(value),
        Value::Array(items) => {
            let mut normalised = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::Null | Value::String(_) | Value::Number(_) | Value::Bool(_) => {
                        normalised.push(item)
                    }
                    _ => bail!("extension args arrays must contain only scalar values"),
                }
            }
            Ok(Value::Array(normalised))
        }
        _ => bail!("extension args must contain only string, number, boolean, or null values"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::graphql::scope::SelectedRepository;

    use super::{build_query_context, normalise_stage_args};

    #[test]
    fn normalise_stage_args_preserves_scalar_json_values() {
        let args = normalise_stage_args(Some(json!({
            "label": "coverage",
            "limit": 5,
            "enabled": true,
            "cursor": null,
        })))
        .expect("normalise args");

        assert_eq!(
            args,
            json!({
                "label": "coverage",
                "limit": 5,
                "enabled": true,
                "cursor": null,
            })
        );
    }

    #[test]
    fn normalise_stage_args_rejects_nested_json_values() {
        let err = normalise_stage_args(Some(json!({
            "filter": { "branch": "main" }
        })))
        .expect_err("nested JSON values must be rejected");

        assert!(
            err.to_string()
                .contains("string, number, boolean, or null values")
        );
    }

    #[test]
    fn build_query_context_prefers_scoped_repository_id() {
        let repository = SelectedRepository::new(
            "repo-selected".to_string(),
            "github".to_string(),
            "bitloops".to_string(),
            "bitloops".to_string(),
            "github://bitloops/bitloops".to_string(),
            Some("main".to_string()),
            None,
        );
        let scope = crate::graphql::ResolverScope::default().with_repository(repository);

        let context = build_query_context(&scope, "repo-default");

        assert_eq!(context["repo_id"], "repo-selected");
    }

    #[test]
    fn build_query_context_falls_back_to_default_repository_id() {
        let context =
            build_query_context(&crate::graphql::ResolverScope::default(), "repo-default");

        assert_eq!(context["repo_id"], "repo-default");
    }
}
