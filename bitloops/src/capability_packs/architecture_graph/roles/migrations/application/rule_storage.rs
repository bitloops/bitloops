use crate::capability_packs::architecture_graph::roles::taxonomy::{
    RuleSpecFile, role_rule_candidate_selector_contract, role_rule_conditions_contract,
};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(in crate::capability_packs::architecture_graph::roles::migrations) fn canonical_rule_hash(
    spec: &RuleSpecFile,
) -> Result<String> {
    let bytes = serde_json::to_vec(&rule_spec_storage_payload(spec)?)
        .context("serialise rule spec for hashing")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[derive(Debug, Serialize)]
pub(super) struct RuleSpecStoragePayload {
    pub candidate_selector: Value,
    pub positive_conditions: Value,
    pub negative_conditions: Value,
    pub score: Value,
}

pub(super) fn rule_spec_storage_payload(spec: &RuleSpecFile) -> Result<RuleSpecStoragePayload> {
    Ok(RuleSpecStoragePayload {
        candidate_selector: serde_json::to_value(role_rule_candidate_selector_contract(
            &spec.candidate_selector,
        ))?,
        positive_conditions: serde_json::to_value(role_rule_conditions_contract(
            &spec.positive_conditions,
        )?)?,
        negative_conditions: serde_json::to_value(role_rule_conditions_contract(
            &spec.negative_conditions,
        )?)?,
        score: serde_json::to_value(&spec.score)?,
    })
}

pub(super) fn sha256_json(value: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(value).context("serialise proposal payload for hashing")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
