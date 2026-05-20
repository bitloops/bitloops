use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use crate::capability_packs::architecture_graph::roles::llm_adjudication::{
    architecture_roles_seed_roles_user_prompt, architecture_roles_seed_rules_user_prompt,
};
use crate::capability_packs::architecture_graph::roles::taxonomy::SeededArchitectureRole;
use crate::devql_transport::SlimCliRepoScope;

pub(crate) const ROLE_DISCOVERY_USER_PROMPT_BUDGET_BYTES: usize = 64 * 1024;
pub(crate) const RULE_GENERATION_USER_PROMPT_BUDGET_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeedEvidenceBudgetReport {
    pub(crate) prompt_budget_bytes: usize,
    pub(crate) original_counts: Map<String, Value>,
    pub(crate) included_counts: Map<String, Value>,
    pub(crate) omitted_counts: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BudgetedSeedEvidence {
    evidence: Value,
    report: SeedEvidenceBudgetReport,
}

impl BudgetedSeedEvidence {
    pub(crate) fn evidence(&self) -> &Value {
        &self.evidence
    }

    pub(crate) fn report(&self) -> &SeedEvidenceBudgetReport {
        &self.report
    }

    #[cfg(test)]
    pub(crate) fn included_count(&self, section: &str) -> usize {
        count_from_map(&self.report.included_counts, section)
    }

    #[cfg(test)]
    pub(crate) fn omitted_total(&self) -> usize {
        self.report
            .omitted_counts
            .values()
            .filter_map(Value::as_u64)
            .map(|value| value as usize)
            .sum()
    }
}

pub(crate) fn budget_role_discovery_evidence(
    scope: &SlimCliRepoScope,
    full_evidence: &Value,
) -> Result<BudgetedSeedEvidence> {
    budget_seed_evidence(
        full_evidence,
        ROLE_DISCOVERY_USER_PROMPT_BUDGET_BYTES,
        |candidate| architecture_roles_seed_roles_user_prompt(scope, candidate),
        None,
    )
}

pub(crate) fn budget_rule_generation_evidence(
    scope: &SlimCliRepoScope,
    full_evidence: &Value,
    roles: &[SeededArchitectureRole],
) -> Result<BudgetedSeedEvidence> {
    budget_seed_evidence(
        full_evidence,
        RULE_GENERATION_USER_PROMPT_BUDGET_BYTES,
        |candidate| architecture_roles_seed_rules_user_prompt(scope, candidate, roles),
        Some(roles),
    )
}

const ARRAY_SECTIONS: [&str; 6] = [
    "language_framework_signals",
    "canonical_files",
    "canonical_artefacts",
    "artefact_summaries",
    "dependency_graph_hints",
    "existing_architecture_graph_facts",
];

fn count_from_map(map: &Map<String, Value>, section: &str) -> usize {
    map.get(section)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(0)
}

fn array_section<'a>(evidence: &'a Value, section: &str) -> &'a [Value] {
    evidence
        .get(section)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn original_counts(evidence: &Value) -> Map<String, Value> {
    ARRAY_SECTIONS
        .iter()
        .map(|section| {
            (
                (*section).to_string(),
                Value::Number(serde_json::Number::from(
                    array_section(evidence, section).len(),
                )),
            )
        })
        .collect()
}

fn included_counts(evidence: &Value) -> Map<String, Value> {
    original_counts(evidence)
}

fn omitted_counts(
    original: &Map<String, Value>,
    included: &Map<String, Value>,
) -> Map<String, Value> {
    ARRAY_SECTIONS
        .iter()
        .map(|section| {
            let original_count = count_from_map(original, section);
            let included_count = count_from_map(included, section);
            (
                (*section).to_string(),
                Value::Number(serde_json::Number::from(
                    original_count.saturating_sub(included_count),
                )),
            )
        })
        .collect()
}

#[derive(Debug, Clone)]
struct RankedItem {
    section: &'static str,
    source_index: usize,
    value: Value,
    score: i64,
    path_key: String,
}

