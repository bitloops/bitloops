use std::collections::BTreeSet;

use anyhow::{Context, Result, anyhow};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::storage::SqliteConnectionPool;

use super::descriptor::{CONTEXT_GUIDANCE_CAPABILITY_ID, CONTEXT_GUIDANCE_DESCRIPTOR};
use super::distillation::{GuidanceDistillationInput, KnowledgeGuidanceDistillationInput};
pub use super::lifecycle::{ApplyTargetCompactionInput, ApplyTargetCompactionOutcome};
use super::storage_codec::category_to_storage;
use super::storage_helpers::{
    compare_guidance_facts, default_targets_for_knowledge_input, insert_compaction_member,
    insert_fact, insert_sources, insert_targets, knowledge_item_source, load_sources, load_targets,
    map_guidance_fact_row, matches_selected_targets, matches_source_filters,
    sources_for_history_fact, targets_for_fact_with_defaults,
};
pub use super::storage_helpers::{
    guidance_hash_for_parts, guidance_id, guidance_input_hash, guidance_run_id,
    knowledge_guidance_input_hash,
};
pub use super::storage_schema::context_guidance_sqlite_schema_sql;
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

    fn persist_knowledge_guidance_distillation(
        &self,
        repo_id: &str,
        input: &KnowledgeGuidanceDistillationInput,
        output: &GuidanceDistillationOutput,
        source_model: Option<&str>,
        source_profile: Option<&str>,
    ) -> Result<PersistGuidanceOutcome>;

    fn list_selected_context_guidance(
        &self,
        input: ListSelectedContextGuidanceInput,
    ) -> Result<Vec<PersistedGuidanceFact>>;

    fn list_active_guidance_for_target(
        &self,
        repo_id: &str,
        target_type: &str,
        target_value: &str,
        limit: usize,
    ) -> Result<Vec<PersistedGuidanceFact>>;

    fn apply_target_compaction(
        &self,
        repo_id: &str,
        input: ApplyTargetCompactionInput,
    ) -> Result<ApplyTargetCompactionOutcome>;

    fn list_target_summaries(
        &self,
        repo_id: &str,
        targets: &[PersistedGuidanceTarget],
    ) -> Result<Vec<PersistedGuidanceTargetSummary>>;

    fn health_check(&self, repo_id: &str) -> Result<()>;
}

