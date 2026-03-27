---
sidebar_position: 4
title: DevQL Query Cookbook
---

# DevQL Query Cookbook

Practical examples for the current DevQL GraphQL surface. All examples assume you have already run `bitloops devql init` and `bitloops devql ingest`.

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

## Tips

- Re-ingest after significant changes so relational, events, and blob-backed enrichments stay in sync
- Use `asOf(...)` when you need reproducible answers against a commit or save state
- Use `/devql/playground` to inspect the live schema before writing a client
- Export `bitloops/schema.graphql` when you need client code generation or schema review
