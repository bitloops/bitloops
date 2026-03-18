use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

#[derive(Debug)]
pub struct BddWorkspace {
    _temp_dir: TempDir,
    repo_dir: PathBuf,
    db_path: PathBuf,
}

impl BddWorkspace {
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("failed to create temp dir for BDD workspace");
        let repo_dir = temp_dir.path().join("fixture-repo");
        let db_path = temp_dir.path().join("testlens.db");

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
            fs::create_dir_all(parent).expect("failed to create parent directory");
        }
        fs::write(target, content.trim_start()).expect("failed to write fixture file");
    }
}

pub fn write_cli_1345_base_fixture(workspace: &BddWorkspace) {
    workspace.write_file(
        "src/service.ts",
        r#"
export function doThing(input: string): string {
  return input.trim().toUpperCase();
}
"#,
    );
    workspace.write_file(
        "src/lib.rs",
        r#"
pub fn normalize(input: &str) -> String {
    input.trim().to_lowercase()
}
"#,
    );
    workspace.write_file(
        "tests/base.test.ts",
        r#"
import { doThing } from "../src/service";

describe("service", () => {
  it("normalizes input", () => {
    expect(doThing(" hello ")).toBe("HELLO");
  });
});
"#,
    );
    workspace.write_file(
        "tests/rust_unit.rs",
        r#"
#[cfg(test)]
mod tests {
    #[test]
    fn sample() {
        assert_eq!(2 + 2, 4);
    }
}
"#,
    );
}

pub fn write_cli_1345_c1_extra_test(workspace: &BddWorkspace) {
    workspace.write_file(
        "tests/new.test.ts",
        r#"
import { doThing } from "../src/service";

describe("service c1", () => {
  it("adds another scenario", () => {
    expect(doThing("world")).toBe("WORLD");
  });
});
"#,
    );
}

pub fn write_cli_1346_base_fixture(workspace: &BddWorkspace) {
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

pub fn write_cli_1346_c1_updated_tests(workspace: &BddWorkspace) {
    workspace.write_file(
        "tests/userRepository.test.ts",
        r#"
import { UserRepository } from "../src/repositories/UserRepository";

describe("ts repo", () => {
  it("finds by id", () => {
    const repo = new UserRepository();
    repo.findById(1);
    repo.findByEmail("c1@bar.com");
  });

  it("calls email lookup only", () => {
    const repo = new UserRepository();
    repo.findByEmail("foo@bar.com");
  });
});
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
        let _ = repo.find_by_email("c1@bar.com");
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

pub fn write_cli_1368_base_fixture(workspace: &BddWorkspace) {
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

pub fn write_cli_1369_base_fixture(workspace: &BddWorkspace) {
    write_cli_1368_base_fixture(workspace);

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

pub fn write_cli_1381_base_fixture(workspace: &BddWorkspace) {
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
