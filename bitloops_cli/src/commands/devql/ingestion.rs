#[derive(Debug, Clone, Default)]
struct IngestionCounters {
    checkpoints_processed: usize,
    events_inserted: usize,
    artefacts_upserted: usize,
    checkpoints_without_commit: usize,
}

#[derive(Debug, Clone)]
struct CheckpointCommitInfo {
    commit_sha: String,
    commit_unix: i64,
    author_name: String,
    author_email: String,
    subject: String,
}

fn resolve_repo_identity(repo_root: &Path) -> Result<RepoIdentity> {
    let fallback_name = repo_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repo")
        .to_string();

    let remote = run_git(repo_root, &["config", "--get", "remote.origin.url"]).unwrap_or_default();
    let remote = remote.trim();

    let (provider, organization, name) = if remote.is_empty() {
        ("local".to_string(), "local".to_string(), fallback_name)
    } else if let Some((org, name)) = parse_remote_owner_name(remote) {
        let provider = if remote.contains("github") {
            "github"
        } else if remote.contains("gitlab") {
            "gitlab"
        } else {
            "git"
        };
        (provider.to_string(), org, name)
    } else {
        ("git".to_string(), "local".to_string(), fallback_name)
    };

    let identity = format!("{}://{}/{}", provider, organization, name);
    let repo_id = deterministic_uuid(&identity);

    Ok(RepoIdentity {
        provider,
        organization,
        name,
        identity,
        repo_id,
    })
}

fn parse_remote_owner_name(remote: &str) -> Option<(String, String)> {
    let trimmed = remote.trim().trim_end_matches('/');

    if let Some(rest) = trimmed.strip_prefix("git@") {
        let (_, path) = rest.split_once(':')?;
        return parse_owner_name_path(path);
    }

    if let Some(pos) = trimmed.find("://") {
        let rest = &trimmed[pos + 3..];
        let (_, path) = rest.split_once('/')?;
        return parse_owner_name_path(path);
    }

    if let Some(path) = trimmed.strip_prefix("ssh://") {
        let (_, path) = path.split_once('/')?;
        return parse_owner_name_path(path);
    }

    None
}

fn parse_owner_name_path(path: &str) -> Option<(String, String)> {
    let clean = path.trim().trim_end_matches(".git");
    let mut parts = clean
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let name = parts.pop()?.to_string();
    let org = parts.pop()?.to_string();
    Some((org, name))
}

async fn init_clickhouse_schema(cfg: &DevqlConfig) -> Result<()> {
    let sql = r#"
CREATE TABLE IF NOT EXISTS checkpoint_events (
    event_id String,
    event_time DateTime64(3, 'UTC'),
    repo_id String,
    checkpoint_id String,
    session_id String,
    commit_sha String,
    branch String,
    event_type String,
    agent String,
    strategy String,
    files_touched Array(String),
    payload String
)
ENGINE = ReplacingMergeTree(event_time)
ORDER BY (repo_id, event_time, event_id)
"#;

    clickhouse_exec(cfg, sql)
        .await
        .context("creating ClickHouse checkpoint_events table")?;
    Ok(())
}

async fn init_postgres_schema(
    _cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    let sql = postgres_schema_sql();
    postgres_exec(pg_client, sql)
        .await
        .context("creating Postgres DevQL tables")?;

    let artefacts_alter_sql = artefacts_upgrade_sql();
    postgres_exec(pg_client, artefacts_alter_sql)
        .await
        .context("updating Postgres artefacts columns for byte offsets/signature")?;

    let artefact_edges_hardening_sql = artefact_edges_hardening_sql();
    postgres_exec(pg_client, artefact_edges_hardening_sql)
        .await
        .context("updating Postgres artefact_edges constraints/indexes")?;
    Ok(())
}

