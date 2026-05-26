#[cfg(test)]
mod deterministic_tests {
    use super::super::*;

    #[test]
    fn stable_role_id_does_not_depend_on_display_name() {
        let first = stable_role_id("repo-1", "application", "entrypoint");
        let second = stable_role_id("repo-1", "Application", "Entrypoint");
        assert_eq!(first, second);
    }

    #[test]
    fn role_fragment_normalization_preserves_separator_boundaries() {
        assert_eq!(normalize_role_fragment("Entry Point"), "entry_point");
        assert_ne!(
            normalize_role_fragment("Entry Point"),
            normalize_role_fragment("Entrypoint")
        );
    }

    #[test]
    fn db_enum_values_match_schema_contract() {
        assert_eq!(RoleLifecycle::Active.as_db(), "active");
        assert_eq!(RoleLifecycle::Deprecated.as_db(), "deprecated");
        assert_eq!(RoleLifecycle::Removed.as_db(), "removed");
        assert_eq!(AssignmentStatus::NeedsReview.as_db(), "needs_review");
        assert_eq!(AssignmentSource::Llm.as_db(), "llm");
        assert_eq!(TargetKind::Artefact.as_db(), "artefact");
    }

    #[test]
    fn assignment_id_is_stable_for_same_target() {
        let target = RoleTarget::artefact("art-1", "sym-1", "src/main.rs");
        let first = assignment_id("repo-1", "role-1", &target);
        let second = assignment_id("repo-1", "role-1", &target);
        assert_eq!(first, second);
    }

    #[test]
    fn fact_id_is_stable_for_same_target_and_fact() {
        let target = RoleTarget::artefact("art-1", "sym-1", "src/main.rs");
        let first = fact_id("repo-1", &target, "path", "suffix", ".rs");
        let second = fact_id("repo-1", &target, "path", "suffix", ".rs");
        assert_eq!(first, second);
    }

    #[test]
    fn rule_signal_id_distinguishes_polarity() {
        let target = RoleTarget::file("src/main.rs");
        let positive = rule_signal_id(
            "repo-1",
            "rule-1",
            1,
            "role-1",
            &target,
            RoleSignalPolarity::Positive,
        );
        let negative = rule_signal_id(
            "repo-1",
            "rule-1",
            1,
            "role-1",
            &target,
            RoleSignalPolarity::Negative,
        );
        assert_ne!(positive, negative);
    }

    #[test]
    fn rule_signal_id_distinguishes_file_paths() {
        let first = rule_signal_id(
            "repo-1",
            "rule-1",
            1,
            "role-1",
            &RoleTarget::file("src/main.rs"),
            RoleSignalPolarity::Positive,
        );
        let second = rule_signal_id(
            "repo-1",
            "rule-1",
            1,
            "role-1",
            &RoleTarget::file("src/lib.rs"),
            RoleSignalPolarity::Positive,
        );
        assert_ne!(first, second);
    }

    #[test]
    fn symbol_target_constructor_uses_symbol_target_kind() {
        let target = RoleTarget::symbol("art-1", "sym-1", "src/main.rs");
        assert_eq!(target.target_kind, TargetKind::Symbol);
        assert_eq!(target.artefact_id.as_deref(), Some("art-1"));
        assert_eq!(target.symbol_id.as_deref(), Some("sym-1"));
    }
}

#[cfg(test)]
mod seeded_tests {
    use super::super::*;

