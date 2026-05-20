use std::collections::BTreeSet;

use super::storage::PersistedGuidanceSource;
use super::types::{
    GuidanceDistillationOutput, GuidanceFactCategory, GuidanceFactConfidence, GuidanceFactDraft,
};

const MAX_SOURCES_PER_FACT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuidanceQualityDiscardReason {
    MissingTarget,
    GuidanceTooShort,
    EvidenceTooShort,
    LowValueStatusFact,
    NonReusableVerification,
    NonDurableContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuidanceQualityDiscard {
    pub kind: String,
    pub category: GuidanceFactCategory,
    pub reason: GuidanceQualityDiscardReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuidanceQualityReport {
    pub input_fact_count: usize,
    pub kept_fact_count: usize,
    pub discarded: Vec<GuidanceQualityDiscard>,
}

#[cfg(test)]
pub(crate) fn filter_value_guidance_output(
    output: GuidanceDistillationOutput,
) -> GuidanceDistillationOutput {
    filter_value_guidance_output_with_report(output).0
}

pub(super) fn filter_value_guidance_output_with_report(
    mut output: GuidanceDistillationOutput,
) -> (GuidanceDistillationOutput, GuidanceQualityReport) {
    let input_fact_count = output.guidance_facts.len();
    let mut kept = Vec::new();
    let mut discarded = Vec::new();

    for fact in output.guidance_facts {
        match discard_reason(&fact) {
            Some(reason) => discarded.push(GuidanceQualityDiscard {
                kind: fact.kind.clone(),
                category: fact.category,
                reason,
            }),
            None => kept.push(fact),
        }
    }

    let kept_fact_count = kept.len();
    output.guidance_facts = kept;

    (
        output,
        GuidanceQualityReport {
            input_fact_count,
            kept_fact_count,
            discarded,
        },
    )
}

pub(super) fn discard_reason(fact: &GuidanceFactDraft) -> Option<GuidanceQualityDiscardReason> {
    if !has_target(fact) {
        return Some(GuidanceQualityDiscardReason::MissingTarget);
    }
    if !has_specific_text(fact.guidance.as_str()) {
        return Some(GuidanceQualityDiscardReason::GuidanceTooShort);
    }
    if !has_specific_text(fact.evidence_excerpt.as_str()) {
        return Some(GuidanceQualityDiscardReason::EvidenceTooShort);
    }
    if is_low_value_status_fact(fact) {
        return Some(GuidanceQualityDiscardReason::LowValueStatusFact);
    }
    match fact.category {
        GuidanceFactCategory::Decision
        | GuidanceFactCategory::Constraint
        | GuidanceFactCategory::Pattern
        | GuidanceFactCategory::Risk => None,
        GuidanceFactCategory::Verification => (!is_reusable_verification(fact))
            .then_some(GuidanceQualityDiscardReason::NonReusableVerification),
        GuidanceFactCategory::Context => {
            (!is_durable_context(fact)).then_some(GuidanceQualityDiscardReason::NonDurableContext)
        }
    }
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

    #[test]
    fn reports_discard_reasons_for_low_value_facts() {
        let output = GuidanceDistillationOutput {
            summary: crate::capability_packs::context_guidance::types::GuidanceSessionSummary {
                intent: "Capture session guidance.".to_string(),
                outcome: "Model returned mixed quality facts.".to_string(),
                decisions: Vec::new(),
                rejected_approaches: Vec::new(),
                patterns: Vec::new(),
                verification: Vec::new(),
                open_items: Vec::new(),
            },
            guidance_facts: vec![
                GuidanceFactDraft {
                    category: GuidanceFactCategory::Decision,
                    kind: "missing_target".to_string(),
                    guidance:
                        "Keep handler responses typed so tests assert status and body directly."
                            .to_string(),
                    evidence_excerpt:
                        "The session chose typed HttpResponse values for API handlers.".to_string(),
                    applies_to: GuidanceAppliesTo::default(),
                    confidence: GuidanceFactConfidence::High,
                },
                fact(
                    GuidanceFactCategory::Verification,
                    "generic_tests_passed",
                    "The tests passed after the refactor completed.",
                    "4 tests passed after the work completed.",
                ),
                fact(
                    GuidanceFactCategory::Decision,
                    "typed_api_response",
                    "Keep API handlers returning HttpResponse so status and body are asserted directly.",
                    "User chose a new src/api/response.rs HttpResponse and no Display compatibility shim.",
                ),
            ],
        };

        let (filtered, report) = filter_value_guidance_output_with_report(output);

        assert_eq!(report.input_fact_count, 3);
        assert_eq!(report.kept_fact_count, 1);
        assert_eq!(filtered.guidance_facts.len(), 1);
        assert_eq!(filtered.guidance_facts[0].kind, "typed_api_response");
        assert_eq!(report.discarded.len(), 2);
        assert_eq!(report.discarded[0].kind, "missing_target");
        assert_eq!(
            report.discarded[0].reason,
            GuidanceQualityDiscardReason::MissingTarget
        );
        assert_eq!(report.discarded[1].kind, "generic_tests_passed");
        assert_eq!(
            report.discarded[1].reason,
            GuidanceQualityDiscardReason::LowValueStatusFact
        );
    }
}
