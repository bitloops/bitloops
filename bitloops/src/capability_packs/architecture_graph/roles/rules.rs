pub use super::taxonomy::{RoleRuleCandidateSelector, RoleRuleCondition, RoleRuleScore};

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde_json::Value;

use super::taxonomy::{
    ArchitectureArtefactFact, ArchitectureRoleDetectionRule, ArchitectureRoleRuleSignal,
    RoleCandidateSelector, RoleFactCondition, RoleFactConditionOp, RoleSignalPolarity, RoleTarget,
    parse_rule_conditions, parse_rule_selector, role_rule_candidate_selector_contract,
    role_rule_conditions_contract, rule_signal_id,
};

#[derive(Debug, Clone)]
pub struct CompiledArchitectureRoleRule {
    pub rule: ArchitectureRoleDetectionRule,
    pub selector: RoleCandidateSelector,
    pub positive_conditions: Vec<RoleFactCondition>,
    pub negative_conditions: Vec<RoleFactCondition>,
}

#[derive(Debug, Clone, Default)]
pub struct RuleEvaluationResult {
    pub signals: Vec<ArchitectureRoleRuleSignal>,
}

pub fn compile_detection_rules(
    rules: Vec<ArchitectureRoleDetectionRule>,
) -> Result<Vec<CompiledArchitectureRoleRule>> {
    rules
        .into_iter()
        .map(|rule| {
            let selector = parse_candidate_selector_contract(&rule)
                .with_context(|| format!("parsing candidate selector for rule {}", rule.rule_id))?;
            let positive_conditions = parse_conditions_contract(&rule.positive_conditions)
                .with_context(|| {
                    format!("parsing positive conditions for rule {}", rule.rule_id)
                })?;
            let negative_conditions = parse_conditions_contract(&rule.negative_conditions)
                .with_context(|| {
                    format!("parsing negative conditions for rule {}", rule.rule_id)
                })?;
            Ok(CompiledArchitectureRoleRule {
                rule,
                selector,
                positive_conditions,
                negative_conditions,
            })
        })
        .collect()
}

fn parse_candidate_selector_contract(
    rule: &ArchitectureRoleDetectionRule,
) -> Result<RoleCandidateSelector> {
    parse_rule_selector(&rule.candidate_selector)
        .map(|selector| role_rule_candidate_selector_contract(&selector))
        .or_else(|_| -> Result<RoleCandidateSelector> {
            serde_json::from_value::<RoleCandidateSelector>(rule.candidate_selector.clone())
                .context("parse fact-backed role rule selector")
        })
}

fn parse_conditions_contract(value: &Value) -> Result<Vec<RoleFactCondition>> {
    serde_json::from_value::<Vec<RoleFactCondition>>(value.clone()).or_else(|_| {
        parse_rule_conditions(value)
            .and_then(|conditions| role_rule_conditions_contract(&conditions))
    })
}

pub fn evaluate_rules_over_facts(
    rules: &[CompiledArchitectureRoleRule],
    facts: &[ArchitectureArtefactFact],
) -> Result<RuleEvaluationResult> {
    let facts_by_target = group_facts_by_target(facts);
    let mut signals = Vec::new();

    for rule in rules {
        for (target, target_facts) in &facts_by_target {
            if !selector_matches(&rule.selector, target, target_facts)? {
                continue;
            }

            let positive_match = matched_facts(&rule.positive_conditions, target_facts)?;
            if positive_match.score > 0.0 {
                signals.push(signal_for(
                    rule,
                    target,
                    RoleSignalPolarity::Positive,
                    positive_match.score,
                    &positive_match.facts,
                ));
            }

            let negative_match = matched_facts(&rule.negative_conditions, target_facts)?;
            if negative_match.score > 0.0 {
                signals.push(signal_for(
                    rule,
                    target,
                    RoleSignalPolarity::Negative,
                    negative_match.score,
                    &negative_match.facts,
                ));
            }
        }
    }

    Ok(RuleEvaluationResult { signals })
}

