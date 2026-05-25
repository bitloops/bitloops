use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::capability_packs::architecture_graph::roles::storage::ArchitectureRoleRuleRecord;
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    MatchableArtefact, RoleCandidateSelector, RoleFactCondition, RuleSpecFile,
    parse_rule_conditions, parse_rule_selector, role_rule_candidate_selector_contract,
    role_rule_conditions_contract, role_rule_contract_matches,
};

use super::compute_rule_matches;

pub(super) fn compute_stored_rule_matches(
    artefacts: &[MatchableArtefact],
    rule: &ArchitectureRoleRuleRecord,
) -> Result<BTreeSet<String>> {
    if let (Ok(selector), Ok(positive), Ok(negative)) = (
        parse_rule_selector(&rule.candidate_selector),
        parse_rule_conditions(&rule.positive_conditions),
        parse_rule_conditions(&rule.negative_conditions),
    ) {
        return Ok(compute_rule_matches(
            artefacts, &selector, &positive, &negative,
        ));
    }

    let selector = serde_json::from_value::<RoleCandidateSelector>(rule.candidate_selector.clone())
        .with_context(|| format!("parse fact-backed selector for rule `{}`", rule.rule_id))?;
    let positive =
        serde_json::from_value::<Vec<RoleFactCondition>>(rule.positive_conditions.clone())
            .with_context(|| {
                format!(
                    "parse fact-backed positive conditions for rule `{}`",
                    rule.rule_id
                )
            })?;
    let negative =
        serde_json::from_value::<Vec<RoleFactCondition>>(rule.negative_conditions.clone())
            .with_context(|| {
                format!(
                    "parse fact-backed negative conditions for rule `{}`",
                    rule.rule_id
                )
            })?;

    Ok(artefacts
        .iter()
        .filter(|artefact| role_rule_contract_matches(&selector, &positive, &negative, artefact))
        .map(|artefact| artefact.artefact_id.clone())
        .collect())
}

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
