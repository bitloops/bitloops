use anyhow::{Context, Result, anyhow};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::storage::SqliteConnectionPool;

use super::descriptor::{CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_DESCRIPTOR};
use super::distillation::{GuidanceDistillationInput, GuidanceToolEvidence};
use super::types::{
    GuidanceDistillationOutput, GuidanceFactCategory, GuidanceFactConfidence, GuidanceFactDraft,
};
use super::workplane::history_source_scope_key;

pub trait ContextGuidanceRepository: Send + Sync {
    fn persist_history_guidance_distillation(
        &self,
        repo_id: &str,
        input: &GuidanceDistillationInput,
        output: &GuidanceDistillationOutput,
        source_model: Option<&str>,
        source_profile: Option<&str>,
    ) -> Result<PersistGuidanceOutcome>;

    fn list_selected_context_guidance(
        &self,
        input: ListSelectedContextGuidanceInput,
    ) -> Result<Vec<PersistedGuidanceFact>>;

    fn health_check(&self, repo_id: &str) -> Result<()>;
}

pub struct PersistGuidanceOutcome {
    pub inserted_run: bool,
    pub inserted_facts: usize,
    pub unchanged: bool,
}

pub struct ListSelectedContextGuidanceInput {
    pub repo_id: String,
    pub selected_paths: Vec<String>,
    pub selected_symbol_ids: Vec<String>,
    pub selected_symbol_fqns: Vec<String>,
    pub agent: Option<String>,
    pub since: Option<String>,
    pub evidence_kind: Option<String>,
    pub category: Option<GuidanceFactCategory>,
    pub kind: Option<String>,
    pub limit: usize,
}

