# Bitloops daemon and DevQL components

This component view focuses on the daemon-side runtime: HTTP and GraphQL serving, task execution, DevQL orchestration, capability packs, and post-sync consumers.

Use this when the question is "what runs inside the daemon, and how does DevQL execution relate to sync and enrichment?"

```mermaid
flowchart TD
    subgraph Serving["Serving surfaces"]
        Router["HTTP router"]
        Slim["/devql"]
        Global["/devql/global"]
        Dashboard["/devql/dashboard"]
    end

    subgraph Core["Daemon core"]
        Bootstrap["Daemon runtime bootstrap"]
        Queue["Task queue"]
        Sync["Sync coordinator"]
        Consumers["Current-state consumers"]
    end

    subgraph Host["Host-owned runtime"]
        CapabilityHost["DevqlCapabilityHost"]
        ExtensionHost["CoreExtensionHost"]
        Languages["Language services"]
        Connectors["Connector registry"]
        Inference["Inference + embeddings gateways"]
    end

    subgraph Packs["Built-in capability packs"]
        Knowledge["knowledge"]
        Tests["test_harness"]
        Clones["semantic_clones"]
    end

    subgraph Storage["Storage"]
        Current["Current-state relational model"]
        History["Historical / event / blob state"]
    end

    Bootstrap --> Router
    Bootstrap --> Queue

    Router --> Slim
    Router --> Global
    Router --> Dashboard

    Slim --> CapabilityHost
    Global --> CapabilityHost
    Dashboard --> CapabilityHost

    CapabilityHost --> ExtensionHost
    CapabilityHost --> Languages
    CapabilityHost --> Connectors
    CapabilityHost --> Knowledge
    CapabilityHost --> Tests
    CapabilityHost --> Clones
    CapabilityHost --> Current
    CapabilityHost --> History

    Queue --> Sync
    Sync --> Current
    Sync --> History
    Sync --> Consumers

    Consumers --> Tests
    Consumers --> Clones
    Consumers --> Inference
```

## Notes

- The GraphQL surfaces are distinct product interfaces, even though they share the same daemon runtime.
- The daemon owns task execution and async follow-up work.
- The host owns capability execution, language resolution, connector access, and storage access beneath the GraphQL contract.
- Sync and post-sync consumers are part of the daemon runtime, not the capture plane.
