use crate::adapters::languages::rust::test_support::scenarios::collect_rust_suites;
use crate::adapters::model_providers::embeddings::{EmbeddingInputType, EmbeddingProvider};
use crate::capability_packs::semantic_clones::embeddings as semantic_embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::capability_packs::semantic_clones::{
    ensure_semantic_embeddings_schema, load_pre_stage_artefacts_for_blob,
    load_pre_stage_dependencies_for_blob, upsert_semantic_feature_rows,
    upsert_symbol_embedding_rows,
};
use crate::capability_packs::test_harness::mapping::linker::build_production_index;
use crate::capability_packs::test_harness::mapping::materialize::{
    MaterializationContext, materialize_source_discovery,
};
use crate::capability_packs::test_harness::mapping::model::{
    DiscoveredTestFile, ReferenceCandidate, StructuralMappingStats,
};
use crate::capability_packs::test_harness::storage::TestHarnessRepository;
use crate::host::devql::cucumber_world::{DevqlBddWorld, EdgeExpectation};
use crate::host::devql::*;
use crate::models::{
    CoverageCaptureRecord, CoverageFormat, CoverageHitRecord, ProductionArtefact, ScopeKind,
    TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord, TestDiscoveryRunRecord,
};
use crate::telemetry::logging;
use crate::test_support::git_fixtures::{git_ok, init_test_repo};
use crate::test_support::logger_lock::with_logger_test_lock;
use crate::test_support::process_state::enter_process_state;
use anyhow::{Context, Result, bail};
use cucumber::{codegen::LocalBoxFuture, step::Collection};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll, RawWaker, RawWakerVTable, Waker};
use tempfile::TempDir;
use tree_sitter::Parser;
use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;

fn doc_string(ctx: &cucumber::step::Context) -> String {
    ctx.step
        .docstring
        .as_ref()
        .map(ToString::to_string)
        .expect("step docstring should be present")
}

fn table_rows(ctx: &cucumber::step::Context) -> Vec<Vec<String>> {
    ctx.step
        .table
        .as_ref()
        .map(|table| table.rows.clone())
        .expect("step table should be present")
}

fn table_row_maps(ctx: &cucumber::step::Context) -> Vec<std::collections::HashMap<String, String>> {
    let rows = table_rows(ctx);
    let (header, values) = rows
        .split_first()
        .expect("table should include a header row");
    values
        .iter()
        .map(|row| {
            header
                .iter()
                .cloned()
                .zip(row.iter().cloned())
                .collect::<std::collections::HashMap<_, _>>()
        })
        .collect()
}

fn cell_to_opt(cell: &str) -> Option<&str> {
    match cell.trim() {
        "" | "-" => None,
        other => Some(other),
    }
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|err| panic!("invalid step regex `{pattern}`: {err}"))
}

fn link_metadata(link: &TestArtefactEdgeCurrentRecord) -> Value {
    serde_json::from_str(&link.metadata)
        .unwrap_or_else(|err| panic!("invalid link metadata `{}`: {err}", link.metadata))
}

fn link_confidence(link: &TestArtefactEdgeCurrentRecord) -> f64 {
    link_metadata(link)
        .get("confidence")
        .and_then(Value::as_f64)
        .expect("link metadata should include confidence")
}

fn link_status(link: &TestArtefactEdgeCurrentRecord) -> String {
    link_metadata(link)
        .get("linkage_status")
        .and_then(Value::as_str)
        .expect("link metadata should include linkage_status")
        .to_string()
}

#[derive(Debug, Clone)]
struct FixtureSummaryProvider {
    candidate: Option<semantic::SemanticSummaryCandidate>,
}

impl semantic::SemanticSummaryProvider for FixtureSummaryProvider {
    fn cache_key(&self) -> String {
        match self.candidate.as_ref() {
            Some(candidate) => format!(
                "provider=fixture:{}",
                candidate.source_model.as_deref().unwrap_or("synthetic")
            ),
            None => "provider=fixture:none".to_string(),
        }
    }

    fn generate(
        &self,
        _input: &semantic::SemanticFeatureInput,
    ) -> Option<semantic::SemanticSummaryCandidate> {
        self.candidate.clone()
    }
}

#[derive(Debug, Clone)]
struct FixtureSummaryMapProvider {
    candidates_by_symbol_fqn: HashMap<String, semantic::SemanticSummaryCandidate>,
}

impl semantic::SemanticSummaryProvider for FixtureSummaryMapProvider {
    fn cache_key(&self) -> String {
        let mut names = self
            .candidates_by_symbol_fqn
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        format!("provider=fixture-map:{}", names.join(","))
    }

    fn generate(
        &self,
        input: &semantic::SemanticFeatureInput,
    ) -> Option<semantic::SemanticSummaryCandidate> {
        self.candidates_by_symbol_fqn
            .get(&input.symbol_fqn)
            .cloned()
    }
}

#[derive(Debug, Clone)]
struct FixtureEmbeddingProvider {
    embeddings_by_document: HashMap<String, Vec<f32>>,
}

impl EmbeddingProvider for FixtureEmbeddingProvider {
    fn provider_name(&self) -> &str {
        "fixture"
    }

    fn model_name(&self) -> &str {
        "fixture-embedding-model"
    }

    fn output_dimension(&self) -> Option<usize> {
        self.embeddings_by_document
            .values()
            .next()
            .map(std::vec::Vec::len)
    }

    fn cache_key(&self) -> String {
        format!(
            "provider=fixture::model=fixture-embedding-model::documents={}",
            self.embeddings_by_document.len()
        )
    }

    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>> {
        if input_type != EmbeddingInputType::Document {
            bail!("fixture embedding provider only supports document inputs");
        }

        self.embeddings_by_document
            .get(input)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("fixture embedding not configured for `{input}`"))
    }
}

fn fixture_summary_candidate(summary: &str) -> semantic::SemanticSummaryCandidate {
    semantic::SemanticSummaryCandidate {
        summary: summary.to_string(),
        confidence: 0.88,
        source_model: Some("fixture-summary-model".to_string()),
    }
}

async fn load_persisted_summary_map(
    relational: &RelationalStorage,
    artefact_ids: &[String],
) -> Result<HashMap<String, String>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let ids_sql = artefact_ids
        .iter()
        .map(|artefact_id| format!("'{}'", esc_pg(artefact_id)))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = relational
        .query_rows(&format!(
            "SELECT artefact_id, summary FROM symbol_semantics WHERE artefact_id IN ({ids_sql})"
        ))
        .await?;
    let mut summaries = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(summary) = row.get("summary").and_then(Value::as_str) else {
            continue;
        };
        summaries.insert(artefact_id.to_string(), summary.to_string());
    }
    Ok(summaries)
}

fn build_fixture_embedding_provider(
    inputs: &[semantic::SemanticFeatureInput],
    summary_by_artefact_id: &HashMap<String, String>,
    embeddings_by_artefact_id: &HashMap<String, Vec<f32>>,
) -> Result<Arc<dyn EmbeddingProvider>> {
    let embedding_inputs =
        semantic_embeddings::build_symbol_embedding_inputs(inputs, summary_by_artefact_id);
    let mut embeddings_by_document = HashMap::with_capacity(embedding_inputs.len());
    for input in embedding_inputs {
        let embedding = embeddings_by_artefact_id
            .get(&input.artefact_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "fixture embedding not configured for artefact `{}`",
                    input.artefact_id
                )
            })?;
        embeddings_by_document.insert(
            semantic_embeddings::build_symbol_embedding_text(&input),
            embedding,
        );
    }
    Ok(Arc::new(FixtureEmbeddingProvider {
        embeddings_by_document,
    }))
}

fn stage1_sample_input(kind: &str, name: &str) -> semantic::SemanticFeatureInput {
    semantic::SemanticFeatureInput {
        artefact_id: "artefact-stage1".to_string(),
        symbol_id: Some("symbol-stage1".to_string()),
        repo_id: "repo-1".to_string(),
        blob_sha: "blob-stage1".to_string(),
        path: "src/services/user.ts".to_string(),
        language: "typescript".to_string(),
        canonical_kind: kind.to_string(),
        language_kind: kind.to_string(),
        symbol_fqn: format!("src/services/user.ts::{name}"),
        name: name.to_string(),
        signature: Some(format!("function {name}()")),
        modifiers: vec!["export".to_string()],
        body: "return value;".to_string(),
        docstring: None,
        parent_kind: Some("module".to_string()),
        dependency_signals: vec!["calls:user_repo::load_by_id".to_string()],
        content_hash: Some("hash-stage1".to_string()),
    }
}

fn clone_language_kind(kind: &str) -> &'static str {
    match kind {
        "method" => "method_definition",
        "function" => "function_declaration",
        "file" => "source_file",
        _ => "symbol",
    }
}

#[derive(Debug, Clone)]
struct RealCloneFixtureSymbol {
    symbol_id: String,
    artefact_id: String,
    path: String,
    canonical_kind: String,
    symbol_fqn: String,
    signature: Option<String>,
    modifiers: Vec<String>,
    body: String,
    docstring: Option<String>,
    summary: String,
    embedding: Vec<f32>,
    call_targets: Vec<String>,
    dependency_targets: Vec<String>,
    churn_count: usize,
}

#[derive(Debug, Clone)]
struct RealCloneFixture {
    symbols: Vec<RealCloneFixtureSymbol>,
}

#[derive(Debug, Clone)]
struct MaterializedCloneFile {
    path: String,
    blob_sha: String,
    content: String,
}

#[allow(clippy::too_many_arguments)]
fn real_clone_symbol(
    symbol_id: &str,
    artefact_id: &str,
    path: &str,
    canonical_kind: &str,
    symbol_fqn: &str,
    signature: Option<&str>,
    body: &str,
    summary: &str,
    embedding: Vec<f32>,
    call_targets: Vec<&str>,
    dependency_targets: Vec<&str>,
    churn_count: usize,
) -> RealCloneFixtureSymbol {
    RealCloneFixtureSymbol {
        symbol_id: symbol_id.to_string(),
        artefact_id: artefact_id.to_string(),
        path: path.to_string(),
        canonical_kind: canonical_kind.to_string(),
        symbol_fqn: symbol_fqn.to_string(),
        signature: signature.map(str::to_string),
        modifiers: vec!["export".to_string()],
        body: body.to_string(),
        docstring: None,
        summary: summary.to_string(),
        embedding,
        call_targets: call_targets.into_iter().map(str::to_string).collect(),
        dependency_targets: dependency_targets.into_iter().map(str::to_string).collect(),
        churn_count,
    }
}

fn clone_symbol_name(symbol_fqn: &str) -> String {
    symbol_fqn
        .rsplit("::")
        .next()
        .unwrap_or(symbol_fqn)
        .to_string()
}

fn clone_container_name(symbol_fqn: &str) -> Option<String> {
    let parts = symbol_fqn.split("::").collect::<Vec<_>>();
    if parts.len() >= 3 {
        return Some(parts[parts.len() - 2].to_string());
    }
    None
}

fn count_lines(content: &str) -> i32 {
    content.lines().count().max(1) as i32
}

fn materialize_function_file(
    repo_id: &str,
    blob_sha: &str,
    symbol: &RealCloneFixtureSymbol,
) -> (MaterializedCloneFile, Vec<semantic::PreStageArtefactRow>) {
    let name = clone_symbol_name(&symbol.symbol_fqn);
    let mut content = String::new();
    if let Some(docstring) = symbol.docstring.as_deref() {
        content.push_str("/** ");
        content.push_str(docstring);
        content.push_str(" */\n");
    }
    content.push_str(&format!(
        "export function {name}(orderId: string, locale: string, totalCents: number) {{\n"
    ));
    let start_line = count_lines(&content) + 1;
    let start_byte = content.len() as i32;
    for line in symbol.body.lines() {
        content.push_str("  ");
        content.push_str(line);
        content.push('\n');
    }
    let end_line = count_lines(&content);
    let end_byte = content.len() as i32;
    content.push_str("}\n");
    (
        MaterializedCloneFile {
            path: symbol.path.clone(),
            blob_sha: blob_sha.to_string(),
            content,
        },
        vec![semantic::PreStageArtefactRow {
            artefact_id: symbol.artefact_id.clone(),
            symbol_id: Some(symbol.symbol_id.clone()),
            repo_id: repo_id.to_string(),
            blob_sha: blob_sha.to_string(),
            path: symbol.path.clone(),
            language: "typescript".to_string(),
            canonical_kind: symbol.canonical_kind.clone(),
            language_kind: clone_language_kind(&symbol.canonical_kind).to_string(),
            symbol_fqn: symbol.symbol_fqn.clone(),
            parent_artefact_id: None,
            start_line: Some(start_line),
            end_line: Some(end_line),
            start_byte: Some(start_byte),
            end_byte: Some(end_byte),
            signature: symbol.signature.clone(),
            modifiers: symbol.modifiers.clone(),
            docstring: symbol.docstring.clone(),
            content_hash: Some(format!("content-hash-{}", symbol.symbol_id)),
        }],
    )
}

fn materialize_method_file(
    repo_id: &str,
    blob_sha: &str,
    path: &str,
    symbols: &[RealCloneFixtureSymbol],
) -> Result<(MaterializedCloneFile, Vec<semantic::PreStageArtefactRow>)> {
    let container_name = clone_container_name(&symbols[0].symbol_fqn)
        .ok_or_else(|| anyhow::anyhow!("method fixture is missing a container name"))?;
    let class_artefact_id = format!("artefact::{container_name}");
    let mut content = format!("export class {container_name} {{\n");
    let class_start_line = 1;
    let class_start_byte = 0;
    let mut rows = Vec::with_capacity(symbols.len() + 1);

    rows.push(semantic::PreStageArtefactRow {
        artefact_id: class_artefact_id.clone(),
        symbol_id: Some(format!("symbol::{container_name}")),
        repo_id: repo_id.to_string(),
        blob_sha: blob_sha.to_string(),
        path: path.to_string(),
        language: "typescript".to_string(),
        canonical_kind: "class_declaration".to_string(),
        language_kind: "class_declaration".to_string(),
        symbol_fqn: format!("{path}::{container_name}"),
        parent_artefact_id: None,
        start_line: Some(class_start_line),
        end_line: None,
        start_byte: Some(class_start_byte),
        end_byte: None,
        signature: Some(format!("class {container_name}")),
        modifiers: vec!["export".to_string()],
        docstring: None,
        content_hash: Some(format!("content-hash-class-{container_name}")),
    });

    for symbol in symbols {
        let method_name = clone_symbol_name(&symbol.symbol_fqn);
        content.push_str(&format!(
            "  {}(fileId: string, requestedPath: string) {{\n",
            method_name
        ));
        let start_line = count_lines(&content) + 1;
        let start_byte = content.len() as i32;
        for line in symbol.body.lines() {
            content.push_str("    ");
            content.push_str(line);
            content.push('\n');
        }
        let end_line = count_lines(&content);
        let end_byte = content.len() as i32;
        content.push_str("  }\n");

        rows.push(semantic::PreStageArtefactRow {
            artefact_id: symbol.artefact_id.clone(),
            symbol_id: Some(symbol.symbol_id.clone()),
            repo_id: repo_id.to_string(),
            blob_sha: blob_sha.to_string(),
            path: path.to_string(),
            language: "typescript".to_string(),
            canonical_kind: symbol.canonical_kind.clone(),
            language_kind: clone_language_kind(&symbol.canonical_kind).to_string(),
            symbol_fqn: symbol.symbol_fqn.clone(),
            parent_artefact_id: Some(class_artefact_id.clone()),
            start_line: Some(start_line),
            end_line: Some(end_line),
            start_byte: Some(start_byte),
            end_byte: Some(end_byte),
            signature: symbol.signature.clone(),
            modifiers: Vec::new(),
            docstring: symbol.docstring.clone(),
            content_hash: Some(format!("content-hash-{}", symbol.symbol_id)),
        });
    }

    content.push_str("}\n");
    let class_end_line = count_lines(&content);
    let class_end_byte = content.len() as i32;
    if let Some(class_row) = rows.first_mut() {
        class_row.end_line = Some(class_end_line);
        class_row.end_byte = Some(class_end_byte);
    }

    Ok((
        MaterializedCloneFile {
            path: path.to_string(),
            blob_sha: blob_sha.to_string(),
            content,
        },
        rows,
    ))
}

