#![allow(dead_code)]

pub mod production_seed;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bitloops::utils::paths;
use rusqlite::{Connection, params};
use serde::Deserialize;
use tempfile::TempDir;

#[derive(Debug)]
pub struct Workspace {
    _temp_dir: TempDir,
    repo_dir: PathBuf,
    db_path: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ListedArtefact {
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
}

impl Workspace {
    pub fn new(name: &str) -> Self {
        let temp_dir = TempDir::new().expect("create temp dir");
        let repo_dir = temp_dir.path().join(name);
        fs::create_dir_all(&repo_dir).expect("create repo dir");
        init_git_repo(&repo_dir);

        let db_path = repo_dir
            .join(paths::BITLOOPS_RELATIONAL_STORE_DIR)
            .join(paths::RELATIONAL_DB_FILE_NAME);

        Self {
            _temp_dir: temp_dir,
            repo_dir,
            db_path,
        }
    }

    pub fn repo_dir(&self) -> &Path {
        &self.repo_dir
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn write_file(&self, relative_path: &str, content: &str) {
        let target = self.repo_dir.join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(target, content.trim_start()).expect("write fixture file");
    }

    pub fn path(&self, relative_path: &str) -> PathBuf {
        self.repo_dir.join(relative_path)
    }
}

pub fn run_bitloops_or_panic(workdir: &Path, args: &[&str]) -> String {
    let output = run_bitloops(workdir, args);
    if !output.status.success() {
        panic!(
            "bitloops command failed in {}: {:?}\nstdout:\n{}\nstderr:\n{}",
            workdir.display(),
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).expect("stdout should be valid utf-8")
}

pub fn seed_production_artefacts(workspace: &Workspace, commit_sha: &str) {
    production_seed::seed_production_artefacts(workspace, commit_sha)
}

pub fn discovered_languages(conn: &Connection, commit: &str) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(
            r#"
SELECT DISTINCT language
FROM (
  SELECT language FROM test_suites WHERE commit_sha = ?1
  UNION
  SELECT language FROM test_scenarios WHERE commit_sha = ?1
)
"#,
        )
        .expect("prepare language query");

    let rows = stmt
        .query_map(params![commit], |row| row.get::<_, String>(0))
        .expect("query languages");

    let mut languages = BTreeSet::new();
    for row in rows {
        languages.insert(row.expect("read language row"));
    }
    languages
}

pub fn load_symbol_fqn(conn: &Connection, commit_sha: &str, pattern: &str) -> String {
    conn.query_row(
        r#"
SELECT symbol_fqn
FROM artefacts_current
WHERE commit_sha = ?1
  AND symbol_fqn LIKE ?2
ORDER BY symbol_fqn ASC
LIMIT 1
"#,
        params![commit_sha, pattern],
        |row| row.get(0),
    )
    .unwrap_or_else(|_| panic!("expected symbol_fqn matching pattern {pattern}"))
}

pub fn load_test_scenario_signatures(conn: &Connection, commit_sha: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            r#"
SELECT signature
FROM test_scenarios
WHERE commit_sha = ?1
ORDER BY signature ASC
"#,
        )
        .expect("prepare scenario signature query");

    stmt.query_map(params![commit_sha], |row| row.get::<_, Option<String>>(0))
        .expect("query scenario signatures")
        .filter_map(|row| row.expect("read scenario signature"))
        .collect()
}

pub fn scenario_link_exists(
    conn: &Connection,
    commit: &str,
    scenario_name: &str,
    symbol_pattern: &str,
) -> bool {
    let count: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM test_links tl
JOIN test_scenarios t ON t.scenario_id = tl.test_scenario_id
JOIN artefacts p ON p.artefact_id = tl.production_artefact_id
WHERE tl.commit_sha = ?1
  AND t.commit_sha = ?1
  AND t.signature = ?2
  AND p.symbol_fqn LIKE ?3
"#,
            params![commit, scenario_name, symbol_pattern],
            |row| row.get(0),
        )
        .expect("query linkage edge");
    count > 0
}

pub fn write_typescript_static_link_fixture(workspace: &Workspace) {
    workspace.write_file(
        "src/repositories/UserRepository.ts",
        r#"
export class UserRepository {
  findById(id: number): string | null {
    return id > 0 ? `user-${id}` : null;
  }

  findByEmail(email: string): string | null {
    return email.includes("@") ? email : null;
  }
}
"#,
    );

    workspace.write_file(
        "tests/userRepository.test.ts",
        r#"
import { UserRepository } from "../src/repositories/UserRepository";

describe("ts repo", () => {
  it("finds by id", () => {
    const repo = new UserRepository();
    repo.findById(1);
  });

  it("calls email lookup only", () => {
    const repo = new UserRepository();
    repo.findByEmail("foo@bar.com");
  });
});
"#,
    );
}