pub struct PersistedGuidanceDistillationRun {
    pub run_id: String,
    pub repo_id: String,
    pub capability_id: String,
    pub capability_version: Option<String>,
    pub source_scope_key: String,
    pub input_hash: String,
    pub summary_json: Value,
    pub source_model: Option<String>,
    pub source_profile: Option<String>,
    pub generated_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PersistedGuidanceFact {
    pub guidance_id: String,
    pub run_id: String,
    pub repo_id: String,
    pub active: bool,
    pub category: GuidanceFactCategory,
    pub kind: String,
    pub guidance: String,
    pub evidence_excerpt: String,
    pub confidence: GuidanceFactConfidence,
    pub source_model: Option<String>,
    pub generated_at: Option<String>,
    pub targets: Vec<PersistedGuidanceTarget>,
    pub sources: Vec<PersistedGuidanceSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedGuidanceTarget {
    pub target_type: String,
    pub target_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedGuidanceSource {
    pub source_type: String,
    pub source_id: String,
    pub checkpoint_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub tool_invocation_id: Option<String>,
    pub tool_kind: Option<String>,
    pub event_time: Option<String>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub evidence_kind: Option<String>,
    pub match_strength: Option<String>,
    pub knowledge_item_id: Option<String>,
    pub knowledge_item_version_id: Option<String>,
    pub relation_assertion_id: Option<String>,
    pub provider: Option<String>,
    pub source_kind: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub excerpt: Option<String>,
}

pub struct SqliteContextGuidanceRepository {
    sqlite: SqliteConnectionPool,
}

impl SqliteContextGuidanceRepository {
    pub fn new(sqlite: SqliteConnectionPool) -> Self {
        Self { sqlite }
    }

    pub fn initialise_schema(&self) -> Result<()> {
        self.sqlite
            .execute_batch(context_guidance_sqlite_schema_sql())
            .context("initialising SQLite context guidance schema")
    }
}

impl ContextGuidanceRepository for SqliteContextGuidanceRepository {
    fn persist_history_guidance_distillation(
        &self,
        repo_id: &str,
        input: &GuidanceDistillationInput,
        output: &GuidanceDistillationOutput,
        source_model: Option<&str>,
        source_profile: Option<&str>,
    ) -> Result<PersistGuidanceOutcome> {
        let source_scope_key = history_source_scope_key(
            input.session_id.as_str(),
            input.turn_id.as_deref(),
            input.checkpoint_id.as_deref(),
        );
        let input_hash = guidance_input_hash(input);
        let run_id = guidance_run_id(repo_id, source_scope_key.as_str(), input_hash.as_str());
        let summary_json =
            serde_json::to_string(&output.summary).context("serializing guidance summary")?;

        self.sqlite.with_connection(|conn| {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT run_id FROM context_guidance_distillation_runs
                     WHERE repo_id = ?1 AND source_scope_key = ?2 AND input_hash = ?3
                     LIMIT 1",
                    params![repo_id, source_scope_key, input_hash],
                    |row| row.get(0),
                )
                .optional()
                .context("checking existing context guidance distillation run")?;
            if existing.is_some() {
                return Ok(PersistGuidanceOutcome {
                    inserted_run: false,
                    inserted_facts: 0,
                    unchanged: true,
                });
            }

            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")
                .context("starting context guidance persistence transaction")?;
            let result = (|| -> Result<PersistGuidanceOutcome> {
                conn.execute(
                    "UPDATE context_guidance_facts
                     SET active = 0, updated_at = datetime('now')
                     WHERE repo_id = ?1
                       AND active = 1
                       AND run_id IN (
                         SELECT run_id FROM context_guidance_distillation_runs
                         WHERE repo_id = ?1 AND source_scope_key = ?2
                       )",
                    params![repo_id, source_scope_key],
                )
                .context("inactivating previous context guidance facts")?;

                conn.execute(
                    "INSERT INTO context_guidance_distillation_runs (
                        run_id, repo_id, capability_id, capability_version, source_scope_key,
                        input_hash, summary_json, source_model, source_profile, status
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'completed')",
                    params![
                        run_id,
                        repo_id,
                        CONTEXT_GUIDANCE_CAPABILITY_ID,
                        CONTEXT_GUIDANCE_DESCRIPTOR.version,
                        source_scope_key,
                        input_hash,
                        summary_json,
                        source_model.unwrap_or(""),
                        source_profile.unwrap_or("")
                    ],
                )
                .context("inserting context guidance distillation run")?;

                let mut inserted_facts = 0;
                for fact in &output.guidance_facts {
                    let targets = targets_for_fact(fact);
                    if targets.is_empty() {
                        continue;
                    }
                    let guidance_id = guidance_id(
                        run_id.as_str(),
                        fact.category,
                        fact.kind.as_str(),
                        fact.guidance.as_str(),
                        &targets,
                    );
                    conn.execute(
                        "INSERT INTO context_guidance_facts (
                            guidance_id, run_id, repo_id, active, category, kind, guidance,
                            evidence_excerpt, confidence
                         ) VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            guidance_id,
                            run_id,
                            repo_id,
                            category_to_storage(fact.category),
                            fact.kind,
                            fact.guidance,
                            fact.evidence_excerpt,
                            confidence_to_storage(fact.confidence)
                        ],
                    )
                    .context("inserting context guidance fact")?;

                    conn.execute(
                        "DELETE FROM context_guidance_targets WHERE guidance_id = ?1",
                        params![guidance_id],
                    )
                    .context("clearing context guidance targets")?;
                    for (index, target) in targets.iter().enumerate() {
                        conn.execute(
                            "INSERT INTO context_guidance_targets (
                                target_row_id, guidance_id, repo_id, target_type, target_value
                             ) VALUES (?1, ?2, ?3, ?4, ?5)",
                            params![
                                format!("{guidance_id}:target:{index}"),
                                guidance_id,
                                repo_id,
                                target.target_type,
                                target.target_value
                            ],
                        )
                        .context("inserting context guidance target")?;
                    }

                    conn.execute(
                        "DELETE FROM context_guidance_sources WHERE guidance_id = ?1",
                        params![guidance_id],
                    )
                    .context("clearing context guidance sources")?;
                    let sources = sources_for_history_fact(input, fact);
                    for (index, source) in sources.iter().enumerate() {
                        insert_source(conn, repo_id, guidance_id.as_str(), index, source)?;
                    }
                    inserted_facts += 1;
                }

                Ok(PersistGuidanceOutcome {
                    inserted_run: true,
                    inserted_facts,
                    unchanged: false,
                })
            })();

            match result {
                Ok(outcome) => {
                    conn.execute_batch("COMMIT")
                        .context("committing context guidance persistence transaction")?;
                    Ok(outcome)
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(err)
                }
            }
        })
    }

    fn list_selected_context_guidance(
        &self,
        input: ListSelectedContextGuidanceInput,
    ) -> Result<Vec<PersistedGuidanceFact>> {
        self.sqlite.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT f.guidance_id, f.run_id, f.repo_id, f.active, f.category, f.kind,
                        f.guidance, f.evidence_excerpt, f.confidence,
                        NULLIF(r.source_model, ''), r.generated_at
                 FROM context_guidance_facts f
                 JOIN context_guidance_distillation_runs r ON r.run_id = f.run_id
                 WHERE f.repo_id = ?1 AND f.active = 1",
            );
            let mut params_values = vec![input.repo_id.clone()];
            if let Some(category) = input.category {
                sql.push_str(" AND f.category = ?");
                params_values.push(category_to_storage(category).to_string());
            }
            if let Some(kind) = input.kind.as_ref() {
                sql.push_str(" AND f.kind = ?");
                params_values.push(kind.trim().to_string());
            }
            sql.push_str(" ORDER BY r.generated_at DESC, f.guidance_id ASC");

            let mut stmt = conn
                .prepare(&sql)
                .context("preparing context guidance list")?;
            let facts = stmt
                .query_map(rusqlite::params_from_iter(params_values.iter()), |row| {
                    let category_text: String = row.get(4)?;
                    let confidence_text: String = row.get(8)?;
                    Ok(PersistedGuidanceFact {
                        guidance_id: row.get(0)?,
                        run_id: row.get(1)?,
                        repo_id: row.get(2)?,
                        active: row.get::<_, i64>(3)? != 0,
                        category: category_from_storage(category_text.as_str()).map_err(|err| {
                            rusqlite::Error::FromSqlConversionFailure(
                                4,
                                rusqlite::types::Type::Text,
                                Box::new(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    err.to_string(),
                                )),
                            )
                        })?,
                        kind: row.get(5)?,
                        guidance: row.get(6)?,
                        evidence_excerpt: row.get(7)?,
                        confidence: confidence_from_storage(confidence_text.as_str()).map_err(
                            |err| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    8,
                                    rusqlite::types::Type::Text,
                                    Box::new(std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        err.to_string(),
                                    )),
                                )
                            },
                        )?,
                        source_model: row.get(9)?,
                        generated_at: row.get(10)?,
                        targets: Vec::new(),
                        sources: Vec::new(),
                    })
                })
                .context("querying context guidance facts")?;

            let mut out = Vec::new();
            for fact in facts {
                let mut fact = fact.map_err(anyhow::Error::from)?;
                fact.targets = load_targets(conn, fact.guidance_id.as_str())?;
                if !matches_selected_targets(&fact.targets, &input) {
                    continue;
                }
                fact.sources = load_sources(conn, fact.guidance_id.as_str())?;
                if !matches_source_filters(&fact.sources, &input) {
                    continue;
                }
                out.push(fact);
                if out.len() >= input.limit {
                    break;
                }
            }
            Ok(out)
        })
    }

    fn health_check(&self, repo_id: &str) -> Result<()> {
        let _ = repo_id;
        self.sqlite.with_connection(|conn| {
            for table in [
                "context_guidance_distillation_runs",
                "context_guidance_facts",
                "context_guidance_sources",
                "context_guidance_targets",
            ] {
                let exists: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master
                         WHERE type = 'table' AND name = ?1",
                        params![table],
                        |row| row.get(0),
                    )
                    .context("checking context guidance table family")?;
                if exists == 0 {
                    return Err(anyhow!(
                        "context guidance table family is not initialized: missing {table}"
                    ));
                }
            }
            Ok(())
        })
    }
}

