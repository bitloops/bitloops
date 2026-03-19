Feature: Test-harness runtime signals

  Scenario: TypeScript coverage batch and Jest results enrich query output
    Given an initialized copied TypeScript fixture repository with production artefacts for commit "C1"
    When I ingest tests for commit "C1"
    And I ingest coverage from the fixture manifest for commit "C1"
    And I ingest Jest results from the fixture for commit "C1"
    Then querying production artefact matching "findByEmail" with summary view includes coverage percentages for commit "C1"
    And querying production artefact matching "findByEmail" with coverage view returns branch coverage for commit "C1"
    And querying production artefact matching "UserService.createUser" with tests view surfaces a failing last run for commit "C1"
