use super::DevqlGraphqlContext;
use crate::graphql::ResolverScope;
use crate::graphql::types::artefact_selection::{
    HistoricalContextItem, HistoricalEvidenceKind, HistoricalMatchReason, HistoricalMatchStrength,
    HistoricalToolEvent, captured_preview,
};
use crate::graphql::types::{CheckpointFileRelation, DateTimeScalar};
use crate::host::devql::checkpoint_provenance::{
    CheckpointFileActivityFilter, CheckpointFileGateway, CheckpointSelectionEvidenceKind,
    CheckpointSelectionMatch, CheckpointSelectionMatchStrength,
};
use crate::host::interactions::query::{self, InteractionBrowseFilter, InteractionTurnSummary};
use crate::host::relational_store::DefaultRelationalStore;
use anyhow::{Context, Result};
use async_graphql::ID;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tokio::task;

const HISTORICAL_CONTEXT_PREVIEW_CHARS: usize = 500;

#[derive(Debug, Clone)]
pub(crate) struct HistoricalContextSelectionInput {
    pub(crate) symbol_ids: Vec<String>,
    pub(crate) paths: Vec<String>,
    pub(crate) agent: Option<String>,
    pub(crate) since: Option<String>,
    pub(crate) evidence_kind: Option<HistoricalEvidenceKind>,
}

impl DevqlGraphqlContext {
    pub(crate) async fn list_selected_historical_context(
        &self,
        scope: &ResolverScope,
        input: HistoricalContextSelectionInput,
    ) -> Result<Vec<HistoricalContextItem>> {
        if input.symbol_ids.is_empty() && input.paths.is_empty() {
            return Ok(Vec::new());
        }

        let repo_id = self.repo_id_for_scope(scope)?;
        let repo_root = self.repo_root_for_scope(scope)?;
        let relational_store = DefaultRelationalStore::open_local_for_repo_root(&repo_root)?;
        let sqlite_path = relational_store.sqlite_path().to_path_buf();
        if !sqlite_path.is_file() {
            return Ok(Vec::new());
        }
        relational_store
            .initialise_local_relational_checkpoint_schema()
            .context("initialising relational checkpoint schema for historical context")?;

        let relational = relational_store.to_local_inner();
        let matches = CheckpointFileGateway::new(&relational)
            .list_checkpoint_selection_matches(
                &repo_id,
                &input.symbol_ids,
                &input.paths,
                CheckpointFileActivityFilter {
                    agent: input.agent.as_deref(),
                    since: input.since.as_deref(),
                },
            )
            .await?;
        let matches = filter_matches_by_evidence(matches, input.evidence_kind);
        if matches.is_empty() {
            return Ok(Vec::new());
        }

        hydrate_historical_context_items(
            repo_root,
            input.paths,
            matches,
            InteractionBrowseFilter {
                since: input.since,
                agent: input.agent,
                ..InteractionBrowseFilter::default()
            },
        )
        .await
    }
}

fn filter_matches_by_evidence(
    matches: Vec<CheckpointSelectionMatch>,
    evidence_kind: Option<HistoricalEvidenceKind>,
) -> Vec<CheckpointSelectionMatch> {
    let Some(evidence_kind) = evidence_kind else {
        return matches;
    };
    matches
        .into_iter()
        .filter(|row| {
            row.evidence_kinds
                .iter()
                .copied()
                .any(|kind| historical_evidence_kind(kind) == evidence_kind)
        })
        .map(|mut row| {
            row.evidence_kind = checkpoint_selection_evidence_kind(evidence_kind);
            row.match_strength =
                CheckpointSelectionMatchStrength::from_evidence_kind(row.evidence_kind);
            row
        })
        .collect()
}

