Feature: TypeScript full journey end-to-end acceptance
  As a developer validating the full query journey
  I want the real TypeScript fixture to exercise production ingestion, test discovery, coverage, run results, and query levels
  So new query features are proven against the whole SQLite-backed CLI flow

  Scenario: Run the full TypeScript journey against a copied real fixture repository
    Given a temporary sqlite database for the TypeScript full journey
    And a copied real TypeScript fixture repository for the full journey
    When I run the full TypeScript ingestion journey
    Then querying "UserRepository.findByEmail" with summary view returns only summary data
    And querying "UserRepository.findByEmail" with summary view includes coverage percentages
    And querying "UserRepository.findByEmail" with tests view applies default strength filtering
    And querying "UserRepository.findByEmail" with tests view and min_strength 0.0 returns more tests
    And querying "UserRepository.findByEmail" with coverage view returns branch coverage
    And querying "hashPassword" with summary view reports the artefact as untested
    And querying "UserService.createUser" with tests view surfaces a failing last run

  Scenario: Query errors distinguish an unindexed commit from a missing artefact
    Given a temporary sqlite database for the TypeScript full journey
    And a copied real TypeScript fixture repository for the full journey
    When I query "UserRepository.findByEmail" before indexing the TypeScript journey
    Then the query fails with "Repository not indexed"
    When I run the full TypeScript ingestion journey
    And I query "MissingArtefact" after indexing the TypeScript journey
    Then the query fails with "Artefact not found"
