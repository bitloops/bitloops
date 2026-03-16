// Command handler for discovering test suites/scenarios and establishing static
// links from test artefacts to referenced production artefacts.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};
use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;
use tree_sitter_typescript::LANGUAGE_TYPESCRIPT;
use walkdir::WalkDir;

use crate::domain::{ArtefactRecord, ProductionArtefact, TestLinkRecord};
use crate::repository::{TestHarnessRepository, open_sqlite_repository};

#[derive(Debug, Clone)]
struct ParsedSuite {
    name: String,
    start_line: i64,
    end_line: i64,
    scenarios: Vec<ParsedScenario>,
}

#[derive(Debug, Clone)]
struct ParsedScenario {
    name: String,
    start_line: i64,
    end_line: i64,
    called_symbols: HashSet<String>,
    explicit_targets: HashSet<ExplicitProductionTarget>,
    discovery_source: ScenarioDiscoverySource,
}

#[derive(Debug, Clone)]
struct ParsedTestFile {
    line_count: i64,
    import_paths: HashSet<String>,
    suites: Vec<ParsedSuite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScenarioDiscoverySource {
    Source,
    MacroGenerated,
    Doctest,
    Enumeration,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ExplicitProductionTarget {
    path: String,
    start_line: i64,
}

#[derive(Debug, Clone)]
struct DiscoveredTestFile {
    relative_path: String,
    adapter_index: usize,
    priority: u8,
}

#[derive(Debug, Clone)]
struct EnumeratedRustScenario {
    suite_name: String,
    scenario_name: String,
    relative_path: String,
    start_line: i64,
    explicit_targets: HashSet<ExplicitProductionTarget>,
    discovery_source: ScenarioDiscoverySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RustEnumerationMode {
    Skipped,
    Partial,
    Full,
}

#[derive(Debug, Clone)]
struct RustEnumerationResult {
    mode: RustEnumerationMode,
    scenarios: Vec<EnumeratedRustScenario>,
    notes: Vec<String>,
}

#[derive(Debug, Default)]
struct ProductionIndex {
    by_simple_symbol: HashMap<String, Vec<usize>>,
    by_explicit_target: HashMap<(String, i64), usize>,
}

impl Default for RustEnumerationResult {
    fn default() -> Self {
        Self {
            mode: RustEnumerationMode::Skipped,
            scenarios: Vec::new(),
            notes: Vec::new(),
        }
    }
}

impl RustEnumerationResult {
    fn status_label(&self) -> &'static str {
        match self.mode {
            RustEnumerationMode::Skipped => "source-only",
            RustEnumerationMode::Partial => "hybrid-partial",
            RustEnumerationMode::Full => "hybrid-full",
        }
    }
}

trait TestLanguageAdapter {
    fn language(&self) -> &'static str;
    fn priority(&self) -> u8;
    fn supports_path(&self, absolute_path: &Path, relative_path: &str) -> bool;
    fn parse_file(&mut self, absolute_path: &Path, relative_path: &str) -> Result<ParsedTestFile>;
}

struct TypeScriptTestAdapter {
    parser: Parser,
}

impl TypeScriptTestAdapter {
    fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_TYPESCRIPT.into())
            .context("failed to load TypeScript parser")?;
        Ok(Self { parser })
    }
}

impl TestLanguageAdapter for TypeScriptTestAdapter {
    fn language(&self) -> &'static str {
        "typescript"
    }

    fn priority(&self) -> u8 {
        1
    }

    fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
        relative_path.ends_with(".test.ts")
            || relative_path.ends_with(".spec.ts")
            || relative_path.contains("/__tests__/")
    }

    fn parse_file(&mut self, absolute_path: &Path, relative_path: &str) -> Result<ParsedTestFile> {
        let source = read_source_file(absolute_path)?;
        let tree = self
            .parser
            .parse(&source, None)
            .with_context(|| format!("failed parsing test file {}", absolute_path.display()))?;
        let bytes = source.as_bytes();
        let root = tree.root_node();

        let import_paths = collect_typescript_import_paths(root, bytes, relative_path);
        let suites = collect_typescript_suites(root, bytes);

        Ok(ParsedTestFile {
            line_count: line_count(&source),
            import_paths,
            suites,
        })
    }
}

struct RustTestAdapter {
    parser: Parser,
}

impl RustTestAdapter {
    fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .context("failed to load Rust parser")?;
        Ok(Self { parser })
    }
}

impl TestLanguageAdapter for RustTestAdapter {
    fn language(&self) -> &'static str {
        "rust"
    }

    fn priority(&self) -> u8 {
        0
    }

    fn supports_path(&self, absolute_path: &Path, relative_path: &str) -> bool {
        relative_path.ends_with(".test.rs")
            || relative_path.ends_with(".spec.rs")
            || ((relative_path.starts_with("tests/") || relative_path.contains("/tests/"))
                && relative_path.ends_with(".rs"))
            || looks_like_inline_rust_test_source(absolute_path, relative_path)
    }

    fn parse_file(&mut self, absolute_path: &Path, relative_path: &str) -> Result<ParsedTestFile> {
        let source = read_source_file(absolute_path)?;
        let tree = self
            .parser
            .parse(&source, None)
            .with_context(|| format!("failed parsing test file {}", absolute_path.display()))?;
        let bytes = source.as_bytes();
        let root = tree.root_node();

        let mut import_paths = collect_rust_import_paths_for(root, bytes, relative_path);
        import_paths.extend(collect_rust_scoped_call_import_paths_for(
            root,
            bytes,
            relative_path,
        ));
        import_paths.extend(rust_test_context_source_paths(relative_path));
        let suites = collect_rust_suites(root, &source, relative_path);

        Ok(ParsedTestFile {
            line_count: line_count(&source),
            import_paths,
            suites,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct IngestTestsStats {
    files: usize,
    suites: usize,
    scenarios: usize,
    links: usize,
    enumerated_scenarios: usize,
}

pub fn handle(db_path: &Path, repo_dir: &Path, commit_sha: &str) -> Result<()> {
    let mut repository = open_sqlite_repository(db_path)?;
    let repo_id = repository.load_repo_id_for_commit(commit_sha)?;
    let production = repository.load_production_artefacts(commit_sha)?;
    let production_index = build_production_index(&production);
    let rust_enumeration = enumerate_rust_tests(repo_dir);

    let mut adapters: Vec<Box<dyn TestLanguageAdapter>> = vec![
        Box::new(RustTestAdapter::new()?),
        Box::new(TypeScriptTestAdapter::new()?),
    ];

    let test_files = discover_test_files(repo_dir, &adapters)?;
    let mut stats = IngestTestsStats::default();
    let mut link_keys = HashSet::new();
    let mut artefacts = Vec::new();
    let mut links = Vec::new();
    let mut source_scenario_keys = HashSet::new();
    let mut source_doctest_keys = HashSet::new();
    let mut synthetic_suites = HashMap::new();

    for discovered in test_files {
        stats.files += 1;
        let absolute_path = repo_dir.join(&discovered.relative_path);

        let adapter = &mut adapters[discovered.adapter_index];
        let language = adapter.language().to_string();
        let parsed = adapter.parse_file(&absolute_path, &discovered.relative_path)?;
        if parsed.suites.is_empty() {
            continue;
        }

        let test_file_id = format!("test_file:{commit_sha}:{}", discovered.relative_path);
        artefacts.push(build_artefact_record(
            &test_file_id,
            &repo_id,
            commit_sha,
            &discovered.relative_path,
            &language,
            "file",
            Some("source_file"),
            Some(&discovered.relative_path),
            None,
            1,
            parsed.line_count,
            None,
        ));

        let imported_paths: HashSet<String> = parsed
            .import_paths
            .iter()
            .filter(|item| looks_like_production_source_path(item))
            .cloned()
            .collect();

        for suite in parsed.suites {
            stats.suites += 1;
            let suite_id = format!(
                "test_suite:{commit_sha}:{}:{}",
                discovered.relative_path, suite.start_line
            );
            artefacts.push(build_artefact_record(
                &suite_id,
                &repo_id,
                commit_sha,
                &discovered.relative_path,
                &language,
                "test_suite",
                Some("suite_block"),
                Some(&suite.name),
                Some(&test_file_id),
                suite.start_line,
                suite.end_line,
                Some(&suite.name),
            ));

            for scenario in suite.scenarios {
                stats.scenarios += 1;
                source_scenario_keys.extend(source_scenario_match_keys(
                    &discovered.relative_path,
                    &suite.name,
                    &scenario.name,
                ));
                if scenario.discovery_source == ScenarioDiscoverySource::Doctest {
                    source_doctest_keys.extend(doctest_match_keys(
                        &discovered.relative_path,
                        &scenario.name,
                        &scenario.explicit_targets,
                    ));
                }
                let scenario_id = format!(
                    "test_case:{commit_sha}:{}:{}:{}",
                    discovered.relative_path,
                    scenario.start_line,
                    scenario_id_suffix(&scenario.name),
                );
                let scenario_fqn = format!("{}.{}", suite.name, scenario.name);
                artefacts.push(build_artefact_record(
                    &scenario_id,
                    &repo_id,
                    commit_sha,
                    &discovered.relative_path,
                    &language,
                    "test_scenario",
                    Some("test_block"),
                    Some(&scenario_fqn),
                    Some(&suite_id),
                    scenario.start_line,
                    scenario.end_line,
                    Some(&scenario.name),
                ));

                for production_artefact in matched_production_artefacts(
                    &production,
                    &production_index,
                    &imported_paths,
                    &scenario.called_symbols,
                    &scenario.explicit_targets,
                ) {
                    let link_key = format!("{}::{}", scenario_id, production_artefact.artefact_id);
                    if !link_keys.insert(link_key) {
                        continue;
                    }

                    let link_id = format!(
                        "link:{commit_sha}:{}:{}",
                        scenario_id, production_artefact.artefact_id
                    );
                    links.push(build_test_link_record(
                        &link_id,
                        &scenario_id,
                        &production_artefact.artefact_id,
                        commit_sha,
                    ));
                    stats.links += 1;
                }
            }
        }
    }

    materialize_enumerated_rust_scenarios(
        &repo_id,
        commit_sha,
        &production,
        &production_index,
        &rust_enumeration,
        &source_scenario_keys,
        &source_doctest_keys,
        &mut synthetic_suites,
        &mut artefacts,
        &mut links,
        &mut link_keys,
        &mut stats,
    );

    repository.replace_test_discovery(commit_sha, &artefacts, &links)?;
    println!(
        "ingest-tests complete for commit {} (files: {}, suites: {}, scenarios: {}, links: {}, enumeration: {}, enumerated_scenarios: {})",
        commit_sha,
        stats.files,
        stats.suites,
        stats.scenarios,
        stats.links,
        rust_enumeration.status_label(),
        stats.enumerated_scenarios,
    );
    for note in rust_enumeration.notes {
        println!("ingest-tests note: {note}");
    }
    Ok(())
}

fn discover_test_files(
    repo_dir: &Path,
    adapters: &[Box<dyn TestLanguageAdapter>],
) -> Result<Vec<DiscoveredTestFile>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(repo_dir)
        .into_iter()
        .filter_entry(|entry| !is_ignored_path(entry.path()))
        .filter_map(|item| item.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let relative = entry
            .path()
            .strip_prefix(repo_dir)
            .with_context(|| format!("file {} is not under repo dir", entry.path().display()))?;
        let relative_path = normalize_rel_path(relative);

        let Some((adapter_index, priority)) =
            adapters.iter().enumerate().find_map(|(index, adapter)| {
                adapter
                    .supports_path(entry.path(), &relative_path)
                    .then_some((index, adapter.priority()))
            })
        else {
            continue;
        };

        files.push(DiscoveredTestFile {
            relative_path,
            adapter_index,
            priority,
        });
    }

    files.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(a.relative_path.cmp(&b.relative_path))
    });
    Ok(files)
}

