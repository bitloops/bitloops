use std::collections::BTreeSet;

use async_graphql::types::Json;
use serde_json::{Map, Value};

use super::redaction::{is_secret_path, redact_json_value, value_at_path};
use super::types::{
    ConfigTarget, ConfigTargetKind, REDACTED_VALUE, RuntimeConfigFieldObject,
    RuntimeConfigSectionObject,
};

pub(super) fn build_sections_for_target(
    target: &ConfigTarget,
    value: &Value,
    effective: Option<&Value>,
) -> Vec<RuntimeConfigSectionObject> {
    let specs = match target.kind {
        ConfigTargetKind::Daemon => daemon_section_specs(),
        ConfigTargetKind::RepoShared | ConfigTargetKind::RepoLocal => repo_section_specs(),
    };
    let mut known_roots = BTreeSet::new();
    let mut sections = Vec::new();

    for spec in specs {
        known_roots.insert(spec.key.to_string());
        let section_value = value_at_path(value, &[spec.key])
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let effective_section = effective
            .and_then(|effective| value_at_path(effective, &[spec.key]))
            .cloned();
        let fields = spec
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| build_field(field, value, effective, index as i32))
            .collect::<Vec<_>>();
        sections.push(RuntimeConfigSectionObject {
            key: spec.key.to_string(),
            title: spec.title.to_string(),
            description: spec.description.to_string(),
            order: spec.order,
            advanced: false,
            fields,
            value: Json(redact_json_value(&section_value)),
            effective_value: effective_section.map(|value| Json(redact_json_value(&value))),
        });
    }

    if let Value::Object(map) = value {
        for (key, section_value) in map {
            if known_roots.contains(key.as_str()) {
                continue;
            }
            let path = vec![key.clone()];
            sections.push(RuntimeConfigSectionObject {
                key: key.clone(),
                title: title_from_key(key),
                description: "Advanced config section supplied by the runtime.".to_string(),
                order: 10_000,
                advanced: true,
                fields: vec![RuntimeConfigFieldObject {
                    key: key.clone(),
                    path,
                    label: title_from_key(key),
                    description: "Edit this section as structured JSON.".to_string(),
                    field_type: "json".to_string(),
                    value: Json(redact_json_value(section_value)),
                    effective_value: None,
                    default_value: None,
                    allowed_values: Vec::new(),
                    validation_hints: vec![
                        "Enter a valid JSON object, array, or scalar.".to_string(),
                    ],
                    required: false,
                    read_only: false,
                    secret: false,
                    order: 0,
                    source: None,
                }],
                value: Json(redact_json_value(section_value)),
                effective_value: None,
            });
        }
    }

    sections.sort_by(|left, right| {
        left.order
            .cmp(&right.order)
            .then_with(|| left.title.cmp(&right.title))
    });
    sections
}

fn build_field(
    spec: &FieldSpec,
    value: &Value,
    effective: Option<&Value>,
    order: i32,
) -> RuntimeConfigFieldObject {
    let current = value_at_path(value, spec.path)
        .cloned()
        .unwrap_or(Value::Null);
    let effective_value = effective.and_then(|effective| value_at_path(effective, spec.path));
    let secret = spec.secret || is_secret_path(spec.path);
    RuntimeConfigFieldObject {
        key: spec.path.join("."),
        path: spec
            .path
            .iter()
            .map(|segment| segment.to_string())
            .collect(),
        label: spec.label.to_string(),
        description: spec.description.to_string(),
        field_type: spec.field_type.to_string(),
        value: Json(if secret && !current.is_null() {
            Value::String(REDACTED_VALUE.to_string())
        } else {
            redact_json_value(&current)
        }),
        effective_value: effective_value.map(|value| {
            Json(if secret && !value.is_null() {
                Value::String(REDACTED_VALUE.to_string())
            } else {
                redact_json_value(value)
            })
        }),
        default_value: field_default_value(spec).map(Json),
        allowed_values: spec
            .allowed_values
            .iter()
            .map(|value| value.to_string())
            .collect(),
        validation_hints: validation_hints_for_field(spec),
        required: spec.required,
        read_only: spec.read_only,
        secret,
        order,
        source: effective_value.map(|_| "effective".to_string()),
    }
}

#[derive(Clone, Copy)]
struct SectionSpec {
    key: &'static str,
    title: &'static str,
    description: &'static str,
    order: i32,
    fields: &'static [FieldSpec],
}

#[derive(Clone, Copy)]
struct FieldSpec {
    path: &'static [&'static str],
    label: &'static str,
    description: &'static str,
    field_type: &'static str,
    default_value: Option<&'static str>,
    allowed_values: &'static [&'static str],
    required: bool,
    read_only: bool,
    secret: bool,
}

