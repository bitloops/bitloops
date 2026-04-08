use super::*;
use anyhow::{anyhow, bail};

// Query filter types (defined here; used by query_parser, query_executor, deps_query).

#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedDevqlQuery {
    pub(crate) repo: Option<String>,
    pub(crate) project_path: Option<String>,
    pub(crate) as_of: Option<AsOfSelector>,
    pub(crate) select_artefacts: Option<SelectArtefactsFilter>,
    pub(crate) file: Option<String>,
    pub(crate) files_path: Option<String>,
    pub(crate) artefacts: ArtefactFilter,
    pub(super) clones: CloneFilter,
    pub(super) checkpoints: CheckpointFilter,
    pub(super) telemetry: TelemetryFilter,
    pub(super) deps: DepsFilter,
    pub(super) has_artefacts_stage: bool,
    pub(super) has_clones_stage: bool,
    pub(super) has_deps_stage: bool,
    pub(super) has_checkpoints_stage: bool,
    pub(super) has_telemetry_stage: bool,
    pub(super) has_chat_history_stage: bool,
    pub(super) registered_stages: Vec<RegisteredStageCall>,
    pub(crate) limit: usize,
    pub(super) has_limit_stage: bool,
    pub(super) select_fields: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RegisteredStageCall {
    pub(super) stage_name: String,
    pub(super) args: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub(crate) enum AsOfSelector {
    Ref(String),
    Commit(String),
    SaveCurrent,
    SaveRevision(String),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ArtefactFilter {
    pub(crate) kind: Option<String>,
    pub(crate) symbol_fqn: Option<String>,
    pub(crate) lines: Option<(i32, i32)>,
    pub(crate) agent: Option<String>,
    pub(crate) since: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SelectArtefactsFilter {
    pub(crate) symbol_fqn: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) lines: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CloneFilter {
    pub(super) relation_kind: Option<String>,
    pub(super) min_score: Option<f32>,
    pub(super) raw: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CheckpointFilter {
    pub(super) agent: Option<String>,
    pub(super) since: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct TelemetryFilter {
    pub(super) event_type: Option<String>,
    pub(super) agent: Option<String>,
    pub(super) since: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct DepsFilter {
    pub(super) kind: Option<DepsKind>,
    pub(super) direction: DepsDirection,
    pub(super) include_unresolved: bool,
}

impl Default for DepsFilter {
    fn default() -> Self {
        Self {
            kind: None,
            direction: DepsDirection::Out,
            include_unresolved: true,
        }
    }
}

pub(crate) fn parse_devql_query(query: &str) -> Result<ParsedDevqlQuery> {
    let mut parsed = ParsedDevqlQuery {
        limit: 100,
        ..Default::default()
    };

    let stages = query
        .split("->")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    if stages.is_empty() {
        bail!("empty DevQL query")
    }

    for stage in stages {
        if let Some(inner) = stage
            .strip_prefix("repo(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.repo = Some(parse_single_quoted_or_double(inner)?);
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("project(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.project_path = Some(parse_single_quoted_or_double(inner)?);
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("asOf(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            if let Some(commit) = args.get("commit") {
                parsed.as_of = Some(AsOfSelector::Commit(commit.clone()));
            } else if let Some(reference) = args.get("ref") {
                parsed.as_of = Some(AsOfSelector::Ref(reference.clone()));
            } else if let Some(save) = args.get("save") {
                if save.eq_ignore_ascii_case("current") {
                    parsed.as_of = Some(AsOfSelector::SaveCurrent);
                } else {
                    bail!("asOf(save:...) only supports save:\"current\"")
                }
            } else if let Some(revision) = args.get("saveRevision") {
                parsed.as_of = Some(AsOfSelector::SaveRevision(revision.clone()));
            } else {
                bail!("asOf(...) requires `commit:`, `ref:`, `save:`, or `saveRevision:`")
            }
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("selectArtefacts(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.deps.direction = DepsDirection::Both;
            parsed.deps.include_unresolved = false;
            parsed.select_artefacts = Some(SelectArtefactsFilter {
                symbol_fqn: args.get("symbol_fqn").cloned(),
                path: args.get("path").cloned(),
                lines: args
                    .get("lines")
                    .map(|lines| parse_lines_range(lines))
                    .transpose()?,
            });
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("file(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.file = Some(parse_single_quoted_or_double(inner)?);
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("files(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.files_path = args.get("path").cloned();
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("artefacts(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_artefacts_stage = true;
            parsed.artefacts.kind = args.get("kind").cloned();
            parsed.artefacts.symbol_fqn = args.get("symbol_fqn").cloned();
            parsed.artefacts.agent = args.get("agent").cloned();
            parsed.artefacts.since = args.get("since").cloned();
            if let Some(lines) = args.get("lines") {
                parsed.artefacts.lines = Some(parse_lines_range(lines)?);
            }
            continue;
        }

        if stage == "artefacts()" {
            parsed.has_artefacts_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("clones(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_clones_stage = true;
            parsed.clones.relation_kind = args.get("relation_kind").cloned();
            if let Some(min_score) = args.get("min_score") {
                parsed.clones.min_score = Some(
                    min_score
                        .parse::<f32>()
                        .map_err(|_| anyhow!("invalid clones min_score value: {min_score}"))?,
                );
            }
            if let Some(raw) = args.get("raw") {
                parsed.clones.raw = parse_bool_literal("clones raw", raw)?;
            }
            continue;
        }

        if stage == "clones()" {
            parsed.has_clones_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("deps(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_deps_stage = true;
            if let Some(kind) = args.get("kind") {
                parsed.deps.kind = Some(DepsKind::from_str(kind).ok_or_else(|| {
                    anyhow!(
                        "deps(kind:...) must be one of: {}",
                        DepsKind::all_names().join(", ")
                    )
                })?);
            }
            if let Some(direction) = args.get("direction") {
                parsed.deps.direction = DepsDirection::from_str(direction).ok_or_else(|| {
                    anyhow!(
                        "deps(direction:...) must be one of: {}",
                        DepsDirection::all_names().join(", ")
                    )
                })?;
            }
            if let Some(include_unresolved) = args.get("include_unresolved") {
                parsed.deps.include_unresolved = matches!(
                    include_unresolved.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes"
                );
            }
            continue;
        }

        if stage == "deps()" {
            parsed.has_deps_stage = true;
            continue;
        }

        if stage == "chatHistory()" {
            parsed.has_chat_history_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("checkpoints(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_checkpoints_stage = true;
            parsed.checkpoints.agent = args.get("agent").cloned();
            parsed.checkpoints.since = args.get("since").cloned();
            continue;
        }

        if stage == "checkpoints()" {
            parsed.has_checkpoints_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("telemetry(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_telemetry_stage = true;
            parsed.telemetry.event_type = args.get("event_type").cloned();
            parsed.telemetry.agent = args.get("agent").cloned();
            parsed.telemetry.since = args.get("since").cloned();
            continue;
        }

        if stage == "telemetry()" {
            parsed.has_telemetry_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("select(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.select_fields = inner
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("limit(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.limit = inner
                .trim()
                .parse::<usize>()
                .map_err(|_| anyhow!("invalid limit value: {inner}"))?;
            parsed.has_limit_stage = true;
            continue;
        }

        if let Some(call) = parse_registered_stage(stage)? {
            parsed.registered_stages.push(call);
            continue;
        }

        bail!("unsupported DevQL stage: {stage}")
    }

    Ok(parsed)
}

pub(super) fn validate_deps_filter(deps: &DepsFilter) -> Result<()> {
    let _ = deps;
    Ok(())
}

pub(super) fn parse_named_args(input: &str) -> Result<BTreeMap<String, String>> {
    let mut args = BTreeMap::new();
    if input.trim().is_empty() {
        return Ok(args);
    }

    let mut current = String::new();
    let mut pieces = Vec::new();
    let mut in_quotes = false;
    let mut quote_char = '\0';

    for ch in input.chars() {
        match ch {
            '\'' | '"' => {
                if in_quotes && ch == quote_char {
                    in_quotes = false;
                    quote_char = '\0';
                } else if !in_quotes {
                    in_quotes = true;
                    quote_char = ch;
                }
                current.push(ch);
            }
            ',' if !in_quotes => {
                pieces.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        pieces.push(current.trim().to_string());
    }

    for piece in pieces {
        let Some((key, value)) = piece.split_once(':') else {
            bail!("invalid argument segment: {piece}")
        };
        let key = key.trim().to_string();
        let value = value.trim();
        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_single_quoted_or_double(value)?
        } else {
            value.to_string()
        };
        args.insert(key, value);
    }

    Ok(args)
}

pub(super) fn parse_single_quoted_or_double(input: &str) -> Result<String> {
    let s = input.trim();
    if s.len() < 2 {
        bail!("expected quoted string, got: {input}")
    }

    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        return Ok(s[1..s.len() - 1].to_string());
    }

    bail!("expected quoted string, got: {input}")
}

pub(super) fn parse_lines_range(lines: &str) -> Result<(i32, i32)> {
    let Some((start, end)) = lines.split_once("..") else {
        bail!("invalid lines range: {lines}")
    };
    let start = start
        .trim()
        .parse::<i32>()
        .map_err(|_| anyhow!("invalid line start: {start}"))?;
    let end = end
        .trim()
        .parse::<i32>()
        .map_err(|_| anyhow!("invalid line end: {end}"))?;
    if start <= 0 || end <= 0 || end < start {
        bail!("invalid lines range: {lines}")
    }
    Ok((start, end))
}

pub(super) fn parse_bool_literal(field_name: &str, value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        _ => bail!("invalid boolean value for {field_name}: {value}"),
    }
}

pub(super) fn parse_registered_stage(stage: &str) -> Result<Option<RegisteredStageCall>> {
    let Some(open_idx) = stage.find('(') else {
        return Ok(None);
    };
    if !stage.ends_with(')') || open_idx == 0 {
        return Ok(None);
    }

    let stage_name = stage[..open_idx].trim();
    if stage_name.is_empty()
        || !stage_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Ok(None);
    }
    if stage_name.starts_with("__core_") {
        return Ok(None);
    }

    let args_raw = &stage[open_idx + 1..stage.len() - 1];
    let args = parse_named_args(args_raw)?;
    Ok(Some(RegisteredStageCall {
        stage_name: stage_name.to_string(),
        args,
    }))
}