fn postgres_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    organization TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS commits (
    commit_sha TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    author_name TEXT,
    author_email TEXT,
    commit_message TEXT,
    committed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS file_state (
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    PRIMARY KEY (repo_id, commit_sha, path)
);

CREATE INDEX IF NOT EXISTS file_state_blob_idx
ON file_state (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS file_state_commit_idx
ON file_state (repo_id, commit_sha);

CREATE TABLE IF NOT EXISTS artefacts (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT NOT NULL,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    content_hash TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS artefacts_blob_idx
ON artefacts (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefacts_path_idx
ON artefacts (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_kind_idx
ON artefacts (repo_id, canonical_kind);

CREATE TABLE IF NOT EXISTS artefact_edges (
    edge_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata JSONB DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ DEFAULT now(),
    CONSTRAINT artefact_edges_target_chk
        CHECK (to_artefact_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CONSTRAINT artefact_edges_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        )
);

CREATE INDEX IF NOT EXISTS artefact_edges_blob_idx
ON artefact_edges (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefact_edges_from_idx
ON artefact_edges (repo_id, from_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_to_idx
ON artefact_edges (repo_id, to_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_kind_idx
ON artefact_edges (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx
ON artefact_edges (repo_id, edge_kind, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq
ON artefact_edges (
    repo_id,
    blob_sha,
    from_artefact_id,
    edge_kind,
    COALESCE(to_artefact_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1)
);
"#
}

fn artefacts_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS start_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS end_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS signature TEXT;
UPDATE artefacts
SET start_byte = 0
WHERE start_byte IS NULL;
UPDATE artefacts
SET end_byte = 0
WHERE end_byte IS NULL;
ALTER TABLE artefacts ALTER COLUMN start_byte SET NOT NULL;
ALTER TABLE artefacts ALTER COLUMN end_byte SET NOT NULL;
"#
}

fn artefact_edges_hardening_sql() -> &'static str {
    r#"
ALTER TABLE artefact_edges ADD COLUMN IF NOT EXISTS metadata JSONB DEFAULT '{}'::jsonb;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_target_chk'
    ) THEN
        ALTER TABLE artefact_edges
        ADD CONSTRAINT artefact_edges_target_chk
        CHECK (to_artefact_id IS NOT NULL OR to_symbol_ref IS NOT NULL);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_line_range_chk'
    ) THEN
        ALTER TABLE artefact_edges
        ADD CONSTRAINT artefact_edges_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        );
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS artefact_edges_blob_idx
ON artefact_edges (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefact_edges_from_idx
ON artefact_edges (repo_id, from_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_to_idx
ON artefact_edges (repo_id, to_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_kind_idx
ON artefact_edges (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx
ON artefact_edges (repo_id, edge_kind, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq
ON artefact_edges (
    repo_id,
    blob_sha,
    from_artefact_id,
    edge_kind,
    COALESCE(to_artefact_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1)
);
"#
}

async fn ensure_repository_row(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) VALUES ('{}', '{}', '{}', '{}', '{}') \
ON CONFLICT (repo_id) DO UPDATE SET provider = EXCLUDED.provider, organization = EXCLUDED.organization, name = EXCLUDED.name, default_branch = EXCLUDED.default_branch",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&cfg.repo.provider),
        esc_pg(&cfg.repo.organization),
        esc_pg(&cfg.repo.name),
        esc_pg(&default_branch_name(&cfg.repo_root))
    );
    postgres_exec(pg_client, &sql).await
}

fn default_branch_name(repo_root: &Path) -> String {
    run_git(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|_| "main".to_string())
}

fn collect_checkpoint_commit_map(
    repo_root: &Path,
) -> Result<HashMap<String, CheckpointCommitInfo>> {
    let fmt = format!(
        "%H%x1f%ct%x1f%an%x1f%ae%x1f%s%x1f%(trailers:key={CHECKPOINT_TRAILER_KEY},valueonly=true,separator=%x00)%x1e"
    );
    let raw = run_git(
        repo_root,
        &[
            "log",
            "--all",
            "--date-order",
            &format!("--format={fmt}"),
            "--max-count=50000",
            "--no-color",
        ],
    )
    .unwrap_or_default();

    let mut out = HashMap::new();
    for record in raw.split('\u{1e}') {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }

        let mut parts = record.split('\u{1f}');
        let commit_sha = parts.next().unwrap_or_default().trim().to_string();
        let commit_unix = parts
            .next()
            .unwrap_or_default()
            .trim()
            .parse::<i64>()
            .unwrap_or(0);
        let author_name = parts.next().unwrap_or_default().trim().to_string();
        let author_email = parts.next().unwrap_or_default().trim().to_string();
        let subject = parts.next().unwrap_or_default().trim().to_string();
        let checkpoints = parts.next().unwrap_or_default();

        if commit_sha.is_empty() {
            continue;
        }

        for cp in checkpoints.split('\x00').map(str::trim) {
            if !is_valid_checkpoint_id(cp) {
                continue;
            }
            out.entry(cp.to_string())
                .or_insert_with(|| CheckpointCommitInfo {
                    commit_sha: commit_sha.clone(),
                    commit_unix,
                    author_name: author_name.clone(),
                    author_email: author_email.clone(),
                    subject: subject.clone(),
                });
        }
    }

    Ok(out)
}

async fn fetch_existing_checkpoint_event_ids(cfg: &DevqlConfig) -> Result<HashSet<String>> {
    let sql = format!(
        "SELECT event_id FROM checkpoint_events WHERE repo_id = '{}' FORMAT JSON",
        esc_ch(&cfg.repo.repo_id)
    );

    let mut out = HashSet::new();
    let data = clickhouse_query_data(cfg, &sql).await?;
    if let Some(rows) = data.as_array() {
        for row in rows {
            if let Some(id) = row.get("event_id").and_then(Value::as_str) {
                out.insert(id.to_string());
            }
        }
    }
    Ok(out)
}

async fn insert_checkpoint_event(
    cfg: &DevqlConfig,
    cp: &CommittedInfo,
    event_id: &str,
    commit_info: Option<&CheckpointCommitInfo>,
) -> Result<()> {
    let event_time_expr = if !cp.created_at.trim().is_empty() {
        format!(
            "coalesce(parseDateTime64BestEffortOrNull('{}'), now64(3))",
            esc_ch(cp.created_at.trim())
        )
    } else if let Some(info) = commit_info {
        format!("toDateTime64({}, 3, 'UTC')", info.commit_unix)
    } else {
        "now64(3)".to_string()
    };

    let commit_sha = commit_info
        .map(|info| info.commit_sha.as_str())
        .unwrap_or_default();

    let payload = json!({
        "checkpoints_count": cp.checkpoints_count,
        "session_count": cp.session_count,
        "token_usage": cp.token_usage,
    });

    let files_touched = format_ch_array(&cp.files_touched);
    let sql = format!(
        "INSERT INTO checkpoint_events (event_id, event_time, repo_id, checkpoint_id, session_id, commit_sha, branch, event_type, agent, strategy, files_touched, payload) \
VALUES ('{}', {}, '{}', '{}', '{}', '{}', '{}', 'checkpoint_committed', '{}', '{}', {}, '{}')",
        esc_ch(event_id),
        event_time_expr,
        esc_ch(&cfg.repo.repo_id),
        esc_ch(&cp.checkpoint_id),
        esc_ch(&cp.session_id),
        esc_ch(commit_sha),
        esc_ch(&cp.branch),
        esc_ch(&cp.agent),
        esc_ch(&cp.strategy),
        files_touched,
        esc_ch(&serde_json::to_string(&payload)?),
    );

    clickhouse_exec(cfg, &sql).await.map(|_| ())
}

async fn upsert_commit_row(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
    cp: &CommittedInfo,
    commit_info: &CheckpointCommitInfo,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at) VALUES ('{}', '{}', '{}', '{}', '{}', to_timestamp({})) \
ON CONFLICT (commit_sha) DO UPDATE SET repo_id = EXCLUDED.repo_id, author_name = EXCLUDED.author_name, author_email = EXCLUDED.author_email, commit_message = EXCLUDED.commit_message, committed_at = EXCLUDED.committed_at",
        esc_pg(&commit_info.commit_sha),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&commit_info.author_name),
        esc_pg(&commit_info.author_email),
        esc_pg(if commit_info.subject.is_empty() {
            &cp.checkpoint_id
        } else {
            &commit_info.subject
        }),
        commit_info.commit_unix,
    );

    postgres_exec(pg_client, &sql).await
}

async fn upsert_file_state_row(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES ('{}', '{}', '{}', '{}') \
ON CONFLICT (repo_id, commit_sha, path) DO UPDATE SET blob_sha = EXCLUDED.blob_sha",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(commit_sha),
        esc_pg(path),
        esc_pg(blob_sha),
    );

    postgres_exec(pg_client, &sql).await
}

#[derive(Debug, Clone)]
struct FileArtefactRow {
    artefact_id: String,
    language: String,
}

async fn upsert_file_artefact_row(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
    path: &str,
    blob_sha: &str,
) -> Result<FileArtefactRow> {
    let artefact_id =
        deterministic_uuid(&format!("{}|{}|{}|file", cfg.repo.repo_id, blob_sha, path));
    let language = detect_language(path);
    let line_count = git_blob_line_count(&cfg.repo_root, blob_sha)
        .unwrap_or(1)
        .max(1);
    let byte_count = git_blob_content(&cfg.repo_root, blob_sha)
        .map(|content| content.len() as i32)
        .unwrap_or(0)
        .max(0);

    let sql = format!(
        "INSERT INTO artefacts (artefact_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', '{}', '{}', 'file', 'file', '{}', NULL, 1, {}, 0, {}, NULL, '{}') \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, content_hash = EXCLUDED.content_hash",
        esc_pg(&artefact_id),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(blob_sha),
        esc_pg(path),
        esc_pg(&language),
        esc_pg(path),
        line_count,
        byte_count,
        esc_pg(blob_sha),
    );

    postgres_exec(pg_client, &sql).await?;
    Ok(FileArtefactRow {
        artefact_id,
        language,
    })
}

async fn upsert_language_artefacts(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
    path: &str,
    blob_sha: &str,
    file_artefact: &FileArtefactRow,
) -> Result<()> {
    if file_artefact.language != "typescript"
        && file_artefact.language != "javascript"
        && file_artefact.language != "rust"
    {
        return Ok(());
    }

    let Some(content) = git_blob_content(&cfg.repo_root, blob_sha) else {
        return Ok(());
    };

    let items = if file_artefact.language == "rust" {
        extract_rust_artefacts(&content, path)?
    } else {
        extract_js_ts_artefacts(&content, path)?
    };
    let mut symbol_to_artefact_id: HashMap<String, String> = HashMap::new();
    symbol_to_artefact_id.insert(path.to_string(), file_artefact.artefact_id.clone());

    for item in &items {
        let artefact_id = deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}",
            cfg.repo.repo_id, blob_sha, path, item.canonical_kind, item.name, item.start_line
        ));
        let content_hash = deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}",
            blob_sha, path, item.canonical_kind, item.name, item.start_line, item.end_line
        ));
        let parent_artefact_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_to_artefact_id.get(fqn))
            .cloned()
            .unwrap_or_else(|| file_artefact.artefact_id.clone());

        let sql = format!(
            "INSERT INTO artefacts (artefact_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, {}, {}, {}, '{}', '{}') \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, parent_artefact_id = EXCLUDED.parent_artefact_id, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, content_hash = EXCLUDED.content_hash",
            esc_pg(&artefact_id),
            esc_pg(&cfg.repo.repo_id),
            esc_pg(blob_sha),
            esc_pg(path),
            esc_pg(&file_artefact.language),
            esc_pg(&item.canonical_kind),
            esc_pg(&item.language_kind),
            esc_pg(&item.symbol_fqn),
            esc_pg(&parent_artefact_id),
            item.start_line,
            item.end_line,
            item.start_byte,
            item.end_byte,
            esc_pg(&item.signature),
            esc_pg(&content_hash),
        );

        postgres_exec(pg_client, &sql).await?;
        symbol_to_artefact_id.insert(item.symbol_fqn.clone(), artefact_id);
    }

    let edges = if file_artefact.language == "rust" {
        extract_rust_dependency_edges(&content, path, &items)?
    } else {
        extract_js_ts_dependency_edges(&content, path, &items)?
    };
    for edge in edges {
        let Some(from_artefact_id) = symbol_to_artefact_id.get(&edge.from_symbol_fqn).cloned() else {
            continue;
        };

        let to_artefact_id = edge
            .to_target_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_to_artefact_id.get(fqn))
            .cloned();
        let to_symbol_ref = if to_artefact_id.is_some() {
            None
        } else {
            edge.to_symbol_ref.clone()
        };

        if to_artefact_id.is_none() && to_symbol_ref.is_none() {
            continue;
        }

        let edge_id = deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            cfg.repo.repo_id,
            blob_sha,
            from_artefact_id,
            edge.edge_kind,
            to_artefact_id.clone().unwrap_or_default(),
            to_symbol_ref.clone().unwrap_or_default(),
            edge.start_line.unwrap_or(-1),
            edge.end_line.unwrap_or(-1)
        ));

        let to_artefact_sql = to_artefact_id
            .as_ref()
            .map(|id| format!("'{}'", esc_pg(id)))
            .unwrap_or_else(|| "NULL".to_string());
        let to_symbol_sql = to_symbol_ref
            .as_ref()
            .map(|s| format!("'{}'", esc_pg(s)))
            .unwrap_or_else(|| "NULL".to_string());
        let start_line_sql = edge
            .start_line
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
        let end_line_sql = edge
            .end_line
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
        let metadata_sql = format!("'{}'::jsonb", esc_pg(&edge.metadata.to_string()));

        let sql = format!(
            "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata) \
VALUES ('{}', '{}', '{}', '{}', {}, {}, '{}', '{}', {}, {}, {}) \
ON CONFLICT (edge_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, from_artefact_id = EXCLUDED.from_artefact_id, to_artefact_id = EXCLUDED.to_artefact_id, to_symbol_ref = EXCLUDED.to_symbol_ref, edge_kind = EXCLUDED.edge_kind, language = EXCLUDED.language, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, metadata = EXCLUDED.metadata",
            esc_pg(&edge_id),
            esc_pg(&cfg.repo.repo_id),
            esc_pg(blob_sha),
            esc_pg(&from_artefact_id),
            to_artefact_sql,
            to_symbol_sql,
            esc_pg(&edge.edge_kind),
            esc_pg(&file_artefact.language),
            start_line_sql,
            end_line_sql,
            metadata_sql,
        );
        postgres_exec(pg_client, &sql).await?;
    }

    Ok(())
}

