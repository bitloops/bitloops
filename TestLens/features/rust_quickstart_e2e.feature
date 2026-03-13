Feature: Rust quickstart end-to-end acceptance
  As a developer validating the Rust-first prototype flow
  I want to run the documented Rust quickstart against the real fixture repository
  So that production discovery, test discovery, static linkage, and query behavior are verified end-to-end

  Scenario: Run the Rust quickstart flow against the real fixture repository
    Given a temporary sqlite database for the Rust fixture quickstart
    And the real Rust fixture repository passes its own tests
    When I run the Rust quickstart ingestion commands
    Then Rust production artefacts are materialized for the commit
    And Rust test suites and scenarios are materialized for the commit
    And static test links are created for the commit
    And querying "UserRepository.find_by_id" returns covering tests before coverage ingestion
    And the query coverage payload is null before coverage ingestion
