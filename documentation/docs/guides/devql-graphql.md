---
title: DevQL GraphQL
---

# DevQL GraphQL

DevQL is exposed as a GraphQL-compatible schema. The CLI, dashboard, and HTTP/WebSocket clients all execute the same schema, so there is one typed contract for queries, mutations, subscriptions, and capability-pack extensions.

## Surface Summary

When the Bitloops daemon is running, DevQL exposes these routes:

| Route | Method | Purpose |
|---|---|---|
| `/devql` | `POST` | GraphQL queries and mutations |
| `/devql` | `GET` | DevQL Explorer UI |
| `/devql/playground` | `GET` | DevQL Explorer UI |
| `/devql/sdl` | `GET` | Generated schema SDL |
| `/devql/ws` | WebSocket | GraphQL subscriptions |

The checked-in schema snapshot lives at `bitloops/schema.graphql`. The canonical runtime schema is whatever `GET /devql/sdl` returns.

## CLI Query Modes

`bitloops devql query` executes against the local Bitloops daemon. It supports two input styles:

- DevQL DSL pipelines when the query contains `->`
- Raw GraphQL by default for any other input

`--graphql` remains available as an explicit override when you want to force raw GraphQL execution.

```bash
# DSL pipeline compiled to GraphQL before execution
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->limit(5)'

# Raw GraphQL is the default when there is no `->`
bitloops devql query '{ repo(name: "bitloops") { artefacts(first: 5) { edges { node { path symbolFqn canonicalKind } } } } }'

# Emit compact JSON instead of pretty JSON or table output
bitloops devql query --compact '{ health { relational { backend connected } } }'
```

Without `--compact`, raw GraphQL output is printed as formatted JSON. DSL queries keep the CLI table rendering where that still fits the result shape.

Connection fields support both forward and reverse cursor pagination:

- Forward: `first` with optional `after`
- Reverse: `last` with optional `before`

Do not mix the two modes in the same field call.

## Query Examples

### Repository artefacts

```graphql
{
  repo(name: "bitloops") {
    artefacts(first: 10, filter: { kind: FUNCTION }) {
      edges {
        node {
          path
          symbolFqn
          canonicalKind
          startLine
          endLine
        }
      }
    }
  }
}
```

### Monorepo project scope

```graphql
{
  repo(name: "bitloops") {
    project(path: "bitloops/src/graphql") {
      artefacts(first: 10) {
        edges {
          node {
            path
            symbolFqn
          }
        }
      }
    }
  }
}
```

### Historical query with `asOf`

```graphql
{
  repo(name: "bitloops") {
    asOf(input: { ref: "main" }) {
      artefacts(first: 5, filter: { kind: FUNCTION }) {
        edges {
          node {
            path
            symbolFqn
          }
        }
      }
    }
  }
}
```

### Knowledge and capability-pack enrichments

```graphql
{
  repo(name: "bitloops") {
    knowledge(first: 10, provider: JIRA) {
      edges {
        node {
          title
          externalUrl
          latestVersion {
            title
            bodyPreview
          }
        }
      }
    }
  }
}
```

```graphql
{
  repo(name: "bitloops") {
    project(path: "bitloops/src") {
      tests(first: 10, filter: { kind: FUNCTION }, minConfidence: 0.7) {
        artefact {
          filePath
          name
        }
        summary {
          totalCoveringTests
        }
      }
      coverage(first: 10, filter: { kind: FUNCTION }) {
        artefact {
          filePath
          name
        }
        summary {
          uncoveredLineCount
          uncoveredBranchCount
        }
      }
    }
  }
}
```

### Reverse pagination

```graphql
{
  repo(name: "bitloops") {
    commits(last: 10, before: "commit-sha-cursor") {
      pageInfo {
        hasNextPage
        hasPreviousPage
      }
      edges {
        node {
          sha
          commitMessage
        }
      }
    }
  }
}
```

## Mutations

The CLI write commands are thin clients over the local daemon GraphQL surface:

- `bitloops devql init` → `initSchema`
- `bitloops devql ingest` → `ingest`
- `bitloops devql knowledge add` → `addKnowledge`
- `bitloops devql knowledge associate` → `associateKnowledge`
- `bitloops devql knowledge refresh` → `refreshKnowledge`

The daemon now owns normal schema bootstrap on startup. Use `initSchema` when you want an explicit schema-initialisation pass, and use `ingest` for ingestion only.

Example mutation:

```graphql
mutation Ingest($input: IngestInput!) {
  ingest(input: $input) {
    success
    checkpointsProcessed
    eventsInserted
    artefactsUpserted
  }
}
```

```json
{
  "input": {
    "maxCheckpoints": 200
  }
}
```

Capability-pack migrations are also exposed through GraphQL:

```graphql
mutation {
  applyMigrations {
    success
    migrationsApplied {
      packId
      migrationName
      appliedAt
    }
  }
}
```

## Subscriptions

DevQL currently exposes two subscription fields:

- `checkpointIngested(repoName: String!)`
- `ingestionProgress(repoName: String!)`

Example subscription:

```graphql
subscription IngestProgress {
  ingestionProgress(repoName: "bitloops") {
    phase
    checkpointsProcessed
    checkpointsTotal
    currentCommitSha
  }
}
```

Use the WebSocket endpoint at `/devql/ws`. The DevQL Explorer page is already configured to use that endpoint.

## SDL Export And Versioning

The schema is versioned in two places:

- Generated at runtime from the `async-graphql` schema
- Checked in as `bitloops/schema.graphql` for review and client code generation

Export the current schema snapshot from the repository root with:

```bash
cargo run --manifest-path bitloops/Cargo.toml --example export-schema > bitloops/schema.graphql
```

The test suite asserts that the checked-in snapshot still matches the runtime SDL.

## Capability-Pack Extension Points

DevQL exposes capability-pack functionality in three ways:

- Typed fields such as `knowledge`, `tests`, `coverage`, and `clones`
- Generic `extension(stage:, args:, first:)` fields on `Project` and `Artefact`
- `applyMigrations` for capability-pack schema migrations

`extension()` expects a stage name plus an optional JSON object whose values are strings, numbers, booleans, or `null`. The GraphQL `extension()` adapter forwards those scalar values to the stage without stringifying them. Unsupported or ambiguous stage names fail before execution.

## Error Codes And Limits

DevQL currently emits these structured GraphQL error codes from resolver code:

| Code | Meaning |
|---|---|
| `BAD_USER_INPUT` | Invalid field arguments, invalid temporal scope input, invalid references, or unsupported combinations |
| `BAD_CURSOR` | Invalid pagination cursor |
| `BACKEND_ERROR` | Backend configuration, execution, migration, provider, or serialisation failures |

The schema also enforces global limits:

- Maximum query depth: `16`
- Maximum query complexity: `256`

Depth and complexity rejections come from the GraphQL runtime and may not include a custom `extensions.code`.

## Testing And Migration Notes

GraphQL accuracy is covered directly in the repository:

- `bitloops/src/api/tests.rs` covers routes, SDL, mutations, subscriptions, limits, and dashboard wrappers
- `bitloops/tests/graphql/verification.rs` checks end-to-end CLI parity between DSL and raw GraphQL
- `bitloops/schema.graphql` is asserted against the runtime schema

The dashboard still serves legacy `/api/*` routes for compatibility, but those wrappers now execute the DevQL GraphQL schema internally rather than maintaining a separate SQL-style read path.
