Feature: CLI-1347 query levels and noise control
  As a developer
  I want explicit query levels and default noise filtering
  So the test harness returns the right amount of evidence for the task at hand

  Scenario: Q1-1 Summary view returns only the planning summary
    Given a temporary sqlite database and copied TypeScript fixture for query-layer validation
    And the copied TypeScript fixture has been fully indexed for the commit
    When I query "UserRepository.findByEmail" with summary view
    Then the response contains only artefact and summary data
    And the summary includes coverage percentages

  Scenario: Q1-2 Tests view applies the default strength filter and supports override
    Given a temporary sqlite database and copied TypeScript fixture for query-layer validation
    And the copied TypeScript fixture has been fully indexed for the commit
    When I query "UserRepository.findByEmail" with tests view using the default strength filter
    And I query "UserRepository.findByEmail" with tests view and min_strength 0.0
    Then the override returns more covering tests than the default query
    And the summary still reports the full covering-test count

  Scenario: Q1-3 Coverage view returns only artefact coverage data
    Given a temporary sqlite database and copied TypeScript fixture for query-layer validation
    And the copied TypeScript fixture has been fully indexed for the commit
    When I query "UserRepository.findByEmail" with coverage view
    Then the response contains only artefact and coverage data
    And the coverage payload includes branch entries

  Scenario: Q1-4 Summary view reports an untested artefact
    Given a temporary sqlite database and copied TypeScript fixture for query-layer validation
    And the copied TypeScript fixture has been fully indexed for the commit
    When I query "hashPassword" with summary view
    Then the summary reports the artefact as untested

  Scenario: Q1-5 Query errors distinguish unindexed commits from missing artefacts
    Given a temporary sqlite database and copied TypeScript fixture for query-layer validation
    When I query "UserRepository.findByEmail" on an unindexed commit
    Then the query fails with "Repository not indexed"
    Given the copied TypeScript fixture has been fully indexed for the commit
    When I query "MissingArtefact" with summary view expecting failure
    Then the query fails with "Artefact not found"
