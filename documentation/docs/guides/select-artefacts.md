---
title: selectArtefacts
---

# selectArtefacts

`selectArtefacts(by: ...)` is the slim, repo-scoped DevQL GraphQL selector for agent-oriented code analysis.

It lets you:

- select a current set of artefacts once
- ask for one aggregate `summary` across all supported categories
- drill into one category only when that summary suggests it is worth the tokens

This is the intended shape for tool-using agents.

## Availability

`selectArtefacts(by: ...)` is available on the slim GraphQL surface only.

- Raw GraphQL: supported
- Slim `/devql` SDL and Explorer: supported
- DevQL DSL compiler: supported for one explicit terminal stage at a time
- DevQL DSL aggregate `selectArtefacts(...)->summary()`: not supported yet
- Global/full GraphQL surface: not supported

## Selector Modes

Exactly one selector mode must be used.

### By `symbolFqn`

```graphql
{
  selectArtefacts(by: { symbolFqn: "rust-app/src/main.rs::main" }) {
    count
    artefacts {
      path
      symbolFqn
    }
  }
}
```

This usually resolves to `0..1` logical artefacts, but callers should treat the result as a set.

### By `fuzzyName`

```graphql
{
  selectArtefacts(by: { fuzzyName: "payLater()" }) {
    count
    artefacts {
      path
      symbolFqn
    }
  }
}
```

This searches current artefacts in scope by normalized symbol name, including typo-tolerant matches such as `payLater()` or `payLatr()`. v1 returns up to 10 best-first matches and does not expose scores in the API.

### By `naturalLanguage`

```graphql
{
  selectArtefacts(by: { naturalLanguage: "find the artefacts that build invoice PDFs" }) {
    count
    artefacts {
      path
      symbolFqn
    }
  }
}
```

This embeds free-form natural language and compares it against prepared current artefact embeddings in the active slim scope. Results are ordered by internal similarity, weak matches are dropped, and v1 does not expose scores in the API.

### By `path` and `lines`

```graphql
{
  selectArtefacts(by: { path: "rust-app/src/main.rs", lines: { start: 6, end: 10 } }) {
    count
    artefacts {
      path
      symbolFqn
      startLine
      endLine
    }
  }
}
```

This resolves all current artefacts in that file whose ranges overlap the selected line span.

### By `path`

```graphql
{
  selectArtefacts(by: { path: "rust-app/src/main.rs" }) {
    count
    artefacts {
      path
      symbolFqn
    }
  }
}
```

This resolves all current artefacts in the file.

## Validation Rules

- `symbolFqn` cannot be combined with `fuzzyName`, `naturalLanguage`, `path`, or `lines`
- `fuzzyName` cannot be combined with `symbolFqn`, `naturalLanguage`, `path`, or `lines`
- `fuzzyName` must be non-empty
- `naturalLanguage` cannot be combined with `symbolFqn`, `fuzzyName`, `path`, or `lines`
- `naturalLanguage` must be non-empty
- `lines` requires `path`
- empty selectors are rejected
- selector paths are resolved relative to the slim request scope, including project-scoped slim requests
- selection is current-state only in v1; `asOf(...)` is not part of this surface

## Selection Shape

`selectArtefacts(by: ...)` returns one `ArtefactSelection` object representing the selected set.

```graphql
type ArtefactSelection {
  count: Int!
  summary: JSON!
  artefacts(first: Int! = 20): [Artefact!]!
  checkpoints(agent: String, since: DateTime): CheckpointStageResult!
  clones(relationKind: String, minScore: Float): CloneStageResult!
  deps(kind: EdgeKind, direction: DepsDirection! = BOTH, includeUnresolved: Boolean! = true): DependencyStageResult!
  tests(minConfidence: Float, linkageSource: String): TestsStageResult!
}
```

Use:

- `count` to know how many artefacts matched
- `artefacts(...)` when you want to inspect the matched set itself
- `summary` when you want one compact answer across all supported categories
- stage fields when you want one category in more detail

## Aggregate `summary`

The selection-level `summary` field returns a JSON object with one entry per supported category.

```graphql
{
  selectArtefacts(by: { path: "rust-app/src/main.rs", lines: { start: 6, end: 10 } }) {
    summary
  }
}
```

Representative shape:

```json
{
  "selectedArtefactCount": 2,
  "checkpoints": {
    "summary": {
      "totalCount": 0,
      "latestAt": null,
      "agents": []
    },
    "schema": null
  },
  "clones": {
    "summary": {
      "totalCount": 2,
      "groups": [
        { "relationKind": "similar_implementation", "count": 2 }
      ],
      "maxScore": 0.93
    },
    "schema": "type ArtefactSelection { ... }"
  },
  "deps": {
    "summary": {
      "selectedArtefactCount": 2,
      "totalCount": 2,
      "incomingCount": 0,
      "outgoingCount": 2,
      "kindCounts": {
        "imports": 0,
        "calls": 2,
        "references": 0,
        "extends": 0,
        "implements": 0,
        "exports": 0
      }
    },
    "schema": "type ArtefactSelection { ... }"
  },
  "tests": {
    "summary": {
      "selectedArtefactCount": 2,
      "matchedArtefactCount": 2,
      "totalCoveringTests": 2,
      "crossCuttingArtefactCount": 0,
      "diagnosticCount": 0,
      "dataSources": ["static_analysis"]
    },
    "schema": "type ArtefactSelection { ... }"
  }
}
```

Notes:

- `summary` is stage-owned JSON, not stringified JSON
- `schema` is `null` when that stage has no results
- `schema` is included in the aggregate response so an agent can discover the drill-down surface without re-querying first

## Stage Results

Each category field returns a typed stage result object.

```graphql
type CheckpointStageResult {
  summary: JSON!
  schema: String
  items(first: Int! = 20): [Checkpoint!]!
}
```

The other stage result types follow the same pattern:

- `CloneStageResult`
- `DependencyStageResult`
- `TestsStageResult`

Use them like this:

```graphql
{
  selectArtefacts(by: { path: "rust-app/src/main.rs" }) {
    deps {
      summary
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

This is the normal escalation path:

1. Ask for `summary`
2. Decide which category matters
3. Read `schema` only if needed
4. Query `items(first: ...)` for typed detail rows

## Agent Hook Guidance

When Bitloops-managed integrations are installed for supported agents, Bitloops injects a short DevQL reminder at the supported bootstrap and pre-turn surfaces. This currently includes Claude Code, Codex, Gemini CLI, Copilot CLI, Cursor, and OpenCode via its repo-local plugin path. That reminder follows the same workflow documented here:

1. Start with `selectArtefacts(by: ...) { summary }`
2. Read stage `schema` only when the summary says a drill-down is worth it
3. Query `items(first: ...)` on the relevant stage for typed rows
4. Use `bitloops devql schema` or `bitloops devql schema --global` when the full SDL is needed

The injected reminder is guidance only. It does not execute DevQL automatically or attach live query results to the turn.

## Category Summaries

Current category coverage:

| Category | Summary intent | Detail type |
|---|---|---|
| `checkpoints` | Count, latest timestamp, participating agents | `Checkpoint` |
| `clones` | Total count, grouped relation kinds, max score | `Clone` |
| `deps` | Unique edge counts, direction counts, edge-kind counts | `DependencyEdge` |
| `tests` | Matched artefact count, total covering tests, diagnostics, data sources | `TestHarnessTestsResult` |

## Stage Arguments

### `checkpoints`

```graphql
checkpoints(agent: String, since: DateTime)
```

### `clones`

```graphql
clones(relationKind: String, minScore: Float)
```

### `deps`

```graphql
deps(kind: EdgeKind, direction: DepsDirection! = BOTH, includeUnresolved: Boolean! = false)
```

Defaults for selection stages:

- `direction = BOTH`
- `includeUnresolved = false`

### `tests`

```graphql
tests(minConfidence: Float, linkageSource: String)
```

## DevQL DSL Form

The DevQL DSL supports `selectArtefacts(...)` with flat selector args:

```text
selectArtefacts(symbol_fqn:"rust-app/src/main.rs::main")->checkpoints()
selectArtefacts(fuzzy_name:"payLater()")->checkpoints()
selectArtefacts(path:"rust-app/src/main.rs",lines:6..10)->deps()
selectArtefacts(path:"rust-app/src/main.rs")->tests(min_confidence:0.8)
```

Current DSL limitations:

- compiles only against the slim endpoint
- supports one explicit terminal stage at a time
- defaults to selecting `summary`
- `->select(summary,schema)` is supported for the chosen stage
- aggregate `selectArtefacts { summary }` is not yet available through the DSL

## Recommended Agent Flow

For tool-using agents, the lowest-error pattern is:

1. Resolve the artefact set with `selectArtefacts(by: ...)`
2. Ask for aggregate `summary`
3. Choose the category worth expanding
4. Query that category’s `items(first: ...)`

That keeps the default call compact while still allowing full typed detail on demand.
