use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::capability_packs::architecture_graph::roles::fact_extraction::{
    ArchitectureRoleFactExtractionInput, SliceArchitectureRoleCurrentStateSource,
    extract_architecture_role_facts,
};
use crate::capability_packs::architecture_graph::roles::rules::{
    compile_detection_rules, evaluate_rules_over_facts,
};
use crate::capability_packs::architecture_graph::roles::storage::{
    ArchitectureRoleRuleRecord, load_assignments_for_paths, load_current_role_facts,
};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    ArchitectureArtefactFact, ArchitectureRoleDetectionRule, AssignmentStatus, RoleRuleCondition,
    RoleRuleLifecycle, RoleSignalPolarity, RoleTarget, RuleSpecFile, TargetKind, parse_rule_score,
    role_rule_candidate_selector_contract, role_rule_conditions_contract,
};
use crate::host::capability_host::gateways::RelationalGateway;
use crate::host::devql::RelationalStorage;

pub(in crate::capability_packs::architecture_graph::roles::migrations) async fn preview_rule_spec(
    relational: &RelationalStorage,
    gateway: &dyn RelationalGateway,
    repo_id: &str,
    role_id: &str,
    spec: &RuleSpecFile,
    existing_rule: Option<&ArchitectureRoleRuleRecord>,
) -> Result<Value> {
    let facts = load_preview_role_facts(relational, gateway, repo_id).await?;
    let total_targets = facts
        .iter()
        .map(|fact| fact.target.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let new_targets = evaluate_rule_spec_targets(repo_id, role_id, spec, &facts)?;
    let new_matches = target_keys(&new_targets);
    let current_targets = if let Some(rule) = existing_rule {
        evaluate_stored_rule_targets(&facts, rule)?
    } else {
        BTreeSet::new()
    };
    let current_matches = target_keys(&current_targets);
    let affected_assignment_ids =
        active_assignment_ids_for_targets(relational, repo_id, role_id, &current_targets).await?;
    let added_matches = new_matches
        .difference(&current_matches)
        .cloned()
        .collect::<Vec<_>>();
    let removed_matches = current_matches
        .difference(&new_matches)
        .cloned()
        .collect::<Vec<_>>();
    let affected_artefact_ids = current_targets
        .union(&new_targets)
        .filter_map(preview_affected_artefact_id)
        .collect::<BTreeSet<_>>();
    let matched_paths = new_targets
        .iter()
        .map(|target| target.path.clone())
        .collect::<BTreeSet<_>>();
    let safety = rule_preview_safety(total_targets, &new_matches, &matched_paths, spec);
    let affected_target_keys = current_matches
        .union(&new_matches)
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(json!({
        "operation": if existing_rule.is_some() { "edit_rule" } else { "draft_rule" },
        "affected_role_ids": [role_id],
        "affected_rule_ids": existing_rule
            .map(|rule| vec![rule.rule_id.clone()])
            .unwrap_or_default(),
        "affected_assignment_ids": affected_assignment_ids,
        "affected_artefact_ids": affected_artefact_ids.clone(),
        "affected_target_keys": affected_target_keys,
        "affected_roles": 1,
        "affected_rules": if existing_rule.is_some() { 1 } else { 0 },
        "current_matches": current_matches,
        "new_matches": new_matches,
        "added_matches": added_matches,
        "removed_matches": removed_matches,
        "affected_assignments": affected_assignment_ids.len(),
        "affected_artefacts": affected_artefact_ids.len(),
        "safety": safety,
        "downstream_review_work": {
            "reclassification_required": !removed_matches.is_empty() || !added_matches.is_empty(),
        }
    }))
}

async fn active_assignment_ids_for_targets(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
    targets: &BTreeSet<RoleTarget>,
) -> Result<BTreeSet<String>> {
    let paths = targets
        .iter()
        .map(|target| target.path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Ok(load_assignments_for_paths(relational, repo_id, &paths)
        .await?
        .into_iter()
        .filter(|assignment| assignment.role_id == role_id)
        .filter(|assignment| assignment.status == AssignmentStatus::Active)
        .filter(|assignment| targets.contains(&assignment.target))
        .map(|assignment| assignment.assignment_id)
        .collect())
}

async fn load_preview_role_facts(
    relational: &RelationalStorage,
    gateway: &dyn RelationalGateway,
    repo_id: &str,
) -> Result<Vec<ArchitectureArtefactFact>> {
    let facts = load_current_role_facts(relational, repo_id).await?;
    if !facts.is_empty() {
        return Ok(facts);
    }
    fallback_preview_facts(gateway, repo_id)
}

fn fallback_preview_facts(
    gateway: &dyn RelationalGateway,
    repo_id: &str,
) -> Result<Vec<ArchitectureArtefactFact>> {
    let files = optional_fallback_preview_records(gateway.load_current_canonical_files(repo_id))?;
    let artefacts = gateway
        .load_current_canonical_artefacts(repo_id)
        .context("loading current canonical artefacts for role rule preview fallback")?;
    let edges = optional_fallback_preview_records(gateway.load_current_canonical_edges(repo_id))?;
    let affected_paths = files
        .iter()
        .map(|file| file.path.clone())
        .chain(artefacts.iter().map(|artefact| artefact.path.clone()))
        .chain(edges.iter().map(|edge| edge.path.clone()))
        .collect::<BTreeSet<_>>();
    if affected_paths.is_empty() {
        return Ok(Vec::new());
    }
    let current_state = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &edges);
    Ok(extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id,
            generation_seq: 0,
            affected_paths: &affected_paths,
            files: &files,
        },
        &current_state,
    )?
    .facts)
}

