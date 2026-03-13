Feature: CLI-1349 coverage query behavior
  As a developer
  I want coverage queries to be artefact-scoped and explicit about gaps
  So the harness highlights the right uncovered code paths

  Scenario: V2-1 Coverage view is scoped to the queried artefact span
    Given a temporary sqlite database and copied TypeScript fixture for coverage-view validation
    And the copied TypeScript fixture has been fully indexed for coverage-view validation
    When I query coverage view for "UserRepository.findById" and "UserRepository.findByEmail"
    Then both coverage queries refer to the same source file
    And the coverage payloads are different for the two artefacts

  Scenario: V2-2 Coverage view surfaces uncovered branches
    Given a temporary sqlite database and copied TypeScript fixture for coverage-view validation
    And the copied TypeScript fixture has been fully indexed for coverage-view validation
    When I query coverage view for "UserService.createUser"
    Then the coverage payload contains uncovered branches
