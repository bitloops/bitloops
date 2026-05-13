use crate::daemon::{DevqlTaskSource, DevqlTaskSpec, SyncTaskMode};
use std::collections::HashSet;

use super::ProducerSpoolJobPayload;

pub(super) fn prune_excluded_paths_from_payload(
    payload: ProducerSpoolJobPayload,
    matcher: &crate::host::devql::RepoExclusionMatcher,
) -> Option<ProducerSpoolJobPayload> {
    match payload {
        ProducerSpoolJobPayload::Task {
            source,
            spec:
                DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                    mode: SyncTaskMode::Paths { paths },
                    ..
                }),
        } => {
            let paths = paths
                .into_iter()
                .filter(|path| !matcher.excludes_repo_relative_path(path))
                .collect::<Vec<_>>();
            if paths.is_empty() {
                None
            } else {
                Some(ProducerSpoolJobPayload::Task {
                    source,
                    spec: DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                        mode: SyncTaskMode::Paths { paths },
                        post_commit_snapshot: None,
                    }),
                })
            }
        }
        ProducerSpoolJobPayload::PostCommitRefresh {
            commit_sha,
            changed_files,
        } => {
            let changed_files = changed_files
                .into_iter()
                .filter(|path| !matcher.excludes_repo_relative_path(path))
                .collect::<Vec<_>>();
            if changed_files.is_empty() {
                None
            } else {
                Some(ProducerSpoolJobPayload::PostCommitRefresh {
                    commit_sha,
                    changed_files,
                })
            }
        }
        ProducerSpoolJobPayload::PostMergeRefresh {
            head_sha,
            changed_files,
        } => {
            let changed_files = changed_files
                .into_iter()
                .filter(|path| !matcher.excludes_repo_relative_path(path))
                .collect::<Vec<_>>();
            if changed_files.is_empty() {
                None
            } else {
                Some(ProducerSpoolJobPayload::PostMergeRefresh {
                    head_sha,
                    changed_files,
                })
            }
        }
        ProducerSpoolJobPayload::PostMergeSyncRefresh {
            merge_head_sha,
            changed_files,
            is_squash,
        } => {
            let changed_files = changed_files
                .into_iter()
                .filter(|path| !matcher.excludes_repo_relative_path(path))
                .collect::<Vec<_>>();
            if changed_files.is_empty() {
                None
            } else {
                Some(ProducerSpoolJobPayload::PostMergeSyncRefresh {
                    merge_head_sha,
                    changed_files,
                    is_squash,
                })
            }
        }
        payload => Some(payload),
    }
}

pub(super) fn merge_pending_payload(
    existing: ProducerSpoolJobPayload,
    incoming: ProducerSpoolJobPayload,
) -> ProducerSpoolJobPayload {
    match (existing, incoming) {
        (
            ProducerSpoolJobPayload::Task {
                source: existing_source,
                spec:
                    DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                        mode:
                            SyncTaskMode::Paths {
                                paths: existing_paths,
                            },
                        ..
                    }),
            },
            ProducerSpoolJobPayload::Task {
                source: incoming_source,
                spec:
                    DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                        mode:
                            SyncTaskMode::Paths {
                                paths: incoming_paths,
                            },
                        ..
                    }),
            },
        ) if existing_source == incoming_source => {
            let mut paths = existing_paths;
            paths.extend(incoming_paths);
            paths.sort();
            paths.dedup();
            ProducerSpoolJobPayload::Task {
                source: existing_source,
                spec: DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                    mode: SyncTaskMode::Paths { paths },
                    post_commit_snapshot: None,
                }),
            }
        }
        (
            ProducerSpoolJobPayload::PostMergeSyncRefresh {
                merge_head_sha,
                changed_files: existing_paths,
                is_squash: existing_is_squash,
            },
            ProducerSpoolJobPayload::PostMergeSyncRefresh {
                changed_files: incoming_paths,
                is_squash: incoming_is_squash,
                ..
            },
        ) => {
            let mut changed_files = existing_paths;
            changed_files.extend(incoming_paths);
            changed_files.sort();
            changed_files.dedup();
            ProducerSpoolJobPayload::PostMergeSyncRefresh {
                merge_head_sha,
                changed_files,
                is_squash: existing_is_squash || incoming_is_squash,
            }
        }
        (_, incoming) => incoming,
    }
}

pub(super) fn spool_task_dedupe_key(
    source: DevqlTaskSource,
    spec: &DevqlTaskSpec,
) -> Option<String> {
    match spec {
        DevqlTaskSpec::Sync(sync) => Some(format!(
            "task:{source}:sync:{}",
            spool_sync_mode_key(&sync.mode)
        )),
        DevqlTaskSpec::Ingest(spec) => Some(format!(
            "task:{source}:ingest:{}",
            spool_ingest_spec_key(spec)
        )),
        DevqlTaskSpec::EmbeddingsBootstrap(_) | DevqlTaskSpec::SummaryBootstrap(_) => None,
    }
}

pub(super) fn sync_task_spec_from_mode(
    mode: crate::host::devql::SyncMode,
) -> crate::daemon::SyncTaskSpec {
    crate::daemon::SyncTaskSpec {
        mode: match mode {
            crate::host::devql::SyncMode::Auto => SyncTaskMode::Auto,
            crate::host::devql::SyncMode::Full => SyncTaskMode::Full,
            crate::host::devql::SyncMode::Paths(paths) => SyncTaskMode::Paths {
                paths: normalize_paths(&paths),
            },
            crate::host::devql::SyncMode::Repair => SyncTaskMode::Repair,
            crate::host::devql::SyncMode::Validate => SyncTaskMode::Validate,
        },
        post_commit_snapshot: None,
    }
}

pub(super) fn normalize_paths(paths: &[String]) -> Vec<String> {
    let mut normalized = paths
        .iter()
        .map(|path| normalize_repo_path(path))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn spool_sync_mode_key(mode: &SyncTaskMode) -> String {
    match mode {
        SyncTaskMode::Auto => "auto".to_string(),
        SyncTaskMode::Full => "full".to_string(),
        SyncTaskMode::Repair => "repair".to_string(),
        SyncTaskMode::Validate => "validate".to_string(),
        SyncTaskMode::Paths { .. } => "paths".to_string(),
    }
}

fn spool_ingest_spec_key(spec: &crate::daemon::IngestTaskSpec) -> String {
    if !spec.commits.is_empty() {
        let mut seen = HashSet::new();
        let commits = spec
            .commits
            .iter()
            .filter_map(|commit| {
                let commit = commit.trim();
                if commit.is_empty() || !seen.insert(commit.to_string()) {
                    None
                } else {
                    Some(commit.to_string())
                }
            })
            .collect::<Vec<_>>();
        if !commits.is_empty() {
            return format!("commits:{}", commits.join(","));
        }
    }
    spec.backfill
        .map(|backfill| backfill.to_string())
        .unwrap_or_else(|| "all".to_string())
}

fn normalize_repo_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}
