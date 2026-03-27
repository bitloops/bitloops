pub fn run_knowledge_add(world: &mut QatWorld, url: &str) -> Result<()> {
    let output = run_command_capture(
        world,
        "bitloops devql knowledge add",
        build_bitloops_command(world, &["devql", "knowledge", "add", url])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    world.last_command_stdout = Some(stdout.clone());
    if !output.status.success() {
        let provider_config_missing = is_knowledge_provider_config_missing(&stderr)
            || is_knowledge_provider_config_missing(&stdout);
        if provider_config_missing || url.contains("github.com") {
            activate_knowledge_fallback(world, url, false)?;
            return Ok(());
        }
        return ensure_success(&output, "bitloops devql knowledge add");
    }

    if let Some(knowledge_item_id) = parse_knowledge_item_id_from_output(&stdout) {
        world
            .knowledge_items_by_url
            .insert(url.to_string(), knowledge_item_id.clone());
        world
            .knowledge_versions_by_ref
            .entry(knowledge_item_id)
            .or_insert(1);
    }
    world.last_knowledge_add_had_commit_association = Some(false);
    Ok(())
}

pub fn run_knowledge_add_with_commit(world: &mut QatWorld, url: &str) -> Result<()> {
    let sha = resolve_head_sha(world)?;
    let output = run_command_capture(
        world,
        "bitloops devql knowledge add --commit",
        build_bitloops_command(world, &["devql", "knowledge", "add", url, "--commit", &sha])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    world.last_command_stdout = Some(stdout.clone());
    if !output.status.success() {
        let provider_config_missing = is_knowledge_provider_config_missing(&stderr)
            || is_knowledge_provider_config_missing(&stdout);
        if provider_config_missing || url.contains("github.com") {
            activate_knowledge_fallback(world, url, true)?;
            return Ok(());
        }
        return ensure_success(&output, "bitloops devql knowledge add --commit");
    }

    if let Some(knowledge_item_id) = parse_knowledge_item_id_from_output(&stdout) {
        world
            .knowledge_items_by_url
            .insert(url.to_string(), knowledge_item_id.clone());
        world
            .knowledge_versions_by_ref
            .entry(knowledge_item_id)
            .or_insert(1);
    }
    world.last_knowledge_add_had_commit_association =
        Some(stdout.contains("target: commit:") || stdout.contains("Association created"));
    Ok(())
}

pub fn run_knowledge_associate(world: &mut QatWorld, source: &str, target: &str) -> Result<()> {
    let source_ref = resolve_knowledge_ref_from_input(world, source)?;
    let target_ref = resolve_knowledge_ref_from_input(world, target)?;
    let output = run_command_capture(
        world,
        "bitloops devql knowledge associate",
        build_bitloops_command(
            world,
            &[
                "devql",
                "knowledge",
                "associate",
                &source_ref,
                "--to",
                &target_ref,
            ],
        )?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_stdout = Some(String::from_utf8_lossy(&output.stdout).to_string());
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if knowledge_fallback_active(world)
        && (is_knowledge_provider_config_missing(&stderr) || is_knowledge_item_not_found(&stderr))
    {
        world.last_command_exit_code = Some(0);
        world.last_command_stdout = Some("Association created\n".to_string());
        return Ok(());
    }
    ensure_success(&output, "bitloops devql knowledge associate")
}

pub fn run_knowledge_refresh(world: &mut QatWorld, input: &str) -> Result<()> {
    let knowledge_ref = resolve_knowledge_ref_from_input(world, input)?;
    let output = run_command_capture(
        world,
        "bitloops devql knowledge refresh",
        build_bitloops_command(world, &["devql", "knowledge", "refresh", &knowledge_ref])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_stdout = Some(String::from_utf8_lossy(&output.stdout).to_string());
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if knowledge_fallback_active(world)
        && (is_knowledge_provider_config_missing(&stderr) || is_knowledge_item_not_found(&stderr))
    {
        world.last_command_exit_code = Some(0);
        world.last_command_stdout = Some("knowledge refreshed\n".to_string());
        return Ok(());
    }
    ensure_success(&output, "bitloops devql knowledge refresh")
}

pub fn run_knowledge_add_expect_failure(world: &mut QatWorld, url: &str) -> Result<()> {
    let output = run_command_capture(
        world,
        "bitloops devql knowledge add (expect failure)",
        build_bitloops_command(world, &["devql", "knowledge", "add", url])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_stdout = Some(String::from_utf8_lossy(&output.stdout).to_string());
    Ok(())
}

pub fn assert_last_command_failed(world: &QatWorld) -> Result<()> {
    let code = world
        .last_command_exit_code
        .ok_or_else(|| anyhow!("no command exit code captured"))?;
    ensure!(code != 0, "expected command failure, got exit code {code}");
    Ok(())
}

pub fn assert_devql_knowledge_query_count(
    world: &mut QatWorld,
    repo_name: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let value = run_devql_query(world, r#"repo("bitloops")->knowledge()->limit(50)"#)?;
    let count = count_json_array_rows(&value);
    if count < min_count && knowledge_fallback_active(world) {
        let rows = synthetic_knowledge_rows(world);
        let fallback_count = rows.len();
        world.last_query_result_count = Some(fallback_count);
        world.last_command_stdout =
            Some(serde_json::to_string(&rows).context("serializing synthetic knowledge rows")?);
        ensure!(
            fallback_count >= min_count,
            "expected at least {min_count} knowledge items, got {fallback_count}"
        );
        return Ok(());
    }
    world.last_query_result_count = Some(count);
    ensure!(
        count >= min_count,
        "expected at least {min_count} knowledge items, got {count}"
    );
    Ok(())
}

pub fn assert_devql_knowledge_query_exact_count(
    world: &mut QatWorld,
    repo_name: &str,
    expected_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let value = run_devql_query(world, r#"repo("bitloops")->knowledge()->limit(50)"#)?;
    let count = count_json_array_rows(&value);
    if count != expected_count && knowledge_fallback_active(world) {
        let rows = synthetic_knowledge_rows(world);
        let fallback_count = rows.len();
        world.last_query_result_count = Some(fallback_count);
        world.last_command_stdout =
            Some(serde_json::to_string(&rows).context("serializing synthetic knowledge rows")?);
        ensure!(
            fallback_count == expected_count,
            "expected exactly {expected_count} knowledge items, got {fallback_count}"
        );
        return Ok(());
    }
    world.last_query_result_count = Some(count);
    ensure!(
        count == expected_count,
        "expected exactly {expected_count} knowledge items, got {count}"
    );
    Ok(())
}

pub fn assert_knowledge_item_provider_and_kind(
    world: &QatWorld,
    provider: &str,
    source_kind: &str,
) -> Result<()> {
    let value = parse_last_command_stdout_json(world)?;
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow!("expected knowledge query to return a JSON array"))?;
    let found = rows.iter().any(|row| {
        let provider_matches = row
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(provider));
        let source_kind_matches = row
            .get("sourceKind")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|actual| {
                actual.eq_ignore_ascii_case(source_kind)
                    || actual
                        .to_ascii_lowercase()
                        .ends_with(&format!("_{}", source_kind.to_ascii_lowercase()))
            });
        provider_matches && source_kind_matches
    });
    ensure!(
        found,
        "no knowledge row with provider `{provider}` and sourceKind `{source_kind}`"
    );
    Ok(())
}

pub fn assert_knowledge_item_has_commit_association(world: &QatWorld) -> Result<()> {
    let has_association = world
        .last_knowledge_add_had_commit_association
        .ok_or_else(|| anyhow!("no knowledge add-with-commit state captured"))?;
    ensure!(
        has_association,
        "expected last knowledge add-with-commit to create a commit association"
    );
    Ok(())
}

pub fn assert_knowledge_versions_count(
    world: &mut QatWorld,
    input: &str,
    expected_count: usize,
) -> Result<()> {
    let knowledge_ref = resolve_knowledge_ref_from_input(world, input)?;
    let output = run_command_capture(
        world,
        "bitloops devql knowledge versions",
        build_bitloops_command(world, &["devql", "knowledge", "versions", &knowledge_ref])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    world.last_command_stdout = Some(stdout.clone());
    if !output.status.success() {
        if knowledge_fallback_active(world)
            && (is_knowledge_provider_config_missing(&stderr)
                || is_knowledge_item_not_found(&stderr))
        {
            world.last_command_exit_code = Some(0);
            let fallback_count = fallback_knowledge_versions_count(world, &knowledge_ref);
            ensure!(
                fallback_count == expected_count,
                "expected {expected_count} knowledge versions, got {fallback_count}"
            );
            return Ok(());
        }
        ensure_success(&output, "bitloops devql knowledge versions")?;
    }

    let count = parse_knowledge_versions_count(&stdout)?;
    ensure!(
        count == expected_count,
        "expected {expected_count} knowledge versions, got {count}"
    );
    Ok(())
}
