use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

pub async fn postgres_exec(pg_client: &tokio_postgres::Client, sql: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), pg_client.batch_execute(sql))
        .await
        .context("Postgres statement timeout after 30s")?
        .context("executing Postgres statements")?;
    Ok(())
}

pub async fn pg_query_rows(pg_client: &tokio_postgres::Client, sql: &str) -> Result<Vec<Value>> {
    let wrapped = wrap_query_rows_sql(sql);
    let raw = tokio::time::timeout(Duration::from_secs(30), pg_client.query_one(&wrapped, &[]))
        .await
        .context("Postgres query timeout after 30s")?
        .context("executing Postgres query")?
        .try_get::<_, String>(0)
        .context("reading Postgres scalar text result")?;
    parse_query_rows_json(&raw)
}

pub fn esc_pg(value: &str) -> String {
    value.replace('\'', "''")
}

fn wrap_query_rows_sql(sql: &str) -> String {
    format!(
        "SELECT coalesce(json_agg(t), '[]'::json)::text FROM ({}) t",
        sql.trim().trim_end_matches(';')
    )
}

fn parse_query_rows_json(raw: &str) -> Result<Vec<Value>> {
    let parsed: Value = serde_json::from_str(raw.trim()).with_context(|| {
        format!(
            "parsing Postgres JSON payload failed: {}",
            truncate_for_error(raw)
        )
    })?;
    match parsed {
        Value::Array(rows) => Ok(rows),
        Value::Object(_) => Ok(vec![parsed]),
        Value::Null => Ok(vec![]),
        other => bail!("unexpected Postgres JSON payload type: {other}"),
    }
}

fn truncate_for_error(input: &str) -> String {
    const MAX: usize = 500;
    let mut out = input.to_string();
    if out.len() > MAX {
        out.truncate(MAX);
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn postgres_wrap_query_rows_sql_trims_trailing_semicolon() {
        let wrapped = wrap_query_rows_sql("SELECT * FROM artefacts;");
        assert_eq!(
            wrapped,
            "SELECT coalesce(json_agg(t), '[]'::json)::text FROM (SELECT * FROM artefacts) t"
        );
    }

    #[test]
    fn postgres_parse_query_rows_json_handles_array_object_and_null() {
        assert_eq!(
            parse_query_rows_json(r#"[{"id":1},{"id":2}]"#).expect("array"),
            vec![json!({"id": 1}), json!({"id": 2})]
        );
        assert_eq!(
            parse_query_rows_json(r#"{"id":1}"#).expect("object"),
            vec![json!({"id": 1})]
        );
        assert!(
            parse_query_rows_json("null")
                .expect("null should map to empty")
                .is_empty()
        );
    }

    #[test]
    fn postgres_parse_query_rows_json_rejects_unexpected_scalar() {
        let err = parse_query_rows_json("42").expect_err("scalar payload should fail");
        assert!(
            err.to_string()
                .contains("unexpected Postgres JSON payload type")
        );
    }

    #[test]
    fn postgres_escape_and_truncate_helpers_are_stable() {
        assert_eq!(esc_pg("O'Brien"), "O''Brien");

        let input = "x".repeat(600);
        let truncated = truncate_for_error(&input);
        assert!(truncated.len() <= 503);
        assert!(truncated.ends_with("..."));
    }
}
