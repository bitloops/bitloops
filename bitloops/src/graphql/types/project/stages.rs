use super::*;
use crate::graphql::types::Artefact;

pub(super) fn stage_limit(first: i32) -> Result<usize> {
    if first <= 0 {
        return Err(bad_user_input_error("`first` must be greater than 0"));
    }
    Ok(first as usize)
}

pub(super) fn optional_positive_limit(name: &str, value: Option<i32>) -> Result<Option<usize>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value <= 0 {
        return Err(bad_user_input_error(format!(
            "`{name}` must be greater than 0"
        )));
    }
    Ok(Some(value as usize))
}

pub(super) fn normalise_architecture_graph_filter(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    filter: Option<ArchitectureGraphFilterInput>,
) -> Result<Option<ArchitectureGraphFilterInput>> {
    let Some(mut filter) = filter else {
        return Ok(None);
    };
    if let Some(path) = filter.path.as_ref() {
        filter.path = Some(
            context
                .resolve_scope_path(scope, path, false)
                .map_err(bad_user_input_error)?,
        );
    }
    Ok(Some(filter))
}

pub(super) fn build_tests_stage_args(
    min_confidence: Option<f64>,
    linkage_source: Option<String>,
) -> Result<Value> {
    if let Some(min_confidence) = min_confidence
        && !(0.0..=1.0).contains(&min_confidence)
    {
        return Err(bad_user_input_error(
            "`minConfidence` must be between 0 and 1",
        ));
    }

    let mut args = serde_json::Map::new();
    if let Some(min_confidence) = min_confidence {
        args.insert("min_confidence".to_string(), json!(min_confidence));
    }
    if let Some(linkage_source) = linkage_source
        && !linkage_source.trim().is_empty()
    {
        args.insert(
            "linkage_source".to_string(),
            Value::String(linkage_source.trim().to_string()),
        );
    }
    Ok(Value::Object(args))
}

#[derive(Default)]
pub(super) struct CodeCityStageArgs {
    pub(super) include_dependency_arcs: Option<bool>,
    pub(super) include_boundaries: Option<bool>,
    pub(super) include_architecture: Option<bool>,
    pub(super) include_macro_edges: Option<bool>,
    pub(super) include_zone_diagnostics: Option<bool>,
    pub(super) architecture_enabled: Option<bool>,
    pub(super) include_health: Option<bool>,
    pub(super) analysis_window_months: Option<i32>,
}

pub(super) fn build_codecity_stage_args(stage_args: CodeCityStageArgs) -> Value {
    let mut args = serde_json::Map::new();
    let CodeCityStageArgs {
        include_dependency_arcs,
        include_boundaries,
        include_architecture,
        include_macro_edges,
        include_zone_diagnostics,
        architecture_enabled,
        include_health,
        analysis_window_months,
    } = stage_args;
    if let Some(include_dependency_arcs) = include_dependency_arcs {
        args.insert(
            "include_dependency_arcs".to_string(),
            Value::Bool(include_dependency_arcs),
        );
    }
    if let Some(include_boundaries) = include_boundaries {
        args.insert(
            "include_boundaries".to_string(),
            Value::Bool(include_boundaries),
        );
    }
    if let Some(include_architecture) = include_architecture {
        args.insert(
            "include_architecture".to_string(),
            Value::Bool(include_architecture),
        );
    }
    if let Some(include_macro_edges) = include_macro_edges {
        args.insert(
            "include_macro_edges".to_string(),
            Value::Bool(include_macro_edges),
        );
    }
    if let Some(include_zone_diagnostics) = include_zone_diagnostics {
        args.insert(
            "include_zone_diagnostics".to_string(),
            Value::Bool(include_zone_diagnostics),
        );
    }
    if let Some(architecture_enabled) = architecture_enabled {
        args.insert(
            "architecture_enabled".to_string(),
            Value::Bool(architecture_enabled),
        );
    }
    if let Some(include_health) = include_health {
        args.insert("include_health".to_string(), Value::Bool(include_health));
    }
    if let Some(analysis_window_months) = analysis_window_months {
        args.insert(
            "analysis_window_months".to_string(),
            Value::Number(serde_json::Number::from(analysis_window_months as i64)),
        );
    }
    Value::Object(args)
}

