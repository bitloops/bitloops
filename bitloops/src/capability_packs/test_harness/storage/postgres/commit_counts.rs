use anyhow::{Context, Result};

use crate::models::TestHarnessCommitCounts;

pub(super) async fn load_test_harness_commit_counts(
    client: &mut tokio_postgres::Client,
    commit_sha: &str,
) -> Result<TestHarnessCommitCounts> {
    async fn cnt(client: &mut tokio_postgres::Client, sql: &str, commit_sha: &str) -> Result<u64> {
        let row = client
            .query_one(sql, &[&commit_sha])
            .await
            .context("test harness commit count query")?;
        let n: i64 = row.get(0);
        Ok(n.max(0) as u64)
    }

    Ok(TestHarnessCommitCounts {
        test_suites: cnt(
            client,
            "SELECT COUNT(*)::bigint FROM test_suites WHERE commit_sha = $1",
            commit_sha,
        )
        .await?,
        test_scenarios: cnt(
            client,
            "SELECT COUNT(*)::bigint FROM test_scenarios WHERE commit_sha = $1",
            commit_sha,
        )
        .await?,
        test_links: cnt(
            client,
            "SELECT COUNT(*)::bigint FROM test_links WHERE commit_sha = $1",
            commit_sha,
        )
        .await?,
        test_classifications: cnt(
            client,
            "SELECT COUNT(*)::bigint FROM test_classifications WHERE commit_sha = $1",
            commit_sha,
        )
        .await?,
        coverage_captures: cnt(
            client,
            "SELECT COUNT(*)::bigint FROM coverage_captures WHERE commit_sha = $1",
            commit_sha,
        )
        .await?,
        coverage_hits: cnt(
            client,
            r#"
SELECT COUNT(*)::bigint FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = $1
"#,
            commit_sha,
        )
        .await?,
    })
}
