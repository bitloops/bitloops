// Test-only helper for discovering production source artefacts and seeding them
// into the compatibility schema for synthetic acceptance fixtures.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use bitloops::host::capability_host::gateways::{
    DefaultHostServicesGateway, HostServicesGateway, SymbolIdentityInput,
};
use bitloops::host::devql::extract_production_file_artefacts;
use bitloops::models::{
    CommitRecord, CurrentFileStateRecord, CurrentProductionArtefactRecord, FileStateRecord,
    ProductionArtefactRecord, RepositoryRecord,
};

use super::Workspace;

#[derive(Debug, Default, Clone, Copy)]
struct IngestProductionStats {
    files: usize,
    artefacts: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct IngestProductionSummary {
    pub files: usize,
    pub artefacts: usize,
    pub normalized_duplicates: usize,
}

#[derive(Debug, Clone)]
struct ParsedArtefactIds {
    symbol_id: String,
    artefact_id: String,
}

#[derive(Debug, Default)]
struct ProductionBatchBuilder {
    file_states: Vec<FileStateRecord>,
    current_file_states: Vec<CurrentFileStateRecord>,
    artefacts: Vec<ProductionArtefactRecord>,
    current_artefacts: Vec<CurrentProductionArtefactRecord>,
}

#[derive(Debug, Clone)]
struct DuplicateCurrentArtefactSummary {
    artefact_id: String,
    path: String,
    canonical_kind: String,
    symbol_fqn: Option<String>,
    language_kind: Option<String>,
    start_line: i64,
    end_line: i64,
    signature: Option<String>,
}

impl ProductionBatchBuilder {
    fn push_file_state(
        &mut self,
        repo_id: &str,
        commit_sha: &str,
        path: &str,
        blob_sha: &str,
        committed_at: &str,
    ) {
        self.file_states.push(FileStateRecord {
            repo_id: repo_id.to_string(),
            commit_sha: commit_sha.to_string(),
            path: path.to_string(),
            blob_sha: blob_sha.to_string(),
        });
        self.current_file_states.push(CurrentFileStateRecord {
            repo_id: repo_id.to_string(),
            path: path.to_string(),
            commit_sha: commit_sha.to_string(),
            blob_sha: blob_sha.to_string(),
            committed_at: committed_at.to_string(),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn push_artefact(
        &mut self,
        host_services: &dyn HostServicesGateway,
        repo_id: &str,
        commit_sha: &str,
        blob_sha: &str,
        path: &str,
        language: &str,
        canonical_kind: &str,
        language_kind: Option<&str>,
        symbol_fqn: Option<&str>,
        identity_name: &str,
        parent_symbol_id: Option<&str>,
        parent_artefact_id: Option<&str>,
        start_line: i64,
        end_line: i64,
        start_byte: i64,
        end_byte: i64,
        signature: Option<&str>,
        modifiers: &[String],
        docstring: Option<&str>,
        content_bytes: &[u8],
    ) -> ParsedArtefactIds {
        let symbol_id = if canonical_kind == "file" {
            file_symbol_id(path)
        } else {
            host_services.derive_symbol_id(&SymbolIdentityInput {
                path,
                canonical_kind,
                language_kind: language_kind.unwrap_or("<null>"),
                name: identity_name,
                parent_symbol_id,
                signature: signature.unwrap_or_default(),
                modifiers,
            })
        };
        let artefact_id = host_services.derive_artefact_id(blob_sha, &symbol_id);
        let content_hash = Some(sha256_hex(content_bytes));
        let modifiers = serde_json::to_string(modifiers).unwrap_or_else(|_| "[]".to_string());

        self.artefacts.push(ProductionArtefactRecord {
            artefact_id: artefact_id.clone(),
            symbol_id: symbol_id.clone(),
            repo_id: repo_id.to_string(),
            blob_sha: blob_sha.to_string(),
            path: path.to_string(),
            language: language.to_string(),
            canonical_kind: canonical_kind.to_string(),
            language_kind: language_kind.map(str::to_string),
            symbol_fqn: symbol_fqn.map(str::to_string),
            parent_artefact_id: parent_artefact_id.map(str::to_string),
            start_line,
            end_line,
            start_byte,
            end_byte,
            signature: signature.map(str::to_string),
            modifiers: modifiers.clone(),
            docstring: docstring.map(str::to_string),
            content_hash: content_hash.clone(),
        });
        self.current_artefacts
            .push(CurrentProductionArtefactRecord {
                repo_id: repo_id.to_string(),
                symbol_id: symbol_id.clone(),
                artefact_id: artefact_id.clone(),
                commit_sha: commit_sha.to_string(),
                blob_sha: blob_sha.to_string(),
                path: path.to_string(),
                language: language.to_string(),
                canonical_kind: canonical_kind.to_string(),
                language_kind: language_kind.map(str::to_string),
                symbol_fqn: symbol_fqn.map(str::to_string),
                parent_symbol_id: parent_symbol_id.map(str::to_string),
                parent_artefact_id: parent_artefact_id.map(str::to_string),
                start_line,
                end_line,
                start_byte,
                end_byte,
                signature: signature.map(str::to_string),
                modifiers,
                docstring: docstring.map(str::to_string),
                content_hash,
            });

        ParsedArtefactIds {
            symbol_id,
            artefact_id,
        }
    }
}

pub fn seed_production_artefacts_for_repo(
    db_path: &Path,
    repo_dir: &Path,
    commit_sha: &str,
) -> Result<()> {
    execute(db_path, repo_dir, commit_sha)?;
    Ok(())
}

pub fn seed_production_artefacts(workspace: &Workspace, commit_sha: &str) {
    seed_production_artefacts_for_repo(workspace.db_path(), workspace.repo_dir(), commit_sha)
        .expect("seed production artefacts");
}

pub fn execute(
    db_path: &Path,
    repo_dir: &Path,
    commit_sha: &str,
) -> Result<IngestProductionSummary> {
    let repo = resolve_repository_record(repo_dir)?;
    let host_services = DefaultHostServicesGateway::new(repo.repo_id.clone());
    let branch_name = default_branch_name(repo_dir);
    let production_files = find_production_files(repo_dir)?;
    let committed_at = chrono::Utc::now().to_rfc3339();

    let mut stats = IngestProductionStats::default();
    let mut builder = ProductionBatchBuilder::default();

    for relative_path in production_files {
        stats.files += 1;

        let abs_path = repo_dir.join(&relative_path);
        let source = fs::read_to_string(&abs_path)
            .with_context(|| format!("failed reading source file {}", abs_path.display()))?;
        let source_bytes = source.as_bytes();
        let blob_sha = sha256_hex(source_bytes);
        let end_line = std::cmp::max(source.lines().count() as i64, 1);
        let extraction = extract_production_file_artefacts(&relative_path, &source)?;
        let language = extraction
            .as_ref()
            .map(|file| file.language.clone())
            .unwrap_or_else(|| production_source_language(&relative_path).to_string());

        builder.push_file_state(
            &repo.repo_id,
            commit_sha,
            &relative_path,
            &blob_sha,
            &committed_at,
        );

        let no_modifiers: Vec<String> = Vec::new();
        let file_ids = builder.push_artefact(
            &host_services,
            &repo.repo_id,
            commit_sha,
            &blob_sha,
            &relative_path,
            &language,
            "file",
            Some("file"),
            Some(&relative_path),
            &relative_path,
            None,
            None,
            1,
            end_line,
            0,
            source_bytes.len() as i64,
            None,
            &no_modifiers,
            extraction
                .as_ref()
                .and_then(|file| file.file_docstring.as_deref()),
            source_bytes,
        );
        stats.artefacts += 1;
        stats.artefacts += ingest_adapter_artefacts(
            &host_services,
            &mut builder,
            &repo.repo_id,
            commit_sha,
            &relative_path,
            &blob_sha,
            &language,
            &file_ids,
            extraction,
            source_bytes,
        );
    }

    let persisted_artefact_count = builder
        .current_artefacts
        .iter()
        .map(|artefact| artefact.artefact_id.as_str())
        .collect::<HashSet<_>>()
        .len();
    let duplicate_current_artefacts =
        collect_duplicate_current_artefacts(&builder.current_artefacts);
    let normalized_duplicate_count = duplicate_current_artefacts
        .iter()
        .map(|group| group.len().saturating_sub(1))
        .sum::<usize>();

    persist_production_rows(
        db_path,
        &repo,
        branch_name.as_str(),
        &CommitRecord {
            commit_sha: commit_sha.to_string(),
            repo_id: repo.repo_id.clone(),
            author_name: None,
            author_email: None,
            commit_message: None,
            committed_at: Some(committed_at),
        },
        &builder,
    )?;

    if normalized_duplicate_count > 0 && env::var_os("TESTLENS_DEBUG_DUPLICATE_ARTEFACTS").is_some()
    {
        for duplicate_group in &duplicate_current_artefacts {
            if let Some(first) = duplicate_group.first() {
                eprintln!(
                    "ingest-production-artefacts duplicate artefact_id={} occurrences={}",
                    first.artefact_id,
                    duplicate_group.len()
                );
            }
            for artefact in duplicate_group {
                eprintln!(
                    "  path={} kind={} symbol_fqn={} language_kind={} lines={}-{} signature={}",
                    artefact.path,
                    artefact.canonical_kind,
                    artefact.symbol_fqn.as_deref().unwrap_or(""),
                    artefact.language_kind.as_deref().unwrap_or(""),
                    artefact.start_line,
                    artefact.end_line,
                    artefact.signature.as_deref().unwrap_or("")
                );
            }
        }
    }

    Ok(IngestProductionSummary {
        files: stats.files,
        artefacts: persisted_artefact_count,
        normalized_duplicates: normalized_duplicate_count,
    })
}

fn table_exists(conn: &Connection, table_name: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table_name],
            |row| row.get(0),
        )
        .with_context(|| format!("failed checking for sqlite table `{table_name}`"))?;
    Ok(count > 0)
}

fn persist_production_rows(
    db_path: &Path,
    repository: &RepositoryRecord,
    _branch_name: &str,
    commit: &CommitRecord,
    batch: &ProductionBatchBuilder,
) -> Result<()> {
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed creating parent directory for sqlite database at {}",
                db_path.display()
            )
        })?;
    }

    bitloops::storage::SqliteConnectionPool::connect(db_path.to_path_buf())
        .context("creating sqlite pool for production seed")?
        .initialise_devql_schema()
        .context("initialising SQLite DevQL schema for production seed")?;
    bitloops::capability_packs::test_harness::storage::init_test_domain_database(db_path)
        .context("initialising test-harness schema for production seed")?;

    let mut conn = Connection::open(db_path)
        .with_context(|| format!("failed opening sqlite database at {}", db_path.display()))?;
    let tx = conn
        .transaction()
        .context("failed to open sqlite transaction")?;

    tx.execute(
        "DELETE FROM test_artefact_edges_current WHERE repo_id = ?1",
        params![repository.repo_id],
    )
    .context("failed clearing test_artefact_edges_current for repo")?;
    tx.execute(
        r#"DELETE FROM coverage_hits WHERE capture_id IN (
            SELECT capture_id FROM coverage_captures WHERE commit_sha = ?1
        )"#,
        params![commit.commit_sha],
    )
    .context("failed clearing coverage_hits for commit")?;
    tx.execute(
        "DELETE FROM coverage_captures WHERE commit_sha = ?1",
        params![commit.commit_sha],
    )
    .context("failed clearing coverage_captures for commit")?;
    tx.execute(
        "DELETE FROM test_classifications WHERE commit_sha = ?1",
        params![commit.commit_sha],
    )
    .context("failed clearing test_classifications for commit")?;
    tx.execute(
        "DELETE FROM test_runs WHERE commit_sha = ?1",
        params![commit.commit_sha],
    )
    .context("failed clearing test_runs for commit")?;
    tx.execute(
        "DELETE FROM artefact_edges_current WHERE repo_id = ?1",
        params![repository.repo_id],
    )
    .context("failed clearing artefact_edges_current for repo")?;
    tx.execute(
        "DELETE FROM artefacts_current WHERE repo_id = ?1",
        params![repository.repo_id],
    )
    .context("failed clearing artefacts_current for repo")?;
    if table_exists(&tx, "current_file_state")? {
        match tx.execute(
            "DELETE FROM current_file_state WHERE commit_sha = ?1",
            params![commit.commit_sha],
        ) {
            Ok(_) => {}
            Err(err)
                if {
                    let msg = err.to_string();
                    msg.contains("no such column: commit_sha")
                        || msg.contains("no column named commit_sha")
                } =>
            {
                tx.execute(
                    "DELETE FROM current_file_state WHERE repo_id = ?1",
                    params![repository.repo_id],
                )
                .context("failed clearing sync-shaped current_file_state for repo")?;
            }
            Err(err) => {
                return Err(err).context("failed clearing current_file_state for commit");
            }
        }
    }
    tx.execute(
        "DELETE FROM file_state WHERE commit_sha = ?1",
        params![commit.commit_sha],
    )
    .context("failed clearing file_state for commit")?;
    tx.execute(
        "DELETE FROM commits WHERE commit_sha = ?1",
        params![commit.commit_sha],
    )
    .context("failed clearing commits for commit")?;

    tx.execute(
        r#"
INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(repo_id) DO UPDATE SET
  provider = excluded.provider,
  organization = excluded.organization,
  name = excluded.name,
  default_branch = excluded.default_branch
"#,
        params![
            repository.repo_id,
            repository.provider,
            repository.organization,
            repository.name,
            repository.default_branch
        ],
    )
    .with_context(|| format!("failed upserting repository {}", repository.repo_id))?;

    tx.execute(
        r#"
INSERT INTO commits (
  commit_sha, repo_id, author_name, author_email, commit_message, committed_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(commit_sha) DO UPDATE SET
  repo_id = excluded.repo_id,
  author_name = excluded.author_name,
  author_email = excluded.author_email,
  commit_message = excluded.commit_message,
  committed_at = excluded.committed_at
"#,
        params![
            commit.commit_sha,
            commit.repo_id,
            commit.author_name,
            commit.author_email,
            commit.commit_message,
            commit.committed_at
        ],
    )
    .with_context(|| format!("failed upserting commit {}", commit.commit_sha))?;

    for row in &batch.file_states {
        tx.execute(
            r#"
INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(repo_id, commit_sha, path) DO UPDATE SET
  blob_sha = excluded.blob_sha
"#,
            params![row.repo_id, row.commit_sha, row.path, row.blob_sha],
        )
        .with_context(|| {
            format!(
                "failed upserting file_state {} {}",
                row.commit_sha, row.path
            )
        })?;
    }

    for row in &batch.current_file_states {
        match tx.execute(
            r#"
INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(repo_id, path) DO UPDATE SET
  commit_sha = excluded.commit_sha,
  blob_sha = excluded.blob_sha,
  committed_at = excluded.committed_at,
  updated_at = datetime('now')
"#,
            params![
                row.repo_id,
                row.path,
                row.commit_sha,
                row.blob_sha,
                row.committed_at
            ],
        ) {
            Ok(_) => {}
            Err(err)
                if {
                    let msg = err.to_string();
                    msg.contains("no such column: commit_sha")
                        || msg.contains("no column named commit_sha")
                } =>
            {
                let language = production_source_language(&row.path);
                tx.execute(
                    r#"
INSERT INTO current_file_state (
  repo_id, path, language,
  head_content_id, index_content_id, worktree_content_id,
  effective_content_id, effective_source,
  parser_version, extractor_version,
  exists_in_head, exists_in_index, exists_in_worktree,
  last_synced_at
) VALUES (
  ?1, ?2, ?3, ?4, ?4, ?4, ?4, 'head',
  'test-harness', 'test-harness',
  1, 1, 1, ?5
)
ON CONFLICT(repo_id, path) DO UPDATE SET
  head_content_id = excluded.head_content_id,
  index_content_id = excluded.index_content_id,
  worktree_content_id = excluded.worktree_content_id,
  effective_content_id = excluded.effective_content_id,
  effective_source = excluded.effective_source,
  parser_version = excluded.parser_version,
  extractor_version = excluded.extractor_version,
  exists_in_head = excluded.exists_in_head,
  exists_in_index = excluded.exists_in_index,
  exists_in_worktree = excluded.exists_in_worktree,
  last_synced_at = excluded.last_synced_at
"#,
                    params![
                        row.repo_id,
                        row.path,
                        language,
                        row.blob_sha,
                        row.committed_at
                    ],
                )
                .with_context(|| {
                    format!(
                        "failed upserting sync-shaped current_file_state {}",
                        row.path
                    )
                })?;
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed upserting current_file_state {}", row.path));
            }
        }
    }

    for artefact in &batch.artefacts {
        tx.execute(
            r#"
INSERT INTO artefacts (
  artefact_id, symbol_id, repo_id, language, canonical_kind,
  language_kind, symbol_fqn, signature, modifiers, docstring, content_hash
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
)
ON CONFLICT(artefact_id) DO UPDATE SET
  symbol_id = excluded.symbol_id,
  repo_id = excluded.repo_id,
  language = excluded.language,
  canonical_kind = excluded.canonical_kind,
  language_kind = excluded.language_kind,
  symbol_fqn = excluded.symbol_fqn,
  signature = excluded.signature,
  modifiers = excluded.modifiers,
  docstring = excluded.docstring,
  content_hash = excluded.content_hash
"#,
            params![
                artefact.artefact_id,
                artefact.symbol_id,
                artefact.repo_id,
                artefact.language,
                artefact.canonical_kind,
                artefact.language_kind,
                artefact.symbol_fqn,
                artefact.signature,
                artefact.modifiers,
                artefact.docstring,
                artefact.content_hash
            ],
        )
        .with_context(|| format!("failed upserting artefact {}", artefact.artefact_id))?;

        tx.execute(
            r#"
INSERT INTO artefact_snapshots (
  repo_id, blob_sha, path, artefact_id, parent_artefact_id,
  start_line, end_line, start_byte, end_byte
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
)
ON CONFLICT(repo_id, blob_sha, artefact_id) DO UPDATE SET
  path = excluded.path,
  parent_artefact_id = excluded.parent_artefact_id,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  start_byte = excluded.start_byte,
  end_byte = excluded.end_byte
"#,
            params![
                artefact.repo_id,
                artefact.blob_sha,
                artefact.path,
                artefact.artefact_id,
                artefact.parent_artefact_id,
                artefact.start_line,
                artefact.end_line,
                artefact.start_byte,
                artefact.end_byte,
            ],
        )
        .with_context(|| {
            format!(
                "failed upserting artefact snapshot {}",
                artefact.artefact_id
            )
        })?;
    }

    for artefact in &batch.current_artefacts {
        tx.execute(
            r#"
INSERT INTO artefacts_current (
  repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind,
  language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
  start_byte, end_byte, signature, modifiers, docstring, updated_at
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
  datetime('now')
)
ON CONFLICT(repo_id, path, symbol_id) DO UPDATE SET
  content_id = excluded.content_id,
  artefact_id = excluded.artefact_id,
  language = excluded.language,
  canonical_kind = excluded.canonical_kind,
  language_kind = excluded.language_kind,
  symbol_fqn = excluded.symbol_fqn,
  parent_symbol_id = excluded.parent_symbol_id,
  parent_artefact_id = excluded.parent_artefact_id,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  start_byte = excluded.start_byte,
  end_byte = excluded.end_byte,
  signature = excluded.signature,
  modifiers = excluded.modifiers,
  docstring = excluded.docstring,
  updated_at = excluded.updated_at
"#,
            params![
                artefact.repo_id,
                artefact.path,
                artefact.blob_sha,
                artefact.symbol_id,
                artefact.artefact_id,
                artefact.language,
                artefact.canonical_kind,
                artefact.language_kind,
                artefact.symbol_fqn,
                artefact.parent_symbol_id,
                artefact.parent_artefact_id,
                artefact.start_line,
                artefact.end_line,
                artefact.start_byte,
                artefact.end_byte,
                artefact.signature,
                artefact.modifiers,
                artefact.docstring,
            ],
        )
        .with_context(|| format!("failed upserting current artefact {}", artefact.symbol_id))?;
    }

    tx.commit()
        .context("failed to commit production seed transaction")?;
    Ok(())
}

