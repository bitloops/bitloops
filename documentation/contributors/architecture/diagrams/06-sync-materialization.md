# Bitloops sync and materialization flow

This is the main dynamic view for DevQL sync. It shows how sync work is triggered, executed, and followed by consumer and enrichment stages.

Use this when the question is "how does repo state become current-state DevQL data?"

```mermaid
sequenceDiagram
    participant Watcher as "__devql-watcher"
    participant CLI as "Manual CLI trigger"
    participant Capture as "Capture handoff"
    participant Queue as "Daemon task queue"
    participant Sync as "Sync coordinator"
    participant Current as "Current-state relational model"
    participant History as "Historical / event / blob state"
    participant Consumers as "Current-state consumers"
    participant Enrich as "Enrichment workers"
    participant Models as "Inference / embeddings runtimes"

    alt watcher-driven
        Watcher->>Queue: enqueue sync task
    else manual CLI
        CLI->>Queue: enqueue sync / ingest / repair / validate task
    else capture-triggered
        Capture->>Queue: enqueue post_commit / post_merge / post_checkout sync
    end

    Queue->>Sync: run next task
    Sync->>Current: materialize current-state projections
    Sync->>History: persist historical and blob state as needed
    Sync->>Consumers: publish current-state generation
    Consumers->>Enrich: reconcile derived state
    Enrich->>Models: request summaries or embeddings when enabled
```

## Notes

- Sync is a daemon-owned materialization pipeline.
- Sync can be triggered by the watcher, by explicit CLI commands, or by a handoff from capture-side Git lifecycle events.
- Post-sync consumers and enrichment are separate stages after sync succeeds.
