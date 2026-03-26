use super::commit_checkpoints::{
    is_missing_sqlite_store_error, read_commit_checkpoint_mappings_all,
};
use super::{
    DevqlGraphqlContext, GIT_FIELD_SEPARATOR, GIT_RECORD_SEPARATOR, GRAPHQL_GIT_SCAN_LIMIT,
};
use crate::adapters::agents::canonical_agent_key;
use crate::graphql::types::{Branch, Commit, DateTimeScalar};
use crate::host::checkpoints::strategy::manual_commit::{list_committed, run_git};
use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use tokio::task;

impl DevqlGraphqlContext {
    pub(crate) async fn default_branch_name(&self) -> String {
        let repo_root = self.repo_root.clone();
        task::spawn_blocking(move || git_default_branch_name(repo_root.as_path()))
            .await
            .unwrap_or_else(|_| "main".to_string())
    }

    pub(crate) async fn list_commits(
        &self,
        branch: Option<&str>,
        author: Option<&str>,
        since: Option<&DateTimeScalar>,
        until: Option<&DateTimeScalar>,
    ) -> Result<Vec<Commit>> {
        let repo_root = self.repo_root.clone();
        let branch = branch
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let author = author
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let since = since.map(|value| value.as_str().to_string());
        let until = until.map(|value| value.as_str().to_string());

        task::spawn_blocking(move || -> Result<Vec<Commit>> {
            let branch = branch.unwrap_or_else(|| git_default_branch_name(repo_root.as_path()));
            let args = build_git_log_args(
                &branch,
                author.as_deref(),
                since.as_deref(),
                until.as_deref(),
            );
            let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
            let raw = run_git(repo_root.as_path(), &arg_refs)
                .with_context(|| format!("reading git history for branch `{branch}`"))?;
            parse_git_log(&raw, &branch)
        })
        .await
        .context("joining commit history task")?
    }

    pub(crate) async fn list_branches(
        &self,
        since: Option<&DateTimeScalar>,
        until: Option<&DateTimeScalar>,
    ) -> Result<Vec<Branch>> {
        let repo_root = self.repo_root.clone();
        let since = since.map(|value| value.as_str().to_string());
        let until = until.map(|value| value.as_str().to_string());

        task::spawn_blocking(move || -> Result<Vec<Branch>> {
            let committed = match list_committed(repo_root.as_path()) {
                Ok(committed) => committed,
                Err(err) if is_missing_sqlite_store_error(&err) => return Ok(Vec::new()),
                Err(err) => return Err(err).context("reading committed checkpoints for branches"),
            };
            let since_dt = parse_optional_datetime(since.as_deref())?;
            let until_dt = parse_optional_datetime(until.as_deref())?;
            let mut grouped = BTreeMap::<
                String,
                (usize, Option<DateTime<FixedOffset>>, Option<DateTimeScalar>),
            >::new();

            for checkpoint in committed {
                let branch = checkpoint.branch.trim();
                if branch.is_empty() {
                    continue;
                }
                let created_at = parse_datetime_scalar(checkpoint.created_at.as_str());
                if !matches_window(created_at.as_ref(), since_dt.as_ref(), until_dt.as_ref()) {
                    continue;
                }

                let entry = grouped.entry(branch.to_string()).or_insert((0, None, None));
                entry.0 += 1;

                if let Some((created_at_dt, created_at_scalar)) = created_at {
                    let should_replace = entry
                        .1
                        .as_ref()
                        .map(|existing| created_at_dt > *existing)
                        .unwrap_or(true);
                    if should_replace {
                        entry.1 = Some(created_at_dt);
                        entry.2 = Some(created_at_scalar);
                    }
                }
            }

            let mut branches = grouped
                .into_iter()
                .map(
                    |(name, (checkpoint_count, _, latest_checkpoint_at))| Branch {
                        name,
                        checkpoint_count: checkpoint_count.try_into().unwrap_or(i32::MAX),
                        latest_checkpoint_at,
                    },
                )
                .collect::<Vec<_>>();
            branches.sort_by(|left, right| {
                right
                    .latest_checkpoint_at
                    .cmp(&left.latest_checkpoint_at)
                    .then_with(|| left.name.cmp(&right.name))
            });
            Ok(branches)
        })
        .await
        .context("joining branch query task")?
    }

