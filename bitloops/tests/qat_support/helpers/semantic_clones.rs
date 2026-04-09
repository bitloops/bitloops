#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SemanticCloneStoreEvidence {
    current_artefacts: usize,
    embeddings: usize,
    clone_edges: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticCloneStoreSnapshot {
    path: std::path::PathBuf,
    evidence: SemanticCloneStoreEvidence,
}

fn semantic_clone_store_evidence_proves_rebuild(
    _clone_edges_metric: Option<u64>,
    evidence: SemanticCloneStoreEvidence,
) -> bool {
    evidence.current_artefacts > 0 && evidence.embeddings > 0 && evidence.clone_edges > 0
}

fn load_semantic_clone_store_snapshot(
    world: &QatWorld,
) -> Result<SemanticCloneStoreSnapshot> {
    let mut pending = vec![world.run_dir().to_path_buf()];
    let mut candidate_paths = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in
            fs::read_dir(&dir).with_context(|| format!("reading semantic clone dir {}", dir.display()))?
        {
            let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "relational.db")
            {
                candidate_paths.push(path);
            }
        }
    }

    let mut selected: Option<SemanticCloneStoreSnapshot> = None;
    for path in &candidate_paths {
        let Some(evidence) = load_semantic_clone_store_evidence_from_path(path)? else {
            continue;
        };
        let snapshot = SemanticCloneStoreSnapshot {
            path: path.clone(),
            evidence,
        };
        let replace = selected.as_ref().is_none_or(|current| {
            let next_score = (
                snapshot.evidence.clone_edges,
                snapshot.evidence.embeddings,
                snapshot.evidence.current_artefacts,
            );
            let current_score = (
                current.evidence.clone_edges,
                current.evidence.embeddings,
                current.evidence.current_artefacts,
            );
            next_score > current_score
        });
        if replace {
            selected = Some(snapshot);
        }
    }

    selected.ok_or_else(|| {
        anyhow!(
            "could not find a populated semantic clone relational db under {} (candidates: {})",
            world.run_dir().display(),
            candidate_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

fn load_semantic_clone_store_evidence_from_path(
    db_path: &Path,
) -> Result<Option<SemanticCloneStoreEvidence>> {
    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("opening semantic clone db at {}", db_path.display()))?;
    let has_required_tables = ["artefacts_current", "symbol_embeddings", "symbol_clone_edges"]
        .iter()
        .all(|table_name| {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table_name],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count > 0)
            .unwrap_or(false)
        });
    if !has_required_tables {
        return Ok(None);
    }

    let current_artefacts = conn
        .query_row("SELECT COUNT(*) FROM artefacts_current", [], |row| {
            row.get::<_, i64>(0)
        })
        .context("counting semantic clone current artefacts")?;
    let embeddings = conn
        .query_row("SELECT COUNT(*) FROM symbol_embeddings", [], |row| {
            row.get::<_, i64>(0)
        })
        .context("counting semantic clone embeddings")?;
    let clone_edges = conn
        .query_row("SELECT COUNT(*) FROM symbol_clone_edges", [], |row| {
            row.get::<_, i64>(0)
        })
        .context("counting semantic clone clone edges")?;

    Ok(Some(SemanticCloneStoreEvidence {
        current_artefacts: usize::try_from(current_artefacts)
            .context("semantic clone current artefact count overflowed usize")?,
        embeddings: usize::try_from(embeddings)
            .context("semantic clone embedding count overflowed usize")?,
        clone_edges: usize::try_from(clone_edges)
            .context("semantic clone clone-edge count overflowed usize")?,
    }))
}

fn describe_semantic_clone_store_snapshot(snapshot: &SemanticCloneStoreSnapshot) -> String {
    format!(
        "db={}, current_artefacts={}, embeddings={}, clone_edges={}",
        snapshot.path.display(),
        snapshot.evidence.current_artefacts,
        snapshot.evidence.embeddings,
        snapshot.evidence.clone_edges
    )
}

fn semantic_clone_eventual_timeout() -> StdDuration {
    parse_timeout_seconds(
        std::env::var(SEMANTIC_CLONES_EVENTUAL_TIMEOUT_ENV)
            .ok()
            .as_deref(),
        DEFAULT_SEMANTIC_CLONES_EVENTUAL_TIMEOUT_SECS,
    )
}

