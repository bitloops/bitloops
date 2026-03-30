use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::fmt;

use super::config::ClickHouseConfig;

#[derive(Clone)]
pub(super) struct ClickHousePool {
    client: reqwest::Client,
    endpoint: String,
    user: Option<String>,
    password: Option<String>,
}

impl fmt::Debug for ClickHousePool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClickHousePool")
            .field("endpoint", &self.endpoint)
            .field("auth_enabled", &self.user.is_some())
            .finish()
    }
}

impl ClickHousePool {
    pub(super) fn build(cfg: &ClickHouseConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(16)
            .build()
            .context("building ClickHouse HTTP client")?;

        Ok(Self {
            client,
            endpoint: cfg.endpoint(),
            user: cfg.user.clone(),
            password: cfg.password.clone(),
        })
    }

    async fn run_sql(&self, sql: &str) -> Result<String> {
        let mut request = self.client.post(&self.endpoint).body(sql.to_string());
        if let Some(user) = &self.user {
            request = request.basic_auth(user, Some(self.password.clone().unwrap_or_default()));
        }

        let response = request.send().await.context("sending ClickHouse request")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("reading ClickHouse response body")?;
        if !status.is_success() {
            let detail = body.trim();
            if detail.is_empty() {
                bail!("ClickHouse request failed with status {status}");
            }
            bail!("ClickHouse request failed with status {status}: {detail}");
        }

        Ok(body)
    }

    pub(super) async fn ping(&self) -> Result<i32> {
        let raw = self.run_sql("SELECT 1 FORMAT TabSeparated").await?;
        let value_raw = raw
            .lines()
            .last()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .ok_or_else(|| anyhow!("ClickHouse health query returned an empty response"))?;
        let value = value_raw.parse::<i32>().with_context(|| {
            format!("parsing ClickHouse health query result as integer: {value_raw}")
        })?;
        Ok(value)
    }

    pub(super) async fn query_data(&self, sql: &str) -> Result<Value> {
        let mut query = sql.trim().to_string();
        if !query.to_ascii_uppercase().contains("FORMAT JSON") {
            query.push_str(" FORMAT JSON");
        }

        let raw = self.run_sql(&query).await?;
        if raw.trim().is_empty() {
            return Ok(Value::Array(vec![]));
        }

        let parsed: Value = serde_json::from_str(&raw)
            .with_context(|| format!("parsing ClickHouse JSON response: {raw}"))?;
        Ok(parsed
            .get("data")
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])))
    }
}
