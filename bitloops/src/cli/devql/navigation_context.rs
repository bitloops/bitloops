use super::*;

const NAVIGATION_CONTEXT_STATUS_QUERY: &str = r#"
query NavigationContextStatus($path: String!, $filter: NavigationContextFilterInput) {
  project(path: $path) {
    navigationContext(filter: $filter) {
      totalViews
      totalPrimitives
      totalEdges
      views {
        viewId
        viewKind
        label
        acceptedSignature
        currentSignature
        status
        staleReason
        materialisedRef
        updatedAt
        acceptanceHistory {
          acceptanceId
          acceptedSignature
          previousAcceptedSignature
          currentSignature
          expectedCurrentSignature
          source
          reason
          materialisedRef
          acceptedAt
        }
      }
    }
  }
}
"#;

const ACCEPT_NAVIGATION_CONTEXT_VIEW_MUTATION: &str = r#"
mutation AcceptNavigationContextView($input: AcceptNavigationContextViewInput!) {
  acceptNavigationContextView(input: $input) {
    success
    acceptanceId
    viewId
    previousAcceptedSignature
    acceptedSignature
    currentSignature
    status
    source
    reason
    materialisedRef
    acceptedAt
  }
}
"#;

const MATERIALISE_NAVIGATION_CONTEXT_VIEW_MUTATION: &str = r#"
mutation MaterialiseNavigationContextView($input: MaterialiseNavigationContextViewInput!) {
  materialiseNavigationContextView(input: $input) {
    success
    materialisationId
    materialisedRef
    viewId
    viewKind
    label
    acceptedSignature
    currentSignature
    status
    materialisationFormat
    materialisationVersion
    payload
    renderedText
    primitiveCount
    edgeCount
    materialisedAt
  }
}
"#;

pub(super) fn format_schema_sdl_output(args: &DevqlSchemaArgs, sdl: &str) -> String {
    if args.human {
        sdl.to_string()
    } else {
        minify_schema_sdl(sdl)
    }
}

pub(super) async fn write_schema_sdl<F, W>(
    args: &DevqlSchemaArgs,
    writer: &mut W,
    discover_scope: F,
) -> Result<()>
where
    F: FnOnce() -> Result<SlimCliRepoScope>,
    W: Write,
{
    let sdl = if args.global {
        graphql::fetch_global_schema_sdl_via_daemon().await?
    } else {
        let scope = discover_scope().map_err(map_schema_scope_error)?;
        graphql::fetch_slim_schema_sdl_via_daemon(&scope).await?
    };

    writer
        .write_all(format_schema_sdl_output(args, &sdl).as_bytes())
        .context("writing DevQL schema SDL")
}

pub(super) fn map_schema_scope_error(err: anyhow::Error) -> anyhow::Error {
    if is_repo_root_discovery_error(&err) {
        anyhow!(SCHEMA_SCOPE_REQUIRED_MESSAGE)
    } else {
        err
    }
}