fn build_real_clone_fixture(name: &str) -> Result<RealCloneFixture> {
    let symbols = match name {
        "similar implementations" => vec![
            real_clone_symbol(
                "sym::invoice_pdf",
                "artefact::invoice_pdf",
                "src/pdf.ts",
                "function",
                "src/pdf.ts::createInvoicePdf",
                Some(
                    "function createInvoicePdf(orderId: string, locale: string, totalCents: number)",
                ),
                "const invoiceTemplate = billing.loadTemplate(orderId, locale);\nconst renderedTotal = currency.format(totalCents);\nreturn pdf.render(invoiceTemplate, renderedTotal);",
                "Function create invoice pdf. Generates invoice PDF content for an order.",
                vec![0.91, 0.09, 0.0],
                vec!["billing.loadTemplate", "currency.format", "pdf.render"],
                vec![
                    "references:invoice_template::default",
                    "references:billing_formatter::money",
                ],
                1,
            ),
            real_clone_symbol(
                "sym::invoice_doc",
                "artefact::invoice_doc",
                "src/render.ts",
                "function",
                "src/render.ts::renderInvoiceDocument",
                Some(
                    "function renderInvoiceDocument(orderId: string, locale: string, totalCents: number)",
                ),
                "const invoiceDocument = billing.loadTemplate(orderId, locale);\nconst formattedTotal = currency.format(totalCents);\nreturn pdf.render(invoiceDocument, formattedTotal);",
                "Function render invoice document. Renders invoice document content for an order.",
                vec![0.89, 0.11, 0.0],
                vec!["billing.loadTemplate", "currency.format", "pdf.render"],
                vec![
                    "references:invoice_template::default",
                    "references:billing_formatter::money",
                ],
                1,
            ),
        ],
        "exact duplicates" => vec![
            real_clone_symbol(
                "sym::fetch_order_src",
                "artefact::fetch_order_src",
                "src/services/fetch-order.ts",
                "function",
                "src/services/fetch-order.ts::fetch_order",
                Some("function fetch_order(orderId: string, locale: string, totalCents: number)"),
                "const fetchedOrder = orderRepository.fetch(orderId);\nreturn orderPresenter.present(fetchedOrder, locale);",
                "Function fetch order. Fetches an order and presents it for the caller.",
                vec![0.88, 0.12, 0.0],
                vec!["orderRepository.fetch", "orderPresenter.present"],
                vec![
                    "references:order_repository::default",
                    "references:order_presenter::default",
                ],
                1,
            ),
            real_clone_symbol(
                "sym::fetch_order_copy",
                "artefact::fetch_order_copy",
                "src/services/order_copies.ts",
                "function",
                "src/services/order_copies.ts::fetch_order",
                Some("function fetch_order(orderId: string, locale: string, totalCents: number)"),
                "const fetchedOrder = orderRepository.fetch(orderId);\nreturn orderPresenter.present(fetchedOrder, locale);",
                "Function fetch order. Fetches an order and presents it for the caller.",
                vec![0.88, 0.12, 0.0],
                vec!["orderRepository.fetch", "orderPresenter.present"],
                vec![
                    "references:order_repository::default",
                    "references:order_presenter::default",
                ],
                1,
            ),
        ],
        "shared logic candidates" => vec![
            real_clone_symbol(
                "sym::create_invoice_pdf",
                "artefact::create_invoice_pdf",
                "src/billing/invoice.ts",
                "function",
                "src/billing/invoice.ts::create_invoice_pdf",
                Some(
                    "function create_invoice_pdf(orderId: string, locale: string, totalCents: number)",
                ),
                "const invoiceTemplate = billing.loadTemplate(orderId, locale);\nconst renderedInvoice = pdf.render(invoiceTemplate, currency.format(totalCents));\nreturn renderedInvoice;",
                "Function create invoice pdf. Creates invoice PDF content for billing.",
                vec![0.84, 0.16, 0.0],
                vec!["billing.loadTemplate", "currency.format", "pdf.render"],
                vec![
                    "references:invoice_template::default",
                    "references:billing_formatter::money",
                ],
                1,
            ),
            real_clone_symbol(
                "sym::build_invoice_pdf_bundle",
                "artefact::build_invoice_pdf_bundle",
                "src/billing/invoice_helpers.ts",
                "function",
                "src/billing/invoice_helpers.ts::build_invoice_pdf_bundle",
                Some(
                    "function create_invoice_pdf(orderId: string, locale: string, totalCents: number)",
                ),
                "const invoiceTemplate = billing.loadTemplate(orderId, locale);\nconst renderedBundle = pdf.render(invoiceTemplate, currency.format(totalCents));\nreturn buildBundle(renderedBundle);",
                "Function build invoice pdf bundle. Builds invoice pdf content from shared billing steps.",
                vec![0.82, 0.18, 0.0],
                vec!["billing.loadTemplate", "currency.format", "pdf.render"],
                vec![
                    "references:invoice_template::default",
                    "references:billing_formatter::money",
                ],
                4,
            ),
        ],
        "diverged implementations" => vec![
            real_clone_symbol(
                "sym::validate_order_checkout",
                "artefact::validate_order_checkout",
                "src/validation/checkout.ts",
                "function",
                "src/validation/checkout.ts::validate_order_checkout",
                Some("function validateOrder(orderId: string, mode: string)"),
                "const checkoutRules = orderPolicy.load(orderId);\nreturn authorizeCheckout(checkoutRules, cartTotals, shippingAddress);",
                "Function validate order checkout. Validates order data for checkout.",
                vec![0.95, 0.05, 0.0],
                vec!["rules.checkout"],
                vec!["references:order_policy::default"],
                1,
            ),
            real_clone_symbol(
                "sym::validate_order_draft",
                "artefact::validate_order_draft",
                "src/validation/draft.ts",
                "function",
                "src/validation/draft.ts::validate_order_draft",
                Some("function validateOrder(orderId: string, mode: string)"),
                "const draftPolicy = orderPolicy.load(orderId);\nreturn saveDraftRevision(draftPolicy, pendingEdits, autosaveVersion);",
                "Function validate order draft. Validates order data for draft save.",
                vec![0.55, 0.75, 0.0],
                vec!["rules.draft"],
                vec!["references:order_policy::default"],
                1,
            ),
        ],
        "preferred local patterns" => vec![
            real_clone_symbol(
                "sym::render_invoice_document",
                "artefact::render_invoice_document",
                "src/rendering/invoices.ts",
                "function",
                "src/rendering/invoices.ts::render_invoice_document",
                Some("function render_invoice_document(orderId: string, locale: string)"),
                "const invoiceTemplate = billing.loadTemplate(orderId, locale);\nreturn pdf.render(invoiceTemplate, locale);",
                "Function render invoice document. Renders the invoice document for billing.",
                vec![0.80, 0.20, 0.10],
                vec!["billing.loadTemplate", "pdf.render"],
                vec!["references:invoice_template::default"],
                1,
            ),
            real_clone_symbol(
                "sym::create_invoice_pdf",
                "artefact::create_invoice_pdf",
                "src/billing/invoice.ts",
                "function",
                "src/billing/invoice.ts::create_invoice_pdf",
                Some("function create_invoice_pdf(orderId: string, locale: string)"),
                "const invoiceTemplate = billing.loadTemplate(orderId, locale);\nreturn pdf.render(invoiceTemplate, locale);",
                "Function create invoice pdf. Creates the invoice pdf from the billing template.",
                vec![0.82, 0.18, 0.10],
                vec!["billing.loadTemplate", "pdf.render"],
                vec!["references:invoice_template::default"],
                1,
            ),
            real_clone_symbol(
                "sym::render_invoice_preview",
                "artefact::render_invoice_preview",
                "src/rendering/preview.ts",
                "function",
                "src/rendering/preview.ts::render_invoice_preview",
                Some("function render_invoice_preview(orderId: string, locale: string)"),
                "const invoiceTemplate = billing.loadTemplate(orderId, locale);\nreturn preview.serialize(invoiceTemplate);",
                "Function render invoice preview. Returns a lightweight invoice preview.",
                vec![0.77, 0.23, 0.05],
                vec!["billing.loadTemplate", "preview.serialize"],
                vec![
                    "references:invoice_template::default",
                    "references:preview_presenter::default",
                ],
                5,
            ),
        ],
        "generic execute handlers" => vec![
            real_clone_symbol(
                "sym::create_component_execute",
                "artefact::create_component_execute",
                "src/handlers/create-component-snapshots.handler.ts",
                "method",
                "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::execute",
                Some("execute(snapshotId: string, workspaceId: string)"),
                "const componentSnapshot = snapshotRepo.load(snapshotId);\nconst relationshipChanges = snapshotPlanner.collect(componentSnapshot);\nreturn snapshotUpdater.apply(relationshipChanges, workspaceId);",
                "Synchronizes component snapshot relationships for the workspace.",
                vec![0.93, 0.07, 0.02],
                vec![
                    "snapshotRepo.load",
                    "snapshotPlanner.collect",
                    "snapshotUpdater.apply",
                ],
                vec![
                    "references:snapshot_repository::default",
                    "references:snapshot_updater::default",
                ],
                1,
            ),
            real_clone_symbol(
                "sym::create_component_helper",
                "artefact::create_component_helper",
                "src/handlers/create-component-snapshots.handler.ts",
                "method",
                "src/handlers/create-component-snapshots.handler.ts::CreateComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForBelongsToSnapshotRelationship",
                Some(
                    "updateSnapshotRelationshipsForBelongsToSnapshotRelationship(snapshotId: string, workspaceId: string)",
                ),
                "return snapshotFormatter.belongsTo(componentSnapshot);",
                "Updates snapshot relationships for belongs-to edges.",
                vec![0.60, 0.40, 0.10],
                vec!["snapshotFormatter.belongsTo"],
                vec!["references:snapshot_formatter::default"],
                1,
            ),
            real_clone_symbol(
                "sym::sync_component_execute",
                "artefact::sync_component_execute",
                "src/handlers/sync-component-snapshots.handler.ts",
                "method",
                "src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::execute",
                Some("execute(snapshotId: string, workspaceId: string)"),
                "const currentSnapshot = snapshotRepo.load(snapshotId);\nconst reconciliationPlan = snapshotPlanner.planChanges(currentSnapshot, workspaceId);\nreturn snapshotApplier.commit(reconciliationPlan);",
                "Reconciles component snapshot changes for the workspace.",
                vec![0.91, 0.09, 0.03],
                vec![
                    "snapshotRepo.load",
                    "snapshotPlanner.planChanges",
                    "snapshotApplier.commit",
                ],
                vec![
                    "references:snapshot_repository::default",
                    "references:snapshot_applier::default",
                ],
                1,
            ),
            real_clone_symbol(
                "sym::sync_component_helper",
                "artefact::sync_component_helper",
                "src/handlers/sync-component-snapshots.handler.ts",
                "method",
                "src/handlers/sync-component-snapshots.handler.ts::SyncComponentSnapshotsCommandHandler::updateSnapshotRelationshipsForInstanceInSnapshotRelationship",
                Some(
                    "updateSnapshotRelationshipsForInstanceInSnapshotRelationship(snapshotId: string, workspaceId: string)",
                ),
                "return snapshotFormatter.instanceIn(currentSnapshot);",
                "Updates snapshot relationships for instance-in edges.",
                vec![0.59, 0.41, 0.10],
                vec!["snapshotFormatter.instanceIn"],
                vec!["references:snapshot_formatter::default"],
                1,
            ),
        ],
        "weak clone candidates" => vec![
            real_clone_symbol(
                "sym::execute",
                "artefact::execute",
                "src/handlers/change-path.ts",
                "method",
                "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::execute",
                Some("execute(fileId: string, requestedPath: string)"),
                "const currentFile = repo.loadFile(fileId);\nconst nextPath = pathDomain.renamePath(currentFile, requestedPath);\nreturn nextPath;",
                "Method execute. Applies the path change workflow.",
                vec![0.78, 0.22, 0.0],
                vec!["repo.loadFile", "pathDomain.renamePath"],
                vec!["references:file_repository::default"],
                1,
            ),
            real_clone_symbol(
                "sym::command",
                "artefact::command",
                "src/handlers/change-path.ts",
                "method",
                "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::command",
                Some("command(fileId: string, requestedPath: string)"),
                "return commandFactory.buildCommand(requestedPath, fileId);",
                "Method command. Returns the path change command payload.",
                vec![0.76, 0.24, 0.0],
                vec!["commandFactory.buildCommand"],
                vec!["references:command_factory::default"],
                1,
            ),
        ],
        other => bail!("unknown real semantic clone fixture `{other}`"),
    };

    Ok(RealCloneFixture { symbols })
}

fn insert_pre_stage_artefact(
    conn: &rusqlite::Connection,
    row: &semantic::PreStageArtefactRow,
    current: bool,
) -> Result<()> {
    if current {
        ensure_test_repository_catalog_row(conn, row.repo_id.as_str())?;
        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language,
                canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, signature, modifiers, docstring,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            rusqlite::params![
                row.repo_id.as_str(),
                row.path.as_str(),
                row.blob_sha.as_str(),
                row.symbol_id.as_deref(),
                row.artefact_id.as_str(),
                row.language.as_str(),
                row.canonical_kind.as_str(),
                row.language_kind.as_str(),
                row.symbol_fqn.as_str(),
                row.parent_artefact_id.as_deref(),
                row.start_line.unwrap_or(1) as i64,
                row.end_line.unwrap_or(1) as i64,
                row.start_byte.unwrap_or(0) as i64,
                row.end_byte.unwrap_or(0) as i64,
                row.signature.as_deref(),
                serde_json::to_string(&row.modifiers).context("serialize modifiers")?,
                row.docstring.as_deref(),
                "2026-04-02T00:00:00Z",
            ],
        )
        .context("insert current pre-stage artefact")?;
    }

    conn.execute(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
            language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte,
            end_byte, signature, modifiers, docstring, content_hash
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        rusqlite::params![
            row.artefact_id.as_str(),
            row.symbol_id.as_deref(),
            row.repo_id.as_str(),
            row.blob_sha.as_str(),
            row.path.as_str(),
            row.language.as_str(),
            row.canonical_kind.as_str(),
            row.language_kind.as_str(),
            row.symbol_fqn.as_str(),
            row.parent_artefact_id.as_deref(),
            row.start_line.unwrap_or(1) as i64,
            row.end_line.unwrap_or(1) as i64,
            row.start_byte.unwrap_or(0) as i64,
            row.end_byte.unwrap_or(0) as i64,
            row.signature.as_deref(),
            serde_json::to_string(&row.modifiers).context("serialize modifiers")?,
            row.docstring.as_deref(),
            row.content_hash.as_deref(),
        ],
    )
    .context("insert historical pre-stage artefact")?;

    Ok(())
}

fn insert_real_clone_edges(
    conn: &rusqlite::Connection,
    repo_id: &str,
    blob_sha: &str,
    symbol: &RealCloneFixtureSymbol,
    path: &str,
) -> Result<()> {
    ensure_test_repository_catalog_row(conn, repo_id)?;
    for (edge_offset, target_ref) in symbol.call_targets.iter().enumerate() {
        let edge_id = format!("edge-call-{}-{edge_offset}", symbol.symbol_id);
        conn.execute(
            "INSERT INTO artefact_edges (
                edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind,
                language, start_line, end_line, metadata
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                edge_id.as_str(),
                repo_id,
                blob_sha,
                symbol.artefact_id.as_str(),
                target_ref.as_str(),
                EDGE_KIND_CALLS,
                "typescript",
                1i64,
                1i64,
                "{\"resolution\":\"real-pipeline-fixture\"}",
            ],
        )
        .context("insert historical call edge")?;
        conn.execute(
            "INSERT INTO artefact_edges_current (
                repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line,
                end_line, metadata, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                repo_id,
                edge_id.as_str(),
                path,
                blob_sha,
                symbol.symbol_id.as_str(),
                symbol.artefact_id.as_str(),
                target_ref.as_str(),
                EDGE_KIND_CALLS,
                "typescript",
                1i64,
                1i64,
                "{\"resolution\":\"real-pipeline-fixture\"}",
                "2026-04-02T00:00:00Z",
            ],
        )
        .context("insert current call edge")?;
    }

    for (edge_offset, target_ref) in symbol.dependency_targets.iter().enumerate() {
        let edge_id = format!("edge-dependency-{}-{edge_offset}", symbol.symbol_id);
        conn.execute(
            "INSERT INTO artefact_edges (
                edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind,
                language, start_line, end_line, metadata
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                edge_id.as_str(),
                repo_id,
                blob_sha,
                symbol.artefact_id.as_str(),
                target_ref.as_str(),
                "references",
                "typescript",
                1i64,
                1i64,
                "{\"resolution\":\"real-pipeline-fixture\"}",
            ],
        )
        .context("insert historical dependency edge")?;
        conn.execute(
            "INSERT INTO artefact_edges_current (
                repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line,
                end_line, metadata, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                repo_id,
                edge_id.as_str(),
                path,
                blob_sha,
                symbol.symbol_id.as_str(),
                symbol.artefact_id.as_str(),
                target_ref.as_str(),
                "references",
                "typescript",
                1i64,
                1i64,
                "{\"resolution\":\"real-pipeline-fixture\"}",
                "2026-04-02T00:00:00Z",
            ],
        )
        .context("insert current dependency edge")?;
    }

    Ok(())
}

