---
sidebar_position: 3
title: Methodology
---

# The Evidence-Driven Development Methodology

Evidence-Driven Development is a software development methodology in which specifications define intended system behaviour, and every meaningful claim about the system must be supported by fresh, traceable, and invalidatable evidence.

It treats development as the continuous maintenance of justified confidence.

## Definition

In Evidence-Driven Development, a team does not ask only whether code was written or tests passed. It asks whether the current system state justifies the claims the team depends on.

A claim may describe behaviour, architecture, security, performance, compatibility, ownership, compliance, or user experience.

Examples:

- Users cannot access invoices belonging to another organisation.
- Payment retry attempts are idempotent.
- The ingestion worker can replay a checkpoint without duplicating records.
- The public API remains backwards compatible for versioned clients.
- The analytics layer does not depend on presentation-layer modules.

Each claim should be connected to the specification that expresses the intent, the implementation that attempts to realise it, the evidence that supports or weakens it, and the invalidation rules that define when the evidence must be refreshed.

## Lifecycle

The Evidence-Driven Development lifecycle is:

1. Intent
2. Specification
3. Implementation
4. Evidence collection
5. Confidence assessment
6. Invalidation detection
7. Re-evaluation

Intent describes the outcome the team wants.

Specification turns intent into an inspectable claim, requirement, rule, scenario, contract, policy, or constraint.

Implementation changes code, configuration, schema, infrastructure, workflows, or documentation to realise the specification.

Evidence collection gathers signals that support, weaken, or falsify the belief that the implementation satisfies the specification.

Confidence assessment records how strongly the current evidence justifies the claim.

Invalidation detection determines which claims or evidence items have become stale after change.

Re-evaluation refreshes evidence, revises claims, resolves contradictions, or escalates uncertainty.

## Core Artefacts

### Claim

A claim is a statement believed to be true about the system.

Claims should be specific enough to inspect and broad enough to matter. "The system is secure" is too broad. "Cross-organisation invoice access is rejected at every billing entry point" is useful.

### Specification

A specification expresses intended behaviour or constraints. It may be a requirement, acceptance criterion, API contract, architecture rule, ADR, policy, test scenario, or product decision.

In Evidence-Driven Development, a specification is not proof. It is the intent that evidence must answer to.

### Implementation Link

An implementation link connects a claim to the code, configuration, schema, service, route, workflow, or infrastructure that attempts to realise it.

These links should be concrete. A claim should not point vaguely at "the billing service" if the relevant artefacts are specific controllers, guards, policy objects, database constraints, and tests.

### Evidence Item

An evidence item is an artefact or derived fact that supports, weakens, or falsifies a claim.

Evidence items include test results, static analysis findings, type checks, coverage reports, dependency graph facts, runtime traces, metrics, logs, incidents, reviews, decisions, and human adjudications.

### Provenance Record

A provenance record explains how evidence was produced.

It should identify the producer, time, input state, command or tool, system version, and assumptions. Evidence without provenance cannot be inspected or refreshed reliably.

### Freshness Status

Freshness describes whether evidence still applies to the current system state.

Fresh evidence applies to the current state. Stale evidence was valid for an older state. Unknown evidence lacks enough provenance or dependency information to decide.

### Invalidation Rule

An invalidation rule describes what kinds of changes make evidence stale.

For example, access-control evidence for invoice isolation may be invalidated by changes to invoice routes, organisation membership logic, authentication middleware, billing policy, database schema, or test fixtures.

### Confidence Assessment

Confidence records the level of justification for a claim.

Confidence should be tied to evidence, not mood. It may be represented as qualitative levels such as blocked, low, medium, high, and accepted risk. For high-risk claims, confidence should require stronger and fresher evidence.

### Contradiction

A contradiction exists when evidence conflicts with another claim, specification, evidence item, or observed behaviour.

Contradictions should become explicit work items. They should not be hidden behind passing tests, vague documentation, or optimistic summaries.