pub(super) fn minify_schema_sdl(sdl: &str) -> String {
    #[derive(Copy, Clone, Eq, PartialEq)]
    enum State {
        Normal,
        String,
        BlockString,
    }

    fn starts_with_triple_quotes(chars: &[char], index: usize) -> bool {
        chars.get(index) == Some(&'"')
            && chars.get(index + 1) == Some(&'"')
            && chars.get(index + 2) == Some(&'"')
    }

    fn push_pending_space(
        output: &mut String,
        next: char,
        pending_space: &mut bool,
        last_emitted: &mut Option<char>,
    ) {
        if !*pending_space || last_emitted.is_none() {
            *pending_space = false;
            return;
        }

        let previous = *last_emitted;
        if previous == Some('{')
            || next == '}'
            || matches!(previous, Some(' ' | '\n' | '\r' | '\t'))
        {
            *pending_space = false;
            return;
        }

        output.push(' ');
        *last_emitted = Some(' ');
        *pending_space = false;
    }

    let chars = sdl.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(sdl.len());
    let mut state = State::Normal;
    let mut pending_space = false;
    let mut last_emitted = None;
    let mut index = 0usize;

    while index < chars.len() {
        match state {
            State::Normal => {
                if starts_with_triple_quotes(&chars, index) {
                    push_pending_space(&mut output, '"', &mut pending_space, &mut last_emitted);
                    output.push_str("\"\"\"");
                    last_emitted = Some('"');
                    index += 3;
                    state = State::BlockString;
                } else if chars[index] == '"' {
                    push_pending_space(&mut output, '"', &mut pending_space, &mut last_emitted);
                    output.push('"');
                    last_emitted = Some('"');
                    index += 1;
                    state = State::String;
                } else if chars[index].is_whitespace() {
                    pending_space = true;
                    index += 1;
                } else {
                    push_pending_space(
                        &mut output,
                        chars[index],
                        &mut pending_space,
                        &mut last_emitted,
                    );
                    output.push(chars[index]);
                    last_emitted = Some(chars[index]);
                    index += 1;
                }
            }
            State::String => {
                let ch = chars[index];
                output.push(ch);
                last_emitted = Some(ch);
                index += 1;
                if ch == '\\' {
                    if let Some(next) = chars.get(index) {
                        output.push(*next);
                        last_emitted = Some(*next);
                        index += 1;
                    }
                } else if ch == '"' {
                    state = State::Normal;
                }
            }
            State::BlockString => {
                if starts_with_triple_quotes(&chars, index) {
                    output.push_str("\"\"\"");
                    last_emitted = Some('"');
                    index += 3;
                    state = State::Normal;
                } else {
                    output.push(chars[index]);
                    last_emitted = Some(chars[index]);
                    index += 1;
                }
            }
        }
    }

    if !output.ends_with('\n') {
        output.push('\n');
    }

    output
}

