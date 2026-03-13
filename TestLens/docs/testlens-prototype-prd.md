# TestLens Prototype — PRD

## 1. Overview

### What is TestLens?

TestLens is a CLI tool that gives LLM agents (and developers) a structured view of the verification landscape around any code artefact. Before modifying a function, an agent can ask: "what tests cover this, what's tested, what's not?"

### Prototype goal

Build a working end-to-end prototype that validates the core ingestion pipeline and query flow against a real TypeScript + Jest codebase. The prototype parses real test files via Tree-sitter, ingests real LCOV coverage data and Jest test results, computes test-to-artefact linkage and classification, and returns structured test harness responses via a CLI.

### What this prototype proves

- Tree-sitter can parse test files to discover test suites/scenarios and link them to production artefacts.
- LCOV coverage data can be reliably joined against an artefact model to produce artefact-level branch coverage.
- Jest JSON output can be parsed for per-test pass/fail/skip status and duration.
- Coverage-derived test classification (unit/integration/e2e) works based on artefact fan-out.
- The query response shape is useful and correct for agent consumption.
- The data model is sound before investing in PostgreSQL and the full DevQL engine.

### What this prototype does NOT do

- No production artefact extraction. Production artefacts are prepopulated in SQLite via a seed script (`scripts/init-fixture-db.sh`). In the full product, this is handled by the `getBlastRadius` / Static Analysis pipeline.
- No DevQL query engine. The CLI accepts simplified query parameters directly.
- No PostgreSQL. SQLite only.
- No Rust language support. TypeScript + Jest only.
- No convention-based classification overrides.
- No uncommitted workspace state.
- No blast radius composition.

---

## 2. Tech Stack

| Component           | Choice                           | Rationale                                                                            |
| ------------------- | -------------------------------- | ------------------------------------------------------------------------------------ |
| Language            | Rust                             | Target language for the full product. Prototype validates Rust CLI feasibility.      |
| Database            | SQLite                           | Zero infrastructure. Single file. Production artefacts prepopulated via seed script. |
| AST parsing         | Tree-sitter (TypeScript grammar) | Parses test files to discover suites, scenarios, imports, and call sites.            |
| Coverage format     | LCOV                             | Jest supports LCOV natively via Istanbul/nyc.                                        |
| Test results format | Jest JSON reporter               | `jest --json` produces per-test pass/fail/skip status and duration.                  |
| Test codebase       | Small TypeScript repo with Jest  | Real coverage data, real test structures.                                            |

---

## 3. Responsibility Boundary

### What TestLens owns

```
┌─────────────────────────────────────────────┐
│ TestLens Prototype                          │
│                                             │
│  ingest-tests   → Tree-sitter parse of      │
│                   test files, discover       │
│                   suites/scenarios, link     │
│                   to production artefacts    │
│                                             │
│  ingest-coverage → LCOV parse, artefact-    │
│                    level coverage join,      │
│                    classification, scoring   │
│                                             │
│  ingest-results  → Jest JSON parse,         │
│                    per-test pass/fail/skip   │
│                                             │
│  query           → Structured JSON output   │
│                    for agent consumption     │
└─────────────────────────────────────────────┘
```

### What TestLens does NOT own (preconditions)

- **Production artefacts in SQLite** — seeded by `scripts/init-fixture-db.sh` before TestLens runs. This script manually populates the `artefacts` table with all production code entities (functions, classes, methods, interfaces, types, constants) from the fixture TypeScript repo.
- **Running tests** — the developer/agent runs `jest --coverage --json` separately. TestLens ingests the outputs.

---

## 4. Data Model

### 4.1 Artefacts table (prepopulated by seed script)

Represents semantic units of the codebase. Prepopulated for production code; extended by TestLens for test artefacts during `ingest-tests`.

```sql
CREATE TABLE artefacts (
  artefact_id        TEXT PRIMARY KEY,
  symbol_id          TEXT,
  repo_id            TEXT NOT NULL,
  blob_sha           TEXT,
  commit_sha         TEXT NOT NULL,
  path               TEXT NOT NULL,
  language           TEXT NOT NULL,
  canonical_kind     TEXT NOT NULL,
  language_kind      TEXT,
  symbol_fqn         TEXT,
  parent_artefact_id TEXT,
  start_line         INTEGER NOT NULL,
  end_line           INTEGER NOT NULL,
  start_byte         INTEGER,
  end_byte           INTEGER,
  signature          TEXT,
  content_hash       TEXT
);
```

