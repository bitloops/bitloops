# DevQL Token Efficiency Design

**Date:** 2026-04-28
**Status:** Draft design
**Branch:** `improve-bitloops-efficiency-for-tokens`

## Problem

In the `tokio-rs__tokio-6551` SWE-bench run, the Bitloops-enabled condition solved the task but used materially more model context than the baseline condition.

Observed averages across the available local traces:

| Condition | Runs | Avg total processed tokens | Avg cache-read input tokens | Avg input tokens | Avg output tokens |
| --- | ---: | ---: | ---: | ---: | ---: |
| Baseline | 3 | 355k | 332k | 19.2k | 4.2k |
| With Bitloops | 4 | 901k | 851k | 43.9k | 6.4k |

The Bitloops condition used about 2.5x the total processed tokens. The delta is mostly cached input replay rather than visible assistant output: average cache-read input increased by about 519k tokens, while output increased by only about 2.2k tokens.

## Evidence From The Trace

The token increase aligns with a few early, large tool outputs that stay in the conversation and are replayed on later steps.

Representative Bitloops attempts contained these high-volume calls:

| Attempt | Call | Tool action | Output size |
| --- | ---: | --- | ---: |
| 1 | 6 | Full `tokio/src/runtime/metrics/runtime.rs` read | ~40k chars |
| 1 | 10 | Full `tokio/src/runtime/blocking/pool.rs` read | ~22.6k chars |
| 2 | 6 | Full `tokio/src/runtime/metrics/runtime.rs` read | ~40k chars |
| 2 | 10 | Full `tokio/src/runtime/blocking/pool.rs` read | ~22.6k chars |
| 3 | 3 | Full `tokio/src/runtime/metrics/runtime.rs` read | ~40k chars |
| 3 | 6 | Full `tokio/tests/rt_metrics.rs` read | ~23.4k chars |
| 3 | 8 | Full `tokio/src/runtime/blocking/pool.rs` read | ~22.6k chars |

The initial DevQL search already returned `path`, `symbolFqn`, `canonicalKind`, `startLine`, and `endLine`, but the agent still followed with whole-file `Read` calls. This means simply exposing line ranges is not enough. The agent workflow needs to prefer those ranges and avoid full reads unless the bounded context is insufficient.

The `devql_calls_num` metric in the CSV appears to be mislabeled. It matched total opencode tool calls in the inspected traces, not actual `bitloops devql query` invocations. Actual DevQL shell queries were lower, around 4 to 8 per Bitloops attempt.

## Root Cause

Bitloops changes the agent's exploration path:

1. The agent loads the `using-devql` skill, adding about 6k chars to context.
2. The agent runs DevQL searches and overview queries, adding structured but sometimes verbose JSON.
3. DevQL successfully points the agent to relevant artefacts.
4. The agent often reads entire files instead of using the returned line ranges.
5. Those large reads happen early, so each later model step reprocesses the enlarged transcript as cached input.

The largest avoidable cost is therefore not the DevQL query count itself. It is the combination of broad follow-up reads and repeated context replay.

## Goals

- Reduce total processed tokens for Bitloops-enabled benchmark runs without lowering solve rate.
- Preserve DevQL's usefulness for locating relevant code quickly.
- Prefer small, reversible experiments that can be evaluated on the existing benchmark harness.

## Non-Goals

- Do not remove DevQL from the benchmark condition.
- Do not optimize by hiding useful failure output from the agent when it is needed for debugging.
- Do not make benchmark-only changes that would degrade normal developer use.
- Do not change benchmark-harness metric reporting in this repo. Separating DevQL calls from total tool calls belongs with the benchmark tooling.

## Design Options

### Option A: Agent Guidance And Skill Tightening

Update the Bitloops DevQL skill and benchmark prompt guidance to require bounded reads after DevQL returns line ranges.

Suggested policy:

- After `selectArtefacts` returns `startLine` and `endLine`, read only that region plus a small context window.
- Default context window: 20 lines before and 40 lines after for functions and methods.
- Escalate to a full file read only when the bounded read does not explain the relationship or edit location.
- Prefer one bounded read per selected artefact before running additional broad searches.

Advantages:

- Fastest to try.
- Low implementation risk.
- Directly targets the observed failure mode.

Limitations:

- Relies on model compliance.
- Agents may still choose full reads when they are uncertain.
- Different agents may interpret the guidance differently.

### Option B: Compact DevQL Responses

Add or expose compact response shapes for common agent workflows.

Possible query surfaces:

