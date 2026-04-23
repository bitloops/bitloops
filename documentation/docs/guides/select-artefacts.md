---
title: selectArtefacts
---

# selectArtefacts

`selectArtefacts(by: ...)` is the slim, repo-scoped DevQL GraphQL selector for agent-oriented code analysis.

It lets you:

- select a current set of artefacts once
- ask for one aggregate `overview` across all supported categories
- drill into one category only when that overview suggests it is worth the tokens

This is the intended shape for tool-using agents.

## Availability

`selectArtefacts(by: ...)` is available on the slim GraphQL surface only.

- Raw GraphQL: supported
- Slim `/devql` SDL and Explorer: supported
- DevQL DSL compiler: supported for one explicit terminal stage at a time
- DevQL DSL aggregate `selectArtefacts(...)->overview()`: not supported yet
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

### By `search`

```graphql
{
  selectArtefacts(by: { search: "payLater()" }) {
    count
    artefacts {
      path
      symbolFqn
    }
  }
}
```

`search` runs two internal lanes and returns one flat artefact list:

- up to 5 fuzzy symbol-name matches, including typo-tolerant requests such as `payLater()` or `payLatr()`
- up to 5 embedding-backed conceptual matches across identity, code, and summary representations

Fuzzy hits are returned first, embedding-only hits follow, weak matches are dropped, and `score` remains optional debug output rather than part of the default contract.

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

- `symbolFqn` cannot be combined with `search`, `path`, or `lines`
- `search` cannot be combined with `symbolFqn`, `path`, or `lines`
- `search` must be non-empty
- `lines` requires `path`
- empty selectors are rejected
- selector paths are resolved relative to the slim request scope, including project-scoped slim requests
- selection is current-state only in v1; `asOf(...)` is not part of this surface

## Selection Shape

`selectArtefacts(by: ...)` returns one `ArtefactSelection` object representing the selected set.

```graphql
type ArtefactSelection {
  count: Int!
  overview: JSON!
  artefacts(first: Int! = 20): [Artefact!]!
  checkpoints(agent: String, since: DateTime): CheckpointStageResult!
  dependencies(kind: EdgeKind, direction: DepsDirection! = BOTH, includeUnresolved: Boolean! = true): DependencyStageResult!
  codeMatches(relationKind: String, minScore: Float): CloneStageResult!
  tests(minConfidence: Float, linkageSource: String): TestsStageResult!
}
```

Use:

- `count` to know how many artefacts matched
- `artefacts(...)` when you want to inspect the matched set itself
- `overview` when you want one compact answer across all supported categories
- stage fields when you want one category in more detail

## Aggregate `overview`

The selection-level `overview` field returns a JSON object with one entry per supported category.

```graphql
{
  selectArtefacts(by: { path: "rust-app/src/main.rs", lines: { start: 6, end: 10 } }) {
    overview
  }
}
```

Representative shape:

```json
{
  "selectedArtefactCount": 2,
  "checkpoints": {
    "overview": {
      "totalCount": 0,
      "latestAt": null,
      "agents": []
    },
    "schema": null
  },
  "codeMatches": {
    "overview": {
      "counts": {
        "total": 2,
        "similar_implementation": 2
      },
      "expandHint": {
        "intent": "Inspect code matches",
        "template": "bitloops devql query '{ selectArtefacts(by: ...) { codeMatches(relationKind: <KIND>) { items(first: 20) { ... } } } }'",
        "parameters": {
          "kind": {
            "intent": "Choose which relation kind to inspect",
            "supportedValues": [
              "exact_duplicate",
              "similar_implementation",
              "shared_logic_candidate",
              "diverged_implementation",
              "weak_clone_candidate"
            ]
          }
        }
      }
    },
    "schema": "type ArtefactSelection { ... }"
  },
  "dependencies": {
    "overview": {
      "dependencies": {
        "selectedArtefact": 2,
        "total": 2,
        "incoming": 0,
        "outgoing": 2,
        "kindCounts": {
          "calls": 2,
          "exports": 0,
          "extends": 0,
          "implements": 0,
          "imports": 0,
          "references": 0
        }
      }
    },
    "expandHint": {
      "intent": "Use direction to filter dependencies by flow relative to the selected artefacts: incoming maps to IN and outgoing maps to OUT. Use kind to filter dependencies by relationship type: kindCounts.calls maps to CALLS, kindCounts.imports maps to IMPORTS and so on.",
      "template": "Direction example: bitloops devql query '{ selectArtefacts(...) { dependencies(direction: IN) { items(first: 50) { edgeKind fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'\nKind example: bitloops devql query '{ selectArtefacts(...) { dependencies(kind: CALLS) { items(first: 50) { edgeKind fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'\nCombined example: bitloops devql query '{ selectArtefacts(...) { dependencies(direction: IN, kind: CALLS) { items(first: 50) { edgeKind fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'",
      "parameters": {
        "direction": ["IN", "OUT"],
        "kind": ["CALLS", "EXPORTS", "EXTENDS", "IMPLEMENTS", "IMPORTS", "REFERENCES"]
      }
    },
    "schema": "type ArtefactSelection { ... }"
  },
  "tests": {
    "overview": {
      "selectedArtefactCount": 2,
      "matchedArtefactCount": 2,
      "totalCoveringTests": 2,
      "expandHint": {
        "intent": "Inspect concrete covering tests for selected artefacts",
        "template": "bitloops devql query '{ selectArtefacts(by: { symbolFqn: \"<symbol-fqn>\" }) { tests { overview items(first: 20) { coveringTests { testName suiteName filePath startLine endLine } } } } }'"
      }
    },
    "schema": "type ArtefactSelection { ... }"
  }
}
```

