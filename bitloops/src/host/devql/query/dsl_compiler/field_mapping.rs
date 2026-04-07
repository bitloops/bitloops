use anyhow::{Result, bail};
use chrono::{DateTime, NaiveDate, SecondsFormat};

use super::document_builder::{GraphqlArgument, GraphqlField, GraphqlSelection};
use super::*;

pub(super) const KNOWLEDGE_STAGE_NAME: &str = "knowledge";
pub(super) const CLONE_SUMMARY_STAGE_NAME: &str =
    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_STAGE_ID;
pub(super) const TESTS_SUMMARY_STAGE_NAME: &str =
    crate::capability_packs::test_harness::types::TEST_HARNESS_TESTS_SUMMARY_STAGE_ID;

pub(super) fn is_tests_stage_name(stage_name: &str) -> bool {
    stage_name == crate::capability_packs::test_harness::types::TEST_HARNESS_TESTS_STAGE_ID
}

pub(super) fn is_coverage_stage_name(stage_name: &str) -> bool {
    stage_name == crate::capability_packs::test_harness::types::TEST_HARNESS_COVERAGE_STAGE_ID
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectableLeaf {
    Artefact,
    Checkpoint,
    Telemetry,
    DependencyEdge,
    Clone,
    ChatEntry,
    KnowledgeItem,
}

pub(super) fn quote_graphql_string(value: &str) -> String {
    serde_json::to_string(value).expect("string literal serialization must succeed")
}

pub(super) fn compile_as_of_input(selector: &AsOfSelector) -> GraphqlArgument {
    let value = match selector {
        AsOfSelector::Ref(reference) => format!("{{ ref: {} }}", quote_graphql_string(reference)),
        AsOfSelector::Commit(commit) => format!("{{ commit: {} }}", quote_graphql_string(commit)),
        AsOfSelector::SaveCurrent => "{ save: CURRENT }".to_string(),
        AsOfSelector::SaveRevision(revision) => {
            format!("{{ saveRevision: {} }}", quote_graphql_string(revision))
        }
    };
    GraphqlArgument::new("input", value)
}

pub(super) fn compile_datetime_literal(input: &str) -> Result<String> {
    if let Ok(date) = NaiveDate::parse_from_str(input.trim(), "%Y-%m-%d") {
        let midnight = date
            .and_hms_opt(0, 0, 0)
            .expect("midnight should always be valid");
        return Ok(quote_graphql_string(
            &midnight
                .and_utc()
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        ));
    }

    if let Ok(datetime) = DateTime::parse_from_rfc3339(input.trim()) {
        return Ok(quote_graphql_string(&datetime.to_rfc3339()));
    }

    bail!("invalid datetime value `{input}`")
}

pub(super) fn enum_literal(value: &str) -> String {
    value.trim().to_ascii_uppercase().replace('-', "_")
}

pub(super) fn scalar_selections_for_leaf(
    leaf: SelectableLeaf,
    select_fields: &[String],
) -> Result<Vec<GraphqlSelection>> {
    let graphql_fields = if select_fields.is_empty() {
        default_fields_for_leaf(leaf)
            .iter()
            .map(|field| (*field).to_string())
            .collect::<Vec<_>>()
    } else {
        select_fields
            .iter()
            .map(|field| map_selected_field(leaf, field))
            .collect::<Result<Vec<_>>>()?
    };

    Ok(graphql_fields
        .into_iter()
        .map(GraphqlSelection::scalar)
        .collect())
}

pub(super) fn tests_result_selections() -> Vec<GraphqlSelection> {
    vec![
        GraphqlField::new(
            "artefact",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("artefactId"),
                GraphqlSelection::scalar("name"),
                GraphqlSelection::scalar("kind"),
                GraphqlSelection::scalar("filePath"),
                GraphqlSelection::scalar("startLine"),
                GraphqlSelection::scalar("endLine"),
            ],
        )
        .into(),
        GraphqlField::new(
            "coveringTests",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("testId"),
                GraphqlSelection::scalar("testName"),
                GraphqlSelection::scalar("suiteName"),
                GraphqlSelection::scalar("filePath"),
                GraphqlSelection::scalar("startLine"),
                GraphqlSelection::scalar("endLine"),
            ],
        )
        .into(),
        GraphqlField::new(
            "summary",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("totalCoveringTests"),
                GraphqlSelection::scalar("crossCutting"),
                GraphqlSelection::scalar("dataSources"),
                GraphqlSelection::scalar("diagnosticCount"),
            ],
        )
        .into(),
    ]
}

