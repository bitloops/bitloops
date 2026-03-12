#[derive(Debug, Clone, Default)]
struct IngestionCounters {
    checkpoints_processed: usize,
    events_inserted: usize,
    artefacts_upserted: usize,
    checkpoints_without_commit: usize,
    semantic_feature_rows_upserted: usize,
    semantic_feature_rows_skipped: usize,
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

async fn init_events_schema(cfg: &DevqlConfig) -> Result<()> {
    events_store_init_schema(cfg).await
}

const RELATIONAL_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    organization TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
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
    symbol_id TEXT,
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
    start_byte INTEGER,
    end_byte INTEGER,
    signature TEXT,
    content_hash TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS symbol_id TEXT;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS start_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS end_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS signature TEXT;

CREATE INDEX IF NOT EXISTS artefacts_blob_idx
ON artefacts (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefacts_path_idx
ON artefacts (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_kind_idx
ON artefacts (repo_id, canonical_kind);

CREATE TABLE IF NOT EXISTS symbol_semantics (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    doc_comment_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence DOUBLE PRECISION NOT NULL,
    source_model TEXT,
    generated_at TIMESTAMPTZ DEFAULT now()
);

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'symbol_semantics' AND column_name = 'stage1_input_hash'
    ) AND NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'symbol_semantics' AND column_name = 'semantic_features_input_hash'
    ) THEN
        ALTER TABLE symbol_semantics RENAME COLUMN stage1_input_hash TO semantic_features_input_hash;
    END IF;
END $$;

ALTER TABLE symbol_semantics
ADD COLUMN IF NOT EXISTS semantic_features_input_hash TEXT;

ALTER TABLE symbol_semantics
ADD COLUMN IF NOT EXISTS doc_comment_summary TEXT;

ALTER TABLE symbol_semantics
ADD COLUMN IF NOT EXISTS llm_summary TEXT;

ALTER TABLE symbol_semantics
ADD COLUMN IF NOT EXISTS template_summary TEXT;

ALTER TABLE symbol_semantics
DROP COLUMN IF EXISTS role_tag;

UPDATE symbol_semantics
SET template_summary = summary
WHERE template_summary IS NULL;

CREATE INDEX IF NOT EXISTS symbol_semantics_repo_blob_idx
ON symbol_semantics (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_features (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    identifier_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    normalized_body_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    parent_kind TEXT,
    parent_symbol TEXT,
    local_relationships JSONB NOT NULL DEFAULT '[]'::jsonb,
    context_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    generated_at TIMESTAMPTZ DEFAULT now()
);

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'symbol_features' AND column_name = 'stage1_input_hash'
    ) AND NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'symbol_features' AND column_name = 'semantic_features_input_hash'
    ) THEN
        ALTER TABLE symbol_features RENAME COLUMN stage1_input_hash TO semantic_features_input_hash;
    END IF;
END $$;

ALTER TABLE symbol_features
ADD COLUMN IF NOT EXISTS semantic_features_input_hash TEXT;

CREATE INDEX IF NOT EXISTS symbol_features_repo_blob_idx
ON symbol_features (repo_id, blob_sha);
"#;

async fn init_relational_schema(relational_store: &dyn store_contracts::RelationalStore) -> Result<()> {
    relational_store.init_schema().await
}

async fn ensure_repository_row(
    cfg: &DevqlConfig,
    relational_store: &dyn store_contracts::RelationalStore,
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
    relational_store.execute(&sql).await
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
    events_store_existing_event_ids(cfg, &cfg.repo.repo_id).await
}

async fn insert_checkpoint_event(
    cfg: &DevqlConfig,
    cp: &CommittedInfo,
    event_id: &str,
    commit_info: Option<&CheckpointCommitInfo>,
) -> Result<()> {
    let payload = json!({
        "checkpoints_count": cp.checkpoints_count,
        "session_count": cp.session_count,
        "token_usage": cp.token_usage,
    });

    let event = store_contracts::CheckpointEventWrite {
        event_id: event_id.to_string(),
        repo_id: cfg.repo.repo_id.clone(),
        checkpoint_id: cp.checkpoint_id.clone(),
        session_id: cp.session_id.clone(),
        commit_sha: commit_info
            .map(|info| info.commit_sha.clone())
            .unwrap_or_default(),
        commit_unix: commit_info.map(|info| info.commit_unix),
        branch: cp.branch.clone(),
        event_type: "checkpoint_committed".to_string(),
        agent: cp.agent.clone(),
        strategy: cp.strategy.clone(),
        files_touched: cp.files_touched.clone(),
        created_at: Some(cp.created_at.trim().to_string()).filter(|value| !value.is_empty()),
        payload,
    };

    events_store_insert_checkpoint_event(cfg, event).await
}

