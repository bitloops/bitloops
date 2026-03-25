Feature: DevQL error BDD scenarios

  @ERR1
  Scenario: ERR1 AST parse failure
    Given a TypeScript source file at "src/broken.ts":
      """
      function broken( {
      """
    When devql extracts artefacts and dependency edges with logger capture
    Then no artefacts are emitted
    And no edges are emitted
    And devql logs a parse-failure event with path "src/broken.ts"

  @ERR2
  Scenario: ERR2 Invalid stage composition
    When devql parses the query:
      """
      repo("temp2")->artefacts(kind:"function")->deps(kind:"calls")->chatHistory()->limit(10)
      """
    And devql executes the query without a Postgres client
    Then the query fails with message containing "deps() cannot be combined with chatHistory()"
    And devql logs a validation-error event containing "deps() cannot be combined with chatHistory() stage"