    pub(crate) async fn list_users(&self) -> Result<Vec<String>> {
        let repo_root = self.repo_root.clone();

        task::spawn_blocking(move || -> Result<Vec<String>> {
            let mappings = match read_commit_checkpoint_mappings_all(repo_root.as_path()) {
                Ok(mappings) => mappings,
                Err(err) if is_missing_sqlite_store_error(&err) => return Ok(Vec::new()),
                Err(err) => {
                    return Err(err).context("reading commit-checkpoint mappings for users");
                }
            };
            let mut users = BTreeMap::<String, String>::new();

            for commit_sha in mappings.keys() {
                let raw = match run_git(
                    repo_root.as_path(),
                    &[
                        "show",
                        "--quiet",
                        "--format=%an%x1f%ae",
                        commit_sha.as_str(),
                    ],
                ) {
                    Ok(raw) => raw,
                    Err(err) if is_unknown_revision_error(&err) => continue,
                    Err(err) => {
                        return Err(err)
                            .with_context(|| format!("reading author for {commit_sha}"));
                    }
                };

                let mut parts = raw.trim().split(GIT_FIELD_SEPARATOR);
                let name = parts.next().unwrap_or_default();
                let email = parts.next().unwrap_or_default();
                if let Some((key, display)) = canonical_user_display(name, email) {
                    users.entry(key).or_insert(display);
                }
            }

            Ok(users.into_values().collect())
        })
        .await
        .context("joining users query task")?
    }

    pub(crate) async fn list_agents(&self) -> Result<Vec<String>> {
        let repo_root = self.repo_root.clone();

        task::spawn_blocking(move || -> Result<Vec<String>> {
            let committed = match list_committed(repo_root.as_path()) {
                Ok(committed) => committed,
                Err(err) if is_missing_sqlite_store_error(&err) => return Ok(Vec::new()),
                Err(err) => return Err(err).context("reading committed checkpoints for agents"),
            };
            let mut agents = BTreeSet::new();
            for checkpoint in committed {
                if checkpoint.agents.is_empty() {
                    let key = canonical_agent_key(&checkpoint.agent);
                    if !key.is_empty() {
                        agents.insert(key);
                    }
                    continue;
                }

                for agent in checkpoint.agents {
                    let key = canonical_agent_key(&agent);
                    if !key.is_empty() {
                        agents.insert(key);
                    }
                }
            }
            Ok(agents.into_iter().collect())
        })
        .await
        .context("joining agents query task")?
    }

    pub(crate) async fn list_commit_files_changed(&self, commit_sha: &str) -> Result<Vec<String>> {
        let repo_root = self.repo_root.clone();
        let commit_sha = commit_sha.to_string();

        task::spawn_blocking(move || -> Result<Vec<String>> {
            let raw = run_git(
                repo_root.as_path(),
                &[
                    "show",
                    "--name-only",
                    "--format=",
                    "--find-renames",
                    "--find-copies",
                    commit_sha.as_str(),
                ],
            )
            .with_context(|| format!("reading changed files for commit {commit_sha}"))?;
            let mut files = raw
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            files.sort();
            files.dedup();
            Ok(files)
        })
        .await
        .context("joining commit files query task")?
    }

