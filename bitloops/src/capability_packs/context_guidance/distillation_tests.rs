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
    "rationale": "Run cargo nextest -p axum-macros from_request because macro-generated span behavior can regress.",
    "evidence": "Ran cargo nextest -p axum-macros from_request after preserving quote_spanned! with ty_span."
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
fn parse_guidance_distillation_output_rejects_incomplete_nested_fact() {
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
  "decision": {
    "appliesTo": {
      "paths": ["axum-macros/src/from_request.rs"],
      "symbols": []
    },
    "rationale": "Keep parser validation centralized."
  }
}
  ]
}"#;

    assert!(
        parse_guidance_distillation_output(raw)
            .expect_err("incomplete nested fact should fail")
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
fn parse_guidance_distillation_output_defaults_empty_targets_to_single_modified_file() -> Result<()>
{
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
fn parse_guidance_distillation_output_drops_low_value_facts() -> Result<()> {
    let raw = r#"{
  "summary": {
"intent": "Refactor from_request extraction.",
"outcome": "Centralized wrapper handling.",
"decisions": ["Keep wrapper-specific branch handling centralized."],
"rejectedApproaches": [],
"patterns": [],
"verification": ["Checked the refactor reduced code size."],
"openItems": []
  },
  "guidanceFacts": [
{
  "category": "VERIFICATION",
  "kind": "code_reduction_verification",
  "guidance": "Confirm the refactor reduces code size by around 110 lines.",
  "evidenceExcerpt": "Refactor 2 - extract_fields lines 422-627 to 422-537, around 110 lines saved.",
  "appliesTo": {
    "paths": ["axum-macros/src/from_request.rs"],
    "symbols": []
  },
  "confidence": "HIGH"
},
{
  "category": "DECISION",
  "kind": "centralize_extraction_logic_in_wrap_extraction",
  "guidance": "Keep classification and map_err computation inside wrap_extraction so future call sites do not duplicate wrapper-specific branches.",
  "evidenceExcerpt": "Classification + map_err computation now lives inside wrap_extraction instead of each call site branch.",
  "appliesTo": {
    "paths": ["axum-macros/src/from_request.rs"],
    "symbols": ["wrap_extraction"]
  },
  "confidence": "HIGH"
}
  ]
}"#;

    let parsed = parse_guidance_distillation_output(raw)?;

    assert_eq!(parsed.guidance_facts.len(), 1);
    assert_eq!(
        parsed.guidance_facts[0].category,
        GuidanceFactCategory::Decision
    );
    assert_eq!(
        parsed.guidance_facts[0].kind,
        "centralize_extraction_logic_in_wrap_extraction"
    );
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

#[test]
fn build_guidance_distillation_prompt_declares_future_session_value_contract() {
    let prompt = build_guidance_distillation_prompt(&input_with_one_modified_file());

    assert!(prompt.contains("future coding session"));
    assert!(prompt.contains(r#""guidanceFacts": []"#));
    assert!(prompt.contains("durable guidance"));
    assert!(prompt.contains("status updates"));
    assert!(prompt.contains("completed-work"));
    assert!(prompt.contains("code-size"));
    assert!(prompt.contains("line-count"));
    assert!(prompt.contains("generic \"tests passed\" facts"));
    assert!(prompt.contains("generic \"ensure quality\" advice"));
    assert!(prompt.contains("\"the agent edited this file\" context"));
    assert!(prompt.contains("VERIFICATION"));
    assert!(prompt.contains("reusable command/check"));
    assert!(prompt.contains("why a future session should run it"));
    assert!(prompt.contains("CONTEXT"));
    assert!(prompt.contains("durable codebase boundary"));
    assert!(prompt.contains("invariant"));
    assert!(prompt.contains("dependency"));
    assert!(prompt.contains("ownership"));
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
