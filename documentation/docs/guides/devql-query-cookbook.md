---
sidebar_position: 4
title: DevQL Query Cookbook
---

# DevQL Query Cookbook

Practical examples for the current DevQL GraphQL surface. All examples assume the daemon has already bootstrapped the schema and you have already run `bitloops devql tasks enqueue --kind ingest`.

`bitloops devql query` accepts both DevQL DSL and raw GraphQL:

- DSL when the query contains `->`
- Raw GraphQL otherwise

## List Repository Artefacts

### DSL

```bash
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->select(path,symbol_fqn,canonical_kind,start_line,end_line)->limit(10)'
```

### Raw GraphQL

```bash
bitloops devql query '{ repo(name: "bitloops") { artefacts(first: 10, filter: { kind: FUNCTION }) { edges { node { path symbolFqn canonicalKind startLine endLine } } } } }'
```

## Scope To A Project In A Monorepo

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

Run it from the CLI with:

```bash
bitloops devql query --compact '{ repo(name: "bitloops") { project(path: "bitloops/src/graphql") { artefacts(first: 10) { edges { node { path symbolFqn } } } } } }'
```

## Aggregate Selection Summary For Agents

Use the slim repo-scoped `selectArtefacts(by: ...)` field when you want one compact summary over a selected set of artefacts.

For the full selector contract and stage semantics, see [selectArtefacts](/guides/select-artefacts).

```graphql
{
  selectArtefacts(by: { path: "rust-app/src/main.rs", lines: { start: 6, end: 10 } }) {
    summary
  }
}
```

```graphql
{
  selectArtefacts(by: { search: "payLater()" }) {
    artefacts(first: 10) {
      path
      symbolFqn
    }
  }
}
```

The `summary` payload includes one entry per available category, currently:

- `checkpoints`
- `clones`
- `dependencies`
- `tests`

Each category includes its default `summary` payload and, when results exist, a stage-local `schema` string.

## Selection Stage Details

When a category summary shows something interesting, ask that stage for typed detail rows through `items(first: ...)`.

```graphql
{
  selectArtefacts(by: { path: "rust-app/src/main.rs" }) {
    dependencies {
      summary
      expandHint {
        intent
        template
        parameters {
          direction
          kind
        }
      }
      schema
      items(first: 10) {
        id
        edgeKind
        toSymbolRef
      }
    }
  }
}
```

This is the intended follow-up flow for agents:

- start with `selectArtefacts { summary }`
- inspect the category summaries
- use `schema` only when needed
- ask the relevant stage for `items(...)`

## Query A Historical Snapshot

### DSL

```bash
bitloops devql query 'repo("bitloops")->asOf(ref:"main")->artefacts(kind:"function")->select(path,symbol_fqn)->limit(5)'
```

### Raw GraphQL

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

## Browse Commits And Checkpoints

```graphql
{
  repo(name: "bitloops") {
    commits(first: 10) {
      edges {
        node {
          sha
          commitMessage
          committedAt
          checkpoints(first: 1) {
            edges {
              node {
                id
                agent
                filesTouched
              }
            }
          }
        }
      }
    }
  }
}
```

## Find Knowledge Linked To The Repository

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

## Inspect Chat History For Artefacts

```graphql
{
  repo(name: "bitloops") {
    file(path: "bitloops/src/graphql.rs") {
      artefacts(first: 5) {
        edges {
          node {
            path
            symbolFqn
            chatHistory(first: 3) {
              edges {
                node {
                  agent
                  role
                  timestamp
                  content
                }
              }
            }
          }
        }
      }
    }
  }
}
```

## Test Harness And Coverage

```graphql
{
  repo(name: "bitloops") {
    project(path: "bitloops/src") {
      tests(first: 10, filter: { kind: FUNCTION }, minConfidence: 0.8) {
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

## Semantic Clones

### DSL

```bash
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->clones(min_score:0.8)->limit(10)'
```

`clones()` now defaults to a user-facing projection that highlights the source artefact, the
matched artefact, the clone relation kind, and the score. Use `clones(raw:true)` when you want the
low-level ids, source/target line ranges, and debug metadata instead.

### Raw GraphQL

```graphql
{
  repo(name: "bitloops") {
    project(path: "bitloops/src") {
      clones(first: 10, filter: { minScore: 0.8 }) {
        edges {
          node {
            relationKind
            score
            sourceArtefact {
              path
              symbolFqn
            }
            targetArtefact {
              path
              symbolFqn
            }
          }
        }
      }
    }
  }
}
```

### Raw GraphQL summary across the filtered artefact set

```graphql
{
  repo(name: "bitloops") {
    cloneSummary(
      filter: { kind: FUNCTION, symbolFqn: "packages/api/src/caller.ts::caller" }
      cloneFilter: { minScore: 0.75 }
    ) {
      totalCount
      groups {
        relationKind
        count
      }
    }
  }
}
```

### Raw GraphQL summary for one resolved artefact

```graphql
{
  repo(name: "bitloops") {
    file(path: "packages/api/src/caller.ts") {
      artefacts(
        filter: { kind: FUNCTION, symbolFqn: "packages/api/src/caller.ts::caller" }
        first: 1
      ) {
        edges {
          node {
            path
            symbolFqn
            clones(first: 10, filter: { minScore: 0.75 }) {
              totalCount
              summary {
                totalCount
                groups {
                  relationKind
                  count
                }
              }
            }
          }
        }
      }
    }
  }
}
```

Use `cloneSummary(...)` for one aggregate result over the filtered artefacts. Use nested `clones { summary }` when you need the summary attached to a specific artefact node.

## Tips

- Re-ingest after significant changes so relational, events, and blob-backed enrichments stay in sync
- Use `asOf(...)` when you need reproducible answers against a commit or save state
- Use `/devql/playground` to inspect the live schema before writing a client
- Use `bitloops devql schema --global --human > bitloops/schema.graphql` when you need client code generation or schema review
- The aggregate `selectArtefacts { summary }` query is currently GraphQL-only; the DevQL DSL `selectArtefacts(...)` path still targets one explicit terminal stage at a time
