// Command handler for discovering production source artefacts and materializing
// them into the commit-addressed SQLite model.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};
use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;
use tree_sitter_typescript::LANGUAGE_TYPESCRIPT;
use walkdir::WalkDir;

use crate::domain::ArtefactRecord;
use crate::repository::{TestHarnessRepository, open_sqlite_repository};

#[derive(Debug, Default, Clone, Copy)]
struct IngestProductionStats {
    files: usize,
    artefacts: usize,
}

pub fn handle(db_path: &Path, repo_dir: &Path, commit_sha: &str) -> Result<()> {
    let mut repository = open_sqlite_repository(db_path)?;
    let repo_id = repo_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("repo")
        .to_string();

    let production_files = find_production_files(repo_dir)?;

    let mut ts_parser = Parser::new();
    ts_parser
        .set_language(&LANGUAGE_TYPESCRIPT.into())
        .context("failed to load TypeScript parser")?;

    let mut rust_parser = Parser::new();
    rust_parser
        .set_language(&LANGUAGE_RUST.into())
        .context("failed to load Rust parser")?;

    let mut stats = IngestProductionStats::default();
    let mut artefacts = Vec::new();

    for relative_path in production_files {
        stats.files += 1;

        let abs_path = repo_dir.join(&relative_path);
        let source = fs::read_to_string(&abs_path)
            .with_context(|| format!("failed reading source file {}", abs_path.display()))?;
        let bytes = source.as_bytes();
        let end_line = std::cmp::max(source.lines().count() as i64, 1);

        let (language, parser) = if relative_path.ends_with(".rs") {
            ("rust", &mut rust_parser)
        } else {
            ("typescript", &mut ts_parser)
        };

        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("failed parsing source file {}", abs_path.display()))?;
        let root = tree.root_node();

        let file_id = format!("prod:{commit_sha}:{relative_path}:file");
        artefacts.push(build_artefact_record(
            &file_id,
            &repo_id,
            commit_sha,
            &relative_path,
            language,
            "file",
            Some("source_file"),
            Some(&relative_path),
            None,
            1,
            end_line,
            None,
        ));
        stats.artefacts += 1;

        if language == "typescript" {
            stats.artefacts += ingest_typescript_artefacts(
                &mut artefacts,
                &repo_id,
                commit_sha,
                &relative_path,
                &file_id,
                root,
                bytes,
            )?;
        } else {
            stats.artefacts += ingest_rust_artefacts(
                &mut artefacts,
                &repo_id,
                commit_sha,
                &relative_path,
                &file_id,
                root,
                bytes,
            )?;
        }
    }

    repository.replace_production_artefacts(commit_sha, &artefacts)?;
    println!(
        "ingest-production-artefacts complete for commit {} (files: {}, artefacts: {})",
        commit_sha, stats.files, stats.artefacts
    );
    Ok(())
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

fn ingest_typescript_artefacts(
    artefacts: &mut Vec<ArtefactRecord>,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    file_id: &str,
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
                    let artefact_id =
                        production_artefact_id(commit_sha, path, "function", declaration, name);
                    artefacts.push(build_artefact_record(
                        &artefact_id,
                        repo_id,
                        commit_sha,
                        path,
                        "typescript",
                        "function",
                        Some("function_declaration"),
                        Some(name),
                        Some(file_id),
                        declaration.start_position().row as i64 + 1,
                        declaration.end_position().row as i64 + 1,
                        compact_signature(declaration, source).as_deref(),
                    ));
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

                let class_id =
                    production_artefact_id(commit_sha, path, "class", declaration, class_name);
                artefacts.push(build_artefact_record(
                    &class_id,
                    repo_id,
                    commit_sha,
                    path,
                    "typescript",
                    "class",
                    Some("class_declaration"),
                    Some(class_name),
                    Some(file_id),
                    declaration.start_position().row as i64 + 1,
                    declaration.end_position().row as i64 + 1,
                    compact_signature(declaration, source).as_deref(),
                ));
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
                        let method_id = production_artefact_id(
                            commit_sha,
                            path,
                            "method",
                            body_child,
                            &symbol_fqn,
                        );

                        artefacts.push(build_artefact_record(
                            &method_id,
                            repo_id,
                            commit_sha,
                            path,
                            "typescript",
                            "method",
                            Some("method_definition"),
                            Some(&symbol_fqn),
                            Some(&class_id),
                            body_child.start_position().row as i64 + 1,
                            body_child.end_position().row as i64 + 1,
                            compact_signature(body_child, source).as_deref(),
                        ));
                        inserted += 1;
                    }
                }
            }
            "interface_declaration" => {
                if let Some(name) = declaration
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    let artefact_id =
                        production_artefact_id(commit_sha, path, "interface", declaration, name);
                    artefacts.push(build_artefact_record(
                        &artefact_id,
                        repo_id,
                        commit_sha,
                        path,
                        "typescript",
                        "interface",
                        Some("interface_declaration"),
                        Some(name),
                        Some(file_id),
                        declaration.start_position().row as i64 + 1,
                        declaration.end_position().row as i64 + 1,
                        compact_signature(declaration, source).as_deref(),
                    ));
                    inserted += 1;
                }
            }
            "type_alias_declaration" => {
                if let Some(name) = declaration
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    let artefact_id =
                        production_artefact_id(commit_sha, path, "type", declaration, name);
                    artefacts.push(build_artefact_record(
                        &artefact_id,
                        repo_id,
                        commit_sha,
                        path,
                        "typescript",
                        "type",
                        Some("type_alias_declaration"),
                        Some(name),
                        Some(file_id),
                        declaration.start_position().row as i64 + 1,
                        declaration.end_position().row as i64 + 1,
                        compact_signature(declaration, source).as_deref(),
                    ));
                    inserted += 1;
                }
            }
            "lexical_declaration" => {
                if !declaration_is_const(declaration, source) {
                    continue;
                }
                for const_name in collect_variable_declarator_names(declaration, source) {
                    let artefact_id = production_artefact_id(
                        commit_sha,
                        path,
                        "constant",
                        declaration,
                        &const_name,
                    );
                    artefacts.push(build_artefact_record(
                        &artefact_id,
                        repo_id,
                        commit_sha,
                        path,
                        "typescript",
                        "constant",
                        Some("variable_declaration"),
                        Some(&const_name),
                        Some(file_id),
                        declaration.start_position().row as i64 + 1,
                        declaration.end_position().row as i64 + 1,
                        compact_signature(declaration, source).as_deref(),
                    ));
                    inserted += 1;
                }
            }
            _ => {}
        }
    }

    Ok(inserted)
}

