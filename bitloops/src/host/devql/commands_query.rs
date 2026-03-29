use super::*;
use std::fmt::Write as _;

pub async fn run_query(
    cfg: &DevqlConfig,
    query: &str,
    compact: bool,
    raw_graphql: bool,
) -> Result<()> {
    let use_raw_graphql = use_raw_graphql_mode(query, raw_graphql);
    let document = compile_query_document(query, raw_graphql)?;
    let data =
        crate::graphql::execute_in_process(cfg.repo_root.clone(), &document, json!({})).await?;
    let output = format_query_output(&data, compact, use_raw_graphql)?;
    println!("{output}");

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
    let backends = resolve_store_backend_config_for_repo(&cfg.config_root)
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

pub(crate) fn compile_query_document(query: &str, raw_graphql: bool) -> Result<String> {
    compile_query_document_for_mode(query, raw_graphql, GraphqlCompileMode::Global)
}

pub(crate) fn compile_query_document_for_mode(
    query: &str,
    raw_graphql: bool,
    mode: GraphqlCompileMode,
) -> Result<String> {
    if use_raw_graphql_mode(query, raw_graphql) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            bail!("empty GraphQL query");
        }
        return Ok(trimmed.to_string());
    }

    let parsed = parse_devql_query(query)?;
    compile_devql_to_graphql_with_mode(&parsed, mode)
}

fn looks_like_devql_pipeline(query: &str) -> bool {
    query.contains("->")
}

pub(crate) fn use_raw_graphql_mode(query: &str, raw_graphql: bool) -> bool {
    raw_graphql || !looks_like_devql_pipeline(query)
}

pub(crate) fn format_query_output(
    data: &Value,
    compact: bool,
    raw_graphql: bool,
) -> Result<String> {
    if raw_graphql {
        return if compact {
            Ok(serde_json::to_string(data)?)
        } else {
            Ok(serde_json::to_string_pretty(data)?)
        };
    }

    let payload = extract_cli_payload(data);
    if compact {
        return Ok(serde_json::to_string(&payload)?);
    }

    render_cli_payload(&payload)
}

fn extract_cli_payload(data: &Value) -> Value {
    let mut current = data;

    loop {
        let Value::Object(map) = current else {
            return current.clone();
        };

        if let Some(nodes) = extract_connection_nodes(map) {
            return Value::Array(nodes);
        }

        let mut non_null_values = map.values().filter(|value| !value.is_null());
        let Some(next) = non_null_values.next() else {
            return Value::Null;
        };
        if non_null_values.next().is_some() {
            return current.clone();
        }

        current = next;
    }
}

fn extract_connection_nodes(map: &serde_json::Map<String, Value>) -> Option<Vec<Value>> {
    let edges = map.get("edges")?.as_array()?;
    edges
        .iter()
        .map(|edge| edge.as_object()?.get("node").cloned())
        .collect()
}

fn render_cli_payload(payload: &Value) -> Result<String> {
    match payload {
        Value::Array(rows) => render_array_rows(rows),
        Value::Object(row) => render_object_rows(&[row]),
        Value::Null => Ok("No results.".to_string()),
        other => Ok(render_scalar_cell(other)),
    }
}

fn render_array_rows(rows: &[Value]) -> Result<String> {
    if rows.is_empty() {
        return Ok("No results.".to_string());
    }

    if rows.iter().all(is_scalar_like) {
        let values = rows
            .iter()
            .map(|row| vec![render_scalar_cell(row)])
            .collect::<Vec<_>>();
        return Ok(render_table(&["value".to_string()], &values));
    }

    if rows.iter().all(Value::is_object) {
        let objects = rows
            .iter()
            .map(|row| row.as_object().expect("checked above"))
            .collect::<Vec<_>>();
        return render_object_rows(&objects);
    }

    Ok(serde_json::to_string_pretty(rows)?)
}

fn render_object_rows(rows: &[&serde_json::Map<String, Value>]) -> Result<String> {
    let columns = collect_table_columns(rows);
    if columns.is_empty() {
        return Ok("No results.".to_string());
    }

    let values = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| row.get(column).map(render_table_cell).unwrap_or_default())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let headers = columns
        .iter()
        .map(|column| cli_header(column))
        .collect::<Vec<_>>();
    Ok(render_table(&headers, &values))
}