fn optional_fallback_preview_records<T>(records: Result<Vec<T>>) -> Result<Vec<T>> {
    match records {
        Ok(records) => Ok(records),
        Err(error) if error.to_string().contains("not implemented") => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn evaluate_rule_spec_targets(
    repo_id: &str,
    role_id: &str,
    spec: &RuleSpecFile,
    facts: &[ArchitectureArtefactFact],
) -> Result<BTreeSet<RoleTarget>> {
    let draft_rule = ArchitectureRoleDetectionRule {
        repo_id: repo_id.to_string(),
        rule_id: "preview-rule".to_string(),
        role_id: role_id.to_string(),
        version: 1,
        lifecycle: RoleRuleLifecycle::Draft,
        priority: spec.score.priority_hint.unwrap_or(100),
        score: spec.score.base_confidence.unwrap_or(0.80),
        min_positive_ratio: spec.score.min_positive_ratio.unwrap_or(1.0),
        candidate_selector: serde_json::to_value(role_rule_candidate_selector_contract(
            &spec.candidate_selector,
        ))?,
        positive_conditions: serde_json::to_value(role_rule_conditions_contract(
            &spec.positive_conditions,
        )?)?,
        negative_conditions: serde_json::to_value(role_rule_conditions_contract(
            &spec.negative_conditions,
        )?)?,
        provenance: json!({"source": "rule_preview"}),
    };
    evaluate_detection_rule_targets(facts, draft_rule)
}

fn evaluate_stored_rule_targets(
    facts: &[ArchitectureArtefactFact],
    rule: &ArchitectureRoleRuleRecord,
) -> Result<BTreeSet<RoleTarget>> {
    evaluate_detection_rule_targets(facts, detection_rule_from_management_record(rule)?)
}

fn evaluate_detection_rule_targets(
    facts: &[ArchitectureArtefactFact],
    rule: ArchitectureRoleDetectionRule,
) -> Result<BTreeSet<RoleTarget>> {
    let compiled = compile_detection_rules(vec![rule])?;
    let evaluated = evaluate_rules_over_facts(&compiled, facts)?;
    let mut positive_targets = BTreeSet::new();
    let mut negative_targets = BTreeSet::new();
    for signal in evaluated.signals {
        match signal.polarity {
            RoleSignalPolarity::Positive => {
                positive_targets.insert(signal.target);
            }
            RoleSignalPolarity::Negative => {
                negative_targets.insert(signal.target);
            }
        }
    }
    Ok(positive_targets
        .difference(&negative_targets)
        .cloned()
        .collect())
}

fn detection_rule_from_management_record(
    rule: &ArchitectureRoleRuleRecord,
) -> Result<ArchitectureRoleDetectionRule> {
    let score = parse_rule_score(&rule.score)?;
    Ok(ArchitectureRoleDetectionRule {
        repo_id: rule.repo_id.clone(),
        rule_id: rule.rule_id.clone(),
        role_id: rule.role_id.clone(),
        version: i64::try_from(rule.version).context("converting rule version")?,
        lifecycle: role_rule_lifecycle_from_management(&rule.lifecycle_status)?,
        priority: score.priority_hint.unwrap_or(100),
        score: score.base_confidence.unwrap_or(0.80),
        min_positive_ratio: score.min_positive_ratio.unwrap_or(1.0),
        candidate_selector: rule.candidate_selector.clone(),
        positive_conditions: rule.positive_conditions.clone(),
        negative_conditions: rule.negative_conditions.clone(),
        provenance: rule.provenance.clone(),
    })
}

fn role_rule_lifecycle_from_management(value: &str) -> Result<RoleRuleLifecycle> {
    match value {
        "draft" => Ok(RoleRuleLifecycle::Draft),
        "active" => Ok(RoleRuleLifecycle::Active),
        "disabled" => Ok(RoleRuleLifecycle::Disabled),
        "deprecated" => Ok(RoleRuleLifecycle::Deprecated),
        other => bail!("unsupported rule lifecycle status `{other}`"),
    }
}

fn target_keys(targets: &BTreeSet<RoleTarget>) -> BTreeSet<String> {
    targets.iter().map(preview_target_key).collect()
}

fn preview_target_key(target: &RoleTarget) -> String {
    match target.target_kind {
        TargetKind::File => format!("file:{}", target.path),
        TargetKind::Artefact => target
            .artefact_id
            .clone()
            .map(|artefact_id| format!("artefact:{artefact_id}"))
            .unwrap_or_else(|| format!("artefact:{}", target.path)),
        TargetKind::Symbol => target
            .symbol_id
            .as_ref()
            .map(|symbol_id| format!("symbol:{symbol_id}"))
            .unwrap_or_else(|| format!("symbol:{}", target.path)),
    }
}

fn preview_affected_artefact_id(target: &RoleTarget) -> Option<String> {
    target.artefact_id.clone()
}

fn rule_preview_safety(
    total_targets: usize,
    new_matches: &BTreeSet<String>,
    matched_paths: &BTreeSet<String>,
    spec: &RuleSpecFile,
) -> Value {
    let matched_targets = new_matches.len();
    let match_ratio = if total_targets == 0 {
        0.0
    } else {
        matched_targets as f64 / total_targets as f64
    };
    let max_match_ratio_without_override = 0.20;
    let uses_target_kinds = !spec.candidate_selector.target_kinds.is_empty();
    let positive_condition_count = spec.positive_conditions.len();
    let negative_condition_count = spec.negative_conditions.len();
    let path_only = positive_condition_count > 0
        && spec
            .positive_conditions
            .iter()
            .all(rule_condition_is_path_only);
    let narrow_path_prefix = spec
        .candidate_selector
        .path_prefixes
        .iter()
        .any(|prefix| prefix.split('/').filter(|part| !part.is_empty()).count() >= 2);
    let exact_path_condition = spec
        .candidate_selector
        .required_facts
        .iter()
        .chain(spec.positive_conditions.iter())
        .any(rule_condition_is_exact_path);
    let matches_test_generated_or_vendor = negative_condition_count == 0
        && matched_paths
            .iter()
            .any(|path| path_has_test_generated_or_vendor_segment(path));

    let mut blocking_reasons = Vec::new();
    let mut warnings = Vec::new();
    if matched_targets == 0 {
        blocking_reasons.push("zero_matches");
    }
    if match_ratio > max_match_ratio_without_override
        && !narrow_path_prefix
        && !exact_path_condition
    {
        blocking_reasons.push("broad_match_ratio");
    }
    if !uses_target_kinds {
        blocking_reasons.push("missing_target_kinds");
    }
    if path_only && matched_targets > 1 {
        blocking_reasons.push("path_only_multiple_matches");
    }
    if matches_test_generated_or_vendor {
        blocking_reasons.push("unbounded_test_generated_or_vendor_matches");
    }
    if path_only {
        warnings.push("path_only_rule");
    }
    let diagnostics = if matched_targets == 0 {
        json!({
            "target_count": total_targets,
            "matched_targets": matched_targets,
            "candidate_selector_target_kinds": spec.candidate_selector.target_kinds,
            "required_fact_group_count": spec.candidate_selector.required_fact_any_groups.len(),
            "positive_condition_count": positive_condition_count,
            "negative_condition_count": negative_condition_count,
        })
    } else {
        Value::Null
    };

    json!({
        "status": if blocking_reasons.is_empty() { "safe" } else { "blocked" },
        "blocking_reasons": blocking_reasons,
        "warnings": warnings,
        "matched_targets": matched_targets,
        "total_targets": total_targets,
        "match_ratio": match_ratio,
        "max_match_ratio_without_override": max_match_ratio_without_override,
        "path_only": path_only,
        "uses_target_kinds": uses_target_kinds,
        "exact_path_condition": exact_path_condition,
        "positive_condition_count": positive_condition_count,
        "negative_condition_count": negative_condition_count,
        "diagnostics": diagnostics,
        "conflicting_active_assignments": 0,
        "conflicting_roles": [],
        "protected_authoritative_collisions": 0
    })
}

fn rule_condition_is_path_only(condition: &RoleRuleCondition) -> bool {
    if let Some(key) = condition.key.as_deref() {
        return condition.kind == "path" && matches!(key, "full" | "segment" | "extension");
    }
    matches!(
        condition.kind.as_str(),
        "path_contains" | "path_equals" | "path_prefix" | "path_suffix"
    )
}

fn rule_condition_is_exact_path(condition: &RoleRuleCondition) -> bool {
    if let Some(key) = condition.key.as_deref() {
        return condition.kind == "path"
            && key == "full"
            && condition.op
                == Some(
                    crate::capability_packs::architecture_graph::roles::taxonomy::RoleFactConditionOp::Eq,
                );
    }
    condition.kind == "path_equals"
}

fn path_has_test_generated_or_vendor_segment(path: &str) -> bool {
    path.split('/').any(|segment| {
        matches!(
            segment,
            "test" | "tests" | "__tests__" | "generated" | "vendor" | "vendors"
        )
    })
}