fn git_blob_sha_at_commit(repo_root: &Path, commit_sha: &str, path: &str) -> Option<String> {
    let spec = format!("{commit_sha}:{path}");
    run_git(repo_root, &["rev-parse", &spec])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn git_blob_content(repo_root: &Path, blob_sha: &str) -> Option<String> {
    run_git(repo_root, &["cat-file", "-p", blob_sha]).ok()
}

fn git_blob_line_count(repo_root: &Path, blob_sha: &str) -> Option<i32> {
    let output = git_blob_content(repo_root, blob_sha)?;
    if output.is_empty() {
        return Some(1);
    }
    let mut count = output.lines().count() as i32;
    if !output.ends_with('\n') {
        count += 1;
    }
    Some(count.max(1))
}

fn detect_language(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".ts") || lower.ends_with(".tsx") {
        "typescript".to_string()
    } else if lower.ends_with(".rs") {
        "rust".to_string()
    } else if lower.ends_with(".js") || lower.ends_with(".jsx") {
        "javascript".to_string()
    } else if lower.ends_with(".py") {
        "python".to_string()
    } else if lower.ends_with(".go") {
        "go".to_string()
    } else if lower.ends_with(".java") {
        "java".to_string()
    } else {
        "text".to_string()
    }
}

#[derive(Debug, Clone)]
struct FunctionArtefact {
    name: String,
    start_line: i32,
    end_line: i32,
    start_byte: i32,
    end_byte: i32,
    signature: String,
}