fn seed_real_clone_fixture(
    conn: &rusqlite::Connection,
    repo_id: &str,
    fixture: &RealCloneFixture,
) -> Result<Vec<MaterializedCloneFile>> {
    let mut symbols_by_path = HashMap::<String, Vec<RealCloneFixtureSymbol>>::new();
    for symbol in &fixture.symbols {
        symbols_by_path
            .entry(symbol.path.clone())
            .or_default()
            .push(symbol.clone());
    }

    let mut files = Vec::with_capacity(symbols_by_path.len());
    for (ordinal, (path, symbols)) in symbols_by_path.into_iter().enumerate() {
        let blob_sha = format!("real-blob-{ordinal}");
        let (file, rows) = if symbols[0].canonical_kind == "method" {
            materialize_method_file(repo_id, &blob_sha, &path, &symbols)?
        } else {
            materialize_function_file(repo_id, &blob_sha, &symbols[0])
        };

        for row in &rows {
            let is_parent = row.canonical_kind == "class_declaration";
            insert_pre_stage_artefact(conn, row, !is_parent)?;
        }

        for symbol in &symbols {
            for churn_index in 0..symbol.churn_count.max(1) {
                let historical_artefact_id =
                    format!("{}::history::{churn_index}", symbol.artefact_id);
                conn.execute(
                    "INSERT INTO artefacts (
                        artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
                        language_kind, symbol_fqn, parent_artefact_id, start_line, end_line,
                        start_byte, end_byte, signature, modifiers, docstring, content_hash
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13, ?14, '[]', NULL, ?15)",
                    rusqlite::params![
                        historical_artefact_id.as_str(),
                        symbol.symbol_id.as_str(),
                        repo_id,
                        format!("history-blob-{}-{churn_index}", symbol.symbol_id),
                        symbol.path.as_str(),
                        "typescript",
                        symbol.canonical_kind.as_str(),
                        clone_language_kind(&symbol.canonical_kind),
                        symbol.symbol_fqn.as_str(),
                        1i64,
                        1i64,
                        0i64,
                        1i64,
                        symbol.signature.as_deref(),
                        format!("history-hash-{}-{churn_index}", symbol.symbol_id),
                    ],
                )
                .context("insert churn artefact row")?;
            }

            insert_real_clone_edges(conn, repo_id, &blob_sha, symbol, &path)?;
        }

        files.push(file);
    }

    Ok(files)
}

async fn execute_clone_query_for_real_fixture(
    fixture_name: &str,
    query: &str,
) -> Result<Vec<Value>> {
    let fixture = build_real_clone_fixture(fixture_name)?;
    let temp = TempDir::new().context("create semantic clone real-path temp dir")?;
    let sqlite_path = temp.path().join("semantic-clones-real.sqlite");
    init_sqlite_schema(&sqlite_path)
        .await
        .context("initialise sqlite schema for real semantic clone fixture")?;
    let relational = RelationalStorage::local_only(sqlite_path.clone());
    let cfg = DevqlBddWorld::test_cfg();
    let repo_id = cfg.repo.repo_id.clone();

    let summary_provider: Arc<dyn semantic::SemanticSummaryProvider> =
        Arc::new(FixtureSummaryMapProvider {
            candidates_by_symbol_fqn: fixture
                .symbols
                .iter()
                .map(|symbol| {
                    (
                        symbol.symbol_fqn.clone(),
                        fixture_summary_candidate(&symbol.summary),
                    )
                })
                .collect(),
        });

    let materialized_files = {
        let conn =
            rusqlite::Connection::open(&sqlite_path).context("open real semantic clone sqlite")?;
        seed_real_clone_fixture(&conn, &repo_id, &fixture)?
    };

    let mut all_semantic_inputs = Vec::new();
    for file in &materialized_files {
        let pre_stage_artefacts =
            load_pre_stage_artefacts_for_blob(&relational, &repo_id, &file.blob_sha, &file.path)
                .await
                .with_context(|| format!("load pre-stage artefacts for {}", file.path))?;
        let pre_stage_dependencies =
            load_pre_stage_dependencies_for_blob(&relational, &repo_id, &file.blob_sha, &file.path)
                .await
                .with_context(|| format!("load pre-stage dependencies for {}", file.path))?;
        let fixture_artefact_ids = fixture
            .symbols
            .iter()
            .map(|symbol| symbol.artefact_id.as_str())
            .collect::<HashSet<_>>();
        let semantic_inputs =
            semantic::build_semantic_feature_inputs_from_artefacts_with_dependencies(
                &pre_stage_artefacts,
                &pre_stage_dependencies,
                &file.content,
            )
            .into_iter()
            .filter(|input| fixture_artefact_ids.contains(input.artefact_id.as_str()))
            .collect::<Vec<_>>();
        all_semantic_inputs.extend(semantic_inputs);
    }

    upsert_semantic_feature_rows(
        &relational,
        &all_semantic_inputs,
        Arc::clone(&summary_provider),
    )
    .await
    .context("upsert semantic feature rows for real-path fixture")?;
    let summary_by_artefact_id = load_persisted_summary_map(
        &relational,
        &all_semantic_inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>(),
    )
    .await
    .context("load persisted summaries for real-path fixture")?;
    let embeddings_by_artefact_id = fixture
        .symbols
        .iter()
        .map(|symbol| (symbol.artefact_id.clone(), symbol.embedding.clone()))
        .collect::<HashMap<_, _>>();
    let embedding_provider = build_fixture_embedding_provider(
        &all_semantic_inputs,
        &summary_by_artefact_id,
        &embeddings_by_artefact_id,
    )
    .context("build fixture embedding provider for real-path fixture")?;
    upsert_symbol_embedding_rows(&relational, &all_semantic_inputs, embedding_provider)
        .await
        .context("upsert symbol embedding rows for real-path fixture")?;

    rebuild_symbol_clone_edges(&relational, &repo_id)
        .await
        .context("rebuild semantic clone edges for real-path fixture")?;

    let parsed = parse_devql_query(query).context("parse clone query")?;
    execute_devql_query(&cfg, &parsed, &empty_events_cfg(), Some(&relational))
        .await
        .context("execute real semantic clone query")
}

#[derive(Debug, Clone)]
struct IncrementalFixtureSymbol {
    symbol_id: String,
    artefact_id: String,
    path: String,
    canonical_kind: String,
    symbol_fqn: String,
    signature: Option<String>,
    body: String,
    summary: String,
    embedding: Vec<f32>,
    call_targets: Vec<String>,
    dependency_targets: Vec<String>,
    blob_sha: String,
    content_hash: String,
}

#[allow(clippy::too_many_arguments)]
fn incremental_fixture_symbol(
    symbol_id: &str,
    artefact_id: &str,
    path: &str,
    symbol_fqn: &str,
    signature: Option<&str>,
    body: &str,
    summary: &str,
    embedding: Vec<f32>,
    call_targets: Vec<&str>,
    dependency_targets: Vec<&str>,
    blob_sha: &str,
    content_hash: &str,
) -> IncrementalFixtureSymbol {
    IncrementalFixtureSymbol {
        symbol_id: symbol_id.to_string(),
        artefact_id: artefact_id.to_string(),
        path: path.to_string(),
        canonical_kind: "function".to_string(),
        symbol_fqn: symbol_fqn.to_string(),
        signature: signature.map(str::to_string),
        body: body.to_string(),
        summary: summary.to_string(),
        embedding,
        call_targets: call_targets.into_iter().map(str::to_string).collect(),
        dependency_targets: dependency_targets.into_iter().map(str::to_string).collect(),
        blob_sha: blob_sha.to_string(),
        content_hash: content_hash.to_string(),
    }
}

fn build_incremental_clone_fixture(
    name: &str,
) -> Result<(Vec<IncrementalFixtureSymbol>, Vec<IncrementalFixtureSymbol>)> {
    match name {
        "single changed artefact" => {
            let snapshot_one = vec![
                incremental_fixture_symbol(
                    "sym::create_invoice",
                    "artefact::create_invoice",
                    "src/billing/create.ts",
                    "src/billing/create.ts::createInvoice",
                    Some(
                        "function createInvoice(orderId: string, totalCents: number, locale: string)",
                    ),
                    "const formatted = currency.format(totalCents, locale);\nreturn invoice.renderLine(formatted);",
                    "Function create invoice. Creates invoice line output from billing totals.",
                    vec![0.86, 0.14, 0.0],
                    vec!["currency.format", "invoice.renderLine"],
                    vec!["references:invoice_template::default"],
                    "blob-create-v1",
                    "content-create-v1",
                ),
                incremental_fixture_symbol(
                    "sym::render_invoice_line",
                    "artefact::render_invoice_line",
                    "src/billing/render.ts",
                    "src/billing/render.ts::renderInvoiceLine",
                    Some("function renderInvoiceLine(totalCents: number, locale: string)"),
                    "const formatted = currency.format(totalCents, locale);\nreturn invoice.renderLine(formatted);",
                    "Function render invoice line. Renders invoice line output from billing totals.",
                    vec![0.85, 0.15, 0.0],
                    vec!["currency.format", "invoice.renderLine"],
                    vec!["references:invoice_template::default"],
                    "blob-render-v1",
                    "content-render-v1",
                ),
                incremental_fixture_symbol(
                    "sym::format_invoice_total",
                    "artefact::format_invoice_total",
                    "src/billing/common.ts",
                    "src/billing/common.ts::formatInvoiceTotal",
                    Some("function formatInvoiceTotal(totalCents: number, locale: string)"),
                    "const formatted = currency.format(totalCents, locale);\nreturn invoice.renderLine(formatted);",
                    "Function format invoice total. Formats invoice totals into billing line output.",
                    vec![0.84, 0.16, 0.0],
                    vec!["currency.format", "invoice.renderLine"],
                    vec!["references:invoice_template::default"],
                    "blob-common-v1",
                    "content-common-v1",
                ),
            ];

            let snapshot_two = vec![
                incremental_fixture_symbol(
                    "sym::create_invoice",
                    "artefact::create_invoice",
                    "src/billing/create.ts",
                    "src/billing/create.ts::createInvoice",
                    Some(
                        "function createInvoice(orderId: string, totalCents: number, locale: string)",
                    ),
                    "const formatted = currency.format(totalCents, locale);\naudit.recordInvoice(orderId);\nreturn invoice.renderLine(formatted);",
                    "Function create invoice. Creates invoice line output from billing totals.",
                    vec![0.86, 0.14, 0.0],
                    vec![
                        "currency.format",
                        "invoice.renderLine",
                        "audit.recordInvoice",
                    ],
                    vec![
                        "references:invoice_template::default",
                        "references:audit_log::default",
                    ],
                    "blob-create-v2",
                    "content-create-v2",
                ),
                snapshot_one[1].clone(),
                snapshot_one[2].clone(),
            ];

            Ok((snapshot_one, snapshot_two))
        }
        other => bail!("unknown incremental semantic clone fixture `{other}`"),
    }
}

fn build_incremental_feature_inputs(
    repo_id: &str,
    symbols: &[IncrementalFixtureSymbol],
) -> Vec<semantic::SemanticFeatureInput> {
    symbols
        .iter()
        .map(|symbol| semantic::SemanticFeatureInput {
            artefact_id: symbol.artefact_id.clone(),
            symbol_id: Some(symbol.symbol_id.clone()),
            repo_id: repo_id.to_string(),
            blob_sha: symbol.blob_sha.clone(),
            path: symbol.path.clone(),
            language: "typescript".to_string(),
            canonical_kind: symbol.canonical_kind.clone(),
            language_kind: clone_language_kind(&symbol.canonical_kind).to_string(),
            symbol_fqn: symbol.symbol_fqn.clone(),
            name: clone_symbol_name(&symbol.symbol_fqn),
            signature: symbol.signature.clone(),
            modifiers: vec!["export".to_string()],
            body: symbol.body.clone(),
            docstring: None,
            parent_kind: Some("module".to_string()),
            dependency_signals: symbol.dependency_targets.clone(),
            content_hash: Some(symbol.content_hash.clone()),
        })
        .collect()
}

fn replace_incremental_snapshot(
    conn: &rusqlite::Connection,
    repo_id: &str,
    symbols: &[IncrementalFixtureSymbol],
) -> Result<()> {
    ensure_test_repository_catalog_row(conn, repo_id)?;
    for symbol in symbols {
        conn.execute(
            "DELETE FROM artefact_edges_current WHERE repo_id = ?1 AND from_symbol_id = ?2",
            rusqlite::params![repo_id, symbol.symbol_id.as_str()],
        )
        .context("delete prior current incremental edges")?;
        conn.execute(
            "DELETE FROM artefact_edges WHERE repo_id = ?1 AND from_artefact_id = ?2",
            rusqlite::params![repo_id, symbol.artefact_id.as_str()],
        )
        .context("delete prior historical incremental edges")?;
        conn.execute(
            "DELETE FROM artefacts_current WHERE repo_id = ?1 AND symbol_id = ?2",
            rusqlite::params![repo_id, symbol.symbol_id.as_str()],
        )
        .context("delete prior current incremental artefact")?;
        conn.execute(
            "DELETE FROM artefacts WHERE artefact_id = ?1",
            rusqlite::params![symbol.artefact_id.as_str()],
        )
        .context("delete prior historical incremental artefact")?;

        let language_kind = clone_language_kind(&symbol.canonical_kind);
        let modifiers =
            serde_json::to_string(&vec!["export"]).context("serialize incremental modifiers")?;

        conn.execute(
            "INSERT INTO artefacts (
                artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
                language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte,
                end_byte, signature, modifiers, docstring, content_hash
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'typescript', ?6, ?7, ?8, NULL, 1, 3, 0, 1, ?9, ?10, NULL, ?11)",
            rusqlite::params![
                symbol.artefact_id.as_str(),
                symbol.symbol_id.as_str(),
                repo_id,
                symbol.blob_sha.as_str(),
                symbol.path.as_str(),
                symbol.canonical_kind.as_str(),
                language_kind,
                symbol.symbol_fqn.as_str(),
                symbol.signature.as_deref(),
                modifiers.as_str(),
                symbol.content_hash.as_str(),
            ],
        )
        .context("insert incremental historical artefact")?;

        conn.execute(
            "INSERT INTO artefacts_current (
                repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind,
                language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line,
                end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'typescript', ?6, ?7, ?8, NULL, NULL, 1, 3, 0, 1, ?9, ?10, NULL, ?11)",
            rusqlite::params![
                repo_id,
                symbol.path.as_str(),
                symbol.blob_sha.as_str(),
                symbol.symbol_id.as_str(),
                symbol.artefact_id.as_str(),
                symbol.canonical_kind.as_str(),
                language_kind,
                symbol.symbol_fqn.as_str(),
                symbol.signature.as_deref(),
                modifiers.as_str(),
                "2026-04-02T00:00:00Z",
            ],
        )
        .context("insert incremental current artefact")?;

        for (edge_offset, target_ref) in symbol.call_targets.iter().enumerate() {
            let edge_id = format!("incremental-call-{}-{edge_offset}", symbol.symbol_id);
            conn.execute(
                "INSERT INTO artefact_edges (
                    edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind,
                    language, start_line, end_line, metadata
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'typescript', 1, 1, ?7)",
                rusqlite::params![
                    edge_id.as_str(),
                    repo_id,
                    symbol.blob_sha.as_str(),
                    symbol.artefact_id.as_str(),
                    target_ref.as_str(),
                    EDGE_KIND_CALLS,
                    "{\"resolution\":\"incremental-fixture\"}",
                ],
            )
            .context("insert incremental historical call edge")?;
            conn.execute(
                "INSERT INTO artefact_edges_current (
                    repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                    to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line,
                    end_line, metadata, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, ?8, 'typescript', 1, 1, ?9, ?10)",
                rusqlite::params![
                    repo_id,
                    edge_id.as_str(),
                    symbol.path.as_str(),
                    symbol.blob_sha.as_str(),
                    symbol.symbol_id.as_str(),
                    symbol.artefact_id.as_str(),
                    target_ref.as_str(),
                    EDGE_KIND_CALLS,
                    "{\"resolution\":\"incremental-fixture\"}",
                    "2026-04-02T00:00:00Z",
                ],
            )
            .context("insert incremental current call edge")?;
        }

        for (edge_offset, target_ref) in symbol.dependency_targets.iter().enumerate() {
            let edge_id = format!("incremental-dependency-{}-{edge_offset}", symbol.symbol_id);
            conn.execute(
                "INSERT INTO artefact_edges (
                    edge_id, repo_id, blob_sha, from_artefact_id, to_symbol_ref, edge_kind,
                    language, start_line, end_line, metadata
                ) VALUES (?1, ?2, ?3, ?4, ?5, 'references', 'typescript', 1, 1, ?6)",
                rusqlite::params![
                    edge_id.as_str(),
                    repo_id,
                    symbol.blob_sha.as_str(),
                    symbol.artefact_id.as_str(),
                    target_ref.as_str(),
                    "{\"resolution\":\"incremental-fixture\"}",
                ],
            )
            .context("insert incremental historical dependency edge")?;
            conn.execute(
                "INSERT INTO artefact_edges_current (
                    repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id,
                    to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line,
                    end_line, metadata, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, 'references', 'typescript', 1, 1, ?8, ?9)",
                rusqlite::params![
                    repo_id,
                    edge_id.as_str(),
                    symbol.path.as_str(),
                    symbol.blob_sha.as_str(),
                    symbol.symbol_id.as_str(),
                    symbol.artefact_id.as_str(),
                    target_ref.as_str(),
                    "{\"resolution\":\"incremental-fixture\"}",
                    "2026-04-02T00:00:00Z",
                ],
            )
            .context("insert incremental current dependency edge")?;
        }
    }

    Ok(())
}

