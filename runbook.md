# Test-Harness Runbook

Validated on 2026-03-19.

This runbook proves the end-to-end flow that matters now:

1. initialize a repo for `claude-code`
2. create a committed Bitloops checkpoint by making production-code changes through a Claude session
3. run `bitloops devql ingest` and verify the production artefact exists
4. create a second committed Claude checkpoint that adds tests and also touches the production file
5. run `bitloops devql ingest` again
6. run `bitloops testlens ingest-tests`
7. verify that test links were created

It uses the curated Rust fixture under `bitloops_cli/tests/fixtures/testlens-fixture-rust`, because that gives a deterministic proof with real production/test structure and non-zero links.

## Preconditions

```bash
cd /Users/markos/code/bitloops/bitloops
cargo install --path ./bitloops_cli --force
```

## 1) Create a disposable repo from the Rust fixture

```bash
REPO=/tmp/test-harness-claude-proof
FIXTURE=/Users/markos/code/bitloops/bitloops/bitloops_cli/tests/fixtures/testlens-fixture-rust

rm -rf "$REPO"
mkdir -p "$REPO"
cp -R "$FIXTURE"/. "$REPO"/
rm -rf "$REPO/target" "$REPO/.bitloops" "$REPO/.git"

cd "$REPO"
git init
git branch -M main
git config user.name "Codex"
git config user.email "codex@example.com"
git add .
git commit -m "Baseline fixture"
```

## 2) Initialize Bitloops and DevQL

```bash
cd /tmp/test-harness-claude-proof

bitloops init --agent claude-code
bitloops enable --project

BITLOOPS_DEVQL_EMBEDDING_PROVIDER=none \
BITLOOPS_DEVQL_SEMANTIC_PROVIDER=none \
bitloops devql init
```

## 3) Claude session 1: add production code and commit it

This creates the first committed checkpoint and gives DevQL a production symbol to ingest.

```bash
cd /tmp/test-harness-claude-proof

export SESSION_ID="claude-prod-session-1"
export TRANSCRIPT_PATH="$PWD/claude-prod-1.jsonl"

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks claude-code session-start

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\",\"prompt\":\"Add a repository helper that checks whether an email belongs to a domain\"}" \
  | bitloops hooks claude-code user-prompt-submit

python3 - <<'PY'
from pathlib import Path

path = Path("src/repositories/user_repository.rs")
text = path.read_text()
needle = """    pub fn find_by_email(&self, email: &str) -> Option<User> {
        self.users
            .iter()
            .find(|user| user.email.eq_ignore_ascii_case(email))
            .cloned()
    }
"""
replacement = needle + """
    pub fn has_email_domain(&self, email: &str, domain: &str) -> bool {
        let suffix = format!("@{}", domain).to_ascii_lowercase();
        self.find_by_email(email)
            .map(|user| user.email.to_ascii_lowercase().ends_with(&suffix))
            .unwrap_or(false)
    }
"""
path.write_text(text.replace(needle, replacement))
Path("claude-prod-1.jsonl").write_text("")
PY

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks claude-code stop

git add src/repositories/user_repository.rs claude-prod-1.jsonl
git commit -m "Add email-domain repository helper"

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks claude-code session-end
```

Verify the checkpoint exists before ingest:

```bash
sqlite3 ./.bitloops/stores/relational/relational.db "
select 'checkpoints', count(*) from checkpoints
union all
select 'commit_checkpoints', count(*) from commit_checkpoints;
"
```

## 4) Ingest production artefacts and verify the symbol exists

```bash
cd /tmp/test-harness-claude-proof

export COMMIT_A="$(git rev-parse HEAD)"

BITLOOPS_DEVQL_EMBEDDING_PROVIDER=none \
BITLOOPS_DEVQL_SEMANTIC_PROVIDER=none \
bitloops devql ingest

sqlite3 ./.bitloops/stores/relational/relational.db "
select symbol_fqn, path, coalesce(canonical_kind, language_kind)
from artefacts_current
where commit_sha = '$COMMIT_A'
  and symbol_fqn like '%has_email_domain'
order by symbol_fqn;
"
```

Expected shape:

- one row for `src/repositories/user_repository.rs::impl@...::has_email_domain`
- kind `method`

## 5) Claude session 2: add tests and touch the production file again

The second checkpoint adds the regression test and also touches the production file so DevQL materializes that symbol for the final commit as well.

