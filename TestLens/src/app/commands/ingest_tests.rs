// Command handler for discovering test suites/scenarios and establishing static
// links from test artefacts to referenced production artefacts.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

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
}

#[derive(Debug, Clone)]
struct ParsedTestFile {
    line_count: i64,
    import_paths: HashSet<String>,
    suites: Vec<ParsedSuite>,
}

#[derive(Debug, Clone)]
struct DiscoveredTestFile {
    relative_path: String,
    adapter_index: usize,
    priority: u8,
}

trait TestLanguageAdapter {
    fn language(&self) -> &'static str;
    fn priority(&self) -> u8;
    fn supports_path(&self, relative_path: &str) -> bool;
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

    fn supports_path(&self, relative_path: &str) -> bool {
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

    fn supports_path(&self, relative_path: &str) -> bool {
        relative_path.ends_with(".test.rs")
            || relative_path.ends_with(".spec.rs")
            || ((relative_path.starts_with("tests/") || relative_path.contains("/tests/"))
                && relative_path.ends_with(".rs"))
    }

    fn parse_file(&mut self, absolute_path: &Path, relative_path: &str) -> Result<ParsedTestFile> {
        let source = read_source_file(absolute_path)?;
        let tree = self
            .parser
            .parse(&source, None)
            .with_context(|| format!("failed parsing test file {}", absolute_path.display()))?;
        let bytes = source.as_bytes();
        let root = tree.root_node();

        let mut import_paths = collect_rust_import_paths(root, bytes);
        import_paths.extend(collect_rust_scoped_call_import_paths(root, bytes));
        let suites = collect_rust_suites(root, bytes, relative_path);

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
}

pub fn handle(db_path: &Path, repo_dir: &Path, commit_sha: &str) -> Result<()> {
    let mut repository = open_sqlite_repository(db_path)?;
    let repo_id = repository.load_repo_id_for_commit(commit_sha)?;
    let production = repository.load_production_artefacts(commit_sha)?;

    let mut adapters: Vec<Box<dyn TestLanguageAdapter>> = vec![
        Box::new(RustTestAdapter::new()?),
        Box::new(TypeScriptTestAdapter::new()?),
    ];

    let test_files = discover_test_files(repo_dir, &adapters)?;
    let mut stats = IngestTestsStats::default();
    let mut link_keys = HashSet::new();
    let mut artefacts = Vec::new();
    let mut links = Vec::new();

    for discovered in test_files {
        stats.files += 1;
        let absolute_path = repo_dir.join(&discovered.relative_path);

        let adapter = &mut adapters[discovered.adapter_index];
        let language = adapter.language().to_string();
        let parsed = adapter.parse_file(&absolute_path, &discovered.relative_path)?;

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
            .filter(|item| item.starts_with("src/"))
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
                let scenario_id = format!(
                    "test_case:{commit_sha}:{}:{}",
                    discovered.relative_path, scenario.start_line
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

                for production_artefact in match_called_production_artefacts(
                    &production,
                    &imported_paths,
                    &scenario.called_symbols,
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

    repository.replace_test_discovery(commit_sha, &artefacts, &links)?;
    println!(
        "ingest-tests complete for commit {} (files: {}, suites: {}, scenarios: {}, links: {})",
        commit_sha, stats.files, stats.suites, stats.scenarios, stats.links
    );
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
                    .supports_path(&relative_path)
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

fn collect_rust_suites(root: Node<'_>, source: &[u8], relative_path: &str) -> Vec<ParsedSuite> {
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
    source: &[u8],
    relative_path: &str,
) -> Vec<RustDiscoveredScenario> {
    let mut scenarios = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "function_item" && is_rust_test_function(node, source) {
            let Some(function_name) = node
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(source).ok())
                .map(str::to_string)
            else {
                continue;
            };

            let called_symbols = node
                .child_by_field_name("body")
                .map(|body| collect_rust_called_symbols(body, source))
                .unwrap_or_default();

            let (suite_name, suite_start_line, suite_end_line) =
                rust_suite_for_function(node, source, relative_path);

            scenarios.push(RustDiscoveredScenario {
                suite_name,
                suite_start_line,
                suite_end_line,
                scenario: ParsedScenario {
                    name: function_name,
                    start_line: node.start_position().row as i64 + 1,
                    end_line: node.end_position().row as i64 + 1,
                    called_symbols,
                },
            });
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    scenarios.sort_by(|a, b| {
        a.suite_start_line
            .cmp(&b.suite_start_line)
            .then(a.scenario.start_line.cmp(&b.scenario.start_line))
    });
    scenarios
}

fn is_rust_test_function(function_node: Node<'_>, source: &[u8]) -> bool {
    if function_node.kind() != "function_item" {
        return false;
    }

    let mut cursor = function_node.walk();
    for child in function_node.children(&mut cursor) {
        if child.kind() == "attribute_item" && rust_attribute_is_test(child, source) {
            return true;
        }
    }

    let mut sibling = function_node.prev_named_sibling();
    while let Some(node) = sibling {
        if node.kind() != "attribute_item" {
            break;
        }

        if rust_attribute_is_test(node, source) {
            return true;
        }

        sibling = node.prev_named_sibling();
    }

    false
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
}

fn rust_suite_for_function(
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

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    symbols
}

fn collect_rust_import_paths(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut paths = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "use_declaration"
            && let Ok(raw_use) = node.utf8_text(source)
        {
            for use_expr in expand_rust_use_statement(raw_use) {
                for path in rust_use_path_to_source_paths(&use_expr) {
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

fn collect_rust_scoped_call_import_paths(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut paths = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && let Some(function_node) = node.child_by_field_name("function")
            && let Ok(raw_call) = function_node.utf8_text(source)
            && raw_call.contains("::")
        {
            for path in rust_use_path_to_source_paths(raw_call) {
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

fn rust_use_path_to_source_paths(raw_path: &str) -> HashSet<String> {
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

    add_rust_source_candidates(&mut paths, normalized_rust_use_segments(&segments));
    if segments[0] != "crate"
        && segments[0] != "self"
        && segments[0] != "super"
        && segments.len() > 1
    {
        add_rust_source_candidates(&mut paths, normalized_rust_use_segments(&segments[1..]));
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

fn add_rust_source_candidates(paths: &mut HashSet<String>, segments: &[&str]) {
    if segments.is_empty() {
        return;
    }

    for end in 1..=segments.len() {
        let module = segments[..end].join("/");
        paths.insert(format!("src/{module}.rs"));
        paths.insert(format!("src/{module}/mod.rs"));
    }
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

fn match_called_production_artefacts<'a>(
    production: &'a [ProductionArtefact],
    imported_paths: &HashSet<String>,
    called_symbols: &HashSet<String>,
) -> Vec<&'a ProductionArtefact> {
    let mut matches = Vec::new();
    for artefact in production {
        if !imported_paths.is_empty() && !imported_paths.contains(&artefact.path) {
            continue;
        }

        let simple_name = simple_symbol_name(&artefact.symbol_fqn);
        if called_symbols.contains(&simple_name) {
            matches.push(artefact);
        }
    }
    matches
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

fn simple_symbol_name(symbol: &str) -> String {
    symbol.rsplit('.').next().unwrap_or(symbol).to_string()
}

#[cfg(test)]
mod tests {
    use tree_sitter::Parser;
    use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;
    use tree_sitter_typescript::LANGUAGE_TYPESCRIPT;

    use super::{
        collect_rust_import_paths, collect_rust_scoped_call_import_paths, collect_rust_suites,
        collect_typescript_suites, extract_import_specifier, resolve_import_to_repo_path, unquote,
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

        let suites = collect_rust_suites(tree.root_node(), source.as_bytes(), "tests/rust_unit.rs");

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

        let import_paths = collect_rust_import_paths(tree.root_node(), source.as_bytes());
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

        let import_paths =
            collect_rust_scoped_call_import_paths(tree.root_node(), source.as_bytes());
        assert!(
            import_paths.contains("src/services/auth_service.rs"),
            "expected scoped call path to resolve to source module path"
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
