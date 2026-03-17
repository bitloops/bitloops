# TestLens User Stories Useful For Ruff F523 Task

Last updated: 2026-03-16

Task brief:
- Confluence page `440238084`
- Title: `Rust Task Brief 1: astral-sh__ruff-15309 (F523 empty format fix)`
- Base commit: `75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`

This note answers a narrow question:

- which TestLens user stories are actually useful for solving the Ruff F523 task right now
- which ones are only partially useful
- which ones are not yet useful on this workspace

Precondition:
- run the Ruff quickstart in [quickstart_ruff_fixture.md](/Users/markos/code/bitloops/bitloops/TestLens/docs/quickstart_ruff_fixture.md)
- the commands below assume the DB is `./target/ruff-real-project.db`

## Most Useful Right Now

### 1. Scenario 1 / 10: pre-change safety assessment on the F523 rule

User story:
- As an agent preparing the fix, I need to know whether the main F523 rule artefact already has meaningful linked tests before I edit it.

Command:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact string_dot_format_extra_positional_arguments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary
```

Current result:
- `verification_level = partially_tested`
- `total_covering_tests = 2`

Why this helps on this task:
- It tells you the rule-level entry point is not a blind edit.
- It confirms there is already harness coverage around the exact F523 rule logic.
- It supports the task-brief advice to patch the fixer while preserving the existing F523 rule gate.

### 2. Scenario 2 / 8: discover the concrete tests and local test style for F523

User story:
- As an agent about to change F523, I need to know which exact tests appear to cover the rule and what naming/style the Ruff harness uses.

Command:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact string_dot_format_extra_positional_arguments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests \
  --min-strength 0.0
```

Current result:
- `rules[StringDotFormatExtraPositionalArguments, F523.py]`
- `StringDotFormatExtraPositionalArguments[doctest:416]`

Why this helps on this task:
- It points you directly at the Ruff rule harness file: `crates/ruff_linter/src/rules/pyflakes/mod.rs`
- It shows the core fixture identity: `F523.py`
- It reveals that the rule also has a nearby doctest, which is useful context when changing the rule text or examples
- It gives the local naming convention you should preserve when extending the fixture/snapshot flow

### 3. Neighbor-rule story: inspect the adjacent F522 path for regression context

User story:
- As an agent changing shared `.format(...)` logic, I need to inspect the neighboring rule in the same ecosystem so I do not regress adjacent behavior.

Commands:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact string_dot_format_extra_named_arguments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary

testlens query \
  --db ./target/ruff-real-project.db \
  --artefact string_dot_format_extra_named_arguments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests \
  --min-strength 0.0
```

Current result:
- `string_dot_format_extra_named_arguments` is also `partially_tested`
- linked tests include `rules[StringDotFormatExtraNamedArguments, F522.py]`

Why this helps on this task:
- The task brief explicitly calls out the shared F522/F523/F524/F525 dispatch path.
- This query gives a fast way to inspect the nearest sibling rule that already uses the same Ruff harness pattern.
- It is useful for choosing non-regression checks after the fixer change.

## Useful, But With Important Limits

### 4. Gap-detection story: find where TestLens stops helping on helper-level attribution

User story:
- As an agent, I need to know whether TestLens can trace tests all the way down to the helper functions named in the task brief, or whether I need manual blast-radius reasoning there.

Commands:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact remove_unused_positional_arguments_from_format_call \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary

testlens query \
  --db ./target/ruff-real-project.db \
  --artefact transform_expression \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary

testlens query \
  --db ./target/ruff-real-project.db \
  --artefact match_call_mut \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary

testlens query \
  --db ./target/ruff-real-project.db \
  --artefact FormatSummary \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary

testlens query \
  --db ./target/ruff-real-project.db \
  --artefact expression \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary
```

Current result:
- all of the above currently return `untested`

Why this is still useful:
- It tells you not to over-trust the current static linkage for helper-level blast radius.
- It separates a real tool limit from a false sense of safety.
- For this task, it means TestLens is strong at the rule-harness layer but still weak for the deeper fixer / CST helper / dispatcher chain.

Practical implication:
- use TestLens to find the correct harness and neighboring rule tests
- do not use TestLens alone to conclude that helper-level functions are untested or safe to ignore

## Not Very Useful Yet For This Task

### 5. Coverage-driven branch hunting

Why not:
- the Ruff workspace does not yet have a validated TestLens coverage quickstart
- this task is already strongly fixture/snapshot-driven in the brief
- even with workspace LCOV ingested (`--scope workspace`), coverage data produces artefact-level stats only — it does not inflate per-test confidence. Tests remain `evidence: "static_only"` until isolated per-test captures (LLVM JSON with `--scope test-scenario`) are ingested.

Meaning:
- `coverage()` is not the main TestLens story for this task today
- when coverage is eventually ingested, the `coverage_mode` field on query output will honestly report `"artefact_only"` for workspace LCOV, making the limitation transparent to agents

### 6. Pre-existing failing test detection

Why not:
- run outcomes remain deferred in TestLens outside the current Jest-oriented flow

Meaning:
- TestLens is not yet the right tool for answering "which Ruff tests already fail before my patch?"

## Recommended TestLens Workflow For This Task

If you are solving the F523 task with today’s Ruff/TestLens support, the highest-value sequence is:

1. Query `string_dot_format_extra_positional_arguments` with `--view summary`
2. Query `string_dot_format_extra_positional_arguments` with `--view tests --min-strength 0.0`
3. Query `string_dot_format_extra_named_arguments` the same way for neighboring-rule context
4. Treat helper-level queries like `remove_unused_positional_arguments_from_format_call` as residual-gap indicators, not as proof that no tests exist

## Bottom Line

The TestLens user stories that are genuinely useful for this Ruff F523 task right now are:

- pre-change safety assessment at the rule-function level
- discovering the exact Ruff harness tests and local test style for F523
- inspecting neighboring rule coverage to guide regression checks
- detecting where current TestLens attribution stops, so you know when to switch to manual reasoning

The most important limitation is that TestLens currently helps at the rule-harness layer, not yet at the full helper-level blast-radius layer for this task.
