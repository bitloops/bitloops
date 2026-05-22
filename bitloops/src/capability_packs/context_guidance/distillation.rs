use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};

use crate::host::inference::TextGenerationService;

use super::evidence::{
    GuidanceEvidenceInput, GuidanceEvidenceSource, GuidanceEvidenceToolEvent, evidence_input_body,
    evidence_input_title, evidence_target_symbols, knowledge_source_label,
};
use super::types::{
    GuidanceDistillationOutput, GuidanceFactCategory, trim_guidance_distillation_output,
};

pub struct GuidanceDistillationInput {
    pub checkpoint_id: Option<String>,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub event_time: Option<String>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub transcript_fragment: Option<String>,
    pub files_modified: Vec<String>,
    pub tool_events: Vec<GuidanceToolEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceToolEvidence {
    pub event_type: Option<String>,
    pub tool_kind: Option<String>,
    pub input_summary: Option<String>,
    pub output_summary: Option<String>,
    pub command: Option<String>,
    pub file_path: Option<String>,
    pub evidence_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistilledGuidance {
    pub output: GuidanceDistillationOutput,
    pub report: GuidanceDistillationReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuidanceDistillationReport {
    pub raw_response_chars: usize,
    pub raw_fact_count: usize,
    pub validation_input_fact_count: usize,
    pub validation_kept_fact_count: usize,
    pub validation_discards: Vec<GuidanceValidationDiscard>,
    pub quality_input_fact_count: usize,
    pub quality_kept_fact_count: usize,
    pub quality_discards: Vec<GuidanceQualityDiscardSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceValidationDiscard {
    pub kind: String,
    pub reason: GuidanceValidationDiscardReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidanceValidationDiscardReason {
    MissingTargetWithoutDefault,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceQualityDiscardSummary {
    pub kind: String,
    pub category: GuidanceFactCategory,
    pub reason: String,
}

pub struct KnowledgeGuidanceDistillationInput {
    pub knowledge_item_id: String,
    pub knowledge_item_version_id: String,
    pub relation_assertion_id: Option<String>,
    pub provider: String,
    pub source_kind: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub updated_at: Option<String>,
    pub body_preview: Option<String>,
    pub normalized_fields_json: String,
    pub target_paths: Vec<String>,
    pub target_symbols: Vec<String>,
}

const MAX_PROMPT_CHARS: usize = 1_000;
const MAX_TRANSCRIPT_CHARS: usize = 6_000;
const MAX_KNOWLEDGE_BODY_CHARS: usize = 6_000;
const MAX_NORMALIZED_FIELDS_CHARS: usize = 2_000;
const MAX_FILES_MODIFIED: usize = 50;
const MAX_FILE_PATH_CHARS: usize = 300;
const MAX_TOOL_EVENTS: usize = 6;
const MAX_TOOL_COMMAND_CHARS: usize = 200;
const MAX_TOOL_INPUT_CHARS: usize = 300;
const MAX_TOOL_OUTPUT_CHARS: usize = 600;
const GUIDANCE_OUTPUT_SCHEMA_INSTRUCTION: &str = r#"Return only one JSON object in this exact shape:
{"summary":{"intent":"string","outcome":"string","decisions":["string"],"rejectedApproaches":["string"],"patterns":["string"],"verification":["string"],"openItems":["string"]},"guidanceFacts":[{"category":"DECISION|CONSTRAINT|PATTERN|RISK|VERIFICATION|CONTEXT","kind":"short_snake_case","guidance":"string","evidenceExcerpt":"string","appliesTo":{"paths":["string"],"symbols":["string"]},"confidence":"HIGH|MEDIUM|LOW"}]}
The summary.intent and summary.outcome fields are required strings.
The guidanceFacts field must be a flat array of fact objects.
Do not nest facts under category keys such as decision, verification, or doNotRepeat.
Do not return summary.context or guidanceFacts.decision objects.
Only emit guidance that will help a future coding session. Preserve durable decisions, constraints, risks, reusable patterns, and specific verification requirements. If no durable guidance is supported by evidence, return "guidanceFacts": [].
Durable decisions in summary.decisions must also be emitted as guidanceFacts with category DECISION or CONSTRAINT.
Every guidanceFact must include appliesTo.paths or appliesTo.symbols; targetless facts are discarded.
For multi-file refactors, target the production files that own the decision. Use tests as evidence unless the guidance is specifically about test contracts.
Do not emit status updates, completed-work summaries, code-size metrics, line-count reductions, generic "tests passed" facts, generic "ensure quality" advice, or "the agent edited this file" context.
VERIFICATION facts must name a reusable command/check and explain why a future session should run it.
CONTEXT facts must describe a durable codebase boundary, invariant, dependency, or ownership fact."#;

pub fn build_guidance_distillation_prompt(input: &GuidanceDistillationInput) -> String {
    let evidence = history_evidence_input(input);
    let (checkpoint_id, session_id, turn_id, event_time, agent_type, model) = match &evidence.source
    {
        GuidanceEvidenceSource::History {
            checkpoint_id,
            session_id,
            turn_id,
            event_time,
            agent_type,
            model,
        } => (
            checkpoint_id.as_deref().unwrap_or(""),
            session_id.as_str(),
            turn_id.as_deref().unwrap_or(""),
            event_time.as_deref().unwrap_or(""),
            agent_type.as_deref().unwrap_or(""),
            model.as_deref().unwrap_or(""),
        ),
        GuidanceEvidenceSource::Knowledge { .. } => {
            unreachable!("history prompt uses history source")
        }
    };
    let files_modified = bounded_files_modified(&evidence.target_paths);
    let tool_events = render_high_value_tool_events(&evidence.tool_events);
    let title = evidence_input_title(&evidence);
    let body = evidence_input_body(&evidence);
    let explicit_symbols = bounded_files_modified(evidence_target_symbols(&evidence));
    let knowledge_label = knowledge_source_label(&evidence).unwrap_or_default();
    let prompt = bounded_text(evidence.prompt.as_deref().unwrap_or(""), MAX_PROMPT_CHARS);
    let transcript =
        bounded_history_transcript(evidence.transcript_fragment.as_deref().unwrap_or(""));
    let transcript = if body.is_empty() {
        transcript
    } else if transcript.is_empty() {
        body
    } else {
        format!("{transcript}\n\nbody:\n{body}")
    };
    let title_block = if title.is_empty() {
        String::new()
    } else {
        format!("title:\n{title}\n")
    };

    format!(
        "{schema}\n\
Emit guidance only when supported by supplied evidence from the prompt, transcript, modified paths, or tool events. \
Prefer decisions, constraints, risks, rejected approaches, and do-not-repeat lessons. \
When the user selected implementation decisions, emit those decisions as guidanceFacts and cite the decision evidence. \
For Write/Edit evidence, prefer facts about the durable design decision over facts that merely say a file changed. \
Use concise evidenceExcerpt text copied or tightly paraphrased from the history. \
Use appliesTo.paths only from modified or explicitly referenced paths. \
Use appliesTo.symbols only when symbols are explicitly named. \
Use VERIFICATION only for reusable checks future sessions should run for a concrete reason.\n\n\
Session:\n\
checkpoint_id: {checkpoint_id}\n\
session_id: {session_id}\n\
turn_id: {turn_id}\n\
event_time: {event_time}\n\
agent_type: {agent_type}\n\
model: {model}\n\
{title_block}\
prompt:\n{prompt}\n\n\
transcript:\n{transcript}\n\n\
files_modified:\n- {files_modified}\n\n\
explicit_symbols:\n- {explicit_symbols}\n\n\
knowledge_source:\n{knowledge_label}\n\n\
tool_events:\n{tool_events}",
        schema = GUIDANCE_OUTPUT_SCHEMA_INSTRUCTION,
        checkpoint_id = checkpoint_id,
        session_id = session_id,
        turn_id = turn_id,
        event_time = event_time,
        agent_type = agent_type,
        model = model,
        title_block = title_block,
        prompt = prompt,
        transcript = transcript,
        files_modified = files_modified,
        explicit_symbols = explicit_symbols,
        knowledge_label = knowledge_label,
        tool_events = tool_events,
    )
}

pub fn build_knowledge_guidance_distillation_prompt(
    input: &KnowledgeGuidanceDistillationInput,
) -> String {
    let evidence = knowledge_evidence_input(input);
    let GuidanceEvidenceSource::Knowledge {
        provider,
        source_kind,
        title: _,
        url,
        updated_at,
        ..
    } = &evidence.source
    else {
        unreachable!("knowledge prompt uses knowledge source")
    };
    let body_preview = bounded_text(
        evidence_input_body(&evidence).as_str(),
        MAX_KNOWLEDGE_BODY_CHARS,
    );
    let normalized_fields_json =
        bounded_text(&input.normalized_fields_json, MAX_NORMALIZED_FIELDS_CHARS);
    let target_paths = bounded_files_modified(&evidence.target_paths);
    let target_symbols = bounded_files_modified(evidence_target_symbols(&evidence));
    let title = evidence_input_title(&evidence);

    format!(
        "{schema}\n\
Emit guidance only when the supplied knowledge source supports a durable future coding-session decision, constraint, reusable pattern, risk, or specific verification requirement. \
Use appliesTo.paths only from target_paths. Use appliesTo.symbols only from target_symbols. \
If the source is merely a status update, completed work summary, or generic note, return an empty guidanceFacts array.\n\n\
Knowledge source:\n\
provider: {provider}\n\
source_kind: {source_kind}\n\
title: {title}\n\
url: {url}\n\
updated_at: {updated_at}\n\n\
body_preview:\n{body_preview}\n\n\
normalized_fields_json:\n{normalized_fields_json}\n\n\
target_paths:\n- {target_paths}\n\n\
target_symbols:\n- {target_symbols}",
        schema = GUIDANCE_OUTPUT_SCHEMA_INSTRUCTION,
        provider = provider,
        source_kind = source_kind,
        title = title.as_str(),
        url = url.as_deref().unwrap_or(""),
        updated_at = updated_at.as_deref().unwrap_or(""),
        body_preview = body_preview,
        normalized_fields_json = normalized_fields_json,
        target_paths = target_paths,
        target_symbols = target_symbols,
    )
}

fn history_evidence_input(input: &GuidanceDistillationInput) -> GuidanceEvidenceInput {
    GuidanceEvidenceInput {
        source: GuidanceEvidenceSource::History {
            checkpoint_id: input.checkpoint_id.clone(),
            session_id: input.session_id.clone(),
            turn_id: input.turn_id.clone(),
            event_time: input.event_time.clone(),
            agent_type: input.agent_type.clone(),
            model: input.model.clone(),
        },
        title: None,
        body: None,
        prompt: input.prompt.clone(),
        transcript_fragment: input.transcript_fragment.clone(),
        target_paths: input.files_modified.clone(),
        target_symbols: Vec::new(),
        tool_events: input
            .tool_events
            .iter()
            .map(|event| GuidanceEvidenceToolEvent {
                tool_kind: event.tool_kind.clone(),
                input_summary: event.input_summary.clone(),
                output_summary: event.output_summary.clone(),
                command: event.command.clone(),
                file_path: event.file_path.clone(),
                evidence_text: event.evidence_text.clone(),
            })
            .collect(),
    }
}

fn knowledge_evidence_input(input: &KnowledgeGuidanceDistillationInput) -> GuidanceEvidenceInput {
    GuidanceEvidenceInput {
        source: GuidanceEvidenceSource::Knowledge {
            knowledge_item_id: input.knowledge_item_id.clone(),
            knowledge_item_version_id: input.knowledge_item_version_id.clone(),
            relation_assertion_id: input.relation_assertion_id.clone(),
            provider: input.provider.clone(),
            source_kind: input.source_kind.clone(),
            title: input.title.clone(),
            url: input.url.clone(),
            updated_at: input.updated_at.clone(),
        },
        title: input.title.clone(),
        body: input.body_preview.clone(),
        prompt: None,
        transcript_fragment: None,
        target_paths: input.target_paths.clone(),
        target_symbols: input.target_symbols.clone(),
        tool_events: Vec::new(),
    }
}

fn render_high_value_tool_events(events: &[GuidanceEvidenceToolEvent]) -> String {
    let mut indexed = events.iter().enumerate().collect::<Vec<_>>();
    indexed.sort_by(|(left_index, left), (right_index, right)| {
        tool_event_score(right)
            .cmp(&tool_event_score(left))
            .then_with(|| left_index.cmp(right_index))
    });

    let selected = indexed
        .into_iter()
        .take(MAX_TOOL_EVENTS)
        .map(|(_, event)| render_tool_event(event))
        .collect::<Vec<_>>();
    let omitted_tool_events = events.len().saturating_sub(selected.len());

    match (selected.is_empty(), omitted_tool_events) {
        (true, 0) => String::new(),
        (true, omitted) => format!("- omitted_tool_events: {omitted}"),
        (false, 0) => format!("high_value_tool_events:\n{}", selected.join("\n")),
        (false, omitted) => format!(
            "high_value_tool_events:\n{}\n- omitted_tool_events: {omitted}",
            selected.join("\n")
        ),
    }
}

fn bounded_history_transcript(value: &str) -> String {
    let bounded = bounded_text(value, MAX_TRANSCRIPT_CHARS);
    let high_value_lines = high_value_transcript_lines(value);
    let missing_high_value_lines = high_value_lines
        .into_iter()
        .filter(|line| !bounded.contains(line))
        .collect::<Vec<_>>();
    if missing_high_value_lines.is_empty() {
        return bounded;
    }
    let high_value_block = missing_high_value_lines.join("\n");
    if bounded.is_empty() {
        return bounded_text(high_value_block.as_str(), MAX_TRANSCRIPT_CHARS);
    }
    bounded_text(
        format!("high_value_transcript_lines:\n{high_value_block}\n\n{bounded}").as_str(),
        MAX_TRANSCRIPT_CHARS,
    )
}

fn high_value_transcript_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| {
            let text = line.to_ascii_lowercase();
            contains_any(
                text.as_str(),
                &[
                    "decisions locked",
                    "questions have been answered",
                    "user chose",
                    "decision:",
                ],
            )
        })
        .take(12)
        .map(str::to_string)
        .collect()
}

fn render_tool_event(event: &GuidanceEvidenceToolEvent) -> String {
    format!(
        "- kind: {}\n  command: {}\n  input: {}\n  output: {}\n  file: {}\n  evidence: {}",
        bounded_text(
            event.tool_kind.as_deref().unwrap_or(""),
            MAX_TOOL_COMMAND_CHARS
        ),
        bounded_text(
            event.command.as_deref().unwrap_or(""),
            MAX_TOOL_COMMAND_CHARS
        ),
        bounded_text(
            event.input_summary.as_deref().unwrap_or(""),
            MAX_TOOL_INPUT_CHARS
        ),
        bounded_text(
            event.output_summary.as_deref().unwrap_or(""),
            MAX_TOOL_OUTPUT_CHARS
        ),
        bounded_text(
            event.file_path.as_deref().unwrap_or(""),
            MAX_FILE_PATH_CHARS
        ),
        bounded_text(
            event.evidence_text.as_deref().unwrap_or(""),
            MAX_TOOL_OUTPUT_CHARS
        ),
    )
}

fn tool_event_score(event: &GuidanceEvidenceToolEvent) -> i32 {
    let text = format!(
        "{}\n{}\n{}\n{}\n{}",
        event.tool_kind.as_deref().unwrap_or(""),
        event.command.as_deref().unwrap_or(""),
        event.input_summary.as_deref().unwrap_or(""),
        event.output_summary.as_deref().unwrap_or(""),
        event.evidence_text.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase();

    let mut score = 0;
    if contains_any(
        text.as_str(),
        &[
            "askuserquestion",
            "questions have been answered",
            "decisions locked",
        ],
    ) {
        score += 100;
    }
    if contains_any(
        text.as_str(),
        &["write", "edit", "multiedit", "structuredpatch", "content:"],
    ) {
        score += 80;
    }
    if contains_any(
        text.as_str(),
        &[
            "error[e",
            "failed",
            "doesn't implement",
            "cannot be formatted",
        ],
    ) {
        score += 70;
    }
    if contains_any(
        text.as_str(),
        &["cargo ", "nextest", "test result", "passed"],
    ) {
        score += 60;
    }
    if contains_any(text.as_str(), &["src/", "tests/"]) {
        score += 20;
    }
    score
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn bounded_files_modified(paths: &[String]) -> String {
    let values = paths
        .iter()
        .take(MAX_FILES_MODIFIED)
        .map(|path| bounded_text(path, MAX_FILE_PATH_CHARS))
        .collect::<Vec<_>>();
    let omitted = paths.len().saturating_sub(MAX_FILES_MODIFIED);
    if omitted == 0 {
        values.join("\n- ")
    } else {
        format!("{}\n- omitted_paths: {omitted}", values.join("\n- "))
    }
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let marker = "\n[... omitted for context guidance prompt budget ...]\n";
    let marker_chars = marker.chars().count();
    if max_chars <= marker_chars {
        return trimmed.chars().take(max_chars).collect();
    }
    let available = max_chars - marker_chars;
    let head_chars = available / 2;
    let tail_chars = available - head_chars;
    let head = trimmed.chars().take(head_chars).collect::<String>();
    let tail = trimmed
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head}{marker}{tail}")
}

pub fn parse_guidance_distillation_output(raw: &str) -> Result<GuidanceDistillationOutput> {
    parse_guidance_distillation_output_with_default_targets(raw, &[], &[], false)
}

pub fn parse_guidance_distillation_output_for_input(
    raw: &str,
    input: &GuidanceDistillationInput,
) -> Result<GuidanceDistillationOutput> {
    Ok(parse_guidance_distillation_output_for_input_with_report(raw, input)?.output)
}

pub fn parse_guidance_distillation_output_for_input_with_report(
    raw: &str,
    input: &GuidanceDistillationInput,
) -> Result<DistilledGuidance> {
    let candidate_paths = candidate_paths_for_input(input);
    parse_guidance_distillation_output_with_default_targets_and_report(
        raw,
        &candidate_paths,
        &[],
        false,
    )
}

fn candidate_paths_for_input(input: &GuidanceDistillationInput) -> Vec<String> {
    input
        .files_modified
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_guidance_distillation_output_with_default_targets(
    raw: &str,
    default_paths: &[String],
    default_symbols: &[String],
    constrain_to_defaults: bool,
) -> Result<GuidanceDistillationOutput> {
    Ok(
        parse_guidance_distillation_output_with_default_targets_and_report(
            raw,
            default_paths,
            default_symbols,
            constrain_to_defaults,
        )?
        .output,
    )
}

fn parse_guidance_distillation_output_with_default_targets_and_report(
    raw: &str,
    default_paths: &[String],
    default_symbols: &[String],
    constrain_to_defaults: bool,
) -> Result<DistilledGuidance> {
    let payload = extract_json_object_from_text(raw)
        .ok_or_else(|| anyhow!("guidance distillation output did not contain a JSON object"))?;
    let parsed = match serde_json::from_str::<GuidanceDistillationOutput>(&payload) {
        Ok(parsed) => parsed,
        Err(err) => parse_flexible_guidance_distillation_output(&payload).map_err(|_| {
            anyhow!(
                "guidance distillation output has invalid summary, guidanceFacts, category, or confidence: {err}"
            )
        })?,
    };
    let raw_fact_count = parsed.guidance_facts.len();
    let validation_input_fact_count = raw_fact_count;
    let (validated, validation_discards) = validate_guidance_distillation_output_with_report(
        trim_guidance_distillation_output(parsed),
        default_paths,
        default_symbols,
        constrain_to_defaults,
    )?;
    let validation_kept_fact_count = validated.guidance_facts.len();
    let (filtered, quality_report) =
        super::quality::filter_value_guidance_output_with_report(validated);
    let quality_discards = quality_report
        .discarded
        .into_iter()
        .map(|discard| GuidanceQualityDiscardSummary {
            kind: discard.kind,
            category: discard.category,
            reason: format!("{:?}", discard.reason),
        })
        .collect();

    Ok(DistilledGuidance {
        output: filtered,
        report: GuidanceDistillationReport {
            raw_response_chars: raw.chars().count(),
            raw_fact_count,
            validation_input_fact_count,
            validation_kept_fact_count,
            validation_discards,
            quality_input_fact_count: quality_report.input_fact_count,
            quality_kept_fact_count: quality_report.kept_fact_count,
            quality_discards,
        },
    })
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlexibleGuidanceDistillationOutput {
    summary: FlexibleGuidanceSessionSummary,
    guidance_facts: Vec<FlexibleGuidanceFactDraft>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlexibleGuidanceSessionSummary {
    intent: Option<String>,
    outcome: Option<String>,
    #[serde(default)]
    decisions: Vec<String>,
    #[serde(default)]
    rejected_approaches: Vec<String>,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    verification: Vec<String>,
    #[serde(default)]
    open_items: Vec<String>,
    context: Option<FlexibleGuidanceSummaryContext>,
    file: Option<String>,
    function: Option<String>,
    purpose: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlexibleGuidanceSummaryContext {
    session: Option<FlexibleGuidanceSummarySession>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlexibleGuidanceSummarySession {
    focus: Option<String>,
    outcome: Option<String>,
    #[serde(default)]
    key_decisions: BTreeMap<String, String>,
    #[serde(default)]
    tool_usage: BTreeMap<String, String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum FlexibleGuidanceFactDraft {
    Canonical(super::types::GuidanceFactDraft),
    Nested(BTreeMap<NestedGuidanceFactKey, FlexibleNestedGuidanceFact>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum NestedGuidanceFactKey {
    Decision,
    Constraint,
    Pattern,
    Risk,
    Verification,
    Context,
    DoNotRepeat,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlexibleNestedGuidanceFact {
    kind: Option<String>,
    guidance: Option<String>,
    rationale: Option<String>,
    evidence: Option<String>,
    evidence_excerpt: Option<String>,
    #[serde(default)]
    applies_to: super::types::GuidanceAppliesTo,
    confidence: Option<super::types::GuidanceFactConfidence>,
}

fn parse_flexible_guidance_distillation_output(raw: &str) -> Result<GuidanceDistillationOutput> {
    let parsed: FlexibleGuidanceDistillationOutput = serde_json::from_str(raw)?;
    let facts = parsed
        .guidance_facts
        .into_iter()
        .map(flexible_guidance_fact_to_draft)
        .collect::<Result<Vec<_>>>()?;
    let summary = flexible_summary_to_summary(parsed.summary);

    Ok(GuidanceDistillationOutput {
        summary,
        guidance_facts: facts,
    })
}

fn flexible_summary_to_summary(
    summary: FlexibleGuidanceSessionSummary,
) -> super::types::GuidanceSessionSummary {
    let FlexibleGuidanceSessionSummary {
        intent,
        outcome,
        mut decisions,
        rejected_approaches,
        patterns,
        mut verification,
        open_items,
        context,
        file,
        function,
        purpose,
    } = summary;
    let session = context.and_then(|context| context.session);

    if let Some(session) = &session {
        decisions.extend(
            session
                .key_decisions
                .values()
                .filter_map(|value| non_empty_string(value.clone())),
        );
        verification.extend(
            session
                .tool_usage
                .values()
                .filter_map(|value| non_empty_string(value.clone())),
        );
    }

    let session_focus = session.as_ref().and_then(|session| session.focus.clone());
    let session_outcome = session.and_then(|session| session.outcome);
    let intent = first_non_empty([
        intent,
        session_focus,
        purpose.clone(),
        summary_subject(file, function),
    ])
    .unwrap_or_else(|| "Distill context guidance from session history.".to_string());
    let outcome = first_non_empty([
        outcome,
        session_outcome,
        purpose,
        Some("Produced evidence-backed guidance facts from the session.".to_string()),
    ])
    .unwrap_or_default();

    super::types::GuidanceSessionSummary {
        intent,
        outcome,
        decisions,
        rejected_approaches,
        patterns,
        verification,
        open_items,
    }
}

fn flexible_guidance_fact_to_draft(
    fact: FlexibleGuidanceFactDraft,
) -> Result<super::types::GuidanceFactDraft> {
    match fact {
        FlexibleGuidanceFactDraft::Canonical(fact) => Ok(fact),
        FlexibleGuidanceFactDraft::Nested(entries) => entries
            .into_iter()
            .next()
            .map(|(key, body)| nested_guidance_fact_to_draft(key, body))
            .unwrap_or_else(|| bail!("nested guidance fact must contain one category")),
    }
}

fn nested_guidance_fact_to_draft(
    key: NestedGuidanceFactKey,
    fact: FlexibleNestedGuidanceFact,
) -> Result<super::types::GuidanceFactDraft> {
    let category = nested_fact_category(key);
    let kind = first_non_empty([fact.kind, Some(nested_fact_kind(key).to_string())])
        .ok_or_else(|| anyhow!("nested guidance fact kind must be non-empty"))?;
    let guidance = first_non_empty([fact.guidance, fact.rationale.clone()])
        .ok_or_else(|| anyhow!("nested guidance fact guidance must be non-empty"))?;
    let evidence_excerpt = first_non_empty([fact.evidence_excerpt, fact.evidence])
        .ok_or_else(|| anyhow!("nested guidance fact evidence must be non-empty"))?;
    Ok(super::types::GuidanceFactDraft {
        category,
        kind,
        guidance,
        evidence_excerpt,
        applies_to: fact.applies_to,
        confidence: fact
            .confidence
            .unwrap_or(super::types::GuidanceFactConfidence::Medium),
    })
}

fn nested_fact_category(key: NestedGuidanceFactKey) -> super::types::GuidanceFactCategory {
    match key {
        NestedGuidanceFactKey::Decision => super::types::GuidanceFactCategory::Decision,
        NestedGuidanceFactKey::Constraint | NestedGuidanceFactKey::DoNotRepeat => {
            super::types::GuidanceFactCategory::Constraint
        }
        NestedGuidanceFactKey::Pattern => super::types::GuidanceFactCategory::Pattern,
        NestedGuidanceFactKey::Risk => super::types::GuidanceFactCategory::Risk,
        NestedGuidanceFactKey::Verification => super::types::GuidanceFactCategory::Verification,
        NestedGuidanceFactKey::Context => super::types::GuidanceFactCategory::Context,
    }
}

fn nested_fact_kind(key: NestedGuidanceFactKey) -> &'static str {
    match key {
        NestedGuidanceFactKey::Decision => "decision",
        NestedGuidanceFactKey::Constraint => "constraint",
        NestedGuidanceFactKey::Pattern => "pattern",
        NestedGuidanceFactKey::Risk => "risk",
        NestedGuidanceFactKey::Verification => "verification",
        NestedGuidanceFactKey::Context => "context",
        NestedGuidanceFactKey::DoNotRepeat => "do_not_repeat",
    }
}

fn first_non_empty(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    values
        .into_iter()
        .find_map(|value| value.and_then(non_empty_string))
}

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn summary_subject(file: Option<String>, function: Option<String>) -> Option<String> {
    let file = non_empty_string(file?)?;
    match function.and_then(non_empty_string) {
        Some(function) => Some(format!(
            "Distill context guidance for {function} in {file}."
        )),
        None => Some(format!("Distill context guidance for {file}.")),
    }
}

fn validate_guidance_distillation_output_with_report(
    mut output: GuidanceDistillationOutput,
    default_paths: &[String],
    default_symbols: &[String],
    constrain_to_defaults: bool,
) -> Result<(GuidanceDistillationOutput, Vec<GuidanceValidationDiscard>)> {
    let mut accepted_facts = Vec::new();
    let mut discards = Vec::new();
    let default_path_set = default_paths
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let default_symbol_set = default_symbols
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    for mut fact in output.guidance_facts {
        if fact.kind.is_empty() {
            bail!("guidance distillation fact kind must be non-empty");
        }
        if fact.guidance.is_empty() {
            bail!("guidance distillation fact guidance must be non-empty");
        }
        if fact.evidence_excerpt.is_empty() {
            bail!("guidance distillation fact evidenceExcerpt must be non-empty");
        }
        if fact.applies_to.paths.is_empty() && fact.applies_to.symbols.is_empty() {
            let inferred_paths = infer_paths_from_fact_text(&fact, default_paths);
            if inferred_paths.is_empty() && default_paths.len() == 1 && default_symbols.is_empty() {
                fact.applies_to.paths.extend(default_paths.iter().cloned());
            } else if inferred_paths.is_empty()
                && default_symbols.len() == 1
                && default_paths.is_empty()
            {
                fact.applies_to
                    .symbols
                    .extend(default_symbols.iter().cloned());
            } else if inferred_paths.is_empty() {
                discards.push(GuidanceValidationDiscard {
                    kind: fact.kind,
                    reason: GuidanceValidationDiscardReason::MissingTargetWithoutDefault,
                });
                continue;
            } else {
                fact.applies_to.paths = inferred_paths;
            }
        } else if constrain_to_defaults {
            fact.applies_to
                .paths
                .retain(|path| default_path_set.contains(path.as_str()));
            fact.applies_to
                .symbols
                .retain(|symbol| default_symbol_set.contains(symbol.as_str()));
            if fact.applies_to.paths.is_empty() && fact.applies_to.symbols.is_empty() {
                fact.applies_to.paths.extend(default_paths.iter().cloned());
                fact.applies_to
                    .symbols
                    .extend(default_symbols.iter().cloned());
            }
        }
        accepted_facts.push(fact);
    }
    output.guidance_facts = accepted_facts;
    Ok((output, discards))
}

fn infer_paths_from_fact_text(
    fact: &super::types::GuidanceFactDraft,
    candidate_paths: &[String],
) -> Vec<String> {
    let text = format!(
        "{}\n{}",
        fact.guidance.as_str(),
        fact.evidence_excerpt.as_str()
    );
    candidate_paths
        .iter()
        .filter(|path| text.contains(path.as_str()))
        .cloned()
        .collect()
}

fn extract_json_object_from_text(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed.to_string());
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (end > start).then(|| trimmed[start..=end].to_string())
}

pub struct GuidanceDistiller {
    service: Arc<dyn TextGenerationService>,
}

impl GuidanceDistiller {
    pub fn new(service: Arc<dyn TextGenerationService>) -> Self {
        Self { service }
    }

    pub fn distill(&self, input: &GuidanceDistillationInput) -> Result<GuidanceDistillationOutput> {
        Ok(self.distill_with_report(input)?.output)
    }

    pub fn distill_with_report(
        &self,
        input: &GuidanceDistillationInput,
    ) -> Result<DistilledGuidance> {
        let raw = self
            .service
            .complete(
                "You distill coding session history into concise summaries and evidence-backed guidance facts. Return only JSON.",
                &build_guidance_distillation_prompt(input),
            )
            .context("guidance distillation text generation failed")?;
        parse_guidance_distillation_output_for_input_with_report(&raw, input)
            .context("guidance distillation model output was invalid")
    }

    pub fn distill_knowledge(
        &self,
        input: &KnowledgeGuidanceDistillationInput,
    ) -> Result<GuidanceDistillationOutput> {
        let raw = self
            .service
            .complete(
                "You distill external project knowledge into concise, evidence-backed guidance facts. Return only JSON.",
                &build_knowledge_guidance_distillation_prompt(input),
            )
            .context("knowledge guidance distillation text generation failed")?;
        parse_guidance_distillation_output_with_default_targets(
            &raw,
            &input.target_paths,
            &input.target_symbols,
            true,
        )
        .context("knowledge guidance distillation model output was invalid")
    }
}

#[cfg(test)]
#[path = "distillation_tests.rs"]
mod tests;
