use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result as AnyhowResult, anyhow, bail};
use async_graphql::{ID, InputObject, SimpleObject, types::Json};
use serde_json::{Map, Number, Value, json};
use sha2::{Digest, Sha256};
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue, de::from_str};

use crate::api::DashboardState;
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, REPO_POLICY_FILE_NAME, REPO_POLICY_LOCAL_FILE_NAME,
    validate_daemon_config_text, validate_repo_policy_text,
};
use crate::graphql::{bad_user_input_error, graphql_error};

type ConfigJsonScalar = Json<Value>;

const REDACTED_VALUE: &str = "********";
const MAX_CONFIG_SCAN_DIRS: usize = 20_000;
const SKIPPED_SCAN_DIRS: [&str; 9] = [
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    "coverage",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigTargetKind {
    Daemon,
    RepoShared,
    RepoLocal,
}

impl ConfigTargetKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
            Self::RepoShared => "repo_shared",
            Self::RepoLocal => "repo_local",
        }
    }

    fn scope_label(&self) -> &'static str {
        match self {
            Self::Daemon => "Daemon",
            Self::RepoShared => "Shared repo",
            Self::RepoLocal => "Local repo",
        }
    }

    fn from_path(path: &Path) -> Option<Self> {
        match path.file_name().and_then(|name| name.to_str()) {
            Some(BITLOOPS_CONFIG_RELATIVE_PATH) => Some(Self::Daemon),
            Some(REPO_POLICY_FILE_NAME) => Some(Self::RepoShared),
            Some(REPO_POLICY_LOCAL_FILE_NAME) => Some(Self::RepoLocal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct ConfigTarget {
    id: ID,
    kind: ConfigTargetKind,
    label: String,
    group: String,
    path: PathBuf,
    repo_root: Option<PathBuf>,
    exists: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeConfigTargetObject {
    pub(crate) id: ID,
    pub(crate) kind: String,
    pub(crate) scope: String,
    pub(crate) label: String,
    pub(crate) group: String,
    pub(crate) path: String,
    pub(crate) repo_root: Option<String>,
    pub(crate) exists: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeConfigFieldObject {
    pub(crate) key: String,
    pub(crate) path: Vec<String>,
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) field_type: String,
    pub(crate) value: ConfigJsonScalar,
    pub(crate) effective_value: Option<ConfigJsonScalar>,
    pub(crate) default_value: Option<ConfigJsonScalar>,
    pub(crate) allowed_values: Vec<String>,
    pub(crate) validation_hints: Vec<String>,
    pub(crate) required: bool,
    pub(crate) read_only: bool,
    pub(crate) secret: bool,
    pub(crate) order: i32,
    pub(crate) source: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeConfigSectionObject {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) order: i32,
    pub(crate) advanced: bool,
    pub(crate) fields: Vec<RuntimeConfigFieldObject>,
    pub(crate) value: ConfigJsonScalar,
    pub(crate) effective_value: Option<ConfigJsonScalar>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeConfigSnapshotObject {
    pub(crate) target: RuntimeConfigTargetObject,
    pub(crate) revision: String,
    pub(crate) valid: bool,
    pub(crate) validation_errors: Vec<String>,
    pub(crate) restart_required: bool,
    pub(crate) reload_required: bool,
    pub(crate) sections: Vec<RuntimeConfigSectionObject>,
    pub(crate) raw_value: ConfigJsonScalar,
    pub(crate) effective_value: Option<ConfigJsonScalar>,
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct RuntimeConfigFieldPatchInput {
    pub(crate) path: Vec<String>,
    #[graphql(default)]
    pub(crate) value: Option<ConfigJsonScalar>,
    #[graphql(default)]
    pub(crate) unset: Option<bool>,
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct UpdateRuntimeConfigInput {
    pub(crate) target_id: ID,
    pub(crate) expected_revision: String,
    pub(crate) patches: Vec<RuntimeConfigFieldPatchInput>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct UpdateRuntimeConfigResult {
    pub(crate) snapshot: RuntimeConfigSnapshotObject,
    pub(crate) restart_required: bool,
    pub(crate) reload_required: bool,
    pub(crate) path: String,
    pub(crate) message: String,
}

pub(crate) async fn list_config_targets(
    state: &DashboardState,
) -> async_graphql::Result<Vec<RuntimeConfigTargetObject>> {
    discover_config_targets(state)
        .await
        .map(|targets| targets.into_iter().map(Into::into).collect())
        .map_err(internal_config_error)
}

pub(crate) async fn load_config_snapshot(
    state: &DashboardState,
    target_id: &ID,
) -> async_graphql::Result<RuntimeConfigSnapshotObject> {
    let target = resolve_target(state, target_id).await?;
    build_snapshot(&target).map_err(map_snapshot_error)
}

pub(crate) async fn update_config(
    state: &DashboardState,
    input: UpdateRuntimeConfigInput,
) -> async_graphql::Result<UpdateRuntimeConfigResult> {
    let target = resolve_target(state, &input.target_id).await?;
    if !target.exists {
        return Err(bad_user_input_error(format!(
            "config target {} does not exist",
            target.path.display()
        )));
    }

    let original = fs::read_to_string(&target.path).map_err(|err| {
        internal_config_error(anyhow!(
            "failed to read config target {}: {err}",
            target.path.display()
        ))
    })?;
    let current_revision = revision_for_bytes(original.as_bytes());
    if current_revision != input.expected_revision {
        return Err(bad_user_input_error(format!(
            "config target changed on disk; reload {} before saving",
            target.path.display()
        )));
    }

    let mut doc = original.parse::<DocumentMut>().map_err(|err| {
        bad_user_input_error(format!(
            "failed to parse config target {}: {err}",
            target.path.display()
        ))
    })?;

    for patch in input.patches {
        apply_patch_to_document(&mut doc, patch)
            .map_err(|err| bad_user_input_error(format!("{err:#}")))?;
    }

    let next = doc.to_string();
    validate_target_text(&target, &next).map_err(|err| {
        bad_user_input_error(format!(
            "updated config is invalid for {}: {err:#}",
            target.path.display()
        ))
    })?;
    write_atomic(&target.path, next.as_bytes()).map_err(internal_config_error)?;

    let snapshot = build_snapshot(&target).map_err(map_snapshot_error)?;
    Ok(UpdateRuntimeConfigResult {
        restart_required: snapshot.restart_required,
        reload_required: snapshot.reload_required,
        path: target.path.display().to_string(),
        snapshot,
        message: "Configuration saved.".to_string(),
    })
}

async fn resolve_target(
    state: &DashboardState,
    target_id: &ID,
) -> async_graphql::Result<ConfigTarget> {
    let targets = discover_config_targets(state)
        .await
        .map_err(internal_config_error)?;
    targets
        .into_iter()
        .find(|target| target.id == *target_id)
        .ok_or_else(|| bad_user_input_error(format!("unknown config target `{target_id:?}`")))
}

async fn discover_config_targets(state: &DashboardState) -> AnyhowResult<Vec<ConfigTarget>> {
    let mut targets = BTreeMap::<String, ConfigTarget>::new();
    let daemon_path = canonicalize_lossy(&state.config_path);
    let daemon = ConfigTarget {
        id: target_id(ConfigTargetKind::Daemon.as_str(), &daemon_path),
        kind: ConfigTargetKind::Daemon,
        label: "Daemon config".to_string(),
        group: "Daemon".to_string(),
        path: daemon_path.clone(),
        repo_root: None,
        exists: daemon_path.is_file(),
    };
    targets.insert(daemon.path.display().to_string(), daemon);

    for root in known_repo_roots(state).await {
        scan_repo_config_targets(&root, &mut targets)?;
    }

    let mut values = targets.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        target_sort_key(left)
            .cmp(&target_sort_key(right))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(values)
}

async fn known_repo_roots(state: &DashboardState) -> BTreeSet<PathBuf> {
    let mut roots = BTreeSet::new();
    roots.insert(canonicalize_lossy(&state.repo_root));

    let context = crate::graphql::DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(Path::to_path_buf),
        state.db.clone(),
    );

    match context.list_known_repositories().await {
        Ok(repositories) => {
            for repository in repositories {
                if let Some(repo_root) = repository.repo_root() {
                    roots.insert(canonicalize_lossy(repo_root));
                }
            }
        }
        Err(err) => {
            log::debug!("config target discovery could not load known repositories: {err:#}");
        }
    }

    roots
}

fn scan_repo_config_targets(
    repo_root: &Path,
    targets: &mut BTreeMap<String, ConfigTarget>,
) -> AnyhowResult<()> {
    let mut queue = VecDeque::from([canonicalize_lossy(repo_root)]);
    let mut visited = BTreeSet::new();
    let mut scanned = 0usize;

    while let Some(directory) = queue.pop_front() {
        if !visited.insert(directory.clone()) {
            continue;
        }
        scanned += 1;
        if scanned > MAX_CONFIG_SCAN_DIRS {
            log::warn!(
                "config target discovery stopped after scanning {MAX_CONFIG_SCAN_DIRS} directories under {}",
                repo_root.display()
            );
            break;
        }

        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(err) => {
                log::debug!(
                    "config target discovery skipped unreadable directory {}: {err}",
                    directory.display()
                );
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if file_type.is_dir() {
                if !file_type.is_symlink() && !SKIPPED_SCAN_DIRS.contains(&name.as_ref()) {
                    queue.push_back(path);
                }
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            let Some(kind) = ConfigTargetKind::from_path(&path) else {
                continue;
            };
            if kind == ConfigTargetKind::Daemon {
                continue;
            }
            let canonical_path = canonicalize_lossy(&path);
            let root =
                canonicalize_lossy(canonical_path.parent().unwrap_or_else(|| Path::new("/")));
            let label = match kind {
                ConfigTargetKind::RepoShared => ".bitloops.toml".to_string(),
                ConfigTargetKind::RepoLocal => ".bitloops.local.toml".to_string(),
                ConfigTargetKind::Daemon => unreachable!("daemon targets are handled separately"),
            };
            let display_root = root
                .strip_prefix(repo_root)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
                .map(|relative| relative.display().to_string())
                .unwrap_or_else(|| repo_root.display().to_string());
            let target = ConfigTarget {
                id: target_id(kind.as_str(), &canonical_path),
                kind,
                label,
                group: display_root,
                path: canonical_path,
                repo_root: Some(canonicalize_lossy(repo_root)),
                exists: true,
            };
            targets.insert(target.path.display().to_string(), target);
        }
    }

    Ok(())
}

fn target_sort_key(target: &ConfigTarget) -> (u8, String, String) {
    let rank = match target.kind {
        ConfigTargetKind::Daemon => 0,
        ConfigTargetKind::RepoShared => 1,
        ConfigTargetKind::RepoLocal => 2,
    };
    (rank, target.group.clone(), target.label.clone())
}

fn build_snapshot(target: &ConfigTarget) -> AnyhowResult<RuntimeConfigSnapshotObject> {
    let raw = fs::read_to_string(&target.path)
        .with_context(|| format!("reading config target {}", target.path.display()))?;
    let revision = revision_for_bytes(raw.as_bytes());
    let value = parse_toml_value(&raw, &target.path)?;
    let redacted_value = redact_json_value(&value);
    let validation = validate_target_text(target, &raw)
        .map(|_| Vec::new())
        .unwrap_or_else(|err| vec![format!("{err:#}")]);
    let effective = effective_value_for_target(target, &value);
    let sections = build_sections_for_target(target, &value, effective.as_ref());

    Ok(RuntimeConfigSnapshotObject {
        target: target.clone().into(),
        revision,
        valid: validation.is_empty(),
        validation_errors: validation,
        restart_required: target.kind == ConfigTargetKind::Daemon,
        reload_required: target.kind != ConfigTargetKind::Daemon,
        sections,
        raw_value: Json(redacted_value),
        effective_value: effective.map(|value| Json(redact_json_value(&value))),
    })
}

fn parse_toml_value(raw: &str, path: &Path) -> AnyhowResult<Value> {
    from_str::<Value>(raw).with_context(|| format!("parsing config target {}", path.display()))
}

fn effective_value_for_target(target: &ConfigTarget, value: &Value) -> Option<Value> {
    match target.kind {
        ConfigTargetKind::Daemon => Some(value.clone()),
        ConfigTargetKind::RepoShared | ConfigTargetKind::RepoLocal => {
            let root = target.path.parent()?;
            crate::config::discover_repo_policy_optional(root)
                .ok()
                .map(repo_policy_snapshot_to_value)
        }
    }
}

fn repo_policy_snapshot_to_value(snapshot: crate::config::RepoPolicySnapshot) -> Value {
    json!({
        "capture": snapshot.capture,
        "watch": snapshot.watch,
        "scope": snapshot.scope,
        "contexts": snapshot.contexts,
        "agents": snapshot.agents,
        "imports": {
            "knowledge": snapshot.knowledge_import_paths,
        },
        "daemon": {
            "config_path": snapshot.daemon_config_path.map(|path| path.display().to_string()),
        },
    })
}

fn build_sections_for_target(
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

fn apply_patch_to_document(
    doc: &mut DocumentMut,
    patch: RuntimeConfigFieldPatchInput,
) -> AnyhowResult<()> {
    if patch.path.is_empty() {
        bail!("patch path cannot be empty");
    }
    if patch
        .path
        .iter()
        .any(|segment| segment.trim().is_empty() || segment.contains('\0'))
    {
        bail!("patch path contains an invalid segment");
    }

    if patch.unset.unwrap_or(false) || patch.value.as_ref().is_none_or(|value| value.0.is_null()) {
        remove_path(doc, &patch.path);
        return Ok(());
    }

    let value = patch
        .value
        .map(|value| value.0)
        .ok_or_else(|| anyhow!("patch value is required unless unset is true"))?;
    if value == Value::String(REDACTED_VALUE.to_string()) && is_secret_path_segments(&patch.path) {
        return Ok(());
    }
    let value = toml_item_at_path(doc, &patch.path)
        .and_then(toml_item_to_json_value)
        .map(|original| preserve_redacted_placeholders(value.clone(), &original))
        .unwrap_or(value);
    set_path(doc, &patch.path, json_value_to_toml_item(&value)?)?;
    Ok(())
}

fn set_path(doc: &mut DocumentMut, path: &[String], value: Item) -> AnyhowResult<()> {
    if path.len() == 1 {
        doc[&path[0]] = value;
        return Ok(());
    }

    if doc.get(&path[0]).is_none_or(|item| !item.is_table()) {
        doc[&path[0]] = Item::Table(Table::new());
    }

    let mut table = doc[&path[0]]
        .as_table_mut()
        .ok_or_else(|| anyhow!("{} is not a table", path[0]))?;
    for segment in &path[1..path.len() - 1] {
        if table.get(segment).is_none_or(|item| !item.is_table()) {
            table[segment] = Item::Table(Table::new());
        }
        table = table[segment]
            .as_table_mut()
            .ok_or_else(|| anyhow!("{segment} is not a table"))?;
    }

    table[path.last().expect("path is non-empty")] = value;
    Ok(())
}

fn remove_path(doc: &mut DocumentMut, path: &[String]) {
    if path.len() == 1 {
        doc.as_table_mut().remove(&path[0]);
        return;
    }
    let Some(mut table) = doc.get_mut(&path[0]).and_then(Item::as_table_mut) else {
        return;
    };
    for segment in &path[1..path.len() - 1] {
        let Some(next) = table.get_mut(segment).and_then(Item::as_table_mut) else {
            return;
        };
        table = next;
    }
    if let Some(last) = path.last() {
        table.remove(last);
    }
}

fn json_value_to_toml_item(value: &Value) -> AnyhowResult<Item> {
    match value {
        Value::Null => Ok(Item::None),
        Value::Bool(value) => Ok(Item::Value(TomlValue::from(*value))),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                return Ok(Item::Value(TomlValue::from(value)));
            }
            if let Some(value) = number.as_u64() {
                return i64::try_from(value)
                    .map(|value| Item::Value(TomlValue::from(value)))
                    .map_err(|_| anyhow!("TOML integer value is too large: {number}"));
            }
            if let Some(value) = number.as_f64() {
                return Ok(Item::Value(TomlValue::from(value)));
            }
            bail!("unsupported numeric config value `{number}`")
        }
        Value::String(value) => Ok(Item::Value(TomlValue::from(value.as_str()))),
        Value::Array(values) => {
            let mut array = Array::new();
            for value in values {
                let Item::Value(value) = json_value_to_toml_item(value)? else {
                    bail!("TOML arrays may only contain scalar values")
                };
                array.push(value);
            }
            Ok(Item::Value(TomlValue::Array(array)))
        }
        Value::Object(map) => {
            let mut table = Table::new();
            for (key, value) in map {
                table[key] = json_value_to_toml_item(value)?;
            }
            Ok(Item::Table(table))
        }
    }
}

fn toml_item_at_path<'a>(doc: &'a DocumentMut, path: &[String]) -> Option<&'a Item> {
    let mut item = doc.get(path.first()?)?;
    for segment in &path[1..] {
        item = item.as_table()?.get(segment)?;
    }
    Some(item)
}

fn toml_item_to_json_value(item: &Item) -> Option<Value> {
    match item {
        Item::None => Some(Value::Null),
        Item::Value(value) => Some(toml_value_to_json_value(value)),
        Item::Table(table) => Some(Value::Object(
            table
                .iter()
                .filter_map(|(key, item)| {
                    toml_item_to_json_value(item).map(|value| (key.to_string(), value))
                })
                .collect(),
        )),
        Item::ArrayOfTables(tables) => Some(Value::Array(
            tables
                .iter()
                .map(|table| {
                    Value::Object(
                        table
                            .iter()
                            .filter_map(|(key, item)| {
                                toml_item_to_json_value(item).map(|value| (key.to_string(), value))
                            })
                            .collect(),
                    )
                })
                .collect(),
        )),
    }
}

fn toml_value_to_json_value(value: &TomlValue) -> Value {
    if let Some(value) = value.as_str() {
        return Value::String(value.to_string());
    }
    if let Some(value) = value.as_integer() {
        return Value::Number(value.into());
    }
    if let Some(value) = value.as_float() {
        return Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(value.to_string()));
    }
    if let Some(value) = value.as_bool() {
        return Value::Bool(value);
    }
    if let Some(value) = value.as_datetime() {
        return Value::String(value.to_string());
    }
    if let Some(array) = value.as_array() {
        return Value::Array(array.iter().map(toml_value_to_json_value).collect());
    }
    if let Some(table) = value.as_inline_table() {
        return Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_string(), toml_value_to_json_value(value)))
                .collect(),
        );
    }
    Value::Null
}

fn preserve_redacted_placeholders(next: Value, original: &Value) -> Value {
    match (next, original) {
        (Value::String(value), original) if value == REDACTED_VALUE => original.clone(),
        (Value::Object(next), Value::Object(original)) => Value::Object(
            next.into_iter()
                .map(|(key, value)| {
                    let value = original
                        .get(&key)
                        .map(|original| preserve_redacted_placeholders(value.clone(), original))
                        .unwrap_or(value);
                    (key, value)
                })
                .collect(),
        ),
        (Value::Array(next), Value::Array(original)) => Value::Array(
            next.into_iter()
                .enumerate()
                .map(|(index, value)| {
                    original
                        .get(index)
                        .map(|original| preserve_redacted_placeholders(value.clone(), original))
                        .unwrap_or(value)
                })
                .collect(),
        ),
        (next, _) => next,
    }
}

fn validate_target_text(target: &ConfigTarget, text: &str) -> AnyhowResult<()> {
    match target.kind {
        ConfigTargetKind::Daemon => validate_daemon_config_text(text, &target.path),
        ConfigTargetKind::RepoShared | ConfigTargetKind::RepoLocal => {
            validate_repo_policy_text(text, &target.path)
        }
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> AnyhowResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("config target has no parent directory: {}", path.display()))?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("bitloops-config"),
        std::process::id()
    ));
    fs::write(&tmp, bytes)
        .with_context(|| format!("writing temporary config file {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "renaming temporary config file {} to {}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn redact_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    if is_secret_segment(key) {
                        (key.clone(), Value::String(REDACTED_VALUE.to_string()))
                    } else {
                        (key.clone(), redact_json_value(value))
                    }
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_json_value).collect()),
        _ => value.clone(),
    }
}