async fn hydrate_historical_context_items(
    repo_root: PathBuf,
    selected_paths: Vec<String>,
    matches: Vec<CheckpointSelectionMatch>,
    interaction_filter: InteractionBrowseFilter,
) -> Result<Vec<HistoricalContextItem>> {
    task::spawn_blocking(move || -> Result<Vec<HistoricalContextItem>> {
        let turns = query::list_turn_summaries(&repo_root, &interaction_filter)?;
        let sessions = query::list_session_summaries(&repo_root, &interaction_filter)?;
        let sessions_by_id = sessions
            .into_iter()
            .map(|summary| (summary.session.session_id.clone(), summary))
            .collect::<BTreeMap<_, _>>();

        let mut out = Vec::new();
        for matched in matches {
            let Some(info) =
                crate::host::checkpoints::strategy::manual_commit::read_committed_info(
                    repo_root.as_path(),
                    &matched.checkpoint_id,
                )?
            else {
                continue;
            };
            let event_time = parse_event_time(&matched.event_time);
            let selected_turn = choose_best_turn_for_checkpoint(
                &matched.checkpoint_id,
                &info.session_id,
                &selected_paths,
                &turns,
                &event_time,
            );
            let session = sessions_by_id.get(&info.session_id);
            let file_relations = load_checkpoint_file_relations_blocking(
                repo_root.as_path(),
                &matched.checkpoint_id,
            )?;

            out.push(build_context_item(
                matched,
                info,
                session,
                selected_turn.as_ref(),
                file_relations,
            ));
        }
        out.sort_by(historical_context_sort);
        Ok(out)
    })
    .await
    .context("joining historical context hydration task")?
}

fn load_checkpoint_file_relations_blocking(
    repo_root: &Path,
    checkpoint_id: &str,
) -> Result<Vec<CheckpointFileRelation>> {
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)?.repo_id;
    let relational_store = DefaultRelationalStore::open_local_for_repo_root(repo_root)?;
    if !relational_store.sqlite_path().is_file() {
        return Ok(Vec::new());
    }
    let relational = relational_store.to_local_inner();
    let rows = if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.block_on(async {
            CheckpointFileGateway::new(&relational)
                .list_checkpoint_files(&repo_id, checkpoint_id)
                .await
        })?
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime for checkpoint file relation loading")
            .block_on(async {
                CheckpointFileGateway::new(&relational)
                    .list_checkpoint_files(&repo_id, checkpoint_id)
                    .await
            })?
    };
    Ok(rows
        .into_iter()
        .map(|row| CheckpointFileRelation {
            filepath: crate::host::devql::checkpoint_provenance::checkpoint_display_path(
                row.path_before.as_deref(),
                row.path_after.as_deref(),
            ),
            change_kind: row.change_kind.as_str().to_string(),
            path_before: row.path_before,
            path_after: row.path_after,
            blob_sha_before: row.blob_sha_before,
            blob_sha_after: row.blob_sha_after,
            copied_from_path: row.copy_source_path,
            copied_from_blob_sha: row.copy_source_blob_sha,
        })
        .collect())
}

fn build_context_item(
    matched: CheckpointSelectionMatch,
    checkpoint: crate::host::checkpoints::strategy::manual_commit::CommittedInfo,
    session: Option<&crate::host::interactions::query::InteractionSessionSummary>,
    turn: Option<&InteractionTurnSummary>,
    file_relations: Vec<CheckpointFileRelation>,
) -> HistoricalContextItem {
    let prompt_preview = turn
        .and_then(|summary| {
            captured_preview(&summary.turn.prompt, HISTORICAL_CONTEXT_PREVIEW_CHARS)
        })
        .or_else(|| {
            session.and_then(|summary| {
                captured_preview(
                    &summary.session.first_prompt,
                    HISTORICAL_CONTEXT_PREVIEW_CHARS,
                )
            })
        })
        .or_else(|| {
            captured_preview(
                &checkpoint.first_prompt_preview,
                HISTORICAL_CONTEXT_PREVIEW_CHARS,
            )
        });

    HistoricalContextItem {
        checkpoint_id: ID::from(matched.checkpoint_id.clone()),
        session_id: checkpoint.session_id.clone(),
        turn_id: turn.map(|summary| summary.turn.turn_id.clone()),
        agent_type: turn
            .map(|summary| summary.turn.agent_type.clone())
            .filter(|value| !value.trim().is_empty())
            .or_else(|| session.map(|summary| summary.session.agent_type.clone()))
            .filter(|value| !value.trim().is_empty())
            .or_else(|| (!checkpoint.agent.trim().is_empty()).then(|| checkpoint.agent.clone())),
        model: turn
            .map(|summary| summary.turn.model.clone())
            .filter(|value| !value.trim().is_empty())
            .or_else(|| session.map(|summary| summary.session.model.clone()))
            .filter(|value| !value.trim().is_empty()),
        event_time: parse_event_time(&matched.event_time),
        match_reason: historical_match_reason(matched.evidence_kind),
        match_strength: historical_match_strength(matched.match_strength),
        prompt_preview,
        turn_summary: turn.and_then(|summary| {
            captured_preview(&summary.turn.summary, HISTORICAL_CONTEXT_PREVIEW_CHARS)
        }),
        transcript_preview: turn.and_then(|summary| {
            captured_preview(
                &summary.turn.transcript_fragment,
                HISTORICAL_CONTEXT_PREVIEW_CHARS,
            )
        }),
        files_modified: turn
            .map(|summary| summary.turn.files_modified.clone())
            .filter(|paths| !paths.is_empty())
            .unwrap_or_else(|| checkpoint.files_touched.clone()),
        file_relations,
        tool_events: turn.map(historical_tool_events).unwrap_or_default(),
        evidence_kinds: matched
            .evidence_kinds
            .iter()
            .copied()
            .map(historical_match_reason)
            .collect(),
    }
}

