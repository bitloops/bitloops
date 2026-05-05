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

    #[derive(Default)]
    struct CapturingWorkplaneGateway {
        jobs: std::sync::Mutex<Vec<crate::host::capability_host::gateways::CapabilityWorkplaneJob>>,
    }

    impl CapturingWorkplaneGateway {
        fn jobs(&self) -> Vec<crate::host::capability_host::gateways::CapabilityWorkplaneJob> {
            self.jobs.lock().expect("lock captured jobs").clone()
        }
    }

    impl crate::host::capability_host::gateways::CapabilityWorkplaneGateway
        for CapturingWorkplaneGateway
    {
        fn enqueue_jobs(
            &self,
            jobs: Vec<crate::host::capability_host::gateways::CapabilityWorkplaneJob>,
        ) -> Result<crate::host::capability_host::gateways::CapabilityWorkplaneEnqueueResult>
        {
            let inserted_jobs = jobs.len() as u64;
            self.jobs.lock().expect("lock captured jobs").extend(jobs);
            Ok(
                crate::host::capability_host::gateways::CapabilityWorkplaneEnqueueResult {
                    inserted_jobs,
                    updated_jobs: 0,
                },
            )
        }

        fn mailbox_status(
            &self,
        ) -> Result<
            std::collections::BTreeMap<
                String,
                crate::host::capability_host::gateways::CapabilityMailboxStatus,
            >,
        > {
            Ok(std::collections::BTreeMap::new())
        }
    }

    struct FakeStructuredService {
        response: serde_json::Value,
    }

    impl crate::host::inference::StructuredGenerationService for FakeStructuredService {
        fn descriptor(&self) -> String {
            "fake:model".to_string()
        }

        fn generate(
            &self,
            _request: crate::host::inference::StructuredGenerationRequest,
        ) -> Result<serde_json::Value> {
            Ok(self.response.clone())
        }
    }

    struct FakeInferenceGateway {
        response: serde_json::Value,
    }

    impl crate::host::inference::InferenceGateway for FakeInferenceGateway {
        fn embeddings(
            &self,
            slot_name: &str,
        ) -> Result<std::sync::Arc<dyn crate::host::inference::EmbeddingService>> {
            anyhow::bail!("no embeddings for slot `{slot_name}`")
        }

        fn text_generation(
            &self,
            slot_name: &str,
        ) -> Result<std::sync::Arc<dyn crate::host::inference::TextGenerationService>> {
            anyhow::bail!("no text generation for slot `{slot_name}`")
        }

        fn structured_generation(
            &self,
            _slot_name: &str,
        ) -> Result<std::sync::Arc<dyn crate::host::inference::StructuredGenerationService>>
        {
            Ok(std::sync::Arc::new(FakeStructuredService {
                response: self.response.clone(),
            }))
        }

        fn has_slot(&self, _slot_name: &str) -> bool {
            true
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
        seed_assignment_with_status(
            relational,
            artefact_id,
            role_id,
            rule_id,
            taxonomy::AssignmentStatus::Active,
        )
        .await
    }

    async fn seed_assignment_with_status(
        relational: &RelationalStorage,
        artefact_id: &str,
        role_id: &str,
        rule_id: Option<&str>,
        status: taxonomy::AssignmentStatus,
    ) -> Result<String> {
        let target = taxonomy::RoleTarget::artefact(
            artefact_id.to_string(),
            format!("symbol-{artefact_id}"),
            format!("src/{artefact_id}.rs"),
        );
        let assignment_id = taxonomy::assignment_id("repo-1", role_id, &target);
        let evidence = rule_id
            .map(|rule_id| {
                json!([{
                    "ruleId": rule_id,
                    "ruleVersion": 1,
                }])
            })
            .unwrap_or_else(|| json!([]));
        upsert_assignment(
            relational,
            &taxonomy::ArchitectureRoleAssignment {
                assignment_id: assignment_id.clone(),
                repo_id: "repo-1".to_string(),
                role_id: role_id.to_string(),
                target,
                priority: taxonomy::AssignmentPriority::Primary,
                status,
                source: taxonomy::AssignmentSource::Rule,
                confidence: 0.9,
                evidence,
                provenance: json!({"source": "test"}),
                classifier_version: "test".to_string(),
                rule_version: Some(1),
                generation_seq: 1,
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
        let invalidated =
            load_current_assignment_by_id(&deprecate_relational, "repo-1", &assignment_id)
                .await?
                .expect("assignment");
        assert_eq!(invalidated.status, taxonomy::AssignmentStatus::NeedsReview);

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
        let migrated = load_current_assignment_by_id(&remove_relational, "repo-1", &assignment_id)
            .await?
            .expect("migrated assignment");
        assert_eq!(migrated.status, taxonomy::AssignmentStatus::Stale);
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
    async fn draft_rule_proposal_round_trips_request_and_persists_fact_backed_rule() -> Result<()> {
        let relational = relational().await?;
        let role = seed_role(&relational).await?;
        let proposal = create_rule_draft_proposal(
            &relational,
            &gateway(),
            "repo-1",
            RuleSpecFile {
                role_ref: role.canonical_key.clone(),
                candidate_selector: RoleRuleCandidateSelector {
                    path_prefixes: vec!["src/cli".to_string()],
                    languages: vec!["rust".to_string(), "typescript".to_string()],
                    ..Default::default()
                },
                positive_conditions: vec![taxonomy::RoleRuleCondition {
                    kind: "path_contains".to_string(),
                    value: json!("commands"),
                }],
                negative_conditions: vec![],
                score: taxonomy::RoleRuleScore {
                    base_confidence: Some(0.8),
                    weight: None,
                },
                evidence: json!([]),
                metadata: json!({}),
            },
            json!({"source": "test"}),
        )
        .await?;

        let stored_proposal =
            load_role_proposal_by_id(&relational, "repo-1", &proposal.proposal_id)
                .await?
                .expect("proposal");
        let request: super::super::DraftRuleRequest =
            serde_json::from_value(stored_proposal.request_payload)?;
        assert_eq!(
            request.spec.candidate_selector.languages,
            vec!["rust", "typescript"]
        );

        apply_proposal(&relational, "repo-1", &proposal.proposal_id).await?;
        let rules = load_role_rules(&relational, "repo-1", &role.role_id).await?;
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].candidate_selector,
            json!({
                "targetKinds": [],
                "pathPrefixes": ["src/cli"],
                "pathSuffixes": [],
                "requiredFacts": [],
                "requiredFactAnyGroups": [[
                    { "kind": "language", "key": "resolved", "op": "eq", "value": "rust", "score": 1.0 },
                    { "kind": "language", "key": "resolved", "op": "eq", "value": "typescript", "score": 1.0 }
                ]]
            })
        );
        assert_eq!(
            rules[0].positive_conditions,
            json!([
                { "kind": "path", "key": "full", "op": "contains", "value": "commands", "score": 1.0 }
            ])
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
        let migrated = load_current_assignment_by_id(&relational, "repo-1", &assignment_id)
            .await?
            .expect("source assignment");
        assert_eq!(migrated.status, taxonomy::AssignmentStatus::Stale);
        Ok(())
    }

    #[tokio::test]
    async fn merge_proposal_ignores_stale_current_assignments() -> Result<()> {
        let relational = relational().await?;
        let source = seed_role(&relational).await?;
        let target = seed_role_with_key(&relational, "web_ui", "Web UI").await?;
        let stale_assignment_id = seed_assignment_with_status(
            &relational,
            "artefact-1",
            &source.role_id,
            None,
            taxonomy::AssignmentStatus::Stale,
        )
        .await?;
        let stale_assignment =
            load_current_assignment_by_id(&relational, "repo-1", &stale_assignment_id)
                .await?
                .expect("stale assignment");
        let target_assignment_id =
            taxonomy::assignment_id("repo-1", &target.role_id, &stale_assignment.target);

        let proposal = create_merge_role_proposal(
            &relational,
            "repo-1",
            &source.canonical_key,
            &target.canonical_key,
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(proposal.preview_payload["affected_assignments"], json!(0));

        let applied = apply_proposal(&relational, "repo-1", &proposal.proposal_id).await?;

        assert_eq!(applied.result_payload["migrated_assignments"], json!(0));
        let still_stale =
            load_current_assignment_by_id(&relational, "repo-1", &stale_assignment_id)
                .await?
                .expect("source assignment");
        assert_eq!(still_stale.status, taxonomy::AssignmentStatus::Stale);
        assert!(
            load_current_assignment_by_id(&relational, "repo-1", &target_assignment_id)
                .await?
                .is_none()
        );
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
        let invalidated =
            load_current_assignment_by_id(&split_relational, "repo-1", &assignment_id)
                .await?
                .expect("split assignment");
        assert_eq!(invalidated.status, taxonomy::AssignmentStatus::NeedsReview);
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
        let invalidated = load_current_assignment_by_id(&relational, "repo-1", &rule_assignment_id)
            .await?
            .expect("rule assignment");
        assert_eq!(invalidated.status, taxonomy::AssignmentStatus::NeedsReview);
        let untouched = load_current_assignment_by_id(&relational, "repo-1", &manual_assignment_id)
            .await?
            .expect("manual assignment");
        assert_eq!(untouched.status, taxonomy::AssignmentStatus::Active);
        Ok(())
    }

    #[tokio::test]
    async fn role_story_flow_connects_rule_classification_queue_llm_and_management() -> Result<()> {
        let relational = relational().await?;
        let role = seed_role(&relational).await?;
        let preview_gateway = gateway();
        let draft = create_rule_draft_proposal(
            &relational,
            &preview_gateway,
            "repo-1",
            RuleSpecFile {
                role_ref: role.canonical_key.clone(),
                candidate_selector: RoleRuleCandidateSelector {
                    path_suffixes: vec!["run.rs".to_string()],
                    languages: vec!["rust".to_string()],
                    ..Default::default()
                },
                positive_conditions: vec![taxonomy::RoleRuleCondition {
                    kind: "path_contains".to_string(),
                    value: json!("commands"),
                }],
                negative_conditions: vec![],
                score: taxonomy::RoleRuleScore {
                    base_confidence: Some(0.6),
                    weight: None,
                },
                evidence: json!([]),
                metadata: json!({}),
            },
            json!({"source": "e2e"}),
        )
        .await?;
        let drafted = apply_proposal(&relational, "repo-1", &draft.proposal_id).await?;
        let rule_id = drafted.result_payload["rule_id"]
            .as_str()
            .expect("drafted rule id")
            .to_string();
        let activate = create_rule_activate_proposal(
            &relational,
            "repo-1",
            &rule_id,
            json!({"source": "e2e"}),
        )
        .await?;
        apply_proposal(&relational, "repo-1", &activate.proposal_id).await?;

        let files = vec![crate::models::CurrentCanonicalFileRecord {
            repo_id: "repo-1".to_string(),
            path: "src/cli/commands/run.rs".to_string(),
            analysis_mode: "code".to_string(),
            file_role: "source".to_string(),
            language: "rust".to_string(),
            resolved_language: "rust".to_string(),
            effective_content_id: "content-1".to_string(),
            parser_version: "parser".to_string(),
            extractor_version: "extractor".to_string(),
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        }];
        let classification =
            crate::capability_packs::architecture_graph::roles::classifier::classify_architecture_roles_for_current_state(
                &relational,
                crate::capability_packs::architecture_graph::roles::classifier::ArchitectureRoleClassificationInput {
                    repo_id: "repo-1",
                    generation_seq: 42,
                    affected_paths: std::collections::BTreeSet::from([
                        "src/cli/commands/run.rs".to_string()
                    ]),
                    removed_paths: std::collections::BTreeSet::new(),
                    files: &files,
                    artefacts: &preview_gateway.artefacts,
                    dependency_edges: &[],
                },
            )
            .await?;
        assert_eq!(classification.metrics.adjudication_candidates, 1);
        assert_eq!(classification.adjudication_requests.len(), 1);
        let request = &classification.adjudication_requests[0];
        assert_eq!(
            request.reason,
            crate::capability_packs::architecture_graph::roles::AdjudicationReason::LowConfidence
        );
        assert_eq!(request.candidate_role_ids, vec![role.role_id.clone()]);

        let queue =
            crate::capability_packs::architecture_graph::roles::InMemoryRoleAdjudicationQueueStore::new();
        let workplane = CapturingWorkplaneGateway::default();
        let enqueue_metrics =
            crate::capability_packs::architecture_graph::roles::enqueue_adjudication_requests(
                &classification.adjudication_requests,
                &workplane,
                &queue,
            )?;
        assert_eq!(enqueue_metrics.enqueued, 1);
        assert_eq!(workplane.jobs().len(), 1);

        let inference = FakeInferenceGateway {
            response: json!({
                "outcome": "assigned",
                "assignments": [{
                    "role_id": role.role_id.clone(),
                    "confidence": 0.93,
                    "primary": true,
                    "evidence": ["src/cli/commands/run.rs"]
                }],
                "confidence": 0.93,
                "evidence": ["rule signals and facts"],
                "reasoning_summary": "command dispatcher role is clear",
                "rule_suggestions": []
            }),
        };
        let taxonomy =
            crate::capability_packs::architecture_graph::roles::DbRoleTaxonomyReader::new(
                &relational,
            );
        let facts =
            crate::capability_packs::architecture_graph::roles::DbRoleFactsReader::new(&relational);
        let writer =
            crate::capability_packs::architecture_graph::roles::DbRoleAssignmentWriter::new(
                &relational,
            );
        let services =
            crate::capability_packs::architecture_graph::roles::RoleAdjudicationServices {
                queue: &queue,
                taxonomy: &taxonomy,
                facts: &facts,
                writer: &writer,
            };
        let write_outcome =
            crate::capability_packs::architecture_graph::roles::run_adjudication_request(
                request,
                &inference,
                std::path::Path::new("."),
                &services,
            )?;
        assert!(write_outcome.persisted);

        let target = taxonomy::RoleTarget::file("src/cli/commands/run.rs");
        let assignment_id = taxonomy::assignment_id("repo-1", &role.role_id, &target);
        let adjudicated_assignment =
            load_current_assignment_by_id(&relational, "repo-1", &assignment_id)
                .await?
                .expect("llm assignment");
        assert_eq!(
            adjudicated_assignment.source,
            taxonomy::AssignmentSource::Llm
        );
        assert_eq!(
            adjudicated_assignment.status,
            taxonomy::AssignmentStatus::Active
        );

        let next_classification =
            crate::capability_packs::architecture_graph::roles::classifier::classify_architecture_roles_for_current_state(
                &relational,
                crate::capability_packs::architecture_graph::roles::classifier::ArchitectureRoleClassificationInput {
                    repo_id: "repo-1",
                    generation_seq: 43,
                    affected_paths: std::collections::BTreeSet::from([
                        "src/cli/commands/run.rs".to_string()
                    ]),
                    removed_paths: std::collections::BTreeSet::new(),
                    files: &files,
                    artefacts: &preview_gateway.artefacts,
                    dependency_edges: &[],
                },
            )
            .await?;
        assert!(next_classification.adjudication_requests.is_empty());
        assert_eq!(next_classification.metrics.adjudication_candidates, 0);
        let preserved_assignment =
            load_current_assignment_by_id(&relational, "repo-1", &assignment_id)
                .await?
                .expect("preserved llm assignment");
        assert_eq!(preserved_assignment.source, taxonomy::AssignmentSource::Llm);
        assert_eq!(
            preserved_assignment.status,
            taxonomy::AssignmentStatus::Active
        );

        let deprecate = create_deprecate_role_proposal(
            &relational,
            "repo-1",
            &role.canonical_key,
            None,
            json!({"source": "e2e"}),
        )
        .await?;
        assert_eq!(deprecate.preview_payload["affected_assignments"], json!(1));
        let deprecated = apply_proposal(&relational, "repo-1", &deprecate.proposal_id).await?;
        assert_eq!(
            deprecated.result_payload["invalidated_assignments"],
            json!(1)
        );
        let managed_assignment =
            load_current_assignment_by_id(&relational, "repo-1", &assignment_id)
                .await?
                .expect("managed assignment");
        assert_eq!(
            managed_assignment.status,
            taxonomy::AssignmentStatus::NeedsReview
        );
        Ok(())
    }
}