async fn upsert_commit_row(
    cfg: &DevqlConfig,
    relational_store: &dyn store_contracts::RelationalStore,
    cp: &CommittedInfo,
    commit_info: &CheckpointCommitInfo,
) -> Result<()> {
    let committed_at_expr = match relational_store.provider() {
        RelationalProvider::Postgres => format!("to_timestamp({})", commit_info.commit_unix),
        RelationalProvider::Sqlite => {
            format!("datetime({}, 'unixepoch')", commit_info.commit_unix)
        }
    };
    let sql = format!(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at) VALUES ('{}', '{}', '{}', '{}', '{}', {}) \
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
        committed_at_expr,
    );

    relational_store.execute(&sql).await
}

async fn upsert_file_state_row(
    cfg: &DevqlConfig,
    relational_store: &dyn store_contracts::RelationalStore,
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

    relational_store.execute(&sql).await
}

#[derive(Debug, Clone)]
struct FileArtefactRow {
    artefact_id: String,
    language: String,
}

async fn upsert_file_artefact_row(
    cfg: &DevqlConfig,
    relational_store: &dyn store_contracts::RelationalStore,
    path: &str,
    blob_sha: &str,
) -> Result<FileArtefactRow> {
    let artefact_id =
        deterministic_uuid(&format!("{}|{}|{}|file", cfg.repo.repo_id, blob_sha, path));
    let language = detect_language(path);
    let line_count = git_blob_line_count(&cfg.repo_root, blob_sha)
        .unwrap_or(1)
        .max(1);

    let sql = format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', 'file', 'module', '{}', NULL, 1, {}, 0, NULL, '{}', '{}') \
ON CONFLICT (artefact_id) DO UPDATE SET symbol_id = EXCLUDED.symbol_id, repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, content_hash = EXCLUDED.content_hash",
        esc_pg(&artefact_id),
        esc_pg(&artefact_id),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(blob_sha),
        esc_pg(path),
        esc_pg(&language),
        esc_pg(path),
        line_count,
        esc_pg(path),
        esc_pg(blob_sha),
    );

    relational_store.execute(&sql).await?;
    Ok(FileArtefactRow {
        artefact_id,
        language,
    })
}

async fn upsert_language_artefacts(
    cfg: &DevqlConfig,
    relational_store: &dyn store_contracts::RelationalStore,
    path: &str,
    blob_sha: &str,
    file_artefact: &FileArtefactRow,
) -> Result<()> {
    if file_artefact.language != "typescript" && file_artefact.language != "javascript" {
        return Ok(());
    }

    let Some(content) = git_blob_content(&cfg.repo_root, blob_sha) else {
        return Ok(());
    };

    let functions = extract_js_ts_functions(&content)?;
    for item in functions {
        let artefact_id = deterministic_uuid(&format!(
            "{}|{}|{}|function|{}|{}",
            cfg.repo.repo_id, blob_sha, path, item.name, item.start_line
        ));
        let symbol_fqn = format!("{}::{}", path, item.name);
        let content_hash = deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}",
            blob_sha, path, item.name, item.start_line, item.end_line
        ));

        let sql = format!(
            "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', 'function', 'function', '{}', '{}', {}, {}, NULL, NULL, {}, '{}') \
ON CONFLICT (artefact_id) DO UPDATE SET symbol_id = EXCLUDED.symbol_id, repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, path = EXCLUDED.path, language = EXCLUDED.language, canonical_kind = EXCLUDED.canonical_kind, language_kind = EXCLUDED.language_kind, symbol_fqn = EXCLUDED.symbol_fqn, parent_artefact_id = EXCLUDED.parent_artefact_id, start_line = EXCLUDED.start_line, end_line = EXCLUDED.end_line, start_byte = EXCLUDED.start_byte, end_byte = EXCLUDED.end_byte, signature = EXCLUDED.signature, content_hash = EXCLUDED.content_hash",
            esc_pg(&artefact_id),
            esc_pg(&artefact_id),
            esc_pg(&cfg.repo.repo_id),
            esc_pg(blob_sha),
            esc_pg(path),
            esc_pg(&file_artefact.language),
            esc_pg(&symbol_fqn),
            esc_pg(&file_artefact.artefact_id),
            item.start_line,
            item.end_line,
            item.signature
                .as_deref()
                .map(|value| format!("'{}'", esc_pg(value)))
                .unwrap_or_else(|| "NULL".to_string()),
            esc_pg(&content_hash),
        );

        relational_store.execute(&sql).await?;
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
    signature: Option<String>,
}

fn extract_js_ts_functions(content: &str) -> Result<Vec<FunctionArtefact>> {
    let function_decl = Regex::new(
        r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
    )?;
    let function_expr = Regex::new(
        r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:async\s*)?(?:function\s*)?\([^)]*\)\s*=>",
    )?;

    let lines: Vec<&str> = content.lines().collect();
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
            end_line,
            name,
            signature: extract_signature_from_line(line),
            start_line,
        });
    }

    Ok(out)
}

fn extract_signature_from_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let signature = trimmed.trim_end_matches('{').trim();
    if signature.is_empty() {
        None
    } else {
        Some(signature.to_string())
    }
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
    has_artefacts_stage: bool,
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
