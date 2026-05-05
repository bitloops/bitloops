use std::collections::HashSet;
use std::fs;
use std::sync::Arc;

use crate::adapters::languages::python::test_support::{
    collect_python_suites, python_test_support, resolve_python_import_to_repo_path,
};
use crate::adapters::languages::rust::test_support::RustTestMappingHelper;
use crate::adapters::languages::rust::test_support::enumeration::parse_enumerated_doctests;
use crate::adapters::languages::rust::test_support::imports::{
    collect_rust_import_paths_for, collect_rust_scoped_call_import_paths_for,
    rust_test_context_source_paths,
};
use crate::adapters::languages::rust::test_support::macros::extract_rust_macro_invocation_body;
use crate::adapters::languages::rust::test_support::rust_source_contains_doctest_markers;
use crate::adapters::languages::rust::test_support::scenarios::collect_rust_suites;
use crate::adapters::languages::ts_js::test_support::{
    collect_typescript_suites, extract_import_specifier, resolve_import_to_repo_path, unquote,
};

use tempfile::TempDir;
use tree_sitter::Parser;
use tree_sitter_python::LANGUAGE as LANGUAGE_PYTHON;
use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;
use tree_sitter_typescript::LANGUAGE_TYPESCRIPT;

use super::execute;
use super::linker::{imported_path_matches_production_path, symbol_match_key};
use super::model::{ReferenceCandidate, ScenarioDiscoverySource};
use crate::host::capability_host::gateways::LanguageServicesGateway;
use crate::host::language_adapter::{DiscoveredTestFile, EnumerationResult, LanguageTestSupport};
use crate::models::ProductionArtefact;

#[test]
fn extracts_import_specifier_from_statement() {
    let statement = r#"import { UserService } from "../src/services/UserService";"#;
    let value = extract_import_specifier(statement).expect("should extract import specifier");
    assert_eq!(value, "../src/services/UserService");
}

#[test]
fn resolves_relative_import_to_repo_path() {
    let resolved = resolve_import_to_repo_path(
        "tests/e2e/userFlow.test.ts",
        "../../src/services/UserService",
    )
    .expect("should resolve relative import");

    assert_eq!(resolved, "src/services/UserService.ts");
}

#[test]
fn unquote_handles_string_literals() {
    assert_eq!(unquote("'hello'"), Some("hello".to_string()));
    assert_eq!(unquote("\"hello\""), Some("hello".to_string()));
    assert_eq!(unquote("`hello`"), Some("hello".to_string()));
}

#[test]
fn resolves_python_import_to_repo_path() {
    let resolved = resolve_python_import_to_repo_path("tests/test_api.py", ".helpers")
        .expect("should resolve relative python import");
    assert_eq!(resolved, "tests/helpers.py");

    let absolute = resolve_python_import_to_repo_path("tests/test_api.py", "app.services.user")
        .expect("should resolve absolute python import");
    assert_eq!(absolute, "app/services/user.py");
}

#[test]
fn python_suites_detect_pytest_functions_and_unittest_methods() {
    let source = r#"
from app.services.user import create_user

def test_creates_user():
    create_user()

class UserFlowTests:
    def test_updates_user(self):
        client.execute()
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_PYTHON.into())
        .expect("failed setting python parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing python source");

    let suites = collect_python_suites(tree.root_node(), source.as_bytes(), "tests/test_user.py");
    assert_eq!(suites.len(), 2, "expected module and class suites");

    let module_suite = suites
        .iter()
        .find(|suite| suite.name == "test_user")
        .expect("missing module suite");
    assert_eq!(module_suite.scenarios.len(), 1);
    assert_eq!(module_suite.scenarios[0].name, "test_creates_user");
    assert!(
        module_suite.scenarios[0]
            .reference_candidates
            .contains(&ReferenceCandidate::SymbolName("create_user".to_string()))
    );

    let class_suite = suites
        .iter()
        .find(|suite| suite.name == "UserFlowTests")
        .expect("missing class suite");
    assert_eq!(class_suite.scenarios.len(), 1);
    assert_eq!(class_suite.scenarios[0].name, "test_updates_user");
    assert!(
        class_suite.scenarios[0]
            .reference_candidates
            .contains(&ReferenceCandidate::SymbolName("execute".to_string()))
    );
}

