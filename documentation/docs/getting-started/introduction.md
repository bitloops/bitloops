---
sidebar_position: 1
title: Introduction
---

# Introduction

So you've decided to try Bitloops. Good call.

Here's what you need to know before diving in.

## What is Bitloops?

Bitloops is a CLI that runs in the background alongside your AI coding agents. You install it, forget about it, and it quietly does two things:

**It captures everything.** Every prompt, every response, every file change, every decision your AI agent makes — recorded and linked to your git commits. Think of it as a flight recorder for AI-assisted development.

**It builds intelligence.** Bitloops parses your codebase into a structured knowledge graph — functions, dependencies, relationships — so your agents can query for exactly what they need instead of re-reading your entire repo every session. The longer you use it, the smarter it gets.

It's local-first (your code never leaves your machine), open source (Apache 2.0), and completely non-intrusive. Your workflow stays exactly the same.

## How does it work?

The short version:

1. **You work with your AI agent** as you normally would — ask questions, make changes, write code
2. **Bitloops captures Draft Commits** in real time — recording the conversation, reasoning, and decisions as they happen
3. **You `git commit`** and those Draft Commits become **Committed Checkpoints** — permanent, immutable records tied to that commit
4. **Next session**, your agent (or a different one) can tap into all that accumulated context instead of starting from zero

The result: every AI session builds on the last one. Your codebase develops a memory.

Want the full architecture? See [How Bitloops Works](/concepts/how-bitloops-works).

## Why teams choose Bitloops

**"My agent keeps forgetting everything."** Every session starts from scratch. You re-explain your architecture, your conventions, your constraints. Bitloops fixes that — agents get persistent context that survives across sessions.

**"I can't review AI-generated code properly."** A PR diff tells you _what_ changed, not _why_. With Committed Checkpoints, reviewers see the full reasoning chain: what was asked, what was considered, what was rejected, and why.

**"We use three different agents and it's chaos."** Claude Code, Cursor, and Copilot each build their own isolated understanding. No shared context. Bitloops gives them one unified knowledge store — same intelligence, regardless of which agent you're using.

**"We're spending a fortune on tokens and can't tell what's working."** Without observability, you can't optimize. Bitloops tracks which agents, which models, and which sessions produce results — so you can make informed decisions about your AI tooling.

**"Our AI doesn't know our architecture."** It's doing surface-level pattern matching, not structural analysis. Bitloops builds a real dependency graph from your code using Tree-sitter parsing, so agents understand blast radius, relationships, and impact before making changes.

## Which agents does Bitloops work with?

All of them. Well, all the major ones:

| Agent | What Bitloops captures |
|-------|----------------------|
| **Claude Code** | Full transcripts — sessions, prompts, tasks, reasoning, tool use |
| **Cursor** | Shell commands, prompt submissions |
| **GitHub Copilot** | Tool use and code interactions |
| **Codex** | Session boundaries |
| **Gemini** | Multi-stage tool chains |
| **OpenCode** | Coming soon |

You can use one agent or all five. `bitloops init` lets you select which ones to connect — and you can always add more later.

The important part: it doesn't matter which agent writes the code. Every session, every agent, feeds into the same knowledge store. One source of truth.

## What do I need?

- A git repo
- At least one AI coding agent
- About 2 minutes

That's it. No database to install, no cloud account to create, no config files to write. Bitloops bundles SQLite and DuckDB right in the binary.

Ready? The [Quickstart](/getting-started/quickstart) is three commands.
