mod parsing;
mod sql;

use self::parsing::{artefact_from_value, dependency_edge_from_value, file_context_from_value};
use self::sql::{
    DependencyScope, build_artefact_by_id_sql, build_child_artefacts_sql,
    build_current_artefacts_sql, build_current_dependency_sql, build_file_context_list_sql,
    build_file_context_lookup_sql, normalise_repo_relative_path, quote_devql_string,
};
use super::DevqlGraphqlContext;
use super::git_history::git_default_branch_name;
use crate::graphql::types::{
    Artefact, ArtefactFilterInput, DependencyEdge, DepsDirection, DepsFilterInput, FileContext,
};
use crate::host::devql::{execute_query_json_with_composition, sqlite_query_rows_path};
use anyhow::{Context, Result};
use serde_json::Value;

impl DevqlGraphqlContext {
    pub(crate) fn validate_repo_relative_path(
        &self,
        path: &str,
        allow_glob: bool,
    ) -> std::result::Result<String, String> {
        normalise_repo_relative_path(path, allow_glob)
    }

    pub(crate) async fn resolve_file_context(&self, path: &str) -> Result<Option<FileContext>> {
        let sql = build_file_context_lookup_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            path,
        );
        let rows = self.query_sqlite_rows(&sql).await?;
        rows.into_iter()
            .next()
            .map(file_context_from_value)
            .transpose()
    }

    pub(crate) async fn list_file_contexts(&self, glob: &str) -> Result<Vec<FileContext>> {
        let sql = build_file_context_list_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            glob,
        );
        let rows = self.query_sqlite_rows(&sql).await?;
        rows.into_iter().map(file_context_from_value).collect()
    }

    pub(crate) async fn list_artefacts(
        &self,
        path: Option<&str>,
        filter: Option<&ArtefactFilterInput>,
    ) -> Result<Vec<Artefact>> {
        if let Some(filter) = filter {
            filter
                .validate()
                .map_err(|err| anyhow::anyhow!(err.message))?;
            if filter.needs_event_backed_filter() {
                return self.list_artefacts_via_devql(path, filter).await;
            }
        }

        let sql = build_current_artefacts_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            path,
            filter,
        );
        let rows = self.query_sqlite_rows(&sql).await?;
        rows.into_iter().map(artefact_from_value).collect()
    }

    pub(crate) async fn load_artefact_by_id(&self, artefact_id: &str) -> Result<Option<Artefact>> {
        let sql = build_artefact_by_id_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            artefact_id,
        );
        let rows = self.query_sqlite_rows(&sql).await?;
        rows.into_iter().next().map(artefact_from_value).transpose()
    }

    pub(crate) async fn list_child_artefacts(
        &self,
        parent_artefact_id: &str,
    ) -> Result<Vec<Artefact>> {
        let sql = build_child_artefacts_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            parent_artefact_id,
        );
        let rows = self.query_sqlite_rows(&sql).await?;
        rows.into_iter().map(artefact_from_value).collect()
    }

    pub(crate) async fn list_file_dependency_edges(
        &self,
        path: &str,
        filter: Option<&DepsFilterInput>,
    ) -> Result<Vec<DependencyEdge>> {
        let sql = build_current_dependency_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            DependencyScope::File(path),
            filter.copied().unwrap_or_default(),
        );
        let rows = self.query_sqlite_rows(&sql).await?;
        rows.into_iter().map(dependency_edge_from_value).collect()
    }

    pub(crate) async fn list_artefact_dependency_edges(
        &self,
        artefact_id: &str,
        direction: DepsDirection,
        filter: Option<&DepsFilterInput>,
    ) -> Result<Vec<DependencyEdge>> {
        let mut filter = filter.copied().unwrap_or_default();
        filter.direction = direction;
        let sql = build_current_dependency_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            DependencyScope::Artefact(artefact_id),
            filter,
        );
        let rows = self.query_sqlite_rows(&sql).await?;
        rows.into_iter().map(dependency_edge_from_value).collect()
    }

    async fn list_artefacts_via_devql(
        &self,
        path: Option<&str>,
        filter: &ArtefactFilterInput,
    ) -> Result<Vec<Artefact>> {
        let mut stages = Vec::new();
        if let Some(path) = path {
            stages.push(format!("file({})", quote_devql_string(path)));
        }

        let mut args = Vec::new();
        if let Some(kind) = filter.kind {
            args.push(format!(
                "kind:{}",
                quote_devql_string(kind.as_devql_value())
            ));
        }
        if let Some(symbol_fqn) = filter.symbol_fqn.as_deref() {
            args.push(format!("symbol_fqn:{}", quote_devql_string(symbol_fqn)));
        }
        if let Some(lines) = filter.lines.as_ref() {
            args.push(format!("lines:{}..{}", lines.start, lines.end));
        }
        if let Some(agent) = filter.agent.as_deref() {
            args.push(format!("agent:{}", quote_devql_string(agent)));
        }
        if let Some(since) = filter.since.as_ref() {
            args.push(format!("since:{}", quote_devql_string(since.as_str())));
        }

        if args.is_empty() {
            stages.push("artefacts()".to_string());
        } else {
            stages.push(format!("artefacts({})", args.join(",")));
        }
        stages.push(format!("limit({})", super::GRAPHQL_DEVQL_SCAN_LIMIT));

        let cfg = self.config.as_ref().with_context(|| {
            self.config_error
                .clone()
                .unwrap_or_else(|| "DevQL configuration unavailable".to_string())
        })?;
        let result = execute_query_json_with_composition(cfg, &stages.join("->"), None).await?;
        let rows = result
            .as_array()
            .cloned()
            .with_context(|| "DevQL artefact query returned a non-array payload")?;
        rows.into_iter().map(artefact_from_value).collect()
    }

    async fn query_sqlite_rows(&self, sql: &str) -> Result<Vec<Value>> {
        let sqlite_path = self.devql_sqlite_path()?;
        sqlite_query_rows_path(&sqlite_path, sql).await
    }

    fn devql_sqlite_path(&self) -> Result<std::path::PathBuf> {
        self.backend_config
            .as_ref()
            .context("store backend configuration unavailable")?
            .relational
            .resolve_sqlite_db_path_for_repo(&self.repo_root)
            .context("resolving SQLite path for GraphQL DevQL queries")
    }

    fn current_branch_name(&self) -> String {
        git_default_branch_name(self.repo_root.as_path())
    }
}
