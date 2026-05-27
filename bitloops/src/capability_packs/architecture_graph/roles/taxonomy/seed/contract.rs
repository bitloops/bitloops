use serde_json::{Value, json};

use super::supported_rule_fact_catalog;

pub fn generic_role_family_examples() -> Value {
    json!([
        {
            "family": "entrypoint",
            "examples": ["cli_command_surface", "http_route_handler", "job_runner"]
        },
        {
            "family": "application",
            "examples": ["use_case_orchestrator", "service_facade", "workflow_coordinator"]
        },
        {
            "family": "domain",
            "examples": ["aggregate_root", "domain_service", "policy_engine"]
        },
        {
            "family": "infrastructure",
            "examples": ["repository_adapter", "queue_adapter", "external_api_client"]
        }
    ])
}

pub fn role_rule_condition_catalog() -> Value {
    json!([
        {
            "kind": "path_contains",
            "fact": "path.full",
            "value": "Substring that must appear in the repository-relative path.",
            "description": "Use for stable path segments such as `src/cli`, `commands`, or `tests`."
        },
        {
            "kind": "path_equals",
            "fact": "path.full",
            "value": "Exact repository-relative path.",
            "description": "Use only when one specific file or artefact path is the intended deterministic match."
        },
        {
            "kind": "path_prefix",
            "fact": "path.full",
            "value": "Repository-relative path prefix.",
            "description": "Use for directories or stable source tree areas."
        },
        {
            "kind": "path_suffix",
            "fact": "path.full",
            "value": "Repository-relative path suffix.",
            "description": "Use for file names, extensions, or stable suffixes such as `_test.rs`."
        },
        {
            "kind": "language_is",
            "fact": "language.resolved",
            "value": "Language identifier from evidence, such as `rust` or `typescript`.",
            "description": "Use to keep a rule scoped to one language."
        },
        {
            "kind": "canonical_kind_is",
            "fact": "artefact.canonical_kind",
            "value": "Canonical artefact kind from evidence, such as `function`, `method`, `class`, or `test`.",
            "description": "Use to constrain rules to specific artefact kinds."
        },
        {
            "kind": "symbol_fqn_contains",
            "fact": "symbol.fqn",
            "value": "Substring that must appear in the fully qualified symbol name.",
            "description": "Use for stable module, namespace, type, or function naming patterns."
        }
    ])
}

pub fn role_rule_fact_to_condition_mapping() -> Value {
    json!([
        {
            "evidence_field": "canonical_files.path",
            "condition_kind": "path_prefix|path_suffix|path_contains|path_equals",
            "guidance": "Use stable repository-relative path structure. Prefer prefixes for directories and suffixes for file names or extensions."
        },
        {
            "evidence_field": "canonical_files.resolved_language",
            "condition_kind": "language_is",
            "guidance": "Use the resolved language value when available."
        },
        {
            "evidence_field": "canonical_artefacts.language",
            "condition_kind": "language_is",
            "guidance": "Use when the rule targets symbols rather than files."
        },
        {
            "evidence_field": "canonical_artefacts.canonical_kind",
            "condition_kind": "canonical_kind_is",
            "guidance": "Use for symbol kind constraints such as function, method, struct, class, module, or test when present in evidence."
        },
        {
            "evidence_field": "canonical_artefacts.symbol_fqn",
            "condition_kind": "symbol_fqn_contains",
            "guidance": "Use stable namespace, module, type, or function substrings. Avoid one-off generated ids."
        }
    ])
}

pub fn unsupported_role_rule_signals() -> Value {
    json!([
        {
            "signal": "dependency_count",
            "reason": "Dependency counts are useful evidence but are not a supported deterministic condition kind today."
        },
        {
            "signal": "dependency_edge_kind",
            "reason": "dependency_graph_hints can support confidence, but edge_kind is not directly matchable by the current classifier."
        }
    ])
}