#[test]
fn rust_suites_detects_test_and_tokio_test_functions() {
    let source = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn sample() {
        assert_eq!(2 + 2, 4);
    }

    #[tokio::test]
    async fn async_sample() {
        helper::run().await;
        client.execute();
    }

    fn helper_only() {
        helper::run();
    }
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "tests/rust_unit.rs");

    assert_eq!(suites.len(), 1, "expected one rust suite");
    assert_eq!(suites[0].name, "tests");

    let scenario_names: Vec<&str> = suites[0]
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(scenario_names, vec!["sample", "async_sample"]);

    let async_scenario = suites[0]
        .scenarios
        .iter()
        .find(|scenario| scenario.name == "async_sample")
        .expect("missing async_sample scenario");
    assert!(
        async_scenario
            .reference_candidates
            .contains(&ReferenceCandidate::SymbolName("run".to_string())),
        "expected rust call-site extraction to include helper::run"
    );
    assert!(
        async_scenario
            .reference_candidates
            .contains(&ReferenceCandidate::SymbolName("execute".to_string())),
        "expected rust call-site extraction to include method call symbols"
    );
}

#[test]
fn rust_use_declarations_map_to_source_paths() {
    let source = r#"
use crate::repositories::user_repository::UserRepository;
use testlens_fixture_rust::services::{user_service::UserService, auth_service::AuthService};
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let import_paths =
        collect_rust_import_paths_for(tree.root_node(), source.as_bytes(), "tests/rust.rs");
    assert!(
        import_paths.contains("src/repositories/user_repository.rs"),
        "expected repository source path from use declaration"
    );
    assert!(
        import_paths.contains("src/services/user_service.rs"),
        "expected user service path from brace use declaration"
    );
    assert!(
        import_paths.contains("src/services/auth_service.rs"),
        "expected auth service path from brace use declaration"
    );
}