fn is_secret_path(path: &[&str]) -> bool {
    path.iter().any(|segment| is_secret_segment(segment))
}

fn is_secret_path_segments(path: &[String]) -> bool {
    path.iter().any(|segment| is_secret_segment(segment))
}

fn is_secret_segment(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    lower.contains("token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("api_key")
        || lower.contains("credentials")
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

fn target_id(kind: &str, path: &Path) -> ID {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update(b"\n");
    hasher.update(path.to_string_lossy().as_bytes());
    ID::from(hex_digest(hasher.finalize().as_slice()))
}

fn revision_for_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher.finalize().as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn canonicalize_lossy(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn internal_config_error(error: anyhow::Error) -> async_graphql::Error {
    graphql_error("internal", format!("{error:#}"))
}

fn map_snapshot_error(error: anyhow::Error) -> async_graphql::Error {
    if error.to_string().contains("No such file") {
        return bad_user_input_error(format!("{error:#}"));
    }
    internal_config_error(error)
}

impl From<ConfigTarget> for RuntimeConfigTargetObject {
    fn from(target: ConfigTarget) -> Self {
        Self {
            id: target.id,
            kind: target.kind.as_str().to_string(),
            scope: target.kind.scope_label().to_string(),
            label: target.label,
            group: target.group,
            path: target.path.display().to_string(),
            repo_root: target.repo_root.map(|path| path.display().to_string()),
            exists: target.exists,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, text).expect("write file");
    }

    #[test]
    fn scan_repo_config_targets_finds_root_and_nested_policy_files() {
        let temp = TempDir::new().expect("temp dir");
        write(
            &temp.path().join(REPO_POLICY_FILE_NAME),
            "[capture]\nenabled = true\n",
        );
        write(
            &temp
                .path()
                .join("packages")
                .join("app")
                .join(REPO_POLICY_LOCAL_FILE_NAME),
            "[capture]\nstrategy = \"manual-commit\"\n",
        );
        write(
            &temp
                .path()
                .join("target")
                .join("ignored")
                .join(REPO_POLICY_FILE_NAME),
            "[capture]\nenabled = false\n",
        );

        let mut targets = BTreeMap::new();
        scan_repo_config_targets(temp.path(), &mut targets).expect("scan targets");

        let temp_root = canonicalize_lossy(temp.path());
        let paths = targets
            .values()
            .map(|target| {
                target
                    .path
                    .strip_prefix(&temp_root)
                    .expect("target under temp")
                    .display()
                    .to_string()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            paths,
            BTreeSet::from([
                REPO_POLICY_FILE_NAME.to_string(),
                format!("packages/app/{REPO_POLICY_LOCAL_FILE_NAME}"),
            ])
        );
    }

    #[test]
    fn snapshot_redacts_secret_values() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
        write(
            &path,
            r#"[runtime]
local_dev = false

[knowledge.providers.github]
token = "secret-token"
"#,
        );
        let target = ConfigTarget {
            id: target_id("daemon", &path),
            kind: ConfigTargetKind::Daemon,
            label: "Daemon config".to_string(),
            group: "Daemon".to_string(),
            path,
            repo_root: None,
            exists: true,
        };

        let snapshot = build_snapshot(&target).expect("snapshot");
        assert_eq!(
            snapshot.raw_value.0["knowledge"]["providers"]["github"]["token"],
            Value::String(REDACTED_VALUE.to_string())
        );
    }

    #[test]
    fn apply_patch_preserves_unrelated_toml_and_updates_nested_value() {
        let original = r#"# keep me
[runtime]
local_dev = false

[stores.relational]
sqlite_path = "old.db"
"#;
        let mut doc = original.parse::<DocumentMut>().expect("parse toml");
        apply_patch_to_document(
            &mut doc,
            RuntimeConfigFieldPatchInput {
                path: vec![
                    "stores".to_string(),
                    "relational".to_string(),
                    "sqlite_path".to_string(),
                ],
                value: Some(Json(Value::String("new.db".to_string()))),
                unset: None,
            },
        )
        .expect("apply patch");
        let updated = doc.to_string();
        assert!(updated.contains("# keep me"));
        assert!(updated.contains("local_dev = false"));
        assert!(updated.contains("sqlite_path = \"new.db\""));
    }

    #[test]
    fn apply_patch_preserves_redacted_nested_secret_values() {
        let original = r#"[knowledge.providers.github]
token = "secret-token"
enabled = true
"#;
        let mut doc = original.parse::<DocumentMut>().expect("parse toml");
        apply_patch_to_document(
            &mut doc,
            RuntimeConfigFieldPatchInput {
                path: vec!["knowledge".to_string(), "providers".to_string()],
                value: Some(Json(json!({
                    "github": {
                        "token": REDACTED_VALUE,
                        "enabled": false,
                    },
                }))),
                unset: None,
            },
        )
        .expect("apply patch");
        let updated = doc.to_string();
        assert!(updated.contains("token = \"secret-token\""));
        assert!(updated.contains("enabled = false"));
    }
}
