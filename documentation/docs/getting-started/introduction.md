---
sidebar_position: 0
title: Introduction
---

# Introduction

Bitloops is a daemon-first CLI for capturing AI-assisted development activity, storing it in queryable backends, and exposing it through DevQL and the dashboard.

The current architecture has three main parts:

- A single global user-level daemon service, `com.bitloops.daemon`
- A thin CLI that starts, stops, queries, and controls that daemon
- Optional repo policy files that shape what hooks and the CLI emit to the daemon

Stores, credentials, dashboard defaults, and runtime paths belong to the daemon config. Repo policy belongs to the repository and controls capture behaviour.

If you are upgrading from the older repo-local JSON model, read the [upgrade note](../reference/upgrading-to-the-daemon-architecture.md) first.

## Core Ideas

- `bitloops start` launches the global daemon once it has been bootstrapped
- `bitloops start --create-default-config` bootstraps the default daemon config plus local default store files
- `bitloops init` bootstraps the current project or subproject
- `bitloops enable` and `bitloops disable` toggle capture in the current project policy
- `bitloops dashboard` opens the dashboard and starts the daemon if needed
- DevQL commands talk to the daemon over the local HTTP and GraphQL transport

## Next Step

Follow the [quickstart](./quickstart.md) to start the daemon, initialise a project, and open the dashboard.