#[test]
fn rust_scoped_call_paths_map_to_source_paths() {
    let source = r#"
#[test]
fn sample() {
    crate::services::auth_service::AuthService::hash_password("x");
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let import_paths = collect_rust_scoped_call_import_paths_for(
        tree.root_node(),
        source.as_bytes(),
        "tests/rust.rs",
    );
    assert!(
        import_paths.contains("src/services/auth_service.rs"),
        "expected scoped call path to resolve to source module path"
    );
}

#[test]
fn rust_workspace_imports_map_to_crate_source_paths() {
    let source = r#"
use red_knot_workspace::db::RootDatabase;
use ruff::commands::version::version;

#[test]
fn sample() {
    let _ = red_knot_workspace::db::RootDatabase::new();
    ruff::commands::version::version();
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let import_paths = collect_rust_import_paths_for(
        tree.root_node(),
        source.as_bytes(),
        "crates/ruff/tests/version.rs",
    );
    assert!(
        import_paths.contains("crates/red_knot_workspace/src/db.rs"),
        "expected workspace crate import to resolve to crate source file"
    );
    assert!(
        import_paths.contains("crates/ruff/src/commands/version.rs"),
        "expected same-workspace crate import to resolve to local crate source file"
    );

    let scoped_call_paths = collect_rust_scoped_call_import_paths_for(
        tree.root_node(),
        source.as_bytes(),
        "crates/ruff/tests/version.rs",
    );
    assert!(
        scoped_call_paths.contains("crates/red_knot_workspace/src/db.rs"),
        "expected scoped call to workspace crate type to resolve to crate source file"
    );
    assert!(
        scoped_call_paths.contains("crates/ruff/src/commands/version.rs"),
        "expected scoped call to local workspace crate function to resolve to crate source file"
    );
}

#[test]
fn rust_suites_expand_test_case_attributes_into_parameterized_scenarios() {
    let source = r#"
#[cfg(test)]
mod tests {
    use std::path::Path;
    use test_case::test_case;

    #[test_case(Rule::StringDotFormatExtraPositionalArguments, Path::new("F523.py"))]
    #[test_case(Rule::StringDotFormatExtraNamedArguments, Path::new("F522.py"))]
    fn rules(rule_code: Rule, path: &Path) {
        test_path(path, rule_code);
    }
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "src/rules/pyflakes/mod.rs");
    assert_eq!(suites.len(), 1, "expected one inline rust test suite");

    let scenario_names: Vec<&str> = suites[0]
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(
        scenario_names,
        vec![
            "rules[StringDotFormatExtraPositionalArguments, F523.py]",
            "rules[StringDotFormatExtraNamedArguments, F522.py]",
        ]
    );
    assert!(
        suites[0].scenarios[0]
            .reference_candidates
            .contains(&ReferenceCandidate::ScopedSymbol(
                "StringDotFormatExtraPositionalArguments".to_string()
            )),
        "expected parameterized scenario to carry its rule variant symbol"
    );
}

#[test]
fn rust_suites_include_additional_test_case_string_arguments_in_scenario_names() {
    let source = r#"
#[cfg(test)]
mod tests {
    use std::path::Path;
    use test_case::test_case;

    #[test_case(
        Rule::PytestFixtureIncorrectParenthesesStyle,
        Path::new("PT001.py"),
        Settings::default(),
        "PT001_default"
    )]
    #[test_case(
        Rule::PytestFixtureIncorrectParenthesesStyle,
        Path::new("PT001.py"),
        Settings { fixture_parentheses: true, ..Settings::default() },
        "PT001_parentheses"
    )]
    fn test_pytest_style(rule_code: Rule, path: &Path, plugin_settings: Settings, name: &str) {}
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(
        tree.root_node(),
        source,
        "crates/ruff_linter/src/rules/flake8_pytest_style/mod.rs",
    );
    assert_eq!(suites.len(), 1, "expected one inline rust test suite");

    let scenario_names: Vec<&str> = suites[0]
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(
        scenario_names,
        vec![
            "test_pytest_style[PytestFixtureIncorrectParenthesesStyle, PT001.py, PT001_default]",
            "test_pytest_style[PytestFixtureIncorrectParenthesesStyle, PT001.py, PT001_parentheses]",
        ]
    );
}

#[test]
fn rust_suites_use_generic_test_case_labels_and_arguments_to_avoid_duplicate_names() {
    let source = r#"
#[cfg(test)]
mod tests {
    use test_case::test_case;

    #[test_case(Type::Any)]
    #[test_case(Type::Unknown)]
    #[test_case(todo_type!())]
    fn build_intersection_t_and_negative_t_does_not_simplify(ty: Type) {}

    #[test_case("\"%s\"", "\"{}\""; "simple string")]
    #[test_case("\"%%%s\"", "\"%{}\""; "three percents")]
    fn test_percent_to_format(sample: &str, expected: &str) {}
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "tests/parameterized.rs");
    assert_eq!(suites.len(), 1, "expected one inline rust test suite");

    let scenario_names: Vec<&str> = suites[0]
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(
        scenario_names,
        vec![
            "build_intersection_t_and_negative_t_does_not_simplify[Type::Any]",
            "build_intersection_t_and_negative_t_does_not_simplify[Type::Unknown]",
            "build_intersection_t_and_negative_t_does_not_simplify[todo_type!()]",
            "test_percent_to_format[simple string]",
            "test_percent_to_format[three percents]",
        ]
    );
}

#[test]
fn rust_suites_detect_wasm_bindgen_test_functions() {
    let source = r#"
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn empty_config() {
    render_message();
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "tests/api.rs");
    assert_eq!(suites.len(), 1, "expected one wasm suite");
    assert_eq!(suites[0].name, "api");
    assert_eq!(suites[0].scenarios.len(), 1);
    assert_eq!(suites[0].scenarios[0].name, "empty_config");
    assert!(
        suites[0].scenarios[0]
            .reference_candidates
            .contains(&ReferenceCandidate::SymbolName(
                "render_message".to_string()
            )),
        "expected wasm test call-site extraction to include render_message"
    );
}