### Decision or Adjudication

Some uncertainty requires human judgement. A decision records how ambiguity, trade-offs, exceptions, or contradictions were resolved.

Decisions are evidence too, but they must be traceable and invalidatable like any other evidence item.

## Evidence Categories

Evidence-Driven Development recognises several evidence categories.

Static evidence describes structure: AST facts, symbols, type relationships, imports, call graphs, dependency direction, cycles, and architecture boundaries.

Behavioural evidence describes executable behaviour: unit tests, integration tests, contract tests, property-based tests, end-to-end tests, and mutation testing.

Runtime evidence describes real operation: logs, traces, metrics, audit events, errors, latency, throughput, and production incidents.

Change evidence describes risk and history: commits, churn, bug-fix density, authorship concentration, recent refactors, and changed line ranges.

Coverage evidence describes exercised surface area: line coverage, branch coverage, scenario coverage, requirement coverage, mutation score, and verification maps.

Architectural evidence describes design conformance: dependency constraints, layering checks, ownership rules, module boundaries, and approved exceptions.

Human evidence describes judgement: reviews, ADRs, approvals, explicit acceptances, incident analyses, and product decisions.

Contradictory evidence weakens or falsifies confidence: failing tests, stale documents, runtime anomalies, broken contracts, drift, inconsistent facts, and unresolved review concerns.

## Development Workflow

For a new feature or change, the workflow is:

1. Identify the affected claims.
2. Create or update the relevant specifications.
3. Link the implementation artefacts.
4. Identify required evidence for the risk level.
5. Implement the change.
6. Collect and refresh evidence.
7. Assess confidence.
8. Resolve contradictions or record accepted risk.
9. Merge only when confidence is sufficient.

Small changes may have a lightweight version of this workflow. High-risk changes should require stronger evidence, more explicit invalidation rules, and clearer review.

## Review Workflow

An Evidence-Driven Development review asks:

- Which claims does this change affect?
- Which specifications define those claims?
- Which implementation artefacts changed?
- Which evidence became stale?
- Which evidence was refreshed?
- Which required evidence is missing?
- Are there contradictions?
- What confidence remains?
- Is the residual uncertainty acceptable for the risk?

The review is no longer only a judgement about code quality. It is a judgement about whether the team has restored justified confidence after change.

## Invalidation Workflow

Invalidation is the differentiator.

Every meaningful change should be treated as a possible invalidation event. The system should determine which claims and evidence items depend on the changed artefacts, then mark affected evidence as stale or uncertain until re-evaluation happens.

Examples:

- A change to authentication middleware invalidates access-control evidence for routes using that middleware.
- A schema migration invalidates evidence about backwards compatibility and data replay.
- A dependency upgrade invalidates evidence about performance, security, and API behaviour.
- A refactor of a policy object invalidates tests and reviews that depended on the previous policy structure.
- A production incident invalidates confidence in any claim contradicted by the incident.

Invalidation does not always mean a claim is false. It means the current evidence is no longer enough.

## Confidence Thresholds

Confidence thresholds should scale with risk.

Low-risk claims may need fresh unit tests, type checks, and code review.

Medium-risk claims may need integration tests, static analysis, coverage evidence, and explicit implementation links.

High-risk claims may need contract tests, property-based tests, security review, runtime monitoring, production audit evidence, and human adjudication.

Accepted risk is allowed, but it must be explicit. A team may ship with missing evidence only when the gap is visible and the decision is recorded.

## Definition of Done

In Evidence-Driven Development, done means:

- The relevant claims are known.
- The affected specifications are linked.
- The implementation artefacts are identified.
- The supporting evidence is fresh.
- The invalidated evidence has been refreshed or explicitly accepted.
- The contradictions are resolved or escalated.
- The remaining uncertainty is visible.
- The confidence level is sufficient for the risk.

Generated code is not completion.

Restored confidence is completion.

