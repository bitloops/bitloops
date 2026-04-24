use serde_json::Value;

use super::types::AnalyticsSqlResult;

pub(crate) fn format_analytics_sql_result_table(result: &AnalyticsSqlResult) -> String {
    let rows = result.rows.as_array().cloned().unwrap_or_default();
    if result.columns.is_empty() {
        return format!(
            "(0 columns, {} rows{})",
            result.row_count,
            if result.truncated { ", truncated" } else { "" }
        );
    }

    let headers = result
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();
    let mut widths = headers
        .iter()
        .map(|header| header.len())
        .collect::<Vec<_>>();

    let cell_rows = rows
        .iter()
        .map(|row| {
            result
                .columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    let cell = row
                        .get(&column.name)
                        .map(compact_cell_value)
                        .unwrap_or_else(|| "null".to_string());
                    widths[index] = widths[index].max(cell.len().min(60));
                    cell
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut out = String::new();
    let header_line = headers
        .iter()
        .enumerate()
        .map(|(index, header)| format!("{header:<width$}", width = widths[index]))
        .collect::<Vec<_>>()
        .join(" | ");
    out.push_str(&header_line);
    out.push('\n');
    out.push_str(
        &widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("-+-"),
    );
    out.push('\n');

    for row in cell_rows {
        let line = row
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let rendered = truncate_cell(value, widths[index]);
                format!("{rendered:<width$}", width = widths[index])
            })
            .collect::<Vec<_>>()
            .join(" | ");
        out.push_str(&line);
        out.push('\n');
    }

    out.push_str(&format!(
        "{} row{} in {} ms",
        result.row_count,
        if result.row_count == 1 { "" } else { "s" },
        result.duration_ms
    ));
    if result.truncated {
        out.push_str(" (truncated)");
    }
    if !result.repo_ids.is_empty() {
        out.push_str(&format!("\nRepos: {}", result.repo_ids.join(", ")));
    }
    if !result.warnings.is_empty() {
        out.push_str(&format!("\nWarnings: {}", result.warnings.join(" | ")));
    }
    out
}

fn compact_cell_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn truncate_cell(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    let mut rendered = value
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    rendered.push('…');
    rendered
}
