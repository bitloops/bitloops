Feature: DevQL query BDD scenarios

  @S4
  Scenario: S4 Reverse dependency query
    When devql parses the query:
      """
      repo("temp2")->artefacts(kind:"function")->deps(kind:"calls",direction:"in",include_unresolved:false)->limit(10)
      """
    And devql builds the deps SQL
    Then the generated SQL contains:
      | fragment                                                        |
      | JOIN artefacts_current at ON at.artefact_id = e.to_artefact_id |
      | e.to_artefact_id IS NOT NULL                                   |

  @S5
  Scenario: S5 Bidirectional dependency query
    When devql parses the query:
      """
      repo("temp2")->artefacts(kind:"function")->deps(kind:"exports",direction:"both")->limit(10)
      """
    And devql builds the deps SQL
    Then the generated SQL contains:
      | fragment         |
      | WITH out_edges AS |
      | UNION ALL        |
