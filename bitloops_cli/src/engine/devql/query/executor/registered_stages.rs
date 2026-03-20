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

async fn execute_registered_stages(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    mut rows: Vec<Value>,
) -> Result<Vec<Value>> {
    if parsed.registered_stages.is_empty() {
        return Ok(rows);
    }

    let mut host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
    for stage in &parsed.registered_stages {
        let capability_id = resolve_registered_stage_owner(&host, &stage.stage_name)?;

        let response = host
            .invoke_stage(
                capability_id,
                &stage.stage_name,
                json!({
                    "input_rows": rows,
                    "args": stage.args,
                    "limit": parsed.limit.max(1),
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
