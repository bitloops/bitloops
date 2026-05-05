#[cfg(test)]
mod deterministic_tests {
    use anyhow::Result;
    use serde_json::json;
    use tempfile::tempdir;

    use super::super::*;
    use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
    use crate::host::devql::RelationalStorage;

    fn test_role() -> ArchitectureRoleRecord {
        ArchitectureRoleRecord {
            role_id: deterministic_role_id("repo-1", "domain_owner"),
            repo_id: "repo-1".to_string(),
            canonical_key: "domain_owner".to_string(),
            display_name: "Domain Owner".to_string(),
            description: "Owns the domain".to_string(),
            family: Some("domain".to_string()),
            lifecycle_status: "active".to_string(),
            provenance: json!({"source": "test"}),
            evidence: json!([{ "path": "src/payments" }]),
            metadata: json!({"scope": "payments"}),
        }
    }

    fn test_alias(role_id: &str, alias: &str) -> ArchitectureRoleAliasRecord {
        ArchitectureRoleAliasRecord {
            alias_id: deterministic_alias_id("repo-1", alias),
            repo_id: "repo-1".to_string(),
            role_id: role_id.to_string(),
            alias_key: alias.to_string(),
            alias_normalized: normalize_role_alias(alias),
            source_kind: "manual".to_string(),
            metadata: json!({"source": "test"}),
        }
    }

    fn test_rule(role_id: &str) -> ArchitectureRoleRuleRecord {
        let version = 1;
        let canonical_hash = "rule-hash-1";
        ArchitectureRoleRuleRecord {
            rule_id: deterministic_rule_id("repo-1", role_id, version, canonical_hash),
            repo_id: "repo-1".to_string(),
            role_id: role_id.to_string(),
            version,
            lifecycle_status: "draft".to_string(),
            canonical_hash: canonical_hash.to_string(),
            candidate_selector: json!({"path_prefixes": ["src/payments"]}),
            positive_conditions: json!([{ "kind": "path_contains", "value": "payments" }]),
            negative_conditions: json!([]),
            score: json!({ "base_confidence": 0.82 }),
            provenance: json!({"source": "seed"}),
            evidence: json!(["src/payments/service.rs"]),
            metadata: json!({"reviewable": true}),
            supersedes_rule_id: None,
        }
    }

    fn test_assignment(role_id: &str) -> ArchitectureRoleAssignmentRecord {
        ArchitectureRoleAssignmentRecord {
            assignment_id: deterministic_assignment_id("repo-1", "artefact-1", role_id),
            repo_id: "repo-1".to_string(),
            artefact_id: "artefact-1".to_string(),
            role_id: role_id.to_string(),
            source_kind: "deterministic_rule".to_string(),
            confidence: 0.91,
            status: "active".to_string(),
            status_reason: String::new(),
            rule_id: Some("rule-1".to_string()),
            migration_id: None,
            migrated_to_assignment_id: None,
            provenance: json!({"source": "test"}),
            evidence: json!(["src/payments/service.rs"]),
            metadata: json!({"ticket": "ARCH-1"}),
        }
    }

    fn test_proposal() -> ArchitectureRoleProposalRecord {
        ArchitectureRoleProposalRecord {
            proposal_id: "proposal-1".to_string(),
            repo_id: "repo-1".to_string(),
            proposal_type: "rename_role".to_string(),
            status: "draft".to_string(),
            request_payload: json!({"role_id": "role-1", "display_name": "Payments Domain Owner"}),
            preview_payload: json!({"affected_assignments": 1}),
            result_payload: json!({}),
            provenance: json!({"source": "cli"}),
            applied_at: None,
        }
    }

    fn test_migration(proposal_id: &str) -> ArchitectureRoleAssignmentMigrationRecord {
        ArchitectureRoleAssignmentMigrationRecord {
            migration_id: deterministic_migration_id("repo-1", proposal_id, "merge_roles"),
            repo_id: "repo-1".to_string(),
            proposal_id: proposal_id.to_string(),
            migration_type: "merge_roles".to_string(),
            status: "applied".to_string(),
            source_role_id: Some("role-1".to_string()),
            target_role_id: Some("role-2".to_string()),
            summary: json!({"migrated_assignments": 1}),
        }
    }

