# Bitloops capture and checkpointing components

This component view covers the capture plane only. It is about provenance, session state, and checkpoint formation, not repo-state sync.

Use this when the question is "how do agent events and Git hooks become checkpoints and workflow history?"

```mermaid
flowchart TD
    subgraph Inputs["Capture inputs"]
        AgentHooks["Agent hook handlers"]
        GitHooks["Git hook handlers"]
    end

    subgraph Capture["Capture components"]
        Dispatcher["Shared hook dispatcher"]
        Lifecycle["Shared lifecycle normalization"]
        StrategyRegistry["Checkpoint strategy registry"]
        Strategy["Concrete checkpoint strategy"]
    end

    subgraph LocalState["Local workflow state"]
        RepoRuntime["Repo runtime SQLite"]
        Spool["Interaction spool"]
        GitState["Working tree + .git"]
    end

    subgraph Durable["Durable provenance state"]
        InteractionRepo["Interaction repository"]
        CheckpointProjection["Committed checkpoint projection"]
    end

    Handoff["Repo-local DevQL producer spool"]

    AgentHooks --> Dispatcher
    GitHooks --> Dispatcher

    Dispatcher --> Lifecycle
    Lifecycle --> RepoRuntime
    Lifecycle --> Spool
    Lifecycle --> StrategyRegistry
    StrategyRegistry --> Strategy

    Strategy --> GitState
    Strategy --> RepoRuntime
    Strategy --> InteractionRepo
    Spool --> InteractionRepo
    Strategy --> CheckpointProjection
    Strategy -. post_commit / post_merge / post_checkout / pre_push .-> Handoff
```

## Notes

- Agent-native hook payloads are normalized into one shared lifecycle before checkpoint logic runs.
- The checkpoint strategy is where temporary snapshots, task checkpoints, and commit-linked checkpoint consolidation happen.
- `post_commit` and related Git lifecycle events can hand work off to DevQL through the repo-local producer spool, but that handoff is downstream of capture rather than the capture flow itself.
