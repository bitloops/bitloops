# Bitloops capture lifecycle

This is the main dynamic view for capture and checkpointing. It shows how agent hooks and Git hooks flow through the shared lifecycle and checkpoint strategy.

Use this when the question is "what side effects happen when a session progresses or a commit is made?"

```mermaid
sequenceDiagram
    participant Agent as "Agent client"
    participant Hooks as "Hook dispatcher"
    participant Lifecycle as "Shared lifecycle"
    participant Runtime as "Repo runtime SQLite"
    participant Strategy as "Checkpoint strategy"
    participant Git as "Working tree + Git"
    participant Repo as "Interaction repository"
    participant Spool as "DevQL producer spool"

    Agent->>Hooks: session-start / prompt / tool / stop hooks
    Hooks->>Lifecycle: normalize native hook payload
    Lifecycle->>Runtime: persist live session state
    Lifecycle->>Strategy: save step or task step
    Strategy->>Git: snapshot temporary checkpoint state
    Strategy->>Runtime: persist checkpoint metadata
    Lifecycle->>Repo: append interaction events

    Note over Agent,Spool: Git lifecycle callbacks are a separate trigger path.

    Git->>Hooks: post-commit / post-merge / post-checkout / pre-push
    Hooks->>Strategy: invoke git lifecycle callback
    Strategy->>Repo: reconcile uncheckpointed turns
    Strategy->>Runtime: update commit and checkpoint mappings
    Strategy-->>Spool: queue DevQL follow-up when required
```

## Notes

- Capture is about provenance and checkpoint formation.
- The strategy decides how session turns map to temporary or committed checkpoints.
- Git lifecycle callbacks can queue repo-local DevQL follow-up work, but that does not make sync part of the capture flow.
