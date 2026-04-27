# Bitloops VS Code Extension

Bitloops for VS Code surfaces current-state DevQL overviews directly in the editor and provides a sidebar search over artefacts in the active workspace folder.

## Features

- File-level and artefact-level Bitloops CodeLens
- Bitloops sidebar search backed by typed DevQL search modes with unified results plus `AUTO` breakdown slices
- CLI-based transport through the local `bitloops` binary

## Commands

- `Bitloops: Search Artefacts`
- `Bitloops: Refresh Active File Overview`

## Requirements

- A working `bitloops` CLI installation
- A running Bitloops daemon for the current workspace

## Settings

- `bitloops.cliPath`
- `bitloops.autoRefresh`
- `bitloops.searchResultLimit`
- `bitloops.activeFileArtefactLimit`
