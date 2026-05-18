---
sidebar_position: 4
title: Technical Model
---

# The Evidence Graph: A Technical Model for Evidence-Driven Development

The technical substrate of Evidence-Driven Development is an evidence graph: a directed, typed graph that links claims, specifications, implementation artefacts, evidence items, provenance records, dependencies, invalidation rules, confidence assessments, contradictions, and decisions.

The purpose of the graph is not to store documents. It is to maintain justified beliefs about a changing software system.

## Goals

An evidence graph should answer questions such as:

- What claims do we believe about this system?
- Where are those claims specified?
- Which artefacts implement them?
- What evidence supports, weakens, or falsifies them?
- Where did the evidence come from?
- Which system state does the evidence apply to?
- What changed since the evidence was produced?
- Which evidence is stale?
- What confidence remains?
- What must be rechecked before merge or release?

This is codebase truth maintenance. It gives humans and agents a way to reason from maintained evidence rather than stale context.

## Primitive Types

### Artefact

An artefact is any addressable thing in or around the system.

Examples include files, symbols, modules, APIs, database tables, migrations, tests, builds, deployments, logs, traces, metrics, issues, pull requests, ADRs, Confluence pages, Jira tickets, and human review comments.

Artefacts need stable identity. A graph that cannot distinguish a renamed function from a deleted and recreated function will struggle to maintain evidence across change.

### Claim

A claim is a statement believed to be true about the system.

Claims should have:

- Identifier
- Statement
- Scope
- Risk level
- Owner
- Linked specifications
- Linked implementation artefacts
- Required evidence profile
- Current confidence
- Freshness state

Example:

```yaml
id: billing.access.invoice-org-isolation
statement: Users cannot access invoices belonging to another organisation.
scope: billing
risk: high
owner: security
required_evidence:
  - access-control unit tests
  - cross-organisation integration test
  - route-policy static analysis
  - production audit signal
```

### Specification

A specification expresses intended truth. It may be structured or informal, but it must be addressable.

Specifications may come from requirements, acceptance criteria, API contracts, ADRs, architecture rules, policies, issue descriptions, or test scenarios.

### Evidence

Evidence is an artefact or derived fact that supports, weakens, or falsifies a claim.

Evidence should have:

- Identifier
- Claim relationship
- Direction: supports, weakens, falsifies, or contextualises
- Evidence type
- Producer
- Provenance
- Applicable system state
- Freshness
- Confidence contribution
- Expiry or invalidation rules

Example:

```yaml
id: evidence.billing.invoice-org-isolation.integration-test.2026-05-16
claim: billing.access.invoice-org-isolation
direction: supports
type: integration-test-result
producer: ci
system_state: commit:abc123
freshness: fresh
confidence_contribution: strong
```

### Provenance

Provenance explains how evidence was produced.

A provenance record should capture:

- Tool or human producer
- Time
- Input artefacts
- Command, query, or workflow
- Environment
- System state
- Model version if an AI inference was involved
- Assumptions
- Limitations

Evidence without provenance is difficult to trust, refresh, or invalidate.

### Dependency

A dependency is a relationship indicating that a claim, artefact, or evidence item relies on another artefact or fact.

Dependencies may be structural, behavioural, operational, semantic, or human.

Examples:

- A claim depends on a policy object.
- A test result depends on test fixtures and schema version.
- A runtime metric depends on deployment environment.
- A review decision depends on a specific diff.
- A generated summary depends on source files and model output.

### Invalidation Rule

An invalidation rule defines when evidence should stop being considered fresh.

Rules may be explicit or derived from graph dependencies.

Example:

```yaml
claim: billing.access.invoice-org-isolation
invalidate_when_changed:
  - route: /billing/invoices/**
  - symbol: BillingPolicy
  - symbol: OrganisationMembershipGuard
  - schema: invoices
  - middleware: authentication
  - fixture: billing-access-control
```

When one of these artefacts changes, evidence that depended on the previous state should be marked stale or uncertain.

### Confidence

Confidence represents the current justification for a claim.

It should not be a free-floating score. It should be derived from evidence presence, freshness, relevance, strength, contradiction status, risk level, and human adjudication.

Suggested levels:

- Blocked: required evidence is missing, stale, or falsifying.
- Low: some evidence exists, but important gaps remain.
- Medium: relevant evidence exists, but coverage, freshness, or contradiction concerns remain.
- High: required evidence is fresh, relevant, and sufficient for the risk.
- Accepted risk: confidence is below the target, but a responsible decision accepts the gap.

### Contradiction

A contradiction records conflict between claims, specifications, evidence, observations, or decisions.

Examples:

- A test passes but production traces show forbidden behaviour.
- A spec says a route is private but static analysis shows no guard.
- A review approves a dependency that violates an architecture rule.
- A generated summary conflicts with current source facts.

Contradictions should be queryable, reviewable, and tied to resolution decisions.

### Decision

A decision records human or model-assisted adjudication.

Decisions may accept risk, resolve ambiguity, approve an exception, override a false positive, or revise a claim. They should include provenance and invalidation rules like any other evidence item.

## Graph Relationships

Useful relationship types include:

- `specifies`: a specification expresses a claim.
- `implements`: an artefact implements or participates in a claim.
- `supports`: evidence supports a claim.
- `weakens`: evidence weakens a claim.
- `falsifies`: evidence contradicts a claim strongly enough to reject confidence.
- `depends_on`: a claim or evidence item relies on another artefact.
- `invalidates`: a change makes evidence stale.
- `derived_from`: an evidence item was produced from input artefacts.
- `adjudicates`: a decision resolves uncertainty or contradiction.
- `observes`: runtime evidence observes system behaviour.

The graph should preserve both direct and derived relationships. A code change may not directly touch a claim, but it may touch an artefact that evidence depends on, which makes the claim's confidence stale.

## Freshness Model

Freshness should be computed relative to system state.

At minimum, an evidence item should know the commit, build, deployment, schema version, environment, or source snapshot it applies to.

Freshness states:

- Fresh: evidence applies to the current relevant state.
- Stale: a dependency changed after evidence was produced.
- Unknown: dependency or provenance data is insufficient.
- Superseded: newer evidence replaces this evidence.
- Rejected: evidence was found irrelevant or incorrect.

Freshness is separate from truth. Stale evidence may still describe behaviour accurately, but the system can no longer rely on it without re-evaluation.

## Confidence Model

Confidence can be computed, declared, or hybrid.

A practical model combines:

- Required evidence profile for the claim's risk level
- Evidence freshness
- Evidence relevance
- Evidence strength
- Coverage of implementation artefacts
- Presence of contradictions
- Change risk
- Runtime observations
- Human decisions

The model should explain its result. "Confidence: medium" is less useful than "Confidence: medium because integration evidence is fresh, but runtime audit evidence is missing after the middleware change."

## Agent Interaction Model

AI agents should query the evidence graph before and after changes.

Before change, an agent should ask:

- Which claims mention this area?
- Which artefacts implement those claims?
- Which tests and analyses support them?
- Which evidence is already stale?
- Which constraints must be preserved?

After change, an agent should ask:

- Which claims were affected?
- Which evidence was invalidated?
- Which evidence was refreshed?
- Which contradictions remain?
- What confidence level is now justified?

The agent's role is not only to edit files. It should help maintain the evidence graph.

## Integration Points

An evidence graph can integrate with:

- Source control for commits, diffs, changed ranges, and history.
- Static analysis for symbols, dependencies, call graphs, and architecture rules.
- Test runners for behavioural evidence and failure provenance.
- Coverage tools for verification maps and untested paths.
- CI/CD for build, release, deployment, and environment state.
- Observability tools for runtime traces, logs, metrics, and incidents.
- Issue trackers and documentation systems for specifications and decisions.
- IDEs and agent tools for evidence queries during development.

For Bitloops, this model fits naturally with the intelligence layer: captured sessions, DevQL, structural graphs, semantic understanding, verification maps, external knowledge, and historical reasoning can become evidence rather than disconnected context.

## Minimal Viable Evidence Graph

A useful first implementation does not need every primitive.

Start with:

- Claims
- Specifications
- Implementation artefacts
- Test evidence
- Provenance
- Invalidation from changed files or symbols
- Confidence labels

Then add:

- Static analysis evidence
- Runtime observations
- Contradiction tracking
- Human adjudication
- Risk-aware confidence thresholds
- Agent-facing queries

The goal is not to model the entire organisation on day one. The goal is to make the most important claims explicit, supported, and invalidatable.