Notes:

- `overview` is stage-owned JSON, not stringified JSON
- `schema` is `null` when that stage has no results
- `schema` is included in the aggregate response so an agent can discover the drill-down surface without re-querying first
- `tests.overview.expandHint` is always included when tests overview is requested and points to the concrete `coveringTests` drill-down query
- `overview.dependencies.expandHint` maps the dependency buckets back to concrete `dependencies(direction:..., kind:...)` follow-up queries
- `overview.dependencies.expandHint` is omitted when no dependencies match the selected artefacts
- `codeMatches` overviews always include `counts.total`
- `expandHint` is omitted when `counts.total` is `0`

## Stage Results

Each category field returns a typed stage result object.

```graphql
type CheckpointStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [Checkpoint!]!
}
```

The other stage result types follow the same pattern:

- `CloneStageResult`
- `DependencyStageResult`
- `TestsStageResult`

`DependencyStageResult` also exposes a typed `expandHint` field:

```graphql
type DependencyStageResult {
  overview: JSON!
  expandHint: DependencyExpandHint
  schema: String
  items(first: Int! = 20): [DependencyEdge!]!
}
```

Use them like this:

```graphql
{
  selectArtefacts(by: { path: "rust-app/src/main.rs" }) {
    dependencies {
      overview
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

This is the normal escalation path:

1. Ask for `overview`
2. Decide which category matters
3. For `dependencies`, use the typed stage field `expandHint`, or the aggregate `overview.dependencies.expandHint` when you are still in overview-only mode
4. Read `schema` only if needed
5. Query `items(first: ...)` for typed detail rows

## Agent Hook Guidance

Bitloops now treats the DevQL hook as skill-gated. When the Bitloops-managed `using-devql` skill is enabled for an agent, Bitloops installs the repo-local DevQL surface and emits direct startup guidance for that surface. When the skill is disabled, Bitloops emits no DevQL guidance at all.

The current enforcement contract is:

- Claude Code and Codex regain targeted prompt-time reinforcement in addition to the repo-local surface
- Cursor remains session-start plus rule-based
- other supported agents follow the same repo-local surface contract when their skill is enabled

The guidance follows the same workflow documented here:

1. Start with `selectArtefacts(by: ...) { overview }`
2. Read stage `schema` only when the overview says a drill-down is worth it
3. Query `items(first: ...)` on the relevant stage for typed rows
4. Use `bitloops devql schema` or `bitloops devql schema --global` when the full SDL is needed

This guidance is guidance only. It does not execute DevQL automatically or attach live query results to the turn.

## Category Overviews

Current category coverage:

| Category | Overview intent | Detail type |
|---|---|---|
| `checkpoints` | Count, latest timestamp, participating agents | `Checkpoint` |
| `codeMatches` | Total count, grouped relation kinds, max score | `Clone` |
| `tests` | Matched artefact count, total covering tests, drill-down hint | `TestHarnessTestsResult` |
| `dependencies` | Nested counts plus a default drill-down hint for `direction` / `kind` follow-up queries | `DependencyEdge` |

## Stage Arguments

### `checkpoints`

```graphql
checkpoints(agent: String, since: DateTime)
```

### `codeMatches`

```graphql
codeMatches(relationKind: String, minScore: Float)
```

### `dependencies`

```graphql
dependencies(kind: EdgeKind, direction: DepsDirection! = BOTH, includeUnresolved: Boolean! = false)
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
selectArtefacts(search:"payLater()")->checkpoints()
selectArtefacts(path:"rust-app/src/main.rs",lines:6..10)->deps()
selectArtefacts(path:"rust-app/src/main.rs")->tests(min_confidence:0.8)
```

Current DSL limitations:

- compiles only against the slim endpoint
- supports one explicit terminal stage at a time
- defaults to selecting `overview`
- `->select(overview,schema)` is supported for the chosen stage
- the DSL stage name remains `deps()`, but it compiles to the raw GraphQL `dependencies(...)` field on `selectArtefacts`
- aggregate `selectArtefacts { overview }` is not yet available through the DSL

## Recommended Agent Flow

For tool-using agents, the lowest-error pattern is:

1. Resolve the artefact set with `selectArtefacts(by: ...)`
2. Ask for aggregate `overview`
3. Choose the category worth expanding
4. Query that category’s `items(first: ...)`

That keeps the default call compact while still allowing full typed detail on demand.
