# Bitloops system context

This is the highest-level view. It treats Bitloops as one system and shows the people, tools, and external services around it.

Use this diagram when the question is about product boundaries rather than internal process layout.

```mermaid
flowchart LR
    Developer["Developer"]
    Agents["Supported agent clients"]
    Git["Git"]
    Browser["Dashboard browser"]

    Bitloops["Bitloops local system"]

    Providers["GitHub / Jira / Confluence"]
    Auth["WorkOS + OS keyring"]
    Stores["Configured durable stores"]
    Inference["Inference and embeddings runtimes"]

    Developer --> Bitloops
    Developer --> Agents
    Developer --> Git
    Browser <--> Bitloops
    Agents --> Bitloops
    Git --> Bitloops

    Bitloops <--> Providers
    Bitloops <--> Auth
    Bitloops <--> Stores
    Bitloops <--> Inference
```

## Notes

- The system is local-first from the developer's perspective.
- The main external dependencies are provider integrations, authentication, storage backends, and optional model runtimes.
- This view intentionally hides the split between the CLI, daemon, watcher, and hook surfaces. See the container view for that.
