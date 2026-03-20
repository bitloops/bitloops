use tree_sitter::Parser;
use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;
use tree_sitter_typescript::LANGUAGE_TYPESCRIPT;

use super::file_discovery::rust_source_contains_doctest_markers;
use super::languages::rust::enumeration::parse_enumerated_doctests;
use super::languages::rust::imports::{
    collect_rust_import_paths_for, collect_rust_scoped_call_import_paths_for,
    rust_test_context_source_paths,
};
use super::languages::rust::macros::extract_rust_macro_invocation_body;
use super::languages::rust::scenarios::collect_rust_suites;
use super::languages::typescript::{
    collect_typescript_suites, extract_import_specifier, resolve_import_to_repo_path, unquote,
};
use super::linker::{imported_path_matches_production_path, symbol_match_key};
use super::model::{ReferenceCandidate, ScenarioDiscoverySource};

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
    assert_eq!(scenarios[0].scenario_name, "sample::documented_increment");
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