fn is_ignored_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.contains("/node_modules/")
        || normalized.contains("/coverage/")
        || normalized.contains("/dist/")
        || normalized.contains("/target/")
}

fn looks_like_inline_rust_test_source(absolute_path: &Path, relative_path: &str) -> bool {
    if !relative_path.ends_with(".rs") {
        return false;
    }
    if !(relative_path.starts_with("src/") || relative_path.contains("/src/")) {
        return false;
    }

    let Ok(source) = fs::read_to_string(absolute_path) else {
        return false;
    };

    rust_source_contains_test_markers(&source) || rust_source_contains_doctest_markers(&source)
}

fn rust_source_contains_test_markers(source: &str) -> bool {
    source.contains("#[cfg(test)]")
        || source.contains("#[test")
        || source.contains("::test")
        || source.contains("#[test_case")
        || source.contains("::test_case")
        || source.contains("#[rstest")
        || source.contains("::rstest")
        || source.contains("#[wasm_bindgen_test")
        || source.contains("::wasm_bindgen_test")
        || source.contains("#[quickcheck")
        || source.contains("::quickcheck")
        || source.contains("proptest!")
}

fn rust_source_contains_doctest_markers(source: &str) -> bool {
    let mut in_block_doc = false;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("/// ```") || trimmed.starts_with("//! ```") {
            return true;
        }

        if trimmed.starts_with("/**") || trimmed.starts_with("/*!") {
            in_block_doc = true;
            if trimmed.contains("```") {
                return true;
            }
        } else if in_block_doc && trimmed.contains("```") {
            return true;
        }

        if in_block_doc && trimmed.contains("*/") {
            in_block_doc = false;
        }
    }

    false
}

fn read_source_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed reading test file {}", path.display()))
}

fn line_count(source: &str) -> i64 {
    std::cmp::max(source.lines().count() as i64, 1)
}

