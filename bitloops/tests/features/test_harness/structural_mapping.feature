Feature: Test-harness structural mapping

  Scenario: TypeScript test scenarios are materialized and linked to the correct production artefacts
    Given an initialized TypeScript repository with production artefacts for commit "C1"
    When I ingest tests for commit "C1"
    Then test suites, test scenarios, and test links are created for commit "C1"
    And "TypeScript" test artefacts are discoverable for commit "C1"
    And production artefact matching "findById" can be queried with covering tests for commit "C1"
    And scenario "calls email lookup only" links to symbol matching "findByEmail" but not "findById" for commit "C1"

  Scenario: Rust unit test scenarios are materialized and linked to the correct production artefacts
    Given an initialized Rust repository with production artefacts for commit "C1"
    When I ingest tests for commit "C1"
    Then test suites, test scenarios, and test links are created for commit "C1"
    And "Rust" test artefacts are discoverable for commit "C1"
    And production artefact matching "find_by_id" can be queried with covering tests for commit "C1"

  Scenario: Rust inline parameterized source tests are materialized as distinct scenarios
    Given an initialized Rust repository with inline parameterized tests for commit "C1"
    When I ingest tests for commit "C1"
    Then case-specific Rust test scenarios are materialized for commit "C1"
    And querying production artefact matching "string_dot_format_extra_positional_arguments" returns covering test "rules[StringDotFormatExtraPositionalArguments, F523.py]" for commit "C1"
    And querying production artefact matching "string_dot_format_extra_named_arguments" returns covering test "rules[StringDotFormatExtraNamedArguments, F522.py]" for commit "C1"

  Scenario: Rust wasm-bindgen and quickcheck declarations are materialized and linked
    Given an initialized Rust repository with additional test declarations for commit "C1"
    When I ingest tests for commit "C1"
    Then Ruff-style additional Rust test scenarios are materialized for commit "C1"
    And querying production artefact matching "render_message" returns covering test "empty_config" for commit "C1"
    And querying production artefact matching "is_equivalent_to" returns covering test "equivalent_to_is_reflexive" for commit "C1"

  Scenario: Cargo-backed Rust hybrid discovery reports hybrid enumeration and materializes mainstream declaration styles
    Given an initialized Cargo-backed Rust repository with rstest, proptest, and doctests for commit "C1"
    When I ingest tests for commit "C1"
    Then ingest-tests reports hybrid enumeration for commit "C1"
    And rstest, proptest, and doctest scenarios are materialized for commit "C1"

  Scenario: Cargo-backed Rust hybrid discovery links rstest, proptest, and doctest scenarios to production artefacts
    Given an initialized Cargo-backed Rust repository with rstest, proptest, and doctests for commit "C1"
    When I ingest tests for commit "C1"
    Then querying production artefact matching "double" returns covering tests "doubles_case_values[2, 4]" and "double_is_even" for commit "C1"
    And querying production artefact matching "triple" returns covering test "triples_from_template[2, 6]" for commit "C1"
    And querying production artefact matching "documented_increment" returns a doctest covering test for commit "C1"