pub(super) async fn run_with_scope_discovery<F, W>(
    args: DevqlArgs,
    schema_writer: &mut W,
    discover_scope: F,
) -> Result<()>
where
    F: FnOnce() -> Result<SlimCliRepoScope>,
    W: Write,
{
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    let command = match command {
        DevqlCommand::Schema(args) => {
            return write_schema_sdl(&args, schema_writer, discover_scope).await;
        }
        DevqlCommand::ConnectionStatus(_) => return run_connection_status().await,
        command => command,
    };

    let scope = discover_scope()?;
    let repo_root = scope.repo_root.clone();
    let repo = scope.repo.clone();

    if let DevqlCommand::Knowledge(args) = command {
        return match args.command {
            DevqlKnowledgeCommand::Add(add) => {
                knowledge::run_knowledge_add_via_graphql(&scope, &add.url, add.commit.as_deref())
                    .await
            }
            DevqlKnowledgeCommand::Associate(associate) => {
                knowledge::run_knowledge_associate_via_graphql(
                    &scope,
                    &associate.source_ref,
                    &associate.target_ref,
                )
                .await
            }
            DevqlKnowledgeCommand::Refresh(refresh) => {
                knowledge::run_knowledge_refresh_via_graphql(&scope, &refresh.knowledge_ref).await
            }
            DevqlKnowledgeCommand::Versions(versions) => {
                run_knowledge_versions_via_host(&repo_root, &repo, &versions.knowledge_ref).await
            }
        };
    }

    if let DevqlCommand::TestHarness(args) = command {
        return test_harness::run(args, &repo_root).await;
    }

    let cfg = DevqlConfig::from_env(repo_root, repo)?;

    match command {
        DevqlCommand::Init(_) => graphql::run_init_via_graphql(&scope).await,
        DevqlCommand::Analytics(args) => match args.command {
            DevqlAnalyticsCommand::Sql(sql) => {
                let scope = if sql.all_repos {
                    AnalyticsRepoScope::AllKnown
                } else if sql.repos.is_empty() {
                    AnalyticsRepoScope::CurrentRepo
                } else {
                    AnalyticsRepoScope::Explicit(sql.repos.clone())
                };
                let result = execute_analytics_sql(&cfg, scope, &sql.query).await?;
                if sql.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&result)
                            .context("serialising analytics SQL result to JSON")?
                    );
                } else {
                    println!("{}", format_analytics_sql_result_table(&result));
                }
                Ok(())
            }
        },
        DevqlCommand::Tasks(args) => run_tasks_command(&scope, args).await,
        DevqlCommand::Projection(args) => match args.command {
            DevqlProjectionCommand::CheckpointFileSnapshots(backfill) => {
                run_checkpoint_file_snapshot_backfill(
                    &cfg,
                    CheckpointFileSnapshotBackfillOptions {
                        batch_size: backfill.batch_size,
                        max_checkpoints: backfill.max_checkpoints,
                        resume_after: backfill.resume_after,
                        dry_run: backfill.dry_run,
                        emit_progress: true,
                    },
                )
                .await
            }
        },
        DevqlCommand::Query(args) => {
            let use_raw_graphql = use_raw_graphql_mode(&args.query, args.graphql);
            let parsed_query = (!use_raw_graphql)
                .then(|| parse_devql_query(&args.query))
                .transpose()?;
            let trace = crate::devql_timing::timings_enabled_from_env()
                .then(crate::devql_timing::TimingTrace::new);

            let compile_started = Instant::now();
            let document = compile_slim_query_document(&args.query, args.graphql, &scope)?;
            if let Some(trace) = trace.as_ref() {
                trace.record(
                    "cli.devql.compile_query_document",
                    compile_started.elapsed(),
                    json!({
                        "inputBytes": args.query.len(),
                        "rawGraphql": use_raw_graphql,
                    }),
                );
            }

            let execute_started = Instant::now();
            let data: serde_json::Value = match crate::daemon::execute_slim_graphql(
                &cfg.repo_root,
                &scope,
                &document,
                serde_json::json!({}),
            )
            .await
            {
                Ok(data) => {
                    if let Some(trace) = trace.as_ref() {
                        trace.record(
                            "cli.devql.execute_graphql",
                            execute_started.elapsed(),
                            Value::Null,
                        );
                    }
                    data
                }
                Err(err) => {
                    if let Some(trace) = trace.as_ref() {
                        trace.record(
                            "cli.devql.execute_graphql",
                            execute_started.elapsed(),
                            json!({
                                "error": format!("{err:#}"),
                            }),
                        );
                        crate::devql_timing::print_summary("cli", &trace.summary_value());
                    }
                    return Err(err);
                }
            };

            let format_started = Instant::now();
            let output = match format_query_output(
                &data,
                args.compact,
                use_raw_graphql,
                parsed_query.as_ref(),
            ) {
                Ok(output) => {
                    if let Some(trace) = trace.as_ref() {
                        trace.record(
                            "cli.devql.format_query_output",
                            format_started.elapsed(),
                            json!({
                                "compact": args.compact,
                                "outputBytes": output.len(),
                            }),
                        );
                    }
                    output
                }
                Err(err) => {
                    if let Some(trace) = trace.as_ref() {
                        trace.record(
                            "cli.devql.format_query_output",
                            format_started.elapsed(),
                            json!({
                                "compact": args.compact,
                                "error": format!("{err:#}"),
                            }),
                        );
                        crate::devql_timing::print_summary("cli", &trace.summary_value());
                    }
                    return Err(err);
                }
            };
            println!("{output}");
            if let Some(trace) = trace.as_ref() {
                crate::devql_timing::print_summary("cli", &trace.summary_value());
            }
            Ok(())
        }
        DevqlCommand::Packs(args) => run_capability_packs_report(
            &cfg,
            args.json,
            args.apply_migrations,
            args.with_health,
            args.with_extensions,
        ),
        DevqlCommand::NavigationContext(args) => run_navigation_context_command(&scope, args).await,
        DevqlCommand::Schema(_) => unreachable!("handled before repo setup"),
        DevqlCommand::ConnectionStatus(_) => unreachable!("handled before repo setup"),
        DevqlCommand::Architecture(_) => unreachable!("handled before repo setup"),
        DevqlCommand::Knowledge(_) => unreachable!("handled before cfg setup"),
        DevqlCommand::TestHarness(_) => unreachable!("handled before cfg setup"),
    }
}

pub(super) async fn run_navigation_context_command(
    scope: &SlimCliRepoScope,
    args: DevqlNavigationContextArgs,
) -> Result<()> {
    match args.command {
        DevqlNavigationContextCommand::Status(args) => {
            run_navigation_context_status(scope, args).await
        }
        DevqlNavigationContextCommand::Materialise(args) => {
            run_navigation_context_materialise(scope, args).await
        }
        DevqlNavigationContextCommand::Accept(args) => {
            run_navigation_context_accept(scope, args).await
        }
    }
}

