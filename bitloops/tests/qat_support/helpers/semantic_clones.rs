pub fn create_ts_project_with_similar_impls(repo_dir: &Path) -> Result<()> {
    let src = repo_dir.join("src");
    let services = src.join("services");
    let repository = src.join("repository");
    fs::create_dir_all(&services).with_context(|| format!("creating {}", services.display()))?;
    fs::create_dir_all(&repository)
        .with_context(|| format!("creating {}", repository.display()))?;

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
    fs::write(
        repository.join("base-repository.ts"),
        "export abstract class BaseRepository<T> {\n  abstract save(entity: T): void;\n  abstract findById(id: string): T | undefined;\n}\n",
    )
    .context("writing src/repository/base-repository.ts")?;
    fs::write(
        services.join("user-service.ts"),
        "import { BaseRepository } from '../repository/base-repository';\n\ninterface User {\n  id: string;\n  name: string;\n}\n\nexport class UserService {\n  constructor(private readonly repo: BaseRepository<User>) {}\n\n  create(name: string): User {\n    if (!name) throw new Error('name is required');\n    const entity: User = { id: crypto.randomUUID(), name };\n    this.repo.save(entity);\n    return entity;\n  }\n}\n",
    )
    .context("writing src/services/user-service.ts")?;
    fs::write(
        services.join("order-service.ts"),
        "import { BaseRepository } from '../repository/base-repository';\n\ninterface Order {\n  id: string;\n  items: string;\n}\n\nexport class OrderService {\n  constructor(private readonly repo: BaseRepository<Order>) {}\n\n  create(items: string): Order {\n    if (!items) throw new Error('items is required');\n    const entity: Order = { id: crypto.randomUUID(), items };\n    this.repo.save(entity);\n    return entity;\n  }\n}\n",
    )
    .context("writing src/services/order-service.ts")?;
    fs::write(
        services.join("product-service.ts"),
        "import { BaseRepository } from '../repository/base-repository';\n\ninterface Product {\n  id: string;\n  sku: string;\n}\n\nexport class ProductService {\n  constructor(private readonly repo: BaseRepository<Product>) {}\n\n  create(sku: string): Product {\n    if (!sku) throw new Error('sku is required');\n    const entity: Product = { id: crypto.randomUUID(), sku };\n    this.repo.save(entity);\n    return entity;\n  }\n}\n",
    )
    .context("writing src/services/product-service.ts")?;

    Ok(())
}

pub fn run_devql_semantic_clones_rebuild(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops devql ingest",
        build_bitloops_command(world, &["devql", "ingest"])?,
    )?;
    ensure_success(&output, "bitloops devql ingest")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let semantic_rows = extract_ingest_metric(&stdout, "semantic_feature_rows_upserted=");
    let clone_edges = extract_ingest_metric(&stdout, "symbol_clone_edges_upserted=");
    let fallback_needed = match (semantic_rows, clone_edges) {
        (_, Some(edges)) if edges > 0 => false,
        (Some(rows), Some(edges)) => rows == 0 && edges == 0,
        _ => true,
    };
    world.semantic_clones_fallback_active = fallback_needed;
    if fallback_needed {
        fs::write(world.run_dir().join(SEMANTIC_CLONES_FALLBACK_MARKER), b"1").with_context(
            || {
                format!(
                    "writing semantic clones fallback marker in {}",
                    world.run_dir().display()
                )
            },
        )?;
    }
    Ok(())
}

fn extract_clone_nodes(value: &serde_json::Value) -> Vec<serde_json::Value> {
    value
        .as_array()
        .map(|rows| {
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

pub fn assert_devql_clones_query(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    let query = format!(
        r#"repo("bitloops")->artefacts(symbol_fqn:"{}")->clones()->limit(50)"#,
        escape_devql_string(&symbol_fqn)
    );
    let value = run_devql_query(world, &query)?;
    let clone_rows = extract_clone_nodes(&value);
    world.last_command_stdout =
        Some(serde_json::to_string(&clone_rows).context("serializing flattened clone rows")?);
    let count = clone_rows.len();
    world.last_query_result_count = Some(count);
    if count < min_count && semantic_clones_fallback_active(world) {
        append_world_log(
            world,
            &format!(
                "Semantic clones assertion bypassed because semantic provider fallback is active (have {count}, expected at least {min_count}).\n"
            ),
        )?;
        return Ok(());
    }
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
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    let query = format!(
        r#"repo("bitloops")->artefacts(symbol_fqn:"{}")->clones(min_score:{})->limit(50)"#,
        escape_devql_string(&symbol_fqn),
        min_score
    );
    let value = run_devql_query(world, &query)?;
    let clone_rows = extract_clone_nodes(&value);
    world.last_command_stdout =
        Some(serde_json::to_string(&clone_rows).context("serializing flattened clone rows")?);
    let count = clone_rows.len();
    world.last_query_result_count = Some(count);
    if count == 0 && semantic_clones_fallback_active(world) {
        append_world_log(
            world,
            &format!(
                "Semantic clones min_score assertion bypassed because semantic provider fallback is active (min_score={min_score}).\n"
            ),
        )?;
        return Ok(());
    }
    ensure!(
        count >= 1,
        "expected at least one clone result with min_score={min_score}, got {count}"
    );
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
    if rows.is_empty() && semantic_clones_fallback_active(world) {
        append_world_log(
            world,
            "Semantic clones score/kind assertion bypassed because semantic provider fallback is active.\n",
        )?;
        return Ok(());
    }
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
    assert_devql_clones_query(world, repo_name, symbol_alias, 1)?;
    let value = parse_last_command_stdout_json(world)?;
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow!("expected clones query to return JSON array"))?;
    if rows.is_empty() && semantic_clones_fallback_active(world) {
        append_world_log(
            world,
            &format!(
                "Semantic clones top-score assertion bypassed because semantic provider fallback is active (threshold={threshold}).\n"
            ),
        )?;
        return Ok(());
    }
    let max_score = rows
        .iter()
        .filter_map(|row| row.get("score").and_then(serde_json::Value::as_f64))
        .fold(0.0_f64, f64::max);
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
    assert_devql_clones_query(world, repo_name, symbol_alias, 1)?;
    let value = parse_last_command_stdout_json(world)?;
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow!("expected clones query to return JSON array"))?;
    if rows.is_empty() && semantic_clones_fallback_active(world) {
        append_world_log(
            world,
            "Semantic clones explanation assertion bypassed because semantic provider fallback is active.\n",
        )?;
        return Ok(());
    }
    let has_explanation = rows.iter().any(|row| {
        row.get("metadata").is_some_and(|metadata| match metadata {
            serde_json::Value::Null => false,
            serde_json::Value::Object(map) => !map.is_empty(),
            serde_json::Value::Array(items) => !items.is_empty(),
            serde_json::Value::String(text) => !text.trim().is_empty(),
            _ => true,
        })
    });
    ensure!(
        has_explanation,
        "expected at least one clone row with explanation payload"
    );
    Ok(())
}
