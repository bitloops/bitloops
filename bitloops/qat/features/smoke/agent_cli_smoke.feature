Feature: Bitloops unified Agent Smoke suite
  As a Bitloops maintainer
  I want deterministic Agent Smoke coverage for the supported agent integrations
  So that `cargo qat-agent-smoke` and `cargo qat` validate the Bitloops golden path in CI

  @agent_smoke
  Scenario Outline: First agent-driven Bitloops session is captured
    Given I run CleanStart for flow "<first_flow>"
    And I start the daemon in bitloops
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent <agent> --sync=false in bitloops
    And I make a first change using <agent> to bitloops
    And I committed today in bitloops
    Then bitloops stores exist in bitloops
    And <agent> session exists in bitloops
    And checkpoint mapping exists in bitloops

    @develop_gate
    Examples:
      | agent       | first_flow              |
      | claude-code | SmokeClaudeFirstSession |

    @develop_gate
    Examples:
      | agent  | first_flow             |
      | cursor | SmokeCursorFirstSession |

    @develop_gate
    Examples:
      | agent | first_flow            |
      | codex | SmokeCodexFirstSession |

    Examples:
      | agent   | first_flow             |
      | gemini  | SmokeGeminiFirstSession |

    Examples:
      | agent   | first_flow              |
      | copilot | SmokeCopilotFirstSession |

    Examples:
      | agent    | first_flow               |
      | opencode | SmokeOpenCodeFirstSession |

  @agent_smoke
  Scenario Outline: Follow-up agent edits create progression
    Given I run CleanStart for flow "<followup_flow>"
    And I start the daemon in bitloops
    And I create a Vite app project in bitloops
    And I run InitCommit for bitloops
    And I run bitloops init --agent <agent> --sync=false in bitloops
    And I make a first change using <agent> to bitloops
    And I committed today in bitloops
    And I make a second change using <agent> to bitloops
    And I committed today in bitloops
    Then bitloops stores exist in bitloops
    And <agent> session exists in bitloops
    And checkpoint mapping count is at least 2 in bitloops

    @develop_gate
    Examples:
      | agent       | followup_flow          |
      | claude-code | SmokeClaudeProgression |

    @develop_gate
    Examples:
      | agent  | followup_flow          |
      | cursor | SmokeCursorProgression |

    @develop_gate
    Examples:
      | agent | followup_flow        |
      | codex | SmokeCodexProgression |

    Examples:
      | agent   | followup_flow         |
      | gemini  | SmokeGeminiProgression |

    Examples:
      | agent   | followup_flow          |
      | copilot | SmokeCopilotProgression |

    Examples:
      | agent    | followup_flow         |
      | opencode | SmokeOpenCodeProgression |
