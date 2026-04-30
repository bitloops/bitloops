use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};

use crate::host::inference::TextGenerationService;

use super::types::{GuidanceDistillationOutput, trim_guidance_distillation_output};

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

pub struct GuidanceToolEvidence {
    pub tool_kind: Option<String>,
    pub input_summary: Option<String>,
    pub output_summary: Option<String>,
    pub command: Option<String>,
}

const MAX_PROMPT_CHARS: usize = 1_000;
const MAX_TRANSCRIPT_CHARS: usize = 6_000;
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
Do not return summary.context or guidanceFacts.decision objects."#;

pub fn build_guidance_distillation_prompt(input: &GuidanceDistillationInput) -> String {
    let files_modified = bounded_files_modified(&input.files_modified);
    let tool_events = input
        .tool_events
        .iter()
        .take(MAX_TOOL_EVENTS)
        .map(|event| {
            format!(
                "- kind: {}\n  command: {}\n  input: {}\n  output: {}",
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
                )
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let omitted_tool_events = input.tool_events.len().saturating_sub(MAX_TOOL_EVENTS);
    let tool_events = if omitted_tool_events == 0 {
        tool_events
    } else if tool_events.is_empty() {
        format!("- omitted_tool_events: {omitted_tool_events}")
    } else {
        format!("{tool_events}\n- omitted_tool_events: {omitted_tool_events}")
    };
    let prompt = bounded_text(input.prompt.as_deref().unwrap_or(""), MAX_PROMPT_CHARS);
    let transcript = bounded_text(
        input.transcript_fragment.as_deref().unwrap_or(""),
        MAX_TRANSCRIPT_CHARS,
    );

    format!(
        "{schema}\n\
Emit guidance only when supported by supplied evidence. Prefer decisions, rejected approaches, and do-not-repeat lessons. \
Use concise evidenceExcerpt text copied or tightly paraphrased from the history. \
Use appliesTo.paths only from modified or explicitly referenced paths. \
Use appliesTo.symbols only when symbols are explicitly named. \
Use VERIFICATION only when tool evidence shows a test, check, or manual verification.\n\n\
Session:\n\
checkpoint_id: {checkpoint_id}\n\
session_id: {session_id}\n\
turn_id: {turn_id}\n\
event_time: {event_time}\n\
agent_type: {agent_type}\n\
model: {model}\n\
prompt:\n{prompt}\n\n\
transcript:\n{transcript}\n\n\
files_modified:\n- {files_modified}\n\n\
tool_events:\n{tool_events}",
        schema = GUIDANCE_OUTPUT_SCHEMA_INSTRUCTION,
        checkpoint_id = input.checkpoint_id.as_deref().unwrap_or(""),
        session_id = input.session_id.as_str(),
        turn_id = input.turn_id.as_deref().unwrap_or(""),
        event_time = input.event_time.as_deref().unwrap_or(""),
        agent_type = input.agent_type.as_deref().unwrap_or(""),
        model = input.model.as_deref().unwrap_or(""),
        prompt = prompt,
        transcript = transcript,
        files_modified = files_modified,
        tool_events = tool_events,
    )
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
    parse_guidance_distillation_output_with_default_path(raw, None)
}

pub fn parse_guidance_distillation_output_for_input(
    raw: &str,
    input: &GuidanceDistillationInput,
) -> Result<GuidanceDistillationOutput> {
    let default_path = input
        .files_modified
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    let default_path = if default_path.len() == 1 {
        Some(default_path[0])
    } else {
        None
    };
    parse_guidance_distillation_output_with_default_path(raw, default_path)
}

fn parse_guidance_distillation_output_with_default_path(
    raw: &str,
    default_path: Option<&str>,
) -> Result<GuidanceDistillationOutput> {
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
    validate_guidance_distillation_output(trim_guidance_distillation_output(parsed), default_path)
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
        .filter_map(flexible_guidance_fact_to_draft)
        .collect::<Vec<_>>();
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
) -> Option<super::types::GuidanceFactDraft> {
    match fact {
        FlexibleGuidanceFactDraft::Canonical(fact) => Some(fact),
        FlexibleGuidanceFactDraft::Nested(entries) => entries
            .into_iter()
            .find_map(|(key, body)| nested_guidance_fact_to_draft(key, body)),
    }
}

fn nested_guidance_fact_to_draft(
    key: NestedGuidanceFactKey,
    fact: FlexibleNestedGuidanceFact,
) -> Option<super::types::GuidanceFactDraft> {
    let category = nested_fact_category(key);
    let kind = first_non_empty([fact.kind, Some(nested_fact_kind(key).to_string())])?;
    let guidance = first_non_empty([fact.guidance, fact.rationale.clone()])?;
    let evidence_excerpt = first_non_empty([fact.evidence_excerpt, fact.evidence])?;
    Some(super::types::GuidanceFactDraft {
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

fn validate_guidance_distillation_output(
    mut output: GuidanceDistillationOutput,
    default_path: Option<&str>,
) -> Result<GuidanceDistillationOutput> {
    let mut accepted_facts = Vec::new();
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
            if let Some(default_path) = default_path {
                fact.applies_to.paths.push(default_path.to_string());
            } else {
                continue;
            }
        }
        accepted_facts.push(fact);
    }
    output.guidance_facts = accepted_facts;
    Ok(output)
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
        let raw = self
            .service
            .complete(
                "You distill coding session history into concise summaries and evidence-backed guidance facts. Return only JSON.",
                &build_guidance_distillation_prompt(input),
            )
            .context("guidance distillation text generation failed")?;
        parse_guidance_distillation_output_for_input(&raw, input)
            .context("guidance distillation model output was invalid")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, anyhow};

    use super::{
        GuidanceDistillationInput, GuidanceDistiller, GuidanceToolEvidence,
        build_guidance_distillation_prompt, parse_guidance_distillation_output,
        parse_guidance_distillation_output_for_input,
    };
    use crate::capability_packs::context_guidance::types::{
        GuidanceFactCategory, GuidanceFactConfidence,
    };
    use crate::host::inference::TextGenerationService;

    const FIXTURE_JSON: &str = r#"{
  "summary": {
    "intent": "Improve attribute parsing.",
    "outcome": "Replaced fragile keyword-name parsing with token-based rendering.",
    "decisions": ["Use kw.to_token_stream().to_string() for duplicate keyword diagnostics."],
    "rejectedApproaches": ["Do not derive keyword names from std::any::type_name."],
    "patterns": ["Keep duplicate keyword handling centralized."],
    "verification": ["cargo nextest run --lib artefact_selection passed."],
    "openItems": []
  },
  "guidanceFacts": [
    {
      "category": "DECISION",
      "kind": "rejected_approach",
      "guidance": "Do not derive attribute keyword names from std::any::type_name.",
      "evidenceExcerpt": "Replaced std::any::type_name::<K>() parsing with kw.to_token_stream().to_string().",
      "appliesTo": {
        "paths": ["axum-macros/src/attr_parsing.rs"],
        "symbols": []
      },
      "confidence": "HIGH"
    }
  ]
}"#;

    fn input_with_one_modified_file() -> GuidanceDistillationInput {
        GuidanceDistillationInput {
            checkpoint_id: Some("checkpoint-1".to_string()),
            session_id: "session-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            event_time: Some("2026-04-29T10:00:00Z".to_string()),
            agent_type: Some("codex".to_string()),
            model: Some("gpt-5.4".to_string()),
            prompt: Some("Improve attr parsing".to_string()),
            transcript_fragment: Some("Replaced fragile keyword-name parsing.".to_string()),
            files_modified: vec!["axum-macros/src/attr_parsing.rs".to_string()],
            tool_events: vec![GuidanceToolEvidence {
                tool_kind: Some("shell".to_string()),
                input_summary: Some("cargo nextest run --lib artefact_selection".to_string()),
                output_summary: Some("passed".to_string()),
                command: Some("cargo nextest run --lib artefact_selection".to_string()),
            }],
        }
    }

    #[test]
    fn parse_guidance_distillation_output_accepts_valid_fixture() -> Result<()> {
        let parsed = parse_guidance_distillation_output(FIXTURE_JSON)?;

        assert_eq!(parsed.summary.intent, "Improve attribute parsing.");
        assert_eq!(parsed.guidance_facts.len(), 1);
        assert_eq!(
            parsed.guidance_facts[0].category,
            GuidanceFactCategory::Decision
        );
        assert_eq!(
            parsed.guidance_facts[0].confidence,
            GuidanceFactConfidence::High
        );
        Ok(())
    }

    #[test]
    fn parse_guidance_distillation_output_accepts_markdown_fence() -> Result<()> {
        let wrapped = format!("```json\n{FIXTURE_JSON}\n```");

        let parsed = parse_guidance_distillation_output(&wrapped)?;

        assert_eq!(parsed.guidance_facts.len(), 1);
        Ok(())
    }

    #[test]
    fn parse_guidance_distillation_output_accepts_nested_model_fact_shape() -> Result<()> {
        let raw = r#"{
  "summary": {
    "context": {
      "session": {
        "focus": "Refactoring axum-macros/src/from_request.rs to simplify extract_fields.",
        "keyDecisions": {
          "span_preservation": "Used quote_spanned! with ty_span to maintain original error-pointing behavior."
        },
        "toolUsage": {
          "tests": "Ran cargo test -p axum-macros and targeted from_request tests."
        }
      }
    }
  },
  "guidanceFacts": [
    {
      "decision": {
        "appliesTo": {
          "paths": ["axum-macros/src/from_request.rs"],
          "symbols": []
        },
        "rationale": "Avoided modifying infer_state_type_from_field_types to prevent unnecessary churn.",
        "evidence": "That function already delegates to crate::infer_state_types."
      }
    },
    {
      "verification": {
        "appliesTo": {
          "paths": ["axum-macros/src/from_request.rs"],
          "symbols": []
        },
        "rationale": "Ensured macro-generated code preserves span context for error reporting.",
        "evidence": "Span preservation: every newly-built TokenStream uses quote_spanned! with ty_span."
      }
    },
    {
      "doNotRepeat": {
        "appliesTo": {
          "paths": [],
          "symbols": ["wrap_extraction"]
        },
        "rationale": "Keep map_err classification centralized in wrap_extraction.",
        "evidence": "Classification + map_err computation lives inside wrap_extraction."
      }
    }
  ]
}"#;

        let parsed = parse_guidance_distillation_output(raw)?;

        assert_eq!(
            parsed.summary.intent,
            "Refactoring axum-macros/src/from_request.rs to simplify extract_fields."
        );
        assert_eq!(parsed.guidance_facts.len(), 3);
        assert_eq!(
            parsed.guidance_facts[0].category,
            GuidanceFactCategory::Decision
        );
        assert_eq!(
            parsed.guidance_facts[1].category,
            GuidanceFactCategory::Verification
        );
        assert_eq!(
            parsed.guidance_facts[2].category,
            GuidanceFactCategory::Constraint
        );
        assert_eq!(parsed.guidance_facts[2].kind, "do_not_repeat");
        assert_eq!(
            parsed.guidance_facts[0].confidence,
            GuidanceFactConfidence::Medium
        );
        Ok(())
    }

    #[test]
    fn parse_guidance_distillation_output_rejects_unsupported_nested_fact_keys() {
        let raw = r#"{
  "summary": {
    "context": {
      "session": {
        "focus": "Refactoring axum-macros/src/from_request.rs."
      }
    }
  },
  "guidanceFacts": [
    {
      "surprise": {
        "appliesTo": {
          "paths": ["axum-macros/src/from_request.rs"],
          "symbols": []
        },
        "rationale": "This category is not part of the schema.",
        "evidence": "Unsupported nested key."
      }
    }
  ]
}"#;

        assert!(
            parse_guidance_distillation_output(raw)
                .expect_err("unsupported nested fact key should fail")
                .to_string()
                .contains("guidanceFacts")
        );
    }

    #[test]
    fn parse_guidance_distillation_output_requires_summary_and_facts() {
        let missing_summary = r#"{"guidanceFacts":[]}"#;
        let missing_facts = r#"{"summary":{"intent":"","outcome":""}}"#;

        assert!(
            parse_guidance_distillation_output(missing_summary)
                .expect_err("missing summary should fail")
                .to_string()
                .contains("summary")
        );
        assert!(
            parse_guidance_distillation_output(missing_facts)
                .expect_err("missing guidanceFacts should fail")
                .to_string()
                .contains("guidanceFacts")
        );
    }

    #[test]
    fn parse_guidance_distillation_output_rejects_unknown_category() {
        let raw = FIXTURE_JSON.replace("\"DECISION\"", "\"SURPRISE\"");

        assert!(
            parse_guidance_distillation_output(&raw)
                .expect_err("unknown category should fail")
                .to_string()
                .contains("category")
        );
    }

    #[test]
    fn parse_guidance_distillation_output_rejects_empty_guidance_or_evidence() {
        let empty_guidance = FIXTURE_JSON.replace(
            "\"Do not derive attribute keyword names from std::any::type_name.\"",
            "\"\"",
        );
        let empty_evidence = FIXTURE_JSON.replace(
            "\"Replaced std::any::type_name::<K>() parsing with kw.to_token_stream().to_string().\"",
            "\"\"",
        );

        assert!(
            parse_guidance_distillation_output(&empty_guidance)
                .expect_err("empty guidance should fail")
                .to_string()
                .contains("guidance")
        );
        assert!(
            parse_guidance_distillation_output(&empty_evidence)
                .expect_err("empty evidence should fail")
                .to_string()
                .contains("evidenceExcerpt")
        );
    }

    #[test]
    fn parse_guidance_distillation_output_defaults_empty_targets_to_single_modified_file()
    -> Result<()> {
        let raw = FIXTURE_JSON.replace(
            r#""paths": ["axum-macros/src/attr_parsing.rs"]"#,
            r#""paths": []"#,
        );

        let parsed =
            parse_guidance_distillation_output_for_input(&raw, &input_with_one_modified_file())?;

        assert_eq!(
            parsed.guidance_facts[0].applies_to.paths,
            vec!["axum-macros/src/attr_parsing.rs".to_string()]
        );
        Ok(())
    }

    #[test]
    fn parse_guidance_distillation_output_drops_targetless_facts_when_ambiguous() -> Result<()> {
        let raw = FIXTURE_JSON.replace(
            r#""paths": ["axum-macros/src/attr_parsing.rs"]"#,
            r#""paths": []"#,
        );
        let mut input = input_with_one_modified_file();
        input.files_modified.push("src/other.rs".to_string());

        let parsed = parse_guidance_distillation_output_for_input(&raw, &input)?;

        assert!(parsed.guidance_facts.is_empty());
        Ok(())
    }

    #[test]
    fn build_guidance_distillation_prompt_bounds_large_history_input() {
        let mut input = input_with_one_modified_file();
        input.prompt = Some("large prompt ".repeat(1_000));
        input.transcript_fragment = Some("large transcript ".repeat(10_000));
        input.files_modified = (0..80)
            .map(|index| format!("src/generated/path_{index}.rs"))
            .collect();
        input.tool_events = (0..20)
            .map(|index| GuidanceToolEvidence {
                tool_kind: Some("shell".to_string()),
                input_summary: Some(format!("input {index} {}", "x".repeat(5_000))),
                output_summary: Some(format!("output {index} {}", "y".repeat(5_000))),
                command: Some(format!("cargo check {index} {}", "z".repeat(5_000))),
            })
            .collect();

        let prompt = build_guidance_distillation_prompt(&input);

        assert!(prompt.len() < 20_000);
        assert!(prompt.contains("omitted for context guidance prompt budget"));
        assert!(prompt.contains("omitted_tool_events: 14"));
        assert!(prompt.contains("omitted_paths: 30"));
        assert!(prompt.contains("src/generated/path_0.rs"));
    }

    #[test]
    fn build_guidance_distillation_prompt_declares_flat_schema_contract() {
        let prompt = build_guidance_distillation_prompt(&input_with_one_modified_file());

        assert!(prompt.contains("\"summary\""));
        assert!(prompt.contains("\"intent\""));
        assert!(prompt.contains("\"outcome\""));
        assert!(prompt.contains("\"guidanceFacts\""));
        assert!(prompt.contains("\"category\""));
        assert!(prompt.contains("\"evidenceExcerpt\""));
        assert!(prompt.contains("\"confidence\""));
        assert!(prompt.contains("flat array"));
        assert!(prompt.contains("Do not nest facts under category keys"));
        assert!(prompt.contains("Do not return summary.context"));
    }

    struct FakeTextGenerationService {
        response: Result<String>,
    }

    impl TextGenerationService for FakeTextGenerationService {
        fn descriptor(&self) -> String {
            "fake-guidance-model".to_string()
        }

        fn complete(&self, _system_prompt: &str, _user_prompt: &str) -> Result<String> {
            self.response
                .as_ref()
                .map(Clone::clone)
                .map_err(|err| anyhow!("{err:#}"))
        }
    }

    #[test]
    fn guidance_distiller_uses_text_generation_service() -> Result<()> {
        let distiller = GuidanceDistiller::new(Arc::new(FakeTextGenerationService {
            response: Ok(FIXTURE_JSON.to_string()),
        }));

        let parsed = distiller.distill(&input_with_one_modified_file())?;

        assert_eq!(parsed.guidance_facts.len(), 1);
        Ok(())
    }

    #[test]
    fn guidance_distiller_reports_malformed_model_output() {
        let distiller = GuidanceDistiller::new(Arc::new(FakeTextGenerationService {
            response: Ok("not json".to_string()),
        }));

        assert!(
            distiller
                .distill(&input_with_one_modified_file())
                .expect_err("malformed model output should fail")
                .to_string()
                .contains("guidance distillation")
        );
    }
}