fn resolve_repository_record(repo_dir: &Path) -> Result<RepositoryRecord> {
    let fallback_name = repo_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("repo")
        .to_string();
    let remote = run_git(repo_dir, &["config", "--get", "remote.origin.url"]).unwrap_or_default();
    let remote = remote.trim();

    let (provider, organization, name) = if remote.is_empty() {
        ("local".to_string(), "local".to_string(), fallback_name)
    } else if let Some((org, name)) = parse_remote_owner_name(remote) {
        let provider = if remote.contains("github") {
            "github"
        } else if remote.contains("gitlab") {
            "gitlab"
        } else {
            "git"
        };
        (provider.to_string(), org, name)
    } else {
        ("git".to_string(), "local".to_string(), fallback_name)
    };

    let identity = format!("{provider}://{organization}/{name}");
    Ok(RepositoryRecord {
        repo_id: deterministic_uuid(&identity),
        provider,
        organization,
        name,
        default_branch: Some(default_branch_name(repo_dir)),
        metadata_json: None,
    })
}

fn run_git(repo_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn default_branch_name(repo_root: &Path) -> String {
    run_git(repo_root, &["branch", "--show-current"])
        .filter(|branch| !branch.trim().is_empty())
        .unwrap_or_else(|| "main".to_string())
}

fn parse_remote_owner_name(remote: &str) -> Option<(String, String)> {
    let trimmed = remote.trim().trim_end_matches('/');

    if let Some(rest) = trimmed.strip_prefix("git@") {
        let (_, path) = rest.split_once(':')?;
        return parse_owner_name_path(path);
    }

    if let Some(pos) = trimmed.find("://") {
        let rest = &trimmed[pos + 3..];
        let (_, path) = rest.split_once('/')?;
        return parse_owner_name_path(path);
    }

    None
}

fn parse_owner_name_path(path: &str) -> Option<(String, String)> {
    let clean = path.trim().trim_end_matches(".git");
    let mut parts = clean
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let name = parts.pop()?.to_string();
    let org = parts.pop()?.to_string();
    Some((org, name))
}

fn find_production_files(repo_dir: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(repo_dir)
        .into_iter()
        .filter_entry(|entry| !is_ignored_path(entry.path()))
        .filter_map(|item| item.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if !is_production_source_file(path) {
            continue;
        }

        let relative = path
            .strip_prefix(repo_dir)
            .with_context(|| format!("file {} is not under repo dir", path.display()))?;
        files.push(normalize_rel_path(relative));
    }

    files.sort();
    Ok(files)
}

fn is_production_source_file(path: &Path) -> bool {
    let normalized = normalize_rel_path(path);
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if production_source_language(filename).is_empty() {
        return false;
    }
    if filename.ends_with(".d.ts") {
        return false;
    }
    !is_test_file(path, &normalized)
}

fn production_source_language(path: &str) -> &'static str {
    let Some(extension) = Path::new(path).extension().and_then(|value| value.to_str()) else {
        return "";
    };

    match extension.trim().to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "cs" => "csharp",
        _ => "",
    }
}

