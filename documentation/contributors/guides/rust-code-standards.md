---
sidebar_position: 4
title: Rust Code Standards
description: Bitloops Rust code standards for contributors and AI reviewers.
---

# Rust Code Standards

This document defines the Rust coding standards that apply to Bitloops Rust code.

It is intended for:
- human contributors writing or reviewing Rust code
- AI coding agents generating Rust code
- AI review agents checking pull requests for standards compliance

These rules are intentionally narrow and explicit.
Only enforce what is written here.
Do not infer additional policy unless another Bitloops document states it clearly.

---

## Purpose

These standards exist to keep Rust code:

- idiomatic
- consistent
- easy to review
- easy to maintain

When reviewing code, focus on compliance with the standards in this document.
Do not broaden the review into architecture, performance, security, or product behavior unless explicitly asked.

---

## Naming Standards

### Use `snake_case` for

- functions
- methods
- modules
- files
- variables
- tests

### Use `UpperCamelCase` for

- structs
- enums
- traits
- type aliases

### Use `SCREAMING_SNAKE_CASE` for

- constants
- statics
- environment variable keys

---

## `#[allow(...)]` usage

Treat `#[allow(...)]` as disallowed by default.

It may be used only when all of the following are true:

- it is narrowly scoped
- it is necessary in that exact location
- it has a concrete inline justification

Acceptable justification must explain the exact reason the allow is needed.

Not acceptable:

- vague comments
- "temporary"
- "needed for now"
- "fix later"

If an `#[allow(...)]` can be removed by renaming or restructuring the code, prefer that.

---

## Review Rules

When reviewing Rust code for standards compliance:

1. inspect the provided code or changed code only
2. check each relevant symbol against the standards in this document
3. report only concrete violations
4. avoid speculative or stylistic comments outside the written rules
5. separate required fixes from optional suggestions

If code is compliant, say so clearly.

---

## Reviewer Output Format

Whether the reviewer is human or AI, findings should use this structure.

### Verdict

One of:

- pass
- pass with comments
- changes requested

### Findings

For each finding include:

- severity: `must_fix` or `should_fix`
- rule: the violated rule
- location: file, symbol, or area
- explanation: concise and specific
- suggested fix: exact preferred form when possible

### Summary

End with:

- what is compliant
- what must change before approval
- whether the findings are purely standards-related

---

## Examples

### Example: non-idiomatic test name

Bad:

```rust
#[test]
fn TestRootCommand_ParseVersionFlag() {}
```

Preferred:

```rust
#[test]
fn root_command_parses_version_flag() {}
```

Reason:
tests must use `snake_case`.

---

### Example: non-standard constant name

Bad:

```rust
const defaultPort: u16 = 3000;
```

Preferred:

```rust
const DEFAULT_PORT: u16 = 3000;
```

Reason:
constants must use `SCREAMING_SNAKE_CASE`.

---

### Example: unjustified allow

Bad:

```rust
#[allow(non_snake_case)]
fn TestSomething() {}
```

Preferred:

```rust
#[test]
fn test_something() {}
```

Reason:
`#[allow(...)]` is not acceptable here because the standards violation should be fixed directly rather than suppressed.

---

## Scope and Extension

This document currently defines only the Rust standards listed above.

If Bitloops adds more Rust standards later, add them explicitly here.

Do not treat unwritten preferences as mandatory review rules.
