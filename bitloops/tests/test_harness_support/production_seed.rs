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
use tree_sitter::{Node, Parser};
use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;
use tree_sitter_typescript::LANGUAGE_TYPESCRIPT;
use walkdir::WalkDir;

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
        content_bytes: &[u8],
    ) -> ParsedArtefactIds {
        let symbol_id = if canonical_kind == "file" {
            file_symbol_id(path)
        } else {
            structural_symbol_id(
                path,
                canonical_kind,
                language_kind,
                parent_symbol_id,
                identity_name,
                signature,
            )
        };
        let artefact_id = revision_artefact_id(repo_id, blob_sha, &symbol_id);
        let content_hash = Some(sha256_hex(content_bytes));
        let modifiers = "[]".to_string();

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
            docstring: None,
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
                docstring: None,
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

pub fn execute(db_path: &Path, repo_dir: &Path, commit_sha: &str) -> Result<IngestProductionSummary> {
    let repo = resolve_repository_record(repo_dir)?;
    let production_files = find_production_files(repo_dir)?;
    let committed_at = chrono::Utc::now().to_rfc3339();

    let mut ts_parser = Parser::new();
    ts_parser
        .set_language(&LANGUAGE_TYPESCRIPT.into())
        .context("failed to load TypeScript parser")?;

    let mut rust_parser = Parser::new();
    rust_parser
        .set_language(&LANGUAGE_RUST.into())
        .context("failed to load Rust parser")?;

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

        let (language, parser) = if relative_path.ends_with(".rs") {
            ("rust", &mut rust_parser)
        } else {
            ("typescript", &mut ts_parser)
        };

        builder.push_file_state(
            &repo.repo_id,
            commit_sha,
            &relative_path,
            &blob_sha,
            &committed_at,
        );

        let file_ids = builder.push_artefact(
            &repo.repo_id,
            commit_sha,
            &blob_sha,
            &relative_path,
            language,
            "file",
            Some("source_file"),
            Some(&relative_path),
            &relative_path,
            None,
            None,
            1,
            end_line,
            0,
            source_bytes.len() as i64,
            None,
            source_bytes,
        );
        stats.artefacts += 1;

        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("failed parsing source file {}", abs_path.display()))?;
        let root = tree.root_node();

        if language == "typescript" {
            stats.artefacts += ingest_typescript_artefacts(
                &mut builder,
                &repo.repo_id,
                commit_sha,
                &relative_path,
                &blob_sha,
                &file_ids,
                root,
                source_bytes,
            )?;
        } else {
            stats.artefacts += ingest_rust_artefacts(
                &mut builder,
                &repo.repo_id,
                commit_sha,
                &relative_path,
                &blob_sha,
                &file_ids,
                root,
                source_bytes,
            )?;
        }
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
    commit: &CommitRecord,
    batch: &ProductionBatchBuilder,
) -> Result<()> {
    let mut conn = Connection::open(db_path)
        .with_context(|| format!("failed opening sqlite database at {}", db_path.display()))?;
    let tx = conn.transaction().context("failed to open sqlite transaction")?;

    tx.execute(
        "DELETE FROM test_artefact_edges_current WHERE commit_sha = ?1",
        params![commit.commit_sha],
    )
    .context("failed clearing test_artefact_edges_current for commit")?;
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
        "DELETE FROM artefact_edges_current WHERE commit_sha = ?1",
        params![commit.commit_sha],
    )
    .context("failed clearing artefact_edges_current for commit")?;
    tx.execute(
        "DELETE FROM artefacts_current WHERE commit_sha = ?1",
        params![commit.commit_sha],
    )
    .context("failed clearing artefacts_current for commit")?;
    if table_exists(&tx, "current_file_state")? {
        tx.execute(
            "DELETE FROM current_file_state WHERE commit_sha = ?1",
            params![commit.commit_sha],
        )
        .context("failed clearing current_file_state for commit")?;
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
        .with_context(|| format!("failed upserting file_state {} {}", row.commit_sha, row.path))?;
    }

    for row in &batch.current_file_states {
        tx.execute(
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
        )
        .with_context(|| format!("failed upserting current_file_state {}", row.path))?;
    }

    for artefact in &batch.artefacts {
        tx.execute(
            r#"
INSERT INTO artefacts (
  artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte,
  end_byte, signature, modifiers, docstring, content_hash
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
)
ON CONFLICT(artefact_id) DO UPDATE SET
  symbol_id = excluded.symbol_id,
  repo_id = excluded.repo_id,
  blob_sha = excluded.blob_sha,
  path = excluded.path,
  language = excluded.language,
  canonical_kind = excluded.canonical_kind,
  language_kind = excluded.language_kind,
  symbol_fqn = excluded.symbol_fqn,
  parent_artefact_id = excluded.parent_artefact_id,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  start_byte = excluded.start_byte,
  end_byte = excluded.end_byte,
  signature = excluded.signature,
  modifiers = excluded.modifiers,
  docstring = excluded.docstring,
  content_hash = excluded.content_hash
"#,
            params![
                artefact.artefact_id,
                artefact.symbol_id,
                artefact.repo_id,
                artefact.blob_sha,
                artefact.path,
                artefact.language,
                artefact.canonical_kind,
                artefact.language_kind,
                artefact.symbol_fqn,
                artefact.parent_artefact_id,
                artefact.start_line,
                artefact.end_line,
                artefact.start_byte,
                artefact.end_byte,
                artefact.signature,
                artefact.modifiers,
                artefact.docstring,
                artefact.content_hash
            ],
        )
        .with_context(|| format!("failed upserting artefact {}", artefact.artefact_id))?;
    }

    for artefact in &batch.current_artefacts {
        tx.execute(
            r#"
INSERT INTO artefacts_current (
  repo_id, branch, symbol_id, artefact_id, commit_sha, blob_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
  start_byte, end_byte, signature, modifiers, docstring, content_hash
) VALUES (
  ?1, 'main', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
)
ON CONFLICT(repo_id, branch, symbol_id) DO UPDATE SET
  artefact_id = excluded.artefact_id,
  commit_sha = excluded.commit_sha,
  blob_sha = excluded.blob_sha,
  path = excluded.path,
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
  content_hash = excluded.content_hash,
  updated_at = datetime('now')
"#,
            params![
                artefact.repo_id,
                artefact.symbol_id,
                artefact.artefact_id,
                artefact.commit_sha,
                artefact.blob_sha,
                artefact.path,
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
                artefact.content_hash
            ],
        )
        .with_context(|| format!("failed upserting current artefact {}", artefact.symbol_id))?;
    }

    tx.commit().context("failed to commit production seed transaction")?;
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
    let is_source_extension = filename.ends_with(".ts") || filename.ends_with(".rs");
    if !is_source_extension {
        return false;
    }
    if filename.ends_with(".d.ts") {
        return false;
    }
    !is_test_file(path, &normalized)
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
fn ingest_typescript_artefacts(
    builder: &mut ProductionBatchBuilder,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
    file_ids: &ParsedArtefactIds,
    root: Node<'_>,
    source: &[u8],
) -> Result<usize> {
    let mut inserted = 0usize;
    let mut cursor = root.walk();

    for child in root.named_children(&mut cursor) {
        let declaration = if child.kind() == "export_statement" {
            child.named_child(0).unwrap_or(child)
        } else {
            child
        };

        match declaration.kind() {
            "function_declaration" => {
                if let Some(name) = declaration
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    builder.push_artefact(
                        repo_id,
                        commit_sha,
                        blob_sha,
                        path,
                        "typescript",
                        "function",
                        Some("function_declaration"),
                        Some(name),
                        name,
                        Some(&file_ids.symbol_id),
                        Some(&file_ids.artefact_id),
                        declaration.start_position().row as i64 + 1,
                        declaration.end_position().row as i64 + 1,
                        declaration.start_byte() as i64,
                        declaration.end_byte() as i64,
                        compact_signature(declaration, source).as_deref(),
                        node_bytes(declaration, source),
                    );
                    inserted += 1;
                }
            }
            "class_declaration" => {
                let Some(class_name) = declaration
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                else {
                    continue;
                };

                let class_ids = builder.push_artefact(
                    repo_id,
                    commit_sha,
                    blob_sha,
                    path,
                    "typescript",
                    "class",
                    Some("class_declaration"),
                    Some(class_name),
                    class_name,
                    Some(&file_ids.symbol_id),
                    Some(&file_ids.artefact_id),
                    declaration.start_position().row as i64 + 1,
                    declaration.end_position().row as i64 + 1,
                    declaration.start_byte() as i64,
                    declaration.end_byte() as i64,
                    compact_signature(declaration, source).as_deref(),
                    node_bytes(declaration, source),
                );
                inserted += 1;

                if let Some(body) = declaration.child_by_field_name("body") {
                    let mut body_cursor = body.walk();
                    for body_child in body.named_children(&mut body_cursor) {
                        if body_child.kind() != "method_definition" {
                            continue;
                        }
                        let Some(method_name_raw) = body_child
                            .child_by_field_name("name")
                            .and_then(|node| node.utf8_text(source).ok())
                        else {
                            continue;
                        };
                        let method_name =
                            unquote(method_name_raw).unwrap_or_else(|| method_name_raw.to_string());
                        let symbol_fqn = format!("{class_name}.{method_name}");

                        builder.push_artefact(
                            repo_id,
                            commit_sha,
                            blob_sha,
                            path,
                            "typescript",
                            "method",
                            Some("method_definition"),
                            Some(&symbol_fqn),
                            &method_name,
                            Some(&class_ids.symbol_id),
                            Some(&class_ids.artefact_id),
                            body_child.start_position().row as i64 + 1,
                            body_child.end_position().row as i64 + 1,
                            body_child.start_byte() as i64,
                            body_child.end_byte() as i64,
                            compact_signature(body_child, source).as_deref(),
                            node_bytes(body_child, source),
                        );
                        inserted += 1;
                    }
                }
            }
            "interface_declaration" => {
                if let Some(name) = declaration
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    builder.push_artefact(
                        repo_id,
                        commit_sha,
                        blob_sha,
                        path,
                        "typescript",
                        "interface",
                        Some("interface_declaration"),
                        Some(name),
                        name,
                        Some(&file_ids.symbol_id),
                        Some(&file_ids.artefact_id),
                        declaration.start_position().row as i64 + 1,
                        declaration.end_position().row as i64 + 1,
                        declaration.start_byte() as i64,
                        declaration.end_byte() as i64,
                        compact_signature(declaration, source).as_deref(),
                        node_bytes(declaration, source),
                    );
                    inserted += 1;
                }
            }
            "type_alias_declaration" => {
                if let Some(name) = declaration
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    builder.push_artefact(
                        repo_id,
                        commit_sha,
                        blob_sha,
                        path,
                        "typescript",
                        "type",
                        Some("type_alias_declaration"),
                        Some(name),
                        name,
                        Some(&file_ids.symbol_id),
                        Some(&file_ids.artefact_id),
                        declaration.start_position().row as i64 + 1,
                        declaration.end_position().row as i64 + 1,
                        declaration.start_byte() as i64,
                        declaration.end_byte() as i64,
                        compact_signature(declaration, source).as_deref(),
                        node_bytes(declaration, source),
                    );
                    inserted += 1;
                }
            }
            "lexical_declaration" => {
                if !declaration_is_const(declaration, source) {
                    continue;
                }
                for const_name in collect_variable_declarator_names(declaration, source) {
                    builder.push_artefact(
                        repo_id,
                        commit_sha,
                        blob_sha,
                        path,
                        "typescript",
                        "constant",
                        Some("variable_declaration"),
                        Some(&const_name),
                        &const_name,
                        Some(&file_ids.symbol_id),
                        Some(&file_ids.artefact_id),
                        declaration.start_position().row as i64 + 1,
                        declaration.end_position().row as i64 + 1,
                        declaration.start_byte() as i64,
                        declaration.end_byte() as i64,
                        compact_signature(declaration, source).as_deref(),
                        node_bytes(declaration, source),
                    );
                    inserted += 1;
                }
            }
            _ => {}
        }
    }

    Ok(inserted)
}

