---
name: using-devql
description: >
  Use when answering repo-understanding questions in a Bitloops repo with DevQL
  guidance enabled, especially when you need to locate code by path, line
  range, exact symbol, or approximate/conceptual search before reading files.
---

# Using DevQL

## Overview

DevQL is Bitloops's typed repo-intelligence surface. In repos where this
guidance is enabled, use DevQL first for repo-understanding questions when it
is available in the current session. If DevQL returns nothing useful or is not
available, fall back to targeted repo search or file reads.

## When to Use

- understanding what a file, function, module, class, or symbol does
- resolving the concrete artefacts matched by a path or line range
- looking up an exact symbol with `symbolFqn`
- looking up an approximate name or conceptual behavior with `search`
- answering architecture questions after selecting a concrete area

## Choosing The `by` Selector

- `path`: use this when the starting point is one file; Bitloops selects
  artefacts from that file.
- `path + lines`: use this when the starting point is a specific region inside
  a file; Bitloops limits the seed artefacts to that line range.
- `symbolFqn`: use this when the starting point is one exact artefact or symbol.
- `search`: use this when the request is approximate, misspelled, or
  conceptual and you do not yet have an exact seed.

## Process

1. Choose the most specific selector available: `path`, `path + lines`, or
   `symbolFqn` when the request is exact; `search` when the request is
   approximate or conceptual.
2. If the request is approximate or conceptual, distill it to a short phrase
   or symbol hint and run `search` first. Inspect `artefacts(first: 10)` to
   see what matched.
3. Once you have a concrete file or artefact, run `overview`.
4. If the response includes `expandHint`, `schema`, or another typed follow-up
   hint, read that hint before composing the next query.
5. Expand only one relevant stage or `artefacts(first: ...)` if the overview
   shows that more detail is needed.
6. If DevQL returns nothing useful, fall back to targeted repo search or file
   reads.

## Query Templates

```bash
# Search first when the request is approximate or conceptual
bitloops devql query '{ selectArtefacts(by: { search: "<distilled phrase or approximate symbol>" }) { count artefacts(first: 10) { path symbolFqn canonicalKind startLine endLine } } }'

# Ask for overview once the selection is concrete
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { overview } }'

# Same overview shape for file or file+line selectors
bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>", lines: { start: <start>, end: <end> } }) { overview } }'
```

## Reading Response Hints

- Treat `overview` as the compact first pass.
- When DevQL returns `expandHint`, `schema`, or another typed follow-up hint,
  read it before composing the next query.
- Prefer the hint's suggested parameters and drill-down shape over guessing
  the next stage call.

## Sandbox Execution

- In sandboxed agent environments, run `bitloops devql ...` outside the sandbox by default.
- These commands rely on Bitloops-managed daemon and runtime state under platform app directories, so they can fail inside a workspace-only sandbox even when DevQL is healthy.
- If your platform requires approval or escalation for out-of-sandbox commands, request it immediately before running `bitloops devql ...`.

## Common Mistakes

- Do not reintroduce obsolete selector names from older DevQL guidance.
- Do not dump the full conversational prompt into `search`; distill it.
- Do not ignore `expandHint` or other response hints when DevQL already tells
  you how to drill down.
- Do not start with stage-specific detail queries before selecting a concrete
  artefact.
- Do not use DevQL for edits, builds, tests, formatting, or git operations.

## Integration

- If `overview` shows something relevant, expand only the one stage needed for
  the current question.
- If DevQL is unavailable or empty, switch to targeted `rg` or file reads.
