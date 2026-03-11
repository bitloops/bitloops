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
    let wrapped = format!(
        "SELECT coalesce(json_agg(t), '[]'::json)::text FROM ({}) t",
        sql.trim().trim_end_matches(';')
    );
    let raw = tokio::time::timeout(Duration::from_secs(30), pg_client.query_one(&wrapped, &[]))
        .await
        .context("Postgres query timeout after 30s")?
        .context("executing Postgres query")?
        .try_get::<_, String>(0)
        .context("reading Postgres scalar text result")?;
    let parsed: Value = serde_json::from_str(raw.trim()).with_context(|| {
        format!(
            "parsing Postgres JSON payload failed: {}",
            truncate_for_error(&raw)
        )
    })?;
    match parsed {
        Value::Array(rows) => Ok(rows),
        Value::Object(_) => Ok(vec![parsed]),
        Value::Null => Ok(vec![]),
        other => bail!("unexpected Postgres JSON payload type: {other}"),
    }
}

pub fn esc_pg(value: &str) -> String {
    value.replace('\'', "''")
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