#[test]
fn rust_suites_expand_macro_generated_quickcheck_tests() {
    let source = r#"
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
    type_property_test!(equivalent_to_is_reflexive, t.is_equivalent_to());
    type_property_test!(subtype_of_is_reflexive, t.is_subtype_of());
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "src/types/property_tests.rs");
    assert_eq!(suites.len(), 1, "expected one property-test suite");
    assert_eq!(suites[0].name, "stable");

    let scenario_names: Vec<&str> = suites[0]
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(
        scenario_names,
        vec!["equivalent_to_is_reflexive", "subtype_of_is_reflexive"]
    );
    assert!(
        suites[0].scenarios[0]
            .reference_candidates
            .contains(&ReferenceCandidate::SymbolName(
                "is_equivalent_to".to_string()
            )),
        "expected quickcheck macro invocation to surface method-call symbols"
    );
}

#[test]
fn rust_suites_extract_matched_macro_case_names() {
    let source = r#"
macro_rules! matched {
    ($kind:ident, $name:ident, $glob:expr, $expected:expr) => {
        #[test]
        fn $name() {
            assert_eq!(evaluate($glob), $expected);
        }
    };
}

#[cfg(test)]
mod tests {
    matched!(not, matchnot1, "foo", true);
    matched!(not, matchnot2, "bar", false);
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "crates/ignore/src/types.rs");
    assert_eq!(suites.len(), 1, "expected one matched! suite");
    assert_eq!(suites[0].name, "tests");

    let scenario_names: Vec<&str> = suites[0]
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(scenario_names, vec!["matchnot1", "matchnot2"]);
}

#[test]
fn rust_suites_canonicalize_cfg_mirrored_macro_generated_cases() {
    let source = r#"
macro_rules! slash_case {
    ($name:ident, $glob:expr) => {
        #[test]
        fn $name() {
            assert!(compile($glob));
        }
    };
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    slash_case!(matchslash2, "foo/bar");
    #[cfg(not(unix))]
    slash_case!(matchslash2, "foo\\bar");

    #[cfg(unix)]
    slash_case!(normal3, "foo/bar");
    #[cfg(not(unix))]
    slash_case!(normal3, "foo\\bar");
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "crates/globset/src/glob.rs");
    assert_eq!(suites.len(), 1, "expected one slash-case suite");
    assert_eq!(suites[0].name, "tests");

    let scenario_names: Vec<&str> = suites[0]
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();
    assert_eq!(scenario_names, vec!["matchslash2", "normal3"]);
}

#[test]
fn rust_suites_expand_rstest_cases_values_and_templates() {
    let source = r#"
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use rstest::{rstest, template, apply};

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
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "src/lib.rs");
    let scenario_names: Vec<&str> = suites[0]
        .scenarios
        .iter()
        .map(|scenario| scenario.name.as_str())
        .collect();

    assert_eq!(
        scenario_names,
        vec![
            "doubles_case_values[2, 4]",
            "doubles_case_values[3, 6]",
            "doubles_values[input=1]",
            "doubles_values[input=2]",
            "triples_from_template[2, 6]",
            "triples_from_template[3, 9]",
            "files_fallback",
        ]
    );
}

#[test]
fn rust_suites_extract_proptest_cases_from_macro_body() {
    let source = r#"
#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn double_is_even(input in 0u32..8) {
            let result = double(input);
            prop_assert_eq!(result % 2, 0);
        }
    }
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "src/property_tests.rs");
    assert_eq!(suites.len(), 1);
    assert_eq!(suites[0].name, "property_tests");
    assert_eq!(suites[0].scenarios.len(), 1);
    assert_eq!(suites[0].scenarios[0].name, "double_is_even");
    assert!(
        suites[0].scenarios[0]
            .reference_candidates
            .contains(&ReferenceCandidate::SymbolName("double".to_string())),
        "expected proptest case body to surface double()"
    );
}

