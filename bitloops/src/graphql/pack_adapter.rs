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
                let Some(value) = normalise_stage_arg_value(value)? else {
                    continue;
                };
                normalised.insert(key, Value::String(value));
            }
            Ok(Value::Object(normalised))
        }
        _ => bail!("extension args must be a JSON object"),
    }
}

fn normalise_stage_arg_value(value: Value) -> Result<Option<String>> {
    match value {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(value)),
        Value::Number(value) => Ok(Some(value.to_string())),
        Value::Bool(value) => Ok(Some(value.to_string())),
        _ => bail!("extension args must contain only string, number, boolean, or null values"),
    }
}
