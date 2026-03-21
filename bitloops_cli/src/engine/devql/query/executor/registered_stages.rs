fn resolve_registered_stage_owner(
    host: &crate::engine::devql::capability_host::DevqlCapabilityHost,
    stage_name: &str,
) -> Result<&'static str> {
    let owners = host
        .descriptors()
        .filter_map(|descriptor| host.has_stage(descriptor.id, stage_name).then_some(descriptor.id))
        .collect::<Vec<_>>();

    match owners.as_slice() {
        [] => bail!("unsupported DevQL stage: {}()", stage_name),
        [capability_id] => Ok(capability_id),
        _ => bail!(
            "ambiguous DevQL stage: {}() is registered by multiple capabilities ({})",
            stage_name,
            owners.join(", ")
        ),
    }
}

fn validate_registered_stage_composition(
    host: &crate::engine::devql::capability_host::DevqlCapabilityHost,
    stage_name: &str,
    capability_id: &str,
    composition: Option<&RegisteredStageCompositionContext>,
) -> Result<()> {
    let Some(composition) = composition else {
        return Ok(());
    };

    if composition.depth > composition.max_depth {
        bail!(
            "DevQL composition depth {} exceeds configured max depth {} for capability `{}`",
            composition.depth,
            composition.max_depth,
            composition.caller_capability_id
        );
    }

    if composition.caller_capability_id == capability_id {
        return Ok(());
    }

    let Some(descriptor) = host.descriptor(composition.caller_capability_id.as_str()) else {
        bail!(
            "DevQL composition caller `{}` is not registered as a capability",
            composition.caller_capability_id
        );
    };

    let dependency_declared = descriptor
        .dependencies
        .iter()
        .any(|dependency| dependency.capability_id == capability_id);
    let grant_ok = host.cross_pack_access().allows_registered_stage_invocation(
        composition.caller_capability_id.as_str(),
        capability_id,
    );
    if dependency_declared || grant_ok {
        return Ok(());
    }

    bail!(
        "capability `{}` cannot invoke stage {}() owned by capability `{}`: no descriptor dependency and no `host.cross_pack_access` grant for resource `{}`",
        composition.caller_capability_id,
        stage_name,
        capability_id,
        crate::engine::devql::capability_host::CrossPackAccessPolicy::RESOURCE_DEVQL_REGISTERED_STAGE
    );
}

fn build_registered_stage_query_context(
    resolved_commit_sha: Option<String>,
    composition: Option<&RegisteredStageCompositionContext>,
) -> Value {
    let mut query_context = serde_json::Map::new();
    query_context.insert(
        "resolved_commit_sha".to_string(),
        resolved_commit_sha.map(Value::String).unwrap_or(Value::Null),
    );
    if let Some(composition) = composition {
        query_context.insert(
            "composition".to_string(),
            json!({
                "caller_capability_id": composition.caller_capability_id,
                "depth": composition.depth,
                "max_depth": composition.max_depth,
            }),
        );
    }
    Value::Object(query_context)
}

#[cfg(test)]
async fn execute_registered_stages(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    rows: Vec<Value>,
) -> Result<Vec<Value>> {
    execute_registered_stages_with_composition(cfg, parsed, rows, None).await
}

async fn execute_registered_stages_with_composition(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    mut rows: Vec<Value>,
    composition: Option<&RegisteredStageCompositionContext>,
) -> Result<Vec<Value>> {
    if parsed.registered_stages.is_empty() {
        return Ok(rows);
    }

    let mut host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
    let resolved_commit_sha = resolve_commit_selector(cfg, parsed)?;
    for stage in &parsed.registered_stages {
        let capability_id = resolve_registered_stage_owner(&host, &stage.stage_name)?;
        validate_registered_stage_composition(&host, &stage.stage_name, capability_id, composition)?;
        let query_context =
            build_registered_stage_query_context(resolved_commit_sha.clone(), composition);

        let response = host
            .invoke_stage(
                capability_id,
                &stage.stage_name,
                json!({
                    "input_rows": rows,
                    "args": stage.args,
                    "limit": parsed.limit.max(1),
                    "query_context": query_context,
                }),
            )
            .await?;
        rows = match response.payload {
            Value::Array(array) => array,
            value => vec![value],
        };
    }

    Ok(rows)
}