#[allow(clippy::too_many_arguments)]
fn ingest_rust_artefacts(
    builder: &mut ProductionBatchBuilder,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
    file_ids: &ParsedArtefactIds,
    root: Node<'_>,
    source: &[u8],
) -> Result<usize> {
    let mut inserted = 0usize;
    let mut cursor = root.walk();
    let mut parent_types: HashMap<String, ParsedArtefactIds> = HashMap::new();

    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    builder.push_artefact(
                        repo_id,
                        commit_sha,
                        blob_sha,
                        path,
                        "rust",
                        "function",
                        Some("function_item"),
                        Some(name),
                        name,
                        Some(&file_ids.symbol_id),
                        Some(&file_ids.artefact_id),
                        child.start_position().row as i64 + 1,
                        child.end_position().row as i64 + 1,
                        child.start_byte() as i64,
                        child.end_byte() as i64,
                        compact_signature(child, source).as_deref(),
                        node_bytes(child, source),
                    );
                    inserted += 1;
                }
            }
            "struct_item" | "enum_item" | "type_item" | "trait_item" => {
                if let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    let canonical_kind = if child.kind() == "trait_item" {
                        "interface"
                    } else {
                        "type"
                    };
                    let ids = builder.push_artefact(
                        repo_id,
                        commit_sha,
                        blob_sha,
                        path,
                        "rust",
                        canonical_kind,
                        Some(child.kind()),
                        Some(name),
                        name,
                        Some(&file_ids.symbol_id),
                        Some(&file_ids.artefact_id),
                        child.start_position().row as i64 + 1,
                        child.end_position().row as i64 + 1,
                        child.start_byte() as i64,
                        child.end_byte() as i64,
                        compact_signature(child, source).as_deref(),
                        node_bytes(child, source),
                    );
                    parent_types.insert(name.to_string(), ids);
                    inserted += 1;
                }
            }
            "impl_item" => {
                let impl_type =
                    extract_rust_impl_type(child, source).unwrap_or_else(|| "impl".to_string());
                let impl_identity =
                    extract_rust_impl_identity(child, source).unwrap_or_else(|| impl_type.clone());
                let parent_ids = parent_types.get(&impl_type).unwrap_or(file_ids);

                let mut impl_cursor = child.walk();
                for impl_child in child.named_children(&mut impl_cursor) {
                    if impl_child.kind() != "declaration_list" {
                        continue;
                    }
                    let mut decl_cursor = impl_child.walk();
                    for decl in impl_child.named_children(&mut decl_cursor) {
                        if decl.kind() != "function_item" {
                            continue;
                        }
                        let Some(method_name) = decl
                            .child_by_field_name("name")
                            .and_then(|node| node.utf8_text(source).ok())
                        else {
                            continue;
                        };
                        let symbol_fqn = format!("{impl_identity}.{method_name}");
                        builder.push_artefact(
                            repo_id,
                            commit_sha,
                            blob_sha,
                            path,
                            "rust",
                            "method",
                            Some("impl_method"),
                            Some(&symbol_fqn),
                            &symbol_fqn,
                            Some(&parent_ids.symbol_id),
                            Some(&parent_ids.artefact_id),
                            decl.start_position().row as i64 + 1,
                            decl.end_position().row as i64 + 1,
                            decl.start_byte() as i64,
                            decl.end_byte() as i64,
                            compact_signature(decl, source).as_deref(),
                            node_bytes(decl, source),
                        );
                        inserted += 1;
                    }
                }
            }
            _ => {}
        }
    }

    Ok(inserted)
}

