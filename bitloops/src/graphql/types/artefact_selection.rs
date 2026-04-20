use async_graphql::{ComplexObject, Context, Enum, InputObject, Result, SimpleObject, types::Json};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::graphql::pack_adapter::StageResolverAdapter;
use crate::graphql::{DevqlGraphqlContext, ResolverScope, backend_error, bad_user_input_error};

use super::{
    Artefact, Checkpoint, CloneSummary, ClonesFilterInput, DateTimeScalar, DependencyEdge,
    DepsDirection, EdgeKind, JsonScalar, LineRangeInput, TestHarnessTestsResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArtefactSelectorMode {
    SymbolFqn(String),
    FuzzyName(String),
    Path {
        path: String,
        lines: Option<LineRangeInput>,
    },
}

#[derive(Debug, Clone, InputObject)]
pub struct ArtefactSelectorInput {
    pub symbol_fqn: Option<String>,
    pub fuzzy_name: Option<String>,
    pub path: Option<String>,
    pub lines: Option<LineRangeInput>,
}

impl ArtefactSelectorInput {
    pub(crate) fn selection_mode(&self) -> Result<ArtefactSelectorMode> {
        let symbol_fqn = self
            .symbol_fqn
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let fuzzy_name = match self.fuzzy_name.as_deref() {
            Some(value) if value.trim().is_empty() => {
                return Err(bad_user_input_error(
                    "`selectArtefacts(by: ...)` requires a non-empty `fuzzyName`",
                ));
            }
            Some(value) => Some(value.trim().to_string()),
            None => None,
        };
        let path = self
            .path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let path_selector_requested = path.is_some() || self.lines.is_some();
        let selector_count = usize::from(symbol_fqn.is_some())
            + usize::from(fuzzy_name.is_some())
            + usize::from(path_selector_requested);
        if selector_count == 0 {
            return Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` requires exactly one selector mode",
            ));
        }
        if selector_count > 1 {
            return Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` allows exactly one of `symbolFqn`, `fuzzyName`, or `path`/`lines`",
            ));
        }
        if path_selector_requested && path.is_none() {
            return Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` requires `path` when `lines` is provided",
            ));
        }

        if let Some(symbol_fqn) = symbol_fqn {
            return Ok(ArtefactSelectorMode::SymbolFqn(symbol_fqn));
        }
        if let Some(fuzzy_name) = fuzzy_name {
            return Ok(ArtefactSelectorMode::FuzzyName(fuzzy_name));
        }

        let path = path.expect("selector_count ensures path selector exists");
        if let Some(lines) = self.lines.as_ref() {
            lines.validate()?;
        }
        Ok(ArtefactSelectorMode::Path {
            path,
            lines: self.lines.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum DirectoryEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct DirectoryEntry {
    pub path: String,
    pub name: String,
    pub entry_kind: DirectoryEntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtefactSelectionMode {
    Artefacts,
    DirectoryEntries,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct ArtefactSelection {
    pub count: IntCount,
    #[graphql(skip)]
    mode: ArtefactSelectionMode,
    #[graphql(skip)]
    pub(crate) artefacts: Vec<Artefact>,
    #[graphql(skip)]
    pub(crate) directory_entries: Vec<DirectoryEntry>,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

pub type IntCount = i32;

impl ArtefactSelection {
    pub(crate) fn new(
        artefacts: Vec<Artefact>,
        directory_entries: Vec<DirectoryEntry>,
        scope: ResolverScope,
    ) -> Self {
        Self {
            count: saturating_i32(artefacts.len()),
            mode: ArtefactSelectionMode::Artefacts,
            artefacts,
            directory_entries,
            scope,
        }
    }

    pub(crate) fn from_directory_entries(
        directory_entries: Vec<DirectoryEntry>,
        scope: ResolverScope,
    ) -> Self {
        Self {
            count: saturating_i32(directory_entries.len()),
            mode: ArtefactSelectionMode::DirectoryEntries,
            artefacts: Vec::new(),
            directory_entries,
            scope,
        }
    }

    fn artefact_ids(&self) -> Vec<String> {
        dedup_strings(self.artefacts.iter().map(|artefact| artefact.id.as_ref()))
    }

    fn symbol_ids(&self) -> Vec<String> {
        dedup_strings(
            self.artefacts
                .iter()
                .map(|artefact| artefact.symbol_id.as_str()),
        )
    }
}

#[derive(Debug, Clone)]
struct CheckpointStageData {
    summary: Value,
    schema: Option<String>,
    items: Vec<Checkpoint>,
}

#[derive(Debug, Clone)]
struct CloneStageData {
    summary: Value,
    schema: Option<String>,
    items: Vec<super::SemanticClone>,
}

#[derive(Debug, Clone)]
struct DependencyStageData {
    summary: Value,
    schema: Option<String>,
    items: Vec<DependencyEdge>,
}

#[derive(Debug, Clone)]
struct TestsStageData {
    summary: Value,
    schema: Option<String>,
    items: Vec<TestHarnessTestsResult>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct CheckpointStageResult {
    pub summary: JsonScalar,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<Checkpoint>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct CloneStageResult {
    pub summary: JsonScalar,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<super::SemanticClone>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct DependencyStageResult {
    pub summary: JsonScalar,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<DependencyEdge>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct TestsStageResult {
    pub summary: JsonScalar,
    pub schema: Option<String>,
    #[graphql(skip)]
    pub(crate) items: Vec<TestHarnessTestsResult>,
}

impl From<CheckpointStageData> for CheckpointStageResult {
    fn from(data: CheckpointStageData) -> Self {
        Self {
            summary: Json(data.summary),
            schema: data.schema,
            items: data.items,
        }
    }
}

impl From<CloneStageData> for CloneStageResult {
    fn from(data: CloneStageData) -> Self {
        Self {
            summary: Json(data.summary),
            schema: data.schema,
            items: data.items,
        }
    }
}

impl From<DependencyStageData> for DependencyStageResult {
    fn from(data: DependencyStageData) -> Self {
        Self {
            summary: Json(data.summary),
            schema: data.schema,
            items: data.items,
        }
    }
}

impl From<TestsStageData> for TestsStageResult {
    fn from(data: TestsStageData) -> Self {
        Self {
            summary: Json(data.summary),
            schema: data.schema,
            items: data.items,
        }
    }
}

#[ComplexObject]
impl ArtefactSelection {
    async fn summary(&self, ctx: &Context<'_>) -> Result<JsonScalar> {
        self.ensure_artefact_selection("summary")?;
        let checkpoints = self.resolve_checkpoint_stage_data(ctx, None, None).await?;
        let clones = self.resolve_clone_stage_data(ctx, None).await?;
        let deps = self
            .resolve_dependency_stage_data(ctx, None, DepsDirection::Both, true)
            .await?;
        let tests = self.resolve_tests_stage_data(ctx, None, None).await?;

        Ok(Json(build_selection_summary(
            self.artefacts.len(),
            &checkpoints,
            &clones,
            &deps,
            &tests,
        )))
    }

    async fn artefacts(&self, #[graphql(default = 20)] first: i32) -> Result<Vec<Artefact>> {
        self.ensure_artefact_selection("artefacts")?;
        take_stage_items(&self.artefacts, first)
    }

    async fn entries(&self, #[graphql(default = 20)] first: i32) -> Result<Vec<DirectoryEntry>> {
        self.ensure_directory_selection("entries")?;
        take_stage_items(&self.directory_entries, first)
    }

    async fn checkpoints(
        &self,
        ctx: &Context<'_>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
    ) -> Result<CheckpointStageResult> {
        self.ensure_artefact_selection("checkpoints")?;
        Ok(self
            .resolve_checkpoint_stage_data(ctx, agent.as_deref(), since.as_ref())
            .await?
            .into())
    }

    async fn clones(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "relationKind")] relation_kind: Option<String>,
        #[graphql(name = "minScore")] min_score: Option<f64>,
    ) -> Result<CloneStageResult> {
        self.ensure_artefact_selection("clones")?;
        let filter = ClonesFilterInput {
            relation_kind,
            min_score,
            neighbors: None,
        };
        Ok(self
            .resolve_clone_stage_data(ctx, Some(&filter))
            .await?
            .into())
    }

    async fn deps(
        &self,
        ctx: &Context<'_>,
        kind: Option<EdgeKind>,
        #[graphql(default_with = "DepsDirection::Both")] direction: DepsDirection,
        #[graphql(name = "includeUnresolved", default = true)] include_unresolved: bool,
    ) -> Result<DependencyStageResult> {
        self.ensure_artefact_selection("deps")?;
        Ok(self
            .resolve_dependency_stage_data(ctx, kind, direction, include_unresolved)
            .await?
            .into())
    }

    async fn tests(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "minConfidence")] min_confidence: Option<f64>,
        #[graphql(name = "linkageSource")] linkage_source: Option<String>,
    ) -> Result<TestsStageResult> {
        self.ensure_artefact_selection("tests")?;
        Ok(self
            .resolve_tests_stage_data(ctx, min_confidence, linkage_source)
            .await?
            .into())
    }
}

impl ArtefactSelection {
    fn ensure_artefact_selection(&self, field: &str) -> Result<()> {
        if self.mode == ArtefactSelectionMode::DirectoryEntries {
            return Err(bad_user_input_error(format!(
                "directory paths only support `entries`; `{field}` requires a file path instead"
            )));
        }
        Ok(())
    }

    fn ensure_directory_selection(&self, field: &str) -> Result<()> {
        if self.mode == ArtefactSelectionMode::Artefacts {
            return Err(bad_user_input_error(format!(
                "file paths do not support `{field}`; select a directory path instead"
            )));
        }
        Ok(())
    }

    async fn resolve_checkpoint_stage_data(
        &self,
        ctx: &Context<'_>,
        agent: Option<&str>,
        since: Option<&DateTimeScalar>,
    ) -> Result<CheckpointStageData> {
        let checkpoints = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_selected_symbol_checkpoints(&self.scope, &self.symbol_ids(), agent, since)
            .await
            .map_err(|err| {
                backend_error(format!("failed to resolve selected checkpoints: {err:#}"))
            })?;
        Ok(CheckpointStageData {
            summary: build_checkpoint_summary(&checkpoints),
            schema: (!checkpoints.is_empty()).then(|| CHECKPOINT_STAGE_SCHEMA.to_string()),
            items: checkpoints,
        })
    }

    async fn resolve_clone_stage_data(
        &self,
        ctx: &Context<'_>,
        filter: Option<&ClonesFilterInput>,
    ) -> Result<CloneStageData> {
        if let Some(filter) = filter {
            filter.validate()?;
        }
        let mut clones = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_selected_clones(&self.artefact_ids(), filter, &self.scope)
            .await
            .map_err(|err| backend_error(format!("failed to resolve selected clones: {err:#}")))?;
        clones.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.id.as_ref().cmp(right.id.as_ref()))
        });
        Ok(CloneStageData {
            summary: build_clone_summary(&clones),
            schema: (!clones.is_empty()).then(|| CLONE_STAGE_SCHEMA.to_string()),
            items: clones,
        })
    }

    async fn resolve_dependency_stage_data(
        &self,
        ctx: &Context<'_>,
        kind: Option<EdgeKind>,
        direction: DepsDirection,
        include_unresolved: bool,
    ) -> Result<DependencyStageData> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let filter = super::DepsFilterInput {
            kind,
            direction,
            include_unresolved,
        };
        let artefact_ids = self.artefact_ids();

        let outgoing = if matches!(direction, DepsDirection::Out | DepsDirection::Both) {
            context
                .load_dependency_edges_by_artefact_ids(
                    &artefact_ids,
                    DepsDirection::Out,
                    super::DepsFilterInput {
                        direction: DepsDirection::Out,
                        ..filter
                    },
                    &self.scope,
                )
                .await
                .map_err(|err| {
                    backend_error(format!(
                        "failed to resolve selected outgoing dependency edges: {err:#}"
                    ))
                })?
        } else {
            HashMap::new()
        };
        let incoming = if matches!(direction, DepsDirection::In | DepsDirection::Both) {
            context
                .load_dependency_edges_by_artefact_ids(
                    &artefact_ids,
                    DepsDirection::In,
                    super::DepsFilterInput {
                        direction: DepsDirection::In,
                        ..filter
                    },
                    &self.scope,
                )
                .await
                .map_err(|err| {
                    backend_error(format!(
                        "failed to resolve selected incoming dependency edges: {err:#}"
                    ))
                })?
        } else {
            HashMap::new()
        };

        let outgoing_edges = dedup_dependency_edges(outgoing.into_values().flatten().collect());
        let incoming_edges = dedup_dependency_edges(incoming.into_values().flatten().collect());
        let mut items = incoming_edges.clone();
        items.extend(outgoing_edges.clone());
        items = dedup_dependency_edges(items);
        items.sort_by(|left, right| {
            left.to_symbol_ref
                .as_deref()
                .unwrap_or("")
                .cmp(right.to_symbol_ref.as_deref().unwrap_or(""))
                .then_with(|| left.id.as_ref().cmp(right.id.as_ref()))
        });

        Ok(DependencyStageData {
            summary: build_dependency_summary(
                &incoming_edges,
                &outgoing_edges,
                self.artefacts.len(),
            ),
            schema: (!items.is_empty()).then(|| DEPENDENCY_STAGE_SCHEMA.to_string()),
            items,
        })
    }

    async fn resolve_tests_stage_data(
        &self,
        ctx: &Context<'_>,
        min_confidence: Option<f64>,
        linkage_source: Option<String>,
    ) -> Result<TestsStageData> {
        let rows: Vec<TestHarnessTestsResult> = decode_stage_rows(
            "tests",
            StageResolverAdapter::new(ctx.data_unchecked::<DevqlGraphqlContext>().clone(), "tests")
                .resolve(
                    &self.scope,
                    self.artefacts
                        .iter()
                        .map(selection_stage_row_from_artefact)
                        .collect(),
                    Some(build_tests_stage_args(min_confidence, linkage_source)?),
                    100,
                )
                .await
                .map_err(|err| {
                    backend_error(format!("failed to resolve selected tests: {err:#}"))
                })?,
        )?;
        let mut rows = rows;
        rows.sort_by(|left, right| {
            left.artefact
                .file_path
                .cmp(&right.artefact.file_path)
                .then_with(|| left.artefact.name.cmp(&right.artefact.name))
        });
        Ok(TestsStageData {
            summary: build_tests_summary(&rows, self.artefacts.len()),
            schema: (!rows.is_empty()).then(|| TESTS_STAGE_SCHEMA.to_string()),
            items: rows,
        })
    }
}

#[ComplexObject]
impl CheckpointStageResult {
    async fn items(&self, #[graphql(default = 20)] first: i32) -> Result<Vec<Checkpoint>> {
        take_stage_items(&self.items, first)
    }
}

#[ComplexObject]
impl CloneStageResult {
    async fn items(
        &self,
        #[graphql(default = 20)] first: i32,
    ) -> Result<Vec<super::SemanticClone>> {
        take_stage_items(&self.items, first)
    }
}

#[ComplexObject]
impl DependencyStageResult {
    async fn items(&self, #[graphql(default = 20)] first: i32) -> Result<Vec<DependencyEdge>> {
        take_stage_items(&self.items, first)
    }
}

#[ComplexObject]
impl TestsStageResult {
    async fn items(
        &self,
        #[graphql(default = 20)] first: i32,
    ) -> Result<Vec<TestHarnessTestsResult>> {
        take_stage_items(&self.items, first)
    }
}

fn decode_stage_rows<T: DeserializeOwned>(stage: &str, rows: Vec<Value>) -> Result<Vec<T>> {
    rows.into_iter()
        .map(|row| {
            serde_json::from_value(row).map_err(|err| {
                backend_error(format!(
                    "failed to decode `{stage}` stage payload into typed GraphQL result: {err}"
                ))
            })
        })
        .collect()
}

fn build_tests_stage_args(
    min_confidence: Option<f64>,
    linkage_source: Option<String>,
) -> Result<Value> {
    if let Some(min_confidence) = min_confidence
        && !(0.0..=1.0).contains(&min_confidence)
    {
        return Err(bad_user_input_error(
            "`minConfidence` must be between 0 and 1",
        ));
    }

    let mut args = serde_json::Map::new();
    if let Some(min_confidence) = min_confidence {
        args.insert("min_confidence".to_string(), json!(min_confidence));
    }
    if let Some(linkage_source) = linkage_source
        && !linkage_source.trim().is_empty()
    {
        args.insert(
            "linkage_source".to_string(),
            Value::String(linkage_source.trim().to_string()),
        );
    }
    Ok(Value::Object(args))
}

fn selection_stage_row_from_artefact(artefact: &Artefact) -> Value {
    json!({
        "artefact_id": artefact.id.as_ref(),
        "symbol_id": &artefact.symbol_id,
        "symbol_fqn": &artefact.symbol_fqn,
        "canonical_kind": artefact.canonical_kind.map(|kind| kind.as_devql_value()),
        "path": &artefact.path,
        "start_line": artefact.start_line,
        "end_line": artefact.end_line,
    })
}

fn build_checkpoint_summary(checkpoints: &[Checkpoint]) -> Value {
    let latest_at = checkpoints
        .first()
        .map(|checkpoint| checkpoint.event_time.as_str().to_string());
    let agents = checkpoints
        .iter()
        .filter_map(|checkpoint| checkpoint.agent.as_deref())
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    json!({
        "totalCount": checkpoints.len(),
        "latestAt": latest_at,
        "agents": agents,
    })
}

fn build_clone_summary(clones: &[super::SemanticClone]) -> Value {
    let summary = CloneSummary::from_clones(clones);
    let max_score = clones
        .iter()
        .map(|clone| clone.score)
        .max_by(|left, right| left.total_cmp(right));
    json!({
        "totalCount": summary.total_count,
        "groups": summary
            .groups
            .iter()
            .map(|group| {
                json!({
                    "relationKind": group.relation_kind,
                    "count": group.count,
                })
            })
            .collect::<Vec<_>>(),
        "maxScore": max_score,
    })
}

fn build_dependency_summary(
    incoming: &[DependencyEdge],
    outgoing: &[DependencyEdge],
    selected_artefact_count: usize,
) -> Value {
    let mut unique_by_id = BTreeMap::<String, EdgeKind>::new();
    for edge in incoming.iter().chain(outgoing.iter()) {
        unique_by_id
            .entry(edge.id.as_ref().to_string())
            .or_insert(edge.edge_kind);
    }

    let mut kind_counts = BTreeMap::<&'static str, usize>::new();
    for kind in unique_by_id.values().copied() {
        *kind_counts.entry(edge_kind_name(kind)).or_default() += 1;
    }

    json!({
        "selectedArtefactCount": selected_artefact_count,
        "totalCount": unique_by_id.len(),
        "incomingCount": incoming.len(),
        "outgoingCount": outgoing.len(),
        "kindCounts": {
            "imports": kind_counts.get("imports").copied().unwrap_or(0),
            "calls": kind_counts.get("calls").copied().unwrap_or(0),
            "references": kind_counts.get("references").copied().unwrap_or(0),
            "extends": kind_counts.get("extends").copied().unwrap_or(0),
            "implements": kind_counts.get("implements").copied().unwrap_or(0),
            "exports": kind_counts.get("exports").copied().unwrap_or(0),
        }
    })
}

fn build_tests_summary(rows: &[TestHarnessTestsResult], selected_artefact_count: usize) -> Value {
    let total_covering_tests = rows
        .iter()
        .map(|row| i64::from(row.summary.total_covering_tests))
        .sum::<i64>();
    let diagnostic_count = rows
        .iter()
        .map(|row| i64::from(row.summary.diagnostic_count))
        .sum::<i64>();
    let cross_cutting_artefacts = rows.iter().filter(|row| row.summary.cross_cutting).count();
    let data_sources = rows
        .iter()
        .flat_map(|row| row.summary.data_sources.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    json!({
        "selectedArtefactCount": selected_artefact_count,
        "matchedArtefactCount": rows.len(),
        "totalCoveringTests": total_covering_tests,
        "crossCuttingArtefactCount": cross_cutting_artefacts,
        "diagnosticCount": diagnostic_count,
        "dataSources": data_sources,
    })
}

fn build_selection_summary(
    selected_artefact_count: usize,
    checkpoints: &CheckpointStageData,
    clones: &CloneStageData,
    deps: &DependencyStageData,
    tests: &TestsStageData,
) -> Value {
    json!({
        "selectedArtefactCount": selected_artefact_count,
        "checkpoints": selection_stage_entry(&checkpoints.summary, checkpoints.schema.as_deref()),
        "clones": selection_stage_entry(&clones.summary, clones.schema.as_deref()),
        "deps": selection_stage_entry(&deps.summary, deps.schema.as_deref()),
        "tests": selection_stage_entry(&tests.summary, tests.schema.as_deref()),
    })
}

fn selection_stage_entry(summary: &Value, schema: Option<&str>) -> Value {
    json!({
        "summary": summary,
        "schema": schema,
    })
}

fn dedup_strings<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    values
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn dedup_dependency_edges(edges: Vec<DependencyEdge>) -> Vec<DependencyEdge> {
    let mut seen = HashSet::<String>::new();
    let mut deduped = Vec::new();
    for edge in edges {
        if seen.insert(edge.id.as_ref().to_string()) {
            deduped.push(edge);
        }
    }
    deduped
}

fn edge_kind_name(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Imports => "imports",
        EdgeKind::Calls => "calls",
        EdgeKind::References => "references",
        EdgeKind::Extends => "extends",
        EdgeKind::Implements => "implements",
        EdgeKind::Exports => "exports",
    }
}

fn saturating_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn take_stage_items<T: Clone>(items: &[T], first: i32) -> Result<Vec<T>> {
    if first <= 0 {
        return Err(bad_user_input_error("`first` must be greater than 0"));
    }
    Ok(items.iter().take(first as usize).cloned().collect())
}

const CHECKPOINT_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  checkpoints(agent: String, since: DateTime): CheckpointStageResult!
}

type CheckpointStageResult {
  summary: JSON!
  schema: String
  items(first: Int! = 20): [Checkpoint!]!
}

type Checkpoint {
  id: ID!
  sessionId: String!
  commitSha: String
  branch: String
  agent: String
  eventTime: DateTime!
  strategy: String
  filesTouched: [String!]!
  payload: JSON
  commit: Commit
  fileRelations: [CheckpointFileRelation!]!
}"#;

const CLONE_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  clones(relationKind: String, minScore: Float): CloneStageResult!
}

type CloneStageResult {
  summary: JSON!
  schema: String
  items(first: Int! = 20): [Clone!]!
}

type Clone {
  id: ID!
  sourceArtefactId: ID!
  targetArtefactId: ID!
  relationKind: String!
  score: Float!
  metadata: JSON
  sourceArtefact: Artefact!
  targetArtefact: Artefact!
}"#;

const DEPENDENCY_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  deps(kind: EdgeKind, direction: DepsDirection! = BOTH, includeUnresolved: Boolean! = true): DependencyStageResult!
}

type DependencyStageResult {
  summary: JSON!
  schema: String
  items(first: Int! = 20): [DependencyEdge!]!
}

type DependencyEdge {
  id: ID!
  edgeKind: EdgeKind!
  fromArtefactId: ID!
  toArtefactId: ID
  toSymbolRef: String
  startLine: Int
  endLine: Int
  metadata: JSON
  fromArtefact: Artefact!
  toArtefact: Artefact
}"#;

const TESTS_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  tests(minConfidence: Float, linkageSource: String): TestsStageResult!
}

type TestsStageResult {
  summary: JSON!
  schema: String
  items(first: Int! = 20): [TestHarnessTestsResult!]!
}

type TestHarnessTestsResult {
  artefact: TestHarnessArtefactRef!
  coveringTests: [TestHarnessCoveringTest!]!
  summary: TestHarnessTestsSummary!
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artefact_selector_accepts_symbol_fqn_or_path_modes() {
        let symbol = ArtefactSelectorInput {
            symbol_fqn: Some("src/main.rs::main".to_string()),
            fuzzy_name: None,
            path: None,
            lines: None,
        };
        assert_eq!(
            symbol.selection_mode().expect("symbol selector"),
            ArtefactSelectorMode::SymbolFqn("src/main.rs::main".to_string())
        );

        let path = ArtefactSelectorInput {
            symbol_fqn: None,
            fuzzy_name: None,
            path: Some("src/main.rs".to_string()),
            lines: Some(LineRangeInput { start: 20, end: 25 }),
        };
        assert_eq!(
            path.selection_mode().expect("path selector"),
            ArtefactSelectorMode::Path {
                path: "src/main.rs".to_string(),
                lines: Some(LineRangeInput { start: 20, end: 25 }),
            }
        );
    }

    #[test]
    fn artefact_selector_accepts_fuzzy_name_mode() {
        let fuzzy = ArtefactSelectorInput {
            symbol_fqn: None,
            fuzzy_name: Some("payLater()".to_string()),
            path: None,
            lines: None,
        };

        assert_eq!(
            fuzzy.selection_mode().expect("fuzzy selector"),
            ArtefactSelectorMode::FuzzyName("payLater()".to_string())
        );
    }

    #[test]
    fn artefact_selector_rejects_invalid_combinations() {
        let err = ArtefactSelectorInput {
            symbol_fqn: Some("src/main.rs::main".to_string()),
            fuzzy_name: None,
            path: Some("src/main.rs".to_string()),
            lines: None,
        }
        .selection_mode()
        .expect_err("mixed selector should fail");
        assert!(
            err.message
                .contains("allows exactly one of `symbolFqn`, `fuzzyName`, or `path`/`lines`")
        );

        let err = ArtefactSelectorInput {
            symbol_fqn: None,
            fuzzy_name: None,
            path: None,
            lines: Some(LineRangeInput { start: 20, end: 25 }),
        }
        .selection_mode()
        .expect_err("lines without path should fail");
        assert!(
            err.message
                .contains("requires `path` when `lines` is provided")
        );

        let err = ArtefactSelectorInput {
            symbol_fqn: None,
            fuzzy_name: Some("  ".to_string()),
            path: None,
            lines: None,
        }
        .selection_mode()
        .expect_err("blank fuzzy selector should fail");
        assert!(err.message.contains("non-empty `fuzzyName`"));

        let err = ArtefactSelectorInput {
            symbol_fqn: None,
            fuzzy_name: Some("payLater".to_string()),
            path: Some("src/main.rs".to_string()),
            lines: None,
        }
        .selection_mode()
        .expect_err("fuzzy selector mixed with path should fail");
        assert!(
            err.message
                .contains("allows exactly one of `symbolFqn`, `fuzzyName`, or `path`/`lines`")
        );

        let err = ArtefactSelectorInput {
            symbol_fqn: None,
            fuzzy_name: Some("payLater".to_string()),
            path: None,
            lines: Some(LineRangeInput { start: 20, end: 25 }),
        }
        .selection_mode()
        .expect_err("fuzzy selector mixed with lines should fail");
        assert!(
            err.message
                .contains("allows exactly one of `symbolFqn`, `fuzzyName`, or `path`/`lines`")
        );

        let err = ArtefactSelectorInput {
            symbol_fqn: Some("src/main.rs::main".to_string()),
            fuzzy_name: None,
            path: None,
            lines: Some(LineRangeInput { start: 20, end: 25 }),
        }
        .selection_mode()
        .expect_err("symbol selector mixed with lines should fail");
        assert!(
            err.message
                .contains("allows exactly one of `symbolFqn`, `fuzzyName`, or `path`/`lines`")
        );

        let err = ArtefactSelectorInput {
            symbol_fqn: None,
            fuzzy_name: None,
            path: None,
            lines: None,
        }
        .selection_mode()
        .expect_err("empty selector should fail");
        assert!(err.message.contains("requires exactly one selector mode"));
    }
}