pub(super) fn build_codecity_violations_args(
    filter: Option<CodeCityViolationFilterInput>,
    pagination: &ConnectionPagination,
) -> Value {
    let mut args = serde_json::Map::new();
    insert_pagination_args(&mut args, pagination);
    if let Some(filter) = filter {
        if let Some(severity) = filter.severity {
            args.insert(
                "severity".to_string(),
                Value::String(severity.as_stage_value().to_string()),
            );
        }
        if let Some(severities) = filter.severities
            && !severities.is_empty()
        {
            args.insert(
                "severities".to_string(),
                Value::Array(
                    severities
                        .into_iter()
                        .map(|severity| Value::String(severity.as_stage_value().to_string()))
                        .collect(),
                ),
            );
        }
        if let Some(pattern) = filter.pattern {
            args.insert(
                "pattern".to_string(),
                Value::String(pattern.as_stage_value().to_string()),
            );
        }
        if let Some(rule) = filter.rule {
            args.insert(
                "rule".to_string(),
                Value::String(rule.as_stage_value().to_string()),
            );
        }
        insert_optional_string(&mut args, "boundary_id", filter.boundary_id);
        insert_optional_string(&mut args, "path", filter.path);
        insert_optional_string(&mut args, "from_path", filter.from_path);
        insert_optional_string(&mut args, "to_path", filter.to_path);
        if let Some(include_suppressed) = filter.include_suppressed {
            args.insert(
                "include_suppressed".to_string(),
                Value::Bool(include_suppressed),
            );
        }
    }
    Value::Object(args)
}

pub(super) fn build_codecity_arcs_args(
    filter: Option<CodeCityArcFilterInput>,
    pagination: &ConnectionPagination,
) -> Value {
    let mut args = serde_json::Map::new();
    insert_pagination_args(&mut args, pagination);
    if let Some(filter) = filter {
        if let Some(kind) = filter.kind {
            args.insert(
                "kind".to_string(),
                Value::String(kind.as_stage_value().to_string()),
            );
        }
        if let Some(visibility) = filter.visibility {
            args.insert(
                "visibility".to_string(),
                Value::String(visibility.as_stage_value().to_string()),
            );
        }
        if let Some(severity) = filter.severity {
            args.insert(
                "severity".to_string(),
                Value::String(severity.as_stage_value().to_string()),
            );
        }
        insert_optional_string(&mut args, "boundary_id", filter.boundary_id);
        insert_optional_string(&mut args, "path", filter.path);
        if let Some(direction) = filter.direction {
            args.insert(
                "direction".to_string(),
                Value::String(direction.as_stage_value().to_string()),
            );
        }
        if let Some(include_hidden) = filter.include_hidden {
            args.insert("include_hidden".to_string(), Value::Bool(include_hidden));
        }
    }
    Value::Object(args)
}

pub(super) fn insert_pagination_args(
    args: &mut serde_json::Map<String, Value>,
    pagination: &ConnectionPagination,
) {
    match pagination {
        ConnectionPagination::Forward { limit, after } => {
            args.insert(
                "first".to_string(),
                Value::Number(serde_json::Number::from(*limit as i64)),
            );
            insert_optional_string(args, "after", after.clone());
        }
        ConnectionPagination::Backward { limit, before } => {
            args.insert(
                "last".to_string(),
                Value::Number(serde_json::Number::from(*limit as i64)),
            );
            insert_optional_string(args, "before", before.clone());
        }
    }
}

pub(super) fn insert_optional_string(
    args: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        args.insert(key.to_string(), Value::String(value));
    }
}

pub(super) fn project_stage_row_from_artefact(artefact: &Artefact) -> Value {
    json!({
        "artefact_id": artefact.id.as_ref(),
        "symbol_id": &artefact.symbol_id,
        "symbol_fqn": &artefact.symbol_fqn,
        "canonical_kind": artefact.canonical_kind.map(|kind| kind.as_devql_value()),
        "path": &artefact.path,
        "start_line": artefact.start_line,
        "end_line": artefact.end_line,
    })
}

pub(super) fn decode_stage_rows<T: DeserializeOwned>(
    stage: &str,
    rows: Vec<Value>,
) -> Result<Vec<T>> {
    rows.into_iter()
        .map(|row| {
            serde_json::from_value(row).map_err(|err| {
                backend_error(format!(
                    "failed to decode `{stage}` stage payload into typed GraphQL result: {err}"
                ))
            })
        })
        .collect()
}

pub(super) fn decode_stage_single<T: DeserializeOwned>(stage: &str, rows: Vec<Value>) -> Result<T> {
    let Some(row) = rows.into_iter().next() else {
        return Err(backend_error(format!(
            "failed to decode `{stage}` stage payload: empty result"
        )));
    };
    serde_json::from_value(row).map_err(|err| {
        backend_error(format!(
            "failed to decode `{stage}` stage payload into typed GraphQL result: {err}"
        ))
    })
}

pub(super) fn map_stage_adapter_error(scope: &str, err: anyhow::Error) -> async_graphql::Error {
    let message = format!("{err:#}");
    if message.contains("unsupported DevQL stage")
        || message.contains("ambiguous DevQL stage")
        || message.contains("extension args must")
        || message.contains("requires a resolved commit")
        || message.contains("unknown CodeCity path")
    {
        return bad_user_input_error(message);
    }
    backend_error(format!("failed to resolve {scope}: {message}"))
}