```bash
cd /tmp/test-harness-claude-proof

export SESSION_ID="claude-test-session-1"
export TRANSCRIPT_PATH="$PWD/claude-test-1.jsonl"

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks claude-code session-start

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\",\"prompt\":\"Add a regression test for the email-domain helper and document the helper\"}" \
  | bitloops hooks claude-code user-prompt-submit

python3 - <<'PY'
from pathlib import Path

repo_file = Path("src/repositories/user_repository.rs")
repo_text = repo_file.read_text()
needle = "    pub fn has_email_domain(&self, email: &str, domain: &str) -> bool {\n"
replacement = "    /// Returns true when a stored email belongs to the supplied domain.\n" + needle
repo_file.write_text(repo_text.replace(needle, replacement))

test_file = Path("tests/user_repository_test.rs")
test_text = test_file.read_text()
marker = """    fn finds_user_by_id() {
        let mut repo = UserRepository::new();
        repo.save(User::new(7, "markos@bitloops.com".to_string(), "hash::secret".to_string()));

        let user = repo.find_by_id(7);

        assert!(user.is_some());
        assert_eq!(user.expect("missing user").email, "markos@bitloops.com");
    }
"""
addition = marker + """
    #[test]
    fn checks_email_domain() {
        let mut repo = UserRepository::new();
        repo.save(User::new(8, "admin@bitloops.com".to_string(), "hash::secret".to_string()));

        assert!(repo.has_email_domain("admin@bitloops.com", "bitloops.com"));
        assert!(!repo.has_email_domain("admin@bitloops.com", "example.com"));
    }
"""
test_file.write_text(test_text.replace(marker, addition))

Path("claude-test-1.jsonl").write_text("")
PY

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks claude-code stop

git add src/repositories/user_repository.rs tests/user_repository_test.rs claude-test-1.jsonl
git commit -m "Add test for email-domain helper"

printf '%s' "{\"session_id\":\"$SESSION_ID\",\"transcript_path\":\"$TRANSCRIPT_PATH\"}" \
  | bitloops hooks claude-code session-end
```

## 6) Re-ingest DevQL, ingest tests, and verify the link

```bash
cd /tmp/test-harness-claude-proof

export COMMIT_B="$(git rev-parse HEAD)"

BITLOOPS_DEVQL_EMBEDDING_PROVIDER=none \
BITLOOPS_DEVQL_SEMANTIC_PROVIDER=none \
bitloops devql ingest

cargo test

bitloops testlens ingest-tests --commit "$COMMIT_B"

sqlite3 ./.bitloops/stores/relational/relational.db "
select 'test_suites', count(*) from test_suites where commit_sha = '$COMMIT_B'
union all
select 'test_scenarios', count(*) from test_scenarios where commit_sha = '$COMMIT_B'
union all
select 'test_links', count(*) from test_links where commit_sha = '$COMMIT_B';
"

sqlite3 ./.bitloops/stores/relational/relational.db "
select t.signature, p.symbol_fqn
from test_links tl
join test_scenarios t
  on t.scenario_id = tl.test_scenario_id
 and t.commit_sha = tl.commit_sha
join artefacts p
  on p.artefact_id = tl.production_artefact_id
where tl.commit_sha = '$COMMIT_B'
  and p.symbol_fqn like '%has_email_domain'
order by t.signature;
"

bitloops testlens query \
  --commit "$COMMIT_B" \
  --artefact has_email_domain \
  --view tests \
  --min-strength 0.0
```

Expected shape:

- non-zero `test_links`
- a direct link from `checks_email_domain`
- query output with `verification_level: "partially_tested"`

## Validated result

This exact flow was validated locally on 2026-03-19 with:

- `COMMIT_A=bb915666e361695ba9d861cff54ba78d2e13aee9`
- `COMMIT_B=4f2419e4b69ba255ca3e2a52680b1966453f252d`

Observed results on `COMMIT_B`:

- `test_suites=4`
- `test_scenarios=5`
- `test_links=9`
- direct symbol linkage:
  - `checks_email_domain -> src/repositories/user_repository.rs::impl@8::has_email_domain`

Important boundary:

- DevQL is still checkpoint-driven.
- If the final checkpoint does not touch the production file you want to query, that symbol will not be materialized for the final commit, and `ingest-tests` may not be able to link to it.
