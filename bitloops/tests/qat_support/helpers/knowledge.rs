use toml_edit::{DocumentMut, Item, Table, Value as TomlValue};

const KNOWLEDGE_FIXTURE_ALPHA: &str = "alpha";
const KNOWLEDGE_FIXTURE_BETA: &str = "beta";
const KNOWLEDGE_FIXTURE_ALPHA_PAGE_ID: &str = "1001";
const KNOWLEDGE_FIXTURE_BETA_PAGE_ID: &str = "1002";

#[derive(Debug, Clone, PartialEq, Eq)]
struct KnowledgeRelationAssertionRecord {
    knowledge_item_id: String,
    target_type: String,
    target_id: String,
    relation_type: String,
    association_method: String,
}

pub fn configure_deterministic_confluence_knowledge_fixtures_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;

    let server = KnowledgeStubServer::start()?;
    let base_url = server.base_url().to_string();
    enqueue_confluence_fixture_responses(&server);
    write_confluence_provider_configs(world.repo_dir(), &base_url)?;
    if world.daemon_process.is_some() || world.daemon_url.is_some() {
        stop_daemon_for_scenario(world)?;
        ensure_daemon_for_scenario(world)?;
    }

    world.knowledge_fixture_urls = HashMap::from([
        (
            KNOWLEDGE_FIXTURE_ALPHA.to_string(),
            fixture_confluence_page_url(&base_url, KNOWLEDGE_FIXTURE_ALPHA_PAGE_ID, "Alpha"),
        ),
        (
            KNOWLEDGE_FIXTURE_BETA.to_string(),
            fixture_confluence_page_url(&base_url, KNOWLEDGE_FIXTURE_BETA_PAGE_ID, "Beta"),
        ),
    ]);
    world.knowledge_stub_server = Some(server);
    Ok(())
}

pub fn run_fixture_knowledge_add(world: &mut QatWorld, fixture_name: &str) -> Result<()> {
    let url = resolve_fixture_knowledge_url(world, fixture_name)?;
    run_knowledge_add(world, &url)?;
    mirror_knowledge_item_id_for_alias(world, fixture_name, &url);
    Ok(())
}

pub fn run_fixture_knowledge_refresh(world: &mut QatWorld, fixture_name: &str) -> Result<()> {
    let url = resolve_fixture_knowledge_url(world, fixture_name)?;
    if let Some(knowledge_item_id) = world.knowledge_items_by_url.get(&url).cloned() {
        world
            .knowledge_items_by_url
            .insert(fixture_name.to_string(), knowledge_item_id);
    }
    run_knowledge_refresh(world, fixture_name)
}

