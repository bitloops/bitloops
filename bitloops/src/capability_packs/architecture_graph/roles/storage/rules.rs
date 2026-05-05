use anyhow::{Context, Result};

use crate::host::devql::{RelationalStorage, sql_json_value, sql_now};

use super::rows::{detection_rule_from_row, sql_text};
use crate::capability_packs::architecture_graph::roles::taxonomy::ArchitectureRoleDetectionRule;

pub async fn upsert_detection_rule(
    relational: &RelationalStorage,
    rule: &ArchitectureRoleDetectionRule,
) -> Result<()> {
    // TODO(CLI-1797): Seed command should call this after validating LLM-proposed rule candidates.
    let sql = format!(
        "INSERT INTO architecture_role_detection_rules (
            repo_id, rule_id, role_id, version, lifecycle, priority, score,
            candidate_selector_json, positive_conditions_json, negative_conditions_json,
            provenance_json, updated_at
         ) VALUES (
            {repo_id}, {rule_id}, {role_id}, {version}, {lifecycle}, {priority}, {score},
            {candidate_selector}, {positive_conditions}, {negative_conditions},
            {provenance}, {now}
         )
         ON CONFLICT(repo_id, rule_id, version) DO UPDATE SET
            lifecycle = excluded.lifecycle,
            priority = excluded.priority,
            score = excluded.score,
            candidate_selector_json = excluded.candidate_selector_json,
            positive_conditions_json = excluded.positive_conditions_json,
            negative_conditions_json = excluded.negative_conditions_json,
            provenance_json = excluded.provenance_json,
            updated_at = {now};",
        repo_id = sql_text(&rule.repo_id),
        rule_id = sql_text(&rule.rule_id),
        role_id = sql_text(&rule.role_id),
        version = rule.version,
        lifecycle = sql_text(rule.lifecycle.as_db()),
        priority = rule.priority,
        score = rule.score,
        candidate_selector = sql_json_value(relational, &rule.candidate_selector),
        positive_conditions = sql_json_value(relational, &rule.positive_conditions),
        negative_conditions = sql_json_value(relational, &rule.negative_conditions),
        provenance = sql_json_value(relational, &rule.provenance),
        now = sql_now(relational),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("upserting architecture role detection rule")
}

pub async fn load_active_detection_rules(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<ArchitectureRoleDetectionRule>> {
    let sql = format!(
        "SELECT repo_id, rule_id, role_id, version, lifecycle, priority, score,
                candidate_selector_json, positive_conditions_json, negative_conditions_json,
                provenance_json
         FROM architecture_role_detection_rules AS rule
         WHERE rule.repo_id = {} AND rule.lifecycle = 'active'
           AND rule.version = (
               SELECT MAX(candidate.version)
               FROM architecture_role_detection_rules AS candidate
               WHERE candidate.repo_id = rule.repo_id
                 AND candidate.rule_id = rule.rule_id
                 AND candidate.lifecycle = 'active'
           )
         ORDER BY priority ASC, rule_id ASC, version DESC",
        sql_text(repo_id)
    );
    relational
        .query_rows(&sql)
        .await
        .context("loading active architecture role detection rules")?
        .into_iter()
        .map(detection_rule_from_row)
        .collect()
}