fn semantic_clone_eventual_poll_interval() -> StdDuration {
    StdDuration::from_millis(SEMANTIC_CLONES_EVENTUAL_POLL_INTERVAL_MILLIS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloneQueryWaitCondition {
    AnyResponse,
    NonEmptyResults,
}

fn clone_query_meets_wait_condition(
    rows: &[serde_json::Value],
    condition: &CloneQueryWaitCondition,
) -> bool {
    match condition {
        CloneQueryWaitCondition::AnyResponse => true,
        CloneQueryWaitCondition::NonEmptyResults => !rows.is_empty(),
    }
}

fn wait_for_semantic_clone_condition<T, Observe, Ready, Describe>(
    timeout: StdDuration,
    poll_interval: StdDuration,
    expected: &str,
    mut observe: Observe,
    is_ready: Ready,
    describe: Describe,
) -> Result<T>
where
    Observe: FnMut() -> Result<T>,
    Ready: Fn(&T) -> bool,
    Describe: Fn(&T) -> String,
{
    let started = Instant::now();
    let mut attempts = 0_usize;
    let mut last_observation: String;

    loop {
        attempts += 1;
        match observe() {
            Ok(value) => {
                let summary = describe(&value);
                if is_ready(&value) {
                    return Ok(value);
                }
                last_observation = format!("value: {summary}");
            }
            Err(err) => {
                last_observation = format!("error: {err:#}");
            }
        }

        if started.elapsed() >= timeout {
            bail!(
                "timed out after {}s waiting for semantic clone {expected}; attempts={attempts}; last observation={}",
                timeout.as_secs(),
                last_observation
            );
        }

        std::thread::sleep(poll_interval);
    }
}

fn wait_for_semantic_clone_store_snapshot(world: &QatWorld) -> Result<SemanticCloneStoreSnapshot> {
    wait_for_semantic_clone_condition(
        semantic_clone_eventual_timeout(),
        semantic_clone_eventual_poll_interval(),
        "persisted semantic clone evidence",
        || load_semantic_clone_store_snapshot(world),
        |snapshot| semantic_clone_store_evidence_proves_rebuild(None, snapshot.evidence),
        describe_semantic_clone_store_snapshot,
    )
}

fn write_semantic_clone_fixture_files(repo_dir: &Path, write_project_files: bool) -> Result<()> {
    let src = repo_dir.join("src");
    fs::create_dir_all(&src).with_context(|| format!("creating {}", src.display()))?;

    if write_project_files {
        fs::write(
            repo_dir.join("package.json"),
            "{\n  \"name\": \"qat-clones-project\",\n  \"private\": true,\n  \"version\": \"0.0.0\",\n  \"type\": \"module\"\n}\n",
        )
        .context("writing package.json")?;
        fs::write(
            repo_dir.join("tsconfig.json"),
            "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\",\n    \"module\": \"ESNext\",\n    \"moduleResolution\": \"bundler\",\n    \"strict\": true,\n    \"outDir\": \"dist\"\n  },\n  \"include\": [\"src\"]\n}\n",
        )
        .context("writing tsconfig.json")?;
    }
    fs::write(
        src.join("render-invoice.ts"),
        "export function renderInvoice(orderId: string, locale: string): string {\n  const invoiceKey = `${orderId}:${locale}`;\n  return invoiceKey.toUpperCase();\n}\n",
    )
    .context("writing src/render-invoice.ts")?;
    fs::write(
        src.join("render-invoice-document.ts"),
        "export function renderInvoiceDocument(orderId: string, locale: string): string {\n  const invoiceKey = `${orderId}:${locale}`;\n  return invoiceKey.toUpperCase();\n}\n",
    )
    .context("writing src/render-invoice-document.ts")?;
    fs::write(
        src.join("render-invoice-draft.ts"),
        "export function renderInvoiceDraft(orderId: string, locale: string): string {\n  const invoiceKey = `${orderId}:${locale}`;\n  return invoiceKey.toUpperCase();\n}\n",
    )
    .context("writing src/render-invoice-draft.ts")?;

    Ok(())
}

pub fn create_ts_project_with_similar_impls(repo_dir: &Path) -> Result<()> {
    write_semantic_clone_fixture_files(repo_dir, true)
}

pub fn add_semantic_clone_fixtures(repo_dir: &Path) -> Result<()> {
    write_semantic_clone_fixture_files(repo_dir, false)
}

pub fn run_devql_semantic_clones_rebuild(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_devql_sync_for_repo(world, repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops devql ingest",
        build_bitloops_command(world, &["devql", "ingest"])?,
    )?;
    ensure_success(&output, "bitloops devql ingest")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let semantic_rows = extract_ingest_metric(&stdout, "semantic_feature_rows_upserted=")
        .ok_or_else(|| {
            anyhow!(
                "bitloops devql ingest completed but did not report semantic_feature_rows_upserted=... in stdout; semantic clones rebuild requires ingest metrics to verify clone setup"
            )
        })?;
    let clone_edges = extract_ingest_metric(&stdout, "symbol_clone_edges_upserted=");
    let store_snapshot = wait_for_semantic_clone_store_snapshot(world).with_context(|| {
        format!(
            "bitloops devql ingest reported semantic_feature_rows_upserted={semantic_rows}, symbol_clone_edges_upserted={clone_edges:?}"
        )
    })?;
    let store_evidence = store_snapshot.evidence;
    if clone_edges.is_none() {
        append_world_log(
            world,
            &format!(
                "Semantic clone ingest stdout omitted symbol_clone_edges_upserted; waited for persisted evidence in {} with current_artefacts={}, embeddings={}, clone_edges={}.\n",
                store_snapshot.path.display(),
                store_evidence.current_artefacts,
                store_evidence.embeddings,
                store_evidence.clone_edges
            ),
        )?;
    } else if clone_edges == Some(0)
        && semantic_clone_store_evidence_proves_rebuild(clone_edges, store_evidence)
    {
        append_world_log(
            world,
            &format!(
                "Semantic clone ingest reported zero clone edges; waited for persisted evidence in {} with current_artefacts={}, embeddings={}, clone_edges={}.\n",
                store_snapshot.path.display(),
                store_evidence.current_artefacts,
                store_evidence.embeddings,
                store_evidence.clone_edges
            ),
        )?;
    }
    ensure!(
        semantic_clone_store_evidence_proves_rebuild(clone_edges, store_evidence),
        "bitloops devql semantic clones rebuild succeeded but did not leave persisted semantic clone evidence in {} (semantic_feature_rows_upserted={semantic_rows}, symbol_clone_edges_upserted={clone_edges:?}, current_artefacts={}, symbol_embeddings={}, symbol_clone_edges={}). Re-run `bitloops devql sync --status` and `bitloops devql ingest` and inspect the semantic provider output.",
        store_snapshot.path.display(),
        store_evidence.current_artefacts,
        store_evidence.embeddings,
        store_evidence.clone_edges
    );
    Ok(())
}

fn extract_clone_nodes(value: &serde_json::Value) -> Vec<serde_json::Value> {
    value
        .as_array()
        .map(|rows| {
            if rows.iter().any(|row| {
                row.get("relationKind").is_some()
                    || row.get("sourceArtefactId").is_some()
                    || row.get("from").is_some()
            }) {
                return rows.clone();
            }
            rows.iter()
                .flat_map(|artefact| {
                    artefact
                        .get("clones")
                        .and_then(|clones| clones.get("edges"))
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flat_map(|edges| edges.iter())
                        .filter_map(|edge| edge.get("node").cloned())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn run_devql_clones_query(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: Option<f64>,
    raw: bool,
) -> Result<Vec<serde_json::Value>> {
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    let mut clone_args = Vec::new();
    if let Some(min_score) = min_score {
        clone_args.push(format!("min_score:{min_score}"));
    }
    if raw {
        clone_args.push("raw:true".to_string());
    }
    let clones_stage = if clone_args.is_empty() {
        "clones()".to_string()
    } else {
        format!("clones({})", clone_args.join(","))
    };
    let query = format!(
        r#"repo("bitloops")->artefacts(symbol_fqn:"{}")->{}->limit(50)"#,
        escape_devql_string(&symbol_fqn),
        clones_stage
    );
    let value = run_devql_query(world, &query)?;
    let clone_rows = extract_clone_nodes(&value);
    world.last_command_stdout =
        Some(serde_json::to_string(&clone_rows).context("serializing flattened clone rows")?);
    world.last_query_result_count = Some(clone_rows.len());
    Ok(clone_rows)
}

fn describe_clone_rows(rows: &[serde_json::Value]) -> String {
    serde_json::to_string(rows).unwrap_or_else(|_| "<failed to serialize clone rows>".to_string())
}

fn max_clone_score(rows: &[serde_json::Value]) -> f64 {
    rows.iter()
        .filter_map(|row| row.get("score").and_then(serde_json::Value::as_f64))
        .fold(0.0_f64, f64::max)
}

fn clone_rows_have_explanation(rows: &[serde_json::Value]) -> bool {
    rows.iter().any(|row| {
        row.get("explanation_json")
            .or_else(|| row.get("metadata"))
            .and_then(|metadata| metadata.get("explanation").or(Some(metadata)))
            .is_some_and(|metadata| match metadata {
                serde_json::Value::Null => false,
                serde_json::Value::Object(map) => !map.is_empty(),
                serde_json::Value::Array(items) => !items.is_empty(),
                serde_json::Value::String(text) => !text.trim().is_empty(),
                _ => true,
            })
    })
}

fn run_devql_clones_query_eventually<Condition>(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: Option<f64>,
    raw: bool,
    expected: &str,
    condition: Condition,
) -> Result<Vec<serde_json::Value>>
where
    Condition: Fn(&[serde_json::Value]) -> bool,
{
    wait_for_semantic_clone_condition(
        semantic_clone_eventual_timeout(),
        semantic_clone_eventual_poll_interval(),
        expected,
        || run_devql_clones_query(world, repo_name, symbol_alias, min_score, raw),
        |rows| condition(rows.as_slice()),
        |rows| describe_clone_rows(rows.as_slice()),
    )
}

fn run_devql_clones_query_eventually_with_wait_condition(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: Option<f64>,
    raw: bool,
    expected: &str,
    wait_condition: CloneQueryWaitCondition,
) -> Result<Vec<serde_json::Value>> {
    run_devql_clones_query_eventually(
        world,
        repo_name,
        symbol_alias,
        min_score,
        raw,
        expected,
        |rows| clone_query_meets_wait_condition(rows, &wait_condition),
    )
}

pub fn assert_devql_clones_query(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_count: usize,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually(
        world,
        repo_name,
        symbol_alias,
        None,
        false,
        &format!("at least {min_count} clone rows for `{symbol_alias}`"),
        |rows| rows.len() >= min_count,
    )?;
    let count = rows.len();
    ensure!(
        count >= min_count,
        "expected at least {min_count} clones for `{symbol_alias}`, got {count}"
    );
    Ok(())
}

pub fn assert_devql_clones_with_min_score(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: f64,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually_with_wait_condition(
        world,
        repo_name,
        symbol_alias,
        Some(min_score),
        false,
        &format!("clone rows with min_score={min_score} for `{symbol_alias}`"),
        CloneQueryWaitCondition::NonEmptyResults,
    )?;
    let count = rows.len();
    ensure!(
        count >= 1,
        "expected at least one clone result with min_score={min_score}, got {count}"
    );
    Ok(())
}

pub fn record_devql_clones_with_min_score(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: f64,
) -> Result<()> {
    let _ = run_devql_clones_query_eventually_with_wait_condition(
        world,
        repo_name,
        symbol_alias,
        Some(min_score),
        false,
        &format!("clone rows with min_score={min_score} for `{symbol_alias}`"),
        CloneQueryWaitCondition::AnyResponse,
    )?;
    Ok(())
}

pub fn assert_last_query_fewer_or_equal(world: &QatWorld, previous_count: usize) -> Result<()> {
    let current = world
        .last_query_result_count
        .ok_or_else(|| anyhow!("no previous query result count captured"))?;
    ensure!(
        current <= previous_count,
        "expected fewer or equal results ({current} <= {previous_count})"
    );
    Ok(())
}

pub fn assert_devql_clones_have_score_and_kind(world: &QatWorld) -> Result<()> {
    let value = parse_last_command_stdout_json(world)?;
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow!("expected clones query to return JSON array"))?;
    ensure!(!rows.is_empty(), "expected at least one clone row");
    for row in rows {
        let has_score = row
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .is_some();
        ensure!(has_score, "clone row missing score field: {row}");
        ensure!(
            row.get("relationKind")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "clone row missing relationKind: {row}"
        );
    }
    Ok(())
}

pub fn assert_devql_clones_top_score_above(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    threshold: f64,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually(
        world,
        repo_name,
        symbol_alias,
        None,
        false,
        &format!("a top clone score above {threshold} for `{symbol_alias}`"),
        |rows| max_clone_score(rows) > threshold,
    )?;
    let max_score = max_clone_score(&rows);
    ensure!(
        max_score > threshold,
        "expected top clone score > {threshold}, got {max_score}"
    );
    Ok(())
}

pub fn assert_devql_clones_have_explanation(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually(
        world,
        repo_name,
        symbol_alias,
        None,
        true,
        &format!("clone explanation payload for `{symbol_alias}`"),
        clone_rows_have_explanation,
    )?;
    ensure!(!rows.is_empty(), "expected at least one clone row");
    let has_explanation = clone_rows_have_explanation(&rows);
    ensure!(
        has_explanation,
        "expected at least one clone row with explanation payload"
    );
    Ok(())
}
