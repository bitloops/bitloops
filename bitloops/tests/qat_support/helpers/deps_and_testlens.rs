pub fn create_ts_project_with_known_deps(repo_dir: &Path) -> Result<()> {
    let src = repo_dir.join("src");
    for dir in [
        src.join("models"),
        src.join("repository"),
        src.join("services"),
        src.join("controllers"),
    ] {
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    fs::write(
        repo_dir.join("package.json"),
        "{\n  \"name\": \"qat-deps-project\",\n  \"private\": true,\n  \"version\": \"0.0.0\",\n  \"type\": \"module\"\n}\n",
    )
    .context("writing package.json")?;
    fs::write(
        repo_dir.join("tsconfig.json"),
        "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\",\n    \"module\": \"ESNext\",\n    \"moduleResolution\": \"bundler\",\n    \"strict\": true,\n    \"outDir\": \"dist\"\n  },\n  \"include\": [\"src\"]\n}\n",
    )
    .context("writing tsconfig.json")?;
    fs::write(
        src.join("models").join("user.ts"),
        "export interface User {\n  id: string;\n  name: string;\n}\n",
    )
    .context("writing src/models/user.ts")?;
    fs::write(
        src.join("repository").join("user-repository.ts"),
        "import { User } from '../models/user';\n\nexport class UserRepository {\n  save(user: User): void {\n    void user;\n  }\n\n  findById(id: string): User | undefined {\n    void id;\n    return undefined;\n  }\n}\n",
    )
    .context("writing src/repository/user-repository.ts")?;
    fs::write(
        src.join("services").join("user-service.ts"),
        "import { User } from '../models/user';\nimport { UserRepository } from '../repository/user-repository';\n\nexport class UserService {\n  createUser(name: string, repo: UserRepository): User {\n    const user: User = { id: crypto.randomUUID(), name };\n    repo.save(user);\n    return user;\n  }\n}\n",
    )
    .context("writing src/services/user-service.ts")?;
    fs::write(
        src.join("controllers").join("user-controller.ts"),
        "import { UserService } from '../services/user-service';\nimport { UserRepository } from '../repository/user-repository';\n\nexport function handleCreate(name: string): string {\n  const repo = new UserRepository();\n  const service = new UserService();\n  const user = service.createUser(name, repo);\n  return user.id;\n}\n",
    )
    .context("writing src/controllers/user-controller.ts")?;
    fs::write(
        src.join("index.ts"),
        "import { handleCreate } from './controllers/user-controller';\n\nconsole.log(handleCreate('Alice'));\n",
    )
    .context("writing src/index.ts")?;

    Ok(())
}

pub fn create_simple_rust_project(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let repo_dir = world.repo_dir();
    fs::create_dir_all(repo_dir.join("src"))
        .with_context(|| format!("creating {}", repo_dir.join("src").display()))?;

    fs::write(
        repo_dir.join("Cargo.toml"),
        "[package]\nname = \"qat-sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .context("writing Cargo.toml")?;
    fs::write(repo_dir.join(".gitignore"), "target/\n").context("writing .gitignore")?;
    fs::write(
        repo_dir.join("src/main.rs"),
        "fn main() {\n    println!(\"Hello, world!\");\n}\n",
    )
    .context("writing src/main.rs")?;
    Ok(())
}

pub fn create_rust_project_with_tests(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let repo_dir = world.repo_dir();
    fs::create_dir_all(repo_dir.join("src"))
        .with_context(|| format!("creating {}", repo_dir.join("src").display()))?;
    fs::create_dir_all(repo_dir.join("coverage"))
        .with_context(|| format!("creating {}", repo_dir.join("coverage").display()))?;

    fs::write(
        repo_dir.join("Cargo.toml"),
        "[package]\nname = \"qat-sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .context("writing Cargo.toml")?;
    fs::write(repo_dir.join(".gitignore"), "target/\n").context("writing .gitignore")?;
    fs::write(
        repo_dir.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\npub fn multiply(a: i32, b: i32) -> i32 {\n    a * b\n}\n\npub fn greet(name: &str) -> String {\n    format!(\"Hello, {}!\", name)\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn test_add() {\n        assert_eq!(add(2, 3), 5);\n    }\n\n    #[test]\n    fn test_multiply() {\n        assert_eq!(multiply(3, 4), 12);\n    }\n\n    #[test]\n    fn test_greet() {\n        assert_eq!(greet(\"World\"), \"Hello, World!\");\n    }\n}\n",
    )
    .context("writing src/lib.rs")?;
    fs::write(
        repo_dir.join("coverage/lcov.info"),
        "TN:\nSF:src/lib.rs\nFN:1,add\nFNDA:1,add\nFN:5,multiply\nFNDA:1,multiply\nFN:9,greet\nFNDA:1,greet\nDA:2,1\nDA:6,1\nDA:10,1\nLF:3\nLH:3\nend_of_record\n",
    )
    .context("writing coverage/lcov.info")?;
    Ok(())
}

pub fn add_new_caller_of_symbol(world: &mut QatWorld, symbol_alias: &str) -> Result<()> {
    let file_path = world.repo_dir().join("src").join("new-caller.ts");
    let content = if let Some((service_name, method_name)) = symbol_alias.split_once('.') {
        let import_path = format!("./services/{}", to_kebab_case(service_name));
        let helper_name = format!("invoke{}", method_name.to_ascii_uppercase());
        format!(
            "import {{ {service_name} }} from '{import_path}';\nimport {{ UserRepository }} from './repository/user-repository';\n\nfunction {helper_name}(name: string, repo: UserRepository): void {{\n  const service = new {service_name}();\n  service.{method_name}(name, repo);\n}}\n\nexport function callCreateUser(): void {{\n  const repo = new UserRepository();\n  {helper_name}('QAT-new-caller', repo);\n}}\n\ncallCreateUser();\n"
        )
    } else {
        format!(
            "import {{ {symbol_alias} }} from './services/user-service';\nimport {{ UserRepository }} from './repository/user-repository';\n\nexport function callCreateUser(): void {{\n  const repo = new UserRepository();\n  {symbol_alias}('QAT-new-caller', repo);\n}}\n\ncallCreateUser();\n"
        )
    };
    fs::write(&file_path, content).with_context(|| format!("writing {}", file_path.display()))?;

    run_devql_ingest_for_repo(world, BITLOOPS_REPO_NAME)
}

pub fn assert_devql_deps_query(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    direction: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    let count = count_deps_for_symbol(world, &symbol_fqn, direction, None)?;
    world.last_query_result_count = Some(count);
    ensure!(
        count >= min_count,
        "expected at least {min_count} deps({direction}) rows for `{symbol_alias}`, got {count}"
    );
    Ok(())
}

pub fn assert_devql_deps_query_as_of_commit(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    direction: &str,
    commit_sha: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    let count = count_deps_for_symbol(world, &symbol_fqn, direction, Some(commit_sha))?;
    world.last_query_result_count = Some(count);
    ensure!(
        count >= min_count,
        "expected at least {min_count} deps({direction}) rows for `{symbol_alias}` asOf `{commit_sha}`, got {count}"
    );
    Ok(())
}

pub fn assert_devql_deps_query_as_of_commit_exact_count(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    direction: &str,
    commit_sha: &str,
    expected_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    let count = count_deps_for_symbol(world, &symbol_fqn, direction, Some(commit_sha))?;
    world.last_query_result_count = Some(count);
    ensure!(
        count == expected_count,
        "expected exactly {expected_count} deps({direction}) rows for `{symbol_alias}` asOf `{commit_sha}`, got {count}"
    );
    Ok(())
}

fn build_deps_graphql_query(
    symbol_fqn: &str,
    direction: &str,
    commit_sha: Option<&str>,
) -> Result<String> {
    let escaped_symbol = escape_devql_string(symbol_fqn);
    let field = match direction {
        "out" => "outgoingDeps",
        "in" => "incomingDeps",
        "both" => "depsBoth: outgoingDeps",
        other => bail!("unsupported deps direction `{other}`"),
    };
    let connection = if direction == "both" {
        format!(
            "incomingDeps(filter: {{ kind: CALLS }}, first: 50) {{ totalCount }}\n          {field}(filter: {{ kind: CALLS }}, first: 50) {{ totalCount }}"
        )
    } else {
        format!("{field}(filter: {{ kind: CALLS }}, first: 50) {{ totalCount }}")
    };
    let scoped_body = format!(
        r#"artefacts(filter: {{ symbolFqn: "{escaped_symbol}" }}, first: 1) {{
        edges {{
          node {{
            {connection}
          }}
        }}
      }}"#
    );
    let query = match commit_sha {
        Some(commit_sha) => format!(
            r#"query {{
  history: asOf(input: {{ commit: "{}" }}) {{
    {}
  }}
}}"#,
            escape_devql_string(commit_sha),
            scoped_body
        ),
        None => format!(
            r#"query {{
  {}
}}"#,
            scoped_body
        ),
    };
    Ok(query)
}