pub struct PersistGuidanceOutcome {
    pub inserted_run: bool,
    pub inserted_facts: usize,
    pub unchanged: bool,
    pub touched_targets: Vec<PersistedGuidanceTarget>,
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
    pub lifecycle_status: String,
    pub fact_fingerprint: String,
    pub value_score: f64,
    pub superseded_by_guidance_id: Option<String>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct PersistedGuidanceTargetSummary {
    pub target_type: String,
    pub target_value: String,
    pub summary_json: Value,
    pub active_guidance_count: usize,
    pub generated_at: Option<String>,
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

struct PersistGuidanceDistillationRequest<'a> {
    source_scope_key: String,
    input_hash: String,
    output: &'a GuidanceDistillationOutput,
    source_model: Option<&'a str>,
    source_profile: Option<&'a str>,
    default_targets: &'a [PersistedGuidanceTarget],
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

    fn persist_guidance_distillation<F>(
        &self,
        repo_id: &str,
        request: PersistGuidanceDistillationRequest<'_>,
        mut sources_for_fact: F,
    ) -> Result<PersistGuidanceOutcome>
    where
        F: FnMut(&GuidanceFactDraft) -> Vec<PersistedGuidanceSource>,
    {
        let PersistGuidanceDistillationRequest {
            source_scope_key,
            input_hash,
            output,
            source_model,
            source_profile,
            default_targets,
        } = request;
        let run_id = guidance_run_id(repo_id, source_scope_key.as_str(), input_hash.as_str());
        let summary_json =
            serde_json::to_string(&output.summary).context("serializing guidance summary")?;

        self.sqlite.with_write_connection(|conn| {
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
                    touched_targets: Vec::new(),
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
                let mut touched_targets = Vec::new();
                let mut seen_targets = BTreeSet::new();
                for fact in &output.guidance_facts {
                    let targets = targets_for_fact_with_defaults(fact, default_targets);
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
                    insert_fact(
                        conn,
                        repo_id,
                        run_id.as_str(),
                        guidance_id.as_str(),
                        fact,
                        &targets,
                    )?;
                    insert_targets(conn, repo_id, guidance_id.as_str(), &targets)?;
                    let sources = super::quality::dedupe_and_cap_sources(sources_for_fact(fact));
                    insert_sources(conn, repo_id, guidance_id.as_str(), &sources)?;
                    for target in targets {
                        if seen_targets
                            .insert((target.target_type.clone(), target.target_value.clone()))
                        {
                            touched_targets.push(target);
                        }
                    }
                    inserted_facts += 1;
                }

                Ok(PersistGuidanceOutcome {
                    inserted_run: true,
                    inserted_facts,
                    unchanged: false,
                    touched_targets,
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
        self.persist_guidance_distillation(
            repo_id,
            PersistGuidanceDistillationRequest {
                source_scope_key,
                input_hash,
                output,
                source_model,
                source_profile,
                default_targets: &[],
            },
            |fact| sources_for_history_fact(input, fact),
        )
    }

    fn persist_knowledge_guidance_distillation(
        &self,
        repo_id: &str,
        input: &KnowledgeGuidanceDistillationInput,
        output: &GuidanceDistillationOutput,
        source_model: Option<&str>,
        source_profile: Option<&str>,
    ) -> Result<PersistGuidanceOutcome> {
        let source_scope_key = format!(
            "knowledge:{}:{}:{}",
            input.knowledge_item_id,
            input.knowledge_item_version_id,
            input.relation_assertion_id.as_deref().unwrap_or("")
        );
        let input_hash = knowledge_guidance_input_hash(input);
        let default_targets = default_targets_for_knowledge_input(input);
        self.persist_guidance_distillation(
            repo_id,
            PersistGuidanceDistillationRequest {
                source_scope_key,
                input_hash,
                output,
                source_model,
                source_profile,
                default_targets: &default_targets,
            },
            |_fact| vec![knowledge_item_source(input)],
        )
    }

    fn list_selected_context_guidance(
        &self,
        input: ListSelectedContextGuidanceInput,
    ) -> Result<Vec<PersistedGuidanceFact>> {
        self.sqlite.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT f.guidance_id, f.run_id, f.repo_id, f.active, f.category, f.kind,
                        f.guidance, f.evidence_excerpt, f.confidence,
                        f.lifecycle_status, f.fact_fingerprint, f.value_score,
                        f.superseded_by_guidance_id, NULLIF(r.source_model, ''), r.generated_at
                 FROM context_guidance_facts f
                 JOIN context_guidance_distillation_runs r ON r.run_id = f.run_id
                 WHERE f.repo_id = ?1 AND f.active = 1 AND f.lifecycle_status = 'active'",
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
                .query_map(
                    rusqlite::params_from_iter(params_values.iter()),
                    map_guidance_fact_row,
                )
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
            }
            out.sort_by(compare_guidance_facts);
            out.truncate(input.limit);
            Ok(out)
        })
    }

    fn list_active_guidance_for_target(
        &self,
        repo_id: &str,
        target_type: &str,
        target_value: &str,
        limit: usize,
    ) -> Result<Vec<PersistedGuidanceFact>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT f.guidance_id, f.run_id, f.repo_id, f.active, f.category, f.kind,
                            f.guidance, f.evidence_excerpt, f.confidence,
                            f.lifecycle_status, f.fact_fingerprint, f.value_score,
                            f.superseded_by_guidance_id, NULLIF(r.source_model, ''), r.generated_at
                     FROM context_guidance_facts f
                     JOIN context_guidance_distillation_runs r ON r.run_id = f.run_id
                     JOIN context_guidance_targets t ON t.guidance_id = f.guidance_id
                     WHERE f.repo_id = ?1
                       AND f.active = 1
                       AND f.lifecycle_status = 'active'
                       AND t.target_type = ?2
                       AND t.target_value = ?3
                     ORDER BY f.value_score DESC, r.generated_at DESC, f.guidance_id ASC
                     LIMIT ?4",
                )
                .context("preparing active context guidance target query")?;
            let rows = stmt
                .query_map(
                    params![
                        repo_id,
                        target_type,
                        target_value,
                        i64::try_from(limit).unwrap_or(i64::MAX)
                    ],
                    map_guidance_fact_row,
                )
                .context("querying active context guidance for target")?;
            let mut facts = Vec::new();
            for row in rows {
                let mut fact = row.map_err(anyhow::Error::from)?;
                fact.targets = load_targets(conn, fact.guidance_id.as_str())?;
                fact.sources = load_sources(conn, fact.guidance_id.as_str())?;
                facts.push(fact);
            }
            Ok(facts)
        })
    }

    fn apply_target_compaction(
        &self,
        repo_id: &str,
        input: ApplyTargetCompactionInput,
    ) -> Result<ApplyTargetCompactionOutcome> {
        self.sqlite.with_write_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")
                .context("starting context guidance compaction transaction")?;
            let result = (|| -> Result<ApplyTargetCompactionOutcome> {
                let compacted_count =
                    input.duplicate_guidance_ids.len() + input.superseded_guidance_ids.len();
                let source_fact_count = input.retained_guidance_ids.len() + compacted_count;
                conn.execute(
                    "INSERT INTO context_guidance_compaction_runs (
                        compaction_run_id, repo_id, target_type, target_value, source_fact_count,
                        retained_fact_count, compacted_fact_count, status, summary_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'completed', ?8)",
                    params![
                        input.compaction_run_id,
                        repo_id,
                        input.target_type,
                        input.target_value,
                        i64::try_from(source_fact_count)?,
                        i64::try_from(input.retained_guidance_ids.len())?,
                        i64::try_from(compacted_count)?,
                        input.summary_json
                    ],
                )
                .context("inserting context guidance compaction run")?;

                for guidance_id in &input.retained_guidance_ids {
                    insert_compaction_member(
                        conn,
                        input.compaction_run_id.as_str(),
                        guidance_id,
                        "retained",
                        "retained by target compaction",
                    )?;
                }
                for guidance_id in &input.duplicate_guidance_ids {
                    insert_compaction_member(
                        conn,
                        input.compaction_run_id.as_str(),
                        guidance_id,
                        "duplicate",
                        "duplicate fact fingerprint for target",
                    )?;
                    conn.execute(
                        "UPDATE context_guidance_facts
                         SET active = 0,
                             lifecycle_status = 'duplicate',
                             lifecycle_reason = ?1,
                             updated_at = datetime('now')
                         WHERE repo_id = ?2 AND guidance_id = ?3",
                        params![
                            "duplicate fact fingerprint for target",
                            repo_id,
                            guidance_id
                        ],
                    )
                    .context("marking duplicate context guidance fact")?;
                }
                for (guidance_id, superseded_by) in &input.superseded_guidance_ids {
                    insert_compaction_member(
                        conn,
                        input.compaction_run_id.as_str(),
                        guidance_id,
                        "superseded",
                        "superseded by target compaction",
                    )?;
                    conn.execute(
                        "UPDATE context_guidance_facts
                         SET active = 0,
                             lifecycle_status = 'superseded',
                             superseded_by_guidance_id = ?1,
                             lifecycle_reason = ?2,
                             updated_at = datetime('now')
                         WHERE repo_id = ?3 AND guidance_id = ?4",
                        params![
                            superseded_by,
                            "superseded by target compaction",
                            repo_id,
                            guidance_id
                        ],
                    )
                    .context("marking superseded context guidance fact")?;
                }

                conn.execute(
                    "INSERT INTO context_guidance_target_summaries (
                        target_summary_id, repo_id, target_type, target_value, summary_json,
                        active_guidance_count, latest_compaction_run_id
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(repo_id, target_type, target_value)
                     DO UPDATE SET
                        summary_json = excluded.summary_json,
                        active_guidance_count = excluded.active_guidance_count,
                        latest_compaction_run_id = excluded.latest_compaction_run_id,
                        updated_at = datetime('now')",
                    params![
                        format!(
                            "target-summary:{}:{}:{}",
                            repo_id, input.target_type, input.target_value
                        ),
                        repo_id,
                        input.target_type,
                        input.target_value,
                        input.summary_json,
                        i64::try_from(input.retained_guidance_ids.len())?,
                        input.compaction_run_id
                    ],
                )
                .context("upserting context guidance target summary")?;

                Ok(ApplyTargetCompactionOutcome {
                    retained_count: input.retained_guidance_ids.len(),
                    compacted_count,
                })
            })();

            match result {
                Ok(outcome) => {
                    conn.execute_batch("COMMIT")
                        .context("committing context guidance compaction transaction")?;
                    Ok(outcome)
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(err)
                }
            }
        })
    }

    fn list_target_summaries(
        &self,
        repo_id: &str,
        targets: &[PersistedGuidanceTarget],
    ) -> Result<Vec<PersistedGuidanceTargetSummary>> {
        self.sqlite.with_connection(|conn| {
            let mut summaries = Vec::new();
            let mut stmt = conn
                .prepare(
                    "SELECT target_type, target_value, summary_json, active_guidance_count, generated_at
                     FROM context_guidance_target_summaries
                     WHERE repo_id = ?1 AND target_type = ?2 AND target_value = ?3
                     LIMIT 1",
                )
                .context("preparing context guidance target summary query")?;
            for target in targets {
                let summary = stmt
                    .query_row(
                        params![repo_id, target.target_type, target.target_value],
                        |row| {
                            let summary_json: String = row.get(2)?;
                            let active_guidance_count: i64 = row.get(3)?;
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                summary_json,
                                active_guidance_count,
                                row.get::<_, Option<String>>(4)?,
                            ))
                        },
                    )
                    .optional()
                    .context("querying context guidance target summary")?;
                if let Some((
                    target_type,
                    target_value,
                    summary_json,
                    active_guidance_count,
                    generated_at,
                )) = summary
                {
                    summaries.push(PersistedGuidanceTargetSummary {
                        target_type,
                        target_value,
                        summary_json: serde_json::from_str(&summary_json)
                            .context("parsing context guidance target summary JSON")?,
                        active_guidance_count: usize::try_from(active_guidance_count)?,
                        generated_at,
                    });
                }
            }
            Ok(summaries)
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
#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
