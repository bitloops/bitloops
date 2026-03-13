Feature: CLI-1346 static call-site linkage
  As a developer
  I want test scenarios linked to production artefacts by real call-sites
  So tests() can surface accurate structural coverage before LCOV ingestion

  Scenario: L1-1 Create static linkage edges for called/referenced artefacts
    Given a fixture repository with production artefacts and tests at commit C1
    And a test scenario that calls UserRepository.findById
    When static linkage is ingested for C1
    Then a linkage edge is created to UserRepository.findById
    And the linkage is queryable before coverage ingestion

  Scenario: L1-2 Call-site sensitivity (not import-only)
    Given a fixture test scenario that imports UserRepository but only calls findByEmail
    When static linkage is ingested for C1
    Then linkage exists to UserRepository.findByEmail for that scenario
    And linkage is not created to UserRepository.findById for that scenario

  Scenario: L1-3 Commit-addressable linkage diffs
    Given commits C0 and C1 where a scenario references a new production artefact only in C1
    When linkage is queried for C0 and C1
    Then the new linkage edge appears only in C1
    And linkage query results are reproducible for both commits