fn is_ignored_path(path: &Path) -> bool {
    let normalized = normalize_rel_path(path);
    normalized.contains("/node_modules/")
        || normalized.contains("/coverage/")
        || normalized.contains("/dist/")
        || normalized.contains("/target/")
}

fn is_test_file(path: &Path, normalized: &str) -> bool {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    normalized.starts_with("tests/")
        || normalized.contains("/tests/")
        || normalized.contains("/__tests__/")
        || filename.ends_with(".test.ts")
        || filename.ends_with(".spec.ts")
        || filename.ends_with(".test.rs")
        || filename.ends_with(".spec.rs")
}

#[allow(clippy::too_many_arguments)]
fn ingest_adapter_artefacts(
    host_services: &dyn HostServicesGateway,
    builder: &mut ProductionBatchBuilder,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
    language: &str,
    file_ids: &ParsedArtefactIds,
    extraction: Option<bitloops::host::devql::ExtractedProductionFile>,
    source: &[u8],
) -> usize {
    let Some(extraction) = extraction else {
        return 0;
    };

    let mut inserted = 0usize;
    let mut symbol_ids_by_fqn = HashMap::new();
    let mut artefact_ids_by_fqn = HashMap::new();
    symbol_ids_by_fqn.insert(path.to_string(), file_ids.symbol_id.clone());
    artefact_ids_by_fqn.insert(path.to_string(), file_ids.artefact_id.clone());

    for artefact in extraction.artefacts {
        let canonical_kind = artefact.canonical_kind.as_deref().unwrap_or("<null>");
        let parent_symbol_id = artefact
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_ids_by_fqn.get(fqn))
            .map(String::as_str)
            .or(Some(file_ids.symbol_id.as_str()));
        let parent_artefact_id = artefact
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| artefact_ids_by_fqn.get(fqn))
            .map(String::as_str)
            .or(Some(file_ids.artefact_id.as_str()));
        let signature = if artefact.signature.trim().is_empty() {
            None
        } else {
            Some(artefact.signature.as_str())
        };

        let ids = builder.push_artefact(
            host_services,
            repo_id,
            commit_sha,
            blob_sha,
            path,
            language,
            canonical_kind,
            Some(artefact.language_kind.as_str()),
            Some(artefact.symbol_fqn.as_str()),
            artefact.name.as_str(),
            parent_symbol_id,
            parent_artefact_id,
            artefact.start_line as i64,
            artefact.end_line as i64,
            artefact.start_byte as i64,
            artefact.end_byte as i64,
            signature,
            artefact.modifiers.as_slice(),
            artefact.docstring.as_deref(),
            artefact_source_bytes(artefact.start_byte, artefact.end_byte, source),
        );
        symbol_ids_by_fqn.insert(artefact.symbol_fqn.clone(), ids.symbol_id.clone());
        artefact_ids_by_fqn.insert(artefact.symbol_fqn, ids.artefact_id.clone());
        inserted += 1;
    }

    inserted
}