async fn load_semantic_feature_hashes(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<HashMap<String, String>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT a.symbol_fqn, ss.semantic_features_input_hash AS hash \
FROM artefacts_current a \
JOIN symbol_semantics ss ON ss.artefact_id = a.artefact_id \
WHERE a.repo_id = '{}'",
            esc_pg(repo_id),
        ))
        .await?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(symbol_fqn) = row.get("symbol_fqn").and_then(Value::as_str) else {
            continue;
        };
        let Some(hash) = row.get("hash").and_then(Value::as_str) else {
            continue;
        };
        out.insert(symbol_fqn.to_string(), hash.to_string());
    }
    Ok(out)
}

async fn load_embedding_hashes(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<HashMap<String, String>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT a.symbol_fqn, e.embedding_input_hash AS hash \
FROM artefacts_current a \
JOIN symbol_embeddings e ON e.artefact_id = a.artefact_id \
WHERE a.repo_id = '{}'",
            esc_pg(repo_id),
        ))
        .await?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(symbol_fqn) = row.get("symbol_fqn").and_then(Value::as_str) else {
            continue;
        };
        let Some(hash) = row.get("hash").and_then(Value::as_str) else {
            continue;
        };
        out.insert(symbol_fqn.to_string(), hash.to_string());
    }
    Ok(out)
}

fn clone_edge_hash_key(source_symbol_fqn: &str, target_symbol_fqn: &str) -> String {
    format!("{source_symbol_fqn} -> {target_symbol_fqn}")
}

async fn load_clone_edge_hashes(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<HashMap<String, String>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT source.symbol_fqn AS source_symbol_fqn, target.symbol_fqn AS target_symbol_fqn, ce.clone_input_hash \
FROM symbol_clone_edges ce \
JOIN artefacts_current source ON source.repo_id = ce.repo_id AND source.symbol_id = ce.source_symbol_id \
JOIN artefacts_current target ON target.repo_id = ce.repo_id AND target.symbol_id = ce.target_symbol_id \
WHERE ce.repo_id = '{}'",
            esc_pg(repo_id),
        ))
        .await?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(source_symbol_fqn) = row.get("source_symbol_fqn").and_then(Value::as_str) else {
            continue;
        };
        let Some(target_symbol_fqn) = row.get("target_symbol_fqn").and_then(Value::as_str) else {
            continue;
        };
        let Some(hash) = row.get("clone_input_hash").and_then(Value::as_str) else {
            continue;
        };
        out.insert(
            clone_edge_hash_key(source_symbol_fqn, target_symbol_fqn),
            hash.to_string(),
        );
    }
    Ok(out)
}

fn empty_events_cfg() -> crate::config::EventsBackendConfig {
    crate::config::EventsBackendConfig {
        duckdb_path: None,
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    }
}

fn noop_waker() -> Waker {
    fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    fn wake(_: *const ()) {}
    fn wake_by_ref(_: *const ()) {}
    fn drop(_: *const ()) {}

    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

fn run_ready_future<F: Future>(future: F) -> F::Output {
    let waker = noop_waker();
    let mut context = TaskContext::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(future.as_mut(), &mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("expected future to complete without awaiting external IO"),
    }
}

fn step_fn(
    f: for<'a> fn(&'a mut DevqlBddWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()>,
) -> for<'a> fn(&'a mut DevqlBddWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()> {
    f
}

fn extract_artefacts(world: &mut DevqlBddWorld) {
    world.artefacts.clear();

    if let Some(content) = &world.source_content {
        let path = world
            .source_path
            .as_deref()
            .expect("source path should be set");
        let artefacts = extract_js_ts_artefacts(content, path).expect("extract JS/TS artefacts");
        world.artefacts.extend(artefacts);
    }

    if let Some(content) = &world.rust_source_content {
        let path = world
            .rust_source_path
            .as_deref()
            .or(world.source_path.as_deref())
            .expect("rust source path should be set");
        let artefacts = extract_rust_artefacts(content, path).expect("extract Rust artefacts");
        world.artefacts.extend(artefacts);
    }
}

fn extract_edges(world: &mut DevqlBddWorld) {
    if world.artefacts.is_empty() {
        extract_artefacts(world);
    }
    world.edges.clear();

    if let Some(content) = &world.source_content {
        let path = world
            .source_path
            .as_deref()
            .expect("source path should be set");
        let ts_artefacts = world
            .artefacts
            .iter()
            .filter(|artefact| artefact.symbol_fqn.starts_with(path))
            .cloned()
            .collect::<Vec<_>>();
        let edges = extract_js_ts_dependency_edges(content, path, &ts_artefacts)
            .expect("extract JS/TS dependency edges");
        world.edges.extend(edges);
    }

    if let Some(content) = &world.rust_source_content {
        let path = world
            .rust_source_path
            .as_deref()
            .or(world.source_path.as_deref())
            .expect("rust source path should be set");
        let rust_artefacts = world
            .artefacts
            .iter()
            .filter(|artefact| artefact.symbol_fqn.starts_with(path))
            .cloned()
            .collect::<Vec<_>>();
        let edges = extract_rust_dependency_edges(content, path, &rust_artefacts)
            .expect("extract Rust dependency edges");
        world.edges.extend(edges);
    }
}

fn given_typescript_source(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        world.source_path = Some(path);
        world.source_language = Some("typescript".to_string());
        world.source_content = Some(doc_string(&ctx));
    })
}

fn given_rust_source(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        if world.source_path.is_none() {
            world.source_path = Some(path.clone());
            world.source_language = Some("rust".to_string());
        }
        world.rust_source_path = Some(path);
        world.rust_source_content = Some(doc_string(&ctx));
    })
}

fn when_extract_artefacts(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        extract_artefacts(world);
    })
}

fn when_extract_dependency_edges(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        extract_edges(world);
    })
}

fn when_parse_query(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.parsed_query = Some(
            parse_devql_query(&doc_string(&ctx)).expect("query should parse for this scenario"),
        );
    })
}

fn given_semantic_clone_fixture(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.semantic_clone_fixture = Some(ctx.matches[1].1.clone());
    })
}

fn when_clones_query_executes(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let fixture_name = world
            .semantic_clone_fixture
            .clone()
            .expect("semantic clone fixture should be configured before query execution");
        world.clone_query_rows =
            execute_clone_query_for_real_fixture(&fixture_name, &doc_string(&ctx))
                .await
                .expect("execute semantic clone query");
    })
}

fn then_clone_rows_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let target_symbol_fqn = row
                .get("target_symbol_fqn")
                .expect("target_symbol_fqn column should exist");
            let relation_kind = row
                .get("relation_kind")
                .expect("relation_kind column should exist");

            let found = world.clone_query_rows.iter().any(|result| {
                result.get("target_symbol_fqn").and_then(Value::as_str)
                    == Some(target_symbol_fqn.as_str())
                    && result.get("relation_kind").and_then(Value::as_str)
                        == Some(relation_kind.as_str())
            });

            assert!(
                found,
                "expected clone row target `{target_symbol_fqn}` with relation_kind `{relation_kind}`, found: {:#?}",
                world.clone_query_rows
            );
        }
    })
}

fn then_every_clone_row_includes_explainable_scores(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        assert!(
            !world.clone_query_rows.is_empty(),
            "expected clone query rows to be present"
        );
        for row in &world.clone_query_rows {
            assert!(
                row.get("relation_kind").and_then(Value::as_str).is_some(),
                "clone row missing relation_kind: {row:#?}"
            );
            assert!(
                row.get("score").and_then(Value::as_f64).is_some(),
                "clone row missing score: {row:#?}"
            );
            assert!(
                row.get("semantic_score").and_then(Value::as_f64).is_some(),
                "clone row missing semantic_score: {row:#?}"
            );
            assert!(
                row.get("lexical_score").and_then(Value::as_f64).is_some(),
                "clone row missing lexical_score: {row:#?}"
            );
            assert!(
                row.get("structural_score")
                    .and_then(Value::as_f64)
                    .is_some(),
                "clone row missing structural_score: {row:#?}"
            );
            assert!(
                row.get("explanation_json").is_some_and(Value::is_object),
                "clone row missing explanation_json object: {row:#?}"
            );
        }
    })
}

fn then_first_clone_row_targets(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let actual = world
            .clone_query_rows
            .first()
            .and_then(|row| row.get("target_symbol_fqn"))
            .and_then(Value::as_str)
            .expect("first clone row should include target_symbol_fqn");
        assert_eq!(actual, expected, "unexpected first clone target");
    })
}

fn then_first_clone_row_has_label(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let label = &ctx.matches[1].1;
        let labels = world
            .clone_query_rows
            .first()
            .and_then(|row| row.get("explanation_json"))
            .and_then(|value| value.get("labels"))
            .and_then(Value::as_array)
            .expect("first clone row should include labels");
        assert!(
            labels
                .iter()
                .filter_map(Value::as_str)
                .any(|candidate| candidate == label),
            "expected first clone row labels to include `{label}`, got {labels:?}"
        );
    })
}

fn then_clone_row_for_target_has_bias_warning(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let target_symbol_fqn = &ctx.matches[1].1;
        let expected_warning = &ctx.matches[2].1;
        let actual_warning = world
            .clone_query_rows
            .iter()
            .find(|row| {
                row.get("target_symbol_fqn").and_then(Value::as_str)
                    == Some(target_symbol_fqn.as_str())
            })
            .and_then(|row| row.get("explanation_json"))
            .and_then(|value| value.get("evidence"))
            .and_then(|value| value.get("bias_warning"))
            .and_then(Value::as_str)
            .expect("target clone row should include bias warning");
        assert_eq!(
            actual_warning, expected_warning,
            "unexpected bias warning for clone row `{target_symbol_fqn}`"
        );
    })
}

fn clone_row_for_target<'a>(world: &'a DevqlBddWorld, target_symbol_fqn: &str) -> &'a Value {
    world
        .clone_query_rows
        .iter()
        .find(|row| row.get("target_symbol_fqn").and_then(Value::as_str) == Some(target_symbol_fqn))
        .unwrap_or_else(|| {
            panic!(
                "missing clone row for `{target_symbol_fqn}`: {:#?}",
                world.clone_query_rows
            )
        })
}

fn clone_metric_value(row: &Value, metric: &str) -> Option<f64> {
    match metric {
        "score" | "semantic_score" | "lexical_score" | "structural_score" => {
            row.get(metric).and_then(Value::as_f64)
        }
        _ => row
            .get("explanation_json")
            .and_then(|value| value.get("scores"))
            .and_then(|value| value.get(metric))
            .and_then(Value::as_f64),
    }
}

fn then_clone_row_metric_at_least(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let target_symbol_fqn = &ctx.matches[1].1;
        let metric = &ctx.matches[2].1;
        let expected: f64 = ctx.matches[3]
            .1
            .parse()
            .expect("metric threshold should be numeric");
        let row = clone_row_for_target(world, target_symbol_fqn);
        let actual = clone_metric_value(row, metric)
            .unwrap_or_else(|| panic!("missing metric `{metric}` in clone row {row:#?}"));
        assert!(
            actual >= expected,
            "expected clone row `{target_symbol_fqn}` metric `{metric}` >= {expected}, got {actual}"
        );
    })
}

fn then_clone_row_metric_at_most(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let target_symbol_fqn = &ctx.matches[1].1;
        let metric = &ctx.matches[2].1;
        let expected: f64 = ctx.matches[3]
            .1
            .parse()
            .expect("metric threshold should be numeric");
        let row = clone_row_for_target(world, target_symbol_fqn);
        let actual = clone_metric_value(row, metric)
            .unwrap_or_else(|| panic!("missing metric `{metric}` in clone row {row:#?}"));
        assert!(
            actual <= expected,
            "expected clone row `{target_symbol_fqn}` metric `{metric}` <= {expected}, got {actual}"
        );
    })
}

fn then_clone_row_has_shared_signal(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let target_symbol_fqn = &ctx.matches[1].1;
        let signal = &ctx.matches[2].1;
        let row = clone_row_for_target(world, target_symbol_fqn);
        let shared = row
            .get("explanation_json")
            .and_then(|value| value.get("evidence"))
            .and_then(|value| value.get("shared_signals"))
            .and_then(|value| value.get(signal))
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("missing shared signal `{signal}` in clone row {row:#?}"));
        assert!(
            !shared.is_empty(),
            "expected clone row `{target_symbol_fqn}` shared signal `{signal}` to be non-empty"
        );
    })
}

fn then_clone_row_duplicate_signal_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let target_symbol_fqn = &ctx.matches[1].1;
        let signal = &ctx.matches[2].1;
        let expected = ctx.matches[3]
            .1
            .parse::<bool>()
            .expect("duplicate signal expectation should be boolean");
        let row = clone_row_for_target(world, target_symbol_fqn);
        let actual = row
            .get("explanation_json")
            .and_then(|value| value.get("duplicate_signals"))
            .and_then(|value| value.get(signal))
            .and_then(Value::as_bool)
            .unwrap_or_else(|| panic!("missing duplicate signal `{signal}` in clone row {row:#?}"));
        assert_eq!(
            actual, expected,
            "unexpected duplicate signal `{signal}` for clone row `{target_symbol_fqn}`"
        );
    })
}

fn then_clone_row_fact_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let target_symbol_fqn = &ctx.matches[1].1;
        let fact = &ctx.matches[2].1;
        let expected = ctx.matches[3]
            .1
            .parse::<bool>()
            .expect("fact expectation should be boolean");
        let row = clone_row_for_target(world, target_symbol_fqn);
        let actual = row
            .get("explanation_json")
            .and_then(|value| value.get("evidence"))
            .and_then(|value| value.get("facts"))
            .and_then(|value| value.get(fact))
            .and_then(Value::as_bool)
            .unwrap_or_else(|| panic!("missing fact `{fact}` in clone row {row:#?}"));
        assert_eq!(
            actual, expected,
            "unexpected explanation fact `{fact}` for clone row `{target_symbol_fqn}`"
        );
    })
}

fn then_clone_row_has_limiting_signal(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let target_symbol_fqn = &ctx.matches[1].1;
        let signal = &ctx.matches[2].1;
        let row = clone_row_for_target(world, target_symbol_fqn);
        let limiting_signals = row
            .get("explanation_json")
            .and_then(|value| value.get("limiting_signals"))
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("missing limiting_signals in clone row {row:#?}"));
        assert!(
            limiting_signals
                .iter()
                .filter_map(Value::as_str)
                .any(|candidate| candidate == signal),
            "expected clone row `{target_symbol_fqn}` limiting signals to include `{signal}`, got {limiting_signals:?}"
        );
    })
}

fn then_clone_row_ranks_below(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let lower_target = &ctx.matches[1].1;
        let higher_target = &ctx.matches[2].1;
        let lower_index = world
            .clone_query_rows
            .iter()
            .position(|row| {
                row.get("target_symbol_fqn").and_then(Value::as_str) == Some(lower_target.as_str())
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing clone row for `{lower_target}`: {:#?}",
                    world.clone_query_rows
                )
            });
        let higher_index = world
            .clone_query_rows
            .iter()
            .position(|row| {
                row.get("target_symbol_fqn").and_then(Value::as_str) == Some(higher_target.as_str())
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing clone row for `{higher_target}`: {:#?}",
                    world.clone_query_rows
                )
            });
        assert!(
            lower_index > higher_index,
            "expected `{lower_target}` to rank below `{higher_target}`, got indices {lower_index} and {higher_index}"
        );
    })
}

fn then_no_clone_row_targets(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let target_symbol_fqn = &ctx.matches[1].1;
        let found = world.clone_query_rows.iter().any(|row| {
            row.get("target_symbol_fqn").and_then(Value::as_str) == Some(target_symbol_fqn.as_str())
        });
        assert!(
            !found,
            "expected no clone row targeting `{target_symbol_fqn}`, found: {:#?}",
            world.clone_query_rows
        );
    })
}

fn then_clone_row_for_target_does_not_have_label(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let target_symbol_fqn = &ctx.matches[1].1;
        let label = &ctx.matches[2].1;
        let row = clone_row_for_target(world, target_symbol_fqn);
        let has_label = row
            .get("explanation_json")
            .and_then(|value| value.get("labels"))
            .and_then(Value::as_array)
            .is_some_and(|labels| {
                labels
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|candidate| candidate == label)
            });
        assert!(
            !has_label,
            "expected clone row `{target_symbol_fqn}` not to include label `{label}`, got {row:#?}"
        );
    })
}

fn given_stage1_input_for_kind_named(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.stage1_input = Some(stage1_sample_input(&ctx.matches[1].1, &ctx.matches[2].1));
    })
}

fn given_stage1_docstring_is_empty(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let input = world
            .stage1_input
            .as_mut()
            .expect("stage1 input should be configured before docstring");
        input.docstring = None;
    })
}

fn given_stage1_docstring(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let input = world
            .stage1_input
            .as_mut()
            .expect("stage1 input should be configured before docstring");
        input.docstring = Some(doc_string(&ctx));
    })
}

fn given_stage1_summary_provider_returns_no_candidate(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.stage1_provider_candidate = None;
    })
}

fn given_stage1_summary_provider_returns_candidate(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.stage1_provider_candidate = Some(semantic::SemanticSummaryCandidate {
            summary: doc_string(&ctx),
            confidence: 0.82,
            source_model: Some("fixture-summary-model".to_string()),
        });
    })
}