pub(super) fn coverage_result_selections() -> Vec<GraphqlSelection> {
    vec![
        GraphqlField::new(
            "artefact",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("artefactId"),
                GraphqlSelection::scalar("name"),
                GraphqlSelection::scalar("kind"),
                GraphqlSelection::scalar("filePath"),
                GraphqlSelection::scalar("startLine"),
                GraphqlSelection::scalar("endLine"),
            ],
        )
        .into(),
        GraphqlField::new(
            "coverage",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("coverageSource"),
                GraphqlSelection::scalar("lineCoveragePct"),
                GraphqlSelection::scalar("branchCoveragePct"),
                GraphqlSelection::scalar("lineDataAvailable"),
                GraphqlSelection::scalar("branchDataAvailable"),
                GraphqlSelection::scalar("uncoveredLines"),
                GraphqlField::new(
                    "branches",
                    Vec::new(),
                    vec![
                        GraphqlSelection::scalar("line"),
                        GraphqlSelection::scalar("block"),
                        GraphqlSelection::scalar("branch"),
                        GraphqlSelection::scalar("covered"),
                        GraphqlSelection::scalar("hitCount"),
                    ],
                )
                .into(),
            ],
        )
        .into(),
        GraphqlField::new(
            "summary",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("uncoveredLineCount"),
                GraphqlSelection::scalar("uncoveredBranchCount"),
                GraphqlSelection::scalar("diagnosticCount"),
            ],
        )
        .into(),
    ]
}

pub(super) fn clone_result_selections(raw: bool) -> Result<Vec<GraphqlSelection>> {
    if raw {
        return scalar_selections_for_leaf(
            SelectableLeaf::Clone,
            &[
                "id".to_string(),
                "source_artefact_id".to_string(),
                "target_artefact_id".to_string(),
                "source_start_line".to_string(),
                "source_end_line".to_string(),
                "target_start_line".to_string(),
                "target_end_line".to_string(),
                "relation_kind".to_string(),
                "score".to_string(),
                "metadata".to_string(),
            ],
        );
    }

    Ok(vec![
        GraphqlSelection::scalar("relationKind"),
        GraphqlSelection::scalar("score"),
        GraphqlField::new(
            "sourceArtefact",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("path"),
                GraphqlSelection::scalar("symbolFqn"),
            ],
        )
        .into(),
        GraphqlField::new(
            "targetArtefact",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("path"),
                GraphqlSelection::scalar("symbolFqn"),
            ],
        )
        .into(),
    ])
}

pub(super) fn tests_summary_result_selections() -> Vec<GraphqlSelection> {
    vec![
        GraphqlSelection::scalar("capability"),
        GraphqlSelection::scalar("stage"),
        GraphqlSelection::scalar("status"),
        GraphqlSelection::scalar("commitSha"),
        GraphqlField::new(
            "counts",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("testArtefacts"),
                GraphqlSelection::scalar("testArtefactEdges"),
                GraphqlSelection::scalar("testClassifications"),
                GraphqlSelection::scalar("coverageCaptures"),
                GraphqlSelection::scalar("coverageHits"),
            ],
        )
        .into(),
        GraphqlSelection::scalar("coveragePresent"),
    ]
}

pub(super) fn clone_summary_selections() -> Vec<GraphqlSelection> {
    vec![
        GraphqlSelection::scalar("totalCount"),
        GraphqlField::new(
            "groups",
            Vec::new(),
            vec![
                GraphqlSelection::scalar("relationKind"),
                GraphqlSelection::scalar("count"),
            ],
        )
        .into(),
    ]
}

fn map_selected_field(leaf: SelectableLeaf, field: &str) -> Result<String> {
    let graphql = dsl_field_to_graphql(field);
    if allowed_fields_for_leaf(leaf).contains(&graphql.as_str()) {
        return Ok(graphql);
    }

    bail!(
        "unsupported select() field `{field}` for {} results",
        leaf.display_name()
    )
}

fn dsl_field_to_graphql(field: &str) -> String {
    match field.trim() {
        "artefact_id" | "checkpoint_id" | "edge_id" => "id".to_string(),
        "symbol_fqn" => "symbolFqn".to_string(),
        "canonical_kind" => "canonicalKind".to_string(),
        "language_kind" => "languageKind".to_string(),
        "parent_artefact_id" => "parentArtefactId".to_string(),
        "start_line" => "startLine".to_string(),
        "end_line" => "endLine".to_string(),
        "start_byte" => "startByte".to_string(),
        "end_byte" => "endByte".to_string(),
        "content_hash" => "contentHash".to_string(),
        "blob_sha" => "blobSha".to_string(),
        "created_at" => "createdAt".to_string(),
        "commit_sha" => "commitSha".to_string(),
        "session_id" => "sessionId".to_string(),
        "event_time" => "eventTime".to_string(),
        "event_type" => "eventType".to_string(),
        "files_touched" => "filesTouched".to_string(),
        "from_artefact_id" => "fromArtefactId".to_string(),
        "to_artefact_id" => "toArtefactId".to_string(),
        "to_symbol_ref" => "toSymbolRef".to_string(),
        "edge_kind" => "edgeKind".to_string(),
        "source_artefact_id" => "sourceArtefactId".to_string(),
        "target_artefact_id" => "targetArtefactId".to_string(),
        "relation_kind" => "relationKind".to_string(),
        "source_id" => "sourceId".to_string(),
        "source_kind" => "sourceKind".to_string(),
        "canonical_external_id" => "canonicalExternalId".to_string(),
        "external_url" => "externalUrl".to_string(),
        other => snake_to_camel(other),
    }
}