pub(super) async fn run_navigation_context_status(
    scope: &SlimCliRepoScope,
    args: DevqlNavigationContextStatusArgs,
) -> Result<()> {
    let DevqlNavigationContextStatusArgs {
        project,
        view,
        status,
        json,
        changed_limit,
    } = args;
    let response: Value = graphql::execute_devql_graphql(
        scope,
        NAVIGATION_CONTEXT_STATUS_QUERY,
        json!({
            "path": project,
            "filter": navigation_context_filter_json(view.as_deref(), status),
        }),
    )
    .await?;
    let snapshot = response
        .get("project")
        .and_then(|project| project.get("navigationContext"))
        .ok_or_else(|| anyhow!("DevQL response did not include project.navigationContext"))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(snapshot)
                .context("serialising navigation context status to JSON")?
        );
    } else {
        println!(
            "{}",
            format_navigation_context_status(snapshot, changed_limit)
        );
    }
    Ok(())
}

pub(super) async fn run_navigation_context_accept(
    scope: &SlimCliRepoScope,
    args: DevqlNavigationContextAcceptArgs,
) -> Result<()> {
    let DevqlNavigationContextAcceptArgs {
        view_id,
        expected_current_signature,
        reason,
        materialised_ref,
        json,
    } = args;
    let response: Value = graphql::execute_devql_graphql(
        scope,
        ACCEPT_NAVIGATION_CONTEXT_VIEW_MUTATION,
        json!({
            "input": {
                "viewId": view_id,
                "expectedCurrentSignature": expected_current_signature,
                "source": "manual_cli",
                "reason": reason,
                "materialisedRef": materialised_ref,
            },
        }),
    )
    .await?;
    let result = response
        .get("acceptNavigationContextView")
        .ok_or_else(|| anyhow!("DevQL response did not include acceptNavigationContextView"))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(result)
                .context("serialising navigation context acceptance to JSON")?
        );
    } else {
        println!("{}", format_navigation_context_acceptance(result));
    }
    Ok(())
}

pub(super) async fn run_navigation_context_materialise(
    scope: &SlimCliRepoScope,
    args: DevqlNavigationContextMaterialiseArgs,
) -> Result<()> {
    let DevqlNavigationContextMaterialiseArgs {
        view_id,
        expected_current_signature,
        json,
        rendered,
    } = args;
    let response: Value = graphql::execute_devql_graphql(
        scope,
        MATERIALISE_NAVIGATION_CONTEXT_VIEW_MUTATION,
        json!({
            "input": {
                "viewId": view_id,
                "expectedCurrentSignature": expected_current_signature,
            },
        }),
    )
    .await?;
    let result = response
        .get("materialiseNavigationContextView")
        .ok_or_else(|| {
            anyhow!("DevQL response did not include materialiseNavigationContextView")
        })?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(result)
                .context("serialising navigation context materialisation to JSON")?
        );
    } else if rendered {
        println!("{}", value_str(result, "renderedText").unwrap_or_default());
    } else {
        println!("{}", format_navigation_context_materialisation(result));
    }
    Ok(())
}

pub(super) fn navigation_context_filter_json(
    view_id: Option<&str>,
    status: Option<DevqlNavigationContextStatusArg>,
) -> Value {
    let mut filter = serde_json::Map::new();
    if let Some(view_id) = view_id {
        filter.insert("viewId".to_string(), json!(view_id));
    }
    if let Some(status) = status {
        filter.insert(
            "viewStatus".to_string(),
            json!(navigation_context_status_arg_name(status)),
        );
    }
    if filter.is_empty() {
        Value::Null
    } else {
        Value::Object(filter)
    }
}

pub(super) fn navigation_context_status_arg_name(
    status: DevqlNavigationContextStatusArg,
) -> &'static str {
    match status {
        DevqlNavigationContextStatusArg::Fresh => "FRESH",
        DevqlNavigationContextStatusArg::Stale => "STALE",
    }
}

pub(super) fn format_navigation_context_status(snapshot: &Value, changed_limit: usize) -> String {
    let views = snapshot
        .get("views")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let stale_count = views
        .iter()
        .filter(|view| {
            view.get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| status == "STALE")
        })
        .count();
    let total_views = snapshot
        .get("totalViews")
        .and_then(Value::as_i64)
        .unwrap_or(views.len() as i64);
    let total_primitives = snapshot
        .get("totalPrimitives")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let total_edges = snapshot
        .get("totalEdges")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let mut lines = vec![format!(
        "navigation context: {total_views} views, {stale_count} stale, {total_primitives} primitives, {total_edges} edges"
    )];
    if views.is_empty() {
        lines.push("no navigation context views found".to_string());
        return lines.join("\n");
    }

    for view in views {
        lines.push(format_navigation_context_view(view, changed_limit));
    }
    lines.join("\n")
}

