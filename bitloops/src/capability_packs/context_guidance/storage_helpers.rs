use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::params;

use super::distillation::{
    GuidanceDistillationInput, GuidanceToolEvidence, KnowledgeGuidanceDistillationInput,
};
use super::storage::{
    ListSelectedContextGuidanceInput, PersistedGuidanceFact, PersistedGuidanceSource,
    PersistedGuidanceTarget,
};
use super::storage_codec::{
    category_from_storage, category_to_storage, confidence_from_storage, confidence_to_storage,
    sha256_hex,
};
use super::types::{GuidanceFactCategory, GuidanceFactDraft};

pub(super) fn compare_guidance_facts(
    left: &PersistedGuidanceFact,
    right: &PersistedGuidanceFact,
) -> std::cmp::Ordering {
    let left_score = persisted_guidance_value_score(left);
    let right_score = persisted_guidance_value_score(right);
    right_score
        .total_cmp(&left_score)
        .then_with(|| right.generated_at.cmp(&left.generated_at))
        .then_with(|| left.guidance_id.cmp(&right.guidance_id))
}

fn persisted_guidance_value_score(fact: &PersistedGuidanceFact) -> f64 {
    if fact.value_score > 0.0 {
        return fact.value_score;
    }
    let has_symbol_target = fact
        .targets
        .iter()
        .any(|target| target.target_type == "symbol_id" || target.target_type == "symbol_fqn");
    super::quality::guidance_value_score(fact.category, fact.confidence, has_symbol_target)
}

pub(super) fn map_guidance_fact_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PersistedGuidanceFact> {
    let category_text: String = row.get(4)?;
    let confidence_text: String = row.get(8)?;
    let lifecycle_status: String = row.get(9)?;
    if !super::lifecycle::is_known_lifecycle_status(lifecycle_status.as_str()) {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            9,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown context guidance lifecycle status {lifecycle_status}"),
            )),
        ));
    }
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
        confidence: confidence_from_storage(confidence_text.as_str()).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    err.to_string(),
                )),
            )
        })?,
        lifecycle_status,
        fact_fingerprint: row.get(10)?,
        value_score: row.get(11)?,
        superseded_by_guidance_id: row.get(12)?,
        source_model: row.get(13)?,
        generated_at: row.get(14)?,
        targets: Vec::new(),
        sources: Vec::new(),
    })
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

pub fn knowledge_guidance_input_hash(input: &KnowledgeGuidanceDistillationInput) -> String {
    let mut parts = vec![
        input.knowledge_item_id.clone(),
        input.knowledge_item_version_id.clone(),
        input
            .relation_assertion_id
            .as_deref()
            .unwrap_or("")
            .to_string(),
        input.provider.clone(),
        input.source_kind.clone(),
        input.title.as_deref().unwrap_or("").to_string(),
        input.url.as_deref().unwrap_or("").to_string(),
        input.updated_at.as_deref().unwrap_or("").to_string(),
        input.body_preview.as_deref().unwrap_or("").to_string(),
        input.normalized_fields_json.clone(),
    ];
    parts.extend(
        input
            .target_paths
            .iter()
            .map(|path| path.trim().to_string()),
    );
    parts.extend(
        input
            .target_symbols
            .iter()
            .map(|symbol| symbol.trim().to_string()),
    );
    sha256_hex(parts.join("\n").as_bytes())
}

pub fn guidance_hash_for_parts(parts: &[&str]) -> String {
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

pub(super) fn targets_for_fact_with_defaults(
    fact: &GuidanceFactDraft,
    default_targets: &[PersistedGuidanceTarget],
) -> Vec<PersistedGuidanceTarget> {
    let targets = targets_for_fact(fact);
    if targets.is_empty() {
        default_targets.to_vec()
    } else {
        targets
    }
}

pub(super) fn default_targets_for_knowledge_input(
    input: &KnowledgeGuidanceDistillationInput,
) -> Vec<PersistedGuidanceTarget> {
    let mut targets = Vec::new();
    targets.extend(
        input
            .target_paths
            .iter()
            .filter_map(|path| non_empty_target("path", path)),
    );
    targets.extend(
        input
            .target_symbols
            .iter()
            .filter_map(|symbol| non_empty_target("symbol_fqn", symbol)),
    );
    targets
}

fn non_empty_target(target_type: &str, value: &str) -> Option<PersistedGuidanceTarget> {
    let value = value.trim();
    (!value.is_empty()).then(|| PersistedGuidanceTarget {
        target_type: target_type.to_string(),
        target_value: value.to_string(),
    })
}

pub(super) fn insert_fact(
    conn: &rusqlite::Connection,
    repo_id: &str,
    run_id: &str,
    guidance_id: &str,
    fact: &GuidanceFactDraft,
    targets: &[PersistedGuidanceTarget],
) -> Result<()> {
    let value_score = super::quality::guidance_value_score(
        fact.category,
        fact.confidence,
        targets
            .iter()
            .any(|target| target.target_type == "symbol_fqn"),
    );
    let fact_fingerprint = super::lifecycle::fact_fingerprint(fact, targets);
    conn.execute(
        "INSERT INTO context_guidance_facts (
            guidance_id, run_id, repo_id, active, category, kind, guidance,
            evidence_excerpt, confidence, lifecycle_status, fact_fingerprint,
            value_score, superseded_by_guidance_id, lifecycle_reason
         ) VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            guidance_id,
            run_id,
            repo_id,
            category_to_storage(fact.category),
            fact.kind,
            fact.guidance,
            fact.evidence_excerpt,
            confidence_to_storage(fact.confidence),
            super::lifecycle::GuidanceLifecycleStatus::Active.as_storage(),
            fact_fingerprint,
            value_score,
            Option::<String>::None,
            ""
        ],
    )
    .context("inserting context guidance fact")?;
    Ok(())
}

