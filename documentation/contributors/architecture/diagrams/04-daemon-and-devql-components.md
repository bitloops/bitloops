# Bitloops daemon and DevQL components

This component view focuses on the daemon-side runtime: HTTP and GraphQL serving, task execution, DevQL orchestration, capability packs, and post-sync consumers.

Use this when the question is "what runs inside the daemon, and how does DevQL execution relate to sync and enrichment?"

```mermaid
flowchart TD
    subgraph Serving["Serving surfaces"]
        Router["HTTP router"]
        Slim["/devql"]
        Global["/devql/global"]
        Runtime["/devql/runtime"]
        Dashboard["/devql/dashboard"]
    end

    subgraph Inputs["Repo-local inputs"]
        Spool["Producer spool"]
    end

    subgraph Core["Daemon core"]
        Bootstrap["Daemon runtime bootstrap"]
        Tasks["DevQL task coordinator"]
        Consumers["Current-state consumer coordinator"]
        Enrichment["Enrichment coordinator + workplane"]
        Init["Init runtime coordinator"]
    end

    subgraph Host["Host-owned runtime"]
        GraphqlCtx["DevQL GraphQL context"]
        CapabilityHost["DevqlCapabilityHost"]
        ExtensionHost["CoreExtensionHost"]
        Languages["Language services"]
        Connectors["Connector registry"]
        Inference["Inference gateways"]
    end

    subgraph Packs["Built-in capability packs"]
        Knowledge["knowledge"]
        Tests["test_harness"]
        Clones["semantic_clones"]
    end

    subgraph Storage["Storage"]
        RuntimeState["Daemon / repo runtime SQLite"]
        Current["Current-state relational model"]
        History["Historical / event / blob state"]
    end

    Bootstrap --> Router
    Bootstrap --> Tasks
    Bootstrap --> Consumers
    Bootstrap --> Enrichment
    Bootstrap --> Init

    Router --> Slim
    Router --> Global
    Router --> Runtime
    Router --> Dashboard

    Slim --> GraphqlCtx
    Global --> GraphqlCtx
    GraphqlCtx --> CapabilityHost
    Runtime --> Init
    Dashboard --> RuntimeState
    Dashboard --> Current
    Dashboard --> History

    Spool --> Tasks
    Init --> Tasks
    Tasks --> Init
    Tasks --> CapabilityHost
    Tasks --> RuntimeState
    Tasks --> Current
    Tasks --> History
    Tasks --> Consumers
    Tasks --> Enrichment

    CapabilityHost --> ExtensionHost
    CapabilityHost --> Languages
    CapabilityHost --> Connectors
    CapabilityHost --> Knowledge
    CapabilityHost --> Tests
    CapabilityHost --> Clones
    CapabilityHost --> Current
    CapabilityHost --> History

    Consumers --> Tests
    Consumers --> Clones
    Enrichment --> Inference
```

## Notes

- The daemon now exposes four distinct GraphQL surfaces: `/devql`, `/devql/global`, `/devql/runtime`, and `/devql/dashboard`.
- The DevQL task coordinator is the main async entrypoint for sync, ingest, bootstrap, and producer-spool work.
- Current-state consumer execution, enrichment, and init/runtime orchestration are separate daemon coordinators rather than one generic sync queue.
- `/devql/runtime` and `/devql/dashboard` are operational surfaces alongside the DevQL query surfaces; they are not just aliases for capability-host execution.
- The host still owns capability execution, language resolution, connector access, and storage access beneath the DevQL GraphQL contract.
