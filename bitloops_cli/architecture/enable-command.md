# Enable and Init Commands

## Rust implementation

- `src/commands/enable.rs`
- `src/commands/init.rs`

## Command split

Recommended execution order:
1. `bitloops init`
2. `bitloops enable`

### `bitloops init`

Responsibilities:
- choose agents (explicit `--agent` or interactive selection)
- install selected agent hooks/plugins
- handle telemetry consent and persistence
- print initialization summary

Telemetry behavior:
- `--telemetry` defaults to `true`
- `--telemetry=false` always persists `telemetry=false`
- `ENTIRE_TELEMETRY_OPTOUT` always persists `telemetry=false`
- interactive path (`bitloops init` without `--agent`) prompts once when telemetry is unset
- non-interactive path (`bitloops init --agent ...`) does not prompt; telemetry remains unset unless explicitly disabled

Settings target for telemetry:
- writes to `.bitloops/settings.local.json` when that file already exists
- otherwise writes to `.bitloops/settings.json`

Supported explicit agents:
- `claude-code`
- `cursor`
- `gemini-cli` (and `gemini` alias)
- `opencode`

Key flags:
- `--agent <name>`
- `--force` / `-f`
- `--telemetry[=true|false]`

### `bitloops enable`

Responsibilities:
- validate repository + settings flags
- ensure `.bitloops/` directory and required `.gitignore` entries
- persist merged settings (`enabled=true`, strategy)
- install git hooks
- report initialized agents

It does not install agent hooks/plugins.

## Shared behaviors

- Both commands require running inside a git repository.
- Both commands are idempotent on repeated runs.
- Agent initialization detection checks:
  - Claude Code
  - Cursor
  - Gemini CLI
  - OpenCode