fn collect_table_columns(rows: &[&serde_json::Map<String, Value>]) -> Vec<String> {
    let mut columns = Vec::new();
    for row in rows {
        for key in row.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    columns
}

fn render_table(headers: &[String], rows: &[Vec<String>]) -> String {
    let widths = column_widths(headers, rows);
    let mut out = String::new();

    writeln!(&mut out, "{}", horizontal_rule(&widths)).expect("writing to string should succeed");
    writeln!(
        &mut out,
        "{}",
        render_row(
            headers
                .iter()
                .zip(widths.iter())
                .map(|(value, width)| pad_cell(value, *width))
                .collect::<Vec<_>>()
        )
    )
    .expect("writing to string should succeed");
    writeln!(&mut out, "{}", horizontal_rule(&widths)).expect("writing to string should succeed");

    for row in rows {
        writeln!(
            &mut out,
            "{}",
            render_row(
                row.iter()
                    .zip(widths.iter())
                    .map(|(value, width)| pad_cell(value, *width))
                    .collect::<Vec<_>>()
            )
        )
        .expect("writing to string should succeed");
    }

    write!(&mut out, "{}", horizontal_rule(&widths)).expect("writing to string should succeed");
    out
}

fn column_widths(headers: &[String], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths = headers
        .iter()
        .map(|header| header.chars().count())
        .collect::<Vec<_>>();
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(value.chars().count());
        }
    }
    widths
}

fn horizontal_rule(widths: &[usize]) -> String {
    let mut out = String::new();
    out.push('+');
    for width in widths {
        out.push_str(&"-".repeat(*width + 2));
        out.push('+');
    }
    out
}

fn render_row(cells: Vec<String>) -> String {
    let mut out = String::new();
    out.push('|');
    for cell in cells {
        out.push(' ');
        out.push_str(&cell);
        out.push(' ');
        out.push('|');
    }
    out
}

fn pad_cell(value: &str, width: usize) -> String {
    format!("{value:<width$}")
}

fn render_table_cell(value: &Value) -> String {
    let rendered = match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Array(items) => {
            if items.is_empty() {
                String::new()
            } else if items.iter().all(is_scalar_like) {
                items
                    .iter()
                    .map(render_scalar_cell)
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                format!("[{} entries]", items.len())
            }
        }
        Value::Object(map) => {
            if let Some(nodes) = extract_connection_nodes(map) {
                format!("[{} entries]", nodes.len())
            } else {
                serde_json::to_string(value).unwrap_or_default()
            }
        }
    };

    truncate_cell(&rendered.replace(['\n', '\r'], " "))
}

fn render_scalar_cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        _ => render_table_cell(value),
    }
}

fn is_scalar_like(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::String(_) | Value::Number(_) | Value::Bool(_)
    )
}