fn checkpoint_selection_evidence_kind(
    kind: HistoricalEvidenceKind,
) -> CheckpointSelectionEvidenceKind {
    match kind {
        HistoricalEvidenceKind::SymbolProvenance => {
            CheckpointSelectionEvidenceKind::SymbolProvenance
        }
        HistoricalEvidenceKind::FileRelation => CheckpointSelectionEvidenceKind::FileRelation,
        HistoricalEvidenceKind::LineOverlap => CheckpointSelectionEvidenceKind::LineOverlap,
    }
}

fn historical_evidence_kind(kind: CheckpointSelectionEvidenceKind) -> HistoricalEvidenceKind {
    match kind {
        CheckpointSelectionEvidenceKind::SymbolProvenance => {
            HistoricalEvidenceKind::SymbolProvenance
        }
        CheckpointSelectionEvidenceKind::FileRelation => HistoricalEvidenceKind::FileRelation,
        CheckpointSelectionEvidenceKind::LineOverlap => HistoricalEvidenceKind::LineOverlap,
    }
}

fn historical_match_reason(kind: CheckpointSelectionEvidenceKind) -> HistoricalMatchReason {
    match kind {
        CheckpointSelectionEvidenceKind::SymbolProvenance => {
            HistoricalMatchReason::SymbolProvenance
        }
        CheckpointSelectionEvidenceKind::FileRelation => HistoricalMatchReason::FileRelation,
        CheckpointSelectionEvidenceKind::LineOverlap => HistoricalMatchReason::LineOverlap,
    }
}

fn historical_match_strength(
    strength: CheckpointSelectionMatchStrength,
) -> HistoricalMatchStrength {
    match strength {
        CheckpointSelectionMatchStrength::High => HistoricalMatchStrength::High,
        CheckpointSelectionMatchStrength::Medium => HistoricalMatchStrength::Medium,
        CheckpointSelectionMatchStrength::Low => HistoricalMatchStrength::Low,
    }
}