fn role_terms(roles: Option<&[SeededArchitectureRole]>) -> Vec<String> {
    let mut terms = Vec::new();
    if let Some(roles) = roles {
        for role in roles {
            push_terms(&mut terms, &role.canonical_key);
            push_terms(&mut terms, &role.display_name);
            push_terms(&mut terms, &role.description);
            if let Some(family) = role.family.as_deref() {
                push_terms(&mut terms, family);
            }
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn push_terms(terms: &mut Vec<String>, text: &str) {
    for raw in text.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        let term = raw.trim().to_ascii_lowercase();
        if term.len() >= 4 {
            terms.push(term);
        }
    }
}

fn item_text(value: &Value) -> String {
    match value {
        Value::Object(map) => map
            .values()
            .map(item_text)
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase(),
        Value::Array(values) => values
            .iter()
            .map(item_text)
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase(),
        Value::String(text) => text.to_ascii_lowercase(),
        other => other.to_string().to_ascii_lowercase(),
    }
}

fn path_key(value: &Value) -> String {
    value
        .get("path")
        .and_then(Value::as_str)
        .map(|path| path.split('/').take(3).collect::<Vec<_>>().join("/"))
        .unwrap_or_default()
}

fn ranked_items(
    full_evidence: &Value,
    roles: Option<&[SeededArchitectureRole]>,
) -> Vec<RankedItem> {
    let terms = role_terms(roles);
    let mut items = Vec::new();
    for section in ARRAY_SECTIONS {
        for (source_index, value) in array_section(full_evidence, section).iter().enumerate() {
            let text = item_text(value);
            let mut score = section_base_score(section);
            score += role_match_score(&text, &terms);
            score += structure_signal_score(section, value);
            items.push(RankedItem {
                section,
                source_index,
                value: value.clone(),
                score,
                path_key: path_key(value),
            });
        }
    }
    items.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.section.cmp(right.section))
            .then_with(|| left.path_key.cmp(&right.path_key))
            .then_with(|| item_text(&left.value).cmp(&item_text(&right.value)))
    });
    items
}

fn section_base_score(section: &str) -> i64 {
    match section {
        "existing_architecture_graph_facts" => 90,
        "language_framework_signals" => 85,
        "canonical_files" => 80,
        "canonical_artefacts" => 70,
        "artefact_summaries" => 60,
        "dependency_graph_hints" => 30,
        _ => 0,
    }
}

fn role_match_score(text: &str, terms: &[String]) -> i64 {
    terms
        .iter()
        .filter(|term| text.contains(term.as_str()))
        .count() as i64
        * 40
}

fn structure_signal_score(section: &str, value: &Value) -> i64 {
    let text = item_text(value);
    let mut score = 0;
    if text.contains("entrypoint") || text.contains("main.rs") || text.contains("lib.rs") {
        score += 20;
    }
    if text.contains("struct") || text.contains("trait") || text.contains("module") {
        score += 10;
    }
    if section == "artefact_summaries" && text.len() > 120 {
        score += 10;
    }
    score
}

fn budget_seed_evidence<F>(
    full_evidence: &Value,
    prompt_budget_bytes: usize,
    render_prompt: F,
    roles: Option<&[SeededArchitectureRole]>,
) -> Result<BudgetedSeedEvidence>
where
    F: Fn(&Value) -> String,
{
    let original = original_counts(full_evidence);
    let mut candidate = seed_evidence_skeleton(full_evidence);
    let skeleton_report = report_for_candidate(prompt_budget_bytes, &original, &candidate);
    let skeleton_evidence = evidence_with_budget_report(&candidate, &skeleton_report)?;
    let skeleton_prompt_bytes = render_prompt(&skeleton_evidence).len();
    if skeleton_prompt_bytes > prompt_budget_bytes {
        return Err(anyhow!(
            "architecture role seed prompt skeleton is {} bytes, exceeding budget {} bytes before evidence arrays are added",
            skeleton_prompt_bytes,
            prompt_budget_bytes
        ));
    }

    let items = ranked_items(full_evidence, roles);
    let mut accepted = std::collections::BTreeSet::new();
    for section in ARRAY_SECTIONS {
        if let Some(item) = items.iter().find(|item| item.section == section)
            && try_append_item(
                &mut candidate,
                item,
                prompt_budget_bytes,
                &original,
                &render_prompt,
            )?
        {
            accepted.insert((item.section, item.source_index));
        }
    }

    for item in &items {
        if accepted.contains(&(item.section, item.source_index)) {
            continue;
        }
        if try_append_item(
            &mut candidate,
            item,
            prompt_budget_bytes,
            &original,
            &render_prompt,
        )? {
            accepted.insert((item.section, item.source_index));
        }
    }

    let report = report_for_candidate(prompt_budget_bytes, &original, &candidate);
    insert_budget_report(&mut candidate, &report)?;

    Ok(BudgetedSeedEvidence {
        evidence: candidate,
        report,
    })
}

