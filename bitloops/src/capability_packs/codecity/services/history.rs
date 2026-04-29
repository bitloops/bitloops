use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;

use crate::capability_packs::codecity::types::{CodeCityDiagnostic, MetricSource};
use crate::host::capability_host::gateways::{
    FileHistoryEvent, GitHistoryGateway, GitHistoryRequest,
};

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryMetric {
    pub churn: u64,
    pub bug_count: u64,
    pub distinct_authors: u64,
    pub author_concentration: Option<f64>,
    pub source: MetricSource,
}

impl HistoryMetric {
    pub fn unavailable() -> Self {
        Self {
            churn: 0,
            bug_count: 0,
            distinct_authors: 0,
            author_concentration: None,
            source: MetricSource::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryCollection {
    pub by_path: BTreeMap<String, HistoryMetric>,
    pub git_history_available: bool,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

pub fn collect_file_history(
    gateway: &dyn GitHistoryGateway,
    repo_root: &Path,
    paths: impl IntoIterator<Item = String>,
    since_unix: i64,
    until_commit_sha: Option<&str>,
    bug_patterns: &[String],
) -> Result<HistoryCollection> {
    let paths = paths.into_iter().collect::<BTreeSet<_>>();
    let mut by_path = paths
        .iter()
        .map(|path| (path.clone(), HistoryMetric::unavailable()))
        .collect::<BTreeMap<_, _>>();
    let mut diagnostics = Vec::new();

    if !gateway.available() {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.health.history_gateway_unavailable".to_string(),
            severity: "info".to_string(),
            message: "Git history gateway is not attached; churn, bug-fix history, and author concentration were excluded from CodeCity health scoring.".to_string(),
            path: None,
            boundary_id: None,
        });
        return Ok(HistoryCollection {
            by_path,
            git_history_available: false,
            diagnostics,
        });
    }

    let request_paths = paths.iter().cloned().collect::<Vec<_>>();
    let events = gateway.load_file_history(
        repo_root,
        GitHistoryRequest {
            paths: request_paths.as_slice(),
            since_unix,
            until_commit_sha,
            bug_patterns,
        },
    )?;

    if events.is_empty() {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.health.history_window_empty".to_string(),
            severity: "info".to_string(),
            message: "No file history events were found in the CodeCity health analysis window."
                .to_string(),
            path: None,
            boundary_id: None,
        });
        return Ok(HistoryCollection {
            by_path,
            git_history_available: false,
            diagnostics,
        });
    }

    let grouped = group_events_by_path(events);
    for path in paths {
        let Some(events) = grouped.get(&path) else {
            by_path.insert(
                path,
                HistoryMetric {
                    churn: 0,
                    bug_count: 0,
                    distinct_authors: 0,
                    author_concentration: None,
                    source: MetricSource::FileLevelFallback,
                },
            );
            continue;
        };
        let commit_shas = events
            .iter()
            .map(|event| event.commit_sha.as_str())
            .collect::<BTreeSet<_>>();
        let bug_shas = events
            .iter()
            .filter(|event| event.is_bug_fix)
            .map(|event| event.commit_sha.as_str())
            .collect::<BTreeSet<_>>();
        let authors = events
            .iter()
            .filter_map(author_key)
            .collect::<BTreeSet<_>>();
        let distinct_authors = authors.len() as u64;
        by_path.insert(
            path,
            HistoryMetric {
                churn: commit_shas.len() as u64,
                bug_count: bug_shas.len() as u64,
                distinct_authors,
                author_concentration: (distinct_authors > 0)
                    .then_some(1.0 / distinct_authors as f64),
                source: MetricSource::FileLevelFallback,
            },
        );
    }

    diagnostics.push(CodeCityDiagnostic {
        code: "codecity.health.history_file_level_fallback".to_string(),
        severity: "info".to_string(),
        message: "Git history was attributed at file level for CodeCity health scoring."
            .to_string(),
        path: None,
        boundary_id: None,
    });

    Ok(HistoryCollection {
        by_path,
        git_history_available: true,
        diagnostics,
    })
}

fn group_events_by_path(events: Vec<FileHistoryEvent>) -> BTreeMap<String, Vec<FileHistoryEvent>> {
    let mut grouped = BTreeMap::<String, Vec<FileHistoryEvent>>::new();
    for event in events {
        grouped.entry(event.path.clone()).or_default().push(event);
    }
    grouped
}

fn author_key(event: &FileHistoryEvent) -> Option<String> {
    event
        .author_email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .or_else(|| {
            event
                .author_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("name:{}", value.to_ascii_lowercase()))
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::Result;

    use super::collect_file_history;
    use crate::host::capability_host::gateways::{
        FileHistoryEvent, GitHistoryGateway, GitHistoryRequest,
    };

    struct FakeHistoryGateway {
        events: Vec<FileHistoryEvent>,
    }

    impl GitHistoryGateway for FakeHistoryGateway {
        fn available(&self) -> bool {
            true
        }

        fn load_file_history(
            &self,
            _repo_root: &Path,
            _request: GitHistoryRequest<'_>,
        ) -> Result<Vec<FileHistoryEvent>> {
            Ok(self.events.clone())
        }
    }

    fn event(path: &str, sha: &str, email: &str, is_bug_fix: bool) -> FileHistoryEvent {
        FileHistoryEvent {
            path: path.to_string(),
            commit_sha: sha.to_string(),
            author_name: Some("Author".to_string()),
            author_email: Some(email.to_string()),
            committed_at_unix: 1,
            message: "fix bug".to_string(),
            is_bug_fix,
            changed_ranges: Vec::new(),
        }
    }

    #[test]
    fn file_history_counts_distinct_commits_bug_commits_and_authors() -> Result<()> {
        let gateway = FakeHistoryGateway {
            events: vec![
                event("src/lib.rs", "a", "one@example.com", true),
                event("src/lib.rs", "a", "one@example.com", true),
                event("src/lib.rs", "b", "two@example.com", false),
            ],
        };
        let result = collect_file_history(
            &gateway,
            Path::new("."),
            ["src/lib.rs".to_string()],
            0,
            Some("HEAD"),
            &["fix".to_string()],
        )?;

        let metric = &result.by_path["src/lib.rs"];
        assert_eq!(metric.churn, 2);
        assert_eq!(metric.bug_count, 1);
        assert_eq!(metric.distinct_authors, 2);
        assert_eq!(metric.author_concentration, Some(0.5));
        assert!(result.git_history_available);
        Ok(())
    }
}