    async fn sqlite_relational_with_schema() -> Result<RelationalStorage> {
        let temp = tempdir()?;
        let sqlite_path = temp.path().join("roles.sqlite");
        rusqlite::Connection::open(&sqlite_path)?;
        let relational = RelationalStorage::local_only(sqlite_path);
        relational
            .exec(architecture_graph_sqlite_schema_sql())
            .await?;
        std::mem::forget(temp);
        Ok(relational)
    }

    #[tokio::test]
    async fn role_and_alias_round_trip_and_conflict_detection() -> Result<()> {
        let relational = sqlite_relational_with_schema().await?;
        let role = test_role();
        let persisted = upsert_role(&relational, &role).await?;
        assert_eq!(persisted, role);

        let loaded = load_role_by_canonical_key(&relational, "repo-1", "domain_owner")
            .await?
            .expect("role");
        assert_eq!(loaded, role);

        let alias = test_alias(&role.role_id, "Domain Owner");
        assert_eq!(create_role_alias(&relational, &alias).await?, Ok(()));

        let by_alias = load_role_by_alias(&relational, "repo-1", "domain owner")
            .await?
            .expect("role by alias");
        assert_eq!(by_alias.role_id, role.role_id);

        let conflicting = ArchitectureRoleAliasRecord {
            alias_id: "alias-conflict".to_string(),
            role_id: "role-2".to_string(),
            ..alias.clone()
        };
        assert_eq!(
            create_role_alias(&relational, &conflicting).await?,
            Err(AliasConflict::AlreadyAssignedToDifferentRole {
                alias: alias.alias_normalized,
                existing_role_id: role.role_id,
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn role_rule_assignment_proposal_and_migration_round_trip() -> Result<()> {
        let relational = sqlite_relational_with_schema().await?;
        let role = test_role();
        upsert_role(&relational, &role).await?;

        let rule = test_rule(&role.role_id);
        insert_role_rule(&relational, &rule).await?;
        let rules = load_role_rules(&relational, "repo-1", &role.role_id).await?;
        assert_eq!(rules, vec![rule.clone()]);

        let assignment = test_assignment(&role.role_id);
        insert_role_assignment(&relational, &assignment).await?;
        assert!(
            mark_assignment_invalidated(
                &relational,
                "repo-1",
                &assignment.assignment_id,
                "needs review after role change",
            )
            .await?
        );
        assert!(
            mark_assignment_migrated(
                &relational,
                "repo-1",
                &assignment.assignment_id,
                "assignment-2",
                Some("migration-1"),
            )
            .await?
        );

        let loaded_assignment =
            load_assignment_by_id(&relational, "repo-1", &assignment.assignment_id)
                .await?
                .expect("assignment");
        assert_eq!(loaded_assignment.status, "migrated");
        assert_eq!(
            loaded_assignment.migrated_to_assignment_id.as_deref(),
            Some("assignment-2")
        );

        let proposal = test_proposal();
        insert_role_proposal(&relational, &proposal).await?;
        let loaded_proposal =
            load_role_proposal_by_id(&relational, "repo-1", &proposal.proposal_id)
                .await?
                .expect("proposal");
        assert_eq!(loaded_proposal.preview_payload, proposal.preview_payload);

        assert!(
            mark_role_proposal_applied(
                &relational,
                "repo-1",
                &proposal.proposal_id,
                &json!({"applied": true}),
            )
            .await?
        );

        let migration = test_migration(&proposal.proposal_id);
        insert_assignment_migration_record(&relational, &migration).await?;
        let migrations =
            list_assignment_migrations_for_proposal(&relational, "repo-1", &proposal.proposal_id)
                .await?;
        assert_eq!(migrations, vec![migration]);
        Ok(())
    }
}
