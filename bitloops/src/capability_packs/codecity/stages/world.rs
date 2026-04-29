use anyhow::Result;
use serde_json::{Value, json};

use crate::capability_packs::codecity::services::architecture::analyse_codecity_architecture;
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::health::apply_health_overlay;
use crate::capability_packs::codecity::services::phase4::enrich_world_with_phase4;
use crate::capability_packs::codecity::services::source_graph::load_current_source_graph;
use crate::capability_packs::codecity::services::world::build_codecity_world;
use crate::capability_packs::codecity::storage::SqliteCodeCityRepository;
use crate::capability_packs::codecity::types::{
    CODECITY_WORLD_STAGE_ID, CodeCityDiagnostic, codecity_current_scope_required_stage_response,
    codecity_source_data_unavailable_stage_response,
};
use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

pub struct CodeCityWorldStageHandler;

impl StageHandler for CodeCityWorldStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, Result<StageResponse>> {
        Box::pin(async move {
            let repo_id = request
                .payload
                .get("query_context")
                .and_then(|query_context| query_context.get("repo_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(ctx.repo().repo_id.as_str())
                .to_string();

            let resolved_commit = request
                .payload
                .get("query_context")
                .and_then(|query_context| query_context.get("resolved_commit_sha"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if resolved_commit.is_some() {
                return Ok(codecity_current_scope_required_stage_response(
                    CODECITY_WORLD_STAGE_ID,
                ));
            }

            let project_path = request
                .payload
                .get("query_context")
                .and_then(|query_context| query_context.get("project_path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != ".")
                .map(str::to_string);

            let args = request
                .payload
                .get("args")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let config = CodeCityConfig::from_stage_args(&args)?;
            let limit = request.limit().unwrap_or(500);

            let source = match load_current_source_graph(
                ctx.host_relational(),
                &repo_id,
                project_path.as_deref(),
                &config,
            ) {
                Ok(source) => source,
                Err(err) if is_source_data_unavailable_error(&err) => {
                    return Ok(codecity_source_data_unavailable_stage_response(
                        CODECITY_WORLD_STAGE_ID,
                        format!("{err:#}"),
                    ));
                }
                Err(err) => return Err(err),
            };

            let current_head = ctx
                .git_history()
                .resolve_head(ctx.repo_root())
                .unwrap_or(None);
            let mut world = build_codecity_world(
                &source,
                &repo_id,
                current_head,
                config.clone(),
                ctx.repo_root(),
            )?;
            let codecity_repo = SqliteCodeCityRepository::open_for_repo_root(ctx.repo_root())
                .and_then(|repo| {
                    repo.initialise_schema()?;
                    Ok(repo)
                });
            let loaded_health = if config.include_health {
                match codecity_repo.as_ref() {
                    Ok(repo) => repo.try_apply_current_snapshot(&mut world)?,
                    Err(_) => false,
                }
            } else {
                false
            };
            if !loaded_health {
                apply_health_overlay(
                    &mut world,
                    &source,
                    &config,
                    ctx.repo_root(),
                    ctx.git_history(),
                    ctx.test_harness_store(),
                )?;
                if config.include_health {
                    match codecity_repo.as_ref() {
                        Ok(repo) => {
                            if let Err(err) = repo.replace_current_snapshot(&world) {
                                world.diagnostics.push(CodeCityDiagnostic {
                                    code: "codecity.health.persistence_unavailable".to_string(),
                                    severity: "warning".to_string(),
                                    message: format!(
                                        "CodeCity health was computed but could not be persisted: {err:#}"
                                    ),
                                    path: None,
                                    boundary_id: None,
                                });
                            }
                        }
                        Err(err) => {
                            world.diagnostics.push(CodeCityDiagnostic {
                                code: "codecity.health.persistence_unavailable".to_string(),
                                severity: "warning".to_string(),
                                message: format!(
                                    "CodeCity health was computed but no snapshot store was available: {err:#}"
                                ),
                                path: None,
                                boundary_id: None,
                            });
                        }
                    }
                }
            }
            let analysis = analyse_codecity_architecture(&source, &config, ctx.repo_root());
            let phase4_snapshot = enrich_world_with_phase4(&source, &analysis, &mut world, &config);
            match codecity_repo.as_ref() {
                Ok(repo) => {
                    if let Err(err) = repo.replace_phase4_snapshot(&phase4_snapshot) {
                        world.diagnostics.push(CodeCityDiagnostic {
                            code: "codecity.phase4.persistence_unavailable".to_string(),
                            severity: "warning".to_string(),
                            message: format!(
                                "CodeCity architecture diagnostics were computed but could not be persisted: {err:#}"
                            ),
                            path: None,
                            boundary_id: None,
                        });
                    }
                }
                Err(err) => {
                    world.diagnostics.push(CodeCityDiagnostic {
                        code: "codecity.phase4.persistence_unavailable".to_string(),
                        severity: "warning".to_string(),
                        message: format!(
                            "CodeCity architecture diagnostics were computed but no snapshot store was available: {err:#}"
                        ),
                        path: None,
                        boundary_id: None,
                    });
                }
            }
            if world.buildings.len() > limit {
                world.buildings.truncate(limit);
            }

            let human = if world.status == "empty" {
                format!("codecity world for repo {repo_id}: no eligible files")
            } else {
                format!(
                    "codecity world for repo {}: files={}, artefacts={}, dependencies={}, returned_buildings={}",
                    world.repo_id,
                    world.summary.file_count,
                    world.summary.artefact_count,
                    world.summary.dependency_count,
                    world.buildings.len()
                )
            };

            Ok(StageResponse::new(serde_json::to_value(world)?, human))
        })
    }
}

fn is_source_data_unavailable_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    message.contains("run DevQL sync first")
        || message.contains("current_file_state")
        || message.contains("artefacts_current")
        || message.contains("artefact_edges_current")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::Result;
    use serde_json::json;

    use super::CodeCityWorldStageHandler;
    use crate::host::capability_host::gateways::{CanonicalGraphGateway, RelationalGateway};
    use crate::host::capability_host::runtime_contexts::LocalCanonicalGraphGateway;
    use crate::host::capability_host::{CapabilityExecutionContext, StageHandler, StageRequest};
    use crate::host::devql::RepoIdentity;
    use crate::models::{
        CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
        ProductionArtefact,
    };

    struct FakeRelationalGateway {
        files: Vec<CurrentCanonicalFileRecord>,
        artefacts: Vec<CurrentCanonicalArtefactRecord>,
        edges: Vec<CurrentCanonicalEdgeRecord>,
        fail_with: Option<String>,
    }

    impl RelationalGateway for FakeRelationalGateway {
        fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
            unreachable!("checkpoint resolution is not used by codecity stage tests")
        }

        fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
            unreachable!("artefact_exists is not used by codecity stage tests")
        }

        fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
            unreachable!("historical loads are not used by codecity stage tests")
        }

        fn load_current_canonical_files(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<CurrentCanonicalFileRecord>> {
            if let Some(message) = self.fail_with.as_ref() {
                return Err(anyhow::anyhow!(message.clone()));
            }
            Ok(self.files.clone())
        }

        fn load_current_canonical_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<CurrentCanonicalArtefactRecord>> {
            if let Some(message) = self.fail_with.as_ref() {
                return Err(anyhow::anyhow!(message.clone()));
            }
            Ok(self.artefacts.clone())
        }

        fn load_current_canonical_edges(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<CurrentCanonicalEdgeRecord>> {
            if let Some(message) = self.fail_with.as_ref() {
                return Err(anyhow::anyhow!(message.clone()));
            }
            Ok(self.edges.clone())
        }

        fn load_current_production_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<ProductionArtefact>> {
            unreachable!("production artefacts are not used by codecity stage tests")
        }

        fn load_production_artefacts(&self, _commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
            unreachable!("production artefacts are not used by codecity stage tests")
        }

        fn load_artefacts_for_file_lines(
            &self,
            _commit_sha: &str,
            _file_path: &str,
        ) -> Result<Vec<(String, i64, i64)>> {
            unreachable!("file line lookups are not used by codecity stage tests")
        }
    }

    struct DummyExecCtx {
        repo: RepoIdentity,
        graph: LocalCanonicalGraphGateway,
        relational: FakeRelationalGateway,
    }

    impl CapabilityExecutionContext for DummyExecCtx {
        fn repo(&self) -> &RepoIdentity {
            &self.repo
        }

        fn repo_root(&self) -> &Path {
            Path::new(".")
        }

        fn graph(&self) -> &dyn CanonicalGraphGateway {
            &self.graph
        }

        fn host_relational(&self) -> &dyn RelationalGateway {
            &self.relational
        }
    }

    fn repo() -> RepoIdentity {
        RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "bitloops".to_string(),
            identity: "local/bitloops/bitloops".to_string(),
            repo_id: "repo-1".to_string(),
        }
    }

    fn file(path: &str) -> CurrentCanonicalFileRecord {
        CurrentCanonicalFileRecord {
            repo_id: "repo-1".to_string(),
            path: path.to_string(),
            analysis_mode: "code".to_string(),
            file_role: "source_code".to_string(),
            language: "typescript".to_string(),
            resolved_language: "typescript".to_string(),
            effective_content_id: format!("content::{path}"),
            parser_version: "parser-v1".to_string(),
            extractor_version: "extractor-v1".to_string(),
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        }
    }

    fn artefact(
        path: &str,
        symbol_id: &str,
        artefact_id: &str,
        canonical_kind: &str,
        parent_artefact_id: Option<&str>,
        symbol_fqn: &str,
        line_span: (i64, i64),
    ) -> CurrentCanonicalArtefactRecord {
        let (start_line, end_line) = line_span;
        CurrentCanonicalArtefactRecord {
            repo_id: "repo-1".to_string(),
            path: path.to_string(),
            content_id: format!("content::{path}"),
            symbol_id: symbol_id.to_string(),
            artefact_id: artefact_id.to_string(),
            language: "typescript".to_string(),
            extraction_fingerprint: "fingerprint".to_string(),
            canonical_kind: Some(canonical_kind.to_string()),
            language_kind: Some("fixture".to_string()),
            symbol_fqn: Some(symbol_fqn.to_string()),
            parent_symbol_id: None,
            parent_artefact_id: parent_artefact_id.map(str::to_string),
            start_line,
            end_line,
            start_byte: 0,
            end_byte: end_line * 10,
            signature: None,
            modifiers: "[]".to_string(),
            docstring: None,
        }
    }

    fn edge(
        path: &str,
        edge_id: &str,
        from_symbol_id: &str,
        from_artefact_id: &str,
        to_symbol_id: Option<&str>,
        to_artefact_id: Option<&str>,
        to_symbol_ref: Option<&str>,
    ) -> CurrentCanonicalEdgeRecord {
        CurrentCanonicalEdgeRecord {
            repo_id: "repo-1".to_string(),
            edge_id: edge_id.to_string(),
            path: path.to_string(),
            content_id: format!("content::{path}"),
            from_symbol_id: from_symbol_id.to_string(),
            from_artefact_id: from_artefact_id.to_string(),
            to_symbol_id: to_symbol_id.map(str::to_string),
            to_artefact_id: to_artefact_id.map(str::to_string),
            to_symbol_ref: to_symbol_ref.map(str::to_string),
            edge_kind: "calls".to_string(),
            language: "typescript".to_string(),
            start_line: Some(5),
            end_line: Some(5),
            metadata: "{}".to_string(),
        }
    }

    #[tokio::test]
    async fn stage_rejects_temporal_scopes() -> Result<()> {
        let handler = CodeCityWorldStageHandler;
        let mut ctx = DummyExecCtx {
            repo: repo(),
            graph: LocalCanonicalGraphGateway,
            relational: FakeRelationalGateway {
                files: Vec::new(),
                artefacts: Vec::new(),
                edges: Vec::new(),
                fail_with: None,
            },
        };

        let response = handler
            .execute(
                StageRequest::new(json!({
                    "query_context": {
                        "repo_id": "repo-1",
                        "resolved_commit_sha": "abc123",
                    }
                })),
                &mut ctx,
            )
            .await?;

        assert_eq!(response.payload["status"], "failed");
        assert_eq!(
            response.payload["reason"],
            "codecity_current_scope_required"
        );
        Ok(())
    }

    #[tokio::test]
    async fn stage_returns_empty_payload_when_no_files_are_available() -> Result<()> {
        let handler = CodeCityWorldStageHandler;
        let mut ctx = DummyExecCtx {
            repo: repo(),
            graph: LocalCanonicalGraphGateway,
            relational: FakeRelationalGateway {
                files: Vec::new(),
                artefacts: Vec::new(),
                edges: Vec::new(),
                fail_with: None,
            },
        };

        let response = handler
            .execute(
                StageRequest::new(json!({
                    "query_context": { "repo_id": "repo-1" },
                    "limit": 10
                })),
                &mut ctx,
            )
            .await?;

        assert_eq!(response.payload["status"], "empty");
        assert_eq!(response.payload["summary"]["file_count"], 0);
        Ok(())
    }

    #[tokio::test]
    async fn stage_builds_scoped_world_and_honours_limits_and_arcs() -> Result<()> {
        let handler = CodeCityWorldStageHandler;
        let mut ctx = DummyExecCtx {
            repo: repo(),
            graph: LocalCanonicalGraphGateway,
            relational: FakeRelationalGateway {
                files: vec![
                    file("packages/api/src/caller.ts"),
                    file("packages/api/src/target.ts"),
                    file("packages/web/src/page.ts"),
                ],
                artefacts: vec![
                    artefact(
                        "packages/api/src/caller.ts",
                        "file::caller",
                        "artefact::file-caller",
                        "file",
                        None,
                        "packages/api/src/caller.ts",
                        (1, 6),
                    ),
                    artefact(
                        "packages/api/src/caller.ts",
                        "sym::caller",
                        "artefact::caller",
                        "function",
                        Some("artefact::file-caller"),
                        "packages/api/src/caller.ts::caller",
                        (4, 6),
                    ),
                    artefact(
                        "packages/api/src/target.ts",
                        "file::target",
                        "artefact::file-target",
                        "file",
                        None,
                        "packages/api/src/target.ts",
                        (1, 3),
                    ),
                    artefact(
                        "packages/api/src/target.ts",
                        "sym::target",
                        "artefact::target",
                        "function",
                        Some("artefact::file-target"),
                        "packages/api/src/target.ts::target",
                        (1, 3),
                    ),
                    artefact(
                        "packages/web/src/page.ts",
                        "sym::page",
                        "artefact::page",
                        "function",
                        None,
                        "packages/web/src/page.ts::render",
                        (1, 3),
                    ),
                ],
                edges: vec![
                    edge(
                        "packages/api/src/caller.ts",
                        "edge-local",
                        "sym::caller",
                        "artefact::caller",
                        Some("sym::target"),
                        Some("artefact::target"),
                        Some("packages/api/src/target.ts::target"),
                    ),
                    edge(
                        "packages/api/src/caller.ts",
                        "edge-cross",
                        "sym::caller",
                        "artefact::caller",
                        Some("sym::page"),
                        Some("artefact::page"),
                        Some("packages/web/src/page.ts::render"),
                    ),
                ],
                fail_with: None,
            },
        };

        let response = handler
            .execute(
                StageRequest::new(json!({
                    "query_context": {
                        "repo_id": "repo-1",
                        "project_path": "packages/api",
                    },
                    "args": {
                        "include_dependency_arcs": true
                    },
                    "limit": 1
                })),
                &mut ctx,
            )
            .await?;

        assert_eq!(response.payload["status"], "ok");
        assert_eq!(response.payload["summary"]["file_count"], 3);
        assert_eq!(response.payload["summary"]["included_file_count"], 2);
        assert_eq!(response.payload["summary"]["dependency_count"], 1);
        assert_eq!(
            response.payload["dependency_arcs"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            response.payload["buildings"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            response.payload["buildings"][0]["path"],
            "packages/api/src/target.ts"
        );
        Ok(())
    }

    #[tokio::test]
    async fn stage_reports_missing_sync_source_data_as_failed() -> Result<()> {
        let handler = CodeCityWorldStageHandler;
        let mut ctx = DummyExecCtx {
            repo: repo(),
            graph: LocalCanonicalGraphGateway,
            relational: FakeRelationalGateway {
                files: Vec::new(),
                artefacts: Vec::new(),
                edges: Vec::new(),
                fail_with: Some(
                    "required DevQL sync table `current_file_state` is unavailable; run DevQL sync first."
                        .to_string(),
                ),
            },
        };

        let response = handler
            .execute(
                StageRequest::new(json!({
                    "query_context": { "repo_id": "repo-1" }
                })),
                &mut ctx,
            )
            .await?;

        assert_eq!(response.payload["status"], "failed");
        assert_eq!(
            response.payload["reason"],
            "codecity_source_data_unavailable"
        );
        Ok(())
    }
}