fn try_append_item<F>(
    candidate: &mut Value,
    item: &RankedItem,
    prompt_budget_bytes: usize,
    original: &Map<String, Value>,
    render_prompt: &F,
) -> Result<bool>
where
    F: Fn(&Value) -> String,
{
    let mut next = candidate.clone();
    append_section_item(&mut next, item.section, item.value.clone())?;
    let next_report = report_for_candidate(prompt_budget_bytes, original, &next);
    let next_evidence = evidence_with_budget_report(&next, &next_report)?;
    if render_prompt(&next_evidence).len() <= prompt_budget_bytes {
        *candidate = next;
        return Ok(true);
    }
    Ok(false)
}

fn report_for_candidate(
    prompt_budget_bytes: usize,
    original: &Map<String, Value>,
    candidate: &Value,
) -> SeedEvidenceBudgetReport {
    let included = included_counts(candidate);
    let omitted = omitted_counts(original, &included);
    SeedEvidenceBudgetReport {
        prompt_budget_bytes,
        original_counts: original.clone(),
        included_counts: included,
        omitted_counts: omitted,
    }
}

fn evidence_with_budget_report(
    evidence: &Value,
    report: &SeedEvidenceBudgetReport,
) -> Result<Value> {
    let mut evidence = evidence.clone();
    insert_budget_report(&mut evidence, report)?;
    Ok(evidence)
}

fn seed_evidence_skeleton(full_evidence: &Value) -> Value {
    json!({
        "repository": full_evidence.get("repository").cloned().unwrap_or_else(|| json!({})),
        "language_framework_signals": [],
        "canonical_files": [],
        "canonical_artefacts": [],
        "artefact_summaries": [],
        "dependency_graph_hints": [],
        "existing_architecture_graph_facts": [],
        "generic_role_family_examples": full_evidence
            .get("generic_role_family_examples")
            .cloned()
            .unwrap_or_else(|| json!([]))
    })
}

fn append_section_item(evidence: &mut Value, section: &str, value: Value) -> Result<()> {
    let array = evidence
        .get_mut(section)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            anyhow!("budgeted architecture seed evidence section `{section}` is not an array")
        })?;
    array.push(value);
    Ok(())
}