fn count_deps_for_symbol(
    world: &mut QatWorld,
    symbol_fqn: &str,
    direction: &str,
    commit_sha: Option<&str>,
) -> Result<usize> {
    let query = build_deps_graphql_query(symbol_fqn, direction, commit_sha)?;
    let value = run_devql_graphql_query(world, &query)?;
    let scope = match commit_sha {
        Some(_) => value.get("history"),
        None => Some(&value),
    }
    .ok_or_else(|| anyhow!("expected GraphQL deps scope in response"))?;
    let node = scope
        .get("artefacts")
        .and_then(|artefacts| artefacts.get("edges"))
        .and_then(serde_json::Value::as_array)
        .and_then(|edges| edges.first())
        .and_then(|edge| edge.get("node"));
    let count = match (direction, node) {
        (_, None) => 0,
        ("out", Some(node)) => node
            .get("outgoingDeps")
            .and_then(|deps| deps.get("totalCount"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize,
        ("in", Some(node)) => node
            .get("incomingDeps")
            .and_then(|deps| deps.get("totalCount"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize,
        ("both", Some(node)) => {
            let incoming = node
                .get("incomingDeps")
                .and_then(|deps| deps.get("totalCount"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let outgoing = node
                .get("depsBoth")
                .and_then(|deps| deps.get("totalCount"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            (incoming + outgoing) as usize
        }
        _ => 0,
    };
    Ok(count)
}

pub fn assert_devql_artefacts_count_stable(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let count_first = count_artefacts_across_source_files(world)?;
    run_devql_ingest_for_repo(world, repo_name)?;
    let count_second = count_artefacts_across_source_files(world)?;
    ensure!(
        count_first == count_second,
        "artefact count changed after re-ingest: {count_first} -> {count_second}"
    );
    Ok(())
}

pub fn create_ts_project_with_tests_and_coverage(repo_dir: &Path) -> Result<()> {
    let src_services = repo_dir.join("src").join("services");
    let tests_dir = repo_dir.join("tests");
    let coverage_dir = repo_dir.join("coverage");
    let test_results_dir = repo_dir.join("test-results");
    for dir in [&src_services, &tests_dir, &coverage_dir, &test_results_dir] {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    fs::write(
        repo_dir.join("package.json"),
        "{\n  \"name\": \"qat-test-project\",\n  \"private\": true,\n  \"version\": \"0.0.0\",\n  \"type\": \"module\"\n}\n",
    )
    .context("writing package.json")?;
    fs::write(
        repo_dir.join("tsconfig.json"),
        "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\",\n    \"module\": \"ESNext\",\n    \"moduleResolution\": \"bundler\",\n    \"strict\": true,\n    \"outDir\": \"dist\"\n  },\n  \"include\": [\"src\", \"tests\"]\n}\n",
    )
    .context("writing tsconfig.json")?;
    fs::write(
        src_services.join("user-service.ts"),
        "export class UserService {\n  createUser(name: string): { id: string; name: string } {\n    if (!name || name.trim().length === 0) {\n      throw new Error('Name is required');\n    }\n    return { id: crypto.randomUUID(), name: name.trim() };\n  }\n\n  deleteUser(id: string): boolean {\n    if (!id) {\n      throw new Error('ID is required');\n    }\n    return true;\n  }\n}\n",
    )
    .context("writing src/services/user-service.ts")?;
    fs::write(
        src_services.join("untestable-singleton.ts"),
        "export class UntestableSingleton {\n  private static instance: UntestableSingleton;\n\n  private constructor() {}\n\n  static getInstance(): UntestableSingleton {\n    if (!this.instance) {\n      this.instance = new UntestableSingleton();\n    }\n    return this.instance;\n  }\n\n  doWork(): void {}\n}\n",
    )
    .context("writing src/services/untestable-singleton.ts")?;
    fs::write(
        tests_dir.join("UserService.test.ts"),
        "import { UserService } from '../src/services/user-service';\n\ndescribe('UserService', () => {\n  const svc = new UserService();\n\n  it('creates a user with a valid name', () => {\n    const user = svc.createUser('Alice');\n    expect(user.name).toBe('Alice');\n    expect(user.id).toBeDefined();\n  });\n\n  it('throws on empty name', () => {\n    expect(() => svc.createUser('')).toThrow('Name is required');\n  });\n\n  it('trims whitespace from name', () => {\n    const user = svc.createUser('  Bob  ');\n    expect(user.name).toBe('Bob');\n  });\n});\n",
    )
    .context("writing tests/UserService.test.ts")?;
    fs::write(
        coverage_dir.join("lcov.info"),
        "TN:\nSF:src/services/user-service.ts\nFN:2,createUser\nFN:9,deleteUser\nFNDA:3,createUser\nFNDA:0,deleteUser\nFNF:2\nFNH:1\nDA:1,1\nDA:2,3\nDA:3,3\nDA:4,1\nDA:5,2\nDA:6,2\nDA:7,2\nDA:8,0\nDA:9,0\nDA:10,0\nDA:11,0\nDA:12,0\nLF:12\nLH:7\nBRDA:3,0,0,2\nBRDA:3,0,1,1\nBRDA:9,1,0,0\nBRDA:9,1,1,0\nBRF:4\nBRH:2\nend_of_record\n",
    )
    .context("writing coverage/lcov.info")?;
    fs::write(
        coverage_dir.join("user-service.test-scenario.json"),
        serde_json::json!({
            "data": [
                {
                    "files": [
                        {
                            "filename": "src/services/user-service.ts",
                            "segments": [
                                [2, 1, 1, true, true, false],
                                [8, 1, 0, true, true, false],
                                [9, 1, 0, true, true, false]
                            ]
                        }
                    ]
                }
            ]
        })
        .to_string(),
    )
    .context("writing coverage/user-service.test-scenario.json")?;
    fs::write(
        test_results_dir.join("jest-results.json"),
        "{\n  \"testResults\": [\n    {\n      \"name\": \"tests/UserService.test.ts\",\n      \"assertionResults\": [\n        {\n          \"title\": \"creates a user with a valid name\",\n          \"status\": \"passed\",\n          \"ancestorTitles\": [\"UserService\"],\n          \"duration\": 5\n        },\n        {\n          \"title\": \"throws on empty name\",\n          \"status\": \"passed\",\n          \"ancestorTitles\": [\"UserService\"],\n          \"duration\": 2\n        },\n        {\n          \"title\": \"trims whitespace from name\",\n          \"status\": \"passed\",\n          \"ancestorTitles\": [\"UserService\"],\n          \"duration\": 1\n        }\n      ]\n    }\n  ]\n}\n",
    )
    .context("writing test-results/jest-results.json")?;
    fs::write(
        test_results_dir.join("jest-results-fail.json"),
        "{\n  \"testResults\": [\n    {\n      \"name\": \"tests/UserService.test.ts\",\n      \"assertionResults\": [\n        {\n          \"title\": \"creates a user with a valid name\",\n          \"status\": \"passed\",\n          \"ancestorTitles\": [\"UserService\"],\n          \"duration\": 5\n        },\n        {\n          \"title\": \"throws on empty name\",\n          \"status\": \"failed\",\n          \"ancestorTitles\": [\"UserService\"],\n          \"duration\": 3\n        },\n        {\n          \"title\": \"trims whitespace from name\",\n          \"status\": \"passed\",\n          \"ancestorTitles\": [\"UserService\"],\n          \"duration\": 1\n        }\n      ]\n    }\n  ]\n}\n",
    )
    .context("writing test-results/jest-results-fail.json")?;

    Ok(())
}

fn resolve_test_harness_relational_db_path(world: &QatWorld) -> Result<std::path::PathBuf> {
    with_scenario_app_env(world, || {
        let cfg = resolve_store_backend_config_for_repo(world.repo_dir())
            .context("resolving TestHarness store backend config")?;
        resolve_sqlite_db_path_for_repo(world.repo_dir(), cfg.relational.sqlite_path.as_deref())
            .context("resolving TestHarness relational db path")
    })
}

fn load_test_harness_scenario_symbol_ids(world: &QatWorld) -> Result<Vec<String>> {
    let db_path = resolve_test_harness_relational_db_path(world)?;
    let conn = rusqlite::Connection::open(&db_path)
        .with_context(|| format!("opening TestHarness db at {}", db_path.display()))?;
    let mut stmt = conn
        .prepare(
            "SELECT symbol_id \
             FROM test_artefacts_current \
             WHERE canonical_kind = 'test_scenario' \
             ORDER BY name, start_line",
        )
        .context("preparing test scenario lookup query")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .context("querying test scenario symbol ids")?;

    let mut symbol_ids = Vec::new();
    for row in rows {
        symbol_ids.push(row.context("decoding test scenario symbol id")?);
    }

    ensure!(
        !symbol_ids.is_empty(),
        "expected TestHarness ingest-tests to discover at least one test scenario"
    );
    Ok(symbol_ids)
}

pub fn delete_test_file(world: &QatWorld) -> Result<()> {
    let preferred = world.repo_dir().join("tests").join("UserService.test.ts");
    if preferred.exists() {
        fs::remove_file(&preferred).with_context(|| format!("deleting {}", preferred.display()))?;
        return Ok(());
    }

    let tests_dir = world.repo_dir().join("tests");
    if tests_dir.exists() {
        let mut pending = vec![tests_dir];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir)
                .with_context(|| format!("reading test directory {}", dir.display()))?
            {
                let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                let matches_test_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.ends_with(".test.ts")
                            || name.ends_with(".spec.ts")
                            || name.ends_with(".test.rs")
                            || name.ends_with("_test.rs")
                    });
                if !matches_test_name {
                    continue;
                }
                fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;
                return Ok(());
            }
        }
    }

    bail!("no test file found to delete in workspace")
}

pub fn run_testlens_ingest_tests(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let sha = resolve_head_sha(world)?;
    run_testlens_command_strict(
        world,
        &["devql", "test-harness", "ingest-tests", "--commit", &sha],
        "bitloops devql test-harness ingest-tests",
    )
}

pub fn run_testlens_ingest_coverage(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let sha = resolve_head_sha(world)?;
    let tool = if world.repo_dir().join("Cargo.toml").exists() {
        "cargo-test"
    } else {
        "jest"
    };
    run_testlens_command_strict(
        world,
        &[
            "devql",
            "test-harness",
            "ingest-coverage",
            "--lcov",
            "coverage/lcov.info",
            "--commit",
            &sha,
            "--scope",
            "workspace",
            "--tool",
            tool,
        ],
        "bitloops devql test-harness ingest-coverage",
    )?;

    for test_symbol_id in load_test_harness_scenario_symbol_ids(world)? {
        run_testlens_command_strict(
            world,
            &[
                "devql",
                "test-harness",
                "ingest-coverage",
                "--input",
                "coverage/user-service.test-scenario.json",
                "--format",
                "llvm-json",
                "--commit",
                &sha,
                "--scope",
                "test-scenario",
                "--tool",
                tool,
                "--test-artefact-id",
                &test_symbol_id,
            ],
            "bitloops devql test-harness ingest-coverage",
        )?;
    }

    Ok(())
}

fn run_testlens_command_strict(world: &mut QatWorld, args: &[&str], label: &str) -> Result<()> {
    let output = run_command_capture(world, label, build_bitloops_command(world, args)?)
        .with_context(|| format!("running {label}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    world.last_command_stdout = Some(format!("{stdout}\n{stderr}"));
    ensure_success(&output, label)
}

pub fn assert_commit_checkpoints_count(
    world: &QatWorld,
    repo_name: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mappings =
        with_scenario_app_env(world, || read_commit_checkpoint_mappings(world.repo_dir()))
            .context("reading commit-checkpoint mappings")?;
    ensure!(
        mappings.len() >= min_count,
        "expected commit_checkpoints count >= {min_count}, got {}",
        mappings.len()
    );
    Ok(())
}

pub fn run_testlens_ingest_results(
    world: &mut QatWorld,
    repo_name: &str,
    results_file: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let sha = resolve_head_sha(world)?;
    run_bitloops_success(
        world,
        &[
            "devql",
            "test-harness",
            "ingest-results",
            "--jest-json",
            results_file,
            "--commit",
            &sha,
        ],
        "bitloops devql test-harness ingest-results",
    )
}

pub fn run_testlens_query(
    world: &mut QatWorld,
    repo_name: &str,
    artefact: &str,
    view: &str,
    state_scope: &str,
) -> Result<serde_json::Value> {
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, artefact)?;
    let temporal_prefix = match state_scope {
        "current workspace state" => String::new(),
        "latest commit" => {
            let commit_scope = world
                .captured_commit_shas
                .last()
                .cloned()
                .ok_or_else(|| anyhow!("no latest commit SHA captured for TestHarness query"))?;
            format!(r#"->asOf(commit:"{}")"#, escape_devql_string(&commit_scope))
        }
        _ => bail!("unsupported TestHarness state scope `{state_scope}`"),
    };
    let query = match view {
        "summary" | "tests" => format!(
            r#"repo("bitloops"){}->artefacts(symbol_fqn:"{}")->tests()->limit(200)"#,
            temporal_prefix,
            escape_devql_string(&symbol_fqn),
        ),
        "coverage" => format!(
            r#"repo("bitloops"){}->artefacts(symbol_fqn:"{}")->coverage()->limit(200)"#,
            temporal_prefix,
            escape_devql_string(&symbol_fqn),
        ),
        _ => bail!("unsupported TestHarness view `{view}`"),
    };
    let value = run_devql_query(world, &query)?;
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow!("expected TestHarness DevQL query to return a JSON array"))?;
    let row = rows.first();

    let payload = match (view, row) {
        ("summary", Some(row)) => {
            let mut map = serde_json::Map::new();
            if let Some(summary) = row
                .get("tests")
                .and_then(serde_json::Value::as_array)
                .and_then(|entries| entries.first())
                .and_then(|entry| entry.get("summary"))
            {
                let total_covering_tests = summary
                    .get("totalCoveringTests")
                    .or_else(|| summary.get("total_covering_tests"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                map.insert(
                    "summary".to_string(),
                    serde_json::json!({ "total_covering_tests": total_covering_tests }),
                );
                map.insert(
                    "test_count".to_string(),
                    serde_json::json!(total_covering_tests),
                );
            }
            serde_json::Value::Object(map)
        }
        ("tests", Some(row)) => {
            let mut map = serde_json::Map::new();
            let covering_tests = row
                .get("tests")
                .and_then(serde_json::Value::as_array)
                .and_then(|entries| entries.first())
                .and_then(|entry| {
                    entry
                        .get("coveringTests")
                        .or_else(|| entry.get("covering_tests"))
                })
                .and_then(serde_json::Value::as_array)
                .map(|tests| {
                    tests
                        .iter()
                        .map(|test| {
                            let last_run =
                                test.get("last_run").cloned().or_else(|| test.get("lastRun").cloned());

                            let mut normalized = serde_json::Map::new();
                            normalized.insert(
                                "test_name".to_string(),
                                test.get("testName")
                                    .or_else(|| test.get("test_name"))
                                    .cloned()
                                    .unwrap_or_else(|| serde_json::json!("")),
                            );
                            normalized.insert(
                                "classification".to_string(),
                                test.get("classification")
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Null),
                            );
                            normalized.insert(
                                "classification_source".to_string(),
                                test.get("classification_source")
                                    .or_else(|| test.get("classificationSource"))
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Null),
                            );
                            normalized.insert(
                                "fan_out".to_string(),
                                test.get("fan_out")
                                    .or_else(|| test.get("fanOut"))
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Null),
                            );
                            if let Some(last_run) = last_run {
                                normalized.insert("last_run".to_string(), last_run);
                            }
                            serde_json::Value::Object(normalized)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            map.insert(
                "covering_tests".to_string(),
                serde_json::Value::Array(covering_tests),
            );
            serde_json::Value::Object(map)
        }
        ("coverage", Some(row)) => {
            let mut map = serde_json::Map::new();
            if let Some(line_coverage_pct) = row
                .get("coverage")
                .and_then(serde_json::Value::as_array)
                .and_then(|entries| entries.first())
                .and_then(|entry| entry.get("coverage"))
                .and_then(|coverage| {
                    coverage
                        .get("lineCoveragePct")
                        .or_else(|| coverage.get("line_coverage_pct"))
                })
                .and_then(serde_json::Value::as_f64)
            {
                map.insert(
                    "coverage".to_string(),
                    serde_json::json!({ "line_coverage_pct": line_coverage_pct }),
                );
            }
            serde_json::Value::Object(map)
        }
        _ => serde_json::json!({}),
    };

    world.last_command_stdout =
        Some(serde_json::to_string(&payload).context("serializing normalized TestHarness payload")?);
    Ok(payload)
}

fn testlens_payload_is_empty_or_zero(value: &serde_json::Value) -> bool {
    let summary_zero = value
        .get("summary")
        .and_then(|summary| summary.get("total_covering_tests"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|count| count == 0);
    let tests_empty = value
        .get("covering_tests")
        .and_then(serde_json::Value::as_array)
        .is_some_and(std::vec::Vec::is_empty);
    let payload_count = count_testlens_payload_rows(value);
    summary_zero || tests_empty || payload_count == 0
}

fn run_testlens_query_eventually(
    world: &mut QatWorld,
    repo_name: &str,
    artefact: &str,
    view: &str,
    state_scope: &str,
    expected: &str,
    condition: impl Fn(&serde_json::Value) -> bool,
) -> Result<serde_json::Value> {
    let timeout = parse_timeout_seconds(
        std::env::var(TESTLENS_EVENTUAL_TIMEOUT_ENV).ok().as_deref(),
        DEFAULT_TESTLENS_EVENTUAL_TIMEOUT_SECS,
    );
    let started = Instant::now();
    let mut attempts = 0_usize;
    let mut last_value = serde_json::json!({});

    loop {
        attempts += 1;
        let value = run_testlens_query(world, repo_name, artefact, view, state_scope)?;
        if condition(&value) {
            return Ok(value);
        }
        last_value = value;
        if started.elapsed() >= timeout {
            let last_payload = serde_json::to_string(&last_value)
                .unwrap_or_else(|_| "<failed to serialize payload>".to_string());
            bail!(
                "timed out after {}s waiting for TestHarness query ({artefact}, {view}) to {expected}; attempts={attempts}; last payload={last_payload}",
                timeout.as_secs()
            );
        }
        std::thread::sleep(StdDuration::from_millis(
            TESTLENS_EVENTUAL_POLL_INTERVAL_MILLIS,
        ));
    }
}

pub fn assert_testlens_query_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
    artefact: &str,
    view: &str,
    state_scope: &str,
) -> Result<()> {
    let value = run_testlens_query_eventually(
        world,
        repo_name,
        artefact,
        view,
        state_scope,
        "return results",
        |value| count_testlens_payload_rows(value) >= 1,
    )?;
    let count = count_testlens_payload_rows(&value);
    ensure!(
        count >= 1,
        "expected TestHarness query ({artefact}, {view}) to return results, got {count}"
    );
    Ok(())
}

pub fn assert_testlens_summary_nonzero(world: &QatWorld) -> Result<()> {
    let value = parse_last_command_stdout_json(world)?;
    let total = value
        .get("summary")
        .and_then(|summary| summary.get("total_covering_tests"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| value.get("test_count").and_then(serde_json::Value::as_u64))
        .unwrap_or(0);
    ensure!(total > 0, "expected non-zero test count, got {total}");
    Ok(())
}

pub fn assert_testlens_tests_have_classification(world: &QatWorld) -> Result<()> {
    let value = parse_last_command_stdout_json(world)?;
    let tests = value
        .get("covering_tests")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("expected covering_tests array in TestHarness response"))?;
    ensure!(!tests.is_empty(), "expected at least one covering test");
    let has_classification = tests.iter().any(|test| {
        test.get("classification")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    });
    ensure!(
        has_classification,
        "expected at least one covering test with a classification"
    );
    Ok(())
}

pub fn assert_testlens_coverage_has_line_pct(world: &QatWorld) -> Result<()> {
    let value = parse_last_command_stdout_json(world)?;
    let line_pct = value
        .get("coverage")
        .and_then(|coverage| coverage.get("line_coverage_pct"))
        .and_then(serde_json::Value::as_f64);
    ensure!(
        line_pct.is_some(),
        "expected coverage response with numeric line_coverage_pct"
    );
    Ok(())
}

pub fn assert_testlens_query_empty_or_zero(
    world: &mut QatWorld,
    repo_name: &str,
    artefact: &str,
    view: &str,
    state_scope: &str,
) -> Result<()> {
    let value = run_testlens_query_eventually(
        world,
        repo_name,
        artefact,
        view,
        state_scope,
        "become empty or zero-count",
        testlens_payload_is_empty_or_zero,
    )?;
    let payload_count = count_testlens_payload_rows(&value);
    ensure!(
        testlens_payload_is_empty_or_zero(&value),
        "expected empty or zero-count TestHarness payload for `{artefact}`, got payload_count={payload_count}"
    );
    Ok(())
}

pub fn assert_testlens_includes_failing_test(
    world: &mut QatWorld,
    repo_name: &str,
    artefact: &str,
    view: &str,
    state_scope: &str,
) -> Result<()> {
    let value = run_testlens_query(world, repo_name, artefact, view, state_scope)?;
    let tests = value
        .get("covering_tests")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("expected covering_tests array in TestHarness response"))?;
    let has_failing = tests.iter().any(|test| {
        test.get("last_run")
            .and_then(|run| run.get("status"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| status == "fail" || status == "failed")
    });
    ensure!(
        has_failing,
        "expected at least one failing test in TestHarness query output"
    );
    Ok(())
}