fn given_stage1_summary_provider_returns_invalid_candidate(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.stage1_provider_candidate = Some(semantic::SemanticSummaryCandidate {
            summary: doc_string(&ctx),
            confidence: 0.82,
            source_model: Some("fixture-invalid-summary-model".to_string()),
        });
    })
}

fn when_stage1_persists_semantic_feature_rows_through_original_pipeline(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let input = world
            .stage1_input
            .clone()
            .expect("stage1 input should be configured before persistence");
        let provider = Arc::new(FixtureSummaryProvider {
            candidate: world.stage1_provider_candidate.clone(),
        });
        let temp = TempDir::new().expect("create stage1 persistence temp dir");
        let sqlite_path = temp.path().join("stage1.sqlite");
        init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise sqlite schema for stage1 persistence");
        let relational = RelationalStorage::local_only(sqlite_path);
        upsert_semantic_feature_rows(&relational, std::slice::from_ref(&input), provider)
            .await
            .expect("persist stage1 semantic feature rows");
        let rows = relational
            .query_rows(&format!(
                "SELECT summary, template_summary FROM symbol_semantics WHERE artefact_id = '{}'",
                esc_pg(&input.artefact_id)
            ))
            .await
            .expect("load persisted stage1 semantics row");
        world.stage1_persisted_semantics_row = rows.into_iter().next();
    })
}

fn then_persisted_stage1_final_summary_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let actual = world
            .stage1_persisted_semantics_row
            .as_ref()
            .and_then(|row| row.get("summary"))
            .and_then(Value::as_str)
            .expect("persisted stage1 summary should be present");
        assert_eq!(
            actual, expected,
            "unexpected persisted Stage 1 final summary"
        );
    })
}

fn then_persisted_stage1_template_summary_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let actual = world
            .stage1_persisted_semantics_row
            .as_ref()
            .and_then(|row| row.get("template_summary"))
            .and_then(Value::as_str)
            .expect("persisted stage1 template summary should be present");
        assert_eq!(
            actual, expected,
            "unexpected persisted Stage 1 template summary"
        );
    })
}

fn then_persisted_stage1_final_summary_starts_with(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let prefix = &ctx.matches[1].1;
        let actual = world
            .stage1_persisted_semantics_row
            .as_ref()
            .and_then(|row| row.get("summary"))
            .and_then(Value::as_str)
            .expect("persisted stage1 summary should be present");
        assert!(
            actual.starts_with(prefix),
            "expected persisted Stage 1 final summary `{actual}` to start with `{prefix}`"
        );
    })
}

fn then_persisted_stage1_final_summary_is_not(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let unexpected = &ctx.matches[1].1;
        let actual = world
            .stage1_persisted_semantics_row
            .as_ref()
            .and_then(|row| row.get("summary"))
            .and_then(Value::as_str)
            .expect("persisted stage1 summary should be present");
        assert_ne!(
            actual, unexpected,
            "expected persisted Stage 1 final summary not to equal `{unexpected}`"
        );
    })
}

fn given_semantic_clone_incremental_fixture(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.semantic_clone_fixture = Some(ctx.matches[1].1.clone());
    })
}

fn when_semantic_clone_incremental_indexing_runs_across_two_snapshots(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let fixture_name = world
            .semantic_clone_fixture
            .clone()
            .expect("incremental semantic clone fixture should be configured");
        let (snapshot_one, snapshot_two) =
            build_incremental_clone_fixture(&fixture_name).expect("build incremental fixture");

        let temp = TempDir::new().expect("create incremental semantic clone temp dir");
        let sqlite_path = temp.path().join("semantic-clones-incremental.sqlite");
        init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise sqlite schema for incremental semantic clone fixture");
        let relational = RelationalStorage::local_only(sqlite_path.clone());
        let repo_id = world.cfg.repo.repo_id.clone();

        let summary_provider: Arc<dyn semantic::SemanticSummaryProvider> =
            Arc::new(FixtureSummaryMapProvider {
                candidates_by_symbol_fqn: snapshot_one
                    .iter()
                    .chain(snapshot_two.iter())
                    .map(|symbol| {
                        (
                            symbol.symbol_fqn.clone(),
                            fixture_summary_candidate(&symbol.summary),
                        )
                    })
                    .collect(),
            });

        {
            let conn = rusqlite::Connection::open(&sqlite_path)
                .expect("open sqlite for incremental snapshot one");
            replace_incremental_snapshot(&conn, &repo_id, &snapshot_one)
                .expect("seed incremental snapshot one");
        }

        let initial_inputs = build_incremental_feature_inputs(&repo_id, &snapshot_one);
        let initial_stage1_stats = upsert_semantic_feature_rows(
            &relational,
            &initial_inputs,
            Arc::clone(&summary_provider),
        )
        .await
        .expect("run stage 1 for incremental snapshot one");
        assert_eq!(
            initial_stage1_stats.upserted,
            snapshot_one.len(),
            "expected initial stage 1 run to upsert every snapshot one symbol"
        );
        let initial_summary_by_artefact_id = load_persisted_summary_map(
            &relational,
            &initial_inputs
                .iter()
                .map(|input| input.artefact_id.clone())
                .collect::<Vec<_>>(),
        )
        .await
        .expect("load snapshot one persisted summaries");
        let initial_embeddings_by_artefact_id = snapshot_one
            .iter()
            .map(|symbol| (symbol.artefact_id.clone(), symbol.embedding.clone()))
            .collect::<HashMap<_, _>>();
        let initial_embedding_provider = build_fixture_embedding_provider(
            &initial_inputs,
            &initial_summary_by_artefact_id,
            &initial_embeddings_by_artefact_id,
        )
        .expect("build snapshot one fixture embedding provider");
        let initial_stage2_stats =
            upsert_symbol_embedding_rows(&relational, &initial_inputs, initial_embedding_provider)
                .await
                .expect("run stage 2 for incremental snapshot one");
        assert_eq!(
            initial_stage2_stats.upserted,
            snapshot_one.len(),
            "expected initial stage 2 run to upsert every snapshot one symbol"
        );
        rebuild_symbol_clone_edges(&relational, &repo_id)
            .await
            .expect("rebuild clone edges for snapshot one");
        world.semantic_clone_semantic_hashes_before =
            load_semantic_feature_hashes(&relational, &repo_id)
                .await
                .expect("load snapshot one semantic hashes");
        world.semantic_clone_embedding_hashes_before = load_embedding_hashes(&relational, &repo_id)
            .await
            .expect("load snapshot one embedding hashes");
        world.semantic_clone_edge_hashes_before = load_clone_edge_hashes(&relational, &repo_id)
            .await
            .expect("load snapshot one clone edge hashes");

        {
            let conn = rusqlite::Connection::open(&sqlite_path)
                .expect("open sqlite for incremental snapshot two");
            replace_incremental_snapshot(&conn, &repo_id, &snapshot_two)
                .expect("seed incremental snapshot two");
        }

        let updated_inputs = build_incremental_feature_inputs(&repo_id, &snapshot_two);
        let stage1_stats = upsert_semantic_feature_rows(
            &relational,
            &updated_inputs,
            Arc::clone(&summary_provider),
        )
        .await
        .expect("run stage 1 for incremental snapshot two");
        let updated_summary_by_artefact_id = load_persisted_summary_map(
            &relational,
            &updated_inputs
                .iter()
                .map(|input| input.artefact_id.clone())
                .collect::<Vec<_>>(),
        )
        .await
        .expect("load snapshot two persisted summaries");
        let updated_embeddings_by_artefact_id = snapshot_two
            .iter()
            .map(|symbol| (symbol.artefact_id.clone(), symbol.embedding.clone()))
            .collect::<HashMap<_, _>>();
        let updated_embedding_provider = build_fixture_embedding_provider(
            &updated_inputs,
            &updated_summary_by_artefact_id,
            &updated_embeddings_by_artefact_id,
        )
        .expect("build snapshot two fixture embedding provider");
        let stage2_stats =
            upsert_symbol_embedding_rows(&relational, &updated_inputs, updated_embedding_provider)
                .await
                .expect("run stage 2 for incremental snapshot two");
        rebuild_symbol_clone_edges(&relational, &repo_id)
            .await
            .expect("rebuild clone edges for snapshot two");

        world.semantic_clone_stage1_upserted = stage1_stats.upserted;
        world.semantic_clone_stage1_skipped = stage1_stats.skipped;
        world.semantic_clone_stage2_upserted = stage2_stats.upserted;
        world.semantic_clone_stage2_skipped = stage2_stats.skipped;
        world.semantic_clone_semantic_hashes_after =
            load_semantic_feature_hashes(&relational, &repo_id)
                .await
                .expect("load snapshot two semantic hashes");
        world.semantic_clone_embedding_hashes_after = load_embedding_hashes(&relational, &repo_id)
            .await
            .expect("load snapshot two embedding hashes");
        world.semantic_clone_edge_hashes_after = load_clone_edge_hashes(&relational, &repo_id)
            .await
            .expect("load snapshot two clone edge hashes");
    })
}

fn then_stage1_incremental_stats_are(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected_upserted = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("stage 1 upserted count should be numeric");
        let expected_skipped = ctx.matches[2]
            .1
            .parse::<usize>()
            .expect("stage 1 skipped count should be numeric");
        assert_eq!(
            world.semantic_clone_stage1_upserted, expected_upserted,
            "unexpected Stage 1 incremental upserted count"
        );
        assert_eq!(
            world.semantic_clone_stage1_skipped, expected_skipped,
            "unexpected Stage 1 incremental skipped count"
        );
    })
}

fn then_stage2_incremental_stats_are(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected_upserted = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("stage 2 upserted count should be numeric");
        let expected_skipped = ctx.matches[2]
            .1
            .parse::<usize>()
            .expect("stage 2 skipped count should be numeric");
        assert_eq!(
            world.semantic_clone_stage2_upserted, expected_upserted,
            "unexpected Stage 2 incremental upserted count"
        );
        assert_eq!(
            world.semantic_clone_stage2_skipped, expected_skipped,
            "unexpected Stage 2 incremental skipped count"
        );
    })
}

fn then_semantic_feature_hash_is_unchanged_across_snapshots(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol_fqn = &ctx.matches[1].1;
        let before = world
            .semantic_clone_semantic_hashes_before
            .get(symbol_fqn)
            .unwrap_or_else(|| panic!("missing snapshot one semantic hash for `{symbol_fqn}`"));
        let after = world
            .semantic_clone_semantic_hashes_after
            .get(symbol_fqn)
            .unwrap_or_else(|| panic!("missing snapshot two semantic hash for `{symbol_fqn}`"));
        assert_eq!(
            before, after,
            "expected semantic feature hash for `{symbol_fqn}` to remain unchanged across snapshots"
        );
    })
}

fn then_embedding_hash_is_unchanged_across_snapshots(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol_fqn = &ctx.matches[1].1;
        let before = world
            .semantic_clone_embedding_hashes_before
            .get(symbol_fqn)
            .unwrap_or_else(|| panic!("missing snapshot one embedding hash for `{symbol_fqn}`"));
        let after = world
            .semantic_clone_embedding_hashes_after
            .get(symbol_fqn)
            .unwrap_or_else(|| panic!("missing snapshot two embedding hash for `{symbol_fqn}`"));
        assert_eq!(
            before, after,
            "expected embedding hash for `{symbol_fqn}` to remain unchanged across snapshots"
        );
    })
}

fn then_clone_edge_hash_is_unchanged_across_snapshots(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let source_symbol_fqn = &ctx.matches[1].1;
        let target_symbol_fqn = &ctx.matches[2].1;
        let key = clone_edge_hash_key(source_symbol_fqn, target_symbol_fqn);
        let before = world
            .semantic_clone_edge_hashes_before
            .get(&key)
            .unwrap_or_else(|| panic!("missing snapshot one clone edge hash for `{key}`"));
        let after = world
            .semantic_clone_edge_hashes_after
            .get(&key)
            .unwrap_or_else(|| panic!("missing snapshot two clone edge hash for `{key}`"));
        assert_eq!(
            before, after,
            "expected clone edge hash `{key}` to remain unchanged across snapshots"
        );
    })
}

fn then_clone_edge_hash_changes_across_snapshots(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let source_symbol_fqn = &ctx.matches[1].1;
        let target_symbol_fqn = &ctx.matches[2].1;
        let key = clone_edge_hash_key(source_symbol_fqn, target_symbol_fqn);
        let before = world
            .semantic_clone_edge_hashes_before
            .get(&key)
            .unwrap_or_else(|| panic!("missing snapshot one clone edge hash for `{key}`"));
        let after = world
            .semantic_clone_edge_hashes_after
            .get(&key)
            .unwrap_or_else(|| panic!("missing snapshot two clone edge hash for `{key}`"));
        assert_ne!(
            before, after,
            "expected clone edge hash `{key}` to change across snapshots"
        );
    })
}

fn invalid_embedding_provider_config(
    world: &mut DevqlBddWorld,
    case_name: &str,
) -> crate::capability_packs::semantic_clones::embeddings::EmbeddingProviderConfig {
    let daemon_config_path = world
        .scenario_config_override_root()
        .join("semantic-clones-embeddings.toml");
    fs::write(&daemon_config_path, "").expect("write fake daemon config");

    match case_name {
        "missing runtime command" => {
            crate::capability_packs::semantic_clones::embeddings::EmbeddingProviderConfig {
                daemon_config_path,
                embedding_profile: Some("local".to_string()),
                runtime_command: "definitely-missing-embeddings-runtime".to_string(),
                runtime_args: Vec::new(),
                startup_timeout_secs: 1,
                request_timeout_secs: 1,
                warnings: Vec::new(),
            }
        }
        "missing embedding profile" => {
            let (runtime_command, runtime_args) =
                failing_embeddings_runtime_command_and_args(world);
            crate::capability_packs::semantic_clones::embeddings::EmbeddingProviderConfig {
                daemon_config_path,
                embedding_profile: Some("missing-profile".to_string()),
                runtime_command,
                runtime_args,
                startup_timeout_secs: 1,
                request_timeout_secs: 1,
                warnings: Vec::new(),
            }
        }
        other => panic!("unknown invalid embedding provider configuration `{other}`"),
    }
}

#[cfg(unix)]
fn failing_embeddings_runtime_command_and_args(world: &mut DevqlBddWorld) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = world
        .scenario_bin_dir()
        .join("failing-embeddings-runtime.sh");
    let script = r#"#!/bin/sh
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"error","request_id":"%s","code":"runtime_error","message":"embedding profile `missing-profile` is not defined"}\n' "$req_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s","protocol_version":1,"accepted":true}\n' "$req_id"
      exit 0
      ;;
    *)
      printf '{"type":"error","request_id":"%s","code":"runtime_error","message":"unexpected request"}\n' "$req_id"
      ;;
  esac
done
"#;
    fs::write(&script_path, script).expect("write failing embeddings runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat failing embeddings runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)
        .expect("chmod failing embeddings runtime script");
    ("sh".to_string(), vec![script_path.display().to_string()])
}

#[cfg(windows)]
fn failing_embeddings_runtime_command_and_args(world: &mut DevqlBddWorld) -> (String, Vec<String>) {
    let script_path = world
        .scenario_bin_dir()
        .join("failing-embeddings-runtime.ps1");
    let script = r#"
$stdin = [Console]::In
while (($line = $stdin.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.type) {
    "describe" {
      $response = @{
        type = "error"
        request_id = $request.request_id
        code = "runtime_error"
        message = "embedding profile `missing-profile` is not defined"
      }
    }
    "shutdown" {
      $response = @{
        type = "shutdown"
        request_id = $request.request_id
        protocol_version = 1
        accepted = $true
      }
      $response | ConvertTo-Json -Compress
      exit 0
    }
    default {
      $response = @{
        type = "error"
        request_id = $request.request_id
        code = "runtime_error"
        message = "unexpected request"
      }
    }
  }
  $response | ConvertTo-Json -Compress
}
"#;
    fs::write(&script_path, script).expect("write failing embeddings runtime script");
    (
        "powershell".to_string(),
        vec![
            "-NoLogo".to_string(),
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.display().to_string(),
        ],
    )
}

async fn run_stage2_with_invalid_embedding_provider_configuration(
    world: &mut DevqlBddWorld,
    case_name: &str,
) {
    let input = world
        .stage1_input
        .clone()
        .expect("stage1 input should be configured before Stage 2 starts");
    let summary_provider = Arc::new(FixtureSummaryProvider {
        candidate: world.stage1_provider_candidate.clone(),
    });
    let temp = TempDir::new().expect("create invalid stage2 temp dir");
    let sqlite_path = temp.path().join("stage2-invalid.sqlite");
    init_sqlite_schema(&sqlite_path)
        .await
        .expect("initialise sqlite schema for invalid stage2 scenario");
    let relational = RelationalStorage::local_only(sqlite_path);
    upsert_semantic_feature_rows(&relational, &[input], summary_provider)
        .await
        .expect("persist stage1 rows before stage2 config failure");
    ensure_semantic_embeddings_schema(&relational)
        .await
        .expect("ensure symbol embeddings schema before invalid stage2 config");

    let config = invalid_embedding_provider_config(world, case_name);
    world.semantic_clone_stage2_error = crate::capability_packs::semantic_clones::extension_descriptor::build_symbol_embedding_provider(
        &config,
        None,
    )
    .err();

    let rows = relational
        .query_rows("SELECT COUNT(*) AS count FROM symbol_embeddings")
        .await
        .expect("count symbol embedding rows after invalid stage2 config");
    world.semantic_clone_stage2_embedding_rows_written = rows
        .first()
        .and_then(|row| row.get("count"))
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
}