fn truncate_cell(value: &str) -> String {
    const MAX_CELL_CHARS: usize = 80;

    let mut chars = value.chars();
    let truncated = chars.by_ref().take(MAX_CELL_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn cli_header(name: &str) -> String {
    let mut header = String::with_capacity(name.len());
    for (index, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            header.push('_');
            header.push(ch.to_ascii_lowercase());
        } else {
            header.push(ch.to_ascii_lowercase());
        }
    }
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_query_document_compiles_devql_pipeline() {
        let document =
            compile_query_document(r#"repo("bitloops-cli")->artefacts()->limit(2)"#, false)
                .expect("dsl query should compile");

        assert!(document.contains("repo(name: \"bitloops-cli\")"));
        assert!(document.contains("artefacts(first: 2)"));
    }

    #[test]
    fn compile_query_document_preserves_raw_graphql() {
        let document = compile_query_document(" { repo(name: \"bitloops-cli\") { name } } ", true)
            .expect("raw graphql should pass through");

        assert_eq!(document, "{ repo(name: \"bitloops-cli\") { name } }");
    }

    #[test]
    fn compile_query_document_defaults_to_raw_graphql_without_pipeline_operator() {
        let document = compile_query_document("{ repo(name: \"bitloops-cli\") { name } }", false)
            .expect("graphql should be the default mode");

        assert_eq!(document, "{ repo(name: \"bitloops-cli\") { name } }");
    }

    #[test]
    fn use_raw_graphql_mode_treats_pipeline_operator_as_dsl_only() {
        assert!(!use_raw_graphql_mode(
            r#"repo("bitloops-cli")->artefacts()->limit(2)"#,
            false
        ));
        assert!(use_raw_graphql_mode(
            r#"{ repo(name: "bitloops-cli") { name } }"#,
            false
        ));
        assert!(use_raw_graphql_mode(
            r#"repo("bitloops-cli")->artefacts()->limit(2)"#,
            true
        ));
    }

    #[test]
    fn extract_cli_payload_unwraps_connection_nodes_through_scopes() {
        let payload = extract_cli_payload(&json!({
            "repo": {
                "project": {
                    "artefacts": {
                        "edges": [
                            { "node": { "path": "src/main.rs", "symbolFqn": "main" } },
                            { "node": { "path": "src/lib.rs", "symbolFqn": "answer" } }
                        ]
                    }
                }
            }
        }));

        assert_eq!(
            payload,
            json!([
                { "path": "src/main.rs", "symbolFqn": "main" },
                { "path": "src/lib.rs", "symbolFqn": "answer" }
            ])
        );
    }

    #[test]
    fn extract_cli_payload_preserves_typed_project_stage_lists() {
        let payload = extract_cli_payload(&json!({
            "repo": {
                "project": {
                    "coverage": [
                        {
                            "artefact": {
                                "name": "run_cli"
                            },
                            "summary": {
                                "uncoveredLineCount": 2
                            }
                        }
                    ]
                }
            }
        }));

        assert_eq!(
            payload,
            json!([
                {
                    "artefact": {
                        "name": "run_cli"
                    },
                    "summary": {
                        "uncoveredLineCount": 2
                    }
                }
            ])
        );
    }

    #[test]
    fn format_query_output_renders_table_for_dsl_results() {
        let rendered = format_query_output(
            &json!({
                "repo": {
                    "artefacts": {
                        "edges": [
                            {
                                "node": {
                                    "path": "src/main.rs",
                                    "symbolFqn": "main",
                                    "chatHistory": {
                                        "edges": [
                                            { "node": { "sessionId": "s1" } },
                                            { "node": { "sessionId": "s2" } }
                                        ]
                                    }
                                }
                            }
                        ]
                    }
                }
            }),
            false,
            false,
        )
        .expect("dsl results should render");

        assert!(rendered.contains("| path"));
        assert!(rendered.contains("| symbol_fqn"));
        assert!(rendered.contains("| chat_history"));
        assert!(rendered.contains("src/main.rs"));
        assert!(rendered.contains("[2 entries]"));
    }

    #[test]
    fn format_query_output_emits_compact_json_for_dsl_results() {
        let rendered = format_query_output(
            &json!({
                "repo": {
                    "artefacts": {
                        "edges": [
                            {
                                "node": {
                                    "path": "src/main.rs",
                                    "symbolFqn": "main"
                                }
                            }
                        ]
                    }
                }
            }),
            true,
            false,
        )
        .expect("compact output should render");

        assert_eq!(rendered, r#"[{"path":"src/main.rs","symbolFqn":"main"}]"#);
    }

    #[test]
    fn format_query_output_keeps_raw_graphql_shape() {
        let rendered = format_query_output(
            &json!({
                "repo": {
                    "artefacts": {
                        "edges": [
                            {
                                "node": {
                                    "path": "src/main.rs"
                                }
                            }
                        ],
                        "pageInfo": {
                            "hasNextPage": false
                        }
                    }
                }
            }),
            false,
            true,
        )
        .expect("raw graphql should render as json");

        assert!(rendered.contains("\"pageInfo\""));
        assert!(rendered.contains("\"hasNextPage\": false"));
    }
}