#[test]
fn rust_suites_materialize_doctest_scenarios() {
    let source = r#"
/// ```rust
/// assert_eq!(documented_increment(1), 2);
/// ```
pub fn documented_increment(value: u32) -> u32 {
    value + 1
}
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_RUST.into())
        .expect("failed setting rust parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing rust source");

    let suites = collect_rust_suites(tree.root_node(), source, "src/docs.rs");
    assert_eq!(suites.len(), 1);
    assert_eq!(suites[0].name, "docs::doctests");
    assert_eq!(suites[0].scenarios.len(), 1);
    assert_eq!(
        suites[0].scenarios[0].discovery_source,
        ScenarioDiscoverySource::Doctest
    );
    assert!(
        suites[0].scenarios[0]
            .reference_candidates
            .contains(&ReferenceCandidate::ExplicitTarget {
                path: "src/docs.rs".to_string(),
                start_line: 5,
            }),
        "expected doctest to point at the documented item"
    );
}

#[test]
fn parses_enumerated_doctest_output() {
    let scenarios = parse_enumerated_doctests(
        "crates/sample/src/lib.rs - sample::documented_increment (line 12): test",
    );

    assert_eq!(scenarios.len(), 1);
    assert_eq!(scenarios[0].relative_path, "crates/sample/src/lib.rs");
    assert_eq!(
        scenarios[0].scenario_name,
        "sample::documented_increment[doctest:12]"
    );
    assert!(
        scenarios[0]
            .reference_candidates
            .contains(&ReferenceCandidate::ExplicitTarget {
                path: "crates/sample/src/lib.rs".to_string(),
                start_line: 12,
            }),
        "expected parsed doctest line target"
    );
}

#[test]
fn enumerated_doctests_preserve_line_identity_for_duplicate_items() {
    let scenarios = parse_enumerated_doctests(
        r#"crates/sample/src/lib.rs - sample::documented_increment (line 12): test
crates/sample/src/lib.rs - sample::documented_increment (line 24): test"#,
    );

    assert_eq!(scenarios.len(), 2);
    assert_eq!(
        scenarios[0].scenario_name,
        "sample::documented_increment[doctest:12]"
    );
    assert_eq!(
        scenarios[1].scenario_name,
        "sample::documented_increment[doctest:24]"
    );
}

#[test]
fn doctest_prefilter_requires_doc_fences_not_plain_doc_comments() {
    assert!(rust_source_contains_doctest_markers(
        r#"
/// ```rust
/// assert_eq!(value(), 1);
/// ```
pub fn value() -> u32 {
    1
}
"#
    ));
    assert!(!rust_source_contains_doctest_markers(
        r#"
/** Plain docs without a fenced block. */
pub fn documented() {}
"#
    ));
}

#[test]
fn rust_test_context_paths_include_parent_module_for_property_test_files() {
    let context_paths = rust_test_context_source_paths(
        "crates/red_knot_python_semantic/src/types/property_tests.rs",
    );

    assert!(
        context_paths.contains("crates/red_knot_python_semantic/src/types.rs"),
        "expected property-tests file to include parent module source path"
    );
}

#[test]
fn extracts_rust_macro_invocation_body() {
    let raw = r#"type_property_test!(equivalent_to_is_reflexive, t.is_equivalent_to(db, t));"#;
    let body = extract_rust_macro_invocation_body(raw).expect("expected macro invocation body");
    assert_eq!(
        body,
        "equivalent_to_is_reflexive, t.is_equivalent_to(db, t)"
    );
}

#[test]
fn rust_module_root_import_matches_nested_rule_paths() {
    assert!(
        imported_path_matches_production_path(
            "src/rules/pyflakes/mod.rs",
            "src/rules/pyflakes/rules/strings.rs"
        ),
        "expected module-root import to match nested rule module"
    );
}

#[test]
fn symbol_match_key_normalizes_camel_case_rule_variants() {
    assert_eq!(
        symbol_match_key("StringDotFormatExtraPositionalArguments"),
        "string_dot_format_extra_positional_arguments"
    );
    assert_eq!(
        symbol_match_key("Rule::StringDotFormatExtraPositionalArguments"),
        "string_dot_format_extra_positional_arguments"
    );
}

