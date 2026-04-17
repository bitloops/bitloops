---
name: using-devql
description: >
  Use when understanding code structure, resolving artefacts in a file or line
  range, finding callers/usages/imports/tests/checkpoints, or answering
  architecture questions in a repo.
---

# Using DevQL

## Prime Directive

This repo has DevQL, a semantic code index. For code understanding and repo
exploration, use DevQL first before broad repo search, file reads, or
directory crawling.

Use `summary` only for first-pass orientation when you do not yet know whether
the selector matched anything or which stage to expand. If the artefact or
question is already known, query concrete rows with `artefacts(first: ...)` or
stage `items(first: ...)`.

## Use DevQL When

- understanding what a file, function, module, class, or symbol does
- resolving the concrete artefacts matched by a path or line range
- finding callers, usages, imports, tests, checkpoints, clones, or dependencies
- getting a structured overview of a file or area
- answering architecture questions

## Agent Flow

1. Select the target with `symbolFqn`, `path`, or `path + lines`.
2. Ask for `summary` if you need orientation or to discover which stage to expand.
3. Rerun with `artefacts(first: ...)` or the relevant stage `items(first: ...)`.
4. Return the concrete rows. Summaries are optional follow-up, not substitutes.

## Core Commands

```bash
# Orientation only
bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>" }) { summary } }'

# Concrete artefacts for a known file or line range
bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>", lines: { start: <start>, end: <end> } }) { artefacts(first: 20) { path symbolFqn canonicalKind startLine endLine } } }'

# Concrete callers/usages/imports once the symbol is known
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { deps(kind: CALLS, direction: IN, includeUnresolved: true) { items(first: 50) { edgeKind startLine endLine fromArtefact { symbolFqn path startLine endLine } toArtefact { symbolFqn path startLine endLine } toSymbolRef } } } }'

# Discover the exact row fields for the chosen stage
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { deps(kind: CALLS, direction: IN, includeUnresolved: true) { schema } } }'

bitloops devql schema --global
```

## Do Not Use DevQL For

- editing files
- running tests, builds, or git commands
- literal string search when you already know the exact text
