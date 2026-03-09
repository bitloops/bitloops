# Problem Definition

## Context

AI coding agents (Claude Code, Gemini CLI, etc.) are becoming a standard part of software development. Developers use them to write features, fix bugs, refactor code, and explore unfamiliar codebases. The agents are effective — but they operate entirely outside the existing tooling that teams rely on to understand, review, and maintain code: **git**.

---

## The Problems

### 1. AI sessions are ephemeral

When an agent session ends, the conversation is gone. There is no persistent record of what was asked, what the agent reasoned, or what it tried before arriving at the final output. The developer who ran the session may remember, but their teammates and future-self do not.

### 2. Git history shows *what* changed, not *why*

`git log` and `git diff` describe the resulting code change, but carry none of the intent behind it. For AI-assisted changes this gap is larger than usual — the prompt, the back-and-forth, the intermediate attempts, and the agent's final reasoning are all invisible.

> A commit message can say *"fix login bug"*, but the 12-turn agent conversation that led to it — including what was tried and discarded — is lost forever.

### 3. No recovery path when agents go wrong

Agents can go sideways: they overwrite files, take the wrong approach, or make a change that compounds into a bigger mess. Without save points, the only option is `git stash` or manually undoing changes — neither of which maps to the agent session boundary.

### 4. No measurement of AI contribution

Teams and organisations have no way to answer basic questions:
- How much of our codebase was written by an AI?
- Which commits were AI-assisted, and to what degree?
- How is AI usage trending across the team over time?

Without instrumentation at the session level, these questions are unanswerable.

### 5. Onboarding and code review lose context

When a teammate reviews a PR or a new engineer tries to understand a module, they have only the diff. If the code was AI-generated, the prompt and the conversation that shaped it — the closest thing to a design rationale — are gone. The reviewer is left inferring intent from output alone.

### 6. Compliance and audit requirements are unmet

Enterprises increasingly need to track AI usage in their codebase for legal, licensing, or internal policy reasons. There is currently no standard mechanism to produce an audit trail of *which code* was AI-generated, *when*, and *by whom*.

---

## Who Is Affected

| Audience | Pain |
|---|---|
| **Individual developer** | No rewind, no record of what the agent did, context lost between sessions |
| **Team** | Code reviews lack context, can't tell AI-assisted from human-written changes |
| **Engineering org** | Can't measure AI adoption, no audit trail, no policy enforcement point |

---

## Why Existing Tools Don't Solve It

| Tool | Gap |
|---|---|
| **Git** | Tracks code changes, not the agent session that produced them |
| **Agent chat history** | Stored in the agent's UI, not linked to the code or the commit |
| **Commit messages** | Free-text, not structured, not machine-readable, rely on the developer to write them |
| **PR descriptions** | Written after the fact, optional, and not linked to individual commits |

The gap is structural: the agent and git operate independently, with no bridge between them.