pub fn write_rust_static_link_fixture(workspace: &Workspace) {
    workspace.write_file(
        "src/repositories/user_repository.rs",
        r#"
#[derive(Debug, Default)]
pub struct UserRepository;

impl UserRepository {
    pub fn new() -> Self {
        Self
    }

    pub fn find_by_id(&self, id: u32) -> Option<String> {
        (id > 0).then(|| format!("user-{}", id))
    }

    pub fn find_by_email(&self, email: &str) -> Option<String> {
        email.contains('@').then(|| email.to_string())
    }
}
"#,
    );

    workspace.write_file(
        "tests/rust_repo_test.rs",
        r#"
use crate::repositories::user_repository::UserRepository;

#[cfg(test)]
mod tests {
    use super::UserRepository;

    #[test]
    fn finds_by_id() {
        let repo = UserRepository::new();
        let _ = repo.find_by_id(1);
    }

    #[test]
    fn calls_email_lookup_only() {
        let repo = UserRepository::new();
        let _ = repo.find_by_email("foo@bar.com");
    }
}
"#,
    );
}

pub fn write_rust_parameterized_fixture(workspace: &Workspace) {
    workspace.write_file(
        "src/lib.rs",
        r#"
pub mod registry;
pub mod rules;
pub mod settings;
pub mod test_support;
"#,
    );

    workspace.write_file(
        "src/registry.rs",
        r#"
#[derive(Clone, Copy, Debug)]
pub enum Rule {
    StringDotFormatExtraPositionalArguments,
    StringDotFormatExtraNamedArguments,
}
"#,
    );

    workspace.write_file(
        "src/settings.rs",
        r#"
use crate::registry::Rule;

#[derive(Clone, Copy, Debug)]
pub struct LinterSettings;

impl LinterSettings {
    pub fn for_rule(rule: Rule) -> Self {
        let _ = rule;
        Self
    }
}
"#,
    );

    workspace.write_file(
        "src/test_support.rs",
        r#"
use std::path::Path;

use crate::settings::LinterSettings;

pub fn test_path(path: &Path, settings: &LinterSettings) -> bool {
    let _ = path;
    let _ = settings;
    true
}
"#,
    );

    workspace.write_file(
        "src/rules/mod.rs",
        r#"
pub mod pyflakes;
"#,
    );

    workspace.write_file(
        "src/rules/pyflakes/settings.rs",
        r#"
pub fn tag() -> &'static str {
    "pyflakes"
}
"#,
    );

    workspace.write_file(
        "src/rules/pyflakes/rules/mod.rs",
        r#"
pub mod strings;
"#,
    );

    workspace.write_file(
        "src/rules/pyflakes/rules/strings.rs",
        r#"
pub fn string_dot_format_extra_positional_arguments() -> &'static str {
    "F523"
}

pub fn string_dot_format_extra_named_arguments() -> &'static str {
    "F522"
}
"#,
    );

    workspace.write_file(
        "src/rules/pyflakes/mod.rs",
        r#"
pub mod rules;
pub mod settings;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::registry::Rule;
    use crate::rules::pyflakes;
    use crate::settings::LinterSettings;
    use crate::test_support::test_path;

    #[test_case(Rule::StringDotFormatExtraPositionalArguments, Path::new("F523.py"))]
    #[test_case(Rule::StringDotFormatExtraNamedArguments, Path::new("F522.py"))]
    fn rules(rule_code: Rule, path: &Path) {
        let _ = test_path(
            Path::new("pyflakes").join(path).as_path(),
            &LinterSettings::for_rule(rule_code),
        );
        let _ = pyflakes::settings::tag();
    }
}
"#,
    );
}

