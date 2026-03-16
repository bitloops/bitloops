Feature: Rust test declarations end-to-end
  Scenario: Inline src #[test_case], wasm-bindgen, and macro-generated quickcheck tests are queryable through the full CLI journey
    Given a temporary sqlite database for Rust parameterized linkage
    And a Rust fixture repository with inline parameterized src tests and Ruff-style additional declarations
    When I run the Rust parameterized linkage journey
    Then listing Rust test scenarios returns the parameterized and Ruff-style cases
    And querying "string_dot_format_extra_positional_arguments" returns the covering test "rules[StringDotFormatExtraPositionalArguments, F523.py]"
    And querying "string_dot_format_extra_named_arguments" returns the covering test "rules[StringDotFormatExtraNamedArguments, F522.py]"
    And querying "render_message" returns the covering test "empty_config"
    And querying "is_equivalent_to" returns the covering test "equivalent_to_is_reflexive"
