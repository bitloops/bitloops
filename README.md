<div align="center">
  <img src="assets/bitloops-logo_320x132.png" alt="Bitloops logo" width="360" height="148" />
  <h1>Give AI coding agents the context they need to ship production-quality software.</h1>

<h4 align="center">Your AI agent forgets your codebase between sessions. You re-explain the architecture, re-paste rules, re-list constraints. Bitloops captures all of it once — automatically, from your agent conversations — and feeds the right pieces back to every future prompt. Runs locally. Replaces your CLAUDE.md, .cursor/rules, and AGENTS.md.</h4>



[![Fork](https://img.shields.io/github/forks/bitloops/bitloops?style=flat-square&label=Fork)](https://github.com/bitloops/bitloops/network/)
[![Star](https://img.shields.io/github/stars/bitloops/bitloops?style=flat-square&label=Star)](https://github.com/bitloops/bitloops/stargazers/)
[![Commits](https://badgen.net/github/commits/bitloops/bitloops?color=6b7280)](https://github.com/bitloops/bitloops/commits/)
[![Version](https://img.shields.io/github/v/tag/bitloops/bitloops?style=flat-square&color=7404e4)](https://github.com/bitloops/bitloops/tags/)
[![Downloads](https://img.shields.io/github/downloads/bitloops/bitloops/total?style=flat-square&color=6b7280)](https://github.com/bitloops/bitloops/releases)
[![License](https://img.shields.io/github/license/bitloops/bitloops?style=flat-square&color=111827)](https://github.com/bitloops/bitloops/blob/main/LICENSE)
[![Local First](https://img.shields.io/badge/Data-Local%20First-7404e4?style=flat-square)](https://github.com/bitloops/bitloops)
[![Agent Agnostic](https://img.shields.io/badge/Agents-Agent%20Agnostic-7404e4?style=flat-square)](https://github.com/bitloops/bitloops)
</div>



<p align="center">    
    <a href="https://bitloops.com">Website</a>
    ·
    <a href="https://bitloops.com/docs/">Docs</a>
    ·
    <a href="https://bitloops.com/docs/getting-started/quickstart">Quickstart</a>
    ·
    <a href="https://bitloops.com/docs/concepts/devql">DevQL</a>
    ·
    <a href="https://github.com/bitloops/bitloops/discussions">Discussions</a>
  </p>




## Quick Start

**macOS, Linux, WSL:**

```bash
curl -fsSL https://bitloops.com/install.sh | bash
```

**Windows PowerShell:**

```powershell
irm https://bitloops.com/install.ps1 | iex
```

Then, from inside the repo you want Bitloops to capture:
 
```bash
bitloops init --install-default-daemon
```
 
Work normally with Codex, Claude Code, Cursor, Gemini, Opencode or Copilot. Commit as usual. Bitloops captures the relevant context around every change and keeps your codebase model fresh in the background.


Open the local dashboard:

```bash
bitloops dashboard
```

Other install paths and full setup → [Docs](https://bitloops.com/docs/getting-started/quickstart)


## What You Get
 
- 🧠 **Persistent codebase substrate** — files, symbols, dependencies, tests, and history modeled as a queryable graph, not text.
- 🪶 **Automatic context capture** — decisions, constraints, and reasoning are pulled from your agent conversations as you work. No markdown file to maintain.
- 🎯 **Relevance-ranked retrieval** — every prompt gets the artifacts that actually matter. Not all files. Not random files. The right ones.
- 🔁 **Cross-agent memory** — what you decided in Claude Code yesterday guides Cursor today and Copilot tomorrow. Same substrate underneath all of them.
- 🧾 **Provenance from commit to prompt** — every commit traces back to the prompt, model, and rejected alternatives that produced it. Two weeks later you can still answer "why."
- 🗺️ **Code City spatial view** — a live map of your codebase. Files as buildings, height as size, arcs as dependencies. Filter by what AI touched.
- 🔍 **DevQL** — a typed query language for your codebase model. Ask precise questions instead of grepping.
- 🔒 **Local-first** — your code isn't stored on our infrastructure. The daemon runs on your machine.
- 🧩 **Drops in alongside your agent** — no agent switch, no IDE switch. Bitloops is the substrate underneath.


---
 
## Why Bitloops Exists
 
You've already built the workaround. A CLAUDE.md that grew to 2,000 lines. A `.cursor/rules` folder you copy-paste between projects. An AGENTS.md you swear you'll keep updated. A shell script that concatenates the "right" files into every prompt.
 
The pattern is always the same: a second brain for your codebase, built in markdown, manually maintained, going stale every week.
 
AI coding agents are bottlenecked by state, not generation. A codebase isn't fundamentally text — it's a system of files, symbols, APIs, tests, decisions, and history. Most agents force that structured system through an unstructured medium (prompt text, RAG chunks, embeddings) and have to infer the system over and over.
 
The loop today:
 
```
retrieve → infer → act → forget
```
 
Bitloops changes the loop to:
 
```
capture → persist → rank → serve → refresh
```
 
> Agents can't reliably change systems they can't model. Bitloops builds and maintains the model.
 
---

## The Old Way vs The Bitloops Way
 
| You're doing this today | With Bitloops |
| --- | --- |
| Re-explaining your architecture every session | Substrate captured once. Re-served on every prompt. |
| Hand-writing CLAUDE.md / `.cursor/rules` / AGENTS.md | Decisions and constraints captured automatically from your agent conversations. |
| Agent re-implements a util that already exists three folders over | Static analysis + clone detection feeds existing code into pre-edit context. |
| "Why is this here?" → the prompt is gone | Commit → prompt → rejected alternatives, all traceable. |
| Five rules files for five agents | One substrate. Claude Code, Cursor, Copilot — same model underneath. |
| RAG embeddings rebuilt every session | Long-lived daemon. Index is always-on, always-current. |
| Repository uploaded and stored on a vendor's infra | Runs locally. Code processed, not stored. |
 
---

## How It Works
 
```
┌──────────────────────────────────────────────────────────────────────┐
│  Your agent (Claude Code / Cursor / Copilot) makes a request         │
└──────────────────────────────────────────────────────────────────────┘
                                  ↓
┌──────────────────────────────────────────────────────────────────────┐
│  Bitloops hooks capture the prompt, transcript, and tool events      │
└──────────────────────────────────────────────────────────────────────┘
                                  ↓
┌──────────────────────────────────────────────────────────────────────┐
│  Local daemon updates the codebase graph (files, symbols, deps)      │
└──────────────────────────────────────────────────────────────────────┘
                                  ↓
┌──────────────────────────────────────────────────────────────────────┐
│  DevQL ranks and serves the right artifacts back into the agent      │
└──────────────────────────────────────────────────────────────────────┘
                                  ↓
┌──────────────────────────────────────────────────────────────────────┐
│  Commit links to the prompt, model, and decision that produced it    │
└──────────────────────────────────────────────────────────────────────┘
```
 
Architecture deep-dive → [Docs › Architecture](https://bitloops.com/docs/)
 
---

## Who Bitloops Is For
 
- **Devs shipping multiple AI-assisted PRs per week** with Claude Code, Cursor, Copilot, or Codex — who've stopped being amazed and started being annoyed.
- **Anyone maintaining a CLAUDE.md, `.cursor/rules`, or AGENTS.md** and quietly knowing it's already out of date.
- **Engineering teams** who want a reviewable trail for AI-assisted work — prompts, tool events, decisions, commits — not just diffs.
- **Platform and DevEx teams** building internal AI workflows who need a typed local substrate for agent context.
---

## Supported Agents & Languages
 
**Agents:**
 
- [x] Claude Code
- [x] Codex
- [x] Cursor
- [x] Gemini
- [x] Copilot
- [x] OpenCode

**Languages:** 
- [x] Rust
- [x] TypeScript / JavaScript
- [x] Python
- [x] Go
- [x] Java
- [x] C#
- [x] PHP
- [x] C++

---
## Local-First Trust Model
 
Your code isn't stored on our infrastructure. The daemon runs on your machine; configuration, repository model, event data, and blobs stay local by default.
 
LLM reasoning over commits, code, and conversations is part of the system — that processing requires sending content to a model provider, and an account is required to authenticate it. The line: **your code isn't stored, it is processed.**
 
--- 

## Demo video


<p align="center">
  <a href="https://www.youtube.com/watch?v=hb8EAWlRjt8" target="_blank">
    <img src="assets/bitloops_getting_started.png" alt="Bitloops Getting Started" width="640" />
  </a>
</p>

---

## Documentation
 
- 📖 [Quickstart](https://bitloops.com/docs/getting-started/quickstart) — set up and first capture
- 🔍 [DevQL](https://bitloops.com/docs/concepts/devql) — query the codebase model
- 🏗️ [Docs home](https://bitloops.com/docs/) — guides, concepts, troubleshooting
- 🛠️ [Contributing](https://github.com/bitloops/bitloops/blob/main/CONTRIBUTING.md) — rules, dev setup, extension guides
---
 
## Community
 
- 💬 [GitHub Discussions](https://github.com/bitloops/bitloops/discussions) — questions, ideas, feedback
- 🐛 [Issues](https://github.com/bitloops/bitloops/issues) — bug reports, feature requests
- 🔒 [Security](https://github.com/bitloops/bitloops/blob/main/SECURITY.md) — responsible disclosure
- 🤝 [Code of Conduct](https://github.com/bitloops/bitloops/blob/main/CODE_OF_CONDUCT.md)
If Bitloops is solving a problem you've felt — **star the repo**. It tells us we're building the right thing, and it tells other devs the project is real.
 
---
 
## License
 
Apache-2.0. See [LICENSE](https://github.com/bitloops/bitloops/blob/main/LICENSE).