pub fn write_rust_additional_declarations_fixture(workspace: &Workspace) {
    write_rust_parameterized_fixture(workspace);

    workspace.write_file(
        "src/lib.rs",
        r#"
pub mod registry;
pub mod rules;
pub mod settings;
pub mod test_support;
pub mod types;
pub mod wasm_api;
"#,
    );

    workspace.write_file(
        "src/wasm_api.rs",
        r#"
pub fn render_message() -> &'static str {
    "ok"
}
"#,
    );

    workspace.write_file(
        "tests/api.rs",
        r#"
use wasm_bindgen_test::wasm_bindgen_test;

use crate::wasm_api::render_message;

#[wasm_bindgen_test]
fn empty_config() {
    let _ = render_message();
}
"#,
    );

    workspace.write_file(
        "src/types.rs",
        r#"
pub struct Type;

impl Type {
    pub fn is_equivalent_to(&self) -> bool {
        true
    }

    pub fn is_subtype_of(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod property_tests;
"#,
    );

    workspace.write_file(
        "src/types/property_tests.rs",
        r#"
use super::Type;

macro_rules! type_property_test {
    ($test_name:ident, $property:expr) => {
        #[quickcheck_macros::quickcheck]
        #[ignore]
        fn $test_name(t: Type) -> bool {
            $property
        }
    };
}

mod stable {
    use super::Type;

    type_property_test!(equivalent_to_is_reflexive, t.is_equivalent_to());
    type_property_test!(subtype_of_is_reflexive, t.is_subtype_of());
}
"#,
    );
}

pub fn write_rust_hybrid_fixture(workspace: &Workspace) {
    workspace.write_file(
        "Cargo.toml",
        r#"
[package]
name = "rust_detection_fixture"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
proptest = "1"
rstest = "0.24"
rstest_reuse = "0.7"
"#,
    );

    workspace.write_file(
        "src/lib.rs",
        r#"
pub mod docs;
pub mod numbers;

#[cfg(test)]
mod hybrid_tests;
"#,
    );

    workspace.write_file(
        "src/numbers.rs",
        r#"
pub fn double(value: u32) -> u32 {
    value * 2
}

pub fn triple(value: u32) -> u32 {
    value * 3
}
"#,
    );

    workspace.write_file(
        "src/docs.rs",
        r#"
/// ```rust
/// use rust_detection_fixture::docs::documented_increment;
///
/// assert_eq!(documented_increment(1), 2);
/// ```
pub fn documented_increment(value: u32) -> u32 {
    value + 1
}
"#,
    );

    workspace.write_file(
        "src/hybrid_tests.rs",
        r#"
use std::path::PathBuf;

use proptest::prelude::*;
use rstest::rstest;
use rstest_reuse::{self, *};

use crate::docs::documented_increment;
use crate::numbers::{double, triple};

#[rstest]
#[case(2, 4)]
#[case(3, 6)]
fn doubles_case_values(#[case] input: u32, #[case] expected: u32) {
    assert_eq!(double(input), expected);
}

#[rstest]
fn doubles_values(#[values(1, 2)] input: u32) {
    assert!(double(input) > 0);
}

#[template]
#[rstest]
#[case(2, 6)]
#[case(3, 9)]
fn triple_cases(#[case] input: u32, #[case] expected: u32) {}

#[apply(triple_cases)]
fn triples_from_template(input: u32, expected: u32) {
    assert_eq!(triple(input), expected);
}

#[rstest]
fn files_fallback(#[files("fixtures/*.txt")] path: PathBuf) {
    let _ = path;
}

proptest! {
    #[test]
    fn double_is_even(input in 0u32..8) {
        let result = double(input);
        prop_assert_eq!(result % 2, 0);
    }
}

#[test]
fn documented_increment_is_callable() {
    assert_eq!(documented_increment(1), 2);
}
"#,
    );

    workspace.write_file("fixtures/sample.txt", "fixture\n");
}

pub fn write_rust_coverage_fixture(workspace: &Workspace) {
    workspace.write_file(
        "src/lib.rs",
        r#"
pub struct UserRepository;

impl UserRepository {
    pub fn find_by_id(&self, id: u32) -> Option<String> {
        (id > 0).then(|| format!("user-{}", id))
    }

    pub fn find_by_email(&self, email: &str) -> Option<String> {
        email.contains('@').then(|| email.to_string())
    }
}
"#,
    );

    workspace.write_file(
        "tests/rust_repo_test.rs",
        r#"
use crate::UserRepository;

#[cfg(test)]
mod tests {
    use super::UserRepository;

    #[test]
    fn finds_by_id() {
        let repo = UserRepository;
        let _ = repo.find_by_id(1);
    }
}
"#,
    );
}

/// LCOV covering both find_by_id and find_by_email with line + branch data.
pub fn write_valid_lcov_fixture(workspace: &Workspace) {
    workspace.write_file(
        "coverage.lcov",
        r#"
SF:src/lib.rs
DA:4,1
DA:5,1
DA:6,1
DA:8,0
DA:9,0
DA:10,0
BRDA:5,0,0,1
BRDA:5,0,1,0
end_of_record
"#,
    );
}

/// LCOV with line data only (no BRDA lines).
pub fn write_line_only_lcov_fixture(workspace: &Workspace) {
    workspace.write_file(
        "coverage.lcov",
        r#"
SF:src/lib.rs
DA:4,1
DA:5,1
DA:6,1
end_of_record
"#,
    );
}

/// LCOV with one valid file and one unmappable path.
pub fn write_unmappable_lcov_fixture(workspace: &Workspace) {
    workspace.write_file(
        "coverage.lcov",
        r#"
SF:src/lib.rs
DA:4,1
DA:5,1
end_of_record
SF:src/does_not_exist.rs
DA:1,1
DA:2,1
end_of_record
"#,
    );
}

/// LCOV with malformed DA lines mixed with valid ones.
pub fn write_malformed_lcov_fixture(workspace: &Workspace) {
    workspace.write_file(
        "coverage.lcov",
        r#"
SF:src/lib.rs
DA:4,1
DA:bad_line
DA:5,1
DA:,
BRDA:5,0,0,1
end_of_record
"#,
    );
}

fn init_git_repo(repo_dir: &Path) {
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(repo_dir)
        .status()
        .expect("run git init");
    assert!(status.success(), "git init should succeed");
}

fn run_bitloops(workdir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bitloops"))
        .current_dir(workdir)
        .args(args)
        .output()
        .expect("execute bitloops command")
}
