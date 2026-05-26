use std::collections::{BTreeMap, BTreeSet};

use super::super::contracts::{
    AdjudicationReason, RoleAdjudicationRequest, placeholder_request_hash,
    role_adjudication_stable_request_key,
};
use super::super::taxonomy::{
    ArchitectureArtefactFact, ArchitectureRoleAssignment, RoleTarget, TargetKind,
};
use super::request_target_fields;

#[derive(Debug, Clone)]
pub(super) struct RoleTargetSummary {
    pub(super) target: RoleTarget,
    pub(super) language: Option<String>,
    pub(super) canonical_kind: Option<String>,
    pub(super) language_kind: Option<String>,
    pub(super) file_role: Option<String>,
    pub(super) analysis_mode: Option<String>,
    pub(super) symbol_name: Option<String>,
    pub(super) symbol_suffix: Option<String>,
    pub(super) dependency_kinds: BTreeSet<String>,
    pub(super) high_impact: bool,
}

#[derive(Debug, Default)]
pub(super) struct UnknownTargetPolicyOutcome {
    pub(super) adjudication_requests: Vec<RoleAdjudicationRequest>,
    pub(super) rule_mining_inputs: Vec<super::super::rule_mining::RoleRuleMiningClusterInput>,
    pub(super) unknown_targets_total: usize,
    pub(super) suppressed_non_role: usize,
    pub(super) rule_mining_eligible: usize,
    pub(super) adjudication_escalated: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnknownTargetDisposition {
    SuppressNonRole,
    RuleMiningCandidate,
    AdjudicateHighImpact,
}

pub(super) fn target_summaries_from_facts(
    facts: &[ArchitectureArtefactFact],
) -> BTreeMap<RoleTarget, RoleTargetSummary> {
    let mut summaries = BTreeMap::new();
    for fact in facts {
        let entry = summaries
            .entry(fact.target.clone())
            .or_insert_with(|| RoleTargetSummary {
                target: fact.target.clone(),
                language: fact.language.clone(),
                canonical_kind: None,
                language_kind: None,
                file_role: None,
                analysis_mode: None,
                symbol_name: None,
                symbol_suffix: None,
                dependency_kinds: BTreeSet::new(),
                high_impact: false,
            });
        if entry.language.is_none() {
            entry.language = fact.language.clone();
        }
        match (fact.fact_kind.as_str(), fact.fact_key.as_str()) {
            ("artefact", "canonical_kind") => {
                entry.canonical_kind = Some(fact.fact_value.clone());
            }
            ("artefact", "language_kind") => {
                entry.language_kind = Some(fact.fact_value.clone());
            }
            ("file", "role") => {
                entry.file_role = Some(fact.fact_value.clone());
            }
            ("file", "analysis_mode") => {
                entry.analysis_mode = Some(fact.fact_value.clone());
            }
            ("symbol", "name") => {
                entry.symbol_name = Some(fact.fact_value.clone());
                if fact.fact_value == "main" {
                    entry.high_impact = true;
                }
            }
            ("symbol", "name_suffix") => {
                entry.symbol_suffix = Some(fact.fact_value.clone());
            }
            ("dependency", "incoming_kind" | "outgoing_kind") => {
                entry.dependency_kinds.insert(fact.fact_value.clone());
            }
            ("path", "full")
                if entry.target.target_kind == TargetKind::File
                    && (fact.fact_value == "main.rs" || fact.fact_value.ends_with("/main.rs")) =>
            {
                entry.high_impact = true;
            }
            _ => {}
        }
    }
    summaries
}

pub(super) fn facts_by_target(
    facts: &[ArchitectureArtefactFact],
) -> BTreeMap<RoleTarget, Vec<ArchitectureArtefactFact>> {
    let mut grouped: BTreeMap<RoleTarget, Vec<ArchitectureArtefactFact>> = BTreeMap::new();
    for fact in facts {
        grouped
            .entry(fact.target.clone())
            .or_default()
            .push(fact.clone());
    }
    grouped
}

fn unknown_target_disposition(
    summary: &RoleTargetSummary,
    file_targets_by_path: &BTreeSet<String>,
) -> UnknownTargetDisposition {
    if high_impact_role_bearing_target(summary, file_targets_by_path) {
        return UnknownTargetDisposition::AdjudicateHighImpact;
    }
    if non_role_bearing_target(summary, file_targets_by_path) {
        return UnknownTargetDisposition::SuppressNonRole;
    }
    if rule_mining_role_bearing_target(summary) {
        return UnknownTargetDisposition::RuleMiningCandidate;
    }
    UnknownTargetDisposition::SuppressNonRole
}

fn high_impact_role_bearing_target(
    summary: &RoleTargetSummary,
    file_targets_by_path: &BTreeSet<String>,
) -> bool {
    if !summary.high_impact {
        return false;
    }
    if file_like_artefact_duplicate(summary, file_targets_by_path) {
        return false;
    }
    true
}

fn non_role_bearing_target(
    summary: &RoleTargetSummary,
    file_targets_by_path: &BTreeSet<String>,
) -> bool {
    if file_like_artefact_duplicate(summary, file_targets_by_path) {
        return true;
    }
    if import_artefact(summary) {
        return true;
    }
    if non_role_file(summary) {
        return true;
    }
    if path_suppressed_for_roles(&summary.target.path) {
        return true;
    }
    false
}

fn rule_mining_role_bearing_target(summary: &RoleTargetSummary) -> bool {
    match summary.target.target_kind {
        TargetKind::File => {
            summary.analysis_mode.as_deref() == Some("code")
                && matches!(
                    summary.file_role.as_deref(),
                    Some("source_code") | Some("source")
                )
                && source_like_language(summary.language.as_deref())
        }
        TargetKind::Artefact | TargetKind::Symbol => {
            !import_artefact(summary)
                && !file_like_kind(summary)
                && source_like_language(summary.language.as_deref())
        }
    }
}

fn file_like_artefact_duplicate(
    summary: &RoleTargetSummary,
    file_targets_by_path: &BTreeSet<String>,
) -> bool {
    summary.target.target_kind == TargetKind::Artefact
        && file_targets_by_path.contains(&summary.target.path)
        && file_like_kind(summary)
}

fn file_like_kind(summary: &RoleTargetSummary) -> bool {
    matches!(
        lower_opt(summary.canonical_kind.as_deref()).as_deref(),
        Some("file" | "source_file" | "module")
    ) || matches!(
        lower_opt(summary.language_kind.as_deref()).as_deref(),
        Some("file" | "source_file" | "module")
    )
}

fn import_artefact(summary: &RoleTargetSummary) -> bool {
    matches!(
        lower_opt(summary.canonical_kind.as_deref()).as_deref(),
        Some("import")
    ) || matches!(
        lower_opt(summary.language_kind.as_deref()).as_deref(),
        Some("import" | "import_declaration" | "use_declaration")
    )
}

fn non_role_file(summary: &RoleTargetSummary) -> bool {
    if summary.target.target_kind != TargetKind::File {
        return false;
    }
    if summary.analysis_mode.as_deref() != Some("code") {
        return true;
    }
    matches!(
        lower_opt(summary.file_role.as_deref()).as_deref(),
        Some(
            "documentation"
                | "configuration"
                | "lockfile"
                | "project_manifest"
                | "context_seed"
                | "generated"
                | "dependency_tree"
                | "other"
        )
    )
}

pub(super) fn path_suppressed_for_roles(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let file_name = lower.rsplit('/').next().unwrap_or(lower.as_str());
    let extension = file_name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    if matches!(
        file_name,
        "cargo.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "poetry.lock"
            | "pipfile.lock"
    ) || file_name.ends_with(".lock")
    {
        return true;
    }
    if file_name.starts_with('.') {
        return true;
    }
    if matches!(
        extension,
        "md" | "mdx"
            | "txt"
            | "rst"
            | "adoc"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "jsonc"
            | "ini"
            | "cfg"
            | "conf"
    ) {
        return true;
    }
    lower.split('/').any(|segment| {
        matches!(
            segment,
            "target"
                | "node_modules"
                | "vendor"
                | "vendors"
                | "generated"
                | "dist"
                | "build"
                | "coverage"
                | ".next"
                | "__pycache__"
                | ".mypy_cache"
                | ".pytest_cache"
        )
    })
}

fn source_like_language(language: Option<&str>) -> bool {
    !matches!(
        language,
        None | Some("") | Some("plaintext") | Some("track_only")
    )
}

fn lower_opt(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

pub(super) fn select_unknown_target_policy(
    repo_id: &str,
    generation_seq: u64,
    target_summaries: &BTreeMap<RoleTarget, RoleTargetSummary>,
    facts_by_target: &BTreeMap<RoleTarget, Vec<ArchitectureArtefactFact>>,
    deterministic_assignments: &[ArchitectureRoleAssignment],
    authoritative_targets: &BTreeSet<RoleTarget>,
    eligible_paths: Option<&BTreeSet<String>>,
) -> UnknownTargetPolicyOutcome {
    let assigned_targets = deterministic_assignments
        .iter()
        .map(|assignment| assignment.target.clone())
        .collect::<BTreeSet<_>>();
    let file_targets_by_path = target_summaries
        .keys()
        .filter(|target| target.target_kind == TargetKind::File)
        .map(|target| target.path.clone())
        .collect::<BTreeSet<_>>();

    let mut outcome = UnknownTargetPolicyOutcome::default();
    for summary in target_summaries.values() {
        if eligible_paths
            .map(|paths| !paths.contains(&summary.target.path))
            .unwrap_or(false)
        {
            continue;
        }
        if assigned_targets.contains(&summary.target)
            || authoritative_targets.contains(&summary.target)
        {
            continue;
        }

        outcome.unknown_targets_total += 1;
        match unknown_target_disposition(summary, &file_targets_by_path) {
            UnknownTargetDisposition::SuppressNonRole => {
                outcome.suppressed_non_role += 1;
            }
            UnknownTargetDisposition::RuleMiningCandidate => {
                outcome.rule_mining_eligible += 1;
                outcome.rule_mining_inputs.push(
                    super::super::rule_mining::RoleRuleMiningClusterInput {
                        reason: AdjudicationReason::Unknown,
                        target: summary.target.clone(),
                        facts: facts_by_target
                            .get(&summary.target)
                            .cloned()
                            .unwrap_or_default(),
                        candidate_role_ids: Vec::new(),
                        deterministic_confidence: None,
                        high_impact: false,
                    },
                );
            }
            UnknownTargetDisposition::AdjudicateHighImpact => {
                outcome.adjudication_escalated += 1;
                outcome
                    .adjudication_requests
                    .push(adjudication_request_for_unknown_summary(
                        repo_id,
                        generation_seq,
                        summary,
                        AdjudicationReason::HighImpact,
                    ));
            }
        }
    }
    outcome
}

fn adjudication_request_for_unknown_summary(
    repo_id: &str,
    generation_seq: u64,
    summary: &RoleTargetSummary,
    reason: AdjudicationReason,
) -> RoleAdjudicationRequest {
    let target = &summary.target;
    let (target_kind, artefact_id, symbol_id) = request_target_fields(target);
    let stable_request_key = role_adjudication_stable_request_key(
        target_kind.as_deref(),
        artefact_id.as_deref(),
        symbol_id.as_deref(),
        Some(&target.path),
    );
    RoleAdjudicationRequest {
        repo_id: repo_id.to_string(),
        generation: generation_seq,
        stable_request_key,
        facts_hash: placeholder_request_hash(),
        rules_hash: placeholder_request_hash(),
        cluster_key: None,
        target_kind,
        artefact_id,
        symbol_id,
        path: Some(target.path.clone()),
        language: summary.language.clone(),
        canonical_kind: summary.canonical_kind.clone(),
        reason,
        deterministic_confidence: None,
        candidate_role_ids: Vec::new(),
        current_assignment: None,
    }
}
