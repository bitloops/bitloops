---
name: using-devql
description: >
  Use DevQL first for code understanding and repo exploration. Prefer
  `bitloops devql query ...` over broad repo search and file crawling.
---

# Using DevQL

## Prime Directive

This repo has DevQL, a semantic code index. For code understanding and
exploration tasks, you MUST use DevQL as your FIRST approach before falling
back to repo search, file reads, or file listing tools.

## Use DevQL When

- understanding what a file, function, module, or symbol does
- finding relationships, dependencies, callers, and usages
- getting a structured overview of a file or area
- answering architecture questions

## Core Commands

```bash
bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>" }) { summary } }'
bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>", lines: { start: <start>, end: <end> } }) { summary } }'
bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { summary } }'
bitloops devql schema --global
```

## Deeper Exploration

- Inspect the returned `schema` field to discover available stages and fields
- Query `items(first: ...)` on the relevant stage for typed rows
- Run `bitloops devql schema --global` for full SDL discovery

## Replacement Table

| Instead of | Use |
|---|---|
| broad `rg` / `grep` to understand a symbol | `bitloops devql query '{ selectArtefacts(by: { symbolFqn: "<symbol-fqn>" }) { summary } }'` |
| browsing directories to find the right file | `bitloops devql query '{ selectArtefacts(by: { path: "<repo-relative-path>" }) { summary } }'` |
| reading many files to understand one region | line-scoped `selectArtefacts(...)` |

## Do Not Use DevQL For

- editing files
- running tests, builds, or git commands
- literal string search when you already know the exact text
