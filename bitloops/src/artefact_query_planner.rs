use anyhow::Result;
use std::path::Path;

use crate::graphql::{ArtefactFilterInput, CanonicalKind, ResolverScope, TemporalAccessMode};
use crate::host::checkpoints::strategy::manual_commit::{resolve_default_branch_name, run_git};
use crate::host::devql::{DevqlAsOfSelector, DevqlConfig, ParsedDevqlQuery};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtefactQuerySpec {
    pub repo_id: String,
    pub branch: Option<String>,
    pub historical_path_blob_sha: Option<String>,
    pub scope: ArtefactScope,
    pub temporal_scope: ArtefactTemporalScope,
    pub structural_filter: ArtefactStructuralFilter,
    pub activity_filter: Option<ArtefactActivityFilter>,
    pub pagination: Option<ArtefactPagination>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ArtefactScope {
    pub project_path: Option<String>,
    pub path: Option<String>,
    pub files_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ArtefactStructuralFilter {
    pub kind: Option<ArtefactKindFilter>,
    pub symbol_fqn: Option<String>,
    pub lines: Option<ArtefactLineRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArtefactLineRange {
    pub start: i32,
    pub end: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArtefactKindFilter {
    File,
    Namespace,
    Module,
    Import,
    Type,
    Interface,
    Enum,
    Callable,
    Function,
    Method,
    Value,
    Variable,
    Constant,
    Member,
    Parameter,
    TypeParameter,
    Alias,
    Raw(String),
}

impl ArtefactKindFilter {
    fn from_graphql(kind: CanonicalKind) -> Self {
        match kind {
            CanonicalKind::File => Self::File,
            CanonicalKind::Namespace => Self::Namespace,
            CanonicalKind::Module => Self::Module,
            CanonicalKind::Import => Self::Import,
            CanonicalKind::Type => Self::Type,
            CanonicalKind::Interface => Self::Interface,
            CanonicalKind::Enum => Self::Enum,
            CanonicalKind::Callable => Self::Callable,
            CanonicalKind::Function => Self::Function,
            CanonicalKind::Method => Self::Method,
            CanonicalKind::Value => Self::Value,
            CanonicalKind::Variable => Self::Variable,
            CanonicalKind::Member => Self::Member,
            CanonicalKind::Parameter => Self::Parameter,
            CanonicalKind::TypeParameter => Self::TypeParameter,
            CanonicalKind::Alias => Self::Alias,
        }
    }

    fn from_devql(kind: &str) -> Self {
        let normalized = kind.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "file" => Self::File,
            "namespace" => Self::Namespace,
            "module" => Self::Module,
            "import" => Self::Import,
            "type" => Self::Type,
            "interface" => Self::Interface,
            "enum" => Self::Enum,
            "callable" => Self::Callable,
            "function" => Self::Function,
            "method" => Self::Method,
            "value" => Self::Value,
            "variable" => Self::Variable,
            "constant" => Self::Constant,
            "member" => Self::Member,
            "parameter" => Self::Parameter,
            "type_parameter" => Self::TypeParameter,
            "alias" => Self::Alias,
            _ => Self::Raw(normalized),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum ArtefactTemporalScope {
    #[default]
    Current,
    HistoricalCommit {
        commit_sha: String,
    },
    SaveCurrent,
    SaveRevision {
        revision_id: String,
    },
}

impl ArtefactTemporalScope {
    pub(crate) fn resolved_commit(&self) -> Option<&str> {
        match self {
            Self::HistoricalCommit { commit_sha } => Some(commit_sha.as_str()),
            Self::Current | Self::SaveCurrent | Self::SaveRevision { .. } => None,
        }
    }

    pub(crate) fn use_historical_tables(&self) -> bool {
        matches!(self, Self::HistoricalCommit { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtefactActivityFilter {
    pub agent: Option<String>,
    pub since: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArtefactPaginationDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtefactPagination {
    pub direction: ArtefactPaginationDirection,
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: usize,
}

impl ArtefactPagination {
    pub(crate) fn forward(after: Option<&str>, limit: usize) -> Self {
        Self {
            direction: ArtefactPaginationDirection::Forward,
            after: normalise_optional_text(after),
            before: None,
            limit: limit.max(1),
        }
    }

    pub(crate) fn backward(before: Option<&str>, limit: usize) -> Self {
        Self {
            direction: ArtefactPaginationDirection::Backward,
            after: None,
            before: normalise_optional_text(before),
            limit: limit.max(1),
        }
    }
}

pub(crate) fn plan_graphql_artefact_query(
    repo_id: &str,
    branch: &str,
    path: Option<&str>,
    filter: Option<&ArtefactFilterInput>,
    scope: &ResolverScope,
    pagination: Option<ArtefactPagination>,
) -> ArtefactQuerySpec {
    let temporal_scope = scope.temporal_scope().map_or_else(
        || ArtefactTemporalScope::Current,
        |scope| match scope.access_mode() {
            TemporalAccessMode::HistoricalCommit => ArtefactTemporalScope::HistoricalCommit {
                commit_sha: scope.resolved_commit().to_string(),
            },
            TemporalAccessMode::SaveCurrent => ArtefactTemporalScope::SaveCurrent,
            TemporalAccessMode::SaveRevision(revision_id) => ArtefactTemporalScope::SaveRevision {
                revision_id: revision_id.clone(),
            },
        },
    );
    let structural_filter = filter.map_or_else(ArtefactStructuralFilter::default, |filter| {
        ArtefactStructuralFilter {
            kind: filter.kind.map(ArtefactKindFilter::from_graphql),
            symbol_fqn: normalise_optional_text(filter.symbol_fqn.as_deref()),
            lines: filter.lines.as_ref().map(|lines| ArtefactLineRange {
                start: lines.start,
                end: lines.end,
            }),
        }
    });

    ArtefactQuerySpec {
        repo_id: repo_id.to_string(),
        branch: (!temporal_scope.use_historical_tables()).then(|| branch.to_string()),
        historical_path_blob_sha: None,
        scope: ArtefactScope {
            project_path: scope.project_path().map(str::to_string),
            path: normalise_optional_text(path),
            files_path: None,
        },
        temporal_scope,
        structural_filter,
        activity_filter: filter.and_then(|filter| {
            build_activity_filter(
                filter.agent.as_deref(),
                filter.since.as_ref().map(|value| value.as_str()),
            )
        }),
        pagination,
    }
}

pub(crate) fn plan_devql_artefact_query(
    cfg: &DevqlConfig,
    repo_id: &str,
    parsed: &ParsedDevqlQuery,
) -> Result<ArtefactQuerySpec> {
    let temporal_scope = plan_devql_temporal_scope(cfg, parsed.as_of.as_ref())?;
    let historical_path_blob_sha = temporal_scope.resolved_commit().and_then(|commit_sha| {
        resolve_historical_path_blob_sha(
            cfg.repo_root.as_path(),
            commit_sha,
            parsed.file.as_deref(),
        )
    });
    Ok(ArtefactQuerySpec {
        repo_id: repo_id.to_string(),
        branch: (!temporal_scope.use_historical_tables())
            .then(|| resolve_active_branch_name(cfg.repo_root.as_path())),
        historical_path_blob_sha,
        scope: ArtefactScope {
            project_path: normalise_optional_text(parsed.project_path.as_deref()),
            path: normalise_optional_text(parsed.file.as_deref()),
            files_path: normalise_optional_text(parsed.files_path.as_deref()),
        },
        temporal_scope,
        structural_filter: ArtefactStructuralFilter {
            kind: parsed
                .artefacts
                .kind
                .as_deref()
                .map(ArtefactKindFilter::from_devql),
            symbol_fqn: normalise_optional_text(parsed.artefacts.symbol_fqn.as_deref()),
            lines: parsed
                .artefacts
                .lines
                .map(|(start, end)| ArtefactLineRange { start, end }),
        },
        activity_filter: build_activity_filter(
            parsed.artefacts.agent.as_deref(),
            parsed.artefacts.since.as_deref(),
        ),
        pagination: Some(ArtefactPagination::forward(None, parsed.limit)),
    })
}

fn plan_devql_temporal_scope(
    cfg: &DevqlConfig,
    as_of: Option<&DevqlAsOfSelector>,
) -> Result<ArtefactTemporalScope> {
    match as_of {
        None => Ok(ArtefactTemporalScope::Current),
        Some(DevqlAsOfSelector::Commit(commit_sha)) => {
            Ok(ArtefactTemporalScope::HistoricalCommit {
                commit_sha: commit_sha.clone(),
            })
        }
        Some(DevqlAsOfSelector::Ref(reference)) => Ok(ArtefactTemporalScope::HistoricalCommit {
            commit_sha: resolve_git_revision(cfg.repo_root.as_path(), reference)?,
        }),
        Some(DevqlAsOfSelector::SaveCurrent) => Ok(ArtefactTemporalScope::SaveCurrent),
        Some(DevqlAsOfSelector::SaveRevision(revision_id)) => {
            Ok(ArtefactTemporalScope::SaveRevision {
                revision_id: revision_id.clone(),
            })
        }
    }
}

fn resolve_active_branch_name(repo_root: &Path) -> String {
    let branch = run_git(repo_root, &["branch", "--show-current"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    branch.unwrap_or_else(|| resolve_default_branch_name(repo_root))
}

fn resolve_git_revision(repo_root: &Path, revision: &str) -> Result<String> {
    run_git(repo_root, &["rev-parse", revision]).map(|value| value.trim().to_string())
}

fn resolve_historical_path_blob_sha(
    repo_root: &Path,
    commit_sha: &str,
    path: Option<&str>,
) -> Option<String> {
    let path = path?;
    build_historical_path_candidates(path)
        .into_iter()
        .find_map(|candidate| resolve_git_blob_sha(repo_root, commit_sha, &candidate))
}

fn build_historical_path_candidates(path: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let raw = path.trim();
    if !raw.is_empty() {
        candidates.push(raw.to_string());
    }

    let normalized = raw
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim()
        .to_string();
    if !normalized.is_empty() {
        candidates.push(normalized.clone());
        candidates.push(format!("./{normalized}"));
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

fn resolve_git_blob_sha(repo_root: &Path, commit_sha: &str, path: &str) -> Option<String> {
    let spec = format!("{commit_sha}:{path}");
    run_git(repo_root, &["rev-parse", &spec])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn build_activity_filter(
    agent: Option<&str>,
    since: Option<&str>,
) -> Option<ArtefactActivityFilter> {
    let agent = normalise_optional_text(agent);
    let since = normalise_optional_timestamp(since);
    if agent.is_none() && since.is_none() {
        return None;
    }

    Some(ArtefactActivityFilter { agent, since })
}

fn normalise_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalise_optional_timestamp(value: Option<&str>) -> Option<String> {
    normalise_optional_text(value).map(|value| {
        chrono::DateTime::parse_from_rfc3339(&value)
            .map(|parsed| parsed.to_rfc3339())
            .unwrap_or(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql::{LineRangeInput, ResolvedTemporalScope};
    use crate::host::devql::{RepoIdentity, deterministic_uuid, parse_devql_query};
    use tempfile::tempdir;

    fn test_cfg(repo_root: &Path) -> DevqlConfig {
        DevqlConfig {
            daemon_config_root: repo_root.to_path_buf(),
            repo_root: repo_root.to_path_buf(),
            repo: RepoIdentity {
                provider: "github".to_string(),
                organization: "bitloops".to_string(),
                name: "temp2".to_string(),
                identity: "github/bitloops/temp2".to_string(),
                repo_id: deterministic_uuid("repo://github/bitloops/temp2"),
            },
            pg_dsn: None,
            clickhouse_url: "http://localhost:8123".to_string(),
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: "default".to_string(),
            semantic_provider: None,
            semantic_model: None,
            semantic_api_key: None,
            semantic_base_url: None,
        }
    }

    #[test]
    fn graphql_and_devql_planners_match_for_current_repository_queries() {
        let temp = tempdir().expect("tempdir");
        let cfg = test_cfg(temp.path());
        let parsed =
            parse_devql_query(r#"repo("temp2")->artefacts(kind:"function")->limit(25)"#).unwrap();

        let devql_spec = plan_devql_artefact_query(&cfg, "repo-1", &parsed).unwrap();
        let graphql_spec = plan_graphql_artefact_query(
            "repo-1",
            "main",
            None,
            Some(&ArtefactFilterInput {
                kind: Some(CanonicalKind::Function),
                ..Default::default()
            }),
            &ResolverScope::default(),
            Some(ArtefactPagination::forward(None, 25)),
        );

        assert_eq!(graphql_spec, devql_spec);
    }

    #[test]
    fn graphql_and_devql_planners_match_for_historical_file_activity_filters() {
        let temp = tempdir().expect("tempdir");
        let cfg = test_cfg(temp.path());
        let parsed = parse_devql_query(
            r#"repo("temp2")->asOf(commit:"commit-123")->file("src/main.rs")->artefacts(kind:"function",symbol_fqn:"src/main.rs::main",lines:10..25,agent:"codex",since:"2026-03-20T00:00:00Z")->limit(10)"#,
        )
        .unwrap();

        let devql_spec = plan_devql_artefact_query(&cfg, "repo-1", &parsed).unwrap();
        let graphql_spec = plan_graphql_artefact_query(
            "repo-1",
            "main",
            Some("src/main.rs"),
            Some(&ArtefactFilterInput {
                kind: Some(CanonicalKind::Function),
                symbol_fqn: Some("src/main.rs::main".to_string()),
                lines: Some(LineRangeInput { start: 10, end: 25 }),
                agent: Some("codex".to_string()),
                since: Some(
                    crate::graphql::DateTimeScalar::from_rfc3339("2026-03-20T00:00:00Z")
                        .expect("valid datetime"),
                ),
            }),
            &ResolverScope::default().with_temporal_scope(ResolvedTemporalScope::new(
                "commit-123".to_string(),
                TemporalAccessMode::HistoricalCommit,
            )),
            Some(ArtefactPagination::forward(None, 10)),
        );

        assert_eq!(graphql_spec, devql_spec);
    }

    #[test]
    fn graphql_and_devql_planners_match_for_project_save_revision_queries() {
        let temp = tempdir().expect("tempdir");
        let cfg = test_cfg(temp.path());
        let parsed = parse_devql_query(
            r#"repo("temp2")->project("packages/api")->asOf(saveRevision:"temp:42")->artefacts(kind:"method")->limit(5)"#,
        )
        .unwrap();

        let devql_spec = plan_devql_artefact_query(&cfg, "repo-1", &parsed).unwrap();
        let graphql_spec = plan_graphql_artefact_query(
            "repo-1",
            "main",
            None,
            Some(&ArtefactFilterInput {
                kind: Some(CanonicalKind::Method),
                ..Default::default()
            }),
            &ResolverScope::default()
                .with_project_path("packages/api".to_string())
                .with_temporal_scope(ResolvedTemporalScope::new(
                    "ignored".to_string(),
                    TemporalAccessMode::SaveRevision("temp:42".to_string()),
                )),
            Some(ArtefactPagination::forward(None, 5)),
        );

        assert_eq!(graphql_spec, devql_spec);
    }
}