    pub(crate) async fn load_commits_by_shas(
        &self,
        commit_shas: &[String],
    ) -> Result<HashMap<String, Commit>> {
        if commit_shas.is_empty() {
            return Ok(HashMap::new());
        }

        let repo_root = self.repo_root.clone();
        let commit_shas = commit_shas.to_vec();

        task::spawn_blocking(move || -> Result<HashMap<String, Commit>> {
            let mut args = vec![
                "show".to_string(),
                "--quiet".to_string(),
                "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%cI%x1f%s%x1e".to_string(),
                "--ignore-missing".to_string(),
            ];
            args.extend(commit_shas.iter().cloned());
            let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
            let raw = run_git(repo_root.as_path(), &arg_refs)
                .context("reading git metadata for batched commit lookup")?;
            let mut commits_by_sha = HashMap::new();
            for commit in parse_git_log(&raw, "")? {
                commits_by_sha.insert(commit.sha.clone(), commit);
            }
            Ok(commits_by_sha)
        })
        .await
        .context("joining batched commit lookup task")?
    }

    pub(crate) fn is_unknown_revision_error(&self, err: &anyhow::Error) -> bool {
        is_unknown_revision_error(err)
    }
}

pub(super) fn git_default_branch_name(repo_root: &Path) -> String {
    run_git(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "main".to_string())
}

fn build_git_log_args(
    branch: &str,
    author: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "log".to_string(),
        branch.to_string(),
        "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%cI%x1f%s%x1e".to_string(),
        "--max-count".to_string(),
        GRAPHQL_GIT_SCAN_LIMIT.to_string(),
        "--no-color".to_string(),
    ];
    if let Some(author) = author {
        args.push(format!("--author={author}"));
    }
    if let Some(since) = since {
        args.push(format!("--since={since}"));
    }
    if let Some(until) = until {
        args.push(format!("--until={until}"));
    }
    args
}

fn parse_git_log(raw: &str, branch: &str) -> Result<Vec<Commit>> {
    let branch = branch.trim();
    let branch = (!branch.is_empty()).then(|| branch.to_string());
    let mut commits = Vec::new();

    for record in raw.split(GIT_RECORD_SEPARATOR) {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }

        let mut parts = record.split(GIT_FIELD_SEPARATOR);
        let Some(sha) = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let parents = parts
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let author_name = parts.next().unwrap_or_default().trim().to_string();
        let author_email = parts.next().unwrap_or_default().trim().to_string();
        let committed_at_raw = parts.next().unwrap_or_default().trim();
        let commit_message = parts.next().unwrap_or_default().trim().to_string();
        let committed_at = DateTimeScalar::from_rfc3339(committed_at_raw.to_string())
            .with_context(|| format!("parsing commit timestamp for {sha}"))?;

        commits.push(Commit {
            sha: sha.to_string(),
            parents,
            author_name,
            author_email,
            commit_message,
            committed_at,
            branch: branch.clone(),
        });
    }

    Ok(commits)
}

fn canonical_user_display(name: &str, email: &str) -> Option<(String, String)> {
    let email = email.trim().to_ascii_lowercase();
    if !email.is_empty() {
        return Some((email.clone(), email));
    }

    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    Some((
        format!("name:{}", name.to_ascii_lowercase()),
        name.to_string(),
    ))
}

fn parse_datetime_scalar(value: &str) -> Option<(DateTime<FixedOffset>, DateTimeScalar)> {
    let scalar = DateTimeScalar::from_rfc3339(value.to_string()).ok()?;
    let parsed = DateTimeScalar::parse_rfc3339(scalar.as_str()).ok()?;
    Some((parsed, scalar))
}

fn parse_optional_datetime(value: Option<&str>) -> Result<Option<DateTime<FixedOffset>>> {
    value
        .map(DateTimeScalar::parse_rfc3339)
        .transpose()
        .context("parsing GraphQL datetime filter")
}

fn matches_window(
    value: Option<&(DateTime<FixedOffset>, DateTimeScalar)>,
    since: Option<&DateTime<FixedOffset>>,
    until: Option<&DateTime<FixedOffset>>,
) -> bool {
    if since.is_none() && until.is_none() {
        return true;
    }

    let Some((value, _)) = value else {
        return false;
    };

    if let Some(since) = since
        && value < since
    {
        return false;
    }
    if let Some(until) = until
        && value > until
    {
        return false;
    }
    true
}

fn is_unknown_revision_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    message.contains("unknown revision")
        || message.contains("bad object")
        || message.contains("ambiguous argument")
}
