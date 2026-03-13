Feature: CLI-1348 coverage ingestion
  As a developer
  I want LCOV ingestion to be commit-addressed
  So coverage is only attached to the commit that produced it

  Scenario: C2-1 Ingest Rust coverage for one commit without leaking to another
    Given a temporary sqlite database for Rust coverage ingestion validation
    And the real Rust fixture repository is indexed for commits C0 and C1
    When I generate Rust LCOV and ingest it for commit C1
    Then querying "UserRepository.find_by_id" with coverage view at commit C0 returns null coverage
    And querying "UserRepository.find_by_id" with coverage view at commit C1 returns non-null coverage
    And coverage rows exist only for commit C1
