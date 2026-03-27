#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TemporalAccessMode {
    HistoricalCommit,
    SaveCurrent,
    SaveRevision(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ResolvedTemporalScope {
    resolved_commit: String,
    access_mode: TemporalAccessMode,
}

impl ResolvedTemporalScope {
    pub(crate) fn new(resolved_commit: String, access_mode: TemporalAccessMode) -> Self {
        Self {
            resolved_commit,
            access_mode,
        }
    }

    pub(crate) fn resolved_commit(&self) -> &str {
        self.resolved_commit.as_str()
    }

    pub(crate) fn access_mode(&self) -> &TemporalAccessMode {
        &self.access_mode
    }

    pub(crate) fn use_historical_tables(&self) -> bool {
        matches!(self.access_mode, TemporalAccessMode::HistoricalCommit)
    }

    pub(crate) fn save_revision(&self) -> Option<&str> {
        match &self.access_mode {
            TemporalAccessMode::SaveRevision(revision_id) => Some(revision_id.as_str()),
            TemporalAccessMode::HistoricalCommit | TemporalAccessMode::SaveCurrent => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) struct ResolverScope {
    project_path: Option<String>,
    temporal_scope: Option<ResolvedTemporalScope>,
}

impl ResolverScope {
    pub(crate) fn project_path(&self) -> Option<&str> {
        self.project_path.as_deref()
    }

    pub(crate) fn temporal_scope(&self) -> Option<&ResolvedTemporalScope> {
        self.temporal_scope.as_ref()
    }

    pub(crate) fn with_project_path(&self, project_path: String) -> Self {
        Self {
            project_path: Some(project_path),
            temporal_scope: self.temporal_scope.clone(),
        }
    }

    pub(crate) fn with_temporal_scope(&self, temporal_scope: ResolvedTemporalScope) -> Self {
        Self {
            project_path: self.project_path.clone(),
            temporal_scope: Some(temporal_scope),
        }
    }

    pub(crate) fn without_project_path(&self) -> Self {
        Self {
            project_path: None,
            temporal_scope: self.temporal_scope.clone(),
        }
    }

    pub(crate) fn contains_repo_path(&self, path: &str) -> bool {
        match self.project_path() {
            Some(project_path) => {
                path == project_path
                    || path
                        .strip_prefix(project_path)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            }
            None => true,
        }
    }
}
