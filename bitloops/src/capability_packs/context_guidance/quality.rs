use std::collections::BTreeSet;

use super::storage::PersistedGuidanceSource;
use super::types::{
    GuidanceDistillationOutput, GuidanceFactCategory, GuidanceFactConfidence, GuidanceFactDraft,
};

const MAX_SOURCES_PER_FACT: usize = 3;

pub(super) fn filter_value_guidance_output(
    mut output: GuidanceDistillationOutput,
) -> GuidanceDistillationOutput {
    output.guidance_facts = output
        .guidance_facts
        .into_iter()
        .filter(is_valuable_fact)
        .collect();
    output
}

pub(super) fn is_valuable_fact(fact: &GuidanceFactDraft) -> bool {
    has_target(fact)
        && has_specific_text(fact.guidance.as_str())
        && has_specific_text(fact.evidence_excerpt.as_str())
        && !is_low_value_status_fact(fact)
        && is_category_valuable(fact)
}

pub(super) fn guidance_value_score(
    category: GuidanceFactCategory,
    confidence: GuidanceFactConfidence,
    has_symbol_target: bool,
) -> f64 {
    let category_score = match category {
        GuidanceFactCategory::Decision => 0.95,
        GuidanceFactCategory::Constraint => 0.93,
        GuidanceFactCategory::Risk => 0.90,
        GuidanceFactCategory::Pattern => 0.86,
        GuidanceFactCategory::Context => 0.74,
        GuidanceFactCategory::Verification => 0.70,
    };
    let confidence_bonus = match confidence {
        GuidanceFactConfidence::High => 0.04,
        GuidanceFactConfidence::Medium => 0.02,
        GuidanceFactConfidence::Low => 0.0,
    };
    let symbol_bonus = if has_symbol_target { 0.01 } else { 0.0 };
    category_score + confidence_bonus + symbol_bonus
}

pub(super) fn dedupe_and_cap_sources(
    sources: impl IntoIterator<Item = PersistedGuidanceSource>,
) -> Vec<PersistedGuidanceSource> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for source in sources {
        let key = source_dedupe_key(&source);
        if seen.insert(key) {
            out.push(source);
        }
        if out.len() >= MAX_SOURCES_PER_FACT {
            break;
        }
    }
    out
}

fn has_target(fact: &GuidanceFactDraft) -> bool {
    !fact.applies_to.paths.is_empty() || !fact.applies_to.symbols.is_empty()
}

fn has_specific_text(value: &str) -> bool {
    value.split_whitespace().count() >= 5
}

fn is_category_valuable(fact: &GuidanceFactDraft) -> bool {
    match fact.category {
        GuidanceFactCategory::Decision
        | GuidanceFactCategory::Constraint
        | GuidanceFactCategory::Pattern
        | GuidanceFactCategory::Risk => true,
        GuidanceFactCategory::Verification => is_reusable_verification(fact),
        GuidanceFactCategory::Context => is_durable_context(fact),
    }
}

fn is_reusable_verification(fact: &GuidanceFactDraft) -> bool {
    let text = normalized_fact_text(fact);
    contains_any(
        text.as_str(),
        &[
            "run ",
            "cargo ",
            "nextest",
            "clippy",
            "fmt",
            "regression",
            "because",
            "risk",
        ],
    ) && !contains_any(
        text.as_str(),
        &[
            "tests passed",
            "build passed",
            "completed",
            "line saved",
            "lines saved",
        ],
    )
}

fn is_durable_context(fact: &GuidanceFactDraft) -> bool {
    let text = normalized_fact_text(fact);
    contains_any(
        text.as_str(),
        &[
            "because",
            "depends on",
            "owned by",
            "contract",
            "invariant",
            "boundary",
        ],
    ) && !contains_any(text.as_str(), &["edited ", "changed ", "worked on "])
}

fn is_low_value_status_fact(fact: &GuidanceFactDraft) -> bool {
    let text = normalized_fact_text(fact);
    contains_any(
        text.as_str(),
        &[
            "confirm the refactor",
            "code reduction",
            "line reduction",
            "lines saved",
            "line saved",
            "ensure code quality",
            "review the code",
            "tests passed",
            "build passed",
            "work completed",
        ],
    )
}