fn group_facts_by_target(
    facts: &[ArchitectureArtefactFact],
) -> BTreeMap<RoleTarget, Vec<ArchitectureArtefactFact>> {
    let mut grouped = BTreeMap::new();
    for fact in facts {
        grouped
            .entry(fact.target.clone())
            .or_insert_with(Vec::new)
            .push(fact.clone());
    }
    grouped
}

fn selector_matches(
    selector: &RoleCandidateSelector,
    target: &RoleTarget,
    facts: &[ArchitectureArtefactFact],
) -> Result<bool> {
    if !selector.target_kinds.is_empty() && !selector.target_kinds.contains(&target.target_kind) {
        return Ok(false);
    }
    if !selector.path_prefixes.is_empty()
        && !selector
            .path_prefixes
            .iter()
            .any(|prefix| target.path.starts_with(prefix))
    {
        return Ok(false);
    }
    if !selector.path_suffixes.is_empty()
        && !selector
            .path_suffixes
            .iter()
            .any(|suffix| target.path.ends_with(suffix))
    {
        return Ok(false);
    }
    for required in &selector.required_facts {
        if !any_condition_matches(required, facts)? {
            return Ok(false);
        }
    }
    for group in &selector.required_fact_any_groups {
        if group.is_empty() {
            continue;
        }
        let mut matched = false;
        for condition in group {
            if any_condition_matches(condition, facts)? {
                matched = true;
                break;
            }
        }
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, Default)]
struct MatchedRoleFacts {
    score: f64,
    facts: Vec<ArchitectureArtefactFact>,
}

fn matched_facts(
    conditions: &[RoleFactCondition],
    facts: &[ArchitectureArtefactFact],
) -> Result<MatchedRoleFacts> {
    let mut matched = MatchedRoleFacts::default();
    for condition in conditions {
        if let Some(fact) = first_condition_match(condition, facts)? {
            matched.score += condition.score;
            matched.facts.push(fact);
        }
    }
    Ok(matched)
}

fn any_condition_matches(
    condition: &RoleFactCondition,
    facts: &[ArchitectureArtefactFact],
) -> Result<bool> {
    Ok(first_condition_match(condition, facts)?.is_some())
}

fn first_condition_match(
    condition: &RoleFactCondition,
    facts: &[ArchitectureArtefactFact],
) -> Result<Option<ArchitectureArtefactFact>> {
    for fact in facts {
        if condition_matches(condition, fact)? {
            return Ok(Some(fact.clone()));
        }
    }
    Ok(None)
}

fn condition_matches(
    condition: &RoleFactCondition,
    fact: &ArchitectureArtefactFact,
) -> Result<bool> {
    if fact.fact_kind != condition.kind || fact.fact_key != condition.key {
        return Ok(false);
    }
    match condition.op {
        RoleFactConditionOp::Eq => Ok(fact.fact_value == condition.value),
        RoleFactConditionOp::Contains => Ok(fact.fact_value.contains(&condition.value)),
        RoleFactConditionOp::Prefix => Ok(fact.fact_value.starts_with(&condition.value)),
        RoleFactConditionOp::Suffix => Ok(fact.fact_value.ends_with(&condition.value)),
        RoleFactConditionOp::Gte => {
            compare_numeric(&fact.fact_value, &condition.value, |left, right| {
                left >= right
            })
        }
        RoleFactConditionOp::Lte => {
            compare_numeric(&fact.fact_value, &condition.value, |left, right| {
                left <= right
            })
        }
    }
}

fn compare_numeric(
    left: &str,
    right: &str,
    predicate: impl FnOnce(f64, f64) -> bool,
) -> Result<bool> {
    let left = left
        .parse::<f64>()
        .with_context(|| format!("parsing numeric fact value `{left}`"))?;
    let right = right
        .parse::<f64>()
        .with_context(|| format!("parsing numeric rule value `{right}`"))?;
    Ok(predicate(left, right))
}