#[derive(Debug, Clone)]
struct JsTsArtefact {
    canonical_kind: String,
    language_kind: String,
    name: String,
    symbol_fqn: String,
    parent_symbol_fqn: Option<String>,
    start_line: i32,
    end_line: i32,
    start_byte: i32,
    end_byte: i32,
    signature: String,
}

#[derive(Debug, Clone)]
struct JsTsDependencyEdge {
    edge_kind: String,
    from_symbol_fqn: String,
    to_target_symbol_fqn: Option<String>,
    to_symbol_ref: Option<String>,
    start_line: Option<i32>,
    end_line: Option<i32>,
    metadata: Value,
}

fn extract_js_ts_functions(content: &str) -> Result<Vec<FunctionArtefact>> {
    let function_decl = Regex::new(
        r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
    )?;
    let function_expr = Regex::new(
        r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:async\s*)?(?:function\s*)?\([^)]*\)\s*=>",
    )?;

    let lines: Vec<&str> = content.lines().collect();
    let line_spans = line_byte_spans(content);
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let start_line = (idx + 1) as i32;

        let name = function_decl
            .captures(line)
            .or_else(|| function_expr.captures(line))
            .and_then(|captures| captures.get(1).map(|m| m.as_str().to_string()));

        let Some(name) = name else {
            continue;
        };

        if !seen.insert((name.clone(), start_line)) {
            continue;
        }

        let end_line = find_block_end_line(&lines, idx).unwrap_or(start_line);
        out.push(FunctionArtefact {
            start_byte: line_start_byte(&line_spans, start_line),
            end_byte: line_end_byte(&line_spans, end_line),
            end_line,
            name,
            signature: line.trim().to_string(),
            start_line,
        });
    }

    Ok(out)
}

fn extract_js_ts_artefacts(content: &str, path: &str) -> Result<Vec<JsTsArtefact>> {
    if let Some(items) = extract_js_ts_artefacts_treesitter(content, path)? {
        return Ok(items);
    }

    extract_js_ts_artefacts_regex(content, path)
}