**`canonical_kind` values:**

| Kind            | Description           | Seeded by      | Example                              |
| --------------- | --------------------- | -------------- | ------------------------------------ |
| `file`          | Source file           | Seed script    | `src/services/user.ts`               |
| `function`      | Standalone function   | Seed script    | `export function validateEmail(...)` |
| `class`         | Class declaration     | Seed script    | `export class UserRepository`        |
| `method`        | Method within a class | Seed script    | `UserRepository.findById`            |
| `constant`      | Constant declaration  | Seed script    | `export const MAX_RETRIES = 3`       |
| `type`          | Type alias            | Seed script    | `export type UserId = string`        |
| `interface`     | Interface declaration | Seed script    | `export interface UserDTO { ... }`   |
| `test_suite`    | Test grouping         | `ingest-tests` | `describe('UserRepository', ...)`    |
| `test_scenario` | Individual test case  | `ingest-tests` | `it('should find user by id', ...)`  |

### 4.2 Test linkages table (populated by `ingest-tests`)

Static linkage between test scenarios and production artefacts, discovered by Tree-sitter parsing of test file imports and call sites.

```sql
CREATE TABLE test_links (
  test_link_id           TEXT PRIMARY KEY,
  test_artefact_id       TEXT NOT NULL REFERENCES artefacts(artefact_id),
  production_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  link_source            TEXT NOT NULL DEFAULT 'static_analysis',
  commit_sha             TEXT NOT NULL
);
```

### 4.3 Test coverage table (populated by `ingest-coverage`)

Per-branch coverage data joined against artefact spans.

```sql
CREATE TABLE test_coverage (
  coverage_id      TEXT PRIMARY KEY,
  repo_id          TEXT NOT NULL,
  commit_sha       TEXT NOT NULL,
  test_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  artefact_id      TEXT NOT NULL REFERENCES artefacts(artefact_id),
  line             INTEGER NOT NULL,
  branch_id        INTEGER,
  covered          BOOLEAN NOT NULL,
  hit_count        INTEGER DEFAULT 0
);
```

### 4.4 Test runs table (populated by `ingest-results`)

Test execution outcomes parsed from Jest JSON output.

```sql
CREATE TABLE test_runs (
  run_id           TEXT PRIMARY KEY,
  repo_id          TEXT NOT NULL,
  commit_sha       TEXT NOT NULL,
  test_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  status           TEXT NOT NULL,  -- pass | fail | skip
  duration_ms      INTEGER,
  ran_at           TEXT NOT NULL   -- ISO 8601 timestamp
);
```

### 4.5 Test classification table (computed by `ingest-coverage`)

Derived classification based on coverage fan-out.

```sql
CREATE TABLE test_classifications (
  classification_id   TEXT PRIMARY KEY,
  test_artefact_id    TEXT NOT NULL REFERENCES artefacts(artefact_id),
  commit_sha          TEXT NOT NULL,
  classification      TEXT NOT NULL,  -- unit | integration | e2e
  classification_source TEXT NOT NULL DEFAULT 'coverage_derived',
  fan_out             INTEGER NOT NULL,
  boundary_crossings  INTEGER NOT NULL DEFAULT 0
);
```

---

## 5. CLI Interface

### 5.1 Commands

```bash
# Parse test files, discover test suites/scenarios, link to production artefacts
testlens ingest-tests --repo-dir ./testlens-fixture --commit <sha>

# Ingest LCOV coverage report, compute classification and scoring
testlens ingest-coverage --lcov ./coverage/lcov.info --commit <sha>

# Ingest Jest JSON test results (pass/fail/skip per test)
testlens ingest-results --jest-json ./test-results.json --commit <sha>

# Query test harness for an artefact
testlens query --artefact "UserRepository.findById" --commit <sha>

# Query with optional classification filter
testlens query --artefact "UserRepository.findById" --commit <sha> --classification unit

# List all artefacts (debug/exploration)
testlens list --commit <sha>
```

**Typical workflow:**

```bash
# 1. Seed production artefacts (one-time setup)
./scripts/init-fixture-db.sh

# 2. Run tests with coverage and JSON output
cd testlens-fixture
npx jest --coverage --json --outputFile=../test-results.json

# 3. Ingest everything
testlens ingest-tests --repo-dir ./testlens-fixture --commit abc123
testlens ingest-coverage --lcov ./testlens-fixture/coverage/lcov.info --commit abc123
testlens ingest-results --jest-json ./test-results.json --commit abc123

# 4. Query
testlens query --artefact "UserRepository.findById" --commit abc123
```

