---
sidebar_position: 2
title: Foundational Essay
---

# Evidence-Driven Development: Closing the Loop from Specification to Truth

Spec-Driven Development made software development more explicit by putting intent before implementation. That was the right first move. A team should not begin with a blank editor and hope that structure emerges from code. It should first describe what the system is meant to do, what constraints it must obey, and what outcomes it should produce.

But intent is not truth.

A specification describes what should be true. It does not prove what is true. It does not know whether the implementation actually satisfies it. It does not know whether tests are relevant, whether architecture still conforms, whether production behaviour matches expectation, or whether a later change has invalidated the assumptions under which the spec was written.

That is the central failure mode of Spec-Driven Development: it can confuse specified intent with verified reality.

## The Blank-Page Problem and the Truth Problem

Spec-Driven Development solves the blank-page problem. It gives developers and agents a structured starting point. It turns vague intent into something inspectable. It reduces ambiguity before code is written. It makes it possible to discuss requirements, constraints, interfaces, and behaviours before implementation momentum takes over.

This matters. A well-written specification can improve alignment, reduce rework, and make AI-generated code less arbitrary.

But software does not fail only because teams lacked a starting point.

Software fails because the relationship between intent and reality decays.

The spec says one thing. The code does another. The tests prove a narrower claim than the team thinks. The documentation describes an older system. The architecture has drifted. The production system behaves differently under real conditions. A dependency changes beneath a previously valid assumption. An AI agent retrieves stale context and treats it as authoritative.

This is the truth problem.

The truth problem is not solved by writing more specifications. It is solved by maintaining evidence about the system.

## Why Specifications Decay

A specification is usually written at a moment in time. It captures what a team believes, intends, or agrees to at that moment. Then the system changes.

Code is refactored. Tests are added, removed, or weakened. Database schemas evolve. Authentication models shift. Feature flags accumulate. Incidents reveal behaviours nobody specified. Performance characteristics change under load. Users discover paths the design did not anticipate. New developers join. AI agents generate patches from partial context.

The spec may remain textually unchanged while its relationship to the system deteriorates.

This deterioration is often invisible because teams mistake artefact existence for evidence. A ticket exists, so the requirement is assumed to be known. A test exists, so the behaviour is assumed to be covered. A document exists, so the architecture is assumed to be current. A CI run passed, so the change is assumed to be safe.

Those assumptions are often false.

The existence of a specification is not evidence that the system satisfies it. The existence of a test is not evidence that the test proves the relevant claim. The existence of a green build is not evidence for behaviours that build did not exercise.

Evidence-Driven Development begins by refusing those shortcuts.

## The Missing Feedback Layer

The development loop most teams want is simple:

1. Express intent.
2. Implement the intent.
3. Verify that the implementation satisfies the intent.
4. Keep verifying as the system changes.

Spec-Driven Development strengthens the first two steps. It asks teams and agents to express intent before implementation. It may also generate tests or code from that intent.

But the loop remains open unless the system maintains an account of evidence.

Evidence-Driven Development closes the loop:

1. Intent defines the desired outcome.
2. Specification expresses the intended behaviour or constraint.
3. Implementation attempts to realise it.
4. Evidence supports, weakens, or falsifies the belief that the implementation satisfies the specification.
5. Confidence summarises the current justification.
6. Change invalidates affected confidence.
7. Re-evaluation restores, revises, or rejects the belief.

A loop is not closed when code is generated from a spec. It is closed only when fresh evidence continuously justifies confidence that the implementation still satisfies the spec.

## Evidence Is More Than Tests

Tests are one of the most important evidence types. They are executable, repeatable, and usually close to the code. But tests are not the whole evidence model.

Unit tests can support claims about small behaviours. Integration tests can support claims about component interaction. Contract tests can support claims about service boundaries. Property-based tests can support broader invariants. Mutation testing can reveal whether tests catch meaningful faults.

Static analysis can support claims about structure, ownership, safety, and dependency direction. Type systems can support claims about valid composition. Architecture checks can support claims about layering, cycles, and boundaries. Coverage can show what is exercised and what is not.

Runtime traces, logs, metrics, and production observations can support or weaken claims about real behaviour under real conditions. Incidents can falsify previous confidence. Reviews, architectural decisions, and explicit approvals can record human judgement where evidence is ambiguous or trade-offs are deliberate.

Evidence-Driven Development does not worship one signal. It composes many signals into justified confidence.

## Why AI Makes This Urgent

AI makes output cheap. It makes plausible implementation cheap. It makes plausible explanation cheap. It makes plausible documentation cheap.

That is useful, but it changes the failure mode. The bottleneck shifts from producing code to knowing whether the produced code is trustworthy.

An AI agent can read a specification, inspect nearby files, generate an implementation, write tests, and explain its reasoning. But if it does not know which information is fresh, which claims are supported, which assumptions have been invalidated, and which evidence is missing, it can act with unjustified confidence.

Context is not enough. Retrieval is not enough. A long prompt is not enough.

Agents need a maintained evidence model: claims, artefacts, provenance, dependencies, contradictions, freshness, invalidation rules, and confidence.

Without evidence, agents amplify stale assumptions. With evidence, agents can participate in disciplined software change.

## From Code Generation to Truth Maintenance

The deeper shift is this: reliable AI-native software engineering is not primarily a code generation problem. It is a truth maintenance problem.

Teams need to know what is believed about the system, why those beliefs are justified, what evidence supports them, what evidence contradicts them, what changes make them stale, and what must be rechecked before confidence can be restored.

This is where Evidence-Driven Development becomes a methodology rather than a slogan.

It changes the definition of done. Done does not mean the code was written, the agent produced an implementation, the pull request was approved, or the tests passed. Done means the relevant claims are known, the affected evidence is fresh, contradictions are resolved or accepted, remaining uncertainty is visible, and confidence is sufficient for the risk.

It changes review. Review is not only "does this code look right?" Review becomes "what beliefs does this change affect, what evidence supports those beliefs now, what became stale, and what confidence remains?"

It changes tooling. A development environment should not merely retrieve files and documents. It should query a maintained graph of claims, code artefacts, evidence items, provenance records, dependencies, invalidation rules, and confidence assessments.

It changes the role of Bitloops. The value is not only capturing what agents did. The value is building an intelligence layer that helps humans and agents understand what is true, why it is believed, and when that belief must be rechecked.

## The Claim

Spec-Driven Development gave software a better starting point.

Evidence-Driven Development gives software a better feedback loop.

Specs define intent. Evidence establishes trust. Change invalidates confidence. Re-evaluation restores it.

That is the move from specified intent to justified confidence.

And in a world where humans and AI agents build software together, justified confidence is the scarce resource.