pub fn guidance_input_hash(input: &GuidanceDistillationInput) -> String {
    let mut parts = vec![
        input.checkpoint_id.as_deref().unwrap_or("").to_string(),
        input.session_id.clone(),
        input.turn_id.as_deref().unwrap_or("").to_string(),
        input.event_time.as_deref().unwrap_or("").to_string(),
        input.agent_type.as_deref().unwrap_or("").to_string(),
        input.model.as_deref().unwrap_or("").to_string(),
        input.prompt.as_deref().unwrap_or("").to_string(),
        input
            .transcript_fragment
            .as_deref()
            .unwrap_or("")
            .to_string(),
    ];
    parts.extend(
        input
            .files_modified
            .iter()
            .map(|path| path.trim().to_string()),
    );
    for event in &input.tool_events {
        parts.push(event.tool_kind.as_deref().unwrap_or("").to_string());
        parts.push(event.input_summary.as_deref().unwrap_or("").to_string());
        parts.push(event.output_summary.as_deref().unwrap_or("").to_string());
        parts.push(event.command.as_deref().unwrap_or("").to_string());
    }
    sha256_hex(parts.join("\n").as_bytes())
}

pub fn guidance_run_id(repo_id: &str, source_scope_key: &str, input_hash: &str) -> String {
    format!(
        "guidance-run:{}",
        sha256_hex(format!("{repo_id}\n{source_scope_key}\n{input_hash}").as_bytes())
    )
}