### 5.2 Query output

The `query` command returns JSON to stdout. This is the same response shape an agent would consume.

```json
{
  "artefact": {
    "artefact_id": "a1b2c3d4-...",
    "name": "findById",
    "kind": "method",
    "file_path": "src/repositories/UserRepository.ts",
    "start_line": 42,
    "end_line": 67
  },

  "covering_tests": [
    {
      "test_id": "t1a2b3c4-...",
      "test_name": "should find user by id",
      "suite_name": "UserRepository",
      "file_path": "tests/UserRepository.test.ts",
      "classification": "unit",
      "classification_source": "coverage_derived",
      "confidence": 0.95,
      "strength": 0.88,
      "last_run": {
        "status": "pass",
        "duration_ms": 12,
        "commit_sha": "abc123"
      }
    }
  ],

  "coverage": {
    "line_coverage_pct": 92.3,
    "branch_coverage_pct": 75.0,
    "branches": [
      {
        "line": 45,
        "description": "if (id === null)",
        "covered": true,
        "covering_test_ids": ["t1a2b3c4-..."]
      },
      {
        "line": 52,
        "description": "else branch: user not found",
        "covered": false,
        "covering_test_ids": []
      }
    ]
  },

  "summary": {
    "verification_level": "well_tested",
    "total_covering_tests": 1,
    "unit_count": 1,
    "integration_count": 0,
    "e2e_count": 0,
    "untested_branch_count": 1
  }
}
```

---

## 6. Ingestion Pipelines

### 6.1 `ingest-tests` — Tree-sitter Test File Parsing

**Input:** Path to repo directory + commit SHA.

**Process:**

1. Scan for test files (files matching `*.test.ts`, `*.spec.ts`, or within `__tests__/` directories).
2. Parse each test file with Tree-sitter (TypeScript grammar).
3. Extract `describe` blocks → create `test_suite` artefacts with `start_line`/`end_line`.
4. Extract `it`/`test` blocks → create `test_scenario` artefacts nested under their parent suite.
5. Extract `import` statements → resolve which production modules are imported.
6. Extract function call sites within test bodies → match against known production artefact `symbol_fqn` values in the DB.
7. Create `test_links` entries for each test_scenario → production_artefact pair.

**Output:** `artefacts` table extended with test suites and scenarios. `test_links` table populated.

### 6.2 `ingest-coverage` — LCOV Parsing + Classification

**Input:** Path to LCOV file + commit SHA.

**Process:**

1. Parse LCOV file: extract per-file records (`SF:`), line hits (`DA:`), branch data (`BRDA:`).
2. For each file in LCOV, find matching production artefacts by `path` and `start_line`/`end_line` overlap.
3. For each artefact span, compute line coverage and branch coverage.
4. Store per-line and per-branch results in `test_coverage`.
5. For each test scenario (from `test_links`), compute fan-out: count of distinct production artefacts covered.
6. Count boundary crossings: distinct parent directories of covered artefacts.
7. Apply classification thresholds → store in `test_classifications`.
8. Compute confidence and strength scores.

**Output:** `test_coverage` and `test_classifications` tables populated.

### 6.3 `ingest-results` — Jest JSON Parsing

**Input:** Path to Jest JSON output file + commit SHA.

**Process:**

1. Parse Jest JSON output (produced by `jest --json`).
2. For each test result, match to a `test_scenario` artefact by test name + suite name + file path.
3. Extract status (`passed`/`failed`/`pending` → `pass`/`fail`/`skip`), duration, and timestamp.
4. Store in `test_runs`.

**Output:** `test_runs` table populated.

---

## 7. Scoring Logic

### 7.1 Confidence

How sure are we this test covers this artefact?

| Linkage type                                                  | Confidence |
| ------------------------------------------------------------- | ---------- |
| Coverage-verified (test hit lines within artefact span)       | 0.9–1.0    |
| Static linkage + coverage exists but didn't hit this artefact | 0.3–0.5    |
| Static linkage only (no coverage data ingested)               | 0.5–0.7    |

### 7.2 Strength

How meaningful is this test as verification of this specific artefact?

```
strength = (1.0 / fan_out) * classification_weight
```

Where `classification_weight`:

| Classification | Weight |
| -------------- | ------ |
| unit           | 1.0    |
| integration    | 0.7    |
| e2e            | 0.4    |

A unit test covering 1 artefact scores ~1.0. An e2e test covering 30 artefacts scores ~0.01.

### 7.3 Classification (coverage-derived)

Based on fan-out and boundary crossings:

| Fan-out | Boundary crossings | Classification |
| ------- | ------------------ | -------------- |
| 1–3     | 0–1                | `unit`         |
| 4–10    | 1–3                | `integration`  |
| 11+     | 3+                 | `e2e`          |

These are initial heuristics to be tuned against the fixture repo.

### 7.4 Verification level

| Condition                                       | Level              |
| ----------------------------------------------- | ------------------ |
| No covering tests                               | `untested`         |
| Covering tests exist but branch coverage < 50%  | `partially_tested` |
| Covering tests exist and branch coverage >= 50% | `well_tested`      |

---

## 8. Test TypeScript Fixture Repo

### Requirements

A small but realistic TypeScript project with Jest that exercises enough patterns to validate the ingestion pipeline:

- At least 3–4 production source files with functions, classes, methods, interfaces, types, and constants.
- At least 2–3 test files with `describe`/`it` blocks.
- Tests that vary in scope: some focused unit tests (covering 1 artefact), some broader tests (covering multiple artefacts across directories).
- At least one untested production artefact (to validate the `untested` path).
- At least one failing test (to validate pre-existing failure detection via Jest JSON).
- Jest configured with `--coverage` (LCOV output) and `--json` (test results output).

### Suggested structure

```
testlens-fixture/
  src/
    models/
      User.ts            -- interface User, type UserId, const MAX_NAME_LENGTH
    repositories/
      UserRepository.ts  -- class UserRepository { findById, findByEmail, delete }
    services/
      UserService.ts     -- class UserService { createUser, getUser, deleteUser }
      AuthService.ts     -- function validateToken, function hashPassword (untested)
  tests/
    UserRepository.test.ts  -- focused unit tests for UserRepository methods
    UserService.test.ts     -- integration-style tests touching UserService + UserRepository
    e2e/
      userFlow.test.ts      -- broad test touching models + repo + service
  jest.config.ts
  package.json
  tsconfig.json
```

---

## 9. Implementation Plan

### Step 1: Scaffold Rust CLI project

- Create Rust binary with `clap` for argument parsing.
- Subcommands: `ingest-tests`, `ingest-coverage`, `ingest-results`, `query`, `list`.
- SQLite via `rusqlite`.
- Assume DB already exists with schema and production artefacts seeded.

### Step 2: Build test TypeScript fixture repo

- Create the fixture repo with the suggested structure.
- Write focused and broad tests.
- Include at least one untested artefact (`AuthService.hashPassword`) and one failing test.
- Configure Jest for LCOV coverage and JSON output.
- Create `scripts/init-fixture-db.sh` that creates the SQLite DB, creates all tables, and inserts production artefact records.

### Step 3: Implement `ingest-tests` (Tree-sitter)

- Integrate Tree-sitter with TypeScript grammar in Rust.
- Scan for test files by naming convention.
- Parse `describe` and `it`/`test` blocks → insert `test_suite` and `test_scenario` artefacts.
- Parse `import` statements → resolve to production modules.
- Parse function calls within test bodies → match to production artefact `symbol_fqn`.
- Insert `test_links`.

### Step 4: Implement `ingest-coverage` (LCOV parser)

- Parse LCOV format: `SF:`, `DA:`, `BRDA:`, `end_of_record`.
- Map line/branch hits to artefact spans using `path` + `start_line`/`end_line` overlap.
- Store in `test_coverage`.
- Compute fan-out per test scenario.
- Count boundary crossings.
- Apply classification thresholds → store in `test_classifications`.
- Compute confidence and strength scores.

### Step 5: Implement `ingest-results` (Jest JSON parser)

- Parse Jest JSON output structure.
- Match test results to `test_scenario` artefacts by name + suite + file path.
- Map Jest statuses (`passed`/`failed`/`pending`) to `pass`/`fail`/`skip`.
- Store in `test_runs`.

### Step 6: Implement `query` command

- Accept `--artefact` (name or fqn), `--commit`, optional `--classification`.
- Join `artefacts` → `test_links` → `test_coverage` → `test_classifications` → `test_runs`.
- Assemble response JSON matching the defined schema.
- Output to stdout.

### Step 7: Implement `list` command