fn when_stage2_starts_with_invalid_embedding_provider_configuration(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_stage2_with_invalid_embedding_provider_configuration(world, "missing runtime command")
            .await;
    })
}

fn when_stage2_starts_with_named_embedding_provider_configuration(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let case_name = ctx.matches[1].1.clone();
        run_stage2_with_invalid_embedding_provider_configuration(world, &case_name).await;
    })
}

fn then_stage2_fails_with_message_containing(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let error = world
            .semantic_clone_stage2_error
            .as_ref()
            .expect("expected Stage 2 error to be captured");
        assert!(
            error.to_string().contains(expected),
            "expected Stage 2 error to contain `{expected}`, got `{error}`"
        );
    })
}

fn then_stage2_writes_embedding_rows(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("expected embedding row count should be numeric");
        assert_eq!(
            world.semantic_clone_stage2_embedding_rows_written, expected,
            "unexpected number of Stage 2 embedding rows written"
        );
    })
}

fn when_build_deps_sql(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let parsed = world
            .parsed_query
            .as_ref()
            .expect("query should be parsed before building SQL");
        world.query_sql = Some(
            build_postgres_deps_query(&world.cfg, parsed, &world.cfg.repo.repo_id)
                .expect("deps SQL should build"),
        );
    })
}

fn when_execute_query_without_pg_client(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.init_test_logger();
        let workspace = world.logger_workspace_path().to_path_buf();
        let state_root_value = workspace.join("state-root").display().to_string();
        let parsed = world
            .parsed_query
            .clone()
            .expect("query should be parsed before execution");
        let cfg = world.cfg.clone();

        let error = {
            let _guard = enter_process_state(
                Some(&workspace),
                &[(
                    "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                    Some(state_root_value.as_str()),
                )],
            );
            with_logger_test_lock(|| {
                logging::reset_logger_for_tests();
                logging::init("bdd-devql-session").expect("initialize test logger");
                let events_cfg = EventsBackendConfig {
                    duckdb_path: None,
                    clickhouse_url: None,
                    clickhouse_user: None,
                    clickhouse_password: None,
                    clickhouse_database: None,
                };
                let result =
                    run_ready_future(execute_devql_query(&cfg, &parsed, &events_cfg, None));
                logging::close();
                result.expect_err("query should fail without a Postgres client")
            })
        };
        world.query_error = Some(error);
    })
}

fn when_extract_artefacts_and_edges_with_logger(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.init_test_logger();
        let workspace = world.logger_workspace_path().to_path_buf();
        let state_root_value = workspace.join("state-root").display().to_string();
        let source_content = world
            .source_content
            .clone()
            .expect("typescript source should be set");
        let source_path = world
            .source_path
            .clone()
            .expect("typescript source path should be set");

        let (artefacts, edges) = {
            let _guard = enter_process_state(
                Some(&workspace),
                &[(
                    "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                    Some(state_root_value.as_str()),
                )],
            );
            with_logger_test_lock(|| {
                logging::reset_logger_for_tests();
                logging::init("bdd-devql-session").expect("initialize test logger");
                let artefacts = extract_js_ts_artefacts(&source_content, &source_path)
                    .expect("extract artefacts");
                let edges =
                    extract_js_ts_dependency_edges(&source_content, &source_path, &artefacts)
                        .expect("extract edges");
                logging::close();
                (artefacts, edges)
            })
        };

        world.artefacts = artefacts;
        world.edges = edges;
    })
}

fn then_artefacts_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            world.assert_artefact(
                row.get("language_kind")
                    .map(String::as_str)
                    .expect("language_kind column should exist"),
                cell_to_opt(
                    row.get("canonical_kind")
                        .map(String::as_str)
                        .expect("canonical_kind column should exist"),
                ),
                row.get("name")
                    .map(String::as_str)
                    .expect("name column should exist"),
            );
        }
    })
}

fn then_edges_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            world.assert_edge(EdgeExpectation {
                edge_kind: row
                    .get("edge_kind")
                    .map(String::as_str)
                    .expect("edge_kind column should exist"),
                from_symbol_fqn: row
                    .get("from")
                    .map(String::as_str)
                    .expect("from column should exist"),
                to_target_symbol_fqn: cell_to_opt(
                    row.get("to_target")
                        .map(String::as_str)
                        .expect("to_target column should exist"),
                ),
                to_symbol_ref: cell_to_opt(
                    row.get("to_ref")
                        .map(String::as_str)
                        .expect("to_ref column should exist"),
                ),
                metadata_key: cell_to_opt(
                    row.get("metadata_key")
                        .map(String::as_str)
                        .expect("metadata_key column should exist"),
                ),
                metadata_value: cell_to_opt(
                    row.get("metadata_value")
                        .map(String::as_str)
                        .expect("metadata_value column should exist"),
                ),
            });
        }
    })
}

fn then_no_artefacts_are_emitted(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        assert!(
            world.artefacts.is_empty(),
            "expected no artefacts, got {:#?}",
            world.artefacts
        );
    })
}

fn then_no_edges_are_emitted(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        assert!(
            world.edges.is_empty(),
            "expected no edges, got {:#?}",
            world.edges
        );
    })
}

fn then_generated_sql_contains(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let fragment = row
                .get("fragment")
                .map(String::as_str)
                .expect("fragment column should exist");
            world.assert_sql_contains(fragment);
        }
    })
}

fn then_query_fails_with_message(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let fragment = &ctx.matches[1].1;
        let err = world
            .query_error
            .as_ref()
            .expect("query error should be set before assertion");
        assert!(
            err.to_string().contains(fragment),
            "expected error containing `{fragment}`, got `{err}`"
        );
    })
}

fn then_export_edge_named_appears_count(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let export_name = &ctx.matches[1].1;
        let expected_count: usize = ctx.matches[2]
            .1
            .parse()
            .expect("export count should be numeric");
        let actual_count = world
            .edges
            .iter()
            .filter(|edge| {
                edge.edge_kind == "exports"
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some(export_name.as_str())
            })
            .count();
        assert_eq!(
            actual_count, expected_count,
            "unexpected export edge count for `{export_name}`"
        );
    })
}

fn then_logs_parse_failure(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = &ctx.matches[1].1;
        let entries = world.read_log_entries();
        assert!(
            entries.iter().any(|entry| {
                entry.get("msg").and_then(Value::as_str) == Some("devql parse failure fallback")
                    && entry.get("path").and_then(Value::as_str) == Some(path.as_str())
                    && entry.get("component").and_then(Value::as_str) == Some("devql")
                    && entry.get("failure_kind").and_then(Value::as_str) == Some("parse_error")
            }),
            "expected parse-failure log entry for `{path}`, got {entries:#?}"
        );
    })
}

fn given_rust_production_file(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        let source = doc_string(&ctx);
        world.production_sources.push((path, source));
    })
}

fn given_rust_test_file(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        let source = doc_string(&ctx);
        world.test_sources.push((path, source));
    })
}

fn run_test_discovery(world: &mut DevqlBddWorld) {
    let lang: tree_sitter::Language = LANGUAGE_RUST.into();
    let mut parser = Parser::new();
    parser.set_language(&lang).expect("set rust language");

    world.discovered_suites.clear();
    world.discovered_scenarios.clear();

    for (path, source) in &world.test_sources {
        let tree = match parser.parse(source, None) {
            Some(tree) => tree,
            None => {
                world.discovery_issues.push(
                    crate::capability_packs::test_harness::mapping::model::DiscoveryIssue {
                        path: path.clone(),
                        message: "failed to parse source".to_string(),
                    },
                );
                continue;
            }
        };

        let suites = collect_rust_suites(tree.root_node(), source, path);

        let repo_id = "test-repo";
        let commit_sha = "test-commit";

        for suite in &suites {
            let suite_symbol_id = format!("test_suite:{commit_sha}:{path}:{}", suite.start_line);
            let suite_artefact_id = format!("test_artefact:{suite_symbol_id}");
            world.discovered_suites.push(TestArtefactCurrentRecord {
                artefact_id: suite_artefact_id.clone(),
                symbol_id: suite_symbol_id.clone(),
                repo_id: repo_id.to_string(),
                content_id: format!("blob:{commit_sha}:{path}"),
                path: path.clone(),
                language: "rust".to_string(),
                canonical_kind: "test_suite".to_string(),
                language_kind: None,
                symbol_fqn: Some(suite.name.clone()),
                name: suite.name.clone(),
                parent_artefact_id: None,
                parent_symbol_id: None,
                start_line: suite.start_line,
                end_line: suite.end_line,
                start_byte: None,
                end_byte: None,
                signature: None,
                modifiers: "[]".to_string(),
                docstring: None,
                discovery_source: "source".to_string(),
            });

            for scenario in &suite.scenarios {
                let scenario_symbol_id = format!(
                    "test_case:{commit_sha}:{path}:{}:{}",
                    scenario.start_line, scenario.name
                );
                world.discovered_scenarios.push(TestArtefactCurrentRecord {
                    artefact_id: format!("test_artefact:{scenario_symbol_id}"),
                    symbol_id: scenario_symbol_id,
                    repo_id: repo_id.to_string(),
                    content_id: format!("blob:{commit_sha}:{path}"),
                    path: path.clone(),
                    language: "rust".to_string(),
                    canonical_kind: "test_scenario".to_string(),
                    language_kind: None,
                    symbol_fqn: Some(format!("{}.{}", suite.name, scenario.name)),
                    name: scenario.name.clone(),
                    parent_artefact_id: Some(suite_artefact_id.clone()),
                    parent_symbol_id: Some(suite_symbol_id.clone()),
                    start_line: scenario.start_line,
                    end_line: scenario.end_line,
                    start_byte: None,
                    end_byte: None,
                    signature: None,
                    modifiers: "[]".to_string(),
                    docstring: None,
                    discovery_source: scenario.discovery_source.as_str().to_string(),
                });
            }
        }
    }
}

fn run_linkage_resolution(world: &mut DevqlBddWorld) {
    let lang: tree_sitter::Language = LANGUAGE_RUST.into();
    let mut parser = Parser::new();
    parser.set_language(&lang).expect("set rust language");

    // Build production artefacts from production sources
    let mut production_artefacts: Vec<ProductionArtefact> = Vec::new();
    for (path, source) in &world.production_sources {
        let _tree = match parser.parse(source, None) {
            Some(tree) => tree,
            None => continue,
        };
        let artefacts = extract_rust_artefacts(source, path).unwrap_or_default();
        for artefact in artefacts {
            production_artefacts.push(ProductionArtefact {
                artefact_id: format!("artefact:{}:{}", path, artefact.name),
                symbol_id: format!("sym:{}:{}", path, artefact.name),
                symbol_fqn: artefact.symbol_fqn.clone(),
                path: path.clone(),
                start_line: artefact.start_line as i64,
            });
        }
    }

    let production_index = build_production_index(&production_artefacts);

    // Discover test files and materialize links
    let mut test_artefacts = Vec::new();
    let mut test_edges = Vec::new();
    let mut link_keys = HashSet::new();
    let mut stats = StructuralMappingStats::default();

    let mut discovered_files: Vec<DiscoveredTestFile> = Vec::new();
    for (path, source) in &world.test_sources {
        let tree = match parser.parse(source, None) {
            Some(tree) => tree,
            None => continue,
        };
        let test_suites = collect_rust_suites(tree.root_node(), source, path);

        // Collect reference candidates from the test file's import paths
        let reference_candidates = vec![ReferenceCandidate::SourcePath(path.clone())];
        // Also add source paths from production sources
        let mut file_references = reference_candidates;
        for (prod_path, _) in &world.production_sources {
            file_references.push(ReferenceCandidate::SourcePath(prod_path.clone()));
        }

        discovered_files.push(DiscoveredTestFile {
            relative_path: path.clone(),
            language: "rust".to_string(),
            reference_candidates: file_references,
            suites: test_suites,
        });
    }

    let content_ids = HashMap::new();
    let mut materialization = MaterializationContext {
        repo_id: "test-repo",
        content_ids: &content_ids,
        production: &production_artefacts,
        production_index: &production_index,
        test_artefacts: &mut test_artefacts,
        test_edges: &mut test_edges,
        link_keys: &mut link_keys,
        stats: &mut stats,
    };

    materialize_source_discovery(&mut materialization, &discovered_files);

    world.discovered_suites = test_artefacts
        .iter()
        .filter(|artefact| artefact.canonical_kind == "test_suite")
        .cloned()
        .collect();
    world.discovered_scenarios = test_artefacts
        .iter()
        .filter(|artefact| artefact.canonical_kind == "test_scenario")
        .cloned()
        .collect();
    world.materialized_links = test_edges;
}

#[derive(Copy, Clone)]
enum RegisteredStageQueryMode {
    Current,
    AsOfCommit,
    AsOfRef,
}

struct SeededArtefact {
    path: String,
    symbol_id: String,
    current_artefact_id: String,
    historical_artefact_id: String,
}

fn write_repo_sources(repo_root: &Path, world: &DevqlBddWorld) {
    for (path, source) in world
        .production_sources
        .iter()
        .chain(world.test_sources.iter())
    {
        let full_path = repo_root.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("create source parent directory");
        }
        std::fs::write(&full_path, source).expect("write fixture source file");
    }
}

fn write_repo_config(repo_root: &Path, sqlite_path: &Path) {
    std::fs::write(
        repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        format!(
            "[stores.relational]\nsqlite_path = {path:?}\n",
            path = sqlite_path.to_string_lossy()
        ),
    )
    .expect("write config");
}

fn rewrite_test_artefact(
    artefact: &TestArtefactCurrentRecord,
    repo_id: &str,
    commit_sha: &str,
) -> TestArtefactCurrentRecord {
    let mut rewritten = artefact.clone();
    rewritten.repo_id = repo_id.to_string();
    rewritten.content_id = format!("blob:{commit_sha}:{}", rewritten.path);
    rewritten
}

fn rewrite_test_edge(
    edge: &TestArtefactEdgeCurrentRecord,
    repo_id: &str,
    commit_sha: &str,
    artefact_name: &str,
    symbol_id: &str,
    current_artefact_id: &str,
) -> TestArtefactEdgeCurrentRecord {
    let mut rewritten = edge.clone();
    rewritten.repo_id = repo_id.to_string();
    rewritten.content_id = format!("blob:{commit_sha}:{}", rewritten.path);
    if edge_targets_artefact(edge, artefact_name) {
        rewritten.to_symbol_id = Some(symbol_id.to_string());
        rewritten.to_artefact_id = Some(current_artefact_id.to_string());
    }
    rewritten
}

fn edge_targets_artefact(edge: &TestArtefactEdgeCurrentRecord, artefact_name: &str) -> bool {
    edge.to_artefact_id
        .as_deref()
        .is_some_and(|artefact_id| artefact_id.contains(artefact_name))
        || edge
            .to_symbol_id
            .as_deref()
            .is_some_and(|symbol_id| symbol_id.contains(artefact_name))
}

fn discovery_run_record(repo_id: &str, commit_sha: &str) -> TestDiscoveryRunRecord {
    TestDiscoveryRunRecord {
        discovery_run_id: format!("discovery:{commit_sha}:bdd"),
        repo_id: repo_id.to_string(),
        sync_mode: "full".to_string(),
        language: Some("rust".to_string()),
        started_at: "2026-03-24T00:00:00Z".to_string(),
        finished_at: Some("2026-03-24T00:00:01Z".to_string()),
        status: "complete".to_string(),
        enumeration_status: Some("hybrid_full".to_string()),
        notes_json: None,
        stats_json: None,
    }
}

fn coverage_capture_record(
    repo_id: &str,
    commit_sha: &str,
    scenario_id: &str,
) -> CoverageCaptureRecord {
    CoverageCaptureRecord {
        capture_id: format!("capture:{commit_sha}:bdd"),
        repo_id: repo_id.to_string(),
        commit_sha: commit_sha.to_string(),
        tool: "llvm-cov".to_string(),
        format: CoverageFormat::Lcov,
        scope_kind: ScopeKind::TestScenario,
        subject_test_symbol_id: Some(scenario_id.to_string()),
        line_truth: true,
        branch_truth: true,
        captured_at: "2026-03-24T00:00:02Z".to_string(),
        status: "complete".to_string(),
        metadata_json: Some("{\"runner\":\"cargo test\"}".to_string()),
    }
}