fn extract_js_ts_artefacts_regex(content: &str, path: &str) -> Result<Vec<JsTsArtefact>> {
    let import_re = Regex::new(r#"^\s*import\b.*$"#)?;
    let from_re = Regex::new(r#"from\s+['"]([^'"]+)['"]"#)?;
    let side_effect_import_re = Regex::new(r#"^\s*import\s+['"]([^'"]+)['"]"#)?;
    let class_re = Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?class\s+([A-Za-z_][A-Za-z0-9_]*)\b")?;
    let interface_re =
        Regex::new(r"^\s*(?:export\s+)?interface\s+([A-Za-z_][A-Za-z0-9_]*)\b")?;
    let type_re = Regex::new(r"^\s*(?:export\s+)?type\s+([A-Za-z_][A-Za-z0-9_]*)\b")?;
    let variable_re =
        Regex::new(r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\b")?;
    let method_re = Regex::new(
        r"^\s*(?:(?:public|private|protected|static|async|readonly|get|set)\s+)*([A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*\)\s*\{",
    )?;

    let lines: Vec<&str> = content.lines().collect();
    let spans = line_byte_spans(content);
    let functions = extract_js_ts_functions(content)?;
    let mut items: Vec<JsTsArtefact> = Vec::new();
    let mut class_ranges: Vec<(String, i32, i32)> = Vec::new();
    let mut seen: HashSet<(String, String, i32)> = HashSet::new();

    for (idx, line) in lines.iter().enumerate() {
        let start_line = (idx + 1) as i32;
        if let Some(cap) = class_re.captures(line) {
            let name = cap.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            if !name.is_empty() && seen.insert(("class".to_string(), name.clone(), start_line)) {
                let end_line = find_block_end_line(&lines, idx).unwrap_or(start_line);
                let symbol_fqn = format!("{path}::{name}");
                items.push(JsTsArtefact {
                    canonical_kind: "class".to_string(),
                    language_kind: "class_declaration".to_string(),
                    name: name.clone(),
                    symbol_fqn: symbol_fqn.clone(),
                    parent_symbol_fqn: None,
                    start_line,
                    end_line,
                    start_byte: line_start_byte(&spans, start_line),
                    end_byte: line_end_byte(&spans, end_line),
                    signature: line.trim().to_string(),
                });
                class_ranges.push((name, start_line, end_line));
            }
        }

        if let Some(cap) = interface_re.captures(line) {
            let name = cap.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            if !name.is_empty()
                && seen.insert(("interface".to_string(), name.clone(), start_line))
            {
                let end_line = find_block_end_line(&lines, idx).unwrap_or(start_line);
                items.push(JsTsArtefact {
                    canonical_kind: "interface".to_string(),
                    language_kind: "interface_declaration".to_string(),
                    name: name.clone(),
                    symbol_fqn: format!("{path}::{name}"),
                    parent_symbol_fqn: None,
                    start_line,
                    end_line,
                    start_byte: line_start_byte(&spans, start_line),
                    end_byte: line_end_byte(&spans, end_line),
                    signature: line.trim().to_string(),
                });
            }
        }

        if let Some(cap) = type_re.captures(line) {
            let name = cap.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            if !name.is_empty() && seen.insert(("type".to_string(), name.clone(), start_line)) {
                let end_line = if line.contains(';') { start_line } else { find_block_end_line(&lines, idx).unwrap_or(start_line) };
                items.push(JsTsArtefact {
                    canonical_kind: "type".to_string(),
                    language_kind: "type_alias_declaration".to_string(),
                    name: name.clone(),
                    symbol_fqn: format!("{path}::{name}"),
                    parent_symbol_fqn: None,
                    start_line,
                    end_line,
                    start_byte: line_start_byte(&spans, start_line),
                    end_byte: line_end_byte(&spans, end_line),
                    signature: line.trim().to_string(),
                });
            }
        }

        if let Some(cap) = variable_re.captures(line) {
            let name = cap.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            if !name.is_empty()
                && seen.insert(("variable".to_string(), name.clone(), start_line))
            {
                items.push(JsTsArtefact {
                    canonical_kind: "variable".to_string(),
                    language_kind: "variable_declarator".to_string(),
                    name: name.clone(),
                    symbol_fqn: format!("{path}::{name}"),
                    parent_symbol_fqn: None,
                    start_line,
                    end_line: start_line,
                    start_byte: line_start_byte(&spans, start_line),
                    end_byte: line_end_byte(&spans, start_line),
                    signature: line.trim().to_string(),
                });
            }
        }

        if import_re.is_match(line) {
            let module_ref = from_re
                .captures(line)
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
                .or_else(|| {
                    side_effect_import_re
                        .captures(line)
                        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
                });
            if let Some(module_name) = module_ref {
                let import_name = format!("{module_name}@{start_line}");
                if seen.insert(("import".to_string(), import_name.clone(), start_line)) {
                    items.push(JsTsArtefact {
                        canonical_kind: "import".to_string(),
                        language_kind: "import_declaration".to_string(),
                        name: import_name.clone(),
                        symbol_fqn: format!("{path}::import::{import_name}"),
                        parent_symbol_fqn: None,
                        start_line,
                        end_line: start_line,
                        start_byte: line_start_byte(&spans, start_line),
                        end_byte: line_end_byte(&spans, start_line),
                        signature: line.trim().to_string(),
                    });
                }
            }
        }
    }

    for f in functions {
        if seen.insert(("function".to_string(), f.name.clone(), f.start_line)) {
            items.push(JsTsArtefact {
                canonical_kind: "function".to_string(),
                language_kind: "function_declaration".to_string(),
                name: f.name.clone(),
                symbol_fqn: format!("{path}::{}", f.name),
                parent_symbol_fqn: None,
                start_line: f.start_line,
                end_line: f.end_line,
                start_byte: f.start_byte,
                end_byte: f.end_byte,
                signature: f.signature.clone(),
            });
        }
    }

    for (class_name, class_start, class_end) in class_ranges {
        for (idx, line) in lines.iter().enumerate() {
            let line_no = (idx + 1) as i32;
            if line_no <= class_start || line_no > class_end {
                continue;
            }
            let Some(cap) = method_re.captures(line) else {
                continue;
            };
            let Some(method_name_m) = cap.get(1) else {
                continue;
            };
            let method_name = method_name_m.as_str().to_string();
            if method_name == "constructor" {
                continue;
            }
            if !seen.insert(("method".to_string(), format!("{class_name}::{method_name}"), line_no))
            {
                continue;
            }
            let end_line = find_block_end_line(&lines, idx).unwrap_or(line_no);
            let class_fqn = format!("{path}::{class_name}");
            items.push(JsTsArtefact {
                canonical_kind: "method".to_string(),
                language_kind: "method_definition".to_string(),
                name: method_name.clone(),
                symbol_fqn: format!("{class_fqn}::{method_name}"),
                parent_symbol_fqn: Some(class_fqn),
                start_line: line_no,
                end_line,
                start_byte: line_start_byte(&spans, line_no),
                end_byte: line_end_byte(&spans, end_line),
                signature: line.trim().to_string(),
            });
        }
    }

    items.sort_by_key(|i| (i.start_line, i.end_line, i.canonical_kind.clone(), i.name.clone()));
    Ok(items)
}

fn extract_js_ts_artefacts_treesitter(content: &str, path: &str) -> Result<Option<Vec<JsTsArtefact>>> {
    let mut parser = tree_sitter::Parser::new();
    let ts_lang: tree_sitter::Language = tree_sitter_typescript::language_typescript();
    let js_lang: tree_sitter::Language = tree_sitter_javascript::language();

    let mut out = Vec::new();
    let mut seen: HashSet<(String, String, i32)> = HashSet::new();

    for lang in [ts_lang, js_lang] {
        if parser.set_language(&lang).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(content, None) else {
            continue;
        };
        let root = tree.root_node();
        if root.has_error() {
            continue;
        }
        collect_js_ts_nodes_recursive(root, content, path, &mut out, &mut seen);
        if !out.is_empty() {
            out.sort_by_key(|i| (i.start_line, i.end_line, i.canonical_kind.clone(), i.name.clone()));
            return Ok(Some(out));
        }
    }

    Ok(None)
}

fn collect_js_ts_nodes_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<JsTsArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
) {
    let kind = node.kind();
    let start_line = node.start_position().row as i32 + 1;
    let end_line = node.end_position().row as i32 + 1;
    let start_byte = node.start_byte() as i32;
    let end_byte = node.end_byte() as i32;
    let line_sig = node
        .utf8_text(content.as_bytes())
        .ok()
        .and_then(|s| s.lines().next())
        .unwrap_or("")
        .trim()
        .to_string();

    let mut push = |canonical_kind: &str,
                    language_kind: &str,
                    name: String,
                    symbol_fqn: String,
                    parent_symbol_fqn: Option<String>| {
        if name.is_empty() {
            return;
        }
        if !seen.insert((canonical_kind.to_string(), name.clone(), start_line)) {
            return;
        }
        out.push(JsTsArtefact {
            canonical_kind: canonical_kind.to_string(),
            language_kind: language_kind.to_string(),
            name,
            symbol_fqn,
            parent_symbol_fqn,
            start_line,
            end_line,
            start_byte,
            end_byte,
            signature: line_sig.clone(),
        });
    };

    match kind {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push("function", "function_declaration", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    let class_fqn = format!("{path}::{name}");
                    push("class", "class_declaration", name.to_string(), class_fqn.clone(), None);
                    if let Some(body) = node.child_by_field_name("body") {
                        let mut cur = body.walk();
                        for child in body.named_children(&mut cur) {
                            if child.kind() == "method_definition" {
                                if let Some(mn) = child.child_by_field_name("name") {
                                    if let Ok(method_name) = mn.utf8_text(content.as_bytes()) {
                                        let m_start_line = child.start_position().row as i32 + 1;
                                        let m_end_line = child.end_position().row as i32 + 1;
                                        let m_start_byte = child.start_byte() as i32;
                                        let m_end_byte = child.end_byte() as i32;
                                        let m_sig = child
                                            .utf8_text(content.as_bytes())
                                            .ok()
                                            .and_then(|s| s.lines().next())
                                            .unwrap_or("")
                                            .trim()
                                            .to_string();
                                        if seen.insert((
                                            "method".to_string(),
                                            method_name.to_string(),
                                            m_start_line,
                                        )) {
                                            out.push(JsTsArtefact {
                                                canonical_kind: "method".to_string(),
                                                language_kind: "method_definition".to_string(),
                                                name: method_name.to_string(),
                                                symbol_fqn: format!("{class_fqn}::{method_name}"),
                                                parent_symbol_fqn: Some(class_fqn.clone()),
                                                start_line: m_start_line,
                                                end_line: m_end_line,
                                                start_byte: m_start_byte,
                                                end_byte: m_end_byte,
                                                signature: m_sig,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push("interface", "interface_declaration", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "type_alias_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push("type", "type_alias_declaration", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "variable_declarator" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push("variable", "variable_declarator", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "import_statement" => {
            let import_name = format!("import@{start_line}");
            push(
                "import",
                "import_declaration",
                import_name.clone(),
                format!("{path}::import::{import_name}"),
                None,
            );
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_js_ts_nodes_recursive(child, content, path, out, seen);
    }
}

fn extract_rust_artefacts(content: &str, path: &str) -> Result<Vec<JsTsArtefact>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::language();
    parser
        .set_language(&lang)
        .context("setting tree-sitter rust language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String, i32)> = HashSet::new();
    collect_rust_nodes_recursive(root, content, path, &mut out, &mut seen, None);
    out.sort_by_key(|i| (i.start_line, i.end_line, i.canonical_kind.clone(), i.name.clone()));
    Ok(out)
}

fn collect_rust_nodes_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<JsTsArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
    current_impl_fqn: Option<String>,
) {
    let kind = node.kind();
    let start_line = node.start_position().row as i32 + 1;
    let end_line = node.end_position().row as i32 + 1;
    let start_byte = node.start_byte() as i32;
    let end_byte = node.end_byte() as i32;
    let signature = node
        .utf8_text(content.as_bytes())
        .ok()
        .and_then(|s| s.lines().next())
        .unwrap_or("")
        .trim()
        .to_string();

    let push = |out: &mut Vec<JsTsArtefact>,
                seen: &mut HashSet<(String, String, i32)>,
                canonical_kind: &str,
                language_kind: &str,
                name: String,
                symbol_fqn: String,
                parent_symbol_fqn: Option<String>| {
        if name.is_empty() || !seen.insert((canonical_kind.to_string(), name.clone(), start_line)) {
            return;
        }
        out.push(JsTsArtefact {
            canonical_kind: canonical_kind.to_string(),
            language_kind: language_kind.to_string(),
            name,
            symbol_fqn,
            parent_symbol_fqn,
            start_line,
            end_line,
            start_byte,
            end_byte,
            signature: signature.clone(),
        });
    };

    let mut next_impl_fqn = current_impl_fqn.clone();

    match kind {
        "mod_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(out, seen, "module", "mod_item", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "struct_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(out, seen, "struct", "struct_item", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "enum_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(out, seen, "enum", "enum_item", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(out, seen, "trait", "trait_item", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "type_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(out, seen, "type", "type_item", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "const_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(out, seen, "const", "const_item", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "static_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(out, seen, "static", "static_item", name.to_string(), format!("{path}::{name}"), None);
                }
            }
        }
        "use_declaration" => {
            let name = format!("use@{start_line}");
            push(out, seen, "import", "use_declaration", name.clone(), format!("{path}::{name}"), None);
        }
        "impl_item" => {
            let name = format!("impl@{start_line}");
            let impl_fqn = format!("{path}::{name}");
            push(out, seen, "impl", "impl_item", name.clone(), impl_fqn.clone(), None);
            next_impl_fqn = Some(impl_fqn);
        }
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    if let Some(impl_fqn) = current_impl_fqn.clone() {
                        push(
                            out,
                            seen,
                            "method",
                            "function_item",
                            name.to_string(),
                            format!("{impl_fqn}::{name}"),
                            Some(impl_fqn),
                        );
                    } else {
                        push(out, seen, "function", "function_item", name.to_string(), format!("{path}::{name}"), None);
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_nodes_recursive(child, content, path, out, seen, next_impl_fqn.clone());
    }
}

fn extract_js_ts_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[JsTsArtefact],
) -> Result<Vec<JsTsDependencyEdge>> {
    let mut edges = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let import_re = Regex::new(r#"^\s*import\s+(.+?)\s+from\s+['"]([^'"]+)['"]\s*;?\s*$"#)?;
    let side_effect_import_re = Regex::new(r#"^\s*import\s+['"]([^'"]+)['"]\s*;?\s*$"#)?;
    let call_ident_re = Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(")?;
    let call_member_re = Regex::new(r"\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(")?;
    let function_decl_re =
        Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+[A-Za-z_]")?;
    let method_decl_re = Regex::new(
        r"^\s*(?:(?:public|private|protected|static|async|readonly|get|set)\s+)*[A-Za-z_][A-Za-z0-9_]*\s*\([^;]*\)\s*\{",
    )?;

    let callables = artefacts
        .iter()
        .filter(|a| a.canonical_kind == "function" || a.canonical_kind == "method")
        .cloned()
        .collect::<Vec<_>>();
    let mut callable_name_to_fqn: HashMap<String, String> = HashMap::new();
    for c in &callables {
        callable_name_to_fqn
            .entry(c.name.clone())
            .or_insert_with(|| c.symbol_fqn.clone());
    }

    let mut imported_symbol_refs: HashMap<String, String> = HashMap::new();

    for (idx, line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as i32;
        let trimmed = line.trim();

        if let Some(caps) = import_re.captures(line) {
            let clause = caps.get(1).map(|m| m.as_str()).unwrap_or("").trim();
            let module_ref = caps.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
            if !module_ref.is_empty() {
                edges.push(JsTsDependencyEdge {
                    edge_kind: "imports".to_string(),
                    from_symbol_fqn: path.to_string(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(module_ref.clone()),
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: json!({"import_form":"module"}),
                });
            }
            parse_import_clause_symbols(clause, &module_ref, &mut imported_symbol_refs);
            continue;
        }

        if let Some(caps) = side_effect_import_re.captures(line) {
            let module_ref = caps.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
            if !module_ref.is_empty() {
                edges.push(JsTsDependencyEdge {
                    edge_kind: "imports".to_string(),
                    from_symbol_fqn: path.to_string(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(module_ref),
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: json!({"import_form":"side_effect"}),
                });
            }
            continue;
        }

        let Some(owner) = smallest_enclosing_callable(line_no, &callables) else {
            continue;
        };
        if function_decl_re.is_match(line) || method_decl_re.is_match(line) {
            continue;
        }

        for caps in call_ident_re.captures_iter(line) {
            let Some(name_m) = caps.get(1) else {
                continue;
            };
            let name = name_m.as_str().to_string();
            if is_control_keyword(&name) {
                continue;
            }

            if let Some(target_fqn) = callable_name_to_fqn.get(&name) {
                edges.push(JsTsDependencyEdge {
                    edge_kind: "calls".to_string(),
                    from_symbol_fqn: owner.symbol_fqn.clone(),
                    to_target_symbol_fqn: Some(target_fqn.clone()),
                    to_symbol_ref: None,
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: json!({"call_form":"identifier","resolution":"local"}),
                });
            } else if let Some(import_ref) = imported_symbol_refs.get(&name) {
                edges.push(JsTsDependencyEdge {
                    edge_kind: "calls".to_string(),
                    from_symbol_fqn: owner.symbol_fqn.clone(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(import_ref.clone()),
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: json!({"call_form":"identifier","resolution":"import"}),
                });
            } else {
                edges.push(JsTsDependencyEdge {
                    edge_kind: "calls".to_string(),
                    from_symbol_fqn: owner.symbol_fqn.clone(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(format!("{path}::{name}")),
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: json!({"call_form":"identifier","resolution":"unresolved"}),
                });
            }
        }

        for caps in call_member_re.captures_iter(line) {
            let Some(name_m) = caps.get(1) else {
                continue;
            };
            let name = name_m.as_str().to_string();
            edges.push(JsTsDependencyEdge {
                edge_kind: "calls".to_string(),
                from_symbol_fqn: owner.symbol_fqn.clone(),
                to_target_symbol_fqn: None,
                to_symbol_ref: Some(format!("{path}::member::{name}")),
                start_line: Some(line_no),
                end_line: Some(line_no),
                metadata: json!({"call_form":"member","resolution":"unresolved"}),
            });
        }

        if trimmed.is_empty() {
            continue;
        }
    }

    Ok(edges)
}

fn extract_rust_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[JsTsArtefact],
) -> Result<Vec<JsTsDependencyEdge>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::language();
    parser
        .set_language(&lang)
        .context("setting tree-sitter rust language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut edges = Vec::new();
    let rust_callables = artefacts
        .iter()
        .filter(|a| a.canonical_kind == "function" || a.canonical_kind == "method")
        .cloned()
        .collect::<Vec<_>>();
    let mut name_to_fqn = HashMap::new();
    for c in &rust_callables {
        name_to_fqn
            .entry(c.name.clone())
            .or_insert_with(|| c.symbol_fqn.clone());
    }

    collect_rust_edges_recursive(
        root,
        content,
        path,
        &rust_callables,
        &name_to_fqn,
        &mut edges,
    );
    Ok(edges)
}

fn collect_rust_edges_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    rust_callables: &[JsTsArtefact],
    callable_name_to_fqn: &HashMap<String, String>,
    out: &mut Vec<JsTsDependencyEdge>,
) {
    let kind = node.kind();
    let start_line = node.start_position().row as i32 + 1;

    if kind == "use_declaration" {
        if let Ok(text) = node.utf8_text(content.as_bytes()) {
            let cleaned = text
                .trim()
                .trim_start_matches("use")
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string();
            if !cleaned.is_empty() {
                out.push(JsTsDependencyEdge {
                    edge_kind: "imports".to_string(),
                    from_symbol_fqn: path.to_string(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(cleaned),
                    start_line: Some(start_line),
                    end_line: Some(node.end_position().row as i32 + 1),
                    metadata: json!({"import_form":"use"}),
                });
            }
        }
    }

    if kind == "call_expression" || kind == "method_call_expression" {
        let owner = smallest_enclosing_callable(start_line, rust_callables);
        if let Some(owner) = owner {
            let target_name = node
                .child_by_field_name("function")
                .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                .map(|s| s.split("::").last().unwrap_or(s).to_string())
                .or_else(|| {
                    node.child_by_field_name("name")
                        .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                        .map(|s| s.to_string())
                });

            if let Some(target_name) = target_name {
                if let Some(target_fqn) = callable_name_to_fqn.get(&target_name) {
                    out.push(JsTsDependencyEdge {
                        edge_kind: "calls".to_string(),
                        from_symbol_fqn: owner.symbol_fqn.clone(),
                        to_target_symbol_fqn: Some(target_fqn.clone()),
                        to_symbol_ref: None,
                        start_line: Some(start_line),
                        end_line: Some(start_line),
                        metadata: json!({"call_form":"rust","resolution":"local"}),
                    });
                } else {
                    out.push(JsTsDependencyEdge {
                        edge_kind: "calls".to_string(),
                        from_symbol_fqn: owner.symbol_fqn.clone(),
                        to_target_symbol_fqn: None,
                        to_symbol_ref: Some(format!("{path}::{target_name}")),
                        start_line: Some(start_line),
                        end_line: Some(start_line),
                        metadata: json!({"call_form":"rust","resolution":"unresolved"}),
                    });
                }
            }
        }
    }

    if kind == "impl_item" {
        if let Ok(text) = node.utf8_text(content.as_bytes()) {
            let impl_re = Regex::new(r"impl\s+([A-Za-z0-9_:<>]+)\s+for\s+([A-Za-z0-9_:<>]+)");
            if let Ok(impl_re) = impl_re {
                if let Some(cap) = impl_re.captures(text) {
                    let trait_name = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                    if !trait_name.is_empty() {
                        out.push(JsTsDependencyEdge {
                            edge_kind: "implements".to_string(),
                            from_symbol_fqn: format!("{path}::impl@{start_line}"),
                            to_target_symbol_fqn: None,
                            to_symbol_ref: Some(trait_name),
                            start_line: Some(start_line),
                            end_line: Some(node.end_position().row as i32 + 1),
                            metadata: json!({"relation":"trait_impl"}),
                        });
                    }
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_edges_recursive(
            child,
            content,
            path,
            rust_callables,
            callable_name_to_fqn,
            out,
        );
    }
}

fn smallest_enclosing_callable(line_no: i32, callables: &[JsTsArtefact]) -> Option<JsTsArtefact> {
    callables
        .iter()
        .filter(|c| c.start_line <= line_no && c.end_line >= line_no)
        .min_by_key(|c| c.end_line - c.start_line)
        .cloned()
}

fn is_control_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "for" | "while" | "switch" | "catch" | "return" | "new" | "typeof"
    )
}

fn parse_import_clause_symbols(
    clause: &str,
    module_ref: &str,
    imported_symbol_refs: &mut HashMap<String, String>,
) {
    let trimmed = clause.trim();
    if trimmed.is_empty() {
        return;
    }

    if let Some((default_part, rest)) = trimmed.split_once(',') {
        let default_alias = default_part.trim();
        if !default_alias.is_empty() {
            imported_symbol_refs.insert(default_alias.to_string(), format!("{module_ref}::default"));
        }
        parse_import_clause_symbols(rest, module_ref, imported_symbol_refs);
        return;
    }

    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let inner = &trimmed[1..trimmed.len() - 1];
        for part in inner.split(',') {
            let token = part.trim();
            if token.is_empty() {
                continue;
            }
            if let Some((orig, alias)) = token.split_once(" as ") {
                let orig = orig.trim();
                let alias = alias.trim();
                if !orig.is_empty() && !alias.is_empty() {
                    imported_symbol_refs.insert(alias.to_string(), format!("{module_ref}::{orig}"));
                }
            } else {
                imported_symbol_refs.insert(token.to_string(), format!("{module_ref}::{token}"));
            }
        }
        return;
    }

    if let Some(ns) = trimmed.strip_prefix("* as ") {
        let ns = ns.trim();
        if !ns.is_empty() {
            imported_symbol_refs.insert(ns.to_string(), format!("{module_ref}::*"));
        }
        return;
    }

    imported_symbol_refs.insert(trimmed.to_string(), format!("{module_ref}::default"));
}

fn line_byte_spans(content: &str) -> Vec<(i32, i32)> {
    if content.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut cursor: i32 = 0;
    for segment in content.split_inclusive('\n') {
        let start = cursor;
        let end = start + segment.len() as i32;
        spans.push((start, end));
        cursor = end;
    }
    spans
}

fn line_start_byte(spans: &[(i32, i32)], line: i32) -> i32 {
    if line <= 0 {
        return 0;
    }
    spans
        .get((line - 1) as usize)
        .map(|(start, _)| *start)
        .unwrap_or(0)
}

fn line_end_byte(spans: &[(i32, i32)], line: i32) -> i32 {
    if line <= 0 {
        return 0;
    }
    if let Some((_, end)) = spans.get((line - 1) as usize) {
        return *end;
    }
    spans.last().map(|(_, end)| *end).unwrap_or(0)
}

fn find_block_end_line(lines: &[&str], start_index: usize) -> Option<i32> {
    let mut found_open = false;
    let mut depth = 0i32;

    for (line_idx, line) in lines.iter().enumerate().skip(start_index) {
        for ch in line.chars() {
            match ch {
                '{' => {
                    found_open = true;
                    depth += 1;
                }
                '}' if found_open => {
                    depth -= 1;
                    if depth <= 0 {
                        return Some((line_idx + 1) as i32);
                    }
                }
                _ => {}
            }
        }
    }

    if found_open {
        Some(lines.len() as i32)
    } else {
        None
    }
}

#[derive(Debug, Clone, Default)]
struct ParsedDevqlQuery {
    repo: Option<String>,
    as_of: Option<AsOfSelector>,
    file: Option<String>,
    files_path: Option<String>,
    artefacts: ArtefactFilter,
    checkpoints: CheckpointFilter,
    telemetry: TelemetryFilter,
    deps: DepsFilter,
    has_artefacts_stage: bool,
    has_deps_stage: bool,
    has_checkpoints_stage: bool,
    has_telemetry_stage: bool,
    has_chat_history_stage: bool,
    limit: usize,
    select_fields: Vec<String>,
}

#[derive(Debug, Clone)]
enum AsOfSelector {
    Ref(String),
    Commit(String),
}

#[derive(Debug, Clone, Default)]
struct ArtefactFilter {
    kind: Option<String>,
    lines: Option<(i32, i32)>,
    agent: Option<String>,
    since: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct CheckpointFilter {
    agent: Option<String>,
    since: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct TelemetryFilter {
    event_type: Option<String>,
    agent: Option<String>,
    since: Option<String>,
}

#[derive(Debug, Clone)]
struct DepsFilter {
    kind: Option<String>,
    direction: String,
    include_unresolved: bool,
}

impl Default for DepsFilter {
    fn default() -> Self {
        Self {
            kind: None,
            direction: "out".to_string(),
            include_unresolved: true,
        }
    }
}
