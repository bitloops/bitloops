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
    fn seed_rule_schema_exposes_fact_backed_conditions() {
        let schema = architecture_roles_seed_rule_candidates_schema();
        let selector = schema
            .pointer("/properties/rule_candidates/items/properties/candidate_selector/properties")
            .and_then(serde_json::Value::as_object)
            .expect("selector properties");

        assert!(selector.contains_key("target_kinds"));
        assert!(selector.contains_key("required_facts"));
        assert!(selector.contains_key("required_fact_any_groups"));

        let condition = schema
            .pointer(
                "/properties/rule_candidates/items/properties/positive_conditions/items/properties",
            )
            .and_then(serde_json::Value::as_object)
            .expect("condition properties");
        for key in ["kind", "key", "op", "value", "score"] {
            assert!(condition.contains_key(key), "missing condition key {key}");
        }

        let ops = schema
            .pointer("/properties/rule_candidates/items/properties/positive_conditions/items/properties/op/enum")
            .and_then(serde_json::Value::as_array)
            .expect("condition op enum")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<std::collections::BTreeSet<_>>();

        assert!(ops.contains("gte"));
        assert!(ops.contains("lte"));
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
        assert!(unsupported.iter().any(|entry| {
            entry.get("signal").and_then(serde_json::Value::as_str) == Some("signature_contains")
        }));
        assert!(unsupported.iter().any(|entry| {
            entry.get("signal").and_then(serde_json::Value::as_str) == Some("file_role")
        }));
    }

    #[test]
    fn rule_candidate_examples_use_only_supported_condition_kinds() {
        let allowed: std::collections::BTreeSet<_> =
            allowed_rule_condition_kinds().iter().copied().collect();
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
                    let kind = condition
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .expect("condition has kind");
                    assert!(allowed.contains(kind), "unsupported example kind `{kind}`");
                    assert!(
                        condition
                            .get("value")
                            .and_then(serde_json::Value::as_str)
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
                "/properties/rule_candidates/items/properties/positive_conditions/items/properties/kind/enum",
                "/properties/rule_candidates/items/properties/negative_conditions/items/properties/kind/enum",
            ] {
                let enum_values = schema
                    .pointer(pointer)
                    .and_then(serde_json::Value::as_array)
                    .unwrap_or_else(|| panic!("missing schema enum at {pointer}"));
                let actual = enum_values
                    .iter()
                    .map(|value| value.as_str().expect("enum value is a string"))
                    .collect::<Vec<_>>();
                assert_eq!(actual, allowed_rule_condition_kinds());
            }
        }
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
                kind: "path_contains".to_string(),
                value: json!("commands"),
                ..Default::default()
            }],
            negative_conditions: vec![RoleRuleCondition {
                kind: "canonical_kind_is".to_string(),
                value: json!("test"),
                ..Default::default()
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
                "symbol_fqn_contains": []
            })
        );
        assert_eq!(
            value["positive_conditions"],
            json!([
                { "kind": "path_contains", "value": "commands" }
            ])
        );
        assert_eq!(
            value["negative_conditions"],
            json!([
                { "kind": "canonical_kind_is", "value": "test" }
            ])
        );
        assert_eq!(round_tripped, spec);
        Ok(())
    }
}