#[test]
fn typescript_nested_describe_does_not_duplicate_inner_tests_into_outer_suite() {
    let source = r#"
describe("outer", () => {
  describe("inner", () => {
    it("inner test", () => {
      expect(1).toBe(1);
    });
  });
});
"#;

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_TYPESCRIPT.into())
        .expect("failed setting TypeScript parser language");

    let tree = parser
        .parse(source, None)
        .expect("failed parsing TypeScript source");

    let suites = collect_typescript_suites(tree.root_node(), source.as_bytes());
    assert_eq!(suites.len(), 2, "expected nested describe suites");

    let outer = suites
        .iter()
        .find(|suite| suite.name == "outer")
        .expect("missing outer suite");
    assert!(
        outer.scenarios.is_empty(),
        "outer suite should not duplicate inner suite scenarios"
    );

    let inner = suites
        .iter()
        .find(|suite| suite.name == "inner")
        .expect("missing inner suite");
    assert_eq!(
        inner.scenarios.len(),
        1,
        "expected exactly one inner scenario"
    );
    assert_eq!(inner.scenarios[0].name, "inner test");
}

#[test]
fn rust_mapping_canonicalizes_duplicate_macro_generated_scenarios() {
    let temp = TempDir::new().expect("failed creating temp repo");
    let repo_root = temp.path();

    fs::create_dir_all(repo_root.join("tests")).expect("failed creating tests directory");
    fs::create_dir_all(repo_root.join("src")).expect("failed creating src directory");
    fs::write(
        repo_root.join("tests/macro_cases.rs"),
        r#"
macro_rules! matched {
    ($kind:ident, $name:ident, $glob:expr, $expected:expr) => {
        #[test]
        fn $name() {
            assert_eq!(evaluate($glob), $expected);
        }
    };
}

macro_rules! slash_case {
    ($name:ident, $glob:expr) => {
        #[test]
        fn $name() {
            assert!(compile($glob));
        }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn direct_case() {
        assert!(compile("baz"));
    }

    matched!(not, matchnot1, "foo", true);
    matched!(not, matchnot2, "bar", false);

    #[cfg(unix)]
    slash_case!(matchslash2, "foo/bar");
    #[cfg(not(unix))]
    slash_case!(matchslash2, "foo\\bar");

    #[cfg(unix)]
    slash_case!(normal3, "foo/bar");
    #[cfg(not(unix))]
    slash_case!(normal3, "foo\\bar");
}
"#,
    )
    .expect("failed writing rust test fixture");
    fs::write(
        repo_root.join("src/lib.rs"),
        r#"
pub fn compile(value: &str) -> bool {
    !value.is_empty()
}

pub fn evaluate(value: &str) -> bool {
    !value.is_empty()
}
"#,
    )
    .expect("failed writing rust source fixture");

    let gateway = SourceOnlyLanguageServicesGateway {
        support: Arc::new(SourceOnlyRustSupport),
    };
    let output = execute(
        "repo-1",
        repo_root,
        "commit-1",
        &[
            ProductionArtefact {
                artefact_id: "prod-compile".to_string(),
                symbol_id: "prod-compile-symbol".to_string(),
                symbol_fqn: "compile".to_string(),
                path: "src/lib.rs".to_string(),
                start_line: 1,
            },
            ProductionArtefact {
                artefact_id: "prod-evaluate".to_string(),
                symbol_id: "prod-evaluate-symbol".to_string(),
                symbol_fqn: "evaluate".to_string(),
                path: "src/lib.rs".to_string(),
                start_line: 5,
            },
        ],
        &gateway,
    )
    .expect("mapping execution should succeed");

    let mut scenario_names: Vec<&str> = output
        .test_artefacts
        .iter()
        .filter(|artefact| artefact.canonical_kind == "test_scenario")
        .map(|artefact| artefact.name.as_str())
        .collect();
    scenario_names.sort_unstable();
    assert_eq!(
        scenario_names,
        vec![
            "direct_case",
            "matchnot1",
            "matchnot2",
            "matchslash2",
            "normal3"
        ]
    );

    let scenario_artefacts: Vec<_> = output
        .test_artefacts
        .iter()
        .filter(|artefact| artefact.canonical_kind == "test_scenario")
        .collect();
    let artefact_ids: HashSet<&str> = scenario_artefacts
        .iter()
        .map(|artefact| artefact.artefact_id.as_str())
        .collect();
    assert_eq!(artefact_ids.len(), scenario_artefacts.len());

    let symbol_ids: HashSet<&str> = scenario_artefacts
        .iter()
        .map(|artefact| artefact.symbol_id.as_str())
        .collect();
    assert_eq!(symbol_ids.len(), scenario_artefacts.len());

    assert!(
        output
            .test_edges
            .iter()
            .any(|edge| edge.to_symbol_id.as_deref() == Some("prod-compile-symbol")),
        "expected at least one static link to the compile production artefact"
    );
}

#[test]
fn execute_records_issue_for_non_utf8_python_test_file_and_continues() {
    let temp = TempDir::new().expect("failed creating temp repo");
    let repo_root = temp.path();
    fs::create_dir_all(repo_root.join("tests")).expect("failed creating tests directory");
    fs::write(
        repo_root.join("tests/test_good.py"),
        "def test_good():\n    assert True\n",
    )
    .expect("failed writing UTF-8 python fixture");
    fs::write(
        repo_root.join("tests/test_big5.py"),
        [
            0x23, 0x20, 0x2d, 0x2a, 0x2d, 0x20, 0x63, 0x6f, 0x64, 0x69, 0x6e, 0x67, 0x3a, 0x20,
            0x62, 0x69, 0x67, 0x35, 0x20, 0x2d, 0x2a, 0x2d, 0x0a, 0x23, 0x20, 0xa4, 0x40, 0xa8,
            0xc7, 0xa4, 0xa4, 0xa4, 0xe5, 0xa6, 0x72, 0x0a, 0x64, 0x65, 0x66, 0x20, 0x74, 0x65,
            0x73, 0x74, 0x5f, 0x62, 0x69, 0x67, 0x35, 0x28, 0x29, 0x3a, 0x0a, 0x20, 0x20, 0x20,
            0x20, 0x61, 0x73, 0x73, 0x65, 0x72, 0x74, 0x20, 0x54, 0x72, 0x75, 0x65, 0x0a,
        ],
    )
    .expect("failed writing Big5 python fixture");

    let gateway = SourceOnlyLanguageServicesGateway {
        support: python_test_support(),
    };
    let output = execute("repo-1", repo_root, "commit-1", &[], &gateway)
        .expect("mapping execution should degrade non-UTF-8 files");

    assert!(
        output
            .test_artefacts
            .iter()
            .any(|artefact| artefact.path == "tests/test_good.py"),
        "UTF-8 test file should still be discovered"
    );
    assert_eq!(output.issues.len(), 1);
    assert_eq!(output.issues[0].path, "tests/test_big5.py");
    assert!(
        output.issues[0].message.contains("valid UTF-8"),
        "issue should preserve the UTF-8 decoding failure: {}",
        output.issues[0].message
    );
}

struct SourceOnlyLanguageServicesGateway {
    support: Arc<dyn LanguageTestSupport>,
}

impl LanguageServicesGateway for SourceOnlyLanguageServicesGateway {
    fn test_supports(&self) -> Vec<Arc<dyn LanguageTestSupport>> {
        vec![self.support.clone()]
    }
}

struct SourceOnlyRustSupport;

impl LanguageTestSupport for SourceOnlyRustSupport {
    fn language_id(&self) -> &'static str {
        "rust"
    }

    fn priority(&self) -> u8 {
        0
    }

    fn supports_path(&self, absolute_path: &std::path::Path, relative_path: &str) -> bool {
        RustTestMappingHelper::supports_path(absolute_path, relative_path)
    }

    fn discover_tests(
        &self,
        absolute_path: &std::path::Path,
        relative_path: &str,
    ) -> anyhow::Result<DiscoveredTestFile> {
        let mut helper = RustTestMappingHelper::new()?;
        helper.discover_tests(absolute_path, relative_path)
    }

    fn enumerate_tests(
        &self,
        _ctx: &crate::host::language_adapter::LanguageAdapterContext,
    ) -> EnumerationResult {
        EnumerationResult::default()
    }
}