fn extract_rust_impl_type(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(type_name) = node
        .child_by_field_name("type")
        .and_then(|type_node| clean_node_text(type_node, source))
    {
        return Some(type_name);
    }

    let raw = node.utf8_text(source).ok()?;
    let after_impl = raw.strip_prefix("impl")?.trim_start();
    let first = after_impl.split_whitespace().next()?;
    let cleaned = first
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_matches('{')
        .trim_matches(':')
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn extract_rust_impl_identity(node: Node<'_>, source: &[u8]) -> Option<String> {
    let impl_type = extract_rust_impl_type(node, source)?;
    let impl_trait = node
        .child_by_field_name("trait")
        .and_then(|trait_node| clean_node_text(trait_node, source));
    match impl_trait {
        Some(trait_name) => Some(format!("{trait_name} for {impl_type}")),
        None => Some(impl_type),
    }
}

fn clean_node_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    let raw = node.utf8_text(source).ok()?;
    let cleaned = raw.trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn declaration_is_const(node: Node<'_>, source: &[u8]) -> bool {
    node.utf8_text(source)
        .ok()
        .map(|text| text.trim_start().starts_with("const "))
        .unwrap_or(false)
}

fn collect_variable_declarator_names(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut stack = vec![node];

    while let Some(current) = stack.pop() {
        if current.kind() == "variable_declarator"
            && let Some(name_node) = current.child_by_field_name("name")
            && name_node.kind() == "identifier"
            && let Ok(name) = name_node.utf8_text(source)
        {
            names.push(name.to_string());
        }

        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }

    names
}

fn compact_signature(node: Node<'_>, source: &[u8]) -> Option<String> {
    let raw = node.utf8_text(source).ok()?;
    let first_line = raw.lines().next()?.trim();
    if first_line.is_empty() {
        None
    } else {
        Some(first_line.chars().take(240).collect())
    }
}

fn node_bytes<'a>(node: Node<'_>, source: &'a [u8]) -> &'a [u8] {
    &source[node.start_byte()..node.end_byte()]
}

fn normalize_identity_fragment(input: &str) -> String {
    let normalized = input
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if normalized.is_empty() {
        input.trim().to_string()
    } else {
        normalized
    }
}

fn structural_symbol_id(
    path: &str,
    canonical_kind: &str,
    language_kind: Option<&str>,
    parent_symbol_id: Option<&str>,
    identity_name: &str,
    signature: Option<&str>,
) -> String {
    deterministic_uuid(&format!(
        "{}|{}|{}|{}|{}|{}",
        path,
        canonical_kind,
        language_kind.unwrap_or(""),
        parent_symbol_id.unwrap_or(""),
        normalize_identity_fragment(identity_name),
        normalize_identity_fragment(signature.unwrap_or(identity_name))
    ))
}

fn file_symbol_id(path: &str) -> String {
    deterministic_uuid(&format!("{path}|file"))
}

fn revision_artefact_id(repo_id: &str, blob_sha: &str, symbol_id: &str) -> String {
    deterministic_uuid(&format!("{repo_id}|{blob_sha}|{symbol_id}"))
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
    format!("{:x}", hasher.finalize())
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

fn unquote(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() < 2 {
        return None;
    }

    let start = trimmed.as_bytes()[0] as char;
    let end = trimmed.as_bytes()[trimmed.len() - 1] as char;
    if (start == '\'' || start == '"' || start == '`') && start == end {
        Some(trimmed[1..trimmed.len() - 1].to_string())
    } else {
        None
    }
}
