use super::*;

pub(super) async fn mirror_completed_current_artefacts_for_head(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    head_sha: &str,
) -> Result<usize> {
    let head_sha = head_sha.trim();
    if head_sha.is_empty() {
        return Ok(0);
    }

    if !repo_sync_state_matches_completed_head(relational, &cfg.repo.repo_id, head_sha).await? {
        return Ok(0);
    }

    let current_rows = current_artefact_rows_for_repo_count(relational, &cfg.repo.repo_id).await?;
    if current_rows == 0 {
        return Ok(0);
    }

    if relational.has_remote_shared_relational_authority() {
        mirror_current_artefacts_to_remote_shared(relational, &cfg.repo.repo_id).await?;
    } else {
        relational
            .exec_for_role(
                RelationalStorageRole::SharedRelational,
                &build_current_artefact_mirror_insert_select_sql(&cfg.repo.repo_id),
            )
            .await
            .context("mirroring current artefacts into canonical artefacts")?;
    }

    Ok(current_rows)
}

async fn repo_sync_state_matches_completed_head(
    relational: &RelationalStorage,
    repo_id: &str,
    head_sha: &str,
) -> Result<bool> {
    let rows = relational
        .query_rows(&format!(
            "SELECT last_sync_status, head_commit_sha \
             FROM repo_sync_state \
             WHERE repo_id = '{}'",
            esc_pg(repo_id),
        ))
        .await
        .context("loading repo sync state before ingest current artefact mirror")?;
    let Some(row) = rows.first() else {
        return Ok(false);
    };
    let status = row
        .get("last_sync_status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let synced_head = row
        .get("head_commit_sha")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Ok(status == "completed" && synced_head == head_sha)
}

async fn current_artefact_rows_for_repo_count(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<usize> {
    let rows = relational
        .query_rows(&format!(
            "SELECT COUNT(*) AS row_count \
             FROM artefacts_current \
             WHERE repo_id = '{}'",
            esc_pg(repo_id),
        ))
        .await
        .context("counting current artefacts before ingest mirror")?;
    let raw = rows
        .first()
        .and_then(|row| row.get("row_count"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        })
        .unwrap_or(0);
    usize::try_from(raw).context("current artefact row count exceeds usize")
}

fn build_current_artefact_mirror_insert_select_sql(repo_id: &str) -> String {
    let repo_id = esc_pg(repo_id);
    format!(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, language, extraction_fingerprint, canonical_kind,
            language_kind, symbol_fqn, signature, modifiers, docstring, content_hash
         )
         SELECT
            artefact_id, symbol_id, repo_id, language, extraction_fingerprint, canonical_kind,
            language_kind, symbol_fqn, signature, modifiers, docstring, content_id
         FROM artefacts_current
         WHERE repo_id = '{repo_id}'
         ON CONFLICT (artefact_id) DO UPDATE SET
            symbol_id = EXCLUDED.symbol_id,
            repo_id = EXCLUDED.repo_id,
            language = EXCLUDED.language,
            extraction_fingerprint = EXCLUDED.extraction_fingerprint,
            canonical_kind = EXCLUDED.canonical_kind,
            language_kind = EXCLUDED.language_kind,
            symbol_fqn = EXCLUDED.symbol_fqn,
            signature = EXCLUDED.signature,
            modifiers = EXCLUDED.modifiers,
            docstring = EXCLUDED.docstring,
            content_hash = EXCLUDED.content_hash",
    )
}

async fn mirror_current_artefacts_to_remote_shared(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    let rows = relational
        .query_rows(&format!(
            "SELECT
                artefact_id,
                symbol_id,
                repo_id,
                language,
                extraction_fingerprint,
                canonical_kind,
                language_kind,
                symbol_fqn,
                signature,
                modifiers,
                docstring,
                content_id AS content_hash
             FROM artefacts_current
             WHERE repo_id = '{}'",
            esc_pg(repo_id),
        ))
        .await
        .context("loading current artefacts for remote canonical mirror")?;
    if rows.is_empty() {
        return Ok(());
    }

    let dialect = relational.dialect_for_role(RelationalStorageRole::SharedRelational);
    let statements = rows
        .iter()
        .map(|row| build_current_artefact_mirror_upsert_sql(dialect, row))
        .collect::<Result<Vec<_>>>()?;
    relational
        .exec_batch_transactional_for_role(RelationalStorageRole::SharedRelational, &statements)
        .await
        .context("mirroring current artefacts into remote canonical artefacts")
}

fn build_current_artefact_mirror_upsert_sql(
    dialect: RelationalDialect,
    row: &serde_json::Value,
) -> Result<String> {
    Ok(format!(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, language, extraction_fingerprint, canonical_kind,
            language_kind, symbol_fqn, signature, modifiers, docstring, content_hash
         ) VALUES (
            '{artefact_id}', {symbol_id}, '{repo_id}', '{language}', {extraction_fingerprint},
            {canonical_kind}, {language_kind}, {symbol_fqn}, {signature}, {modifiers},
            {docstring}, '{content_hash}'
         )
         ON CONFLICT (artefact_id) DO UPDATE SET
            symbol_id = EXCLUDED.symbol_id,
            repo_id = EXCLUDED.repo_id,
            language = EXCLUDED.language,
            extraction_fingerprint = EXCLUDED.extraction_fingerprint,
            canonical_kind = EXCLUDED.canonical_kind,
            language_kind = EXCLUDED.language_kind,
            symbol_fqn = EXCLUDED.symbol_fqn,
            signature = EXCLUDED.signature,
            modifiers = EXCLUDED.modifiers,
            docstring = EXCLUDED.docstring,
            content_hash = EXCLUDED.content_hash",
        artefact_id = esc_pg(row_required_str(row, "artefact_id")?),
        symbol_id = sql_nullable_text(row_optional_str(row, "symbol_id")),
        repo_id = esc_pg(row_required_str(row, "repo_id")?),
        language = esc_pg(row_required_str(row, "language")?),
        extraction_fingerprint = sql_nullable_text(row_optional_str(row, "extraction_fingerprint")),
        canonical_kind = sql_nullable_text(row_optional_str(row, "canonical_kind")),
        language_kind = sql_nullable_text(row_optional_str(row, "language_kind")),
        symbol_fqn = sql_nullable_text(row_optional_str(row, "symbol_fqn")),
        signature = sql_nullable_text(row_optional_str(row, "signature")),
        modifiers = sql_json_value_for_dialect(dialect, &row_json_value(row, "modifiers", "[]")?),
        docstring = sql_nullable_text(row_optional_str(row, "docstring")),
        content_hash = esc_pg(row_required_str(row, "content_hash")?),
    ))
}

fn row_required_str<'a>(row: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("current artefact mirror row is missing string column `{key}`"))
}

fn row_optional_str<'a>(row: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
}

fn row_json_value(row: &serde_json::Value, key: &str, default: &str) -> Result<serde_json::Value> {
    match row.get(key) {
        Some(serde_json::Value::String(raw)) if !raw.is_empty() => serde_json::from_str(raw)
            .with_context(|| format!("parsing current artefact mirror JSON column `{key}`")),
        Some(serde_json::Value::Array(_)) | Some(serde_json::Value::Object(_)) => {
            Ok(row.get(key).cloned().unwrap_or(serde_json::Value::Null))
        }
        Some(serde_json::Value::Null) | None => serde_json::from_str(default).with_context(|| {
            format!("parsing default JSON for current artefact mirror column `{key}`")
        }),
        Some(other) => serde_json::from_str(&other.to_string())
            .with_context(|| format!("parsing current artefact mirror JSON column `{key}`")),
    }
}

fn sql_nullable_text(value: Option<&str>) -> String {
    value
        .map(|text| format!("'{}'", esc_pg(text)))
        .unwrap_or_else(|| "NULL".to_string())
}
