use sha2::{Digest, Sha256};

use super::storage::PersistedGuidanceTarget;
use super::types::{GuidanceFactCategory, GuidanceFactDraft};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuidanceLifecycleStatus {
    Active,
    Superseded,
    Duplicate,
    Rejected,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyTargetCompactionInput {
    pub compaction_run_id: String,
    pub target_type: String,
    pub target_value: String,
    pub retained_guidance_ids: Vec<String>,
    pub duplicate_guidance_ids: Vec<String>,
    pub superseded_guidance_ids: Vec<(String, String)>,
    pub summary_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyTargetCompactionOutcome {
    pub retained_count: usize,
    pub compacted_count: usize,
}

impl GuidanceLifecycleStatus {
    pub(super) fn as_storage(self) -> &'static str {
        match self {
            GuidanceLifecycleStatus::Active => "active",
            GuidanceLifecycleStatus::Superseded => "superseded",
            GuidanceLifecycleStatus::Duplicate => "duplicate",
            GuidanceLifecycleStatus::Rejected => "rejected",
            GuidanceLifecycleStatus::Stale => "stale",
        }
    }
}

pub(super) fn fact_fingerprint(
    fact: &GuidanceFactDraft,
    targets: &[PersistedGuidanceTarget],
) -> String {
    let input = format!(
        "{}\n{}",
        normalized_guidance_key(fact.category, fact.kind.as_str(), targets),
        normalize_text(fact.guidance.as_str())
    );
    sha256_hex(input.as_bytes())
}

pub(super) fn normalized_guidance_key(
    category: GuidanceFactCategory,
    kind: &str,
    targets: &[PersistedGuidanceTarget],
) -> String {
    let mut normalized_targets = targets
        .iter()
        .map(|target| {
            format!(
                "{}={}",
                target.target_type,
                normalize_text(target.target_value.as_str())
            )
        })
        .collect::<Vec<_>>();
    normalized_targets.sort();
    format!(
        "{}:{}:{}",
        category_storage_name(category),
        normalize_text(kind),
        normalized_targets.join("|")
    )
}

fn category_storage_name(category: GuidanceFactCategory) -> &'static str {
    match category {
        GuidanceFactCategory::Decision => "DECISION",
        GuidanceFactCategory::Constraint => "CONSTRAINT",
        GuidanceFactCategory::Pattern => "PATTERN",
        GuidanceFactCategory::Risk => "RISK",
        GuidanceFactCategory::Verification => "VERIFICATION",
        GuidanceFactCategory::Context => "CONTEXT",
    }
}

pub(super) fn is_known_lifecycle_status(value: &str) -> bool {
    [
        GuidanceLifecycleStatus::Active,
        GuidanceLifecycleStatus::Superseded,
        GuidanceLifecycleStatus::Duplicate,
        GuidanceLifecycleStatus::Rejected,
        GuidanceLifecycleStatus::Stale,
    ]
    .iter()
    .any(|status| status.as_storage() == value)
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_ascii_lowercase()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