fn coverage_hits(symbol_id: &str, path: &str, capture_id: &str) -> Vec<CoverageHitRecord> {
    vec![
        CoverageHitRecord {
            capture_id: capture_id.to_string(),
            production_symbol_id: symbol_id.to_string(),
            file_path: path.to_string(),
            line: 1,
            branch_id: -1,
            covered: true,
            hit_count: 3,
        },
        CoverageHitRecord {
            capture_id: capture_id.to_string(),
            production_symbol_id: symbol_id.to_string(),
            file_path: path.to_string(),
            line: 2,
            branch_id: -1,
            covered: false,
            hit_count: 0,
        },
        CoverageHitRecord {
            capture_id: capture_id.to_string(),
            production_symbol_id: symbol_id.to_string(),
            file_path: path.to_string(),
            line: 3,
            branch_id: 0,
            covered: true,
            hit_count: 1,
        },
        CoverageHitRecord {
            capture_id: capture_id.to_string(),
            production_symbol_id: symbol_id.to_string(),
            file_path: path.to_string(),
            line: 3,
            branch_id: 1,
            covered: false,
            hit_count: 0,
        },
    ]
}

fn seed_target_production_artefact(
    conn: &rusqlite::Connection,
    repo_root: &Path,
    repo_id: &str,
    commit_sha: &str,
    world: &DevqlBddWorld,
    artefact_name: &str,
) -> anyhow::Result<SeededArtefact> {
    ensure_test_repository_catalog_row(conn, repo_id)?;
    for (path, source) in &world.production_sources {
        let artefacts = extract_rust_artefacts(source, path).context("extract rust artefacts")?;
        if artefacts
            .iter()
            .any(|artefact| artefact.name == artefact_name)
        {
            let blob_sha = git_ok(repo_root, &["rev-parse", &format!("{commit_sha}:{path}")]);
            let symbol_id = format!("sym:{path}:{artefact_name}");
            let current_artefact_id = format!("current:{path}:{artefact_name}");
            let historical_artefact_id = format!("historical:{path}:{artefact_name}");
            let symbol_fqn = format!("{path}::{artefact_name}");

            conn.execute(
                "INSERT INTO artefacts_current (
                    repo_id, path, content_id, symbol_id, artefact_id, language,
                    canonical_kind, language_kind, symbol_fqn, parent_symbol_id,
                    parent_artefact_id, start_line, end_line, start_byte, end_byte,
                    signature, modifiers, docstring, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                rusqlite::params![
                    repo_id,
                    path,
                    blob_sha.as_str(),
                    symbol_id.as_str(),
                    current_artefact_id.as_str(),
                    "rust",
                    "function",
                    "function_item",
                    symbol_fqn.as_str(),
                    Option::<String>::None,
                    Option::<String>::None,
                    1i64,
                    3i64,
                    0i64,
                    64i64,
                    "fn create_user()",
                    "[]",
                    Some("seeded current artefact".to_string()),
                    "2026-03-31T00:00:00Z",
                ],
            )
            .context("insert current artefact row")?;

            conn.execute(
                "INSERT INTO artefacts (
                    artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
                    language_kind, symbol_fqn, start_line, end_line, start_byte, end_byte,
                    modifiers, content_hash
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    historical_artefact_id.as_str(),
                    symbol_id.as_str(),
                    repo_id,
                    blob_sha.as_str(),
                    path,
                    "rust",
                    "function",
                    "function_item",
                    symbol_fqn.as_str(),
                    1i64,
                    3i64,
                    0i64,
                    64i64,
                    "[]",
                    "hash-historical",
                ],
            )
            .context("insert historical artefact row")?;

            return Ok(SeededArtefact {
                path: path.clone(),
                symbol_id,
                current_artefact_id,
                historical_artefact_id,
            });
        }
    }

    anyhow::bail!("target production artefact `{artefact_name}` not found in fixture sources")
}

fn ensure_test_repository_catalog_row(conn: &rusqlite::Connection, repo_id: &str) -> Result<()> {
    let repo_name = repo_id
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("temp2");
    conn.execute(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
         VALUES (?1, 'github', 'bitloops', ?2, 'main') \
         ON CONFLICT(repo_id) DO UPDATE SET \
           provider = excluded.provider, \
           organization = excluded.organization, \
           name = excluded.name, \
           default_branch = excluded.default_branch",
        rusqlite::params![repo_id, repo_name],
    )
    .context("ensure test repository catalog row")?;
    Ok(())
}

async fn execute_registered_stage_query(
    world: &mut DevqlBddWorld,
    stage_name: &str,
    artefact_name: &str,
    mode: RegisteredStageQueryMode,
) -> anyhow::Result<Value> {
    run_linkage_resolution(world);

    let temp = TempDir::new().context("create temp dir")?;
    let home = TempDir::new().context("create temp home")?;
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
            ("BITLOOPS_DEVQL_PG_DSN", None),
            ("BITLOOPS_DEVQL_CH_URL", None),
            ("BITLOOPS_DEVQL_CH_USER", None),
            ("BITLOOPS_DEVQL_CH_PASSWORD", None),
            ("BITLOOPS_DEVQL_CH_DATABASE", None),
        ],
    );

    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).context("create repo root")?;
    init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    write_repo_sources(&repo_root, world);
    git_ok(&repo_root, &["add", "."]);
    git_ok(&repo_root, &["commit", "-m", "seed devql bdd fixture"]);
    let commit_sha = git_ok(&repo_root, &["rev-parse", "HEAD"]);

    let mut cfg = DevqlBddWorld::test_cfg();
    cfg.config_root = repo_root.clone();
    cfg.repo_root = repo_root.clone();
    let sqlite_path = temp.path().join("relational.sqlite");
    write_repo_config(&repo_root, &sqlite_path);
    init_sqlite_schema(&sqlite_path)
        .await
        .context("initialise sqlite relational schema")?;
    crate::capability_packs::test_harness::storage::init_schema_for_repo(&repo_root)
        .context("initialise test harness schema")?;

    let conn = rusqlite::Connection::open(&sqlite_path).context("open sqlite db")?;
    let seeded = seed_target_production_artefact(
        &conn,
        &repo_root,
        &cfg.repo.repo_id,
        commit_sha.trim(),
        world,
        artefact_name,
    )?;

    let mut repository =
        crate::capability_packs::test_harness::storage::open_repository_for_repo(&repo_root)
            .context("open test harness repository")?;
    let rewritten_suites = world
        .discovered_suites
        .iter()
        .map(|artefact| rewrite_test_artefact(artefact, &cfg.repo.repo_id, commit_sha.trim()))
        .collect::<Vec<_>>();
    let rewritten_scenarios = world
        .discovered_scenarios
        .iter()
        .map(|artefact| rewrite_test_artefact(artefact, &cfg.repo.repo_id, commit_sha.trim()))
        .collect::<Vec<_>>();
    let rewritten_edges = world
        .materialized_links
        .iter()
        .map(|edge| {
            rewrite_test_edge(
                edge,
                &cfg.repo.repo_id,
                commit_sha.trim(),
                artefact_name,
                &seeded.symbol_id,
                &seeded.current_artefact_id,
            )
        })
        .collect::<Vec<_>>();
    let mut test_artefacts = rewritten_suites;
    test_artefacts.extend(rewritten_scenarios.clone());
    repository
        .replace_test_discovery(
            commit_sha.trim(),
            &test_artefacts,
            &rewritten_edges,
            &discovery_run_record(&cfg.repo.repo_id, commit_sha.trim()),
            &[],
        )
        .context("seed test discovery rows")?;

    if stage_name == "coverage"
        && world
            .materialized_links
            .iter()
            .any(|edge| edge_targets_artefact(edge, artefact_name))
    {
        let scenario_id = rewritten_scenarios
            .first()
            .map(|scenario| scenario.symbol_id.as_str())
            .expect("expected discovered scenario");
        let capture = coverage_capture_record(&cfg.repo.repo_id, commit_sha.trim(), scenario_id);
        repository
            .insert_coverage_capture(&capture)
            .context("seed coverage capture")?;
        repository
            .insert_coverage_hits(&coverage_hits(
                &seeded.symbol_id,
                &seeded.path,
                &capture.capture_id,
            ))
            .context("seed coverage hits")?;
    }

    let query = match mode {
        RegisteredStageQueryMode::Current => format!(
            r#"repo("temp2")->file("{}")->artefacts(kind:"function")->{}()->limit(10)"#,
            seeded.path, stage_name
        ),
        RegisteredStageQueryMode::AsOfCommit => format!(
            r#"repo("temp2")->asOf(commit:"{}")->file("{}")->artefacts(kind:"function")->{}()->limit(10)"#,
            commit_sha.trim(),
            seeded.path,
            stage_name
        ),
        RegisteredStageQueryMode::AsOfRef => format!(
            r#"repo("temp2")->asOf(ref:"main")->file("{}")->artefacts(kind:"function")->{}()->limit(10)"#,
            seeded.path, stage_name
        ),
    };

    let expected_artefact_id = match mode {
        RegisteredStageQueryMode::Current => seeded.current_artefact_id.as_str(),
        RegisteredStageQueryMode::AsOfCommit | RegisteredStageQueryMode::AsOfRef => {
            seeded.historical_artefact_id.as_str()
        }
    };

    let parsed = parse_devql_query(&query).context("parse query")?;
    let relational = RelationalStorage::local_only(sqlite_path.clone());
    let events_cfg = crate::config::EventsBackendConfig {
        duckdb_path: None,
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };
    let base_rows = execute_devql_query(&cfg, &parsed, &events_cfg, Some(&relational))
        .await
        .context("execute base pipeline")?;
    let mut rows = execute_registered_stages(&cfg, &parsed, base_rows)
        .await
        .context("execute registered stage")?;
    let row = rows
        .pop()
        .context("expected registered stage response row")?;
    if stage_name == "tests"
        && row
            .get("covering_tests")
            .and_then(Value::as_array)
            .is_some_and(|tests| tests.is_empty())
    {
        let mode_label = match mode {
            RegisteredStageQueryMode::Current => "current",
            RegisteredStageQueryMode::AsOfCommit => "asof_commit",
            RegisteredStageQueryMode::AsOfRef => "asof_ref",
        };
        eprintln!(
            "empty tests() response for mode {mode_label}: {}",
            serde_json::to_string_pretty(&row).unwrap_or_else(|_| "<unserializable>".to_string())
        );
    }
    let artefact_id = row
        .get("artefact")
        .and_then(|artefact| artefact.get("artefact_id"))
        .and_then(Value::as_str)
        .context("response artefact_id should exist")?;
    anyhow::ensure!(
        artefact_id == expected_artefact_id,
        "expected artefact_id `{expected_artefact_id}`, got `{artefact_id}`"
    );

    Ok(row)
}

fn when_test_discovery(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_test_discovery(world);
    })
}

fn when_linkage_resolution(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_linkage_resolution(world);
    })
}

fn when_linkage_and_tests_query(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "tests",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::Current,
            )
            .await
            .expect("execute current tests() query"),
        );
    })
}

fn when_linkage_and_tests_query_asof_commit(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "tests",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::AsOfCommit,
            )
            .await
            .expect("execute historical tests() commit query"),
        );
    })
}

fn when_linkage_and_tests_query_asof_ref(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "tests",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::AsOfRef,
            )
            .await
            .expect("execute historical tests() ref query"),
        );
    })
}

fn when_test_discovery_with_diagnostics(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let lang: tree_sitter::Language = LANGUAGE_RUST.into();
        let mut parser = Parser::new();
        parser.set_language(&lang).expect("set rust language");

        for (path, source) in &world.test_sources {
            let tree = parser.parse(source, None);
            if tree.is_none() || source.matches('{').count() != source.matches('}').count() {
                world.discovery_issues.push(
                    crate::capability_packs::test_harness::mapping::model::DiscoveryIssue {
                        path: path.clone(),
                        message: "parse error or incomplete source".to_string(),
                    },
                );
            }
        }
    })
}

fn then_test_suites_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let name = row.get("name").expect("name column should exist");
            let expected_count: usize = row
                .get("scenario_count")
                .expect("scenario_count column should exist")
                .parse()
                .expect("scenario_count should be numeric");

            let suite = world.discovered_suites.iter().find(|s| s.name == *name);

            assert!(
                suite.is_some(),
                "expected suite `{name}` in discovered suites, found: {:?}",
                world
                    .discovered_suites
                    .iter()
                    .map(|s| &s.name)
                    .collect::<Vec<_>>()
            );

            let suite = suite.unwrap();
            let actual_count = world
                .discovered_scenarios
                .iter()
                .filter(|scenario| scenario.parent_symbol_id.as_deref() == Some(&suite.symbol_id))
                .count();

            assert_eq!(
                actual_count, expected_count,
                "suite `{name}` expected {expected_count} scenarios, got {actual_count}"
            );
        }
    })
}

fn then_test_scenarios_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let name = row.get("name").expect("name column should exist");
            let discovery_source = row
                .get("discovery_source")
                .expect("discovery_source column should exist");

            let found = world
                .discovered_scenarios
                .iter()
                .any(|s| s.name == *name && s.discovery_source == *discovery_source);

            assert!(
                found,
                "expected scenario `{name}` with discovery_source `{discovery_source}`, found: {:?}",
                world
                    .discovered_scenarios
                    .iter()
                    .map(|s| (&s.name, &s.discovery_source))
                    .collect::<Vec<_>>()
            );
        }
    })
}

fn then_direct_links_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let production_name = row
                .get("production_name")
                .expect("production_name column should exist");
            let expected_confidence: f64 = row
                .get("confidence")
                .expect("confidence column should exist")
                .parse()
                .expect("confidence should be numeric");
            let expected_status = row
                .get("linkage_status")
                .expect("linkage_status column should exist");

            let found = world.materialized_links.iter().any(|link| {
                link.to_artefact_id
                    .as_deref()
                    .is_some_and(|artefact_id| artefact_id.contains(production_name))
                    && (link_confidence(link) - expected_confidence).abs() < 0.01
                    && link_status(link) == expected_status.as_str()
            });

            assert!(
                found,
                "expected link to `{production_name}` with confidence {expected_confidence} and status `{expected_status}`, found: {:?}",
                world
                    .materialized_links
                    .iter()
                    .map(|l| {
                        (
                            l.to_artefact_id.as_deref().unwrap_or("<unresolved>"),
                            link_confidence(l),
                            link_status(l).to_string(),
                        )
                    })
                    .collect::<Vec<_>>()
            );
        }
    })
}

fn then_no_links_are_created(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        assert!(
            world.materialized_links.is_empty(),
            "expected no links, got {:?}",
            world.materialized_links
        );
    })
}

fn then_no_links_to_from(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let production_name = &ctx.matches[1].1;
        let test_name = &ctx.matches[2].1;

        let found = world.materialized_links.iter().any(|link| {
            link.to_artefact_id
                .as_deref()
                .is_some_and(|artefact_id| artefact_id.contains(production_name.as_str()))
                && world
                    .discovered_scenarios
                    .iter()
                    .find(|scenario| scenario.symbol_id == link.from_symbol_id)
                    .is_some_and(|scenario| scenario.name.contains(test_name.as_str()))
        });

        assert!(
            !found,
            "expected no link from `{test_name}` to `{production_name}`, but found one"
        );
    })
}

fn then_diagnostics_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        for row in table_row_maps(&ctx) {
            let path = row.get("path").expect("path column should exist");
            let _severity = row.get("severity").expect("severity column should exist");

            let found = world
                .discovery_issues
                .iter()
                .any(|issue| issue.path == *path);
            assert!(
                found,
                "expected diagnostic for path `{path}`, found: {:?}",
                world.discovery_issues
            );
        }
    })
}

fn then_response_has_covering_tests(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let response = world
            .tests_query_response
            .as_ref()
            .expect("tests query response should be set");
        let covering_tests = response
            .get("covering_tests")
            .and_then(Value::as_array)
            .expect("response should have covering_tests");

        for row in table_row_maps(&ctx) {
            let test_name = row.get("test_name").expect("test_name column should exist");
            let expected_confidence: f64 = row
                .get("confidence")
                .expect("confidence column should exist")
                .parse()
                .expect("confidence should be numeric");

            let found = covering_tests.iter().any(|test| {
                test.get("test_name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n == test_name)
                    && test
                        .get("confidence")
                        .and_then(Value::as_f64)
                        .is_some_and(|c| (c - expected_confidence).abs() < 0.01)
            });

            assert!(
                found,
                "expected covering test `{test_name}` with confidence {expected_confidence}, found: {covering_tests:?}"
            );
        }
    })
}

fn then_logs_validation_error(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let reason = &ctx.matches[1].1;
        let entries = world.read_log_entries();
        assert!(
            entries.iter().any(|entry| {
                entry.get("msg").and_then(Value::as_str) == Some("devql query validation failed")
                    && entry.get("reason").and_then(Value::as_str) == Some(reason.as_str())
                    && entry.get("component").and_then(Value::as_str) == Some("devql")
            }),
            "expected validation-error log entry containing `{reason}`, got {entries:#?}"
        );
    })
}

fn when_coverage_ingested_and_query(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "coverage",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::Current,
            )
            .await
            .expect("execute current coverage() query"),
        );
    })
}

fn when_coverage_ingested_and_query_asof_commit(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "coverage",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::AsOfCommit,
            )
            .await
            .expect("execute historical coverage() commit query"),
        );
    })
}