fn ingest_rust_artefacts(
    artefacts: &mut Vec<ArtefactRecord>,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    file_id: &str,
    root: Node<'_>,
    source: &[u8],
) -> Result<usize> {
    let mut inserted = 0usize;
    let mut cursor = root.walk();

    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    let artefact_id =
                        production_artefact_id(commit_sha, path, "function", child, name);
                    artefacts.push(build_artefact_record(
                        &artefact_id,
                        repo_id,
                        commit_sha,
                        path,
                        "rust",
                        "function",
                        Some("function_item"),
                        Some(name),
                        Some(file_id),
                        child.start_position().row as i64 + 1,
                        child.end_position().row as i64 + 1,
                        compact_signature(child, source).as_deref(),
                    ));
                    inserted += 1;
                }
            }
            "struct_item" | "enum_item" | "type_item" => {
                if let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    let artefact_id = production_artefact_id(commit_sha, path, "type", child, name);
                    artefacts.push(build_artefact_record(
                        &artefact_id,
                        repo_id,
                        commit_sha,
                        path,
                        "rust",
                        "type",
                        Some(child.kind()),
                        Some(name),
                        Some(file_id),
                        child.start_position().row as i64 + 1,
                        child.end_position().row as i64 + 1,
                        compact_signature(child, source).as_deref(),
                    ));
                    inserted += 1;
                }
            }
            "trait_item" => {
                if let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source).ok())
                {
                    let artefact_id =
                        production_artefact_id(commit_sha, path, "interface", child, name);
                    artefacts.push(build_artefact_record(
                        &artefact_id,
                        repo_id,
                        commit_sha,
                        path,
                        "rust",
                        "interface",
                        Some("trait_item"),
                        Some(name),
                        Some(file_id),
                        child.start_position().row as i64 + 1,
                        child.end_position().row as i64 + 1,
                        compact_signature(child, source).as_deref(),
                    ));
                    inserted += 1;
                }
            }
            "impl_item" => {
                let impl_type =
                    extract_rust_impl_type(child, source).unwrap_or_else(|| "impl".to_string());
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
                        let symbol_fqn = format!("{impl_type}.{method_name}");
                        let artefact_id =
                            production_artefact_id(commit_sha, path, "method", decl, &symbol_fqn);

                        artefacts.push(build_artefact_record(
                            &artefact_id,
                            repo_id,
                            commit_sha,
                            path,
                            "rust",
                            "method",
                            Some("impl_method"),
                            Some(&symbol_fqn),
                            Some(file_id),
                            decl.start_position().row as i64 + 1,
                            decl.end_position().row as i64 + 1,
                            compact_signature(decl, source).as_deref(),
                        ));
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
    if let Some(type_node) = node.child_by_field_name("type")
        && let Ok(raw) = type_node.utf8_text(source)
    {
        return Some(raw.to_string());
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

fn production_artefact_id(
    commit_sha: &str,
    path: &str,
    canonical_kind: &str,
    node: Node<'_>,
    symbol_fqn: &str,
) -> String {
    let slug = symbol_fqn
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!(
        "prod:{commit_sha}:{path}:{canonical_kind}:{}:{slug}",
        node.start_position().row as i64 + 1
    )
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

fn build_artefact_record(
    artefact_id: &str,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    language: &str,
    canonical_kind: &str,
    language_kind: Option<&str>,
    symbol_fqn: Option<&str>,
    parent_artefact_id: Option<&str>,
    start_line: i64,
    end_line: i64,
    signature: Option<&str>,
) -> ArtefactRecord {
    ArtefactRecord {
        artefact_id: artefact_id.to_string(),
        repo_id: repo_id.to_string(),
        commit_sha: commit_sha.to_string(),
        path: path.to_string(),
        language: language.to_string(),
        canonical_kind: canonical_kind.to_string(),
        language_kind: language_kind.map(str::to_string),
        symbol_fqn: symbol_fqn.map(str::to_string),
        parent_artefact_id: parent_artefact_id.map(str::to_string),
        start_line,
        end_line,
        signature: signature.map(str::to_string),
    }
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

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::{Connection, params};
    use tempfile::tempdir;

    use super::handle;
    use crate::db::init_database;

    #[test]
    fn ingests_typescript_and_rust_production_artefacts() {
        let temp = tempdir().expect("failed to create temp dir");
        let repo_dir = temp.path().join("repo");
        let db_path = temp.path().join("testlens.db");

        fs::create_dir_all(repo_dir.join("src")).expect("failed creating src dir");
        fs::create_dir_all(repo_dir.join("tests")).expect("failed creating tests dir");

        fs::write(
            repo_dir.join("src/service.ts"),
            r#"
export interface User {
  id: string;
}

export type UserId = string;
export const MAX_RETRIES = 3;

export function validateEmail(email: string): boolean {
  return email.includes("@");
}

export class UserService {
  createUser(name: string): string {
    return name.trim();
  }
}
"#,
        )
        .expect("failed writing service.ts");

        fs::write(
            repo_dir.join("src/lib.rs"),
            r#"
pub fn normalize(input: &str) -> String {
    input.trim().to_lowercase()
}
"#,
        )
        .expect("failed writing lib.rs");

        fs::write(
            repo_dir.join("tests/service.test.ts"),
            r#"
describe("service", () => {
  it("works", () => {
    expect(true).toBe(true);
  });
});
"#,
        )
        .expect("failed writing test file");

        init_database(&db_path, false, "ignored").expect("failed to init db");
        handle(&db_path, &repo_dir, "c1").expect("ingest should succeed");

        let conn = Connection::open(db_path).expect("failed to open db");

        let file_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artefacts WHERE commit_sha = 'c1' AND canonical_kind = 'file'",
                [],
                |row| row.get(0),
            )
            .expect("failed querying file count");
        assert_eq!(file_count, 2, "expected only src files, not test files");

        assert_has_symbol(&conn, "c1", "function", "validateEmail");
        assert_has_symbol(&conn, "c1", "class", "UserService");
        assert_has_symbol(&conn, "c1", "method", "UserService.createUser");
        assert_has_symbol(&conn, "c1", "interface", "User");
        assert_has_symbol(&conn, "c1", "type", "UserId");
        assert_has_symbol(&conn, "c1", "constant", "MAX_RETRIES");
        assert_has_symbol(&conn, "c1", "function", "normalize");

        let test_path_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artefacts WHERE commit_sha = 'c1' AND path LIKE 'tests/%'",
                [],
                |row| row.get(0),
            )
            .expect("failed querying test path rows");
        assert_eq!(
            test_path_rows, 0,
            "production ingestion should not ingest tests/* files"
        );
    }

    #[test]
    fn rerun_replaces_stale_production_artefacts_for_same_commit() {
        let temp = tempdir().expect("failed to create temp dir");
        let repo_dir = temp.path().join("repo");
        let db_path = temp.path().join("testlens.db");

        fs::create_dir_all(repo_dir.join("src")).expect("failed creating src dir");
        fs::write(
            repo_dir.join("src/service.ts"),
            r#"
export function oldName(): string {
  return "old";
}
"#,
        )
        .expect("failed writing initial service.ts");

        init_database(&db_path, false, "ignored").expect("failed to init db");
        handle(&db_path, &repo_dir, "c1").expect("first ingest should succeed");

        fs::write(
            repo_dir.join("src/service.ts"),
            r#"
export function newName(): string {
  return "new";
}
"#,
        )
        .expect("failed updating service.ts");

        handle(&db_path, &repo_dir, "c1").expect("second ingest should succeed");

        let conn = Connection::open(db_path).expect("failed to open db");
        assert_has_symbol(&conn, "c1", "function", "newName");

        let old_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artefacts WHERE commit_sha = 'c1' AND symbol_fqn = 'oldName'",
                [],
                |row| row.get(0),
            )
            .expect("failed querying old symbol count");
        assert_eq!(old_count, 0, "stale symbol oldName should be removed");
    }

    fn assert_has_symbol(conn: &Connection, commit: &str, kind: &str, symbol: &str) {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artefacts WHERE commit_sha = ?1 AND canonical_kind = ?2 AND symbol_fqn = ?3",
                params![commit, kind, symbol],
                |row| row.get(0),
            )
            .expect("failed querying symbol presence");
        assert!(count > 0, "expected symbol `{symbol}` of kind `{kind}`");
    }
}
