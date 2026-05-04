use anyhow::Result;
use serde_json::json;

use super::snapshot_support::{build_snapshot_stage_data, missing_world_payload};
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::types::CODECITY_WORLD_STAGE_ID;
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
            let args = request
                .payload
                .get("args")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let config = CodeCityConfig::from_stage_args(&args)?;
            let limit = request.limit().unwrap_or(500);
            let data =
                match build_snapshot_stage_data(CODECITY_WORLD_STAGE_ID, &request, ctx, config)? {
                    Ok(data) => data,
                    Err(response) => return Ok(response),
                };
            let mut world = data.world.unwrap_or_else(|| {
                missing_world_payload(
                    &data.repo_id,
                    data.project_path.as_deref(),
                    &data.snapshot_key,
                    data.snapshot_status.clone(),
                    &data.config,
                )
            });
            if world.buildings.len() > limit {
                world.buildings.truncate(limit);
            }

            let human = if world.status == "empty" {
                format!(
                    "codecity world for repo {}: no eligible files",
                    world.repo_id
                )
            } else if world.snapshot_status.state.as_str() == "missing" {
                format!(
                    "codecity world for repo {} snapshot {}: missing",
                    world.repo_id, world.snapshot_status.snapshot_key
                )
            } else {
                format!(
                    "codecity world for repo {} snapshot {}: state={}, stale={}, files={}, artefacts={}, dependencies={}, returned_buildings={}",
                    world.repo_id,
                    world.snapshot_status.snapshot_key,
                    world.snapshot_status.state.as_str(),
                    world.snapshot_status.stale,
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
    async fn stage_returns_missing_payload_when_no_snapshot_is_available() -> Result<()> {
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

        assert_eq!(response.payload["status"], "missing");
        assert_eq!(response.payload["snapshot_status"]["state"], "missing");
        assert_eq!(response.payload["summary"]["file_count"], 0);
        assert_eq!(
            response.payload["diagnostics"][0]["code"],
            "codecity.snapshot.missing"
        );
        Ok(())
    }

    #[tokio::test]
    async fn stage_returns_scoped_missing_snapshot_without_building_on_read() -> Result<()> {
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

        assert_eq!(response.payload["status"], "missing");
        assert_eq!(response.payload["snapshot_status"]["state"], "missing");
        assert_eq!(
            response.payload["snapshot_status"]["project_path"],
            "packages/api"
        );
        assert_eq!(response.payload["summary"]["file_count"], 0);
        assert_eq!(
            response.payload["dependency_arcs"].as_array().map(Vec::len),
            Some(0)
        );
        assert_eq!(
            response.payload["buildings"].as_array().map(Vec::len),
            Some(0)
        );
        Ok(())
    }

    #[tokio::test]
    async fn stage_reports_missing_snapshot_without_read_time_source_loading() -> Result<()> {
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

        assert_eq!(response.payload["status"], "missing");
        assert_eq!(
            response.payload["diagnostics"][0]["code"],
            "codecity.snapshot.missing"
        );
        Ok(())
    }
}
