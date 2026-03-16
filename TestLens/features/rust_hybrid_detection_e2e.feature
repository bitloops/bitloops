Feature: Rust hybrid test detection end-to-end
  Scenario: rstest, proptest, doctests, and hybrid enumeration are queryable through the full CLI journey
    Given a temporary sqlite database for Rust hybrid detection
    And a Cargo-backed Rust fixture repository with rstest, proptest, and doctests
    When I run the Rust hybrid detection journey
    Then listing Rust test scenarios returns rstest, proptest, and doctest cases
    And querying "double" returns the covering test "doubles_case_values[2, 4]"
    And querying "double" returns the covering test "double_is_even"
    And querying "triple" returns the covering test "triples_from_template[2, 6]"
    And querying "documented_increment" returns a doctest covering test