fn normalized_fact_text(fact: &GuidanceFactDraft) -> String {
    format!(
        "{}\n{}\n{}",
        fact.kind, fact.guidance, fact.evidence_excerpt
    )
    .to_ascii_lowercase()
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn source_dedupe_key(source: &PersistedGuidanceSource) -> String {
    [
        source.source_type.as_str(),
        source.checkpoint_id.as_deref().unwrap_or(""),
        source.session_id.as_deref().unwrap_or(""),
        source.turn_id.as_deref().unwrap_or(""),
        source.tool_kind.as_deref().unwrap_or(""),
        source.excerpt.as_deref().unwrap_or(""),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::context_guidance::types::GuidanceAppliesTo;

    fn fact(
        category: GuidanceFactCategory,
        kind: &str,
        guidance: &str,
        evidence_excerpt: &str,
    ) -> GuidanceFactDraft {
        GuidanceFactDraft {
            category,
            kind: kind.to_string(),
            guidance: guidance.to_string(),
            evidence_excerpt: evidence_excerpt.to_string(),
            applies_to: GuidanceAppliesTo {
                paths: vec!["axum-macros/src/from_request.rs".to_string()],
                symbols: Vec::new(),
            },
            confidence: GuidanceFactConfidence::High,
        }
    }

    #[test]
    fn drops_low_value_code_reduction_verification() {
        let output = GuidanceDistillationOutput {
            summary: crate::capability_packs::context_guidance::types::GuidanceSessionSummary {
                intent: "Refactor from_request extraction.".to_string(),
                outcome: "Reduced repeated branches.".to_string(),
                decisions: Vec::new(),
                rejected_approaches: Vec::new(),
                patterns: Vec::new(),
                verification: Vec::new(),
                open_items: Vec::new(),
            },
            guidance_facts: vec![fact(
                GuidanceFactCategory::Verification,
                "code_reduction_verification",
                "Confirm the refactor reduces code size by around 110 lines.",
                "Refactor 2 - extract_fields lines 422-627 to 422-537, around 110 lines saved.",
            )],
        };

        let filtered = filter_value_guidance_output(output);

        assert!(filtered.guidance_facts.is_empty());
    }

    #[test]
    fn drops_generic_context_status() {
        let output = GuidanceDistillationOutput {
            summary: crate::capability_packs::context_guidance::types::GuidanceSessionSummary {
                intent: "Capture session context.".to_string(),
                outcome: "Stored history details.".to_string(),
                decisions: Vec::new(),
                rejected_approaches: Vec::new(),
                patterns: Vec::new(),
                verification: Vec::new(),
                open_items: Vec::new(),
            },
            guidance_facts: vec![fact(
                GuidanceFactCategory::Context,
                "edited_file_context",
                "The agent edited axum-macros/src/from_request.rs during the session.",
                "Session focus: editing axum-macros/src/from_request.rs.",
            )],
        };

        let filtered = filter_value_guidance_output(output);

        assert!(filtered.guidance_facts.is_empty());
    }

    #[test]
    fn keeps_future_session_decision() {
        let output = GuidanceDistillationOutput {
            summary: crate::capability_packs::context_guidance::types::GuidanceSessionSummary {
                intent: "Refactor from_request extraction.".to_string(),
                outcome: "Centralized wrapper handling.".to_string(),
                decisions: Vec::new(),
                rejected_approaches: Vec::new(),
                patterns: Vec::new(),
                verification: Vec::new(),
                open_items: Vec::new(),
            },
            guidance_facts: vec![fact(
                GuidanceFactCategory::Decision,
                "centralize_extraction_logic_in_wrap_extraction",
                "Keep classification and map_err computation inside wrap_extraction so call sites do not duplicate wrapper-specific branches.",
                "Classification + map_err computation lives inside wrap_extraction. Call sites do not need to know the field wrapper kind.",
            )],
        };

        let filtered = filter_value_guidance_output(output);

        assert_eq!(filtered.guidance_facts.len(), 1);
    }

    #[test]
    fn keeps_reusable_verification_with_specific_command_and_reason() {
        let output = GuidanceDistillationOutput {
            summary: crate::capability_packs::context_guidance::types::GuidanceSessionSummary {
                intent: "Preserve debug_handler receiver behavior.".to_string(),
                outcome: "Captured verification requirement.".to_string(),
                decisions: Vec::new(),
                rejected_approaches: Vec::new(),
                patterns: Vec::new(),
                verification: Vec::new(),
                open_items: Vec::new(),
            },
            guidance_facts: vec![fact(
                GuidanceFactCategory::Verification,
                "debug_handler_self_receiver_regression_check",
                "Run cargo nextest for debug_handler Self receiver cases because span-sensitive macro behavior can regress.",
                "Ran cargo nextest run -p axum-macros debug_handler_self_receiver and verified receiver handling.",
            )],
        };

        let filtered = filter_value_guidance_output(output);

        assert_eq!(filtered.guidance_facts.len(), 1);
    }
}
