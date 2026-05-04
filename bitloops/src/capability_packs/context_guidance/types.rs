#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceDistillationOutput {
    pub summary: GuidanceSessionSummary,
    pub guidance_facts: Vec<GuidanceFactDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceSessionSummary {
    pub intent: String,
    pub outcome: String,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub rejected_approaches: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub verification: Vec<String>,
    #[serde(default)]
    pub open_items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceFactDraft {
    pub category: GuidanceFactCategory,
    pub kind: String,
    pub guidance: String,
    pub evidence_excerpt: String,
    pub applies_to: GuidanceAppliesTo,
    pub confidence: GuidanceFactConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GuidanceFactCategory {
    Decision,
    Constraint,
    Pattern,
    Risk,
    Verification,
    Context,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GuidanceFactConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceAppliesTo {
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
}

pub fn trim_guidance_distillation_output(
    mut output: GuidanceDistillationOutput,
) -> GuidanceDistillationOutput {
    output.summary.intent = output.summary.intent.trim().to_string();
    output.summary.outcome = output.summary.outcome.trim().to_string();
    output.summary.decisions = trim_non_empty_values(output.summary.decisions);
    output.summary.rejected_approaches = trim_non_empty_values(output.summary.rejected_approaches);
    output.summary.patterns = trim_non_empty_values(output.summary.patterns);
    output.summary.verification = trim_non_empty_values(output.summary.verification);
    output.summary.open_items = trim_non_empty_values(output.summary.open_items);
    output.guidance_facts = output
        .guidance_facts
        .into_iter()
        .map(trim_guidance_fact)
        .collect();
    output
}

fn trim_guidance_fact(mut fact: GuidanceFactDraft) -> GuidanceFactDraft {
    fact.kind = fact.kind.trim().to_string();
    fact.guidance = fact.guidance.trim().to_string();
    fact.evidence_excerpt = fact.evidence_excerpt.trim().to_string();
    fact.applies_to.paths = trim_non_empty_values(fact.applies_to.paths);
    fact.applies_to.symbols = trim_non_empty_values(fact.applies_to.symbols);
    fact
}

fn trim_non_empty_values(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}
