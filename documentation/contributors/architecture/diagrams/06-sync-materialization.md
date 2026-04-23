# Bitloops sync and materialization flow

This is the main dynamic view for DevQL sync. It shows how sync work is triggered, executed, and followed by consumer and enrichment stages.

Use this when the question is "how does repo state become current-state DevQL data?"

```mermaid
sequenceDiagram
    participant Watcher as "__devql-watcher"
    participant CLI as "Manual CLI trigger"
    participant Capture as "Capture-side Git follow-up"
    participant Spool as "Repo-local producer spool"
    participant Tasks as "Daemon task coordinator"
    participant Sync as "Sync / refresh execution"
    participant Current as "Current-state relational model"
    participant History as "Historical / event / blob state"
    participant Consumers as "Current-state consumer coordinator"
    participant Enrich as "Enrichment coordinator"
    participant Models as "Inference / embeddings runtimes"

    alt watcher-driven
        Watcher->>Spool: queue changed-path sync job
    else manual CLI
        CLI->>Tasks: enqueue sync / ingest / repair / validate task
    else capture-triggered
        Capture->>Spool: queue post_commit / post_merge / post_checkout follow-up
    end

    Spool->>Tasks: daemon claims pending producer jobs
    Tasks->>Sync: run sync or refresh work
    Sync->>Current: materialize current-state projections
    Sync->>History: persist historical / event / blob state as needed
    opt current-state generation advanced
        Sync->>Consumers: enqueue current-state consumer runs
        Consumers->>Enrich: reconcile derived state and workplane jobs
        Enrich->>Models: request summaries or embeddings when enabled
    end
```

## Notes

- Sync is a daemon-owned materialization pipeline.
- Watcher-driven and capture-triggered follow-up work lands in the repo-local producer spool before the daemon claims it.
- Explicit CLI commands enqueue daemon tasks directly, while producer-spool jobs are drained by the same daemon task coordinator.
- Current-state consumers and enrichment are downstream stages after current-state generation advances.
