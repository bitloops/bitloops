use super::*;

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
    {
        return bad_user_input_error(message);
    }
    backend_error(format!("failed to resolve {scope}: {message}"))
}
