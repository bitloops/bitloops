# Settings

## Go Reference
- `golang-reference/cmd/entire/cli/settings/settings.go` — core load/save/merge logic
- `golang-reference/cmd/entire/cli/config.go` — high-level wrappers (`LoadEntireSettings`, `SaveEntireSettings`, `IsEnabled`)
- `golang-reference/cmd/entire/cli/config_test.go` — unit tests (ported to Rust)

## Rust Implementation
`src/engine/settings/mod.rs`

---

## Files

| File | Purpose |
|------|---------|
| `.bitloops/settings.json` | Project-level settings (committed to git) |
| `.bitloops/settings.local.json` | User-local overrides (gitignored) |

`.bitloops/.gitignore` always contains `settings.local.json`.

## Schema

```json
{
  "strategy": "manual-commit",
  "enabled": true,
  "local_dev": false,
  "log_level": "info",
  "telemetry": true,
  "strategy_options": {
    "push_sessions": true
  }
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `strategy` | string | `"manual-commit"` | Active strategy |
| `enabled` | bool | `true` | Whether Bitloops is active |
| `local_dev` | bool | `false` | Use local dev binary for hooks |
| `log_level` | string | `"info"` | Logging verbosity |
| `telemetry` | bool or omitted | omitted (`None`) | Telemetry opt-in state; omitted means not asked yet |
| `strategy_options` | map | `{}` | Strategy-specific config |

## Load / Merge Logic

Ported from `golang-reference/cmd/entire/cli/settings/settings.go` — `Load()` and `mergeJSON()`.

1. Load `settings.json` (project). If missing, use defaults.
2. Load `settings.local.json` (local). If missing, skip merge.
3. Merge local into project field-by-field:
   - Non-empty string values override
   - Bool values always override
   - `telemetry` overrides when present and boolean
   - `strategy_options` map is merged (local keys win)
   - Empty string in local does NOT override
4. Unknown keys in either file → return `Err`

## Rust Struct

```rust
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct BitloopsSettings {
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub local_dev: bool,
    #[serde(default)]
    pub log_level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<bool>,
    #[serde(default)]
    pub strategy_options: std::collections::HashMap<String, serde_json::Value>,
}
```

## Public API

```rust
pub fn load_settings(dir: &Path) -> Result<BitloopsSettings>
pub fn save_settings(settings: &BitloopsSettings, path: &Path) -> Result<()>
pub fn is_enabled(dir: &Path) -> Result<bool>
```