fn signal_for(
    rule: &CompiledArchitectureRoleRule,
    target: &RoleTarget,
    polarity: RoleSignalPolarity,
    matched_score: f64,
    facts: &[ArchitectureArtefactFact],
) -> ArchitectureRoleRuleSignal {
    let score = (rule.rule.score * matched_score).clamp(0.0, 1.0);
    ArchitectureRoleRuleSignal {
        repo_id: rule.rule.repo_id.clone(),
        signal_id: rule_signal_id(
            &rule.rule.repo_id,
            &rule.rule.rule_id,
            rule.rule.version,
            &rule.rule.role_id,
            target,
            polarity,
        ),
        rule_id: rule.rule.rule_id.clone(),
        rule_version: rule.rule.version,
        role_id: rule.rule.role_id.clone(),
        target: target.clone(),
        polarity,
        score,
        evidence: Value::Array(
            facts
                .iter()
                .map(|fact| {
                    serde_json::json!({
                        "factId": fact.fact_id,
                        "kind": fact.fact_kind,
                        "key": fact.fact_key,
                        "value": fact.fact_value,
                    })
                })
                .collect(),
        ),
        generation_seq: facts
            .iter()
            .map(|fact| fact.generation_seq)
            .max()
            .map_or(0, |generation_seq| generation_seq),
    }
}

#[cfg(test)]
mod tests {
    use super::super::taxonomy::{
        ArchitectureArtefactFact, ArchitectureRoleDetectionRule, RoleRuleLifecycle,
        RoleSignalPolarity, RoleTarget,
    };
    use super::*;

    #[test]
    fn rule_evaluation_emits_positive_and_negative_signals() -> anyhow::Result<()> {
        let target = RoleTarget::file("bitloops/src/cli/main.rs");
        let facts = vec![
            ArchitectureArtefactFact {
                repo_id: "repo-1".to_string(),
                fact_id: "fact-1".to_string(),
                target: target.clone(),
                language: Some("rust".to_string()),
                fact_kind: "path".to_string(),
                fact_key: "segment".to_string(),
                fact_value: "cli".to_string(),
                source: "test".to_string(),
                confidence: 1.0,
                evidence: serde_json::json!([]),
                generation_seq: 1,
            },
            ArchitectureArtefactFact {
                repo_id: "repo-1".to_string(),
                fact_id: "fact-2".to_string(),
                target,
                language: Some("rust".to_string()),
                fact_kind: "path".to_string(),
                fact_key: "segment".to_string(),
                fact_value: "tests".to_string(),
                source: "test".to_string(),
                confidence: 1.0,
                evidence: serde_json::json!([]),
                generation_seq: 1,
            },
        ];
        let rules = compile_detection_rules(vec![ArchitectureRoleDetectionRule {
            repo_id: "repo-1".to_string(),
            rule_id: "rule-1".to_string(),
            role_id: "role-1".to_string(),
            version: 1,
            lifecycle: RoleRuleLifecycle::Active,
            priority: 10,
            score: 1.0,
            candidate_selector: serde_json::json!({ "targetKinds": ["file"] }),
            positive_conditions: serde_json::json!([
                { "kind": "path", "key": "segment", "op": "eq", "value": "cli", "score": 0.7 }
            ]),
            negative_conditions: serde_json::json!([
                { "kind": "path", "key": "segment", "op": "eq", "value": "tests", "score": 0.4 }
            ]),
            provenance: serde_json::json!({ "source": "test" }),
        }])?;

        let result = evaluate_rules_over_facts(&rules, &facts)?;
        assert_eq!(result.signals.len(), 2);
        assert!(
            result
                .signals
                .iter()
                .any(|signal| signal.polarity == RoleSignalPolarity::Positive)
        );
        assert!(
            result
                .signals
                .iter()
                .any(|signal| signal.polarity == RoleSignalPolarity::Negative)
        );
        Ok(())
    }