pub fn guidance_id(
    run_id: &str,
    category: GuidanceFactCategory,
    kind: &str,
    guidance: &str,
    targets: &[PersistedGuidanceTarget],
) -> String {
    let mut normalized_targets = targets
        .iter()
        .map(|target| format!("{}={}", target.target_type, target.target_value))
        .collect::<Vec<_>>();
    normalized_targets.sort();
    format!(
        "guidance:{}",
        sha256_hex(
            format!(
                "{run_id}\n{}\n{}\n{}\n{}",
                category_to_storage(category),
                kind.trim(),
                guidance.trim(),
                normalized_targets.join("\n")
            )
            .as_bytes()
        )
    )
}

pub fn context_guidance_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS context_guidance_distillation_runs (
    run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    capability_id TEXT NOT NULL DEFAULT 'context_guidance',
    capability_version TEXT DEFAULT '',
    source_scope_key TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    summary_json TEXT NOT NULL DEFAULT '{}',
    source_model TEXT DEFAULT '',
    source_profile TEXT DEFAULT '',
    status TEXT NOT NULL DEFAULT 'completed',
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS context_guidance_runs_scope_input_idx
ON context_guidance_distillation_runs (repo_id, source_scope_key, input_hash);

CREATE INDEX IF NOT EXISTS context_guidance_runs_scope_idx
ON context_guidance_distillation_runs (repo_id, source_scope_key, generated_at);

CREATE TABLE IF NOT EXISTS context_guidance_facts (
    guidance_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    category TEXT NOT NULL,
    kind TEXT NOT NULL,
    guidance TEXT NOT NULL,
    evidence_excerpt TEXT NOT NULL,
    confidence TEXT NOT NULL,
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_facts_repo_category_idx
ON context_guidance_facts (repo_id, active, category, kind);

CREATE INDEX IF NOT EXISTS context_guidance_facts_run_idx
ON context_guidance_facts (run_id);

CREATE TABLE IF NOT EXISTS context_guidance_sources (
    source_row_id TEXT PRIMARY KEY,
    guidance_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    checkpoint_id TEXT,
    session_id TEXT,
    turn_id TEXT,
    tool_invocation_id TEXT,
    tool_kind TEXT,
    event_time TEXT,
    agent_type TEXT,
    model TEXT,
    evidence_kind TEXT,
    match_strength TEXT,
    knowledge_item_id TEXT,
    knowledge_item_version_id TEXT,
    relation_assertion_id TEXT,
    provider TEXT,
    source_kind TEXT,
    title TEXT,
    url TEXT,
    excerpt TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_sources_guidance_idx
ON context_guidance_sources (guidance_id);

CREATE INDEX IF NOT EXISTS context_guidance_sources_history_idx
ON context_guidance_sources (repo_id, checkpoint_id, session_id, turn_id);

CREATE INDEX IF NOT EXISTS context_guidance_sources_filter_idx
ON context_guidance_sources (repo_id, source_type, agent_type, event_time, evidence_kind);

CREATE INDEX IF NOT EXISTS context_guidance_sources_knowledge_idx
ON context_guidance_sources (repo_id, knowledge_item_id, knowledge_item_version_id, relation_assertion_id);

CREATE TABLE IF NOT EXISTS context_guidance_targets (
    target_row_id TEXT PRIMARY KEY,
    guidance_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_value TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_targets_lookup_idx
ON context_guidance_targets (repo_id, target_type, target_value);

CREATE INDEX IF NOT EXISTS context_guidance_targets_guidance_idx
ON context_guidance_targets (guidance_id);
"#
}

fn targets_for_fact(fact: &GuidanceFactDraft) -> Vec<PersistedGuidanceTarget> {
    let mut targets = Vec::new();
    targets.extend(
        fact.applies_to
            .paths
            .iter()
            .map(|path| PersistedGuidanceTarget {
                target_type: "path".to_string(),
                target_value: path.clone(),
            }),
    );
    targets.extend(
        fact.applies_to
            .symbols
            .iter()
            .map(|symbol| PersistedGuidanceTarget {
                target_type: "symbol_fqn".to_string(),
                target_value: symbol.clone(),
            }),
    );
    targets
}

fn sources_for_history_fact(
    input: &GuidanceDistillationInput,
    fact: &GuidanceFactDraft,
) -> Vec<PersistedGuidanceSource> {
    let mut sources = vec![history_turn_source(input, fact)];
    if fact.category == GuidanceFactCategory::Verification {
        sources.extend(
            input
                .tool_events
                .iter()
                .enumerate()
                .map(|(index, event)| history_tool_source(input, index, event, fact)),
        );
    }
    sources
}

fn history_turn_source(
    input: &GuidanceDistillationInput,
    fact: &GuidanceFactDraft,
) -> PersistedGuidanceSource {
    PersistedGuidanceSource {
        source_type: "history.turn".to_string(),
        source_id: format!(
            "{}:{}",
            input.session_id,
            input.turn_id.as_deref().unwrap_or("")
        ),
        checkpoint_id: input.checkpoint_id.clone(),
        session_id: Some(input.session_id.clone()),
        turn_id: input.turn_id.clone(),
        tool_invocation_id: None,
        tool_kind: None,
        event_time: input.event_time.clone(),
        agent_type: input.agent_type.clone(),
        model: input.model.clone(),
        evidence_kind: Some(historical_evidence_kind_for_fact(fact).to_string()),
        match_strength: Some("HIGH".to_string()),
        knowledge_item_id: None,
        knowledge_item_version_id: None,
        relation_assertion_id: None,
        provider: None,
        source_kind: None,
        title: None,
        url: None,
        excerpt: Some(fact.evidence_excerpt.to_string()),
    }
}

fn history_tool_source(
    input: &GuidanceDistillationInput,
    index: usize,
    event: &GuidanceToolEvidence,
    fact: &GuidanceFactDraft,
) -> PersistedGuidanceSource {
    PersistedGuidanceSource {
        source_type: "history.tool_event".to_string(),
        source_id: format!(
            "{}:{}:tool:{index}",
            input.session_id,
            input.turn_id.as_deref().unwrap_or("")
        ),
        checkpoint_id: input.checkpoint_id.clone(),
        session_id: Some(input.session_id.clone()),
        turn_id: input.turn_id.clone(),
        tool_invocation_id: Some(index.to_string()),
        tool_kind: event.tool_kind.clone(),
        event_time: input.event_time.clone(),
        agent_type: input.agent_type.clone(),
        model: input.model.clone(),
        evidence_kind: Some(historical_evidence_kind_for_fact(fact).to_string()),
        match_strength: Some("HIGH".to_string()),
        knowledge_item_id: None,
        knowledge_item_version_id: None,
        relation_assertion_id: None,
        provider: None,
        source_kind: None,
        title: None,
        url: None,
        excerpt: event
            .output_summary
            .clone()
            .or_else(|| Some(fact.evidence_excerpt.clone())),
    }
}

fn historical_evidence_kind_for_fact(fact: &GuidanceFactDraft) -> &'static str {
    if fact.applies_to.symbols.is_empty() {
        "FILE_RELATION"
    } else {
        "SYMBOL_PROVENANCE"
    }
}

fn insert_source(
    conn: &rusqlite::Connection,
    repo_id: &str,
    guidance_id: &str,
    index: usize,
    source: &PersistedGuidanceSource,
) -> Result<()> {
    conn.execute(
        "INSERT INTO context_guidance_sources (
            source_row_id, guidance_id, repo_id, source_type, source_id, checkpoint_id,
            session_id, turn_id, tool_invocation_id, tool_kind, event_time, agent_type,
            model, evidence_kind, match_strength, knowledge_item_id, knowledge_item_version_id,
            relation_assertion_id, provider, source_kind, title, url, excerpt
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
        params![
            format!("{guidance_id}:source:{index}"),
            guidance_id,
            repo_id,
            source.source_type,
            source.source_id,
            source.checkpoint_id,
            source.session_id,
            source.turn_id,
            source.tool_invocation_id,
            source.tool_kind,
            source.event_time,
            source.agent_type,
            source.model,
            source.evidence_kind,
            source.match_strength,
            source.knowledge_item_id,
            source.knowledge_item_version_id,
            source.relation_assertion_id,
            source.provider,
            source.source_kind,
            source.title,
            source.url,
            source.excerpt
        ],
    )
    .context("inserting context guidance source")?;
    Ok(())
}

fn load_targets(
    conn: &rusqlite::Connection,
    guidance_id: &str,
) -> Result<Vec<PersistedGuidanceTarget>> {
    let mut stmt = conn
        .prepare(
            "SELECT target_type, target_value
             FROM context_guidance_targets
             WHERE guidance_id = ?1
             ORDER BY target_type ASC, target_value ASC",
        )
        .context("preparing context guidance targets query")?;
    let rows = stmt
        .query_map(params![guidance_id], |row| {
            Ok(PersistedGuidanceTarget {
                target_type: row.get(0)?,
                target_value: row.get(1)?,
            })
        })
        .context("querying context guidance targets")?;
    rows.map(|row| row.map_err(anyhow::Error::from)).collect()
}

fn load_sources(
    conn: &rusqlite::Connection,
    guidance_id: &str,
) -> Result<Vec<PersistedGuidanceSource>> {
    let mut stmt = conn
        .prepare(
            "SELECT source_type, source_id, checkpoint_id, session_id, turn_id,
                    tool_invocation_id, tool_kind, event_time, agent_type, model,
                    evidence_kind, match_strength, knowledge_item_id,
                    knowledge_item_version_id, relation_assertion_id, provider,
                    source_kind, title, url, excerpt
             FROM context_guidance_sources
             WHERE guidance_id = ?1
             ORDER BY source_row_id ASC",
        )
        .context("preparing context guidance sources query")?;
    let rows = stmt
        .query_map(params![guidance_id], |row| {
            Ok(PersistedGuidanceSource {
                source_type: row.get(0)?,
                source_id: row.get(1)?,
                checkpoint_id: row.get(2)?,
                session_id: row.get(3)?,
                turn_id: row.get(4)?,
                tool_invocation_id: row.get(5)?,
                tool_kind: row.get(6)?,
                event_time: row.get(7)?,
                agent_type: row.get(8)?,
                model: row.get(9)?,
                evidence_kind: row.get(10)?,
                match_strength: row.get(11)?,
                knowledge_item_id: row.get(12)?,
                knowledge_item_version_id: row.get(13)?,
                relation_assertion_id: row.get(14)?,
                provider: row.get(15)?,
                source_kind: row.get(16)?,
                title: row.get(17)?,
                url: row.get(18)?,
                excerpt: row.get(19)?,
            })
        })
        .context("querying context guidance sources")?;
    rows.map(|row| row.map_err(anyhow::Error::from)).collect()
}

fn matches_selected_targets(
    targets: &[PersistedGuidanceTarget],
    input: &ListSelectedContextGuidanceInput,
) -> bool {
    targets
        .iter()
        .any(|target| match target.target_type.as_str() {
            "path" => input
                .selected_paths
                .iter()
                .any(|path| path == &target.target_value),
            "symbol_id" => input
                .selected_symbol_ids
                .iter()
                .any(|symbol_id| symbol_id == &target.target_value),
            "symbol_fqn" => input
                .selected_symbol_fqns
                .iter()
                .any(|symbol_fqn| symbol_fqn == &target.target_value),
            _ => false,
        })
}

fn matches_source_filters(
    sources: &[PersistedGuidanceSource],
    input: &ListSelectedContextGuidanceInput,
) -> bool {
    sources.iter().any(|source| {
        if let Some(agent) = input.agent.as_deref()
            && source.agent_type.as_deref() != Some(agent)
        {
            return false;
        }
        if let Some(since) = input.since.as_deref()
            && source
                .event_time
                .as_deref()
                .is_none_or(|event_time| event_time < since)
        {
            return false;
        }
        if let Some(evidence_kind) = input.evidence_kind.as_deref()
            && source.evidence_kind.as_deref() != Some(evidence_kind)
        {
            return false;
        }
        true
    })
}

fn category_to_storage(category: GuidanceFactCategory) -> &'static str {
    match category {
        GuidanceFactCategory::Decision => "DECISION",
        GuidanceFactCategory::Constraint => "CONSTRAINT",
        GuidanceFactCategory::Pattern => "PATTERN",
        GuidanceFactCategory::Risk => "RISK",
        GuidanceFactCategory::Verification => "VERIFICATION",
        GuidanceFactCategory::Context => "CONTEXT",
    }
}

fn category_from_storage(value: &str) -> Result<GuidanceFactCategory> {
    match value {
        "DECISION" => Ok(GuidanceFactCategory::Decision),
        "CONSTRAINT" => Ok(GuidanceFactCategory::Constraint),
        "PATTERN" => Ok(GuidanceFactCategory::Pattern),
        "RISK" => Ok(GuidanceFactCategory::Risk),
        "VERIFICATION" => Ok(GuidanceFactCategory::Verification),
        "CONTEXT" => Ok(GuidanceFactCategory::Context),
        other => Err(anyhow!("unknown context guidance category `{other}`")),
    }
}

fn confidence_to_storage(confidence: GuidanceFactConfidence) -> &'static str {
    match confidence {
        GuidanceFactConfidence::High => "HIGH",
        GuidanceFactConfidence::Medium => "MEDIUM",
        GuidanceFactConfidence::Low => "LOW",
    }
}

fn confidence_from_storage(value: &str) -> Result<GuidanceFactConfidence> {
    match value {
        "HIGH" => Ok(GuidanceFactConfidence::High),
        "MEDIUM" => Ok(GuidanceFactConfidence::Medium),
        "LOW" => Ok(GuidanceFactConfidence::Low),
        other => Err(anyhow!("unknown context guidance confidence `{other}`")),
    }
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