fn field_default_value(spec: &FieldSpec) -> Option<Value> {
    let raw = spec.default_value?;
    match spec.field_type {
        "boolean" => raw.parse::<bool>().ok().map(Value::Bool),
        "integer" => raw
            .parse::<i64>()
            .ok()
            .map(|value| Value::Number(value.into())),
        _ => Some(Value::String(raw.to_string())),
    }
}

fn validation_hints_for_field(spec: &FieldSpec) -> Vec<String> {
    let mut hints = Vec::new();
    match spec.field_type {
        "boolean" => hints.push("Use true or false.".to_string()),
        "integer" => hints.push("Enter a whole number.".to_string()),
        "json" => hints.push("Enter valid JSON; the runtime converts it back to TOML.".to_string()),
        _ => {}
    }
    if !spec.allowed_values.is_empty() {
        hints.push(format!(
            "Allowed values: {}.",
            spec.allowed_values.join(", ")
        ));
    }
    if spec.required {
        hints.push("This field is required by the runtime config loader.".to_string());
    }
    if spec.secret {
        hints.push("Existing values are redacted; leave the placeholder unchanged to preserve the current secret.".to_string());
    }
    hints
}

const DAEMON_RUNTIME_FIELDS: &[FieldSpec] = &[FieldSpec {
    path: &["runtime", "local_dev"],
    label: "Local development",
    description: "Enable local-development runtime behaviour.",
    field_type: "boolean",
    default_value: Some("false"),
    allowed_values: &[],
    required: false,
    read_only: false,
    secret: false,
}];

const DAEMON_TELEMETRY_FIELDS: &[FieldSpec] = &[FieldSpec {
    path: &["telemetry", "enabled"],
    label: "Telemetry",
    description: "Enable or disable local CLI telemetry consent.",
    field_type: "boolean",
    default_value: None,
    allowed_values: &[],
    required: false,
    read_only: false,
    secret: false,
}];

const DAEMON_LOGGING_FIELDS: &[FieldSpec] = &[FieldSpec {
    path: &["logging", "level"],
    label: "Log level",
    description: "Daemon log verbosity.",
    field_type: "enum",
    default_value: None,
    allowed_values: &["trace", "debug", "info", "warn", "error"],
    required: false,
    read_only: false,
    secret: false,
}];

const DAEMON_STORE_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        path: &["stores", "relational", "sqlite_path"],
        label: "SQLite path",
        description: "Local relational store path.",
        field_type: "string",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["stores", "events", "duckdb_path"],
        label: "DuckDB path",
        description: "Local event store path.",
        field_type: "string",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["stores", "blob", "local_path"],
        label: "Blob store path",
        description: "Local blob store directory.",
        field_type: "string",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
];

const DAEMON_SEMANTIC_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        path: &["semantic_clones", "summary_mode"],
        label: "Summary mode",
        description: "Controls semantic summary generation.",
        field_type: "enum",
        default_value: Some("auto"),
        allowed_values: &["auto", "off"],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["semantic_clones", "embedding_mode"],
        label: "Embedding mode",
        description: "Controls semantic clone embedding refresh behaviour.",
        field_type: "enum",
        default_value: Some("semantic_aware_once"),
        allowed_values: &[
            "off",
            "deterministic",
            "semantic_aware_once",
            "refresh_on_upgrade",
        ],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["semantic_clones", "ann_neighbors"],
        label: "ANN neighbours",
        description: "Nearest-neighbour count for semantic clone lookup.",
        field_type: "integer",
        default_value: Some("5"),
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["semantic_clones", "summary_workers"],
        label: "Summary workers",
        description: "Concurrent summary worker count.",
        field_type: "integer",
        default_value: Some("1"),
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["semantic_clones", "embedding_workers"],
        label: "Embedding workers",
        description: "Concurrent embedding worker count.",
        field_type: "integer",
        default_value: Some("1"),
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["semantic_clones", "clone_rebuild_workers"],
        label: "Clone rebuild workers",
        description: "Concurrent clone rebuild worker count.",
        field_type: "integer",
        default_value: Some("1"),
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
];

const DAEMON_DASHBOARD_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        path: &["dashboard", "local_dashboard", "tls"],
        label: "Local dashboard TLS",
        description: "Serve the local dashboard over HTTPS when available.",
        field_type: "boolean",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["dashboard", "bundle_dir"],
        label: "Bundle directory",
        description: "Optional dashboard bundle override directory.",
        field_type: "string",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
];

const DAEMON_PROVIDER_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        path: &["knowledge", "providers"],
        label: "Knowledge providers",
        description: "Provider configuration. Secret values are redacted in snapshots.",
        field_type: "json",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["inference"],
        label: "Inference",
        description: "Inference runtimes and profiles.",
        field_type: "json",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
];

