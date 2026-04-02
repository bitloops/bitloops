Feature: Semantic and embeddings resilience BDD scenarios

  Scenario: SE1 Health reports deterministic semantic fallback and disabled embeddings
    Given a daemon config:
      """
      [semantic_clones]
      summary_mode = "auto"
      """
    When semantic clone health checks run
    Then semantic clone health includes:
      | check                               | healthy | message_fragment                                          |
      | semantic_clones.semantic_summaries | true    | deterministic fallback only                               |
      | semantic_clones.profile_resolution | true    | embeddings disabled                                       |
      | semantic_clones.runtime_command    | true    | runtime command not required                              |
      | semantic_clones.runtime_handshake  | true    | runtime handshake skipped                                 |

  Scenario: SE2 Health resolves an arbitrary embedding profile name through the runtime
    Given a daemon config using the fake embeddings runtime:
      """
      [semantic_clones]
      summary_mode = "off"
      embedding_profile = "default"

      [embeddings.profiles.default]
      kind = "openai"
      model = "text-embedding-3-small"
      api_key = "test-key"
      """
    When semantic clone health checks run
    Then semantic clone health includes:
      | check                               | healthy | message_fragment                              |
      | semantic_clones.semantic_summaries | true    | semantic summaries disabled                   |
      | semantic_clones.profile_resolution | true    | embedding profile `default` resolved          |
      | semantic_clones.runtime_command    | true    | runtime command available                     |
      | semantic_clones.runtime_handshake  | true    | runtime describe succeeded for profile        |

  Scenario: SE3 Local pull succeeds through the standalone embeddings runtime
    Given a daemon config using the fake embeddings runtime:
      """
      [semantic_clones]
      embedding_profile = "local"

      [embeddings.profiles.local]
      kind = "local_fastembed"
      """
    When bitloops embeddings pull runs for profile "local"
    Then the last operation succeeds

  Scenario: SE4 Doctor reports the active local embedding profile
    Given a daemon config:
      """
      [semantic_clones]
      embedding_profile = "local"

      [embeddings.profiles.local]
      kind = "local_fastembed"
      """
    When bitloops embeddings doctor runs
    Then the last operation succeeds
    And the last operation output includes:
      | line_fragment         |
      | Profile: local        |
      | Kind: local_fastembed |
      | Cache status: missing |

  Scenario: SE5 Clear-cache removes the local embedding cache directory
    Given a daemon config:
      """
      [semantic_clones]
      embedding_profile = "local"

      [embeddings.profiles.local]
      kind = "local_fastembed"
      """
    And the local embedding cache exists for profile "local"
    When bitloops embeddings clear-cache runs for profile "local"
    Then the last operation succeeds
    And the local embedding cache is absent for profile "local"

  Scenario: SE6 Enrichment queue controls apply to semantic, embedding, and clone-edge jobs together
    Given an enrichment queue state with jobs:
      | kind                | status  |
      | semantic_summaries  | pending |
      | symbol_embeddings   | failed  |
      | clone_edges_rebuild | failed  |
    When the enrichment queue status is requested
    Then the enrichment queue reports:
      | metric                           | value |
      | pending_jobs                     | 1     |
      | pending_semantic_jobs            | 1     |
      | pending_embedding_jobs           | 0     |
      | pending_clone_edges_rebuild_jobs | 0     |
      | failed_jobs                      | 2     |
      | failed_embedding_jobs            | 1     |
      | failed_clone_edges_rebuild_jobs  | 1     |
    When the enrichment queue is paused with reason "bdd pause"
    And the enrichment queue status is requested
    Then the enrichment queue mode is "paused"
    And the enrichment queue pause reason is "bdd pause"
    When failed enrichment jobs are retried
    And the enrichment queue status is requested
    Then the enrichment queue reports:
      | metric                           | value |
      | pending_jobs                     | 3     |
      | pending_semantic_jobs            | 1     |
      | pending_embedding_jobs           | 1     |
      | pending_clone_edges_rebuild_jobs | 1     |
      | failed_jobs                      | 0     |
      | retried_failed_jobs              | 2     |
    When the enrichment queue is resumed
    And the enrichment queue status is requested
    Then the enrichment queue mode is "running"
