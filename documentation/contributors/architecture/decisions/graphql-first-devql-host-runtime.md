# ADR: GraphQL-first DevQL with host-owned capability isolation

## Status

Accepted

## Context

Bitloops has three architectural goals that need to hold at the same time:

1. GraphQL is the canonical DevQL product contract for the CLI, dashboard, and third-party clients.
2. Capability packs must be isolated from infrastructure details such as store selection, process execution, and cross-pack wiring.
3. Language-specific semantics should be reusable across features rather than reimplemented inside each capability pack.

Earlier iterations exposed a generic public `extension(stage: ...)` GraphQL field and allowed some capability packs to own more of their storage or language-specific runtime than intended.

## Decision

We standardise on the following decisions.

### 1. GraphQL stays canonical

DevQL DSL compiles to GraphQL and executes through the `/devql` endpoints.

GraphQL is not optional or legacy in this architecture. It is the public contract.

### 2. The host owns execution beneath GraphQL

Capability packs execute only through host-owned contexts and gateways.

The host is responsible for:

- stage and ingester dispatch
- storage attachment and backend selection
- connector access
- provenance
- language-service resolution

### 3. Public capability-pack GraphQL access is typed

The public `extension(stage: ...)` field is removed.

Capability behaviour must be exposed through typed GraphQL fields such as:

- `tests`
- `coverage`
- `testsSummary`

The host remains free to use stages and ingesters internally as execution seams.

### 4. Language adapters own reusable language semantics

Reusable language-specific concerns belong in the language-adapter runtime.

The first shared facet is `LanguageTestSupport`, which allows `test_harness` to consume:

- test discovery
- runtime enumeration
- reconciliation

without maintaining a separate pack-owned language registry as the execution surface.

## Consequences

### Positive

- pack isolation is clearer and easier to enforce
- GraphQL becomes a more stable product contract
- language-specific logic is centralised
- `test_harness` no longer needs to bootstrap its own repository or choose its own language parsers at runtime

### Trade-offs

- the GraphQL layer needs explicit typed fields for pack behaviour instead of a generic escape hatch
- host contracts become more important and must be curated carefully
- some internal migration scaffolding remains while older pack-local language code is moved fully behind the adapter facet

## Implementation status

Implemented in this repository:

- `test_harness` repository ownership moved to the host context
- `LanguageServicesGateway` exposed through capability contexts
- `LanguageTestSupport` added to the language-adapter runtime
- Rust, TypeScript/JavaScript, and Python test support exposed through adapter-side wrappers
- `semantic_clones` moved to `clone_edges_rebuild_relational()`
- public GraphQL `extension(stage: ...)` removed
- typed GraphQL `testsSummary` added for project and slim scopes

Still to complete in later slices:

- host-owned runtime GraphQL contribution registration
- host-owned DevQL DSL contribution registration
- full runtime schema stitching from capability contributions
- complete removal of transitional pack-local test-language scaffolding