fn when_coverage_ingested_and_query_asof_ref(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.tests_query_response = Some(
            execute_registered_stage_query(
                world,
                "coverage",
                &ctx.matches[1].1,
                RegisteredStageQueryMode::AsOfRef,
            )
            .await
            .expect("execute historical coverage() ref query"),
        );
    })
}

fn then_response_has_coverage_pct(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let response = world
            .tests_query_response
            .as_ref()
            .expect("coverage query response should be set");
        let coverage = response
            .get("coverage")
            .expect("response should have coverage");
        let line_pct = coverage
            .get("line_coverage_pct")
            .and_then(Value::as_f64)
            .expect("coverage should have line_coverage_pct");
        assert!(
            line_pct >= 0.0,
            "line_coverage_pct should be non-negative, got {line_pct}"
        );
    })
}

fn then_response_artefact_has_id(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let response = world
            .tests_query_response
            .as_ref()
            .expect("stage response should be set");
        let artefact_id = response
            .get("artefact")
            .and_then(|artefact| artefact.get("artefact_id"))
            .and_then(Value::as_str)
            .expect("response should include artefact.artefact_id");
        assert_eq!(artefact_id, expected, "unexpected response artefact_id");
    })
}

pub(super) fn collection() -> Collection<DevqlBddWorld> {
    Collection::new()
        .given(
            None,
            regex(r#"^a TypeScript source file at "([^"]+)":$"#),
            step_fn(given_typescript_source),
        )
        .given(
            None,
            regex(r#"^a Rust source file at "([^"]+)":$"#),
            step_fn(given_rust_source),
        )
        .given(
            None,
            regex(r#"^a Rust production file at "([^"]+)":$"#),
            step_fn(given_rust_production_file),
        )
        .given(
            None,
            regex(r#"^a Rust test file at "([^"]+)":$"#),
            step_fn(given_rust_test_file),
        )
        .given(
            None,
            regex(r#"^the semantic clone fixture "([^"]+)" is indexed$"#),
            step_fn(given_semantic_clone_fixture),
        )
        .given(
            None,
            regex(r#"^the semantic clone incremental fixture "([^"]+)" is prepared$"#),
            step_fn(given_semantic_clone_incremental_fixture),
        )
        .given(
            None,
            regex(r#"^a Stage 1 semantic input for a "([^"]+)" named "([^"]+)"$"#),
            step_fn(given_stage1_input_for_kind_named),
        )
        .given(
            None,
            regex(r"^the Stage 1 docstring is empty$"),
            step_fn(given_stage1_docstring_is_empty),
        )
        .given(
            None,
            regex(r"^the Stage 1 docstring is:$"),
            step_fn(given_stage1_docstring),
        )
        .given(
            None,
            regex(r"^the Stage 1 summary provider returns no candidate$"),
            step_fn(given_stage1_summary_provider_returns_no_candidate),
        )
        .given(
            None,
            regex(r"^the Stage 1 summary provider returns:$"),
            step_fn(given_stage1_summary_provider_returns_candidate),
        )
        .given(
            None,
            regex(r"^the Stage 1 summary provider returns invalid candidate:$"),
            step_fn(given_stage1_summary_provider_returns_invalid_candidate),
        )
        .when(
            None,
            regex(r"^devql ingest extracts artefacts$"),
            step_fn(when_extract_artefacts),
        )
        .when(
            None,
            regex(r"^devql ingest extracts dependency edges$"),
            step_fn(when_extract_dependency_edges),
        )
        .when(
            None,
            regex(r"^devql parses the query:$"),
            step_fn(when_parse_query),
        )
        .when(
            None,
            regex(r"^clones\(\) query executes:$"),
            step_fn(when_clones_query_executes),
        )
        .when(
            None,
            regex(r"^semantic clone incremental indexing runs across two snapshots$"),
            step_fn(when_semantic_clone_incremental_indexing_runs_across_two_snapshots),
        )
        .when(
            None,
            regex(r"^devql builds the deps SQL$"),
            step_fn(when_build_deps_sql),
        )
        .when(
            None,
            regex(r"^devql executes the query without a Postgres client$"),
            step_fn(when_execute_query_without_pg_client),
        )
        .when(
            None,
            regex(r"^devql extracts artefacts and dependency edges with logger capture$"),
            step_fn(when_extract_artefacts_and_edges_with_logger),
        )
        .then(
            None,
            regex(r"^artefacts include:$"),
            step_fn(then_artefacts_include),
        )
        .then(
            None,
            regex(r"^edges include:$"),
            step_fn(then_edges_include),
        )
        .then(
            None,
            regex(r"^no artefacts are emitted$"),
            step_fn(then_no_artefacts_are_emitted),
        )
        .then(
            None,
            regex(r"^no edges are emitted$"),
            step_fn(then_no_edges_are_emitted),
        )
        .then(
            None,
            regex(r"^the generated SQL contains:$"),
            step_fn(then_generated_sql_contains),
        )
        .then(
            None,
            regex(r#"^the query fails with message containing "([^"]+)"$"#),
            step_fn(then_query_fails_with_message),
        )
        .then(
            None,
            regex(r#"^the export edge named "([^"]+)" appears (\d+) time\(s\)$"#),
            step_fn(then_export_edge_named_appears_count),
        )
        .then(
            None,
            regex(r#"^devql logs a parse-failure event with path "([^"]+)"$"#),
            step_fn(then_logs_parse_failure),
        )
        .then(
            None,
            regex(r#"^devql logs a validation-error event containing "([^"]+)"$"#),
            step_fn(then_logs_validation_error),
        )
        .when(
            None,
            regex(r"^test discovery runs$"),
            step_fn(when_test_discovery),
        )
        .when(
            None,
            regex(r"^linkage resolution runs$"),
            step_fn(when_linkage_resolution),
        )
        .when(
            None,
            regex(r#"^linkage resolution runs and tests\(\) query executes for "([^"]+)"$"#),
            step_fn(when_linkage_and_tests_query),
        )
        .when(
            None,
            regex(r#"^linkage resolution runs and asOf\(commit\) tests\(\) query executes for "([^"]+)"$"#),
            step_fn(when_linkage_and_tests_query_asof_commit),
        )
        .when(
            None,
            regex(r#"^linkage resolution runs and asOf\(ref\) tests\(\) query executes for "([^"]+)"$"#),
            step_fn(when_linkage_and_tests_query_asof_ref),
        )
        .when(
            None,
            regex(r"^test discovery runs with diagnostics$"),
            step_fn(when_test_discovery_with_diagnostics),
        )
        .then(
            None,
            regex(r"^test suites include:$"),
            step_fn(then_test_suites_include),
        )
        .then(
            None,
            regex(r"^test scenarios include:$"),
            step_fn(then_test_scenarios_include),
        )
        .then(
            None,
            regex(r"^direct links include:$"),
            step_fn(then_direct_links_include),
        )
        .then(
            None,
            regex(r"^no links are created$"),
            step_fn(then_no_links_are_created),
        )
        .then(
            None,
            regex(r#"^no links to "([^"]+)" from "([^"]+)"$"#),
            step_fn(then_no_links_to_from),
        )
        .then(
            None,
            regex(r"^diagnostics include:$"),
            step_fn(then_diagnostics_include),
        )
        .then(
            None,
            regex(r"^the response has covering_tests with:$"),
            step_fn(then_response_has_covering_tests),
        )
        .then(
            None,
            regex(r#"^the response artefact has artefact_id "([^"]+)"$"#),
            step_fn(then_response_artefact_has_id),
        )
        .when(
            None,
            regex(r#"^coverage is ingested and coverage\(\) query executes for "([^"]+)"$"#),
            step_fn(when_coverage_ingested_and_query),
        )
        .when(
            None,
            regex(r#"^coverage is ingested and asOf\(commit\) coverage\(\) query executes for "([^"]+)"$"#),
            step_fn(when_coverage_ingested_and_query_asof_commit),
        )
        .when(
            None,
            regex(r#"^coverage is ingested and asOf\(ref\) coverage\(\) query executes for "([^"]+)"$"#),
            step_fn(when_coverage_ingested_and_query_asof_ref),
        )
        .when(
            None,
            regex(r"^Stage 1 persists semantic feature rows through the original pipeline$"),
            step_fn(when_stage1_persists_semantic_feature_rows_through_original_pipeline),
        )
        .when(
            None,
            regex(r"^Stage 2 starts with invalid embedding provider configuration$"),
            step_fn(when_stage2_starts_with_invalid_embedding_provider_configuration),
        )
        .when(
            None,
            regex(r#"^Stage 2 starts with embedding provider configuration "([^"]+)"$"#),
            step_fn(when_stage2_starts_with_named_embedding_provider_configuration),
        )
        .then(
            None,
            regex(r"^the response has coverage with line_coverage_pct$"),
            step_fn(then_response_has_coverage_pct),
        )
        .then(
            None,
            regex(r"^clone rows include:$"),
            step_fn(then_clone_rows_include),
        )
        .then(
            None,
            regex(r"^every clone row includes explainable scores$"),
            step_fn(then_every_clone_row_includes_explainable_scores),
        )
        .then(
            None,
            regex(r#"^the first clone row targets "([^"]+)"$"#),
            step_fn(then_first_clone_row_targets),
        )
        .then(
            None,
            regex(r#"^the first clone row has label "([^"]+)"$"#),
            step_fn(then_first_clone_row_has_label),
        )
        .then(
            None,
            regex(r#"^the clone row for "([^"]+)" has bias warning "([^"]+)"$"#),
            step_fn(then_clone_row_for_target_has_bias_warning),
        )
        .then(
            None,
            regex(r#"^the clone row for "([^"]+)" has metric "([^"]+)" at least ([0-9]+(?:\.[0-9]+)?)$"#),
            step_fn(then_clone_row_metric_at_least),
        )
        .then(
            None,
            regex(r#"^the clone row for "([^"]+)" has metric "([^"]+)" at most ([0-9]+(?:\.[0-9]+)?)$"#),
            step_fn(then_clone_row_metric_at_most),
        )
        .then(
            None,
            regex(r#"^the clone row for "([^"]+)" has shared signal "([^"]+)"$"#),
            step_fn(then_clone_row_has_shared_signal),
        )
        .then(
            None,
            regex(r#"^the clone row for "([^"]+)" has duplicate signal "([^"]+)" set to (true|false)$"#),
            step_fn(then_clone_row_duplicate_signal_is),
        )
        .then(
            None,
            regex(r#"^the clone row for "([^"]+)" has explanation fact "([^"]+)" set to (true|false)$"#),
            step_fn(then_clone_row_fact_is),
        )
        .then(
            None,
            regex(r#"^the clone row for "([^"]+)" has limiting signal "([^"]+)"$"#),
            step_fn(then_clone_row_has_limiting_signal),
        )
        .then(
            None,
            regex(r#"^the clone row for "([^"]+)" ranks below "([^"]+)"$"#),
            step_fn(then_clone_row_ranks_below),
        )
        .then(
            None,
            regex(r#"^no clone row targets "([^"]+)"$"#),
            step_fn(then_no_clone_row_targets),
        )
        .then(
            None,
            regex(r#"^the clone row for "([^"]+)" does not have label "([^"]+)"$"#),
            step_fn(then_clone_row_for_target_does_not_have_label),
        )
        .then(
            None,
            regex(r#"^the persisted Stage 1 final summary is "([^"]+)"$"#),
            step_fn(then_persisted_stage1_final_summary_is),
        )
        .then(
            None,
            regex(r#"^the persisted Stage 1 template summary is "([^"]+)"$"#),
            step_fn(then_persisted_stage1_template_summary_is),
        )
        .then(
            None,
            regex(r#"^the persisted Stage 1 final summary starts with "([^"]+)"$"#),
            step_fn(then_persisted_stage1_final_summary_starts_with),
        )
        .then(
            None,
            regex(r#"^the persisted Stage 1 final summary is not "([^"]+)"$"#),
            step_fn(then_persisted_stage1_final_summary_is_not),
        )
        .then(
            None,
            regex(r"^Stage 1 incremental stats are (\d+) upserted and (\d+) skipped$"),
            step_fn(then_stage1_incremental_stats_are),
        )
        .then(
            None,
            regex(r"^Stage 2 incremental stats are (\d+) upserted and (\d+) skipped$"),
            step_fn(then_stage2_incremental_stats_are),
        )
        .then(
            None,
            regex(r#"^the semantic features hash for "([^"]+)" is unchanged across snapshots$"#),
            step_fn(then_semantic_feature_hash_is_unchanged_across_snapshots),
        )
        .then(
            None,
            regex(r#"^the embedding hash for "([^"]+)" is unchanged across snapshots$"#),
            step_fn(then_embedding_hash_is_unchanged_across_snapshots),
        )
        .then(
            None,
            regex(r#"^the clone edge hash from "([^"]+)" to "([^"]+)" is unchanged across snapshots$"#),
            step_fn(then_clone_edge_hash_is_unchanged_across_snapshots),
        )
        .then(
            None,
            regex(r#"^the clone edge hash from "([^"]+)" to "([^"]+)" changes across snapshots$"#),
            step_fn(then_clone_edge_hash_changes_across_snapshots),
        )
        .then(
            None,
            regex(r#"^Stage 2 fails with message containing "([^"]+)"$"#),
            step_fn(then_stage2_fails_with_message_containing),
        )
        .then(
            None,
            regex(r"^Stage 2 writes (\d+) embedding rows$"),
            step_fn(then_stage2_writes_embedding_rows),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_world() -> DevqlBddWorld {
        let mut world = DevqlBddWorld::default();
        world.production_sources.push((
            "src/user/service.rs".to_string(),
            r#"
pub fn create_user(name: &str) -> String {
    name.to_string()
}

pub fn delete_user(id: u64) -> bool {
    let _ = id;
    true
}
"#
            .trim()
            .to_string(),
        ));
        world.test_sources.push((
            "src/user/service_tests.rs".to_string(),
            r#"
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_create_user() {
        create_user("Alice");
    }
}
"#
            .trim()
            .to_string(),
        ));
        world
    }

    #[tokio::test]
    async fn execute_registered_stage_query_tests_current_uses_live_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "tests",
            "create_user",
            RegisteredStageQueryMode::Current,
        )
        .await
        .expect("execute current tests query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("current:src/user/service.rs:create_user".to_string())
        );
        assert_eq!(response["covering_tests"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn execute_registered_stage_query_tests_asof_commit_uses_historical_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "tests",
            "create_user",
            RegisteredStageQueryMode::AsOfCommit,
        )
        .await
        .expect("execute commit tests query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("historical:src/user/service.rs:create_user".to_string())
        );
        assert_eq!(response["covering_tests"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn execute_registered_stage_query_tests_asof_ref_uses_historical_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "tests",
            "create_user",
            RegisteredStageQueryMode::AsOfRef,
        )
        .await
        .expect("execute ref tests query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("historical:src/user/service.rs:create_user".to_string())
        );
        assert_eq!(response["covering_tests"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn execute_registered_stage_query_tests_current_does_not_invent_links_for_other_artefacts()
     {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "tests",
            "delete_user",
            RegisteredStageQueryMode::Current,
        )
        .await
        .expect("execute current tests query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("current:src/user/service.rs:delete_user".to_string())
        );
        assert_eq!(response["covering_tests"].as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn execute_registered_stage_query_coverage_current_uses_live_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "coverage",
            "create_user",
            RegisteredStageQueryMode::Current,
        )
        .await
        .expect("execute current coverage query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("current:src/user/service.rs:create_user".to_string())
        );
        assert!(
            response["coverage"]["line_coverage_pct"]
                .as_f64()
                .is_some_and(|value| value >= 0.0)
        );
    }

    #[tokio::test]
    async fn execute_registered_stage_query_coverage_asof_commit_uses_historical_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "coverage",
            "create_user",
            RegisteredStageQueryMode::AsOfCommit,
        )
        .await
        .expect("execute commit coverage query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("historical:src/user/service.rs:create_user".to_string())
        );
        assert!(
            response["coverage"]["line_coverage_pct"]
                .as_f64()
                .is_some_and(|value| value >= 0.0)
        );
    }

    #[tokio::test]
    async fn execute_registered_stage_query_coverage_asof_ref_uses_historical_artefact_rows() {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "coverage",
            "create_user",
            RegisteredStageQueryMode::AsOfRef,
        )
        .await
        .expect("execute ref coverage query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("historical:src/user/service.rs:create_user".to_string())
        );
        assert!(
            response["coverage"]["line_coverage_pct"]
                .as_f64()
                .is_some_and(|value| value >= 0.0)
        );
    }

    #[tokio::test]
    async fn execute_registered_stage_query_coverage_current_does_not_invent_hits_for_other_artefacts()
     {
        let mut world = fixture_world();
        let response = execute_registered_stage_query(
            &mut world,
            "coverage",
            "delete_user",
            RegisteredStageQueryMode::Current,
        )
        .await
        .expect("execute current coverage query");

        assert_eq!(
            response["artefact"]["artefact_id"],
            Value::String("current:src/user/service.rs:delete_user".to_string())
        );
        assert_eq!(
            response["coverage"]["line_data_available"].as_bool(),
            Some(false)
        );
        assert_eq!(
            response["coverage"]["branch_data_available"].as_bool(),
            Some(false)
        );
    }
}