pub(super) fn insert_targets(
    conn: &rusqlite::Connection,
    repo_id: &str,
    guidance_id: &str,
    targets: &[PersistedGuidanceTarget],
) -> Result<()> {
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
    Ok(())
}

pub(super) fn insert_sources(
    conn: &rusqlite::Connection,
    repo_id: &str,
    guidance_id: &str,
    sources: &[PersistedGuidanceSource],
) -> Result<()> {
    conn.execute(
        "DELETE FROM context_guidance_sources WHERE guidance_id = ?1",
        params![guidance_id],
    )
    .context("clearing context guidance sources")?;
    for (index, source) in sources.iter().enumerate() {
        insert_source(conn, repo_id, guidance_id, index, source)?;
    }
    Ok(())
}

pub(super) fn insert_compaction_member(
    conn: &rusqlite::Connection,
    compaction_run_id: &str,
    guidance_id: &str,
    action: &str,
    reason: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO context_guidance_compaction_members (
            compaction_member_id, compaction_run_id, guidance_id, action, reason
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            format!("{compaction_run_id}:{action}:{guidance_id}"),
            compaction_run_id,
            guidance_id,
            action,
            reason
        ],
    )
    .context("inserting context guidance compaction member")?;
    Ok(())
}

pub(super) fn sources_for_history_fact(
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
                .filter(|(_, event)| tool_event_supports_fact(event, fact))
                .map(|(index, event)| history_tool_source(input, index, event, fact)),
        );
    }
    super::quality::dedupe_and_cap_sources(sources)
}

pub(super) fn knowledge_item_source(
    input: &KnowledgeGuidanceDistillationInput,
) -> PersistedGuidanceSource {
    PersistedGuidanceSource {
        source_type: "knowledge.item_version".to_string(),
        source_id: format!(
            "{}:{}:{}",
            input.knowledge_item_id,
            input.knowledge_item_version_id,
            input.relation_assertion_id.as_deref().unwrap_or("")
        ),
        checkpoint_id: None,
        session_id: None,
        turn_id: None,
        tool_invocation_id: None,
        tool_kind: None,
        event_time: input.updated_at.clone(),
        agent_type: None,
        model: None,
        evidence_kind: Some("KNOWLEDGE_RELATION".to_string()),
        match_strength: Some("HIGH".to_string()),
        knowledge_item_id: Some(input.knowledge_item_id.clone()),
        knowledge_item_version_id: Some(input.knowledge_item_version_id.clone()),
        relation_assertion_id: input.relation_assertion_id.clone(),
        provider: Some(input.provider.clone()),
        source_kind: Some(input.source_kind.clone()),
        title: input.title.clone(),
        url: input.url.clone(),
        excerpt: input.body_preview.clone(),
    }
}

fn tool_event_supports_fact(event: &GuidanceToolEvidence, fact: &GuidanceFactDraft) -> bool {
    let event_text = [
        event.command.as_deref().unwrap_or(""),
        event.input_summary.as_deref().unwrap_or(""),
        event.output_summary.as_deref().unwrap_or(""),
    ]
    .join("\n")
    .to_ascii_lowercase();
    if event_text.trim().is_empty() {
        return false;
    }
    let fact_text = [
        fact.kind.as_str(),
        fact.guidance.as_str(),
        fact.evidence_excerpt.as_str(),
    ]
    .join("\n")
    .to_ascii_lowercase();
    if command_phrase_supports_fact(event_text.as_str(), fact_text.as_str()) {
        return true;
    }
    let event_terms = support_tokens(event_text.as_str());
    let overlap_count = fact_support_terms(fact)
        .into_iter()
        .filter(|term| event_terms.contains(term.as_str()))
        .count();
    overlap_count >= 2
}

fn fact_support_terms(fact: &GuidanceFactDraft) -> Vec<String> {
    let mut terms = Vec::new();
    for value in [
        fact.kind.as_str(),
        fact.guidance.as_str(),
        fact.evidence_excerpt.as_str(),
    ] {
        terms.extend(support_tokens(value));
    }
    terms.sort();
    terms.dedup();
    terms
}

fn support_tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|word| is_specific_support_term(word))
        .map(str::to_ascii_lowercase)
        .collect()
}

fn is_specific_support_term(word: &str) -> bool {
    let word = word.to_ascii_lowercase();
    word.len() >= 5 && !is_generic_support_term(word.as_str())
}

fn is_generic_support_term(word: &str) -> bool {
    matches!(
        word,
        "because"
            | "behavior"
            | "cases"
            | "check"
            | "checks"
            | "clippy"
            | "command"
            | "future"
            | "macro"
            | "macros"
            | "miri"
            | "nextest"
            | "passed"
            | "receiver"
            | "regress"
            | "regression"
            | "reusable"
            | "rustfmt"
            | "session"
            | "should"
            | "tests"
    )
}

fn command_phrase_supports_fact(event_text: &str, fact_text: &str) -> bool {
    [
        "cargo build",
        "cargo check",
        "cargo clippy",
        "cargo fmt",
        "cargo miri",
        "cargo nextest",
        "cargo test",
        "clippy",
        "rustfmt",
    ]
    .iter()
    .any(|phrase| event_text.contains(phrase) && fact_text.contains(phrase))
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
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
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

pub(super) fn load_targets(
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

pub(super) fn load_sources(
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

pub(super) fn matches_selected_targets(
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

pub(super) fn matches_source_filters(
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