pub(super) fn format_navigation_context_view(view: &Value, changed_limit: usize) -> String {
    let view_id = value_str(view, "viewId").unwrap_or("<unknown>");
    let label = value_str(view, "label").unwrap_or(view_id);
    let status = value_str(view, "status")
        .unwrap_or("UNKNOWN")
        .to_ascii_lowercase();
    let current = short_signature(value_str(view, "currentSignature"));
    let accepted = short_signature(value_str(view, "acceptedSignature"));
    let materialised_ref = value_str(view, "materialisedRef");
    let mut lines = vec![format!(
        "- {view_id} [{status}] {label} current={current} accepted={accepted}"
    )];
    if let Some(materialised_ref) = materialised_ref {
        lines.push(format!("  materialised: {materialised_ref}"));
    }
    if let Some(latest_acceptance) = view
        .get("acceptanceHistory")
        .and_then(Value::as_array)
        .and_then(|history| history.first())
    {
        lines.push(format!(
            "  last accepted: {} by {}",
            value_str(latest_acceptance, "acceptedAt").unwrap_or("<unknown>"),
            value_str(latest_acceptance, "source").unwrap_or("<unknown>")
        ));
    }

    let changes = view
        .get("staleReason")
        .and_then(|reason| reason.get("changedPrimitives"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if changes.is_empty() {
        return lines.join("\n");
    }

    for change in changes.iter().take(changed_limit) {
        lines.push(format!("  {}", format_navigation_context_change(change)));
    }
    if changes.len() > changed_limit {
        lines.push(format!(
            "  ... {} more changed primitives",
            changes.len() - changed_limit
        ));
    }
    lines.join("\n")
}

pub(super) fn format_navigation_context_change(change: &Value) -> String {
    let kind = value_str(change, "primitiveKind").unwrap_or("UNKNOWN");
    let label = value_str(change, "label").unwrap_or("<unlabelled>");
    let path = value_str(change, "path");
    let change_kind = value_str(change, "changeKind").unwrap_or("changed");
    let previous = short_signature(value_str(change, "previousHash"));
    let current = short_signature(value_str(change, "currentHash"));
    match path {
        Some(path) => {
            format!("{change_kind}: {kind} {label} ({path}) {previous}->{current}")
        }
        None => format!("{change_kind}: {kind} {label} {previous}->{current}"),
    }
}

pub(super) fn format_navigation_context_acceptance(result: &Value) -> String {
    let view_id = value_str(result, "viewId").unwrap_or("<unknown>");
    let status = value_str(result, "status")
        .unwrap_or("UNKNOWN")
        .to_ascii_lowercase();
    let previous = short_signature(value_str(result, "previousAcceptedSignature"));
    let accepted = short_signature(value_str(result, "acceptedSignature"));
    let mut line = format!(
        "navigation context view accepted: {view_id} status={status} previous={previous} accepted={accepted}"
    );
    if let Some(materialised_ref) = value_str(result, "materialisedRef") {
        line.push_str(&format!(" materialised={materialised_ref}"));
    }
    if let Some(accepted_at) = value_str(result, "acceptedAt") {
        line.push_str(&format!(" accepted_at={accepted_at}"));
    }
    line
}

pub(super) fn format_navigation_context_materialisation(result: &Value) -> String {
    let view_id = value_str(result, "viewId").unwrap_or("<unknown>");
    let status = value_str(result, "status")
        .unwrap_or("UNKNOWN")
        .to_ascii_lowercase();
    let current = short_signature(value_str(result, "currentSignature"));
    let materialised_ref = value_str(result, "materialisedRef").unwrap_or("<none>");
    let primitive_count = result
        .get("primitiveCount")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let edge_count = result
        .get("edgeCount")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let materialised_at = value_str(result, "materialisedAt").unwrap_or("<unknown>");
    format!(
        "navigation context view materialised: {view_id} status={status} current={current} primitives={primitive_count} edges={edge_count} ref={materialised_ref} materialised_at={materialised_at}"
    )
}

pub(super) fn value_str<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

pub(super) fn short_signature(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "<none>".to_string();
    };
    value.chars().take(12).collect()
}
