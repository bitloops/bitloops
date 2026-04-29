use std::sync::Arc;

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

pub fn build_guidance_distillation_prompt(input: &GuidanceDistillationInput) -> String {
    let files_modified = input.files_modified.join("\n- ");
    let tool_events = input
        .tool_events
        .iter()
        .map(|event| {
            format!(
                "- kind: {}\n  command: {}\n  input: {}\n  output: {}",
                event.tool_kind.as_deref().unwrap_or(""),
                event.command.as_deref().unwrap_or(""),
                event.input_summary.as_deref().unwrap_or(""),
                event.output_summary.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Return only JSON with keys summary and guidanceFacts.\n\
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
        checkpoint_id = input.checkpoint_id.as_deref().unwrap_or(""),
        session_id = input.session_id.as_str(),
        turn_id = input.turn_id.as_deref().unwrap_or(""),
        event_time = input.event_time.as_deref().unwrap_or(""),
        agent_type = input.agent_type.as_deref().unwrap_or(""),
        model = input.model.as_deref().unwrap_or(""),
        prompt = input.prompt.as_deref().unwrap_or(""),
        transcript = input.transcript_fragment.as_deref().unwrap_or(""),
        files_modified = files_modified,
        tool_events = tool_events,
    )
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
    let parsed: GuidanceDistillationOutput = serde_json::from_str(&payload).map_err(|err| {
        anyhow!("guidance distillation output has invalid summary, guidanceFacts, category, or confidence: {err}")
    })?;
    validate_guidance_distillation_output(trim_guidance_distillation_output(parsed), default_path)
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
        parse_guidance_distillation_output, parse_guidance_distillation_output_for_input,
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
