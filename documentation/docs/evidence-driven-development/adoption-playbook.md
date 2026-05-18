---
sidebar_position: 5
title: Adoption Playbook
---

# Adopting Evidence-Driven Development

Evidence-Driven Development should not begin as a grand modelling exercise. Start with the claims where false confidence would be expensive, then make evidence visible around those claims.

The goal is to adopt a closed loop gradually:

1. Identify critical claims.
2. Link them to specifications.
3. Link them to implementation.
4. Attach evidence.
5. Track freshness.
6. Detect invalidation.
7. Restore confidence after change.

## Start with Critical Claims

Do not attempt to model the whole system.

Pick ten claims where being wrong would matter. Good candidates include:

- Security boundaries
- Billing rules
- Data loss paths
- Compliance obligations
- Authentication and authorisation flows
- Core user workflows
- Backwards compatibility guarantees
- Idempotency and replay behaviour
- Operational safety properties
- High-churn modules with recent incidents

For each claim, write one sentence that can be inspected.

Weak claim:

```text
Billing is secure.
```

Stronger claim:

```text
Users cannot access invoices belonging to another organisation.
```

## Phase 1: Manual EDD

Manual Evidence-Driven Development requires no new platform. It requires discipline and a small amount of structure.

For each critical claim, record:

- Claim statement
- Specification link
- Implementation artefacts
- Current supporting evidence
- Known evidence gaps
- Owner
- Risk level
- Review questions

This can live in Markdown, an issue tracker, a spreadsheet, or a lightweight internal registry. The first win is visibility.

Pull request reviews should add a small evidence section:

```text
Affected claims:
- billing.access.invoice-org-isolation

Evidence refreshed:
- Cross-organisation invoice integration test
- Billing policy unit tests

Evidence still missing:
- Production audit signal for denied invoice access

Confidence:
- Medium. Behavioural evidence is fresh, runtime evidence is not yet connected.
```

This is deliberately simple. The team learns which claims matter and which evidence is routinely missing.

## Phase 2: CI-Assisted EDD

Once claims and evidence are visible, move repeatable checks into CI.

Useful checks include:

- Required tests for critical claims
- Architecture boundary checks
- Static dependency checks
- Coverage thresholds for high-risk artefacts
- Contract test execution for public APIs
- Stale evidence warnings when changed files match known implementation links
- Pull request comments summarising affected claims and evidence status

The goal is not to block every change. The goal is to make confidence visible at review time.

CI should answer:

- Which claims appear affected by this change?
- Which evidence was refreshed?
- Which evidence is stale?
- Which required evidence is missing?
- Are contradictions present?

## Phase 3: Graph-Backed EDD

As manual links become too expensive, move from lists to a graph.

The graph should connect:

- Claims
- Specifications
- Code artefacts
- Tests
- Static analysis facts
- Runtime evidence
- Human decisions
- Dependencies
- Invalidation rules
- Confidence assessments

This is where Evidence-Driven Development becomes operational at scale. Developers and agents can ask targeted questions instead of searching through documents, CI logs, and dashboards.

Example questions:

- What claims depend on this function?
- Which tests support this requirement?
- What evidence became stale after this schema change?
- Which high-risk claims have low confidence?
- Which production incidents contradict accepted claims?
- What needs to be refreshed before release?

For Bitloops, this phase aligns with the knowledge store, DevQL, captured sessions, verification maps, external knowledge, and structural intelligence.

## Phase 4: Agent-Native EDD

In agent-native Evidence-Driven Development, AI agents do not merely retrieve context and edit files.

They operate against claims and evidence.

Before making a change, an agent should identify:

- Relevant claims
- Linked specifications
- Implementation artefacts
- Existing evidence
- Known stale evidence
- Architecture constraints
- Required checks for the risk level

After making a change, an agent should report:

- Claims affected
- Evidence invalidated
- Evidence refreshed
- Tests or analyses run
- Contradictions found
- Remaining uncertainty
- Confidence after the change

This turns the agent from a code generator into a participant in truth maintenance.

## Review Template

A lightweight pull request template can introduce the practice:

```markdown
## Evidence

### Affected claims

- 

### Evidence refreshed

- 

### Evidence made stale

- 

### Missing evidence or accepted risk

- 

### Confidence

- Blocked / Low / Medium / High / Accepted risk
```

The template should remain short. If it becomes bureaucratic, teams will invent vague answers. The point is to surface evidence, not to create paperwork.

## Confidence Levels

Use simple labels at first.

Blocked means required evidence is missing, stale, or contradicted.

Low means some evidence exists, but important gaps remain.

Medium means relevant evidence exists, but coverage, freshness, or runtime confidence is incomplete.

High means required evidence is fresh, relevant, and sufficient for the claim's risk.

Accepted risk means confidence is below the target, but an accountable person has explicitly accepted the gap.

Avoid false precision early. A well-explained qualitative judgement is better than a numeric score nobody trusts.

## Evidence Profiles

Define evidence profiles by risk.

Low-risk claims may require:

- Relevant unit tests
- Type checks
- Code review

Medium-risk claims may require:

- Unit tests
- Integration tests
- Static analysis
- Coverage visibility
- Implementation links

High-risk claims may require:

- Integration or end-to-end tests
- Contract tests
- Property-based tests where useful
- Architecture checks
- Security or domain review
- Runtime monitoring
- Explicit owner sign-off

The profile should be clear enough that teams know what evidence is expected before review.

## Adoption Risks

The most common failure is over-modelling. A team tries to define every claim, every artefact, every dependency, and every confidence rule before using the practice. That delays value.

Start narrow.

Another risk is treating Evidence-Driven Development as paperwork. If evidence is not connected to review, CI, runtime observations, or agent behaviour, the practice becomes documentation theatre.

Keep evidence close to decisions.

A third risk is treating tests as the entire model. Tests matter, but evidence also includes static structure, architecture, runtime behaviour, change history, incidents, reviews, and explicit decisions.

Use tests as the first evidence layer, not the last.

## First Week Checklist

In the first week:

- Choose ten critical claims.
- Write each claim as one inspectable sentence.
- Link each claim to a spec, decision, ticket, or requirement.
- Identify the implementation artefacts for each claim.
- List current tests or checks that support each claim.
- Mark missing evidence honestly.
- Add an evidence section to pull request reviews.
- Pick one claim and define what would invalidate its evidence.

By the end of the week, the team should know something it did not know before: where confidence is justified, where it is assumed, and where it is missing.

## First Month Checklist

In the first month:

- Add CI checks for the most important evidence.
- Create a small registry for claims and evidence.
- Link high-risk claims to owners.
- Add stale evidence warnings for changed files or symbols.
- Connect at least one runtime signal to a critical claim.
- Review confidence levels during release preparation.
- Capture accepted risks explicitly.

The first month should produce a working evidence loop for a small but important part of the system.

## Success Criteria

Evidence-Driven Development is working when:

- Developers can name the claims affected by a change.
- Reviewers can see which evidence was refreshed.
- Stale evidence is visible before merge.
- Missing evidence is treated as risk, not ignored.
- Contradictions become work items.
- AI agents can query claims and evidence before editing.
- Release decisions include confidence, not just test status.

The result is not more ceremony. The result is less false confidence.