const REPO_CAPTURE_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        path: &["capture", "enabled"],
        label: "Capture enabled",
        description: "Enable Bitloops capture for this config scope.",
        field_type: "boolean",
        default_value: Some("true"),
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["capture", "strategy"],
        label: "Capture strategy",
        description: "Checkpoint strategy for this config scope.",
        field_type: "string",
        default_value: Some("manual-commit"),
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
];

const REPO_AGENT_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        path: &["agents", "supported"],
        label: "Supported agents",
        description: "Agents enabled for Bitloops prompt surfaces.",
        field_type: "json",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["agents", "devql_guidance_enabled"],
        label: "DevQL guidance",
        description: "Enable local DevQL guidance surfaces for supported agents.",
        field_type: "boolean",
        default_value: Some("true"),
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
];

const REPO_SCOPE_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        path: &["scope", "exclude"],
        label: "Exclude",
        description: "Path patterns excluded from capture.",
        field_type: "json",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["scope", "exclude_from"],
        label: "Exclude from files",
        description: "Files containing additional exclusion patterns.",
        field_type: "json",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
];

const REPO_WATCH_FIELDS: &[FieldSpec] = &[FieldSpec {
    path: &["watch"],
    label: "Watch",
    description: "Watcher settings for this config scope.",
    field_type: "json",
    default_value: None,
    allowed_values: &[],
    required: false,
    read_only: false,
    secret: false,
}];

const REPO_CONTEXT_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        path: &["contexts"],
        label: "Contexts",
        description: "Context definitions for this config scope.",
        field_type: "json",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
    FieldSpec {
        path: &["imports", "knowledge"],
        label: "Knowledge imports",
        description: "Imported knowledge config files.",
        field_type: "json",
        default_value: None,
        allowed_values: &[],
        required: false,
        read_only: false,
        secret: false,
    },
];

const REPO_DAEMON_FIELDS: &[FieldSpec] = &[FieldSpec {
    path: &["daemon", "config_path"],
    label: "Daemon config path",
    description: "Local binding to the daemon config. This belongs in .bitloops.local.toml.",
    field_type: "string",
    default_value: None,
    allowed_values: &[],
    required: false,
    read_only: false,
    secret: false,
}];

fn daemon_section_specs() -> Vec<SectionSpec> {
    vec![
        SectionSpec {
            key: "runtime",
            title: "Runtime",
            description: "Daemon runtime behaviour.",
            order: 10,
            fields: DAEMON_RUNTIME_FIELDS,
        },
        SectionSpec {
            key: "telemetry",
            title: "Telemetry",
            description: "Telemetry consent settings.",
            order: 20,
            fields: DAEMON_TELEMETRY_FIELDS,
        },
        SectionSpec {
            key: "logging",
            title: "Logging",
            description: "Daemon log output.",
            order: 30,
            fields: DAEMON_LOGGING_FIELDS,
        },
        SectionSpec {
            key: "stores",
            title: "Stores",
            description: "Local storage backend paths.",
            order: 40,
            fields: DAEMON_STORE_FIELDS,
        },
        SectionSpec {
            key: "semantic_clones",
            title: "Semantic clones",
            description: "Semantic clone enrichment settings.",
            order: 50,
            fields: DAEMON_SEMANTIC_FIELDS,
        },
        SectionSpec {
            key: "dashboard",
            title: "Dashboard",
            description: "Local dashboard runtime settings.",
            order: 60,
            fields: DAEMON_DASHBOARD_FIELDS,
        },
        SectionSpec {
            key: "knowledge",
            title: "Providers and inference",
            description: "Provider and inference configuration.",
            order: 70,
            fields: DAEMON_PROVIDER_FIELDS,
        },
    ]
}

fn repo_section_specs() -> Vec<SectionSpec> {
    vec![
        SectionSpec {
            key: "capture",
            title: "Capture",
            description: "Checkpoint capture policy for this config scope.",
            order: 10,
            fields: REPO_CAPTURE_FIELDS,
        },
        SectionSpec {
            key: "agents",
            title: "Agents",
            description: "Agent prompt-surface configuration.",
            order: 20,
            fields: REPO_AGENT_FIELDS,
        },
        SectionSpec {
            key: "scope",
            title: "Scope",
            description: "Repository scope filters.",
            order: 30,
            fields: REPO_SCOPE_FIELDS,
        },
        SectionSpec {
            key: "watch",
            title: "Watch",
            description: "Watcher behaviour for this config scope.",
            order: 40,
            fields: REPO_WATCH_FIELDS,
        },
        SectionSpec {
            key: "contexts",
            title: "Contexts and imports",
            description: "Context and imported knowledge configuration.",
            order: 50,
            fields: REPO_CONTEXT_FIELDS,
        },
        SectionSpec {
            key: "daemon",
            title: "Daemon binding",
            description: "Local daemon binding for this config scope.",
            order: 60,
            fields: REPO_DAEMON_FIELDS,
        },
    ]
}

fn title_from_key(key: &str) -> String {
    key.split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
