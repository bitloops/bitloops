mod parsing;
mod sql;

use self::parsing::{artefact_from_value, dependency_edge_from_value, file_context_from_value};
use self::sql::{
    DependencyScope, build_artefacts_by_ids_sql, build_child_artefacts_sql,
    build_current_artefacts_count_sql, build_current_artefacts_cursor_exists_sql,
    build_current_artefacts_sql, build_current_artefacts_window_sql,
    build_current_dependency_batch_sql, build_current_dependency_sql, build_file_context_list_sql,
    build_file_context_lookup_sql, normalise_repo_relative_path,
};
use super::DevqlGraphqlContext;
use super::git_history::git_default_branch_name;
use crate::artefact_query_planner::{ArtefactPagination, plan_graphql_artefact_query};
use crate::graphql::ResolverScope;
use crate::graphql::types::{
    Artefact, ArtefactFilterInput, DependencyEdge, DepsDirection, DepsFilterInput, FileContext,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;

impl DevqlGraphqlContext {
    pub(crate) fn validate_project_path(&self, path: &str) -> std::result::Result<String, String> {
        let normalized = normalise_repo_relative_path(path, false)?;
        let candidate = self.repo_root.join(&normalized);
        if !candidate.exists() {
            return Err(format!("unknown project path `{normalized}`"));
        }
        if !candidate.is_dir() {
            return Err(format!("project path `{normalized}` is not a directory"));
        }
        Ok(normalized)
    }

    pub(crate) fn resolve_scope_path(
        &self,
        scope: &ResolverScope,
        path: &str,
        allow_glob: bool,
    ) -> std::result::Result<String, String> {
        let normalized = normalise_repo_relative_path(path, allow_glob)?;
        Ok(match scope.project_path() {
            Some(project_path) => format!("{project_path}/{normalized}"),
            None => normalized,
        })
    }

    pub(crate) async fn resolve_file_context(
        &self,
        path: &str,
        scope: &ResolverScope,
    ) -> Result<Option<FileContext>> {
        if !scope.contains_repo_path(path) {
            return Ok(None);
        }
        let sql = build_file_context_lookup_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            path,
            scope.temporal_scope(),
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .next()
            .map(file_context_from_value)
            .map(|result| result.map(|file| file.with_scope(scope.clone())))
            .transpose()
    }

    pub(crate) async fn list_file_contexts(
        &self,
        glob: &str,
        scope: &ResolverScope,
    ) -> Result<Vec<FileContext>> {
        let sql = build_file_context_list_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            glob,
            scope.temporal_scope(),
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(file_context_from_value)
            .map(|result| result.map(|file| file.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn list_artefacts(
        &self,
        path: Option<&str>,
        filter: Option<&ArtefactFilterInput>,
        scope: &ResolverScope,
    ) -> Result<Vec<Artefact>> {
        if let Some(filter) = filter {
            filter
                .validate()
                .map_err(|err| anyhow::anyhow!(err.message))?;
        }
        let spec = plan_graphql_artefact_query(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            path,
            filter,
            scope,
            None,
        );
        let sql = build_current_artefacts_sql(&spec);
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(artefact_from_value)
            .map(|result| result.map(|artefact| artefact.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn count_artefacts(
        &self,
        path: Option<&str>,
        filter: Option<&ArtefactFilterInput>,
        scope: &ResolverScope,
    ) -> Result<usize> {
        let spec = plan_graphql_artefact_query(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            path,
            filter,
            scope,
            None,
        );
        let sql = build_current_artefacts_count_sql(&spec);
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        let total_count = rows
            .first()
            .and_then(|row| row.get("total_count"))
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_i64().map(|value| value as u64))
            })
            .context("missing total_count for artefact query")?;
        usize::try_from(total_count).context("artefact total_count does not fit in usize")
    }

    pub(crate) async fn artefact_cursor_exists(
        &self,
        path: Option<&str>,
        filter: Option<&ArtefactFilterInput>,
        scope: &ResolverScope,
        cursor: &str,
    ) -> Result<bool> {
        let spec = plan_graphql_artefact_query(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            path,
            filter,
            scope,
            None,
        );
        let sql = build_current_artefacts_cursor_exists_sql(&spec, cursor);
        Ok(!self.query_devql_sqlite_rows(&sql).await?.is_empty())
    }

    pub(crate) async fn list_artefacts_window(
        &self,
        path: Option<&str>,
        filter: Option<&ArtefactFilterInput>,
        scope: &ResolverScope,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Artefact>> {
        let spec = plan_graphql_artefact_query(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            path,
            filter,
            scope,
            Some(ArtefactPagination::new(after, limit)),
        );
        let sql = build_current_artefacts_window_sql(&spec);
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(artefact_from_value)
            .map(|result| result.map(|artefact| artefact.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn load_artefacts_by_ids(
        &self,
        artefact_ids: &[String],
        scope: &ResolverScope,
    ) -> Result<HashMap<String, Artefact>> {
        if artefact_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let sql = build_artefacts_by_ids_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            artefact_ids,
            scope.project_path(),
            scope.temporal_scope(),
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        let mut artefacts = HashMap::new();
        for row in rows {
            let artefact = artefact_from_value(row)?.with_scope(scope.clone());
            artefacts.insert(artefact.id.to_string(), artefact);
        }
        Ok(artefacts)
    }

    pub(crate) async fn list_child_artefacts(
        &self,
        parent_artefact_id: &str,
        scope: &ResolverScope,
    ) -> Result<Vec<Artefact>> {
        let sql = build_child_artefacts_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            parent_artefact_id,
            scope.project_path(),
            scope.temporal_scope(),
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(artefact_from_value)
            .map(|result| result.map(|artefact| artefact.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn list_file_dependency_edges(
        &self,
        path: &str,
        filter: Option<&DepsFilterInput>,
        scope: &ResolverScope,
    ) -> Result<Vec<DependencyEdge>> {
        if !scope.contains_repo_path(path) {
            return Ok(Vec::new());
        }
        let sql = build_current_dependency_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            DependencyScope::File(path),
            scope.project_path(),
            filter.copied().unwrap_or_default(),
            scope.temporal_scope(),
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(dependency_edge_from_value)
            .map(|result| result.map(|edge| edge.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn list_project_dependency_edges(
        &self,
        scope: &ResolverScope,
        filter: Option<&DepsFilterInput>,
    ) -> Result<Vec<DependencyEdge>> {
        let Some(project_path) = scope.project_path() else {
            return Ok(Vec::new());
        };

        let sql = build_current_dependency_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            DependencyScope::Project(project_path),
            None,
            filter.copied().unwrap_or_default(),
            scope.temporal_scope(),
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(dependency_edge_from_value)
            .map(|result| result.map(|edge| edge.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn load_dependency_edges_by_artefact_ids(
        &self,
        artefact_ids: &[String],
        direction: DepsDirection,
        filter: DepsFilterInput,
        scope: &ResolverScope,
    ) -> Result<HashMap<String, Vec<DependencyEdge>>> {
        if artefact_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let sql = build_current_dependency_batch_sql(
            self.repo_identity.repo_id.as_str(),
            &self.current_branch_name(),
            artefact_ids,
            direction,
            filter,
            scope.project_path(),
            scope.temporal_scope(),
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        let mut edges_by_artefact = HashMap::<String, Vec<DependencyEdge>>::new();
        for row in rows {
            let owner_artefact_id = row
                .get("owner_artefact_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .context("missing owner artefact id for batched dependency edge")?;
            let edge = dependency_edge_from_value(row)?.with_scope(scope.clone());
            edges_by_artefact
                .entry(owner_artefact_id)
                .or_default()
                .push(edge);
        }
        Ok(edges_by_artefact)
    }

    pub(crate) fn devql_sqlite_path(&self) -> Result<std::path::PathBuf> {
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