pub fn rule_authoring_contract_json() -> Value {
    json!({
        "contract_version": "fact-backed-rule-v2",
        "supported_predicates": supported_rule_fact_catalog(),
        "target_kinds": ["file", "artefact", "symbol"],
        "same_target_semantics": "Every required fact group is matched against a single file, artefact, or symbol target. A rule does not join facts across parent/child targets.",
        "predicate_guidance": [
            "Use only predicate ids listed in supported_predicates.",
            "signature.contains:eq means exact match against an extracted signature token.",
            "Do not use natural language operators such as `contains` unless that exact predicate id is listed."
        ],
        "scoring": {
            "base_confidence": "Maximum confidence for this rule when positive evidence is fully satisfied.",
            "condition_score": "Relative contribution within the rule; normalized at evaluation time.",
            "min_positive_ratio": "Minimum normalized positive evidence required before emitting a signal."
        },
        "rule_candidate_examples": role_rule_candidate_examples(),
    })
}

pub fn role_rule_candidate_examples() -> Value {
    json!([
        {
            "target_role_key": "cli_command_surface",
            "candidate_selector": {
                "target_kinds": ["artefact"],
                "path_prefixes": ["src/cli"],
                "path_suffixes": [".rs"],
                "path_contains": ["commands"],
                "languages": ["rust"],
                "canonical_kinds": ["function"],
                "symbol_fqn_contains": [],
                "required_facts": [
                    { "predicate": "language.resolved:eq", "value": "rust", "score": 1.0 }
                ],
                "required_fact_any_groups": []
            },
            "positive_conditions": [
                { "predicate": "path.full:prefix", "value": "src/cli", "score": 0.35 },
                { "predicate": "path.full:contains", "value": "commands", "score": 0.35 },
                { "predicate": "language.resolved:eq", "value": "rust", "score": 0.30 },
                { "predicate": "signature.contains:eq", "value": "Result", "score": 0.10 }
            ],
            "negative_conditions": [
                { "predicate": "path.full:suffix", "value": "_test.rs", "score": 1.0 }
            ],
            "score": {
                "base_confidence": 0.82,
                "priority_hint": 100,
                "min_positive_ratio": 1.0
            },
            "evidence": {
                "inspected_paths": ["src/cli/commands/run.rs"],
                "positive_examples": [
                    {
                        "path": "src/cli/commands/run.rs",
                        "symbol_fqn": "crate::cli::commands::run",
                        "canonical_kind": "function",
                        "why": "Command path and function symbol match the CLI command surface role."
                    }
                ],
                "negative_examples": [
                    {
                        "path": "src/cli/commands/run_test.rs",
                        "symbol_fqn": null,
                        "canonical_kind": "test",
                        "why": "Test files should not define the runtime command surface role."
                    }
                ],
                "db_sections_used": ["canonical_files", "canonical_artefacts"],
                "reasoning_summary": "CLI command files under src/cli/commands in Rust are likely command-surface artefacts.",
                "confidence_reason": "Path, language, and canonical kind are stable deterministic signals.",
                "uncertainty": ""
            },
            "metadata": {}
        },
        {
            "target_role_key": "domain_policy",
            "candidate_selector": {
                "target_kinds": ["artefact"],
                "path_prefixes": ["src/domain"],
                "path_suffixes": [],
                "path_contains": [],
                "languages": ["rust"],
                "canonical_kinds": ["struct", "enum", "function"],
                "symbol_fqn_contains": ["policy"],
                "required_facts": [
                    { "predicate": "language.resolved:eq", "value": "rust", "score": 1.0 }
                ],
                "required_fact_any_groups": []
            },
            "positive_conditions": [
                { "predicate": "artefact.canonical_kind:eq", "value": "function", "score": 0.50 },
                { "predicate": "symbol.fqn:contains", "value": "policy", "score": 0.50 }
            ],
            "negative_conditions": [],
            "score": {
                "base_confidence": 0.74,
                "priority_hint": 100,
                "min_positive_ratio": 1.0
            },
            "evidence": {
                "inspected_paths": ["src/domain/policy.rs"],
                "positive_examples": [
                    {
                        "path": "src/domain/policy.rs",
                        "symbol_fqn": "crate::domain::policy::apply_policy",
                        "canonical_kind": "function",
                        "why": "Domain path and policy symbol naming match the role."
                    }
                ],
                "negative_examples": [],
                "db_sections_used": ["canonical_files", "canonical_artefacts"],
                "reasoning_summary": "Domain path and policy naming are stable enough for reviewable deterministic suggestions.",
                "confidence_reason": "The rule uses stable path and symbol naming constraints.",
                "uncertainty": ""
            },
            "metadata": {}
        }
    ])
}