    fn condition_schema_properties<'a>(
        schema: &'a serde_json::Value,
        pointer: &str,
    ) -> &'a serde_json::Map<String, serde_json::Value> {
        schema
            .pointer(pointer)
            .and_then(|value| value.get("properties"))
            .and_then(serde_json::Value::as_object)
            .expect("condition schema properties")
    }

    fn string_enum_values(
        properties: &serde_json::Map<String, serde_json::Value>,
        key: &str,
    ) -> std::collections::BTreeSet<String> {
        properties
            .get(key)
            .and_then(|value| value.get("enum"))
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("missing enum for {key}"))
            .iter()
            .map(|value| value.as_str().expect("enum value is a string").to_string())
            .collect()
    }

    fn assert_schema_has_no_composition(path: &str, value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for keyword in ["oneOf", "anyOf", "allOf"] {
                    assert!(
                        !map.contains_key(keyword),
                        "{path}: provider-neutral seed schema must not use {keyword}"
                    );
                }
                for (key, child) in map {
                    assert_schema_has_no_composition(&format!("{path}/{key}"), child);
                }
            }
            serde_json::Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    assert_schema_has_no_composition(&format!("{path}/{index}"), item);
                }
            }
            _ => {}
        }
    }

    fn assert_schema_objects_require_every_property(value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                let strict_object = map.get("type").and_then(serde_json::Value::as_str)
                    == Some("object")
                    && matches!(
                        map.get("additionalProperties"),
                        Some(serde_json::Value::Bool(false))
                    );
                if let Some(properties) =
                    map.get("properties").and_then(serde_json::Value::as_object)
                {
                    let required = map
                        .get("required")
                        .and_then(serde_json::Value::as_array)
                        .expect("object with properties must declare required fields")
                        .iter()
                        .map(|value| value.as_str().expect("required field must be a string"))
                        .collect::<std::collections::BTreeSet<_>>();
                    let property_keys = properties
                        .keys()
                        .map(String::as_str)
                        .collect::<std::collections::BTreeSet<_>>();
                    assert_eq!(required, property_keys);
                } else if strict_object {
                    panic!("strict object schemas must declare properties and required fields");
                }
                for child in map.values() {
                    assert_schema_objects_require_every_property(child);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    assert_schema_objects_require_every_property(item);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn architecture_roles_seed_schema_requires_all_declared_properties() {
        let schema = architecture_roles_seed_schema();

        assert_schema_objects_require_every_property(&schema);
    }

    #[test]
    fn architecture_roles_seed_phase_schemas_require_all_declared_properties() {
        let role_schema = architecture_roles_seed_roles_schema();
        let rule_schema = architecture_roles_seed_rule_candidates_schema();

        assert_schema_objects_require_every_property(&role_schema);
        assert_schema_objects_require_every_property(&rule_schema);
    }

    #[test]
    fn architecture_roles_seed_role_schema_does_not_ask_llm_for_lifecycle() {
        let schema = architecture_roles_seed_roles_schema();
        let role_properties = schema
            .pointer("/properties/roles/items/properties")
            .and_then(serde_json::Value::as_object)
            .expect("role schema properties");

        assert!(
            !role_properties.contains_key("lifecycle_status"),
            "seed LLM should not control role lifecycle"
        );
    }

    #[test]
    fn seeded_role_schema_allows_bounded_evidence_fields() {
        let schema = architecture_roles_seed_roles_schema();
        let evidence_properties = schema
            .pointer("/properties/roles/items/properties/evidence/properties")
            .and_then(serde_json::Value::as_object)
            .expect("role evidence properties");

        for key in [
            "inspected_paths",
            "supporting_paths",
            "supporting_symbols",
            "db_sections_used",
            "reasoning_summary",
            "confidence_reason",
            "uncertainty",
        ] {
            assert!(
                evidence_properties.contains_key(key),
                "missing role evidence key {key}"
            );
        }
    }

    #[test]
    fn seeded_rule_candidate_schema_allows_bounded_evidence_fields() {
        let schema = architecture_roles_seed_rule_candidates_schema();
        let evidence_properties = schema
            .pointer("/properties/rule_candidates/items/properties/evidence/properties")
            .and_then(serde_json::Value::as_object)
            .expect("rule evidence properties");

        for key in [
            "inspected_paths",
            "positive_examples",
            "negative_examples",
            "db_sections_used",
            "reasoning_summary",
            "confidence_reason",
            "uncertainty",
        ] {
            assert!(
                evidence_properties.contains_key(key),
                "missing rule evidence key {key}"
            );
        }
    }

    #[test]
    fn seeded_rule_score_schema_exposes_only_used_runtime_fields() {
        let schema = architecture_roles_seed_rule_candidates_schema();
        let score = schema
            .pointer("/properties/rule_candidates/items/properties/score/properties")
            .and_then(serde_json::Value::as_object)
            .expect("score properties");

        assert!(score.contains_key("base_confidence"));
        assert!(score.contains_key("priority_hint"));
        assert!(score.contains_key("min_positive_ratio"));
        assert!(!score.contains_key("weight"));
    }

    #[test]
    fn supported_fact_predicates_include_only_valid_fact_op_pairs() {
        let predicates = supported_fact_predicates();

        assert!(!predicates.is_empty());
        for predicate in predicates {
            let parsed = parse_supported_fact_predicate(predicate.id).expect("predicate parses");
            assert_eq!(parsed.id, predicate.id);
            assert_eq!(parsed.kind, predicate.kind);
            assert_eq!(parsed.key, predicate.key);
            assert_eq!(parsed.op, predicate.op);
        }
    }

    #[test]
    fn supported_fact_predicates_include_signature_contains_eq() {
        let predicate = parse_supported_fact_predicate("signature.contains:eq")
            .expect("signature predicate should exist");

        assert_eq!(predicate.kind, "signature");
        assert_eq!(predicate.key, "contains");
        assert_eq!(predicate.op, "eq");
    }

    #[test]
    fn supported_fact_predicates_reject_signature_contains_contains() {
        assert!(parse_supported_fact_predicate("signature.contains:contains").is_none());
    }

    #[test]
    fn seed_rule_schema_uses_provider_neutral_predicate_conditions() {
        let schema = architecture_roles_seed_rule_candidates_schema();
        let selector = schema
            .pointer("/properties/rule_candidates/items/properties/candidate_selector/properties")
            .and_then(serde_json::Value::as_object)
            .expect("selector properties");

        assert!(selector.contains_key("target_kinds"));
        assert!(selector.contains_key("required_facts"));
        assert!(selector.contains_key("required_fact_any_groups"));

        let properties = condition_schema_properties(
            &schema,
            "/properties/rule_candidates/items/properties/positive_conditions/items",
        );
        for key in ["predicate", "value", "score"] {
            assert!(properties.contains_key(key), "missing condition key {key}");
        }
        for key in ["kind", "key", "op"] {
            assert!(
                !properties.contains_key(key),
                "condition schema should not expose legacy key {key}"
            );
        }

        let predicates = string_enum_values(properties, "predicate");
        assert!(predicates.contains("signature.contains:eq"));
        assert!(!predicates.contains("signature.contains:contains"));
        assert!(predicates.contains("path.full:prefix"));
        assert!(predicates.contains("dependency.outgoing_count:gte"));
    }

    #[test]
    fn seed_rule_schema_avoids_schema_composition() {
        let schema = architecture_roles_seed_rule_candidates_schema();

        assert_schema_has_no_composition("$", &schema);
    }

    #[test]
    fn rule_condition_catalog_documents_all_supported_condition_kinds() {
        let allowed: std::collections::BTreeSet<_> =
            allowed_rule_condition_kinds().iter().copied().collect();

        assert_eq!(
            allowed,
            std::collections::BTreeSet::from([
                "path_contains",
                "path_equals",
                "path_prefix",
                "path_suffix",
                "language_is",
                "canonical_kind_is",
                "symbol_fqn_contains",
            ])
        );

        let catalog = role_rule_condition_catalog();
        let entries = catalog.as_array().expect("catalog is an array");
        let catalog_kinds: std::collections::BTreeSet<_> = entries
            .iter()
            .map(|entry| {
                entry
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .expect("catalog entry has kind")
            })
            .collect();

        assert_eq!(catalog_kinds, allowed);
        for entry in entries {
            assert!(
                entry
                    .get("fact")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            );
            assert!(
                entry
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            );
            assert!(
                entry
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            );
        }
    }

    #[test]
    fn rule_authoring_guidance_maps_supported_facts_and_names_unsupported_signals() {
        let mapping = role_rule_fact_to_condition_mapping();
        let mapping = mapping.as_array().expect("mapping is an array");
        assert!(mapping.iter().any(|entry| {
            entry
                .get("evidence_field")
                .and_then(serde_json::Value::as_str)
                == Some("canonical_files.path")
        }));
        assert!(mapping.iter().any(|entry| {
            entry
                .get("condition_kind")
                .and_then(serde_json::Value::as_str)
                == Some("canonical_kind_is")
        }));

        let unsupported = unsupported_role_rule_signals();
        let unsupported = unsupported.as_array().expect("unsupported is an array");
        assert!(!unsupported.iter().any(|entry| {
            entry.get("signal").and_then(serde_json::Value::as_str) == Some("signature_contains")
        }));
        assert!(unsupported.iter().any(|entry| {
            entry.get("signal").and_then(serde_json::Value::as_str) == Some("dependency_edge_kind")
        }));
    }

    #[test]
    fn rule_authoring_contract_documents_same_target_semantics() {
        let contract = rule_authoring_contract_json();
        let text = serde_json::to_string(&contract).expect("contract json");
        assert!(text.contains("same_target_semantics"));
        assert!(text.contains("single file, artefact, or symbol target"));
        assert!(text.contains("file"));
        assert!(text.contains("artefact"));
        assert!(text.contains("symbol"));
    }

    #[test]
    fn rule_candidate_examples_use_only_supported_condition_kinds() {
        let catalog = supported_rule_fact_catalog();
        let catalog = catalog.as_array().expect("catalog is an array");
        let supported = catalog
            .iter()
            .map(|predicate| {
                predicate
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .expect("predicate has id")
                    .to_string()
            })
            .collect::<std::collections::BTreeSet<_>>();

        let examples = role_rule_candidate_examples();
        let examples = examples.as_array().expect("examples are an array");

        assert!(
            !examples.is_empty(),
            "seed prompt should include at least one rule example"
        );

        for example in examples {
            let selector = example
                .get("candidate_selector")
                .and_then(serde_json::Value::as_object)
                .expect("example includes selector");
            for key in [
                "path_prefixes",
                "path_suffixes",
                "path_contains",
                "languages",
                "canonical_kinds",
                "symbol_fqn_contains",
            ] {
                assert!(
                    selector
                        .get(key)
                        .and_then(serde_json::Value::as_array)
                        .is_some()
                );
            }

            for conditions_key in ["positive_conditions", "negative_conditions"] {
                let conditions = example
                    .get(conditions_key)
                    .and_then(serde_json::Value::as_array)
                    .expect("example includes condition arrays");
                for condition in conditions {
                    let predicate = condition
                        .get("predicate")
                        .and_then(serde_json::Value::as_str)
                        .expect("condition has predicate");
                    assert!(
                        supported.contains(predicate),
                        "unsupported example condition `{predicate}`"
                    );
                    assert!(
                        condition
                            .get("value")
                            .and_then(serde_json::Value::as_str)
                            .is_some()
                    );
                    assert!(
                        condition
                            .get("score")
                            .and_then(serde_json::Value::as_f64)
                            .is_some()
                    );
                }
            }

            let evidence = example
                .get("evidence")
                .and_then(serde_json::Value::as_object)
                .expect("example includes bounded evidence");
            let evidence_keys = evidence
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                evidence_keys,
                std::collections::BTreeSet::from([
                    "inspected_paths",
                    "positive_examples",
                    "negative_examples",
                    "db_sections_used",
                    "reasoning_summary",
                    "confidence_reason",
                    "uncertainty",
                ])
            );

            let metadata = example
                .get("metadata")
                .and_then(serde_json::Value::as_object)
                .expect("example includes metadata");
            assert!(
                metadata.is_empty(),
                "schema examples should keep metadata as a strict empty object"
            );
        }
    }

    #[test]
    fn seed_schema_enumerates_supported_rule_condition_kinds() {
        let schemas = [
            architecture_roles_seed_schema(),
            architecture_roles_seed_rule_candidates_schema(),
        ];

        for schema in schemas {
            for pointer in [
                "/properties/rule_candidates/items/properties/positive_conditions/items",
                "/properties/rule_candidates/items/properties/negative_conditions/items",
            ] {
                let properties = condition_schema_properties(&schema, pointer);
                assert_eq!(
                    string_enum_values(properties, "predicate"),
                    std::collections::BTreeSet::from([
                        "artefact.canonical_kind:eq".to_string(),
                        "artefact.has_parent_artefact:eq".to_string(),
                        "artefact.language_kind:contains".to_string(),
                        "artefact.language_kind:eq".to_string(),
                        "dependency.incoming_count:eq".to_string(),
                        "dependency.incoming_count:gte".to_string(),
                        "dependency.incoming_count:lte".to_string(),
                        "dependency.incoming_kind:eq".to_string(),
                        "dependency.outgoing_count:eq".to_string(),
                        "dependency.outgoing_count:gte".to_string(),
                        "dependency.outgoing_count:lte".to_string(),
                        "dependency.outgoing_kind:eq".to_string(),
                        "file.analysis_mode:eq".to_string(),
                        "file.role:eq".to_string(),
                        "language.resolved:eq".to_string(),
                        "path.extension:eq".to_string(),
                        "path.full:contains".to_string(),
                        "path.full:eq".to_string(),
                        "path.full:prefix".to_string(),
                        "path.full:suffix".to_string(),
                        "path.segment:eq".to_string(),
                        "signature.contains:eq".to_string(),
                        "symbol.fqn:contains".to_string(),
                        "symbol.fqn:eq".to_string(),
                        "symbol.fqn:prefix".to_string(),
                        "symbol.fqn:suffix".to_string(),
                        "symbol.has_signature:eq".to_string(),
                        "symbol.name:contains".to_string(),
                        "symbol.name:eq".to_string(),
                        "symbol.name:prefix".to_string(),
                        "symbol.name:suffix".to_string(),
                        "symbol.name_suffix:eq".to_string(),
                    ])
                );
            }
        }
    }

    #[test]
    fn decode_seeded_rule_candidates_repairs_signature_contains_contains() {
        let decoded = decode_seeded_rule_candidates_with_recovery(json!({
            "rule_candidates": [
                {
                    "target_role_key": "command_dispatcher",
                    "candidate_selector": {
                        "target_kinds": ["artefact"],
                        "path_prefixes": [],
                        "path_suffixes": [],
                        "path_contains": [],
                        "languages": [],
                        "canonical_kinds": [],
                        "symbol_fqn_contains": [],
                        "required_facts": [
                            { "kind": "signature", "key": "contains", "op": "contains", "value": "Result", "score": 0.7 }
                        ],
                        "required_fact_any_groups": []
                    },
                    "positive_conditions": [
                        { "predicate": "path.full:contains", "value": "src", "score": 0.8 }
                    ],
                    "negative_conditions": [],
                    "score": { "base_confidence": 0.8, "priority_hint": 100, "min_positive_ratio": 1.0 },
                    "evidence": {
                        "inspected_paths": [],
                        "positive_examples": [],
                        "negative_examples": [],
                        "db_sections_used": [],
                        "reasoning_summary": "",
                        "confidence_reason": "",
                        "uncertainty": ""
                    },
                    "metadata": {}
                }
            ]
        }));

        assert_eq!(decoded.accepted.len(), 1);
        assert_eq!(decoded.repaired.len(), 1);
        assert!(decoded.rejected.is_empty());
        assert_eq!(
            decoded.accepted[0].candidate_selector.required_facts[0].op,
            Some(RoleFactConditionOp::Eq)
        );
    }

    #[test]
    fn decode_seeded_rule_candidates_keeps_valid_siblings_when_one_candidate_is_invalid() {
        let decoded = decode_seeded_rule_candidates_with_recovery(json!({
            "rule_candidates": [
                {
                    "target_role_key": "command_dispatcher",
                    "candidate_selector": {
                        "target_kinds": ["file"],
                        "path_prefixes": [],
                        "path_suffixes": [],
                        "path_contains": [],
                        "languages": [],
                        "canonical_kinds": [],
                        "symbol_fqn_contains": [],
                        "required_facts": [],
                        "required_fact_any_groups": []
                    },
                    "positive_conditions": [
                        { "predicate": "path.full:prefix", "value": "src/cli", "score": 1.0 }
                    ],
                    "negative_conditions": [],
                    "score": { "base_confidence": 0.8, "priority_hint": 100, "min_positive_ratio": 1.0 },
                    "evidence": {
                        "inspected_paths": [],
                        "positive_examples": [],
                        "negative_examples": [],
                        "db_sections_used": [],
                        "reasoning_summary": "",
                        "confidence_reason": "",
                        "uncertainty": ""
                    },
                    "metadata": {}
                },
                {
                    "target_role_key": "command_dispatcher",
                    "candidate_selector": {
                        "target_kinds": ["file"],
                        "path_prefixes": [],
                        "path_suffixes": [],
                        "path_contains": [],
                        "languages": [],
                        "canonical_kinds": [],
                        "symbol_fqn_contains": [],
                        "required_facts": [],
                        "required_fact_any_groups": []
                    },
                    "positive_conditions": [
                        { "predicate": "signature.contains:contains", "value": "Result", "score": 1.0 }
                    ],
                    "negative_conditions": [],
                    "score": { "base_confidence": 0.8, "priority_hint": 100, "min_positive_ratio": 1.0 },
                    "evidence": {
                        "inspected_paths": [],
                        "positive_examples": [],
                        "negative_examples": [],
                        "db_sections_used": [],
                        "reasoning_summary": "",
                        "confidence_reason": "",
                        "uncertainty": ""
                    },
                    "metadata": {}
                }
            ]
        }));

        assert_eq!(decoded.accepted.len(), 1);
        assert_eq!(decoded.rejected.len(), 1);
        assert_eq!(decoded.accepted[0].positive_conditions[0].kind, "path");
    }

    #[test]
    fn decode_seeded_rule_candidates_rejects_ambiguous_unsupported_ops() {
        let decoded = decode_seeded_rule_candidates_with_recovery(json!({
            "rule_candidates": [
                {
                    "target_role_key": "command_dispatcher",
                    "candidate_selector": {
                        "target_kinds": ["file"],
                        "path_prefixes": [],
                        "path_suffixes": [],
                        "path_contains": [],
                        "languages": [],
                        "canonical_kinds": [],
                        "symbol_fqn_contains": [],
                        "required_facts": [],
                        "required_fact_any_groups": []
                    },
                    "positive_conditions": [
                        { "kind": "path", "key": "extension", "op": "contains", "value": ".rs", "score": 1.0 }
                    ],
                    "negative_conditions": [],
                    "score": { "base_confidence": 0.8, "priority_hint": 100, "min_positive_ratio": 1.0 },
                    "evidence": {
                        "inspected_paths": [],
                        "positive_examples": [],
                        "negative_examples": [],
                        "db_sections_used": [],
                        "reasoning_summary": "",
                        "confidence_reason": "",
                        "uncertainty": ""
                    },
                    "metadata": {}
                }
            ]
        }));

        assert!(decoded.accepted.is_empty());
        assert!(decoded.repaired.is_empty());
        assert_eq!(decoded.rejected.len(), 1);
        assert!(decoded.rejected[0].reason.contains("unsupported op"));
    }

    #[test]
    fn validate_seeded_taxonomy_reports_rule_condition_location() {
        let taxonomy = SeededArchitectureTaxonomy {
            roles: vec![SeededArchitectureRole {
                canonical_key: "command_dispatcher".to_string(),
                display_name: "Command Dispatcher".to_string(),
                description: String::new(),
                family: Some("entrypoint".to_string()),
                lifecycle_status: Some("active".to_string()),
                provenance: json!({}),
                evidence: json!([]),
            }],
            rule_candidates: vec![SeededArchitectureRuleCandidate {
                target_role_key: "command_dispatcher".to_string(),
                candidate_selector: RoleRuleCandidateSelector::default(),
                positive_conditions: vec![RoleRuleCondition {
                    kind: "path".to_string(),
                    key: Some("path".to_string()),
                    op: Some(RoleFactConditionOp::Eq),
                    value: json!("src/main.rs"),
                    score: Some(1.0),
                }],
                negative_conditions: vec![],
                score: RoleRuleScore::default(),
                evidence: json!([]),
                metadata: json!({}),
            }],
        };

        let err = validate_seeded_taxonomy(&taxonomy).expect_err("path.path should fail");
        let message = err.to_string();

        assert!(message.contains("rule_candidates[0:command_dispatcher].positive_conditions[0]"));
        assert!(message.contains("unsupported fact condition path.path"));
    }

    #[test]
    fn validates_seeded_taxonomy_and_rejects_unknown_target_roles() {
        let valid = SeededArchitectureTaxonomy {
            roles: vec![SeededArchitectureRole {
                canonical_key: "command_dispatcher".to_string(),
                display_name: "Command Dispatcher".to_string(),
                description: String::new(),
                family: Some("entrypoint".to_string()),
                lifecycle_status: Some("active".to_string()),
                provenance: json!({}),
                evidence: json!([]),
            }],
            rule_candidates: vec![SeededArchitectureRuleCandidate {
                target_role_key: "command_dispatcher".to_string(),
                candidate_selector: RoleRuleCandidateSelector {
                    path_prefixes: vec!["src/cli".to_string()],
                    ..Default::default()
                },
                positive_conditions: vec![],
                negative_conditions: vec![],
                score: RoleRuleScore {
                    base_confidence: Some(0.8),
                    priority_hint: None,
                    min_positive_ratio: None,
                },
                evidence: json!([]),
                metadata: json!({}),
            }],
        };
        validate_seeded_taxonomy(&valid).expect("valid taxonomy");

        let invalid = SeededArchitectureTaxonomy {
            roles: valid.roles.clone(),
            rule_candidates: vec![SeededArchitectureRuleCandidate {
                target_role_key: "unknown".to_string(),
                ..valid.rule_candidates[0].clone()
            }],
        };
        let err = validate_seeded_taxonomy(&invalid).expect_err("invalid taxonomy");
        assert!(err.to_string().contains("unknown target role key"));

        let invalid_condition = SeededArchitectureTaxonomy {
            roles: vec![SeededArchitectureRole {
                canonical_key: "command_dispatcher".to_string(),
                display_name: "Command Dispatcher".to_string(),
                description: String::new(),
                family: Some("entrypoint".to_string()),
                lifecycle_status: Some("active".to_string()),
                provenance: json!({}),
                evidence: json!([]),
            }],
            rule_candidates: vec![SeededArchitectureRuleCandidate {
                target_role_key: "command_dispatcher".to_string(),
                candidate_selector: RoleRuleCandidateSelector::default(),
                positive_conditions: vec![RoleRuleCondition {
                    kind: "unsupported".to_string(),
                    value: json!("x"),
                    ..Default::default()
                }],
                negative_conditions: vec![],
                score: RoleRuleScore::default(),
                evidence: json!([]),
                metadata: json!({}),
            }],
        };
        let err = validate_seeded_taxonomy(&invalid_condition).expect_err("invalid condition kind");
        assert!(err.to_string().contains("unsupported rule condition kind"));
    }

    #[test]
    fn validate_seeded_taxonomy_rejects_llm_lifecycle_words() {
        for lifecycle_status in ["stable", "", "   "] {
            let taxonomy = SeededArchitectureTaxonomy {
                roles: vec![SeededArchitectureRole {
                    canonical_key: "command_dispatcher".to_string(),
                    display_name: "Command Dispatcher".to_string(),
                    description: "Routes CLI commands".to_string(),
                    family: Some("entrypoint".to_string()),
                    lifecycle_status: Some(lifecycle_status.to_string()),
                    provenance: json!({}),
                    evidence: json!({}),
                }],
                rule_candidates: vec![],
            };

            let err = validate_seeded_taxonomy(&taxonomy).expect_err("unsupported seed lifecycle");
            assert!(
                err.to_string()
                    .contains("unsupported seeded role lifecycle_status")
            );
        }
    }

    #[test]
    fn validate_seeded_taxonomy_accepts_missing_null_and_active_lifecycle() {
        for lifecycle_status in [None, Some("active".to_string())] {
            let taxonomy = SeededArchitectureTaxonomy {
                roles: vec![SeededArchitectureRole {
                    canonical_key: "command_dispatcher".to_string(),
                    display_name: "Command Dispatcher".to_string(),
                    description: "Routes CLI commands".to_string(),
                    family: Some("entrypoint".to_string()),
                    lifecycle_status,
                    provenance: json!({}),
                    evidence: json!({}),
                }],
                rule_candidates: vec![],
            };

            validate_seeded_taxonomy(&taxonomy).expect("seed lifecycle should be accepted");
        }
    }

    #[test]
    fn selector_and_conditions_match_expected_artefacts() {
        let artefact = MatchableArtefact {
            artefact_id: "artefact-1".to_string(),
            path: "src/cli/commands/run.rs".to_string(),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            symbol_fqn: Some("crate::cli::commands::run".to_string()),
        };

        let selector = RoleRuleCandidateSelector {
            path_prefixes: vec!["src/cli".to_string()],
            languages: vec!["rust".to_string()],
            ..Default::default()
        };
        let positive = vec![RoleRuleCondition {
            kind: "path_contains".to_string(),
            value: json!("commands"),
            ..Default::default()
        }];

        assert!(role_rule_matches(&selector, &positive, &[], &artefact));

        let negative = vec![RoleRuleCondition {
            kind: "path_suffix".to_string(),
            value: json!(".ts"),
            ..Default::default()
        }];
        assert!(role_rule_matches(
            &selector, &positive, &negative, &artefact
        ));
    }

    #[test]
    fn path_equals_condition_validates_and_maps_to_exact_path_match() -> anyhow::Result<()> {
        let taxonomy = SeededArchitectureTaxonomy {
            roles: vec![SeededArchitectureRole {
                canonical_key: "command_dispatcher".to_string(),
                display_name: "Command Dispatcher".to_string(),
                description: String::new(),
                family: Some("entrypoint".to_string()),
                lifecycle_status: Some("active".to_string()),
                provenance: json!({}),
                evidence: json!({}),
            }],
            rule_candidates: vec![SeededArchitectureRuleCandidate {
                target_role_key: "command_dispatcher".to_string(),
                candidate_selector: RoleRuleCandidateSelector::default(),
                positive_conditions: vec![RoleRuleCondition {
                    kind: "path_equals".to_string(),
                    value: json!("src/cli/commands/run.rs"),
                    ..Default::default()
                }],
                negative_conditions: vec![],
                score: RoleRuleScore::default(),
                evidence: json!({}),
                metadata: json!({}),
            }],
        };

        validate_seeded_taxonomy(&taxonomy)?;
        let conditions =
            role_rule_conditions_contract(&taxonomy.rule_candidates[0].positive_conditions)?;

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].kind, "path");
        assert_eq!(conditions[0].key, "full");
        assert_eq!(conditions[0].op, RoleFactConditionOp::Eq);
        assert_eq!(conditions[0].value, "src/cli/commands/run.rs");
        Ok(())
    }

    #[test]
    fn legacy_selector_multi_values_keep_or_semantics() {
        let rust_artefact = MatchableArtefact {
            artefact_id: "artefact-1".to_string(),
            path: "src/cli/commands/run.rs".to_string(),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            symbol_fqn: None,
        };
        let python_artefact = MatchableArtefact {
            artefact_id: "artefact-2".to_string(),
            path: "src/cli/commands/run.py".to_string(),
            language: Some("python".to_string()),
            canonical_kind: Some("function".to_string()),
            symbol_fqn: None,
        };
        let selector = RoleRuleCandidateSelector {
            path_prefixes: vec!["src/cli".to_string()],
            languages: vec!["rust".to_string(), "typescript".to_string()],
            ..Default::default()
        };

        assert!(role_rule_matches(&selector, &[], &[], &rust_artefact));
        assert!(!role_rule_matches(&selector, &[], &[], &python_artefact));
    }

    #[test]
    fn rule_spec_serializes_to_rule_management_contract() -> anyhow::Result<()> {
        let spec = RuleSpecFile {
            role_ref: "command_dispatcher".to_string(),
            candidate_selector: RoleRuleCandidateSelector {
                path_prefixes: vec!["src/cli".to_string()],
                languages: vec!["rust".to_string()],
                ..Default::default()
            },
            positive_conditions: vec![RoleRuleCondition {
                kind: "path".to_string(),
                key: Some("full".to_string()),
                op: Some(RoleFactConditionOp::Contains),
                value: json!("commands"),
                score: Some(1.0),
            }],
            negative_conditions: vec![RoleRuleCondition {
                kind: "artefact".to_string(),
                key: Some("canonical_kind".to_string()),
                op: Some(RoleFactConditionOp::Eq),
                value: json!("test"),
                score: Some(1.0),
            }],
            score: RoleRuleScore {
                base_confidence: Some(0.8),
                priority_hint: Some(100),
                min_positive_ratio: Some(1.0),
            },
            evidence: json!([]),
            metadata: json!({}),
        };

        let value = serde_json::to_value(&spec)?;
        let round_tripped: RuleSpecFile = serde_json::from_value(value.clone())?;

        assert_eq!(
            value["candidate_selector"],
            json!({
                "path_prefixes": ["src/cli"],
                "path_suffixes": [],
                "path_contains": [],
                "languages": ["rust"],
                "canonical_kinds": [],
                "symbol_fqn_contains": [],
                "target_kinds": [],
                "required_facts": [],
                "required_fact_any_groups": []
            })
        );
        assert_eq!(
            value["positive_conditions"],
            json!([
                { "kind": "path", "key": "full", "op": "contains", "value": "commands", "score": 1.0 }
            ])
        );
        assert_eq!(
            value["negative_conditions"],
            json!([
                { "kind": "artefact", "key": "canonical_kind", "op": "eq", "value": "test", "score": 1.0 }
            ])
        );
        assert_eq!(round_tripped, spec);
        Ok(())
    }
}