fn snake_to_camel(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut uppercase_next = false;
    for ch in input.chars() {
        if ch == '_' {
            uppercase_next = true;
            continue;
        }

        if uppercase_next {
            output.push(ch.to_ascii_uppercase());
            uppercase_next = false;
        } else {
            output.push(ch);
        }
    }
    output
}

fn default_fields_for_leaf(leaf: SelectableLeaf) -> &'static [&'static str] {
    match leaf {
        SelectableLeaf::Artefact => &[
            "id",
            "path",
            "symbolFqn",
            "canonicalKind",
            "languageKind",
            "startLine",
            "endLine",
            "language",
        ],
        SelectableLeaf::Checkpoint => &[
            "id",
            "sessionId",
            "commitSha",
            "branch",
            "agent",
            "eventTime",
            "strategy",
            "filesTouched",
        ],
        SelectableLeaf::Telemetry => &[
            "id",
            "sessionId",
            "eventType",
            "agent",
            "eventTime",
            "commitSha",
            "branch",
        ],
        SelectableLeaf::DependencyEdge => &[
            "id",
            "edgeKind",
            "fromArtefactId",
            "toArtefactId",
            "toSymbolRef",
            "startLine",
            "endLine",
        ],
        SelectableLeaf::Clone => &[
            "id",
            "sourceArtefactId",
            "targetArtefactId",
            "sourceStartLine",
            "sourceEndLine",
            "targetStartLine",
            "targetEndLine",
            "relationKind",
            "score",
        ],
        SelectableLeaf::ChatEntry => &["sessionId", "agent", "timestamp", "role", "content"],
        SelectableLeaf::KnowledgeItem => &[
            "id",
            "provider",
            "sourceKind",
            "canonicalExternalId",
            "externalUrl",
            "title",
        ],
    }
}

fn allowed_fields_for_leaf(leaf: SelectableLeaf) -> &'static [&'static str] {
    match leaf {
        SelectableLeaf::Artefact => &[
            "id",
            "symbolId",
            "path",
            "language",
            "canonicalKind",
            "languageKind",
            "symbolFqn",
            "parentArtefactId",
            "startLine",
            "endLine",
            "startByte",
            "endByte",
            "signature",
            "modifiers",
            "docstring",
            "contentHash",
            "blobSha",
            "createdAt",
        ],
        SelectableLeaf::Checkpoint => &[
            "id",
            "sessionId",
            "commitSha",
            "branch",
            "agent",
            "eventTime",
            "strategy",
            "filesTouched",
            "payload",
            "checkpointsCount",
            "sessionCount",
            "agents",
            "firstPromptPreview",
            "createdAt",
            "isTask",
            "toolUseId",
        ],
        SelectableLeaf::Telemetry => &[
            "id",
            "sessionId",
            "eventType",
            "agent",
            "eventTime",
            "commitSha",
            "branch",
            "payload",
        ],
        SelectableLeaf::DependencyEdge => &[
            "id",
            "edgeKind",
            "language",
            "fromArtefactId",
            "toArtefactId",
            "toSymbolRef",
            "startLine",
            "endLine",
            "metadata",
        ],
        SelectableLeaf::Clone => &[
            "id",
            "sourceArtefactId",
            "targetArtefactId",
            "sourceStartLine",
            "sourceEndLine",
            "targetStartLine",
            "targetEndLine",
            "relationKind",
            "score",
            "metadata",
        ],
        SelectableLeaf::ChatEntry => &[
            "sessionId",
            "agent",
            "timestamp",
            "role",
            "content",
            "metadata",
        ],
        SelectableLeaf::KnowledgeItem => &[
            "id",
            "sourceId",
            "provider",
            "sourceKind",
            "canonicalExternalId",
            "externalUrl",
            "title",
        ],
    }
}

impl SelectableLeaf {
    fn display_name(self) -> &'static str {
        match self {
            Self::Artefact => "artefact",
            Self::Checkpoint => "checkpoint",
            Self::Telemetry => "telemetry",
            Self::DependencyEdge => "dependency edge",
            Self::Clone => "clone",
            Self::ChatEntry => "chat history",
            Self::KnowledgeItem => "knowledge",
        }
    }
}
