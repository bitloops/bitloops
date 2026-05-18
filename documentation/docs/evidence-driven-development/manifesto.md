---
sidebar_position: 1
title: Manifesto
---

# The Evidence-Driven Development Manifesto

## From Intent to Evidence

Software teams have become better at describing intent before implementation. Spec-Driven Development is one expression of that shift: before writing code, define what the system is meant to do, how it should behave, which constraints it must respect, and which outcomes it should produce.

That starting point matters. A clear specification gives developers and agents useful direction, reduces ambiguity, and turns vague prompts, tickets, and conversations into something concrete.

But a specification describes intended behaviour. It does not, by itself, show that the implementation matches that intent. It cannot tell us when the code has decayed, when tests have gone stale, when architecture has drifted, when production behaviour contradicts the design, or when an AI agent is reasoning from an outdated assumption.

Evidence-Driven Development starts from that gap. It treats specifications, code, tests, documentation, architecture, telemetry, and agent context as related artefacts whose claims need to stay aligned as the system changes.

Software fails when the relationship between what we believe and what the system actually does starts to decay.

The specification may say one thing, while the code does another. The tests may support a narrower claim than the team believes. The documentation may describe an older system. The production system may behave differently under real conditions. An agent may retrieve a stale artefact and treat it as current.

The existence of a specification is not evidence that the system satisfies it.

A specification is a source of intent, not a source of truth. Evidence is how we understand the state of the system and how much confidence we can place in claims about it.

Evidence-Driven Development is the discipline of maintaining justified confidence in software systems. Specifications define what should be true. Implementation attempts to make it true. Evidence establishes whether we are justified in believing it is true. Change can invalidate that confidence, so it has to be re-established from current evidence.

For that reason, finishing a change means more than generating code, matching the spec at a glance, or getting a passing test run. Those may all be useful steps, but the relevant question is whether justified confidence has been restored.

The principles are straightforward:

- Specifications are necessary, but they are not proof.
- A specification is a claim about desired behaviour, not confirmation of actual behaviour.
- Implementation is an attempt to satisfy intent, not evidence that the intent has been fulfilled.
- Tests are evidence, but evidence is broader than tests.
- Meaningful claims about a system should be supported by fresh, traceable evidence.
- Evidence should have provenance: where it came from, when it was produced, what it supports, and under which assumptions it remains valid.
- Evidence expires as the system, its dependencies, and its environment change.
- Every meaningful change is a possible invalidation event.
- Stale evidence should be treated as risk, not reassurance.
- Contradictions should be surfaced, even when builds are green, documents look authoritative, or explanations sound plausible.
- Confidence should be earned from current evidence, not assumed from past work.
- AI agents need more than context. They need claims, artefacts, provenance, dependencies, contradictions, freshness, invalidation rules, and confidence.

Evidence-Driven Development does not reject specifications, tests, or human judgement. It makes specifications accountable, gives tests a clearer role as evidence linked to claims and system states, and makes judgement explicit, inspectable, and open to revision. It treats confidence as a maintained property of the development process rather than as a feeling.

Spec-Driven Development asks: what should we build? Evidence-Driven Development adds a second question: what are we justified in believing about what we have built?

The goal is not just to produce code that appears to follow intent. The goal is to keep our understanding of the system honest as the system changes.