fn artefact_source_bytes(start_byte: i32, end_byte: i32, source: &[u8]) -> &[u8] {
    let len = source.len();
    let start = usize::try_from(start_byte).unwrap_or_default().min(len);
    let end = usize::try_from(end_byte).unwrap_or_default().min(len);
    if start >= end {
        &[]
    } else {
        &source[start..end]
    }
}

fn file_symbol_id(path: &str) -> String {
    deterministic_uuid(&format!("{path}|file"))
}

fn deterministic_uuid(input: &str) -> String {
    let digest = sha256_hex(input.as_bytes());
    let hex = &digest[..32];
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn collect_duplicate_current_artefacts(
    artefacts: &[CurrentProductionArtefactRecord],
) -> Vec<Vec<DuplicateCurrentArtefactSummary>> {
    let mut by_artefact_id: BTreeMap<&str, Vec<DuplicateCurrentArtefactSummary>> = BTreeMap::new();
    for artefact in artefacts {
        by_artefact_id
            .entry(artefact.artefact_id.as_str())
            .or_default()
            .push(DuplicateCurrentArtefactSummary {
                artefact_id: artefact.artefact_id.clone(),
                path: artefact.path.clone(),
                canonical_kind: artefact.canonical_kind.clone(),
                symbol_fqn: artefact.symbol_fqn.clone(),
                language_kind: artefact.language_kind.clone(),
                start_line: artefact.start_line,
                end_line: artefact.end_line,
                signature: artefact.signature.clone(),
            });
    }

    by_artefact_id
        .into_values()
        .filter(|group| group.len() > 1)
        .collect()
}

fn normalize_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