pub fn run_knowledge_add(world: &mut QatWorld, url: &str) -> Result<()> {
    let output = run_command_capture(
        world,
        "bitloops devql knowledge add",
        build_bitloops_command(world, &["devql", "knowledge", "add", url])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    ensure_success(&output, "bitloops devql knowledge add")?;

    if let Some(knowledge_item_id) = parse_knowledge_item_id_from_output(&stdout) {
        world
            .knowledge_items_by_url
            .insert(url.to_string(), knowledge_item_id.clone());
        world
            .knowledge_versions_by_ref
            .entry(knowledge_item_id)
            .or_insert(1);
    }
    world.last_knowledge_add_had_commit_association = Some(false);
    Ok(())
}

pub fn run_knowledge_add_with_commit(world: &mut QatWorld, url: &str) -> Result<()> {
    let sha = resolve_head_sha(world)?;
    let output = run_command_capture(
        world,
        "bitloops devql knowledge add --commit",
        build_bitloops_command(world, &["devql", "knowledge", "add", url, "--commit", &sha])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    ensure_success(&output, "bitloops devql knowledge add --commit")?;

    if let Some(knowledge_item_id) = parse_knowledge_item_id_from_output(&stdout) {
        world
            .knowledge_items_by_url
            .insert(url.to_string(), knowledge_item_id.clone());
        world
            .knowledge_versions_by_ref
            .entry(knowledge_item_id)
            .or_insert(1);
    }
    world.last_knowledge_add_had_commit_association =
        Some(stdout.contains("target: commit:") || stdout.contains("Association created"));
    Ok(())
}

pub fn run_knowledge_associate(world: &mut QatWorld, source: &str, target: &str) -> Result<()> {
    let source_ref = resolve_knowledge_ref_from_input(world, source)?;
    let target_ref = resolve_knowledge_ref_from_input(world, target)?;
    let output = run_command_capture(
        world,
        "bitloops devql knowledge associate",
        build_bitloops_command(
            world,
            &[
                "devql",
                "knowledge",
                "associate",
                &source_ref,
                "--to",
                &target_ref,
            ],
        )?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_stdout = Some(String::from_utf8_lossy(&output.stdout).to_string());
    ensure_success(&output, "bitloops devql knowledge associate")
}

pub fn run_knowledge_refresh(world: &mut QatWorld, input: &str) -> Result<()> {
    let knowledge_ref = resolve_knowledge_ref_from_input(world, input)?;
    let output = run_command_capture(
        world,
        "bitloops devql knowledge refresh",
        build_bitloops_command(world, &["devql", "knowledge", "refresh", &knowledge_ref])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_stdout = Some(String::from_utf8_lossy(&output.stdout).to_string());
    ensure_success(&output, "bitloops devql knowledge refresh")
}

pub fn run_knowledge_add_expect_failure(world: &mut QatWorld, url: &str) -> Result<()> {
    let output = run_command_capture(
        world,
        "bitloops devql knowledge add (expect failure)",
        build_bitloops_command(world, &["devql", "knowledge", "add", url])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_stdout = Some(String::from_utf8_lossy(&output.stdout).to_string());
    Ok(())
}

pub fn assert_last_command_failed(world: &QatWorld) -> Result<()> {
    let code = world
        .last_command_exit_code
        .ok_or_else(|| anyhow!("no command exit code captured"))?;
    ensure!(code != 0, "expected command failure, got exit code {code}");
    Ok(())
}

pub fn assert_devql_knowledge_query_count(
    world: &mut QatWorld,
    repo_name: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let value = run_devql_query(world, r#"repo("bitloops")->knowledge()->limit(50)"#)?;
    let count = count_json_array_rows(&value);
    world.last_query_result_count = Some(count);
    ensure!(
        count >= min_count,
        "expected at least {min_count} knowledge items, got {count}"
    );
    Ok(())
}

pub fn assert_devql_knowledge_query_exact_count(
    world: &mut QatWorld,
    repo_name: &str,
    expected_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let value = run_devql_query(world, r#"repo("bitloops")->knowledge()->limit(50)"#)?;
    let count = count_json_array_rows(&value);
    world.last_query_result_count = Some(count);
    ensure!(
        count == expected_count,
        "expected exactly {expected_count} knowledge items, got {count}"
    );
    Ok(())
}

pub fn assert_knowledge_item_provider_and_kind(
    world: &QatWorld,
    provider: &str,
    source_kind: &str,
) -> Result<()> {
    let value = parse_last_command_stdout_json(world)?;
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow!("expected knowledge query to return a JSON array"))?;
    let found = rows.iter().any(|row| {
        let provider_matches = row
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(provider));
        let source_kind_matches = row
            .get("sourceKind")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|actual| {
                actual.eq_ignore_ascii_case(source_kind)
                    || actual
                        .to_ascii_lowercase()
                        .ends_with(&format!("_{}", source_kind.to_ascii_lowercase()))
            });
        provider_matches && source_kind_matches
    });
    ensure!(
        found,
        "no knowledge row with provider `{provider}` and sourceKind `{source_kind}`"
    );
    Ok(())
}

pub fn assert_knowledge_item_has_commit_association(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let source_knowledge_item_id = resolve_last_command_knowledge_item_id(world)?;
    let target_commit_sha = world
        .last_command_stdout
        .as_deref()
        .and_then(|stdout| parse_association_target_from_output(stdout, "commit"))
        .or_else(|| resolve_head_sha(world).ok())
        .ok_or_else(|| anyhow!("could not resolve commit association target SHA"))?;
    let relations = load_knowledge_relation_assertions(world, repo_name)?;
    ensure!(
        knowledge_relation_exists_for_target(
            &relations,
            &source_knowledge_item_id,
            "commit",
            &target_commit_sha,
        ),
        "expected persisted commit association for knowledge item `{source_knowledge_item_id}` and commit `{target_commit_sha}`"
    );
    Ok(())
}

pub fn assert_knowledge_item_associated_to_knowledge_item(
    world: &QatWorld,
    repo_name: &str,
    source_input: &str,
    target_input: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let source_knowledge_item_id = resolve_knowledge_item_id_from_input(world, source_input)?;
    let target_knowledge_item_id = resolve_knowledge_item_id_from_input(world, target_input)?;
    let relations = load_knowledge_relation_assertions(world, repo_name)?;
    ensure!(
        knowledge_relation_exists_for_target(
            &relations,
            &source_knowledge_item_id,
            "knowledge_item",
            &target_knowledge_item_id,
        ),
        "expected persisted knowledge association from `{source_knowledge_item_id}` to `{target_knowledge_item_id}`"
    );
    Ok(())
}

pub fn assert_knowledge_versions_count(
    world: &mut QatWorld,
    input: &str,
    expected_count: usize,
) -> Result<()> {
    let knowledge_ref = resolve_knowledge_ref_from_input(world, input)?;
    let output = run_command_capture(
        world,
        "bitloops devql knowledge versions",
        build_bitloops_command(world, &["devql", "knowledge", "versions", &knowledge_ref])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    ensure_success(&output, "bitloops devql knowledge versions")?;

    let count = parse_knowledge_versions_count(&stdout)?;
    ensure!(
        count == expected_count,
        "expected {expected_count} knowledge versions, got {count}"
    );
    Ok(())
}

fn resolve_fixture_knowledge_url(world: &QatWorld, fixture_name: &str) -> Result<String> {
    world
        .knowledge_fixture_urls
        .get(fixture_name)
        .cloned()
        .ok_or_else(|| anyhow!("no deterministic knowledge fixture named `{fixture_name}`"))
}

fn mirror_knowledge_item_id_for_alias(world: &mut QatWorld, alias: &str, url: &str) {
    if let Some(knowledge_item_id) = world.knowledge_items_by_url.get(url).cloned() {
        world
            .knowledge_items_by_url
            .insert(alias.to_string(), knowledge_item_id);
    }
}

fn load_knowledge_relation_assertions(
    world: &QatWorld,
    repo_name: &str,
) -> Result<Vec<KnowledgeRelationAssertionRecord>> {
    ensure_bitloops_repo_name(repo_name)?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT knowledge_item_id, target_type, target_id, relation_type, association_method
             FROM knowledge_relation_assertions
             WHERE repo_id = ?1
             ORDER BY created_at DESC, relation_assertion_id DESC",
        )
        .context("preparing knowledge_relation_assertions query for QAT assertions")?;
    let rows = stmt.query_map([repo_id.as_str()], |row| {
        Ok(KnowledgeRelationAssertionRecord {
            knowledge_item_id: row.get::<_, String>(0)?.trim().to_string(),
            target_type: row.get::<_, String>(1)?.trim().to_string(),
            target_id: row.get::<_, String>(2)?.trim().to_string(),
            relation_type: row.get::<_, String>(3)?.trim().to_string(),
            association_method: row.get::<_, String>(4)?.trim().to_string(),
        })
    })?;

    rows.map(|row| row.map_err(anyhow::Error::from)).collect()
}

fn knowledge_relation_exists_for_target(
    relations: &[KnowledgeRelationAssertionRecord],
    source_knowledge_item_id: &str,
    target_type: &str,
    target_id: &str,
) -> bool {
    relations.iter().any(|relation| {
        relation.knowledge_item_id == source_knowledge_item_id
            && relation.target_type == target_type
            && relation.target_id == target_id
            && relation.relation_type == "associated_with"
            && relation.association_method == "manual_attachment"
    })
}

fn resolve_knowledge_item_id_from_input(world: &QatWorld, input: &str) -> Result<String> {
    if let Some(knowledge_item_id) = input.strip_prefix("knowledge:") {
        return Ok(knowledge_item_id.to_string());
    }

    world
        .knowledge_items_by_url
        .get(input)
        .cloned()
        .ok_or_else(|| anyhow!("no knowledge item id captured for `{input}`"))
}

fn resolve_last_command_knowledge_item_id(world: &QatWorld) -> Result<String> {
    if let Some(stdout) = world.last_command_stdout.as_deref()
        && let Some(knowledge_item_id) = parse_knowledge_item_id_from_output(stdout)
    {
        return Ok(knowledge_item_id);
    }

    let unique_ids = world
        .knowledge_items_by_url
        .values()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if unique_ids.len() == 1 {
        return unique_ids
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("expected a captured knowledge item id"));
    }

    bail!("could not resolve the last knowledge item id from captured QAT state")
}

fn parse_association_target_from_output(stdout: &str, target_type: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let raw = line.trim().strip_prefix("target:")?.trim();
        let (actual_type, target_id) = raw.split_once(':')?;
        (actual_type.trim() == target_type)
            .then(|| target_id.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn fixture_confluence_page_url(base_url: &str, page_id: &str, title_slug: &str) -> String {
    format!("{base_url}/wiki/spaces/QAT/pages/{page_id}/{title_slug}")
}

fn fixture_confluence_content_path(page_id: &str) -> String {
    format!("/wiki/rest/api/content/{page_id}?expand=body.storage,version")
}

fn enqueue_confluence_fixture_responses(server: &KnowledgeStubServer) {
    server.enqueue_json(
        fixture_confluence_content_path(KNOWLEDGE_FIXTURE_ALPHA_PAGE_ID),
        serde_json::json!({
            "title": "Alpha knowledge page",
            "version": {
                "when": "2026-04-16T10:00:00Z",
                "by": { "displayName": "QAT Docs" }
            },
            "body": {
                "storage": {
                    "value": "<p>Initial alpha content</p>"
                }
            }
        }),
    );
    server.enqueue_json(
        fixture_confluence_content_path(KNOWLEDGE_FIXTURE_ALPHA_PAGE_ID),
        serde_json::json!({
            "title": "Alpha knowledge page",
            "version": {
                "when": "2026-04-16T10:05:00Z",
                "by": { "displayName": "QAT Docs" }
            },
            "body": {
                "storage": {
                    "value": "<p>Updated alpha content for refresh</p>"
                }
            }
        }),
    );
    server.enqueue_json(
        fixture_confluence_content_path(KNOWLEDGE_FIXTURE_BETA_PAGE_ID),
        serde_json::json!({
            "title": "Beta knowledge page",
            "version": {
                "when": "2026-04-16T10:01:00Z",
                "by": { "displayName": "QAT Docs" }
            },
            "body": {
                "storage": {
                    "value": "<p>Beta reference content</p>"
                }
            }
        }),
    );
}

fn write_confluence_provider_configs(repo_dir: &Path, base_url: &str) -> Result<()> {
    let mut config_paths = vec![repo_dir.join(bitloops::config::BITLOOPS_CONFIG_RELATIVE_PATH)];
    if let Ok(bound_config_path) = bitloops::config::resolve_bound_daemon_config_path_for_repo(repo_dir)
    {
        if !config_paths.iter().any(|path| path == &bound_config_path) {
            config_paths.push(bound_config_path);
        }
    }
    for config_path in config_paths {
        write_confluence_provider_config(&config_path, base_url)?;
    }
    Ok(())
}

fn write_confluence_provider_config(config_path: &Path, base_url: &str) -> Result<()> {
    let mut doc = if config_path.exists() {
        fs::read_to_string(&config_path)
            .with_context(|| format!("reading {}", config_path.display()))?
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing {}", config_path.display()))?
    } else {
        DocumentMut::new()
    };

    if doc.get("knowledge").is_none_or(|item| !item.is_table()) {
        doc["knowledge"] = Item::Table(Table::new());
    }
    let knowledge = doc["knowledge"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("knowledge config should be a table"))?;
    if knowledge
        .get("providers")
        .is_none_or(|item| !item.is_table())
    {
        knowledge["providers"] = Item::Table(Table::new());
    }
    let providers = knowledge["providers"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("knowledge.providers config should be a table"))?;
    if providers
        .get("confluence")
        .is_none_or(|item| !item.is_table())
    {
        providers["confluence"] = Item::Table(Table::new());
    }
    let confluence = providers["confluence"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("knowledge.providers.confluence config should be a table"))?;
    confluence["site_url"] = Item::Value(TomlValue::from(base_url));
    confluence["email"] = Item::Value(TomlValue::from("qat@example.com"));
    confluence["token"] = Item::Value(TomlValue::from("qat-token"));

    fs::write(&config_path, doc.to_string())
        .with_context(|| format!("writing {}", config_path.display()))
}

#[cfg(test)]
mod knowledge_config_tests {
    use super::write_confluence_provider_config;
    use anyhow::Result;

    #[test]
    fn write_confluence_provider_config_creates_nested_tables_on_existing_config() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let config_path =
            temp_dir.path().join(bitloops::config::BITLOOPS_CONFIG_RELATIVE_PATH);
        std::fs::create_dir_all(
            config_path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("config path missing parent"))?,
        )?;
        std::fs::write(&config_path, "[daemon]\nenabled = true\n")?;

        write_confluence_provider_config(&config_path, "http://127.0.0.1:4242")?;

        let config = std::fs::read_to_string(config_path)?;
        assert!(config.contains("[knowledge.providers.confluence]"));
        assert!(config.contains("site_url = \"http://127.0.0.1:4242\""));
        assert!(config.contains("email = \"qat@example.com\""));
        assert!(config.contains("token = \"qat-token\""));
        Ok(())
    }
}
