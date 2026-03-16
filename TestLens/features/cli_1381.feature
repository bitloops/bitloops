Feature: CLI-1381 Rust test artefact detection completeness
  As a developer
  I want rstest, proptest, doctests, and hybrid cargo enumeration to be reflected in TestLens
  So mainstream Rust test scenarios are not missed because of declaration-style assumptions

  Scenario: R3-1 Mainstream Rust declaration styles are materialized with hybrid enumeration
    Given a Cargo-backed Rust fixture repository with rstest, proptest, and doctests at commit C1
    When static linkage is ingested for C1 in the cargo-backed fixture
    Then ingest-tests reports hybrid enumeration
    And rstest, proptest, and doctest scenarios are materialized

  Scenario: R3-2 Mainstream Rust declaration styles link to production artefacts
    Given a Cargo-backed Rust fixture repository with rstest, proptest, and doctests at commit C1
    When static linkage is ingested for C1 in the cargo-backed fixture
    Then querying double returns the covering tests "doubles_case_values[2, 4]" and "double_is_even"
    And querying triple returns the covering test "triples_from_template[2, 6]"
    And querying documented_increment returns a doctest covering test