fn historical_tool_events(summary: &InteractionTurnSummary) -> Vec<HistoricalToolEvent> {
    summary
        .tool_uses
        .iter()
        .map(|tool| HistoricalToolEvent {
            tool_kind: non_empty(&tool.tool_name),
            input_summary: non_empty(&tool.input_summary),
            output_summary: non_empty(&tool.output_summary),
            command: non_empty(&tool.command),
        })
        .collect()
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn choose_best_turn_for_checkpoint(
    checkpoint_id: &str,
    session_id: &str,
    selected_paths: &[String],
    turns: &[InteractionTurnSummary],
    event_time: &DateTimeScalar,
) -> Option<InteractionTurnSummary> {
    turns
        .iter()
        .find(|turn| turn.turn.checkpoint_id.as_deref() == Some(checkpoint_id))
        .cloned()
        .or_else(|| {
            turns
                .iter()
                .filter(|turn| {
                    turn.turn.session_id == session_id
                        && paths_overlap(&turn.turn.files_modified, selected_paths)
                })
                .min_by_key(|turn| {
                    (
                        turn_distance_from_event_millis(turn, event_time),
                        turn.turn.turn_id.as_str(),
                    )
                })
                .cloned()
        })
}

fn paths_overlap(left: &[String], right: &[String]) -> bool {
    let right = right
        .iter()
        .map(|path| normalize_path(path))
        .collect::<BTreeSet<_>>();
    left.iter()
        .map(|path| normalize_path(path))
        .any(|path| right.contains(&path))
}

fn normalize_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn historical_context_sort(
    left: &HistoricalContextItem,
    right: &HistoricalContextItem,
) -> std::cmp::Ordering {
    right
        .event_time
        .cmp(&left.event_time)
        .then_with(|| {
            match_strength_rank(right.match_strength).cmp(&match_strength_rank(left.match_strength))
        })
        .then_with(|| {
            left.checkpoint_id
                .as_ref()
                .cmp(right.checkpoint_id.as_ref())
        })
        .then_with(|| left.turn_id.cmp(&right.turn_id))
}

fn turn_distance_from_event_millis(
    turn: &InteractionTurnSummary,
    event_time: &DateTimeScalar,
) -> u64 {
    let turn_timestamp = turn
        .turn
        .ended_at
        .as_deref()
        .unwrap_or(turn.turn.started_at.as_str());
    let Ok(turn_time) = DateTimeScalar::parse_rfc3339(turn_timestamp) else {
        return u64::MAX;
    };
    let Ok(event_time) = DateTimeScalar::parse_rfc3339(event_time.as_str()) else {
        return u64::MAX;
    };
    turn_time
        .timestamp_millis()
        .abs_diff(event_time.timestamp_millis())
}

fn match_strength_rank(strength: HistoricalMatchStrength) -> u8 {
    match strength {
        HistoricalMatchStrength::High => 3,
        HistoricalMatchStrength::Medium => 2,
        HistoricalMatchStrength::Low => 1,
    }
}

fn parse_event_time(value: &str) -> DateTimeScalar {
    DateTimeScalar::from_rfc3339(value.to_string())
        .or_else(|_| DateTimeScalar::from_rfc3339("1970-01-01T00:00:00+00:00"))
        .expect("static epoch timestamp must parse")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_turn_prefers_checkpoint_link_then_path_overlap() {
        let event_time =
            DateTimeScalar::from_rfc3339("2026-04-28T12:30:13+00:00").expect("timestamp parses");
        let direct = test_turn_summary(
            "turn-direct",
            "session-1",
            Some("checkpoint-1"),
            &["src/other.rs"],
        );
        let path = test_turn_summary("turn-path", "session-1", None, &["src/lib.rs"]);
        let unrelated = test_turn_summary("turn-unrelated", "session-1", None, &["src/other.rs"]);

        let chosen = choose_best_turn_for_checkpoint(
            "checkpoint-1",
            "session-1",
            &["src/lib.rs".to_string()],
            &[path.clone(), direct.clone(), unrelated.clone()],
            &event_time,
        );

        assert_eq!(chosen.expect("turn selected").turn.turn_id, "turn-direct");

        let path_chosen = choose_best_turn_for_checkpoint(
            "checkpoint-2",
            "session-1",
            &["src/lib.rs".to_string()],
            &[unrelated.clone(), path],
            &event_time,
        );

        assert_eq!(
            path_chosen
                .expect("path-overlap turn selected")
                .turn
                .turn_id,
            "turn-path"
        );

        let missing = choose_best_turn_for_checkpoint(
            "checkpoint-2",
            "session-1",
            &["src/lib.rs".to_string()],
            &[unrelated],
            &event_time,
        );

        assert!(missing.is_none());
    }

    fn test_turn_summary(
        turn_id: &str,
        session_id: &str,
        checkpoint_id: Option<&str>,
        files: &[&str],
    ) -> crate::host::interactions::query::InteractionTurnSummary {
        crate::host::interactions::query::InteractionTurnSummary {
            turn: crate::host::interactions::types::InteractionTurn {
                turn_id: turn_id.to_string(),
                repo_id: "repo-1".to_string(),
                session_id: session_id.to_string(),
                branch: "main".to_string(),
                actor_id: String::new(),
                actor_name: String::new(),
                actor_email: String::new(),
                actor_source: String::new(),
                turn_number: 1,
                prompt: "captured prompt".to_string(),
                agent_type: "codex".to_string(),
                model: "gpt-5.4".to_string(),
                started_at: "2026-04-28T12:29:00+00:00".to_string(),
                ended_at: Some("2026-04-28T12:30:00+00:00".to_string()),
                token_usage: None,
                summary: "captured summary".to_string(),
                prompt_count: 1,
                transcript_offset_start: Some(0),
                transcript_offset_end: Some(20),
                transcript_fragment: "captured transcript".to_string(),
                files_modified: files.iter().map(|path| (*path).to_string()).collect(),
                checkpoint_id: checkpoint_id.map(str::to_string),
                updated_at: "2026-04-28T12:30:00+00:00".to_string(),
            },
            tool_uses: Vec::new(),
            subagent_runs: Vec::new(),
            linked_checkpoints: Vec::new(),
            latest_commit_author: None,
        }
    }
}
