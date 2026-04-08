use async_graphql::{ComplexObject, Context, InputObject, Result, SimpleObject, types::Json};
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
    Path {
        path: String,
        lines: Option<LineRangeInput>,
    },
}

#[derive(Debug, Clone, InputObject)]
pub struct ArtefactSelectorInput {
    pub symbol_fqn: Option<String>,
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
        let path = self
            .path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        match (symbol_fqn, path, self.lines.as_ref()) {
            (Some(symbol_fqn), None, None) => Ok(ArtefactSelectorMode::SymbolFqn(symbol_fqn)),
            (None, Some(path), lines) => {
                if let Some(lines) = lines {
                    lines.validate()?;
                }
                Ok(ArtefactSelectorMode::Path {
                    path,
                    lines: lines.cloned(),
                })
            }
            (None, None, Some(_)) => Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` requires `path` when `lines` is provided",
            )),
            (Some(_), Some(_), _) | (Some(_), None, Some(_)) => Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` allows either `symbolFqn` or `path`/`lines`, not both",
            )),
            (None, None, None) => Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` requires exactly one selector mode",
            )),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct ArtefactSelection {
    pub count: IntCount,
    #[graphql(skip)]
    pub(crate) artefacts: Vec<Artefact>,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

pub type IntCount = i32;

impl ArtefactSelection {
    pub(crate) fn new(artefacts: Vec<Artefact>, scope: ResolverScope) -> Self {
        Self {
            count: saturating_i32(artefacts.len()),
            artefacts,
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

#[derive(Debug, Clone, SimpleObject)]
pub struct CheckpointStageResult {
    pub summary: JsonScalar,
    pub schema: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct CloneStageResult {
    pub summary: JsonScalar,
    pub schema: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct DependencyStageResult {
    pub summary: JsonScalar,
    pub schema: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct TestsStageResult {
    pub summary: JsonScalar,
    pub schema: Option<String>,
}

#[ComplexObject]
impl ArtefactSelection {
    async fn artefacts(&self, #[graphql(default = 20)] first: i32) -> Result<Vec<Artefact>> {
        if first <= 0 {
            return Err(bad_user_input_error("`first` must be greater than 0"));
        }
        Ok(self
            .artefacts
            .iter()
            .take(first as usize)
            .cloned()
            .collect())
    }

    async fn checkpoints(
        &self,
        ctx: &Context<'_>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
    ) -> Result<CheckpointStageResult> {
        let checkpoints = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_selected_symbol_checkpoints(
                &self.scope,
                &self.symbol_ids(),
                agent.as_deref(),
                since.as_ref(),
            )
            .await
            .map_err(|err| {
                backend_error(format!("failed to resolve selected checkpoints: {err:#}"))
            })?;
        Ok(CheckpointStageResult {
            summary: Json(build_checkpoint_summary(&checkpoints)),
            schema: (!checkpoints.is_empty()).then(|| CHECKPOINT_STAGE_SCHEMA.to_string()),
        })
    }

    async fn clones(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "relationKind")] relation_kind: Option<String>,
        #[graphql(name = "minScore")] min_score: Option<f64>,
    ) -> Result<CloneStageResult> {
        let filter = ClonesFilterInput {
            relation_kind,
            min_score,
        };
        filter.validate()?;
        let clones = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_selected_clones(&self.artefact_ids(), Some(&filter), &self.scope)
            .await
            .map_err(|err| backend_error(format!("failed to resolve selected clones: {err:#}")))?;
        Ok(CloneStageResult {
            summary: Json(build_clone_summary(&clones)),
            schema: (!clones.is_empty()).then(|| CLONE_STAGE_SCHEMA.to_string()),
        })
    }

    async fn deps(
        &self,
        ctx: &Context<'_>,
        kind: Option<EdgeKind>,
        #[graphql(default_with = "DepsDirection::Both")] direction: DepsDirection,
        #[graphql(name = "includeUnresolved", default = false)] include_unresolved: bool,
    ) -> Result<DependencyStageResult> {
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
        let has_results = !outgoing_edges.is_empty() || !incoming_edges.is_empty();
        Ok(DependencyStageResult {
            summary: Json(build_dependency_summary(
                &incoming_edges,
                &outgoing_edges,
                self.artefacts.len(),
            )),
            schema: has_results.then(|| DEPENDENCY_STAGE_SCHEMA.to_string()),
        })
    }

    async fn tests(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "minConfidence")] min_confidence: Option<f64>,
        #[graphql(name = "linkageSource")] linkage_source: Option<String>,
    ) -> Result<TestsStageResult> {
        let rows = decode_stage_rows(
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
        Ok(TestsStageResult {
            summary: Json(build_tests_summary(&rows, self.artefacts.len())),
            schema: (!rows.is_empty()).then(|| TESTS_STAGE_SCHEMA.to_string()),
        })
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

const CHECKPOINT_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  checkpoints(agent: String, since: DateTime): CheckpointStageResult!
}

type CheckpointStageResult {
  summary: JSON!
  schema: String
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
  deps(kind: EdgeKind, direction: DepsDirection = BOTH, includeUnresolved: Boolean = false): DependencyStageResult!
}

type DependencyStageResult {
  summary: JSON!
  schema: String
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
            path: None,
            lines: None,
        };
        assert_eq!(
            symbol.selection_mode().expect("symbol selector"),
            ArtefactSelectorMode::SymbolFqn("src/main.rs::main".to_string())
        );

        let path = ArtefactSelectorInput {
            symbol_fqn: None,
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
    fn artefact_selector_rejects_invalid_combinations() {
        let err = ArtefactSelectorInput {
            symbol_fqn: Some("src/main.rs::main".to_string()),
            path: Some("src/main.rs".to_string()),
            lines: None,
        }
        .selection_mode()
        .expect_err("mixed selector should fail");
        assert!(
            err.message
                .contains("allows either `symbolFqn` or `path`/`lines`")
        );

        let err = ArtefactSelectorInput {
            symbol_fqn: None,
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
            path: None,
            lines: None,
        }
        .selection_mode()
        .expect_err("empty selector should fail");
        assert!(err.message.contains("requires exactly one selector mode"));
    }
}
