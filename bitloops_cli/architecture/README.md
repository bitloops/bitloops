# Bitloops CLI — Architecture Docs

This folder documents the design and implementation of the Bitloops CLI.
Each document cross-references the Go reference implementation with the Rust implementation.

## Documents

| Doc | Topic |
|-----|-------|
| [enable-command.md](enable-command.md) | `init` + `enable` split: flags, flow, setup responsibilities |
| [settings.md](settings.md) | `.bitloops/settings.json` schema, load/merge/save logic |
| [claude-code-hooks.md](claude-code-hooks.md) | Claude Code hook types, JSON structure, install/uninstall |
| [dashboard-bundle-config.md](dashboard-bundle-config.md) | Build-time dashboard bundle URL embedding and runtime precedence |

## Go Reference

The Go reference implementation lives in `../golang-reference/cmd/entire/cli/`.
The Bitloops CLI adapts it with the following changes:

| Go original | Bitloops adaptation |
|-------------|---------------------|
| `.entire/` directory | `.bitloops/` directory |
| `entire` binary in hook commands | `bitloops` binary in hook commands |
| `Read(./.entire/metadata/**)` deny rule | `Read(./.bitloops/metadata/**)` deny rule |
| Telemetry opt-in | Captured during `bitloops init` (interactive path), persisted in settings |
| Multi-agent support | Claude Code, Cursor, Gemini CLI, OpenCode |
| Auto-commit strategy | Manual-commit only (for now) |
