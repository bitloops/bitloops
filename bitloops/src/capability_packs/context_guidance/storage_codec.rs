use anyhow::{Result, anyhow};
use sha2::{Digest, Sha256};

use super::types::{GuidanceFactCategory, GuidanceFactConfidence};

pub(super) fn category_to_storage(category: GuidanceFactCategory) -> &'static str {
    match category {
        GuidanceFactCategory::Decision => "DECISION",
        GuidanceFactCategory::Constraint => "CONSTRAINT",
        GuidanceFactCategory::Pattern => "PATTERN",
        GuidanceFactCategory::Risk => "RISK",
        GuidanceFactCategory::Verification => "VERIFICATION",
        GuidanceFactCategory::Context => "CONTEXT",
    }
}

pub(super) fn category_from_storage(value: &str) -> Result<GuidanceFactCategory> {
    match value {
        "DECISION" => Ok(GuidanceFactCategory::Decision),
        "CONSTRAINT" => Ok(GuidanceFactCategory::Constraint),
        "PATTERN" => Ok(GuidanceFactCategory::Pattern),
        "RISK" => Ok(GuidanceFactCategory::Risk),
        "VERIFICATION" => Ok(GuidanceFactCategory::Verification),
        "CONTEXT" => Ok(GuidanceFactCategory::Context),
        other => Err(anyhow!("unknown context guidance category `{other}`")),
    }
}

pub(super) fn confidence_to_storage(confidence: GuidanceFactConfidence) -> &'static str {
    match confidence {
        GuidanceFactConfidence::High => "HIGH",
        GuidanceFactConfidence::Medium => "MEDIUM",
        GuidanceFactConfidence::Low => "LOW",
    }
}

pub(super) fn confidence_from_storage(value: &str) -> Result<GuidanceFactConfidence> {
    match value {
        "HIGH" => Ok(GuidanceFactConfidence::High),
        "MEDIUM" => Ok(GuidanceFactConfidence::Medium),
        "LOW" => Ok(GuidanceFactConfidence::Low),
        other => Err(anyhow!("unknown context guidance confidence `{other}`")),
    }
}

pub(super) fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}