    #[test]
    fn rule_signal_evidence_contains_only_matched_facts() -> anyhow::Result<()> {
        let target = RoleTarget::file("bitloops/src/cli/main.rs");
        let facts = vec![
            ArchitectureArtefactFact {
                repo_id: "repo-1".to_string(),
                fact_id: "fact-1".to_string(),
                target: target.clone(),
                language: Some("rust".to_string()),
                fact_kind: "path".to_string(),
                fact_key: "segment".to_string(),
                fact_value: "cli".to_string(),
                source: "test".to_string(),
                confidence: 1.0,
                evidence: serde_json::json!([]),
                generation_seq: 1,
            },
            ArchitectureArtefactFact {
                repo_id: "repo-1".to_string(),
                fact_id: "fact-2".to_string(),
                target,
                language: Some("rust".to_string()),
                fact_kind: "path".to_string(),
                fact_key: "segment".to_string(),
                fact_value: "tests".to_string(),
                source: "test".to_string(),
                confidence: 1.0,
                evidence: serde_json::json!([]),
                generation_seq: 9,
            },
        ];
        let rules = compile_detection_rules(vec![ArchitectureRoleDetectionRule {
            repo_id: "repo-1".to_string(),
            rule_id: "rule-1".to_string(),
            role_id: "role-1".to_string(),
            version: 1,
            lifecycle: RoleRuleLifecycle::Active,
            priority: 10,
            score: 1.0,
            candidate_selector: serde_json::json!({ "targetKinds": ["file"] }),
            positive_conditions: serde_json::json!([
                { "kind": "path", "key": "segment", "op": "eq", "value": "cli", "score": 0.7 }
            ]),
            negative_conditions: serde_json::json!([]),
            provenance: serde_json::json!({ "source": "test" }),
        }])?;

        let result = evaluate_rules_over_facts(&rules, &facts)?;
        assert_eq!(result.signals.len(), 1);
        let signal = &result.signals[0];
        assert_eq!(signal.polarity, RoleSignalPolarity::Positive);
        assert_eq!(signal.generation_seq, 1);

        let evidence_fact_ids: Vec<&str> = signal
            .evidence
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|fact| fact.get("factId").and_then(serde_json::Value::as_str))
            .collect();
        assert_eq!(evidence_fact_ids, vec!["fact-1"]);
        assert!(!evidence_fact_ids.contains(&"fact-2"));
        Ok(())
    }

    #[test]
    fn compile_detection_rules_accepts_legacy_rule_management_contract() -> anyhow::Result<()> {
        let rules = compile_detection_rules(vec![ArchitectureRoleDetectionRule {
            repo_id: "repo-1".to_string(),
            rule_id: "rule-1".to_string(),
            role_id: "role-1".to_string(),
            version: 1,
            lifecycle: RoleRuleLifecycle::Active,
            priority: 10,
            score: 1.0,
            candidate_selector: serde_json::json!({
                "path_prefixes": ["src/cli"],
                "languages": ["rust"]
            }),
            positive_conditions: serde_json::json!([
                { "kind": "path_contains", "value": "commands" }
            ]),
            negative_conditions: serde_json::json!([]),
            provenance: serde_json::json!({ "source": "test" }),
        }])?;

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.path_prefixes, vec!["src/cli"]);
        assert_eq!(rules[0].selector.required_facts.len(), 1);
        assert_eq!(rules[0].selector.required_facts[0].kind, "language");
        assert_eq!(rules[0].selector.required_facts[0].key, "resolved");
        assert_eq!(rules[0].positive_conditions.len(), 1);
        assert_eq!(rules[0].positive_conditions[0].kind, "path");
        assert_eq!(rules[0].positive_conditions[0].key, "full");
        Ok(())
    }

    #[test]
    fn compile_detection_rules_preserves_fact_backed_selector_contract() -> anyhow::Result<()> {
        let file_target = RoleTarget::file("src/cli/main.rs");
        let symbol_target = RoleTarget::symbol("artefact-1", "symbol-1", "src/cli/main.rs");
        let facts = vec![
            ArchitectureArtefactFact {
                repo_id: "repo-1".to_string(),
                fact_id: "fact-1".to_string(),
                target: file_target,
                language: Some("rust".to_string()),
                fact_kind: "path".to_string(),
                fact_key: "segment".to_string(),
                fact_value: "cli".to_string(),
                source: "test".to_string(),
                confidence: 1.0,
                evidence: serde_json::json!([]),
                generation_seq: 1,
            },
            ArchitectureArtefactFact {
                repo_id: "repo-1".to_string(),
                fact_id: "fact-2".to_string(),
                target: symbol_target,
                language: Some("rust".to_string()),
                fact_kind: "path".to_string(),
                fact_key: "segment".to_string(),
                fact_value: "cli".to_string(),
                source: "test".to_string(),
                confidence: 1.0,
                evidence: serde_json::json!([]),
                generation_seq: 1,
            },
        ];
        let rules = compile_detection_rules(vec![ArchitectureRoleDetectionRule {
            repo_id: "repo-1".to_string(),
            rule_id: "rule-1".to_string(),
            role_id: "role-1".to_string(),
            version: 1,
            lifecycle: RoleRuleLifecycle::Active,
            priority: 10,
            score: 1.0,
            candidate_selector: serde_json::json!({
                "targetKinds": ["file"],
                "requiredFacts": [
                    { "kind": "path", "key": "segment", "op": "eq", "value": "cli" }
                ]
            }),
            positive_conditions: serde_json::json!([
                { "kind": "path", "key": "segment", "op": "eq", "value": "cli", "score": 0.8 }
            ]),
            negative_conditions: serde_json::json!([]),
            provenance: serde_json::json!({ "source": "test" }),
        }])?;

        let result = evaluate_rules_over_facts(&rules, &facts)?;

        assert_eq!(rules[0].selector.target_kinds.len(), 1);
        assert_eq!(rules[0].selector.required_facts.len(), 1);
        assert_eq!(result.signals.len(), 1);
        assert_eq!(result.signals[0].target.target_kind.as_db(), "file");
        Ok(())
    }

    #[test]
    fn compile_detection_rules_preserves_fact_condition_scores_and_numeric_ops()
    -> anyhow::Result<()> {
        let target = RoleTarget::file("src/cli/main.rs");
        let facts = vec![ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: "fact-1".to_string(),
            target,
            language: Some("rust".to_string()),
            fact_kind: "metrics".to_string(),
            fact_key: "fan_in".to_string(),
            fact_value: "12".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: serde_json::json!([]),
            generation_seq: 1,
        }];
        let rules = compile_detection_rules(vec![ArchitectureRoleDetectionRule {
            repo_id: "repo-1".to_string(),
            rule_id: "rule-1".to_string(),
            role_id: "role-1".to_string(),
            version: 1,
            lifecycle: RoleRuleLifecycle::Active,
            priority: 10,
            score: 1.0,
            candidate_selector: serde_json::json!({ "targetKinds": ["file"] }),
            positive_conditions: serde_json::json!([
                { "kind": "metrics", "key": "fan_in", "op": "gte", "value": "10", "score": 0.5 }
            ]),
            negative_conditions: serde_json::json!([]),
            provenance: serde_json::json!({ "source": "test" }),
        }])?;

        let result = evaluate_rules_over_facts(&rules, &facts)?;

        assert_eq!(rules[0].positive_conditions[0].score, 0.5);
        assert_eq!(result.signals.len(), 1);
        assert_eq!(result.signals[0].score, 0.5);
        Ok(())
    }
}
