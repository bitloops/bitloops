#[cfg(test)]
mod deterministic_tests {
    use super::super::*;
    use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
    use crate::models::{CurrentCanonicalArtefactRecord, ProductionArtefact};

    struct FakeRelationalGateway {
        artefacts: Vec<CurrentCanonicalArtefactRecord>,
    }

    impl RelationalGateway for FakeRelationalGateway {
        fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
            bail!("not used")
        }

        fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
            bail!("not used")
        }

        fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
            bail!("not used")
        }

        fn load_current_canonical_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<CurrentCanonicalArtefactRecord>> {
            Ok(self.artefacts.clone())
        }

        fn load_current_production_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<ProductionArtefact>> {
            bail!("not used")
        }

        fn load_production_artefacts(&self, _commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
            bail!("not used")
        }

        fn load_artefacts_for_file_lines(
            &self,
            _commit_sha: &str,
            _file_path: &str,
        ) -> Result<Vec<(String, i64, i64)>> {
            bail!("not used")
        }
    }

    fn gateway() -> FakeRelationalGateway {
        FakeRelationalGateway {
            artefacts: vec![
                CurrentCanonicalArtefactRecord {
                    repo_id: "repo-1".to_string(),
                    path: "src/cli/commands/run.rs".to_string(),
                    content_id: "content-1".to_string(),
                    symbol_id: "symbol-1".to_string(),
                    artefact_id: "artefact-1".to_string(),
                    language: "rust".to_string(),
                    extraction_fingerprint: "fingerprint".to_string(),
                    canonical_kind: Some("function".to_string()),
                    language_kind: Some("function".to_string()),
                    symbol_fqn: Some("crate::cli::commands::run".to_string()),
                    parent_symbol_id: None,
                    parent_artefact_id: None,
                    start_line: 1,
                    end_line: 10,
                    start_byte: 0,
                    end_byte: 50,
                    signature: Some("fn run()".to_string()),
                    modifiers: "[]".to_string(),
                    docstring: None,
                },
                CurrentCanonicalArtefactRecord {
                    repo_id: "repo-1".to_string(),
                    path: "src/domain/payments.rs".to_string(),
                    content_id: "content-2".to_string(),
                    symbol_id: "symbol-2".to_string(),
                    artefact_id: "artefact-2".to_string(),
                    language: "rust".to_string(),
                    extraction_fingerprint: "fingerprint".to_string(),
                    canonical_kind: Some("struct".to_string()),
                    language_kind: Some("struct".to_string()),
                    symbol_fqn: Some("crate::domain::payments".to_string()),
                    parent_symbol_id: None,
                    parent_artefact_id: None,
                    start_line: 1,
                    end_line: 10,
                    start_byte: 0,
                    end_byte: 50,
                    signature: Some("struct Payments".to_string()),
                    modifiers: "[]".to_string(),
                    docstring: None,
                },
            ],
        }
    }

    async fn relational() -> Result<RelationalStorage> {
        let temp = tempfile::tempdir()?;
        let sqlite_path = temp.path().join("roles.sqlite");
        rusqlite::Connection::open(&sqlite_path)?;
        let relational = RelationalStorage::local_only(sqlite_path);
        relational
            .exec(architecture_graph_sqlite_schema_sql())
            .await?;
        std::mem::forget(temp);
        Ok(relational)
    }

    async fn seed_role(relational: &RelationalStorage) -> Result<ArchitectureRoleRecord> {
        upsert_role(
            relational,
            &ArchitectureRoleRecord {
                role_id: deterministic_role_id("repo-1", "command_dispatcher"),
                repo_id: "repo-1".to_string(),
                canonical_key: "command_dispatcher".to_string(),
                display_name: "Command Dispatcher".to_string(),
                description: "Routes commands".to_string(),
                family: Some("entrypoint".to_string()),
                lifecycle_status: "active".to_string(),
                provenance: json!({"source": "test"}),
                evidence: json!([]),
                metadata: json!({}),
            },
        )
        .await
    }

    async fn seed_role_with_key(
        relational: &RelationalStorage,
        canonical_key: &str,
        display_name: &str,
    ) -> Result<ArchitectureRoleRecord> {
        upsert_role(
            relational,
            &ArchitectureRoleRecord {
                role_id: deterministic_role_id("repo-1", canonical_key),
                repo_id: "repo-1".to_string(),
                canonical_key: canonical_key.to_string(),
                display_name: display_name.to_string(),
                description: format!("role {display_name}"),
                family: Some("entrypoint".to_string()),
                lifecycle_status: "active".to_string(),
                provenance: json!({"source": "test"}),
                evidence: json!([]),
                metadata: json!({}),
            },
        )
        .await
    }

    async fn seed_assignment(
        relational: &RelationalStorage,
        artefact_id: &str,
        role_id: &str,
    ) -> Result<String> {
        seed_assignment_with_rule(relational, artefact_id, role_id, None).await
    }

    async fn seed_assignment_with_rule(
        relational: &RelationalStorage,
        artefact_id: &str,
        role_id: &str,
        rule_id: Option<&str>,
    ) -> Result<String> {
        let assignment_id = deterministic_assignment_id("repo-1", artefact_id, role_id);
        insert_role_assignment(
            relational,
            &ArchitectureRoleAssignmentRecord {
                assignment_id: assignment_id.clone(),
                repo_id: "repo-1".to_string(),
                artefact_id: artefact_id.to_string(),
                role_id: role_id.to_string(),
                source_kind: "seed".to_string(),
                confidence: 0.9,
                status: "active".to_string(),
                status_reason: String::new(),
                rule_id: rule_id.map(ToOwned::to_owned),
                migration_id: None,
                migrated_to_assignment_id: None,
                provenance: json!({"source": "test"}),
                evidence: json!([]),
                metadata: json!({}),
            },
        )
        .await?;
        Ok(assignment_id)
    }

    async fn seed_rule(
        relational: &RelationalStorage,
        role_id: &str,
        selector: RoleRuleCandidateSelector,
    ) -> Result<ArchitectureRoleRuleRecord> {
        let version = next_role_rule_version(relational, "repo-1", role_id).await?;
        let spec = RuleSpecFile {
            role_ref: role_id.to_string(),
            candidate_selector: selector,
            positive_conditions: vec![],
            negative_conditions: vec![],
            score: super::super::taxonomy::RoleRuleScore {
                base_confidence: Some(0.8),
                weight: None,
            },
            evidence: json!([]),
            metadata: json!({}),
        };
        let canonical_hash = canonical_rule_hash(&spec)?;
        let record = ArchitectureRoleRuleRecord {
            rule_id: deterministic_rule_id("repo-1", role_id, version, &canonical_hash),
            repo_id: "repo-1".to_string(),
            role_id: role_id.to_string(),
            version,
            lifecycle_status: "active".to_string(),
            canonical_hash,
            candidate_selector: serde_json::to_value(&spec.candidate_selector)?,
            positive_conditions: serde_json::to_value(&spec.positive_conditions)?,
            negative_conditions: serde_json::to_value(&spec.negative_conditions)?,
            score: serde_json::to_value(&spec.score)?,
            provenance: json!({"source": "test"}),
            evidence: json!([]),
            metadata: json!({}),
            supersedes_rule_id: None,
        };
        insert_role_rule(relational, &record).await?;
        Ok(record)
    }

    #[tokio::test]
    async fn rename_proposal_preview_and_apply_keeps_role_identity() -> Result<()> {
        let relational = relational().await?;
        let role = seed_role(&relational).await?;

        let proposal = create_rename_role_proposal(
            &relational,
            "repo-1",
            &role.canonical_key,
            "CLI Command Dispatcher",
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(proposal.proposal_type, PROPOSAL_RENAME_ROLE);
        assert_eq!(proposal.preview_payload["affected_roles"], json!(1));

        let applied = apply_proposal(&relational, "repo-1", &proposal.proposal_id).await?;
        assert_eq!(
            applied.result_payload["new_display_name"],
            json!("CLI Command Dispatcher")
        );

        let loaded = load_role_by_id(&relational, "repo-1", &role.role_id)
            .await?
            .expect("role");
        assert_eq!(loaded.role_id, role.role_id);
        assert_eq!(loaded.display_name, "CLI Command Dispatcher");
        Ok(())
    }

    #[tokio::test]
    async fn deprecate_and_remove_proposals_invalidate_or_migrate_assignments() -> Result<()> {
        let deprecate_relational = relational().await?;
        let role = seed_role(&deprecate_relational).await?;
        let assignment_id =
            seed_assignment(&deprecate_relational, "artefact-1", &role.role_id).await?;

        let deprecate = create_deprecate_role_proposal(
            &deprecate_relational,
            "repo-1",
            &role.canonical_key,
            None,
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(deprecate.preview_payload["affected_assignments"], json!(1));
        let deprecated =
            apply_proposal(&deprecate_relational, "repo-1", &deprecate.proposal_id).await?;
        assert_eq!(
            deprecated.result_payload["invalidated_assignments"],
            json!(1)
        );
        let invalidated = load_assignment_by_id(&deprecate_relational, "repo-1", &assignment_id)
            .await?
            .expect("assignment");
        assert_eq!(invalidated.status, "needs_review");

        let remove_relational = relational().await?;
        let source = seed_role(&remove_relational).await?;
        let target = seed_role_with_key(&remove_relational, "web_ui", "Web UI").await?;
        let assignment_id =
            seed_assignment(&remove_relational, "artefact-1", &source.role_id).await?;
        let remove = create_remove_role_proposal(
            &remove_relational,
            "repo-1",
            &source.canonical_key,
            Some(&target.canonical_key),
            json!({"source": "test"}),
        )
        .await?;
        let removed = apply_proposal(&remove_relational, "repo-1", &remove.proposal_id).await?;
        assert_eq!(removed.result_payload["migrated_assignments"], json!(1));
        let migrated = load_assignment_by_id(&remove_relational, "repo-1", &assignment_id)
            .await?
            .expect("migrated assignment");
        assert_eq!(migrated.status, "migrated");
        assert_eq!(removed.migration_records.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn split_preview_reports_reclassification_and_rule_edit_preview_reports_diff()
    -> Result<()> {
        let relational = relational().await?;
        let role = seed_role(&relational).await?;
        seed_assignment(&relational, "artefact-1", &role.role_id).await?;

        let split = create_split_role_proposal(
            &relational,
            "repo-1",
            &role.canonical_key,
            RoleSplitSpecFile {
                target_roles: vec![crate::capability_packs::architecture_graph::roles::taxonomy::RoleSplitTargetRole {
                    canonical_key: "cli_command".to_string(),
                    display_name: "CLI Command".to_string(),
                    description: String::new(),
                    family: Some("entrypoint".to_string()),
                    alias_keys: vec![],
                }],
                note: Some("split command surface".to_string()),
            },
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(
            split.preview_payload["downstream_review_work"]["reclassification_required"],
            json!(true)
        );
        assert_eq!(split.preview_payload["affected_rules"], json!(0));

        let existing_rule = seed_rule(
            &relational,
            &role.role_id,
            RoleRuleCandidateSelector {
                path_prefixes: vec!["src/cli".to_string()],
                ..Default::default()
            },
        )
        .await?;

        let rule_preview = create_rule_edit_proposal(
            &relational,
            &gateway(),
            "repo-1",
            &existing_rule.rule_id,
            RuleSpecFile {
                role_ref: role.canonical_key.clone(),
                candidate_selector: RoleRuleCandidateSelector {
                    path_prefixes: vec!["src/domain".to_string()],
                    ..Default::default()
                },
                positive_conditions: vec![],
                negative_conditions: vec![],
                score: super::super::taxonomy::RoleRuleScore {
                    base_confidence: Some(0.8),
                    weight: None,
                },
                evidence: json!([]),
                metadata: json!({}),
            },
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(
            rule_preview.preview_payload["added_matches"],
            json!(["artefact-2"])
        );
        assert_eq!(
            rule_preview.preview_payload["removed_matches"],
            json!(["artefact-1"])
        );
        Ok(())
    }

    #[tokio::test]
    async fn merge_preview_counts_impacted_work_and_apply_creates_auditable_migration() -> Result<()>
    {
        let relational = relational().await?;
        let source = seed_role(&relational).await?;
        let target = seed_role_with_key(&relational, "web_ui", "Web UI").await?;
        let assignment_id = seed_assignment(&relational, "artefact-1", &source.role_id).await?;

        let proposal = create_merge_role_proposal(
            &relational,
            "repo-1",
            &source.canonical_key,
            &target.canonical_key,
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(proposal.preview_payload["affected_roles"], json!(2));
        assert_eq!(proposal.preview_payload["affected_assignments"], json!(1));
        assert_eq!(
            proposal.preview_payload["downstream_review_work"]["safe_migration_available"],
            json!(true)
        );

        let applied = apply_proposal(&relational, "repo-1", &proposal.proposal_id).await?;
        assert_eq!(applied.result_payload["migrated_assignments"], json!(1));
        assert_eq!(applied.migration_records.len(), 1);
        let migrated = load_assignment_by_id(&relational, "repo-1", &assignment_id)
            .await?
            .expect("source assignment");
        assert_eq!(migrated.status, "migrated");
        Ok(())
    }

    #[tokio::test]
    async fn alias_create_and_split_apply_workflows_persist_reviewable_changes() -> Result<()> {
        let alias_relational = relational().await?;
        let role = seed_role(&alias_relational).await?;

        let alias = create_alias_proposal(
            &alias_relational,
            "repo-1",
            &role.canonical_key,
            "cli_surface",
            json!({"source": "test"}),
        )
        .await?;
        let applied_alias = apply_proposal(&alias_relational, "repo-1", &alias.proposal_id).await?;
        assert_eq!(
            applied_alias.result_payload["alias_key"],
            json!("cli_surface")
        );
        let resolved = load_role_by_alias(&alias_relational, "repo-1", "cli_surface")
            .await?
            .expect("alias role");
        assert_eq!(resolved.role_id, role.role_id);

        let split_relational = relational().await?;
        let split_role = seed_role(&split_relational).await?;
        let assignment_id =
            seed_assignment(&split_relational, "artefact-1", &split_role.role_id).await?;
        let split = create_split_role_proposal(
            &split_relational,
            "repo-1",
            &split_role.canonical_key,
            RoleSplitSpecFile {
                target_roles: vec![crate::capability_packs::architecture_graph::roles::taxonomy::RoleSplitTargetRole {
                    canonical_key: "cli_command".to_string(),
                    display_name: "CLI Command".to_string(),
                    description: String::new(),
                    family: Some("entrypoint".to_string()),
                    alias_keys: vec!["command_surface".to_string()],
                }],
                note: Some("split command surface".to_string()),
            },
            json!({"source": "test"}),
        )
        .await?;
        let applied_split = apply_proposal(&split_relational, "repo-1", &split.proposal_id).await?;
        assert_eq!(applied_split.migration_records.len(), 1);
        let invalidated = load_assignment_by_id(&split_relational, "repo-1", &assignment_id)
            .await?
            .expect("split assignment");
        assert_eq!(invalidated.status, "needs_review");
        Ok(())
    }

    #[tokio::test]
    async fn rule_disable_only_invalidates_assignments_linked_to_that_rule() -> Result<()> {
        let relational = relational().await?;
        let role = seed_role(&relational).await?;
        let rule = seed_rule(
            &relational,
            &role.role_id,
            RoleRuleCandidateSelector {
                path_prefixes: vec!["src/cli".to_string()],
                ..Default::default()
            },
        )
        .await?;
        let rule_assignment_id = seed_assignment_with_rule(
            &relational,
            "artefact-1",
            &role.role_id,
            Some(&rule.rule_id),
        )
        .await?;
        let manual_assignment_id =
            seed_assignment_with_rule(&relational, "artefact-2", &role.role_id, None).await?;

        let disable = create_rule_disable_proposal(
            &relational,
            "repo-1",
            &rule.rule_id,
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(disable.preview_payload["affected_assignments"], json!(1));

        let applied = apply_proposal(&relational, "repo-1", &disable.proposal_id).await?;
        assert_eq!(applied.result_payload["invalidated_assignments"], json!(1));
        let invalidated = load_assignment_by_id(&relational, "repo-1", &rule_assignment_id)
            .await?
            .expect("rule assignment");
        assert_eq!(invalidated.status, "needs_review");
        let untouched = load_assignment_by_id(&relational, "repo-1", &manual_assignment_id)
            .await?
            .expect("manual assignment");
        assert_eq!(untouched.status, "active");
        Ok(())
    }
}
