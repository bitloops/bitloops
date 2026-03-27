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
        "import { User } from '../models/user';\nimport { UserRepository } from '../repository/user-repository';\n\nexport function createUser(name: string, repo: UserRepository): User {\n  const user: User = { id: crypto.randomUUID(), name };\n  repo.save(user);\n  return user;\n}\n",
    )
    .context("writing src/services/user-service.ts")?;
    fs::write(
        src.join("controllers").join("user-controller.ts"),
        "import { createUser } from '../services/user-service';\nimport { UserRepository } from '../repository/user-repository';\n\nexport function handleCreate(name: string): string {\n  const repo = new UserRepository();\n  const user = createUser(name, repo);\n  return user.id;\n}\n",
    )
    .context("writing src/controllers/user-controller.ts")?;
    fs::write(
        src.join("index.ts"),
        "import { handleCreate } from './controllers/user-controller';\n\nconsole.log(handleCreate('Alice'));\n",
    )
    .context("writing src/index.ts")?;

    Ok(())
}

pub fn add_new_caller_of_symbol(world: &mut QatWorld, symbol_alias: &str) -> Result<()> {
    let parts: Vec<&str> = symbol_alias.split('.').collect();
    let (service_name, method_name) = match parts.as_slice() {
        [service_name, method_name] => (*service_name, *method_name),
        _ => bail!("expected symbol alias in Class.method format, got `{symbol_alias}`"),
    };

    let import_path = format!("./services/{}", to_kebab_case(service_name));
    let file_path = world.repo_dir().join("src").join("new-caller.ts");
    let content = format!(
        "import {{ {method_name} }} from '{import_path}';\nimport {{ UserRepository }} from './repository/user-repository';\n\nexport function callCreateUser(): void {{\n  const repo = new UserRepository();\n  {method_name}('QAT-new-caller', repo);\n}}\n\ncallCreateUser();\n"
    );
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
    let query = format!(
        r#"repo("bitloops")->artefacts(symbol_fqn:"{}")->deps(kind:"calls",direction:"{}")->limit(50)"#,
        escape_devql_string(&symbol_fqn),
        escape_devql_string(direction)
    );
    let value = run_devql_query(world, &query)?;
    let count = count_json_array_rows(&value);
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
    let query = format!(
        r#"repo("bitloops")->asOf(commit:"{}")->artefacts(symbol_fqn:"{}")->deps(kind:"calls",direction:"{}")->limit(50)"#,
        escape_devql_string(commit_sha),
        escape_devql_string(&symbol_fqn),
        escape_devql_string(direction)
    );
    let value = run_devql_query(world, &query)?;
    let count = count_json_array_rows(&value);
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
    let query = format!(
        r#"repo("bitloops")->asOf(commit:"{}")->artefacts(symbol_fqn:"{}")->deps(kind:"calls",direction:"{}")->limit(50)"#,
        escape_devql_string(commit_sha),
        escape_devql_string(&symbol_fqn),
        escape_devql_string(direction)
    );
    let value = run_devql_query(world, &query)?;
    let count = count_json_array_rows(&value);
    world.last_query_result_count = Some(count);
    ensure!(
        count == expected_count,
        "expected exactly {expected_count} deps({direction}) rows for `{symbol_alias}` asOf `{commit_sha}`, got {count}"
    );
    Ok(())
}

pub fn assert_devql_artefacts_count_stable(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let first = run_devql_query(world, r#"repo("bitloops")->artefacts()->limit(500)"#)?;
    let count_first = count_json_array_rows(&first);
    run_devql_ingest_for_repo(world, repo_name)?;
    let second = run_devql_query(world, r#"repo("bitloops")->artefacts()->limit(500)"#)?;
    let count_second = count_json_array_rows(&second);
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

pub fn run_testlens_ingest_tests(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let sha = resolve_head_sha(world)?;
    run_bitloops_success(
        world,
        &["testlens", "ingest-tests", "--commit", &sha],
        "bitloops testlens ingest-tests",
    )
}

pub fn run_testlens_ingest_coverage(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let sha = resolve_head_sha(world)?;
    run_bitloops_success(
        world,
        &[
            "testlens",
            "ingest-coverage",
            "--lcov",
            "coverage/lcov.info",
            "--commit",
            &sha,
            "--scope",
            "workspace",
            "--tool",
            "jest",
        ],
        "bitloops testlens ingest-coverage",
    )
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
            "testlens",
            "ingest-results",
            "--jest-json",
            results_file,
            "--commit",
            &sha,
        ],
        "bitloops testlens ingest-results",
    )
}

pub fn run_testlens_query(
    world: &mut QatWorld,
    repo_name: &str,
    artefact: &str,
    view: &str,
) -> Result<serde_json::Value> {
    ensure_bitloops_repo_name(repo_name)?;
    let sha = resolve_head_sha(world)?;
    let output = run_command_capture(
        world,
        "bitloops testlens query",
        build_bitloops_command(
            world,
            &[
                "testlens",
                "query",
                "--artefact",
                artefact,
                "--commit",
                &sha,
                "--view",
                view,
            ],
        )?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    ensure_success(&output, "bitloops testlens query")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    serde_json::from_str(stdout.trim()).context("parsing testlens query json output")
}

pub fn assert_testlens_query_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
    artefact: &str,
    view: &str,
) -> Result<()> {
    let value = run_testlens_query(world, repo_name, artefact, view)?;
    let count = count_testlens_payload_rows(&value);
    ensure!(
        count >= 1,
        "expected testlens query ({artefact}, {view}) to return results, got {count}"
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
        .ok_or_else(|| anyhow!("expected covering_tests array in testlens response"))?;
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
) -> Result<()> {
    let value = run_testlens_query(world, repo_name, artefact, view)?;
    let summary_zero = value
        .get("summary")
        .and_then(|summary| summary.get("total_covering_tests"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|value| value == 0);
    let tests_empty = value
        .get("covering_tests")
        .and_then(serde_json::Value::as_array)
        .is_some_and(std::vec::Vec::is_empty);
    let payload_count = count_testlens_payload_rows(&value);
    ensure!(
        summary_zero || tests_empty || payload_count == 0,
        "expected empty or zero-count testlens payload for `{artefact}`, got payload_count={payload_count}"
    );
    Ok(())
}

pub fn assert_testlens_includes_failing_test(
    world: &mut QatWorld,
    repo_name: &str,
    artefact: &str,
    view: &str,
) -> Result<()> {
    let value = run_testlens_query(world, repo_name, artefact, view)?;
    let tests = value
        .get("covering_tests")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("expected covering_tests array in testlens response"))?;
    let has_failing = tests.iter().any(|test| {
        test.get("last_run")
            .and_then(|run| run.get("status"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| status == "fail" || status == "failed")
    });
    ensure!(
        has_failing,
        "expected at least one failing test in testlens query output"
    );
    Ok(())
}

