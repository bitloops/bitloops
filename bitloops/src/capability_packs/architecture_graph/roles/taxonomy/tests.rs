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
                    weight: None,
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
        }];

        assert!(role_rule_matches(&selector, &positive, &[], &artefact));

        let negative = vec![RoleRuleCondition {
            kind: "path_suffix".to_string(),
            value: json!(".ts"),
        }];
        assert!(role_rule_matches(
            &selector, &positive, &negative, &artefact
        ));
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
            }],
            negative_conditions: vec![RoleRuleCondition {
                kind: "canonical_kind_is".to_string(),
                value: json!("test"),
            }],
            score: RoleRuleScore {
                base_confidence: Some(0.8),
                weight: Some(1.0),
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