- List all artefacts for a commit, optionally filtered by `canonical_kind`.
- Useful for debugging and exploring what's in the DB.

### Step 8: End-to-end validation

- Run `scripts/init-fixture-db.sh`.
- Run Jest with coverage and JSON output on the fixture repo.
- Run all three `ingest` commands.
- Run `testlens query` for various artefacts.
- Validate JSON output against BDD scenarios.

---

## 10. BDD Scenarios

### Happy paths

**S1 – Query test harness with static linkage only (before coverage ingestion)**
Given production artefacts are seeded in SQLite
And `testlens ingest-tests` has been run
And no coverage or results have been ingested
When `testlens query --artefact "UserRepository.findById" --commit abc123`
Then output JSON contains covering tests discovered by Tree-sitter
And each test has `confidence` in the 0.5–0.7 range (static-only)
And `coverage` section is null
And `summary.verification_level` is `partially_tested`

**S2 – Query test harness after full ingestion**
Given all three ingestion commands have been run
And artefact `UserRepository.findById` spans lines 42–67
And LCOV shows tests hitting lines 42–60 but not 61–67
When `testlens query --artefact "UserRepository.findById" --commit abc123`
Then covering tests have `confidence` >= 0.9
And `coverage.branches` lists covered and uncovered branches within lines 42–67
And `summary.untested_branch_count` > 0

**S3 – Query for untested artefact**
Given `AuthService.hashPassword` exists in production artefacts
And no tests import or call it
When `testlens query --artefact "AuthService.hashPassword" --commit abc123`
Then `covering_tests` is empty
And `summary.verification_level` is `untested`

**S4 – Coverage-derived classification**
Given test `UserRepository.test.ts > should find user by id` covers 1 artefact (fan-out: 1)
And test `userFlow.test.ts > full user creation flow` covers 8+ artefacts across 3 directories
When coverage is ingested and classification computed
Then the first test has `classification: unit` and high `strength`
And the second test has `classification: integration` or `e2e` and low `strength`

**S5 – Pre-existing test failure visible via Jest JSON**
Given Jest JSON output contains a failing test `UserService.test.ts > should reject duplicate email`
And `ingest-results` has been run
When `testlens query --artefact "UserService.createUser" --commit abc123`
Then the failing test appears with `last_run.status: fail` and `duration_ms` populated

**S6 – Filter by classification**
Given artefact `UserRepository.findById` is covered by 2 unit tests and 1 integration test
When `testlens query --artefact "UserRepository.findById" --commit abc123 --classification unit`
Then only the 2 unit tests are returned

### Edge cases

**E1 – Test file imports module but Tree-sitter resolves at call-site level**
Given a test file imports `UserRepository`
But only calls `findById`, not `findByEmail` or `delete`
When `testlens ingest-tests` runs
Then `test_links` are created for `findById` only
And querying for `delete` does not return this test

**E2 – Coverage scoped to artefact span**
Given `UserRepository.ts` contains `findById` (lines 42–67) and `findByEmail` (lines 70–95)
And a test hits lines across both functions
When querying for `findById`
Then `coverage.branches` only includes branches within lines 42–67

**E3 – Artefact not found**
When `testlens query --artefact "NonExistent.method" --commit abc123`
Then CLI exits with error: "Artefact not found"

**E4 – No database found**
When `testlens query --artefact "anything" --commit abc123` and no DB exists
Then CLI exits with error: "Database not found. Run init-fixture-db.sh first."

**E5 – Jest JSON test name doesn't match any test_scenario artefact**
Given Jest JSON contains a result for a test not discovered by `ingest-tests`
When `testlens ingest-results` runs
Then a warning is logged for the unmatched test
And ingestion continues for remaining tests

---

## 11. Success Criteria

The prototype is successful when:

1. `testlens ingest-tests` correctly discovers test suites, scenarios, and linkages from real Jest test files via Tree-sitter.
2. `testlens ingest-coverage` correctly parses a real Jest LCOV report and produces artefact-level coverage in SQLite.
3. `testlens ingest-results` correctly parses Jest JSON output and stores per-test pass/fail/skip status.
4. `testlens query` returns a correct, complete JSON response for any artefact.
5. Coverage-derived classification produces reasonable unit/integration/e2e labels on the fixture repo.
6. Untested artefacts and untested branches are correctly identified.
7. Pre-existing test failures are visible in the response.
8. All BDD scenarios (S1–S6, E1–E5) pass.
