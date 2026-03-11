async fn postgres_exec(pg_client: &tokio_postgres::Client, sql: &str) -> Result<()> {
    run_postgres_exec(pg_client, sql).await
}

async fn pg_query_rows(pg_client: &tokio_postgres::Client, sql: &str) -> Result<Vec<Value>> {
    let wrapped = format!(
        "SELECT coalesce(json_agg(t), '[]'::json)::text FROM ({}) t",
        sql.trim().trim_end_matches(';')
    );
    let raw = run_postgres_query_scalar_text(pg_client, &wrapped).await?;
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

async fn run_postgres_exec(pg_client: &tokio_postgres::Client, sql: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), pg_client.batch_execute(sql))
        .await
        .context("Postgres statement timeout after 30s")?
        .context("executing Postgres statements")?;
    Ok(())
}

async fn run_postgres_query_scalar_text(
    pg_client: &tokio_postgres::Client,
    sql: &str,
) -> Result<String> {
    let row = tokio::time::timeout(Duration::from_secs(30), pg_client.query_one(sql, &[]))
        .await
        .context("Postgres query timeout after 30s")?
        .context("executing Postgres query")?;
    let value: String = row
        .try_get(0)
        .context("reading Postgres scalar text result")?;
    Ok(value)
}

fn validate_postgres_sslmode_for_notls(dsn: &str, ssl_mode: SslMode) -> Result<()> {
    let dsn_lower = dsn.to_ascii_lowercase();
    if dsn_lower.contains("sslmode=verify-ca") || dsn_lower.contains("sslmode=verify-full") {
        bail!(
            "Postgres DSN requires certificate-based TLS (sslmode=verify-ca/verify-full), \
but this client is configured to use an unencrypted connection (NoTls). \
Use sslmode=disable if plaintext is acceptable, or configure a TLS-enabled Postgres client."
        );
    }

    match ssl_mode {
        SslMode::Disable | SslMode::Prefer => {}
        _ => {
            bail!(
                "Postgres DSN requires TLS (sslmode={:?}), \
but this client is configured to use an unencrypted connection (NoTls). \
Either adjust the DSN to use sslmode=disable if plaintext is acceptable, \
or configure a TLS-enabled Postgres client.",
                ssl_mode
            );
        }
    }
    Ok(())
}

async fn connect_postgres_client(dsn: &str) -> Result<tokio_postgres::Client> {
    let mut pg_cfg: tokio_postgres::Config = dsn.parse().context("parsing Postgres DSN")?;
    validate_postgres_sslmode_for_notls(dsn, pg_cfg.get_ssl_mode())?;

    pg_cfg.connect_timeout(Duration::from_secs(10));

    let (client, connection) = tokio::time::timeout(Duration::from_secs(10), pg_cfg.connect(NoTls))
        .await
        .context("Postgres connect timeout after 10s")?
        .context("connecting to Postgres")?;

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::warn!("Postgres connection task ended: {err:#}");
        }
    });

    Ok(client)
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

async fn clickhouse_exec(cfg: &DevqlConfig, sql: &str) -> Result<String> {
    let endpoint = cfg.clickhouse_endpoint()?;
    run_clickhouse_sql_http(
        &endpoint,
        cfg.clickhouse_user(),
        cfg.clickhouse_password(),
        sql,
    )
    .await
}

async fn clickhouse_query_data(cfg: &DevqlConfig, sql: &str) -> Result<Value> {
    let mut query = sql.trim().to_string();
    if !query.to_ascii_uppercase().contains("FORMAT JSON") {
        query.push_str(" FORMAT JSON");
    }

    let raw = clickhouse_exec(cfg, &query).await?;
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

/// Connect timeout (seconds) for HTTP when talking to ClickHouse.
const CLICKHOUSE_CONNECT_TIMEOUT_SECS: u64 = 10;
/// Total transfer timeout (seconds) for HTTP when talking to ClickHouse.
const CLICKHOUSE_MAX_TIME_SECS: u64 = 30;

async fn run_clickhouse_sql_http(
    url: &str,
    user: Option<&str>,
    password: Option<&str>,
    sql: &str,
) -> Result<String> {
    let client = clickhouse_http_client()?;

    let mut request = client.post(url).body(sql.to_string());
    if let Some(username) = user {
        request = request.basic_auth(username, Some(password.unwrap_or("")));
    }

    let response = request.send().await.map_err(|err| {
        if err.is_timeout() {
            anyhow!(
                "ClickHouse request timed out (connect or transfer limit exceeded, {}s/{}s)",
                CLICKHOUSE_CONNECT_TIMEOUT_SECS,
                CLICKHOUSE_MAX_TIME_SECS
            )
        } else {
            anyhow!("sending ClickHouse request: {err}")
        }
    })?;

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

fn clickhouse_http_client() -> Result<&'static reqwest::Client> {
    static CLICKHOUSE_HTTP_CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    let result = CLICKHOUSE_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CLICKHOUSE_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(CLICKHOUSE_MAX_TIME_SECS))
            .build()
            .map_err(|err| format!("{err:#}"))
    });

    match result {
        Ok(client) => Ok(client),
        Err(err) => Err(anyhow!("building ClickHouse HTTP client: {err}")),
    }
}

fn esc_pg(value: &str) -> String {
    value.replace('\'', "''")
}

fn esc_ch(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn normalize_repo_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn build_path_candidates(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let raw = path.trim();
    if !raw.is_empty() {
        out.push(raw.to_string());
    }

    let normalized = normalize_repo_path(raw);
    if !normalized.is_empty() {
        out.push(normalized.clone());
        out.push(format!("./{normalized}"));
    }

    out.sort();
    out.dedup();
    out
}

fn sql_path_candidates_clause(column: &str, candidates: &[String]) -> String {
    if candidates.is_empty() {
        return "1=0".to_string();
    }

    candidates
        .iter()
        .map(|candidate| format!("{column} = '{}'", esc_pg(candidate)))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn format_ch_array(values: &[String]) -> String {
    if values.is_empty() {
        return "[]".to_string();
    }

    let parts = values
        .iter()
        .map(|value| format!("'{}'", esc_ch(value)))
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(","))
}

fn glob_to_sql_like(glob: &str) -> String {
    glob.replace("**", "%").replace('*', "%")
}

fn deterministic_uuid(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = format!("{:x}", hasher.finalize());

    let hex = &digest[..32];
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