```graphql
selectArtefacts(by: { search: "num_blocking_threads", searchMode: LEXICAL }) {
  count
  artefacts(first: 10) {
    path
    symbolFqn
    canonicalKind
    startLine
    endLine
    summary
  }
}
```

For `overview`, provide a compact mode that omits schemas and verbose expand hints unless requested:

```graphql
selectArtefacts(by: { symbolFqn: "..." }) {
  overview(compact: true)
}
```

Advantages:

- Reduces DevQL's own output cost.
- Makes the default agent path easier to keep small.
- Keeps detailed schemas available as an explicit follow-up.

Limitations:

- Does not prevent full-file reads by itself.
- Requires API/schema or formatter work if compact output is not already available.

### Option C: Read-Oriented DevQL Results

Make DevQL return explicit read hints or snippets that can replace immediate follow-up file reads.

Example result fields:

```graphql
artefacts(first: 10) {
  path
  symbolFqn
  canonicalKind
  startLine
  endLine
  readHint {
    startLine
    endLine
    reason
  }
}
```

Or a snippet-focused query:

```graphql
selectArtefacts(by: { search: "num_blocking_threads", searchMode: LEXICAL }) {
  snippets(first: 5, contextBefore: 20, contextAfter: 40) {
    path
    startLine
    endLine
    text
  }
}
```

Advantages:

- Strongly nudges the agent away from full-file reads.
- Keeps the search and first code inspection in one compact response.
- Lets Bitloops own the default context window instead of relying on agent inference.

Limitations:

- More product/API design required.
- Snippets must be sized carefully to avoid replacing one large output with another.
- Needs language-adapter confidence that artefact ranges are accurate.

## Recommended Experiment Sequence

### Experiment 1: Prompt And Skill Policy Only

Change the DevQL skill guidance to say:

> When DevQL returns `path`, `startLine`, and `endLine`, use bounded reads around those ranges before any full-file read. Full-file reads should be the exception and should follow a failed bounded inspection.

Run the same `tokio-rs__tokio-6551` Bitloops condition with the same model and seed.

Success criteria:

- Full-file reads after DevQL decrease by at least 50 percent.
- Total processed tokens drop materially, target 30 percent or better.
- Solve result remains unchanged.

### Experiment 2: Compact Overview Output

Add a compact DevQL overview shape or CLI output option that suppresses schema blocks and verbose expand hints by default.

Run the same benchmark after Experiment 1 so the effects can be separated.

Success criteria:

- DevQL query output chars decrease by at least 40 percent.
- No increase in follow-up searches caused by missing detail.
- Solve result remains unchanged.

### Experiment 3: Read Hints Or Snippets

Expose `readHint` or `snippets` in DevQL results and update guidance to use those as the default inspection path.

Success criteria:

- The agent can complete the task with zero or near-zero full-file reads.
- Total processed tokens approach the baseline range while preserving the faster code-location benefit of DevQL.

## Measurement Plan

For each attempt, collect:

- Total processed tokens.
- Input, output, cache-read, and cache-creation tokens.
- Number and size of DevQL outputs.
- Number and size of full-file reads.
- Number and size of bounded reads.
- Step index where large outputs occur.
- Solve status and patch correctness.

Compare each experiment against:

- The three local baseline attempts in `20260428_115739_cae4b6`.
- The four local Bitloops attempts in `20260428_120816_ccd5fe` and `20260428_123013_2e887e`.

## Risks

- Over-tight guidance could make the agent miss cross-file context and reduce solve rate.
- Compact DevQL output could hide useful relationship information.
- Snippet output could become another large payload if too many snippets or too much context are returned.
- Token gains on this task may not generalize to tasks where whole-file structure is genuinely needed.

## Open Questions

- Should compact overview be a GraphQL argument, a CLI flag, or the default CLI rendering?
- Should snippets be returned by `selectArtefacts`, or should they be a separate stage after artefact selection?
- What line-window default should be used for Rust functions, methods, and tests?

## External Follow-Up

Benchmark-harness metric cleanup remains useful, but it belongs outside this repo. The external benchmark tooling should separate total tool calls from actual `bitloops devql query` calls, and should report full-file reads, bounded reads, and oversized tool outputs so future token regressions can be attributed without manual transcript parsing.

## Initial Recommendation

Start with Experiment 1.

Experiment 1 is the quickest token-reduction test because it changes the agent behavior that caused the largest outputs.

If Experiment 1 reduces full-file reads but token use is still high, move to compact overview output. If agents still ignore line ranges, move to read hints or snippets so DevQL results become the natural first inspection surface.