fn insert_budget_report(evidence: &mut Value, report: &SeedEvidenceBudgetReport) -> Result<()> {
    let object = evidence
        .as_object_mut()
        .ok_or_else(|| anyhow!("budgeted architecture seed evidence root must be an object"))?;
    object.insert(
        "evidence_budget".to_string(),
        json!({
            "prompt_budget_bytes": report.prompt_budget_bytes,
            "original_counts": report.original_counts,
            "included_counts": report.included_counts,
            "omitted_counts": report.omitted_counts
        }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_scope() -> SlimCliRepoScope {
        SlimCliRepoScope {
            repo: crate::host::devql::RepoIdentity {
                repo_id: "repo-large".to_string(),
                provider: "git".to_string(),
                organization: "tokio-rs".to_string(),
                name: "tokio".to_string(),
                identity: "git/tokio-rs/tokio".to_string(),
            },
            repo_root: std::path::PathBuf::from("/tmp/tokio"),
            branch_name: "main".to_string(),
            project_path: None,
            git_dir_relative_path: ".git".to_string(),
            config_fingerprint: "fingerprint".to_string(),
        }
    }

    fn large_seed_evidence() -> Value {
        let files = (0..500)
            .map(|index| {
                json!({
                    "path": format!("tokio/src/runtime/worker_{index}.rs"),
                    "analysis_mode": "source",
                    "file_role": if index % 9 == 0 { "entrypoint" } else { "implementation" },
                    "language": "rust",
                    "resolved_language": "rust"
                })
            })
            .collect::<Vec<_>>();
        let artefacts = (0..600)
            .map(|index| {
                json!({
                    "artefact_id": format!("artefact-{index}"),
                    "path": format!("tokio/src/runtime/worker_{}.rs", index % 80),
                    "language": "rust",
                    "canonical_kind": if index % 11 == 0 { "struct" } else { "function" },
                    "language_kind": "function_item",
                    "symbol_fqn": format!("tokio::runtime::worker_{index}::poll_budget"),
                    "signature": format!("pub fn poll_budget_{index}(&self, cx: &mut Context<'_>) -> Poll<Result<(), RuntimeError>>"),
                    "docstring": "Runtime scheduling and cooperative budget enforcement. ".repeat(24)
                })
            })
            .collect::<Vec<_>>();
        let summaries = (0..600)
            .map(|index| {
                json!({
                    "artefact_id": format!("artefact-{index}"),
                    "path": format!("tokio/src/runtime/worker_{}.rs", index % 80),
                    "summary": "Coordinates runtime scheduling, IO driver state, cooperative task budgeting, and worker handoff behavior. ".repeat(18)
                })
            })
            .collect::<Vec<_>>();
        let edges = (0..700)
            .map(|index| {
                json!({
                    "edge_id": format!("edge-{index}"),
                    "path": format!("tokio/src/runtime/worker_{}.rs", index % 80),
                    "from_artefact_id": format!("artefact-{index}"),
                    "to_artefact_id": format!("artefact-{}", (index + 1) % 600),
                    "to_symbol_ref": format!("tokio::runtime::driver::Driver{}", index % 30),
                    "edge_kind": "calls",
                    "language": "rust"
                })
            })
            .collect::<Vec<_>>();
        let graph = (0..400)
            .map(|index| {
                json!({
                    "node_kind": if index % 7 == 0 { "entry_point" } else { "component" },
                    "label": format!("runtime worker graph node {index}"),
                    "artefact_id": format!("artefact-{index}"),
                    "path": format!("tokio/src/runtime/worker_{}.rs", index % 80),
                    "entry_kind": "runtime",
                    "confidence": 0.91
                })
            })
            .collect::<Vec<_>>();

        json!({
            "repository": {
                "repo_id": "repo-large",
                "provider": "git",
                "organization": "tokio-rs",
                "name": "tokio",
                "identity": "git/tokio-rs/tokio",
                "repo_root": "/tmp/tokio",
                "branch_name": "main",
                "project_path": null,
                "metadata": {}
            },
            "language_framework_signals": files,
            "canonical_files": files,
            "canonical_artefacts": artefacts,
            "artefact_summaries": summaries,
            "dependency_graph_hints": edges,
            "existing_architecture_graph_facts": graph,
            "generic_role_family_examples": [
                {"family": "entrypoint"},
                {"family": "runtime"},
                {"family": "infrastructure"}
            ]
        })
    }

    fn runtime_role() -> SeededArchitectureRole {
        serde_json::from_value(json!({
            "canonical_key": "runtime_scheduler",
            "display_name": "Runtime Scheduler",
            "description": "Coordinates runtime scheduling and cooperative task budgeting.",
            "family": "runtime",
            "lifecycle_status": "active",
            "provenance": {},
            "evidence": {}
        }))
        .expect("valid seeded role")
    }

    #[test]
    fn role_discovery_evidence_keeps_rendered_prompt_under_budget() {
        let scope = test_scope();
        let budgeted = budget_role_discovery_evidence(&scope, &large_seed_evidence())
            .expect("budget role discovery evidence");
        let prompt = architecture_roles_seed_roles_user_prompt(&scope, budgeted.evidence());

        assert!(
            prompt.len() <= ROLE_DISCOVERY_USER_PROMPT_BUDGET_BYTES,
            "prompt length {} exceeded budget {}",
            prompt.len(),
            ROLE_DISCOVERY_USER_PROMPT_BUDGET_BYTES
        );
        assert!(budgeted.omitted_total() > 0);
        assert!(budgeted.included_count("canonical_files") > 0);
        assert!(budgeted.included_count("canonical_artefacts") > 0);
        assert!(budgeted.included_count("artefact_summaries") > 0);
    }

    #[test]
    fn rule_generation_evidence_is_role_scoped_and_keeps_prompt_under_budget() {
        let scope = test_scope();
        let role = runtime_role();
        let budgeted = budget_rule_generation_evidence(
            &scope,
            &large_seed_evidence(),
            std::slice::from_ref(&role),
        )
        .expect("budget rule generation evidence");
        let prompt =
            architecture_roles_seed_rules_user_prompt(&scope, budgeted.evidence(), &[role]);

        assert!(
            prompt.len() <= RULE_GENERATION_USER_PROMPT_BUDGET_BYTES,
            "prompt length {} exceeded budget {}",
            prompt.len(),
            RULE_GENERATION_USER_PROMPT_BUDGET_BYTES
        );
        assert!(budgeted.omitted_total() > 0);
        assert!(prompt.contains("runtime"));
        assert!(prompt.contains("scheduler") || prompt.contains("budget"));
    }

    #[test]
    fn small_evidence_keeps_all_seed_sections() {
        let scope = test_scope();
        let evidence = json!({
            "repository": {"repo_id": "repo-small"},
            "language_framework_signals": [
                {"path": "src/main.rs", "resolved_language": "rust"}
            ],
            "canonical_files": [
                {"path": "src/main.rs", "language": "rust"}
            ],
            "canonical_artefacts": [
                {"artefact_id": "a1", "path": "src/main.rs", "canonical_kind": "function"}
            ],
            "artefact_summaries": [
                {"artefact_id": "a1", "path": "src/main.rs", "summary": "Starts the process."}
            ],
            "dependency_graph_hints": [
                {"edge_id": "e1", "path": "src/main.rs", "edge_kind": "calls"}
            ],
            "existing_architecture_graph_facts": [
                {"label": "main", "path": "src/main.rs"}
            ],
            "generic_role_family_examples": [
                {"family": "entrypoint"}
            ]
        });

        let budgeted = budget_role_discovery_evidence(&scope, &evidence)
            .expect("budget role discovery evidence");

        assert_eq!(budgeted.omitted_total(), 0);
        assert_eq!(budgeted.included_count("canonical_files"), 1);
        assert_eq!(budgeted.included_count("language_framework_signals"), 1);
        assert_eq!(budgeted.included_count("canonical_artefacts"), 1);
        assert_eq!(budgeted.included_count("artefact_summaries"), 1);
        assert_eq!(budgeted.included_count("dependency_graph_hints"), 1);
        assert_eq!(
            budgeted.included_count("existing_architecture_graph_facts"),
            1
        );
    }

    #[test]
    fn tokio_scale_prompt_shape_is_reduced_from_monolithic_seed_size() {
        let scope = test_scope();
        let full_evidence = large_seed_evidence();
        let unbudgeted_prompt = architecture_roles_seed_roles_user_prompt(&scope, &full_evidence);
        assert!(
            unbudgeted_prompt.len() > ROLE_DISCOVERY_USER_PROMPT_BUDGET_BYTES * 2,
            "fixture must exercise a large monolithic prompt; got {} bytes",
            unbudgeted_prompt.len()
        );

        let budgeted =
            budget_role_discovery_evidence(&scope, &full_evidence).expect("budget large evidence");
        let budgeted_prompt =
            architecture_roles_seed_roles_user_prompt(&scope, budgeted.evidence());

        assert!(budgeted_prompt.len() <= ROLE_DISCOVERY_USER_PROMPT_BUDGET_BYTES);
        assert!(budgeted.omitted_total() > 300);
    }
}
