---
sidebar_position: 1
title: Evidence-Driven Development
sidebar_label: Overview
---

# Evidence-Driven Development

Evidence-Driven Development is a methodology for closing the loop left open by Spec-Driven Development.

Spec-Driven Development defines intended behaviour. Evidence-Driven Development asks whether the current system state still justifies confidence that the intended behaviour is true.

This section separates the idea into distinct artefacts so each document can stand on its own:

- [Manifesto](./manifesto.md): the compact statement of belief.
- [Foundational Essay](./foundational-essay.md): the argument for why EDD is needed.
- [Methodology](./methodology.md): the operating model for practising EDD.
- [Technical Model](./technical-model.md): the evidence graph and truth-maintenance primitives.
- [Adoption Playbook](./adoption-playbook.md): the incremental path from manual practice to agent-native evidence workflows.

## Relationship to Bitloops

EDD should be understandable without Bitloops. The methodology is independent: teams can start manually, then add CI checks, graph-backed evidence, and agent-native workflows over time.

Inside Bitloops, EDD is also a product thesis.

Bitloops is intended to provide the evidence layer that makes agent-native EDD practical: captured sessions, provenance, structural intelligence, DevQL queries, verification maps, historical reasoning, external knowledge links, and eventually confidence and invalidation workflows.

The documents in this section therefore keep the methodology independent while treating Bitloops as the reference implementation path.

## Core Claim

Spec-Driven Development solves the blank-page problem.

Evidence-Driven Development solves the truth problem.

Specs define intent. Evidence establishes trust. Change invalidates confidence. Re-evaluation restores it.