fn collect_typescript_import_paths(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> HashSet<String> {
    let mut results = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "import_statement"
            && let Ok(statement) = node.utf8_text(source)
            && let Some(raw_import) = extract_import_specifier(statement)
            && let Some(resolved) = resolve_import_to_repo_path(relative_path, raw_import)
        {
            results.insert(resolved);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    results
}

fn collect_typescript_suites(root: Node<'_>, source: &[u8]) -> Vec<ParsedSuite> {
    let mut suites = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && is_call_named(node, source, "describe")
            && let Some(suite_name) = extract_first_string_argument(node, source)
            && let Some(callback_body) = extract_second_callback_body(node)
        {
            let scenarios = collect_typescript_scenarios(callback_body, source);
            suites.push(ParsedSuite {
                name: suite_name,
                start_line: node.start_position().row as i64 + 1,
                end_line: node.end_position().row as i64 + 1,
                scenarios,
            });
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    suites.sort_by_key(|suite| suite.start_line);
    suites
}

fn collect_typescript_scenarios(scope: Node<'_>, source: &[u8]) -> Vec<ParsedScenario> {
    let mut scenarios = Vec::new();
    let mut stack = vec![scope];

    while let Some(node) = stack.pop() {
        let mut descend = true;

        if node.kind() == "call_expression" {
            let is_test_call =
                is_call_named(node, source, "it") || is_call_named(node, source, "test");
            let is_describe_call = is_call_named(node, source, "describe");

            if is_test_call && let Some(name) = extract_first_string_argument(node, source) {
                let called_symbols = extract_second_callback_body(node)
                    .map(|body| collect_typescript_called_symbols(body, source))
                    .unwrap_or_default();

                scenarios.push(ParsedScenario {
                    name,
                    start_line: node.start_position().row as i64 + 1,
                    end_line: node.end_position().row as i64 + 1,
                    called_symbols,
                    explicit_targets: HashSet::new(),
                    discovery_source: ScenarioDiscoverySource::Source,
                });
                descend = false;
            }

            if is_describe_call {
                descend = false;
            }
        }

        if descend {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }

    scenarios.sort_by_key(|scenario| scenario.start_line);
    scenarios
}

fn collect_typescript_called_symbols(scope: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut symbols = HashSet::new();
    let mut stack = vec![scope];

    while let Some(node) = stack.pop() {
        match node.kind() {
            "call_expression" => {
                if let Some(function_node) = node.child_by_field_name("function") {
                    match function_node.kind() {
                        "identifier" => {
                            if let Ok(name) = function_node.utf8_text(source) {
                                symbols.insert(name.to_string());
                            }
                        }
                        "member_expression" => {
                            if let Some(property) = function_node.child_by_field_name("property")
                                && let Ok(name) = property.utf8_text(source)
                            {
                                symbols.insert(name.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            "new_expression" => {
                if let Some(constructor_node) = node.child_by_field_name("constructor")
                    && let Ok(name) = constructor_node.utf8_text(source)
                {
                    symbols.insert(name.to_string());
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    symbols
}

#[derive(Debug, Clone)]
struct RustDiscoveredScenario {
    suite_name: String,
    suite_start_line: i64,
    suite_end_line: i64,
    scenario: ParsedScenario,
}

#[derive(Debug, Clone)]
struct RustScenarioSeed {
    name: String,
    start_line: i64,
    end_line: i64,
    extra_symbols: HashSet<String>,
    explicit_targets: HashSet<ExplicitProductionTarget>,
    discovery_source: ScenarioDiscoverySource,
}

fn collect_rust_suites(root: Node<'_>, source: &str, relative_path: &str) -> Vec<ParsedSuite> {
    let discovered = collect_rust_test_scenarios(root, source, relative_path);
    let mut grouped: BTreeMap<String, ParsedSuite> = BTreeMap::new();

    for item in discovered {
        let entry = grouped
            .entry(item.suite_name.clone())
            .or_insert_with(|| ParsedSuite {
                name: item.suite_name.clone(),
                start_line: item.suite_start_line,
                end_line: item.suite_end_line,
                scenarios: Vec::new(),
            });

        entry.start_line = entry.start_line.min(item.suite_start_line);
        entry.end_line = entry.end_line.max(item.suite_end_line);
        entry.scenarios.push(item.scenario);
    }

    let mut suites: Vec<ParsedSuite> = grouped.into_values().collect();
    for suite in &mut suites {
        suite.scenarios.sort_by_key(|scenario| scenario.start_line);
    }
    suites.sort_by(|a, b| a.start_line.cmp(&b.start_line).then(a.name.cmp(&b.name)));
    suites
}

fn collect_rust_test_scenarios(
    root: Node<'_>,
    source: &str,
    relative_path: &str,
) -> Vec<RustDiscoveredScenario> {
    let bytes = source.as_bytes();
    let rstest_templates = collect_rust_rstest_templates(root, source);
    let mut scenarios = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "function_item" {
            let scenario_seeds = rust_test_scenarios_for_function(node, source, &rstest_templates);
            if scenario_seeds.is_empty() {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    stack.push(child);
                }
                continue;
            }

            let body_symbols = node
                .child_by_field_name("body")
                .map(|body| collect_rust_called_symbols(body, bytes))
                .unwrap_or_default();

            let (suite_name, suite_start_line, suite_end_line) =
                rust_suite_for_function(node, bytes, relative_path);

            for seed in scenario_seeds {
                let mut called_symbols = body_symbols.clone();
                called_symbols.extend(seed.extra_symbols);

                scenarios.push(RustDiscoveredScenario {
                    suite_name: suite_name.clone(),
                    suite_start_line,
                    suite_end_line,
                    scenario: ParsedScenario {
                        name: seed.name,
                        start_line: seed.start_line,
                        end_line: seed.end_line,
                        called_symbols,
                        explicit_targets: seed.explicit_targets,
                        discovery_source: seed.discovery_source,
                    },
                });
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    scenarios.extend(collect_rust_macro_generated_scenarios(
        root,
        bytes,
        relative_path,
    ));
    scenarios.extend(collect_rust_proptest_scenarios(
        root,
        source,
        relative_path,
    ));
    scenarios.extend(collect_rust_doctest_scenarios(
        root,
        source,
        relative_path,
    ));
    scenarios.sort_by(|a, b| {
        a.suite_start_line
            .cmp(&b.suite_start_line)
            .then(a.scenario.start_line.cmp(&b.scenario.start_line))
    });
    scenarios
}

fn rust_test_scenarios_for_function(
    function_node: Node<'_>,
    source: &str,
    rstest_templates: &HashMap<String, Vec<RustScenarioSeed>>,
) -> Vec<RustScenarioSeed> {
    let source_bytes = source.as_bytes();
    if function_node.kind() != "function_item" {
        return Vec::new();
    }

    let Some(function_name) = function_node
        .child_by_field_name("name")
        .and_then(|name| name.utf8_text(source_bytes).ok())
        .map(str::to_string)
    else {
        return Vec::new();
    };

    let function_start_line = function_node.start_position().row as i64 + 1;
    let function_end_line = function_node.end_position().row as i64 + 1;

    let attributes = rust_function_attributes(function_node);
    let mut has_plain_test = false;
    let mut cases = Vec::new();
    let raw_attributes: Vec<String> = attributes
        .iter()
        .filter_map(|attribute| attribute.utf8_text(source_bytes).ok().map(str::to_string))
        .collect();
    if raw_attributes
        .iter()
        .any(|attribute| rust_attribute_name(attribute).as_deref() == Some("template"))
    {
        return Vec::new();
    }

    for attribute in attributes {
        if rust_attribute_is_test(attribute, source_bytes) {
            has_plain_test = true;
        }

        if rust_attribute_is_parameterized_test(attribute, source_bytes) {
            cases.push(build_rust_parameterized_test_case(
                &function_name,
                attribute,
                source_bytes,
                function_end_line,
            ));
        }
    }

    if let Some(template_name) = extract_rust_apply_template_name(&raw_attributes) {
        if let Some(template_cases) = rstest_templates.get(&template_name) {
            return template_cases
                .iter()
                .map(|seed| RustScenarioSeed {
                    name: seed.name.replacen(&template_name, &function_name, 1),
                    start_line: function_start_line,
                    end_line: function_end_line,
                    extra_symbols: seed.extra_symbols.clone(),
                    explicit_targets: HashSet::new(),
                    discovery_source: ScenarioDiscoverySource::Source,
                })
                .collect();
        }

        return vec![RustScenarioSeed {
            name: function_name,
            start_line: function_start_line,
            end_line: function_end_line,
            extra_symbols: HashSet::new(),
            explicit_targets: HashSet::new(),
            discovery_source: ScenarioDiscoverySource::Source,
        }];
    }

    let rstest_cases = build_rust_rstest_cases(
        &function_name,
        function_node,
        source,
        &raw_attributes,
        function_end_line,
        false,
    );
    if !rstest_cases.is_empty() {
        return rstest_cases;
    }

    if !cases.is_empty() {
        return cases;
    }

    if has_plain_test {
        return vec![RustScenarioSeed {
            name: function_name,
            start_line: function_start_line,
            end_line: function_end_line,
            extra_symbols: HashSet::new(),
            explicit_targets: HashSet::new(),
            discovery_source: ScenarioDiscoverySource::Source,
        }];
    }

    Vec::new()
}

fn collect_rust_macro_generated_scenarios(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> Vec<RustDiscoveredScenario> {
    let test_macro_names = collect_rust_test_generating_macro_names(root, source);
    if test_macro_names.is_empty() {
        return Vec::new();
    }

    let mut scenarios = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "macro_invocation"
            && let Ok(raw_invocation) = node.utf8_text(source)
            && let Some(macro_name) = extract_rust_macro_invocation_name(raw_invocation)
            && test_macro_names.contains(macro_name)
            && let Some(body) = extract_rust_macro_invocation_body(raw_invocation)
            && let Some(scenario_name) = extract_first_identifier_token(body)
        {
            let (suite_name, suite_start_line, suite_end_line) =
                rust_suite_for_node(node, source, relative_path);
            scenarios.push(RustDiscoveredScenario {
                suite_name,
                suite_start_line,
                suite_end_line,
                scenario: ParsedScenario {
                    name: scenario_name,
                    start_line: node.start_position().row as i64 + 1,
                    end_line: node.end_position().row as i64 + 1,
                    called_symbols: extract_callable_symbols_from_rust_text(body),
                    explicit_targets: HashSet::new(),
                    discovery_source: ScenarioDiscoverySource::MacroGenerated,
                },
            });
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    scenarios
}

fn collect_rust_test_generating_macro_names(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "macro_definition"
            && let Ok(raw_definition) = node.utf8_text(source)
            && rust_source_contains_test_markers(raw_definition)
            && let Some(name) = extract_rust_macro_definition_name(raw_definition)
        {
            names.insert(name.to_string());
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    names
}

fn collect_rust_rstest_templates(
    root: Node<'_>,
    source: &str,
) -> HashMap<String, Vec<RustScenarioSeed>> {
    let mut templates = HashMap::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "function_item" {
            let attributes = rust_function_attributes(node);
            let raw_attributes: Vec<String> = attributes
                .iter()
                .filter_map(|attribute| {
                    attribute
                        .utf8_text(source.as_bytes())
                        .ok()
                        .map(str::to_string)
                })
                .collect();

            if raw_attributes
                .iter()
                .any(|attribute| rust_attribute_name(attribute).as_deref() == Some("template"))
                && let Some(name) = node
                    .child_by_field_name("name")
                    .and_then(|name| name.utf8_text(source.as_bytes()).ok())
            {
                let seeds =
                    build_rust_rstest_cases(name, node, source, &raw_attributes, 0, true);
                if !seeds.is_empty() {
                    templates.insert(name.to_string(), seeds);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    templates
}

fn build_rust_rstest_cases(
    function_name: &str,
    function_node: Node<'_>,
    source: &str,
    raw_attributes: &[String],
    function_end_line: i64,
    allow_template_output: bool,
) -> Vec<RustScenarioSeed> {
    let source_bytes = source.as_bytes();
    let has_rstest = raw_attributes
        .iter()
        .any(|attribute| rust_attribute_name(attribute).as_deref() == Some("rstest"));
    let is_template = raw_attributes
        .iter()
        .any(|attribute| rust_attribute_name(attribute).as_deref() == Some("template"));
    if !has_rstest && !is_template {
        return Vec::new();
    }
    if is_template && !allow_template_output {
        return Vec::new();
    }

    let mut cases = Vec::new();
    for attribute in rust_function_attributes(function_node) {
        let Ok(raw_attribute) = attribute.utf8_text(source_bytes) else {
            continue;
        };
        if rust_attribute_name(raw_attribute).as_deref() != Some("case") {
            continue;
        }

        let body = extract_rust_attribute_args(raw_attribute).unwrap_or_default();
        let display = summarize_rstest_values(&split_top_level_arguments(body));
        let name = if display.is_empty() {
            format!("{function_name}[case]")
        } else {
            format!("{function_name}[{display}]")
        };
        cases.push(RustScenarioSeed {
            name,
            start_line: attribute.start_position().row as i64 + 1,
            end_line: function_end_line,
            extra_symbols: HashSet::new(),
            explicit_targets: HashSet::new(),
            discovery_source: ScenarioDiscoverySource::Source,
        });
    }
    if !cases.is_empty() {
        return cases;
    }

    let parameter_expansions = extract_rstest_parameter_expansions(function_node, source_bytes);
    if !parameter_expansions.is_empty() {
        if parameter_expansions.iter().all(|expansion| expansion.statically_visible) {
            let combinations = cross_product_rstest_values(&parameter_expansions);
            if !combinations.is_empty() {
                return combinations
                    .into_iter()
                    .map(|labels| RustScenarioSeed {
                        name: format!("{function_name}[{}]", labels.join(", ")),
                        start_line: function_node.start_position().row as i64 + 1,
                        end_line: function_end_line,
                        extra_symbols: HashSet::new(),
                        explicit_targets: HashSet::new(),
                        discovery_source: ScenarioDiscoverySource::Source,
                    })
                    .collect();
            }
        }

        return vec![RustScenarioSeed {
            name: function_name.to_string(),
            start_line: function_node.start_position().row as i64 + 1,
            end_line: function_end_line,
            extra_symbols: HashSet::new(),
            explicit_targets: HashSet::new(),
            discovery_source: ScenarioDiscoverySource::Source,
        }];
    }

    if has_rstest {
        return vec![RustScenarioSeed {
            name: function_name.to_string(),
            start_line: function_node.start_position().row as i64 + 1,
            end_line: function_end_line,
            extra_symbols: HashSet::new(),
            explicit_targets: HashSet::new(),
            discovery_source: ScenarioDiscoverySource::Source,
        }];
    }

    Vec::new()
}

#[derive(Debug, Clone)]
struct RstestParameterExpansion {
    name: String,
    values: Vec<String>,
    statically_visible: bool,
}

fn extract_rstest_parameter_expansions(
    function_node: Node<'_>,
    source: &[u8],
) -> Vec<RstestParameterExpansion> {
    let Some(parameters_node) = function_node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let Ok(parameters_raw) = parameters_node.utf8_text(source) else {
        return Vec::new();
    };
    let parameters_raw = parameters_raw
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    let mut expansions = Vec::new();

    for parameter in split_top_level_arguments(parameters_raw) {
        let attributes = extract_leading_rust_attributes(parameter);
        if attributes.is_empty() {
            continue;
        }
        let Some(name) = extract_rust_parameter_name(parameter) else {
            continue;
        };

        for attribute in attributes {
            match rust_attribute_name(attribute).as_deref() {
                Some("values") => expansions.push(RstestParameterExpansion {
                    name: name.clone(),
                    values: split_top_level_arguments(
                        extract_rust_attribute_args(attribute).unwrap_or_default(),
                    )
                    .into_iter()
                    .map(display_rstest_argument)
                    .collect(),
                    statically_visible: true,
                }),
                Some("files") => {
                    let values: Vec<String> = split_top_level_arguments(
                        extract_rust_attribute_args(attribute).unwrap_or_default(),
                    )
                    .into_iter()
                    .map(display_rstest_argument)
                    .collect();
                    let statically_visible =
                        values.iter().all(|value| !value.contains('*') && !value.contains('?'));
                    expansions.push(RstestParameterExpansion {
                        name: name.clone(),
                        values,
                        statically_visible,
                    });
                }
                _ => {}
            }
        }
    }

    expansions
}

fn cross_product_rstest_values(expansions: &[RstestParameterExpansion]) -> Vec<Vec<String>> {
    let mut rows = vec![Vec::new()];
    for expansion in expansions {
        let mut next = Vec::new();
        for row in &rows {
            for value in &expansion.values {
                let mut extended = row.clone();
                extended.push(format!("{}={}", expansion.name, value));
                next.push(extended);
            }
        }
        rows = next;
    }
    rows
}

fn collect_rust_proptest_scenarios(
    root: Node<'_>,
    source: &str,
    relative_path: &str,
) -> Vec<RustDiscoveredScenario> {
    let mut scenarios = Vec::new();
    let mut stack = vec![root];
    let source_bytes = source.as_bytes();

    while let Some(node) = stack.pop() {
        if node.kind() == "macro_invocation"
            && let Ok(raw_invocation) = node.utf8_text(source_bytes)
            && let Some(macro_name) = extract_rust_macro_invocation_name(raw_invocation)
            && macro_name == "proptest"
            && let Some(body) = extract_rust_macro_invocation_body(raw_invocation)
        {
            let (suite_name, suite_start_line, suite_end_line) =
                rust_suite_for_node(node, source_bytes, relative_path);
            let invocation_start_line = node.start_position().row as i64 + 1;
            for proptest_case in extract_proptest_cases(body) {
                scenarios.push(RustDiscoveredScenario {
                    suite_name: suite_name.clone(),
                    suite_start_line,
                    suite_end_line,
                    scenario: ParsedScenario {
                        name: proptest_case.name,
                        start_line: invocation_start_line + proptest_case.start_line_offset,
                        end_line: invocation_start_line + proptest_case.end_line_offset,
                        called_symbols: extract_callable_symbols_from_rust_text(
                            &proptest_case.body,
                        ),
                        explicit_targets: HashSet::new(),
                        discovery_source: ScenarioDiscoverySource::Source,
                    },
                });
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    scenarios
}

fn collect_rust_doctest_scenarios(
    root: Node<'_>,
    source: &str,
    relative_path: &str,
) -> Vec<RustDiscoveredScenario> {
    let mut scenarios = Vec::new();
    let mut stack = vec![root];
    let source_bytes = source.as_bytes();
    let lines: Vec<&str> = source.lines().collect();

    while let Some(node) = stack.pop() {
        if !is_rust_doctest_candidate(node.kind()) {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
            continue;
        }

        let Some(item_name) = node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source_bytes).ok())
            .map(str::to_string)
        else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
            continue;
        };

        let item_start_line = node.start_position().row as i64 + 1;
        let doc_lines = collect_preceding_doc_lines(&lines, item_start_line);
        if doc_lines.is_empty() {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
            continue;
        }

        let (suite_name, suite_start_line, suite_end_line) =
            rust_suite_for_node(node, source_bytes, relative_path);
        for block in extract_rust_doctest_blocks(&doc_lines) {
            let mut called_symbols = HashSet::new();
            called_symbols.insert(item_name.clone());

            let mut explicit_targets = HashSet::new();
            explicit_targets.insert(ExplicitProductionTarget {
                path: relative_path.to_string(),
                start_line: item_start_line,
            });

            scenarios.push(RustDiscoveredScenario {
                suite_name: format!("{suite_name}::doctests"),
                suite_start_line,
                suite_end_line,
                scenario: ParsedScenario {
                    name: format!("{item_name}[doctest:{}]", block.start_line),
                    start_line: block.start_line,
                    end_line: block.end_line,
                    called_symbols,
                    explicit_targets,
                    discovery_source: ScenarioDiscoverySource::Doctest,
                },
            });
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    scenarios
}

#[derive(Debug, Clone)]
struct RustDocLine {
    line_number: i64,
    text: String,
}

#[derive(Debug, Clone)]
struct RustDoctestBlock {
    start_line: i64,
    end_line: i64,
}

#[derive(Debug, Clone)]
struct ProptestCase {
    name: String,
    body: String,
    start_line_offset: i64,
    end_line_offset: i64,
}

fn rust_attribute_name(raw_attribute: &str) -> Option<String> {
    let compact: String = raw_attribute.chars().filter(|c| !c.is_whitespace()).collect();
    let stripped = compact.strip_prefix("#[")?.trim_end_matches(']');
    let name = stripped.split_once('(').map(|(name, _)| name).unwrap_or(stripped);
    name.rsplit("::")
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn extract_rust_attribute_args(raw_attribute: &str) -> Option<&str> {
    let compact: String = raw_attribute.chars().filter(|c| !c.is_whitespace()).collect();
    let open = compact.find('(')?;
    let close = compact.rfind(')')?;
    (close > open).then_some(&raw_attribute[raw_attribute.find('(')? + 1..raw_attribute.rfind(')')?])
}

fn extract_rust_apply_template_name(raw_attributes: &[String]) -> Option<String> {
    raw_attributes.iter().find_map(|attribute| {
        (rust_attribute_name(attribute).as_deref() == Some("apply")).then(|| {
            extract_rust_attribute_args(attribute)
                .map(split_top_level_arguments)
                .and_then(|parts| parts.into_iter().next())
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
        })?
    })
}

fn split_top_level_arguments(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0i32;
    let mut brace_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    for (idx, ch) in raw.char_indices() {
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            ',' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => {
                let part = raw[start..idx].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }

    let tail = raw[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn display_rstest_argument(value: &str) -> String {
    value.trim().to_string()
}

fn summarize_rstest_values(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| display_rstest_argument(value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn extract_leading_rust_attributes(parameter: &str) -> Vec<&str> {
    let mut attributes = Vec::new();
    let mut offset = 0usize;
    let bytes = parameter.as_bytes();

    while offset < parameter.len() {
        while offset < parameter.len() && bytes[offset].is_ascii_whitespace() {
            offset += 1;
        }
        if offset + 2 > parameter.len() || &parameter[offset..offset + 2] != "#[" {
            break;
        }

        let end = find_matching_delimiter(parameter, offset + 1)
            .map(|idx| idx + 1)
            .unwrap_or(parameter.len());
        attributes.push(parameter[offset..end].trim());
        offset = end;
    }

    attributes
}

fn extract_rust_parameter_name(parameter: &str) -> Option<String> {
    let without_attributes = {
        let mut offset = 0usize;
        let bytes = parameter.as_bytes();
        while offset < parameter.len() {
            while offset < parameter.len() && bytes[offset].is_ascii_whitespace() {
                offset += 1;
            }
            if offset + 2 > parameter.len() || &parameter[offset..offset + 2] != "#[" {
                break;
            }
            let end = find_matching_delimiter(parameter, offset + 1)? + 1;
            offset = end;
        }
        parameter[offset..].trim()
    };

    let name = without_attributes
        .split(':')
        .next()
        .unwrap_or(without_attributes)
        .trim()
        .trim_start_matches("mut ")
        .trim_start_matches('&')
        .trim();

    let name = name
        .split_whitespace()
        .last()
        .unwrap_or(name)
        .trim()
        .trim_matches('_');

    (!name.is_empty()).then_some(name.to_string())
}

fn extract_proptest_cases(body: &str) -> Vec<ProptestCase> {
    let mut cases = Vec::new();
    let mut search_start = 0usize;

    while let Some(relative_idx) = body[search_start..].find("fn ") {
        let fn_index = search_start + relative_idx;
        let after_fn = fn_index + 3;
        let name_tail = &body[after_fn..];
        let name_len = name_tail
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(name_tail.len());
        if name_len == 0 {
            search_start = after_fn;
            continue;
        }

        let name = name_tail[..name_len].to_string();
        let open_brace = match body[after_fn + name_len..].find('{') {
            Some(idx) => after_fn + name_len + idx,
            None => break,
        };
        let Some(close_brace) = find_matching_delimiter(body, open_brace) else {
            break;
        };
        let body_text = body[open_brace + 1..close_brace].trim().to_string();
        let start_line_offset = body[..fn_index].lines().count() as i64;
        let end_line_offset = body[..close_brace].lines().count() as i64;
        cases.push(ProptestCase {
            name,
            body: body_text,
            start_line_offset,
            end_line_offset,
        });
        search_start = close_brace + 1;
    }

    cases
}

fn is_rust_doctest_candidate(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "const_item"
            | "mod_item"
    )
}

fn collect_preceding_doc_lines(lines: &[&str], item_start_line: i64) -> Vec<RustDocLine> {
    let mut doc_lines = Vec::new();
    let mut index = item_start_line.saturating_sub(2) as isize;

    while index >= 0 {
        let line = lines[index as usize];
        let trimmed = line.trim_start();
        if let Some(content) = trimmed.strip_prefix("///") {
            doc_lines.push(RustDocLine {
                line_number: index as i64 + 1,
                text: content.trim_start().to_string(),
            });
            index -= 1;
            continue;
        }
        if let Some(content) = trimmed.strip_prefix("//!") {
            doc_lines.push(RustDocLine {
                line_number: index as i64 + 1,
                text: content.trim_start().to_string(),
            });
            index -= 1;
            continue;
        }
        if trimmed.starts_with("#[") || trimmed.is_empty() {
            index -= 1;
            continue;
        }
        break;
    }

    doc_lines.reverse();
    doc_lines
}

fn extract_rust_doctest_blocks(doc_lines: &[RustDocLine]) -> Vec<RustDoctestBlock> {
    let mut blocks = Vec::new();
    let mut active_start: Option<i64> = None;

    for line in doc_lines {
        let trimmed = line.text.trim();
        if active_start.is_none() {
            if let Some(info) = trimmed.strip_prefix("```")
                && rust_doc_fence_is_testable(info)
            {
                active_start = Some(line.line_number);
            }
            continue;
        }

        if trimmed.starts_with("```") {
            if let Some(start_line) = active_start.take() {
                blocks.push(RustDoctestBlock {
                    start_line,
                    end_line: line.line_number,
                });
            }
        }
    }

    blocks
}

fn rust_doc_fence_is_testable(info: &str) -> bool {
    let normalized = info.trim().to_ascii_lowercase();
    normalized.is_empty()
        || normalized.contains("rust")
        || normalized.contains("should_panic")
        || normalized.contains("no_run")
        || normalized.contains("compile_fail")
        || normalized.contains("edition")
        || normalized.contains("ignore")
}

fn rust_attribute_is_test(attribute_node: Node<'_>, source: &[u8]) -> bool {
    let Ok(raw) = attribute_node.utf8_text(source) else {
        return false;
    };

    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.starts_with("#[cfg(") {
        return false;
    }

    compact == "#[test]"
        || compact.starts_with("#[test(")
        || compact.contains("::test]")
        || compact.contains("::test(")
        || compact == "#[rstest]"
        || compact.starts_with("#[rstest(")
        || compact.contains("::rstest]")
        || compact.contains("::rstest(")
        || compact == "#[wasm_bindgen_test]"
        || compact.starts_with("#[wasm_bindgen_test(")
        || compact.contains("::wasm_bindgen_test]")
        || compact.contains("::wasm_bindgen_test(")
        || compact == "#[quickcheck]"
        || compact.starts_with("#[quickcheck(")
        || compact.contains("::quickcheck]")
        || compact.contains("::quickcheck(")
}

fn rust_attribute_is_parameterized_test(attribute_node: Node<'_>, source: &[u8]) -> bool {
    let Ok(raw) = attribute_node.utf8_text(source) else {
        return false;
    };

    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    compact.starts_with("#[test_case(")
        || compact.contains("::test_case(")
        || compact.starts_with("#[case(")
        || compact.contains("::case(")
}

fn rust_function_attributes(function_node: Node<'_>) -> Vec<Node<'_>> {
    let mut attributes = Vec::new();
    let mut seen_ranges = HashSet::new();

    let mut cursor = function_node.walk();
    for child in function_node.children(&mut cursor) {
        if child.kind() != "attribute_item" {
            continue;
        }

        let key = (child.start_byte(), child.end_byte());
        if seen_ranges.insert(key) {
            attributes.push(child);
        }
    }

    let mut sibling = function_node.prev_named_sibling();
    while let Some(node) = sibling {
        if node.kind() != "attribute_item" {
            break;
        }

        let key = (node.start_byte(), node.end_byte());
        if seen_ranges.insert(key) {
            attributes.push(node);
        }

        sibling = node.prev_named_sibling();
    }

    attributes.sort_by_key(Node::start_byte);
    attributes
}

fn build_rust_parameterized_test_case(
    function_name: &str,
    attribute_node: Node<'_>,
    source: &[u8],
    function_end_line: i64,
) -> RustScenarioSeed {
    let raw = attribute_node.utf8_text(source).unwrap_or_default();
    let rule_variant = extract_rule_variant_from_rust_test_case(raw);
    let fixture_path = extract_fixture_path_from_rust_test_case(raw);

    let mut name_parts = Vec::new();
    if let Some(rule_variant) = rule_variant.as_deref() {
        name_parts.push(rule_variant.to_string());
    }
    if let Some(fixture_path) = fixture_path.as_deref() {
        name_parts.push(fixture_path.to_string());
    }

    let name = if name_parts.is_empty() {
        function_name.to_string()
    } else {
        format!("{function_name}[{}]", name_parts.join(", "))
    };

    let mut extra_symbols = HashSet::new();
    if let Some(rule_variant) = rule_variant {
        extra_symbols.insert(rule_variant);
    }

    RustScenarioSeed {
        name,
        start_line: attribute_node.start_position().row as i64 + 1,
        end_line: function_end_line,
        extra_symbols,
        explicit_targets: HashSet::new(),
        discovery_source: ScenarioDiscoverySource::Source,
    }
}

fn rust_suite_for_function(
    node: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> (String, i64, i64) {
    rust_suite_for_node(node, source, relative_path)
}

fn rust_suite_for_node(
    node: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> (String, i64, i64) {
    let mut module_names = Vec::new();
    let mut suite_range: Option<(i64, i64)> = None;

    let mut parent = node.parent();
    while let Some(current) = parent {
        if current.kind() == "mod_item" {
            if let Some(name) = current
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(source).ok())
            {
                module_names.push(name.to_string());
            }

            suite_range.get_or_insert((
                current.start_position().row as i64 + 1,
                current.end_position().row as i64 + 1,
            ));
        }
        parent = current.parent();
    }

    if module_names.is_empty() {
        let fallback_name = Path::new(relative_path)
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("rust_tests")
            .to_string();
        (
            fallback_name,
            node.start_position().row as i64 + 1,
            node.end_position().row as i64 + 1,
        )
    } else {
        module_names.reverse();
        let (start_line, end_line) = suite_range.unwrap_or((
            node.start_position().row as i64 + 1,
            node.end_position().row as i64 + 1,
        ));
        (module_names.join("::"), start_line, end_line)
    }
}

fn collect_rust_called_symbols(scope: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut symbols = HashSet::new();
    let mut stack = vec![scope];

    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && let Some(function_node) = node.child_by_field_name("function")
            && let Some(symbol) = extract_rust_callable_symbol(function_node, source)
        {
            symbols.insert(symbol);
        }
        if node.kind() == "macro_invocation"
            && let Ok(raw_invocation) = node.utf8_text(source)
            && let Some(body) = extract_rust_macro_invocation_body(raw_invocation)
        {
            symbols.extend(extract_callable_symbols_from_rust_text(body));
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    symbols
}

fn collect_rust_import_paths_for(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> HashSet<String> {
    let mut paths = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "use_declaration"
            && let Ok(raw_use) = node.utf8_text(source)
        {
            for use_expr in expand_rust_use_statement(raw_use) {
                for path in rust_use_path_to_source_paths(&use_expr, relative_path) {
                    paths.insert(path);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    paths
}

fn rust_test_context_source_paths(relative_path: &str) -> HashSet<String> {
    let mut paths = HashSet::new();
    if !looks_like_production_source_path(relative_path) || !relative_path.ends_with(".rs") {
        return paths;
    }

    paths.insert(relative_path.to_string());

    let path = Path::new(relative_path);
    let Some(file_stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return paths;
    };
    if !file_stem.contains("test") || matches!(file_stem, "lib" | "main" | "mod") {
        return paths;
    }

    let Some(parent) = path.parent() else {
        return paths;
    };
    let parent_path = normalize_rel_path(parent);
    if parent_path == "src" || parent_path.ends_with("/src") {
        return paths;
    }

    paths.insert(format!("{parent_path}.rs"));
    paths.insert(format!("{parent_path}/mod.rs"));
    paths
}

fn collect_rust_scoped_call_import_paths_for(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> HashSet<String> {
    let mut paths = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && let Some(function_node) = node.child_by_field_name("function")
            && let Ok(raw_call) = function_node.utf8_text(source)
            && raw_call.contains("::")
        {
            for path in rust_use_path_to_source_paths(raw_call, relative_path) {
                paths.insert(path);
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    paths
}

fn expand_rust_use_statement(raw_use_statement: &str) -> Vec<String> {
    let mut statement = raw_use_statement.trim();

    if let Some(stripped) = statement.strip_prefix("pub ") {
        statement = stripped.trim_start();
    }
    if let Some(stripped) = statement.strip_prefix("use ") {
        statement = stripped;
    }
    statement = statement.trim().trim_end_matches(';').trim();

    expand_rust_use_expression(statement)
}

fn expand_rust_use_expression(expression: &str) -> Vec<String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Vec::new();
    }

    if let Some(open_idx) = find_top_level_char(expression, '{')
        && let Some(close_idx) = find_matching_brace(expression, open_idx)
    {
        let prefix = expression[..open_idx].trim().trim_end_matches("::");
        let inside = &expression[open_idx + 1..close_idx];
        let suffix = expression[close_idx + 1..].trim();
        let suffix = suffix.trim_start_matches("::");

        let mut expanded = Vec::new();
        for part in split_top_level_commas(inside) {
            for nested in expand_rust_use_expression(part) {
                let base = if nested == "self" {
                    prefix.to_string()
                } else if prefix.is_empty() {
                    nested
                } else if nested.is_empty() {
                    prefix.to_string()
                } else {
                    format!("{prefix}::{nested}")
                };

                if suffix.is_empty() {
                    if !base.is_empty() {
                        expanded.push(base);
                    }
                } else if !base.is_empty() {
                    expanded.push(format!("{base}::{suffix}"));
                }
            }
        }

        return expanded;
    }

    vec![expression.to_string()]
}

fn find_top_level_char(value: &str, target: char) -> Option<usize> {
    let mut brace_depth = 0i32;
    for (idx, ch) in value.char_indices() {
        match ch {
            '{' => {
                if ch == target && brace_depth == 0 {
                    return Some(idx);
                }
                brace_depth += 1;
            }
            '}' => {
                brace_depth -= 1;
            }
            _ if ch == target && brace_depth == 0 => return Some(idx),
            _ => {}
        }
    }
    None
}

fn find_matching_brace(value: &str, open_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (idx, ch) in value.char_indices().skip_while(|(idx, _)| *idx < open_idx) {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;

    for (idx, ch) in value.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                let part = value[start..idx].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }

    let tail = value[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }

    parts
}

fn rust_use_path_to_source_paths(raw_path: &str, test_relative_path: &str) -> HashSet<String> {
    let mut paths = HashSet::new();

    let path = raw_path
        .split(" as ")
        .next()
        .unwrap_or(raw_path)
        .trim()
        .trim_end_matches(';')
        .trim();
    if path.is_empty() {
        return paths;
    }

    let mut sanitized = path.trim_end_matches("::*").trim().to_string();
    if sanitized.ends_with("::self") {
        sanitized = sanitized.trim_end_matches("::self").to_string();
    }

    let segments: Vec<&str> = sanitized
        .split("::")
        .filter(|seg| !seg.is_empty())
        .collect();
    if segments.is_empty() {
        return paths;
    }

    add_rust_source_candidates(&mut paths, None, normalized_rust_use_segments(&segments));

    let current_crate_root = workspace_crate_root(test_relative_path);
    let current_crate_name = current_crate_root
        .as_deref()
        .and_then(|root| root.rsplit('/').next())
        .map(str::to_string);

    if let Some(crate_root) = current_crate_root.as_deref() {
        add_rust_source_candidates(
            &mut paths,
            Some(crate_root),
            normalized_rust_use_segments(&segments),
        );
    }

    if segments[0] != "crate"
        && segments[0] != "self"
        && segments[0] != "super"
        && segments.len() > 1
    {
        add_rust_source_candidates(&mut paths, None, normalized_rust_use_segments(&segments[1..]));
    }

    if let Some(crate_root) = current_crate_root.as_deref()
        && current_crate_name.as_deref() == Some(segments[0])
        && segments.len() > 1
    {
        add_rust_source_candidates(&mut paths, Some(crate_root), &segments[1..]);
    }

    if segments[0] != "crate"
        && segments[0] != "self"
        && segments[0] != "super"
        && segments.len() > 1
    {
        let crate_root = format!("crates/{}", segments[0]);
        add_rust_source_candidates(&mut paths, Some(&crate_root), &segments[1..]);
    }

    paths
}

fn normalized_rust_use_segments<'a>(segments: &'a [&'a str]) -> &'a [&'a str] {
    if segments.is_empty() {
        return segments;
    }
    if segments[0] == "crate" || segments[0] == "self" || segments[0] == "super" {
        &segments[1..]
    } else {
        segments
    }
}

fn add_rust_source_candidates(
    paths: &mut HashSet<String>,
    prefix: Option<&str>,
    segments: &[&str],
) {
    if segments.is_empty() {
        return;
    }

    for end in 1..=segments.len() {
        let module = segments[..end].join("/");
        let file_path = format!("src/{module}.rs");
        let mod_path = format!("src/{module}/mod.rs");
        if let Some(prefix) = prefix {
            paths.insert(format!("{prefix}/{file_path}"));
            paths.insert(format!("{prefix}/{mod_path}"));
        } else {
            paths.insert(file_path);
            paths.insert(mod_path);
        }
    }
}

fn workspace_crate_root(relative_path: &str) -> Option<String> {
    let mut segments = relative_path.split('/');
    let first = segments.next()?;
    let second = segments.next()?;
    let third = segments.next()?;

    (first == "crates" && (third == "src" || third == "tests")).then(|| format!("{first}/{second}"))
}

fn looks_like_production_source_path(path: &str) -> bool {
    path.starts_with("src/") || path.contains("/src/")
}

fn extract_rust_callable_symbol(function_node: Node<'_>, source: &[u8]) -> Option<String> {
    let raw = function_node.utf8_text(source).ok()?.trim();
    if raw.is_empty() {
        return None;
    }

    let symbol = match function_node.kind() {
        "identifier" => raw.to_string(),
        "field_expression" => function_node
            .child_by_field_name("field")
            .and_then(|field| field.utf8_text(source).ok())
            .map(str::to_string)
            .or_else(|| raw.rsplit('.').next().map(str::to_string))
            .unwrap_or_else(|| raw.to_string()),
        "scoped_identifier" | "scoped_type_identifier" => {
            raw.rsplit("::").next().unwrap_or(raw).to_string()
        }
        _ => raw
            .rsplit("::")
            .next()
            .unwrap_or(raw)
            .rsplit('.')
            .next()
            .unwrap_or(raw)
            .to_string(),
    };

    if symbol.is_empty() {
        None
    } else {
        Some(symbol)
    }
}

fn build_production_index(production: &[ProductionArtefact]) -> ProductionIndex {
    let mut index = ProductionIndex::default();

    for (position, artefact) in production.iter().enumerate() {
        index
            .by_simple_symbol
            .entry(symbol_match_key(&artefact.symbol_fqn))
            .or_default()
            .push(position);
        index
            .by_explicit_target
            .insert((artefact.path.clone(), artefact.start_line), position);
    }

    index
}

fn matched_production_artefacts<'a>(
    production: &'a [ProductionArtefact],
    production_index: &ProductionIndex,
    imported_paths: &HashSet<String>,
    called_symbols: &HashSet<String>,
    explicit_targets: &HashSet<ExplicitProductionTarget>,
) -> Vec<&'a ProductionArtefact> {
    let mut matched_indexes = HashSet::new();
    matched_indexes.extend(match_called_production_artefacts(
        production,
        production_index,
        imported_paths,
        called_symbols,
    ));
    matched_indexes.extend(match_explicit_production_targets(
        production_index,
        explicit_targets,
    ));

    let mut matched_indexes: Vec<usize> = matched_indexes.into_iter().collect();
    matched_indexes.sort_unstable();
    matched_indexes
        .into_iter()
        .map(|index| &production[index])
        .collect()
}

fn match_called_production_artefacts(
    production: &[ProductionArtefact],
    production_index: &ProductionIndex,
    imported_paths: &HashSet<String>,
    called_symbols: &HashSet<String>,
) -> HashSet<usize> {
    let mut matches = HashSet::new();

    for symbol in called_symbols {
        let normalized_called = symbol_match_key(symbol);
        if normalized_called.is_empty() {
            continue;
        }

        let Some(candidate_indexes) = production_index.by_simple_symbol.get(&normalized_called)
        else {
            continue;
        };

        for index in candidate_indexes {
            let artefact = &production[*index];
            if !imported_paths.is_empty()
                && !import_path_set_matches_production_path(imported_paths, &artefact.path)
            {
                continue;
            }
            matches.insert(*index);
        }
    }

    matches
}

fn match_explicit_production_targets(
    production_index: &ProductionIndex,
    explicit_targets: &HashSet<ExplicitProductionTarget>,
) -> HashSet<usize> {
    explicit_targets
        .iter()
        .filter_map(|target| {
            production_index
                .by_explicit_target
                .get(&(target.path.clone(), target.start_line))
                .copied()
        })
        .collect()
}

fn import_path_set_matches_production_path(
    imported_paths: &HashSet<String>,
    production_path: &str,
) -> bool {
    imported_paths
        .iter()
        .any(|imported_path| imported_path_matches_production_path(imported_path, production_path))
}

fn imported_path_matches_production_path(imported_path: &str, production_path: &str) -> bool {
    if imported_path == production_path {
        return true;
    }

    let Some(module_prefix) = imported_module_prefix(imported_path) else {
        return false;
    };

    production_path.starts_with(module_prefix)
        && production_path
            .as_bytes()
            .get(module_prefix.len())
            .is_some_and(|byte| *byte == b'/')
}

fn imported_module_prefix(imported_path: &str) -> Option<&str> {
    imported_path
        .strip_suffix("/mod.rs")
        .or_else(|| imported_path.strip_suffix(".rs"))
}

fn source_scenario_match_keys(
    relative_path: &str,
    suite_name: &str,
    scenario_name: &str,
) -> HashSet<String> {
    let base_name = scenario_base_name(scenario_name);
    let module_segments = rust_module_path_from_relative_path(relative_path);
    let mut keys = HashSet::new();

    let mut variants = Vec::new();
    if !suite_name.is_empty() {
        variants.push(format!("{suite_name}::{base_name}"));
    }
    if !module_segments.is_empty() {
        variants.push(format!("{}::{base_name}", module_segments.join("::")));
        if !suite_name.is_empty() {
            variants.push(format!(
                "{}::{}::{base_name}",
                module_segments.join("::"),
                suite_name
            ));
        }
        for start in 1..module_segments.len() {
            let suffix = module_segments[start..].join("::");
            variants.push(format!("{suffix}::{base_name}"));
            if !suite_name.is_empty() {
                variants.push(format!("{suffix}::{suite_name}::{base_name}"));
            }
        }
    }
    variants.push(base_name.to_string());

    for variant in variants {
        keys.insert(normalized_enumerated_test_key(&variant));
    }
    keys
}

fn doctest_match_keys(
    relative_path: &str,
    scenario_name: &str,
    explicit_targets: &HashSet<ExplicitProductionTarget>,
) -> HashSet<String> {
    let mut keys = HashSet::new();
    let item_name = scenario_base_name(scenario_name);
    for target in explicit_targets {
        keys.insert(normalized_enumerated_doctest_key(
            &target.path,
            &item_name,
            target.start_line,
        ));
    }
    if keys.is_empty() {
        keys.insert(normalized_enumerated_doctest_key(relative_path, &item_name, 0));
    }
    keys
}

fn scenario_base_name(name: &str) -> String {
    name.split('[').next().unwrap_or(name).trim().to_string()
}

fn rust_module_path_from_relative_path(relative_path: &str) -> Vec<String> {
    let path = relative_path.trim_end_matches(".rs");
    let path = path.trim_end_matches("/mod");
    let segments: Vec<&str> = path.split('/').collect();
    let Some(src_index) = segments.iter().position(|segment| *segment == "src") else {
        return Vec::new();
    };
    segments[src_index + 1..]
        .iter()
        .map(|segment| segment.to_string())
        .collect()
}

fn normalized_enumerated_test_key(name: &str) -> String {
    name.split(" - ")
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_lowercase()
}

fn normalized_enumerated_doctest_key(path: &str, item_name: &str, start_line: i64) -> String {
    format!("{}|{}|{}", path, item_name.to_ascii_lowercase(), start_line)
}

fn scenario_id_suffix(name: &str) -> String {
    let normalized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let collapsed = normalized
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if collapsed.is_empty() {
        "scenario".to_string()
    } else {
        collapsed
    }
}

fn materialize_enumerated_rust_scenarios(
    repo_id: &str,
    commit_sha: &str,
    production: &[ProductionArtefact],
    production_index: &ProductionIndex,
    enumeration: &RustEnumerationResult,
    source_scenario_keys: &HashSet<String>,
    source_doctest_keys: &HashSet<String>,
    synthetic_suites: &mut HashMap<String, String>,
    artefacts: &mut Vec<ArtefactRecord>,
    links: &mut Vec<TestLinkRecord>,
    link_keys: &mut HashSet<String>,
    stats: &mut IngestTestsStats,
) {
    for enumerated in &enumeration.scenarios {
        let normalized_key = if enumerated.discovery_source == ScenarioDiscoverySource::Doctest {
            enumerated
                .explicit_targets
                .iter()
                .next()
                .map(|target| {
                    normalized_enumerated_doctest_key(
                        &target.path,
                        &enumerated.scenario_name,
                        target.start_line,
                    )
                })
                .unwrap_or_else(|| {
                    normalized_enumerated_doctest_key(
                        &enumerated.relative_path,
                        &enumerated.scenario_name,
                        0,
                    )
                })
        } else {
            normalized_enumerated_test_key(&format!(
                "{}::{}",
                enumerated.suite_name, enumerated.scenario_name
            ))
        };

        let already_discovered = if enumerated.discovery_source == ScenarioDiscoverySource::Doctest {
            source_doctest_keys.contains(&normalized_key)
        } else {
            source_scenario_keys.contains(&normalized_key)
        };
        if already_discovered {
            continue;
        }

        let suite_key = format!("{}::{}", enumerated.relative_path, enumerated.suite_name);
        let suite_id = synthetic_suites.entry(suite_key.clone()).or_insert_with(|| {
            stats.suites += 1;
            let suite_id = format!(
                "test_suite:{commit_sha}:{}:{}",
                enumerated.relative_path,
                scenario_id_suffix(&enumerated.suite_name),
            );
            artefacts.push(build_artefact_record(
                &suite_id,
                repo_id,
                commit_sha,
                &enumerated.relative_path,
                "rust",
                "test_suite",
                Some("enumerated_suite"),
                Some(&enumerated.suite_name),
                None,
                1,
                1,
                Some(&enumerated.suite_name),
            ));
            suite_id
        });

        let scenario_id = format!(
            "test_case:{commit_sha}:{}:{}:{}",
            enumerated.relative_path,
            enumerated.start_line,
            scenario_id_suffix(&enumerated.scenario_name),
        );
        let scenario_fqn = format!("{}.{}", enumerated.suite_name, enumerated.scenario_name);
        artefacts.push(build_artefact_record(
            &scenario_id,
            repo_id,
            commit_sha,
            &enumerated.relative_path,
            "rust",
            "test_scenario",
            Some("enumerated_test"),
            Some(&scenario_fqn),
            Some(suite_id),
            enumerated.start_line,
            enumerated.start_line.max(1),
            Some(&enumerated.scenario_name),
        ));
        stats.scenarios += 1;
        stats.enumerated_scenarios += 1;

        let imported_paths = if enumerated.relative_path.starts_with("__synthetic_tests__/") {
            HashSet::new()
        } else {
            HashSet::from([enumerated.relative_path.clone()])
        };
        let called_symbols = HashSet::from([enumerated.scenario_name.clone()]);

        for production_artefact in matched_production_artefacts(
            production,
            production_index,
            &imported_paths,
            &called_symbols,
            &enumerated.explicit_targets,
        ) {
            let link_key = format!("{}::{}", scenario_id, production_artefact.artefact_id);
            if !link_keys.insert(link_key) {
                continue;
            }
            let link_id = format!(
                "link:{commit_sha}:{}:{}",
                scenario_id, production_artefact.artefact_id
            );
            links.push(build_test_link_record(
                &link_id,
                &scenario_id,
                &production_artefact.artefact_id,
                commit_sha,
            ));
            stats.links += 1;
        }
    }
}

fn build_artefact_record(
    artefact_id: &str,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    language: &str,
    canonical_kind: &str,
    language_kind: Option<&str>,
    symbol_fqn: Option<&str>,
    parent_artefact_id: Option<&str>,
    start_line: i64,
    end_line: i64,
    signature: Option<&str>,
) -> ArtefactRecord {
    ArtefactRecord {
        artefact_id: artefact_id.to_string(),
        repo_id: repo_id.to_string(),
        commit_sha: commit_sha.to_string(),
        path: path.to_string(),
        language: language.to_string(),
        canonical_kind: canonical_kind.to_string(),
        language_kind: language_kind.map(str::to_string),
        symbol_fqn: symbol_fqn.map(str::to_string),
        parent_artefact_id: parent_artefact_id.map(str::to_string),
        start_line,
        end_line,
        signature: signature.map(str::to_string),
    }
}

fn build_test_link_record(
    test_link_id: &str,
    test_artefact_id: &str,
    production_artefact_id: &str,
    commit_sha: &str,
) -> TestLinkRecord {
    TestLinkRecord {
        test_link_id: test_link_id.to_string(),
        test_artefact_id: test_artefact_id.to_string(),
        production_artefact_id: production_artefact_id.to_string(),
        commit_sha: commit_sha.to_string(),
    }
}

fn extract_import_specifier(import_statement: &str) -> Option<&str> {
    let quote = if import_statement.contains('"') {
        '"'
    } else if import_statement.contains('\'') {
        '\''
    } else {
        return None;
    };

    let first = import_statement.find(quote)?;
    let rest = &import_statement[first + 1..];
    let second = rest.find(quote)?;
    Some(&rest[..second])
}

fn resolve_import_to_repo_path(test_relative_path: &str, import_specifier: &str) -> Option<String> {
    if !import_specifier.starts_with('.') {
        return None;
    }

    let test_path = Path::new(test_relative_path);
    let base = test_path.parent()?;
    let combined = normalize_join(base, Path::new(import_specifier));
    let with_extension = if combined.extension().is_none() {
        combined.with_extension("ts")
    } else {
        combined
    };

    Some(normalize_rel_path(&with_extension))
}

fn normalize_join(base: &Path, relative: &Path) -> PathBuf {
    let joined = base.join(relative);
    let mut normalized = PathBuf::new();

    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }

    normalized
}

fn normalize_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_call_named(call_expression: Node<'_>, source: &[u8], name: &str) -> bool {
    call_expression
        .child_by_field_name("function")
        .and_then(|node| node.utf8_text(source).ok())
        .is_some_and(|value| value == name)
}

fn extract_first_string_argument(call_expression: Node<'_>, source: &[u8]) -> Option<String> {
    let args = call_expression.child_by_field_name("arguments")?;
    let arg = args.named_child(0)?;
    let raw = arg.utf8_text(source).ok()?;
    unquote(raw)
}

fn extract_second_callback_body(call_expression: Node<'_>) -> Option<Node<'_>> {
    let args = call_expression.child_by_field_name("arguments")?;
    let callback = args.named_child(1)?;
    callback.child_by_field_name("body")
}

fn unquote(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() < 2 {
        return None;
    }

    let starts = trimmed.as_bytes()[0] as char;
    let ends = trimmed.as_bytes()[trimmed.len() - 1] as char;
    if (starts == '\'' || starts == '"' || starts == '`') && starts == ends {
        return Some(trimmed[1..trimmed.len() - 1].to_string());
    }
    None
}

fn extract_rule_variant_from_rust_test_case(raw_attribute: &str) -> Option<String> {
    rust_scoped_tokens(raw_attribute).into_iter().find_map(|token| {
        token
            .strip_prefix("Rule::")
            .and_then(|value| value.rsplit("::").next())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn extract_fixture_path_from_rust_test_case(raw_attribute: &str) -> Option<String> {
    extract_string_literals(raw_attribute).into_iter().find(|literal| {
        literal.ends_with(".py") || literal.ends_with(".pyi") || literal.ends_with(".ipynb")
    })
}

fn extract_rust_macro_definition_name(raw_definition: &str) -> Option<&str> {
    let (_, remainder) = raw_definition.split_once("macro_rules!")?;
    let trimmed = remainder.trim_start();
    let end = trimmed
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .unwrap_or(trimmed.len());
    (end > 0).then_some(&trimmed[..end])
}

fn extract_rust_macro_invocation_name(raw_invocation: &str) -> Option<&str> {
    let bang_index = raw_invocation.find('!')?;
    let prefix = raw_invocation[..bang_index].trim();
    let name = prefix.rsplit("::").next().unwrap_or(prefix).trim();
    (!name.is_empty()).then_some(name)
}

fn extract_rust_macro_invocation_body(raw_invocation: &str) -> Option<&str> {
    let bang_index = raw_invocation.find('!')?;
    let remainder = &raw_invocation[bang_index + 1..];
    let open_relative = remainder.find(['(', '{', '['])?;
    let open_index = bang_index + 1 + open_relative;
    let close_index = find_matching_delimiter(raw_invocation, open_index)?;
    Some(raw_invocation[open_index + 1..close_index].trim())
}

fn find_matching_delimiter(raw: &str, open_index: usize) -> Option<usize> {
    let open_delimiter = raw[open_index..].chars().next()?;
    let close_delimiter = match open_delimiter {
        '(' => ')',
        '{' => '}',
        '[' => ']',
        _ => return None,
    };

    let mut depth = 0i32;
    for (idx, ch) in raw.char_indices().skip_while(|(idx, _)| *idx < open_index) {
        if ch == open_delimiter {
            depth += 1;
        } else if ch == close_delimiter {
            depth -= 1;
            if depth == 0 {
                return Some(idx);
            }
        }
    }

    None
}

fn extract_first_identifier_token(raw: &str) -> Option<String> {
    let mut chars = raw.char_indices().peekable();

    while let Some((start, ch)) = chars.next() {
        if !is_rust_identifier_start(ch) {
            continue;
        }

        let mut end = start + ch.len_utf8();
        while let Some((idx, next)) = chars.peek().copied() {
            if !is_rust_identifier_continue(next) {
                break;
            }
            end = idx + next.len_utf8();
            chars.next();
        }

        return Some(raw[start..end].to_string());
    }

    None
}

fn extract_callable_symbols_from_rust_text(raw: &str) -> HashSet<String> {
    let mut symbols = HashSet::new();
    let chars: Vec<char> = raw.chars().collect();

    for (idx, ch) in chars.iter().enumerate() {
        if *ch != '(' {
            continue;
        }

        let mut start = idx;
        while start > 0 {
            let previous = chars[start - 1];
            if previous.is_ascii_alphanumeric() || matches!(previous, '_' | ':' | '.') {
                start -= 1;
            } else {
                break;
            }
        }
        if start == idx {
            continue;
        }

        let token: String = chars[start..idx].iter().collect();
        let token = token.trim_matches(':').trim_matches('.');
        if token.is_empty() {
            continue;
        }

        let simple = token
            .rsplit("::")
            .next()
            .unwrap_or(token)
            .rsplit('.')
            .next()
            .unwrap_or(token);
        if simple.is_empty()
            || is_rust_non_callable_token(simple)
            || !simple.chars().next().is_some_and(is_rust_identifier_start)
            || !simple.chars().all(is_rust_identifier_continue)
        {
            continue;
        }

        symbols.insert(simple.to_string());
    }

    symbols
}

fn is_rust_identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_rust_identifier_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_rust_non_callable_token(token: &str) -> bool {
    matches!(
        token,
        "if"
            | "for"
            | "while"
            | "match"
            | "loop"
            | "return"
            | "type_property_test"
    )
}

fn rust_scoped_tokens(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' {
            current.push(ch);
            continue;
        }

        if current.contains("::") {
            tokens.push(current.clone());
        }
        current.clear();
    }

    if current.contains("::") {
        tokens.push(current);
    }

    tokens
}

fn extract_string_literals(raw: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '"' && ch != '\'' {
            continue;
        }

        let quote = ch;
        let mut literal = String::new();
        let mut escaped = false;

        for next in chars.by_ref() {
            if escaped {
                literal.push(next);
                escaped = false;
                continue;
            }

            if next == '\\' {
                escaped = true;
                continue;
            }

            if next == quote {
                literals.push(literal);
                break;
            }

            literal.push(next);
        }
    }

    literals
}

fn symbol_match_key(symbol: &str) -> String {
    let simple = symbol
        .rsplit("::")
        .next()
        .unwrap_or(symbol)
        .rsplit('.')
        .next()
        .unwrap_or(symbol);

    let mut normalized = String::new();
    let chars: Vec<char> = simple.chars().collect();

    for (idx, ch) in chars.iter().enumerate() {
        if !ch.is_ascii_alphanumeric() && *ch != '_' {
            continue;
        }

        if ch.is_ascii_uppercase() {
            let prev = idx.checked_sub(1).and_then(|i| chars.get(i)).copied();
            let next = chars.get(idx + 1).copied();
            let needs_separator = idx > 0
                && !normalized.ends_with('_')
                && prev.is_some_and(|prev| {
                    prev.is_ascii_lowercase()
                        || prev.is_ascii_digit()
                        || (prev.is_ascii_uppercase()
                            && next.is_some_and(|next| next.is_ascii_lowercase()))
                });
            if needs_separator {
                normalized.push('_');
            }
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(ch.to_ascii_lowercase());
        }
    }

    normalized.trim_matches('_').to_string()
}

fn enumerate_rust_tests(repo_dir: &Path) -> RustEnumerationResult {
    if !repo_dir.join("Cargo.toml").exists() {
        return RustEnumerationResult::default();
    }

    let host_output = run_cargo_test_list(repo_dir, false);
    let doc_output = run_cargo_test_list(repo_dir, true);

    let mut result = RustEnumerationResult::default();
    let mut full_success = true;

    match host_output {
        Ok(output) => {
            result
                .scenarios
                .extend(parse_enumerated_host_tests(&output));
        }
        Err(error) => {
            full_success = false;
            result.notes.push(format!(
                "host enumeration unavailable: {}",
                error.replace('\n', " ")
            ));
        }
    }

    match doc_output {
        Ok(output) => {
            result.scenarios.extend(parse_enumerated_doctests(&output));
        }
        Err(error) => {
            full_success = false;
            result.notes.push(format!(
                "doctest enumeration unavailable: {}",
                error.replace('\n', " ")
            ));
        }
    }

    result.mode = if result.notes.is_empty() && full_success {
        RustEnumerationMode::Full
    } else if !result.scenarios.is_empty() {
        RustEnumerationMode::Partial
    } else {
        RustEnumerationMode::Skipped
    };
    result
}

fn run_cargo_test_list(repo_dir: &Path, doctests: bool) -> Result<String, String> {
    let mut command = Command::new("cargo");
    command
        .current_dir(repo_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("test")
        .arg("--workspace");
    if doctests {
        command.arg("--doc");
    }
    command.arg("--").arg("--list");

    let mut child = command.spawn().map_err(|error| {
        format!(
            "failed to execute cargo test list in {}: {}",
            repo_dir.display(),
            error
        )
    })?;
    let timeout = Duration::from_secs(30);
    let deadline = Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|error| format!("failed waiting for cargo test list: {error}"))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}\n{stderr}");
                return if output.status.success() {
                    Ok(combined)
                } else {
                    Err(combined)
                };
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let output = child.wait_with_output().ok();
                let combined = output
                    .map(|output| {
                        format!(
                            "{}\n{}",
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        )
                    })
                    .unwrap_or_default();
                return Err(format!(
                    "timed out after {}s while listing {}tests{}",
                    timeout.as_secs(),
                    if doctests { "doc " } else { "" },
                    if combined.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", combined.replace('\n', " "))
                    }
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(error) => return Err(format!("failed polling cargo test list: {error}")),
        }
    }
}

fn parse_enumerated_host_tests(output: &str) -> Vec<EnumeratedRustScenario> {
    let mut scenarios = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.ends_with(": test") || trimmed.contains(" - ") {
            continue;
        }

        let name = trimmed.trim_end_matches(": test").trim();
        if name.is_empty()
            || name.starts_with("Doc-tests ")
            || name.starts_with("Running ")
            || name.ends_with(" benchmarks")
        {
            continue;
        }

        let segments: Vec<&str> = name.split("::").collect();
        let scenario_name = segments.last().copied().unwrap_or(name).to_string();
        let suite_name = if segments.len() > 1 {
            segments[..segments.len() - 1].join("::")
        } else {
            "enumerated".to_string()
        };
        let relative_path = "__synthetic_tests__/workspace.rs".to_string();

        scenarios.push(EnumeratedRustScenario {
            suite_name,
            scenario_name,
            relative_path,
            start_line: 1,
            explicit_targets: HashSet::new(),
            discovery_source: ScenarioDiscoverySource::Enumeration,
        });
    }

    scenarios
}

fn parse_enumerated_doctests(output: &str) -> Vec<EnumeratedRustScenario> {
    let mut scenarios = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.ends_with(": test") || !trimmed.contains(" - ") {
            continue;
        }
        let Some((path, remainder)) = trimmed.trim_end_matches(": test").split_once(" - ") else {
            continue;
        };
        let Some((item_name, line_number)) = parse_doctest_descriptor(remainder) else {
            continue;
        };

        let mut explicit_targets = HashSet::new();
        explicit_targets.insert(ExplicitProductionTarget {
            path: path.to_string(),
            start_line: line_number,
        });
        scenarios.push(EnumeratedRustScenario {
            suite_name: format!("{}::doctests", path.replace('/', "::")),
            scenario_name: item_name.clone(),
            relative_path: path.to_string(),
            start_line: line_number,
            explicit_targets,
            discovery_source: ScenarioDiscoverySource::Doctest,
        });
    }

    scenarios
}

fn parse_doctest_descriptor(raw: &str) -> Option<(String, i64)> {
    let (item_name, line_part) = raw.rsplit_once("(line ")?;
    let line_number = line_part.trim_end_matches(')').parse().ok()?;
    Some((item_name.trim().to_string(), line_number))
}

#[cfg(test)]
mod tests {
    use tree_sitter::Parser;
    use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;
    use tree_sitter_typescript::LANGUAGE_TYPESCRIPT;

    use super::{
        collect_rust_import_paths_for, collect_rust_scoped_call_import_paths_for,
        collect_rust_suites, collect_typescript_suites, extract_import_specifier,
        extract_rust_macro_invocation_body, imported_path_matches_production_path,
        parse_enumerated_doctests, resolve_import_to_repo_path,
        rust_source_contains_doctest_markers, rust_test_context_source_paths,
        symbol_match_key, unquote, ExplicitProductionTarget, ScenarioDiscoverySource,
    };

    #[test]
    fn extracts_import_specifier_from_statement() {
        let statement = r#"import { UserService } from "../src/services/UserService";"#;
        let value = extract_import_specifier(statement).expect("should extract import specifier");
        assert_eq!(value, "../src/services/UserService");
    }

    #[test]
    fn resolves_relative_import_to_repo_path() {
        let resolved = resolve_import_to_repo_path(
            "tests/e2e/userFlow.test.ts",
            "../../src/services/UserService",
        )
        .expect("should resolve relative import");

        assert_eq!(resolved, "src/services/UserService.ts");
    }

    #[test]
    fn unquote_handles_string_literals() {
        assert_eq!(unquote("'hello'"), Some("hello".to_string()));
        assert_eq!(unquote("\"hello\""), Some("hello".to_string()));
        assert_eq!(unquote("`hello`"), Some("hello".to_string()));
    }

    #[test]
    fn rust_suites_detects_test_and_tokio_test_functions() {
        let source = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn sample() {
        assert_eq!(2 + 2, 4);
    }

    #[tokio::test]
    async fn async_sample() {
        helper::run().await;
        client.execute();
    }

    fn helper_only() {
        helper::run();
    }
}
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let suites = collect_rust_suites(tree.root_node(), source, "tests/rust_unit.rs");

        assert_eq!(suites.len(), 1, "expected one rust suite");
        assert_eq!(suites[0].name, "tests");

        let scenario_names: Vec<&str> = suites[0]
            .scenarios
            .iter()
            .map(|scenario| scenario.name.as_str())
            .collect();
        assert_eq!(scenario_names, vec!["sample", "async_sample"]);

        let async_scenario = suites[0]
            .scenarios
            .iter()
            .find(|scenario| scenario.name == "async_sample")
            .expect("missing async_sample scenario");
        assert!(
            async_scenario.called_symbols.contains("run"),
            "expected rust call-site extraction to include helper::run"
        );
        assert!(
            async_scenario.called_symbols.contains("execute"),
            "expected rust call-site extraction to include method call symbols"
        );
    }

    #[test]
    fn rust_use_declarations_map_to_source_paths() {
        let source = r#"
use crate::repositories::user_repository::UserRepository;
use testlens_fixture_rust::services::{user_service::UserService, auth_service::AuthService};
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let import_paths =
            collect_rust_import_paths_for(tree.root_node(), source.as_bytes(), "tests/rust.rs");
        assert!(
            import_paths.contains("src/repositories/user_repository.rs"),
            "expected repository source path from use declaration"
        );
        assert!(
            import_paths.contains("src/services/user_service.rs"),
            "expected user service path from brace use declaration"
        );
        assert!(
            import_paths.contains("src/services/auth_service.rs"),
            "expected auth service path from brace use declaration"
        );
    }

    #[test]
    fn rust_scoped_call_paths_map_to_source_paths() {
        let source = r#"
#[test]
fn sample() {
    crate::services::auth_service::AuthService::hash_password("x");
}
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let import_paths = collect_rust_scoped_call_import_paths_for(
            tree.root_node(),
            source.as_bytes(),
            "tests/rust.rs",
        );
        assert!(
            import_paths.contains("src/services/auth_service.rs"),
            "expected scoped call path to resolve to source module path"
        );
    }

    #[test]
    fn rust_workspace_imports_map_to_crate_source_paths() {
        let source = r#"
use red_knot_workspace::db::RootDatabase;
use ruff::commands::version::version;

#[test]
fn sample() {
    let _ = red_knot_workspace::db::RootDatabase::new();
    ruff::commands::version::version();
}
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let import_paths = collect_rust_import_paths_for(
            tree.root_node(),
            source.as_bytes(),
            "crates/ruff/tests/version.rs",
        );
        assert!(
            import_paths.contains("crates/red_knot_workspace/src/db.rs"),
            "expected workspace crate import to resolve to crate source file"
        );
        assert!(
            import_paths.contains("crates/ruff/src/commands/version.rs"),
            "expected same-workspace crate import to resolve to local crate source file"
        );

        let scoped_call_paths = collect_rust_scoped_call_import_paths_for(
            tree.root_node(),
            source.as_bytes(),
            "crates/ruff/tests/version.rs",
        );
        assert!(
            scoped_call_paths.contains("crates/red_knot_workspace/src/db.rs"),
            "expected scoped call to workspace crate type to resolve to crate source file"
        );
        assert!(
            scoped_call_paths.contains("crates/ruff/src/commands/version.rs"),
            "expected scoped call to local workspace crate function to resolve to crate source file"
        );
    }

    #[test]
    fn rust_suites_expand_test_case_attributes_into_parameterized_scenarios() {
        let source = r#"
#[cfg(test)]
mod tests {
    use std::path::Path;
    use test_case::test_case;

    #[test_case(Rule::StringDotFormatExtraPositionalArguments, Path::new("F523.py"))]
    #[test_case(Rule::StringDotFormatExtraNamedArguments, Path::new("F522.py"))]
    fn rules(rule_code: Rule, path: &Path) {
        test_path(path, rule_code);
    }
}
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let suites =
            collect_rust_suites(tree.root_node(), source, "src/rules/pyflakes/mod.rs");
        assert_eq!(suites.len(), 1, "expected one inline rust test suite");

        let scenario_names: Vec<&str> = suites[0]
            .scenarios
            .iter()
            .map(|scenario| scenario.name.as_str())
            .collect();
        assert_eq!(
            scenario_names,
            vec![
                "rules[StringDotFormatExtraPositionalArguments, F523.py]",
                "rules[StringDotFormatExtraNamedArguments, F522.py]",
            ]
        );
        assert!(
            suites[0].scenarios[0]
                .called_symbols
                .contains("StringDotFormatExtraPositionalArguments"),
            "expected parameterized scenario to carry its rule variant symbol"
        );
    }

    #[test]
    fn rust_suites_detect_wasm_bindgen_test_functions() {
        let source = r#"
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn empty_config() {
    render_message();
}
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let suites = collect_rust_suites(tree.root_node(), source, "tests/api.rs");
        assert_eq!(suites.len(), 1, "expected one wasm suite");
        assert_eq!(suites[0].name, "api");
        assert_eq!(suites[0].scenarios.len(), 1);
        assert_eq!(suites[0].scenarios[0].name, "empty_config");
        assert!(
            suites[0].scenarios[0].called_symbols.contains("render_message"),
            "expected wasm test call-site extraction to include render_message"
        );
    }

    #[test]
    fn rust_suites_expand_macro_generated_quickcheck_tests() {
        let source = r#"
macro_rules! type_property_test {
    ($test_name:ident, $property:expr) => {
        #[quickcheck_macros::quickcheck]
        #[ignore]
        fn $test_name(t: Type) -> bool {
            $property
        }
    };
}

mod stable {
    type_property_test!(equivalent_to_is_reflexive, t.is_equivalent_to());
    type_property_test!(subtype_of_is_reflexive, t.is_subtype_of());
}
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let suites =
            collect_rust_suites(tree.root_node(), source, "src/types/property_tests.rs");
        assert_eq!(suites.len(), 1, "expected one property-test suite");
        assert_eq!(suites[0].name, "stable");

        let scenario_names: Vec<&str> = suites[0]
            .scenarios
            .iter()
            .map(|scenario| scenario.name.as_str())
            .collect();
        assert_eq!(
            scenario_names,
            vec!["equivalent_to_is_reflexive", "subtype_of_is_reflexive"]
        );
        assert!(
            suites[0].scenarios[0]
                .called_symbols
                .contains("is_equivalent_to"),
            "expected quickcheck macro invocation to surface method-call symbols"
        );
    }

    #[test]
    fn rust_suites_expand_rstest_cases_values_and_templates() {
        let source = r#"
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use rstest::{rstest, template, apply};

    #[rstest]
    #[case(2, 4)]
    #[case(3, 6)]
    fn doubles_case_values(#[case] input: u32, #[case] expected: u32) {
        assert_eq!(double(input), expected);
    }

    #[rstest]
    fn doubles_values(#[values(1, 2)] input: u32) {
        assert!(double(input) > 0);
    }

    #[template]
    #[rstest]
    #[case(2, 6)]
    #[case(3, 9)]
    fn triple_cases(#[case] input: u32, #[case] expected: u32) {}

    #[apply(triple_cases)]
    fn triples_from_template(input: u32, expected: u32) {
        assert_eq!(triple(input), expected);
    }

    #[rstest]
    fn files_fallback(#[files("fixtures/*.txt")] path: PathBuf) {
        let _ = path;
    }
}
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let suites = collect_rust_suites(tree.root_node(), source, "src/lib.rs");
        let scenario_names: Vec<&str> = suites[0]
            .scenarios
            .iter()
            .map(|scenario| scenario.name.as_str())
            .collect();

        assert_eq!(
            scenario_names,
            vec![
                "doubles_case_values[2, 4]",
                "doubles_case_values[3, 6]",
                "doubles_values[input=1]",
                "doubles_values[input=2]",
                "triples_from_template[2, 6]",
                "triples_from_template[3, 9]",
                "files_fallback",
            ]
        );
    }

    #[test]
    fn rust_suites_extract_proptest_cases_from_macro_body() {
        let source = r#"
#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn double_is_even(input in 0u32..8) {
            let result = double(input);
            prop_assert_eq!(result % 2, 0);
        }
    }
}
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let suites = collect_rust_suites(tree.root_node(), source, "src/property_tests.rs");
        assert_eq!(suites.len(), 1);
        assert_eq!(suites[0].name, "property_tests");
        assert_eq!(suites[0].scenarios.len(), 1);
        assert_eq!(suites[0].scenarios[0].name, "double_is_even");
        assert!(
            suites[0].scenarios[0].called_symbols.contains("double"),
            "expected proptest case body to surface double()"
        );
    }

    #[test]
    fn rust_suites_materialize_doctest_scenarios() {
        let source = r#"
/// ```rust
/// assert_eq!(documented_increment(1), 2);
/// ```
pub fn documented_increment(value: u32) -> u32 {
    value + 1
}
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .expect("failed setting rust parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing rust source");

        let suites = collect_rust_suites(tree.root_node(), source, "src/docs.rs");
        assert_eq!(suites.len(), 1);
        assert_eq!(suites[0].name, "docs::doctests");
        assert_eq!(suites[0].scenarios.len(), 1);
        assert_eq!(suites[0].scenarios[0].discovery_source, ScenarioDiscoverySource::Doctest);
        assert!(
            suites[0].scenarios[0]
                .explicit_targets
                .contains(&ExplicitProductionTarget {
                    path: "src/docs.rs".to_string(),
                    start_line: 5,
                }),
            "expected doctest to point at the documented item"
        );
    }

    #[test]
    fn parses_enumerated_doctest_output() {
        let scenarios = parse_enumerated_doctests(
            "crates/sample/src/lib.rs - sample::documented_increment (line 12): test",
        );

        assert_eq!(scenarios.len(), 1);
        assert_eq!(scenarios[0].relative_path, "crates/sample/src/lib.rs");
        assert_eq!(scenarios[0].scenario_name, "sample::documented_increment");
        assert!(
            scenarios[0]
                .explicit_targets
                .contains(&ExplicitProductionTarget {
                    path: "crates/sample/src/lib.rs".to_string(),
                    start_line: 12,
                }),
            "expected parsed doctest line target"
        );
    }

    #[test]
    fn doctest_prefilter_requires_doc_fences_not_plain_doc_comments() {
        assert!(rust_source_contains_doctest_markers(
            r#"
/// ```rust
/// assert_eq!(value(), 1);
/// ```
pub fn value() -> u32 {
    1
}
"#
        ));
        assert!(!rust_source_contains_doctest_markers(
            r#"
/** Plain docs without a fenced block. */
pub fn documented() {}
"#
        ));
    }

    #[test]
    fn rust_test_context_paths_include_parent_module_for_property_test_files() {
        let context_paths = rust_test_context_source_paths(
            "crates/red_knot_python_semantic/src/types/property_tests.rs",
        );

        assert!(
            context_paths.contains("crates/red_knot_python_semantic/src/types.rs"),
            "expected property-tests file to include parent module source path"
        );
    }

    #[test]
    fn extracts_rust_macro_invocation_body() {
        let raw = r#"type_property_test!(equivalent_to_is_reflexive, t.is_equivalent_to(db, t));"#;
        let body =
            extract_rust_macro_invocation_body(raw).expect("expected macro invocation body");
        assert_eq!(body, "equivalent_to_is_reflexive, t.is_equivalent_to(db, t)");
    }

    #[test]
    fn rust_module_root_import_matches_nested_rule_paths() {
        assert!(
            imported_path_matches_production_path(
                "src/rules/pyflakes/mod.rs",
                "src/rules/pyflakes/rules/strings.rs"
            ),
            "expected module-root import to match nested rule module"
        );
    }

    #[test]
    fn symbol_match_key_normalizes_camel_case_rule_variants() {
        assert_eq!(
            symbol_match_key("StringDotFormatExtraPositionalArguments"),
            "string_dot_format_extra_positional_arguments"
        );
        assert_eq!(
            symbol_match_key("Rule::StringDotFormatExtraPositionalArguments"),
            "string_dot_format_extra_positional_arguments"
        );
    }

    #[test]
    fn typescript_nested_describe_does_not_duplicate_inner_tests_into_outer_suite() {
        let source = r#"
describe("outer", () => {
  describe("inner", () => {
    it("inner test", () => {
      expect(1).toBe(1);
    });
  });
});
"#;

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_TYPESCRIPT.into())
            .expect("failed setting TypeScript parser language");

        let tree = parser
            .parse(source, None)
            .expect("failed parsing TypeScript source");

        let suites = collect_typescript_suites(tree.root_node(), source.as_bytes());
        assert_eq!(suites.len(), 2, "expected nested describe suites");

        let outer = suites
            .iter()
            .find(|suite| suite.name == "outer")
            .expect("missing outer suite");
        assert!(
            outer.scenarios.is_empty(),
            "outer suite should not duplicate inner suite scenarios"
        );

        let inner = suites
            .iter()
            .find(|suite| suite.name == "inner")
            .expect("missing inner suite");
        assert_eq!(
            inner.scenarios.len(),
            1,
            "expected exactly one inner scenario"
        );
        assert_eq!(inner.scenarios[0].name, "inner test");
    }
}
