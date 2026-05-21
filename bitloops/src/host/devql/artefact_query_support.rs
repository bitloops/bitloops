use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde_json::Value;

use crate::artefact_query_planner::{
    ArtefactActivityFilter, ArtefactActivitySnapshot, ArtefactQuerySpec,
};

use super::{RelationalStorage, RelationalStorageRole, esc_pg};

type ArtefactSnapshotKey = (String, String);

pub(crate) async fn resolve_current_activity_snapshots(
    relational: &RelationalStorage,
    mut spec: ArtefactQuerySpec,
) -> Result<ArtefactQuerySpec> {
    if spec.activity_filter.is_none() || spec.temporal_scope.use_historical_tables() {
        return Ok(spec);
    }
    let Some(activity_filter) = spec.activity_filter.as_ref() else {
        return Ok(spec);
    };
    spec.current_activity_snapshots =
        Some(load_current_activity_snapshots(relational, &spec.repo_id, activity_filter).await?);
    Ok(spec)
}

pub(crate) async fn hydrate_artefact_rows_for_storage_ownership(
    relational: &RelationalStorage,
    repo_id: &str,
    use_historical_tables: bool,
    rows: Vec<Value>,
) -> Result<Vec<Value>> {
    let snapshot_keys = collect_row_snapshot_keys(&rows);
    if snapshot_keys.is_empty() {
        return Ok(rows);
    }

    let current_summaries = if use_historical_tables {
        BTreeMap::new()
    } else {
        load_summary_map(
            relational,
            RelationalStorageRole::CurrentProjection,
            "symbol_semantics_current",
            "content_id",
            repo_id,
            &snapshot_keys,
        )
        .await?
    };
    let shared_summaries = load_summary_map(
        relational,
        RelationalStorageRole::SharedRelational,
        "symbol_semantics",
        "blob_sha",
        repo_id,
        &snapshot_keys,
    )
    .await?;

    let current_representations = if use_historical_tables {
        BTreeMap::new()
    } else {
        load_embedding_representation_map(
            relational,
            RelationalStorageRole::CurrentProjection,
            "symbol_embeddings_current",
            "content_id",
            repo_id,
            &snapshot_keys,
        )
        .await?
    };
    let shared_representations = load_embedding_representation_map(
        relational,
        RelationalStorageRole::SharedRelational,
        "symbol_embeddings",
        "blob_sha",
        repo_id,
        &snapshot_keys,
    )
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let Some(mut object) = row.as_object().cloned() else {
                return row;
            };
            let Some(artefact_id) = object
                .get("artefact_id")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                return Value::Object(object);
            };
            let Some(snapshot_id) = object
                .get("blob_sha")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                return Value::Object(object);
            };
            let key = (artefact_id, snapshot_id);

            let summary = string_value(object.get("summary"))
                .or_else(|| current_summaries.get(&key).cloned())
                .or_else(|| shared_summaries.get(&key).cloned());
            object.insert(
                "summary".to_string(),
                summary.map(Value::String).unwrap_or(Value::Null),
            );

            let mut representations =
                parse_representation_values(object.get("embedding_representations"));
            if let Some(extra) = current_representations.get(&key) {
                representations.extend(extra.iter().cloned());
            }
            if let Some(extra) = shared_representations.get(&key) {
                representations.extend(extra.iter().cloned());
            }
            object.insert(
                "embedding_representations".to_string(),
                Value::Array(
                    ordered_representation_kinds(representations)
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ),
            );

            Value::Object(object)
        })
        .collect())
}

async fn load_current_activity_snapshots(
    relational: &RelationalStorage,
    repo_id: &str,
    activity_filter: &ArtefactActivityFilter,
) -> Result<Vec<ArtefactActivitySnapshot>> {
    let rows = query_rows_allowing_missing_relations(
        relational,
        RelationalStorageRole::SharedRelational,
        &build_current_activity_snapshot_lookup_sql(repo_id, activity_filter),
        &["checkpoint_files"],
    )
    .await?;
    let mut snapshots = BTreeSet::new();
    for row in rows {
        let Some(path) = row.get("path").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let Some(snapshot_id) = row
            .get("snapshot_id")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            continue;
        };
        if path.is_empty() || snapshot_id.is_empty() {
            continue;
        }
        snapshots.insert(ArtefactActivitySnapshot {
            path: path.to_string(),
            snapshot_id: snapshot_id.to_string(),
        });
    }
    Ok(snapshots.into_iter().collect())
}

async fn load_summary_map(
    relational: &RelationalStorage,
    role: RelationalStorageRole,
    table: &str,
    snapshot_column: &str,
    repo_id: &str,
    snapshot_keys: &[ArtefactSnapshotKey],
) -> Result<BTreeMap<ArtefactSnapshotKey, String>> {
    let Some(sql) = build_summary_lookup_sql(table, snapshot_column, repo_id, snapshot_keys) else {
        return Ok(BTreeMap::new());
    };
    let rows = query_rows_allowing_missing_relations(relational, role, &sql, &[table]).await?;
    let mut summaries = BTreeMap::new();
    for row in rows {
        let Some(artefact_id) = row
            .get("artefact_id")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            continue;
        };
        let Some(snapshot_id) = row
            .get("snapshot_id")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            continue;
        };
        let Some(summary) = string_value(row.get("summary")) else {
            continue;
        };
        summaries.insert((artefact_id.to_string(), snapshot_id.to_string()), summary);
    }
    Ok(summaries)
}

async fn load_embedding_representation_map(
    relational: &RelationalStorage,
    role: RelationalStorageRole,
    table: &str,
    snapshot_column: &str,
    repo_id: &str,
    snapshot_keys: &[ArtefactSnapshotKey],
) -> Result<BTreeMap<ArtefactSnapshotKey, BTreeSet<String>>> {
    let Some(sql) =
        build_embedding_representation_lookup_sql(table, snapshot_column, repo_id, snapshot_keys)
    else {
        return Ok(BTreeMap::new());
    };
    let rows = query_rows_allowing_missing_relations(relational, role, &sql, &[table]).await?;
    let mut representations = BTreeMap::<ArtefactSnapshotKey, BTreeSet<String>>::new();
    for row in rows {
        let Some(artefact_id) = row
            .get("artefact_id")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            continue;
        };
        let Some(snapshot_id) = row
            .get("snapshot_id")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            continue;
        };
        let Some(kind) = row
            .get("representation_kind")
            .and_then(Value::as_str)
            .and_then(canonical_representation_kind)
        else {
            continue;
        };
        representations
            .entry((artefact_id.to_string(), snapshot_id.to_string()))
            .or_default()
            .insert(kind.to_string());
    }
    Ok(representations)
}

async fn query_rows_allowing_missing_relations(
    relational: &RelationalStorage,
    role: RelationalStorageRole,
    sql: &str,
    allowed_missing_relations: &[&str],
) -> Result<Vec<Value>> {
    match relational.query_rows_for_role(role, sql).await {
        Ok(rows) => Ok(rows),
        Err(err)
            if allowed_missing_relations
                .iter()
                .any(|relation| missing_relation_error(&format!("{err:#}"), relation)) =>
        {
            Ok(Vec::new())
        }
        Err(err) => Err(err),
    }
}

fn build_current_activity_snapshot_lookup_sql(
    repo_id: &str,
    activity_filter: &ArtefactActivityFilter,
) -> String {
    let mut clauses = vec![
        format!("repo_id = '{}'", esc_pg(repo_id)),
        "path_after IS NOT NULL".to_string(),
        "blob_sha_after IS NOT NULL".to_string(),
    ];
    if let Some(agent) = activity_filter
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        clauses.push(format!("agent = '{}'", esc_pg(agent)));
    }
    if let Some(since) = activity_filter
        .since
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        clauses.push(format!("event_time >= '{}'", esc_pg(since)));
    }
    format!(
        "SELECT DISTINCT path_after AS path, blob_sha_after AS snapshot_id \
         FROM checkpoint_files \
         WHERE {}",
        clauses.join(" AND "),
    )
}

fn build_summary_lookup_sql(
    table: &str,
    snapshot_column: &str,
    repo_id: &str,
    snapshot_keys: &[ArtefactSnapshotKey],
) -> Option<String> {
    let predicate = pair_lookup_predicate(snapshot_column, snapshot_keys)?;
    Some(format!(
        "SELECT artefact_id, {snapshot_column} AS snapshot_id, summary \
         FROM {table} \
         WHERE repo_id = '{repo_id}' AND ({predicate})",
        snapshot_column = snapshot_column,
        table = table,
        repo_id = esc_pg(repo_id),
        predicate = predicate,
    ))
}

fn build_embedding_representation_lookup_sql(
    table: &str,
    snapshot_column: &str,
    repo_id: &str,
    snapshot_keys: &[ArtefactSnapshotKey],
) -> Option<String> {
    let predicate = pair_lookup_predicate(snapshot_column, snapshot_keys)?;
    Some(format!(
        "SELECT artefact_id, {snapshot_column} AS snapshot_id, representation_kind \
         FROM {table} \
         WHERE repo_id = '{repo_id}' AND ({predicate})",
        snapshot_column = snapshot_column,
        table = table,
        repo_id = esc_pg(repo_id),
        predicate = predicate,
    ))
}

fn pair_lookup_predicate(
    snapshot_column: &str,
    snapshot_keys: &[ArtefactSnapshotKey],
) -> Option<String> {
    if snapshot_keys.is_empty() {
        return None;
    }
    let values = snapshot_keys
        .iter()
        .map(|(artefact_id, snapshot_id)| {
            format!(
                "('{artefact_id}', '{snapshot_id}')",
                artefact_id = esc_pg(artefact_id),
                snapshot_id = esc_pg(snapshot_id),
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "(artefact_id, {snapshot_column}) IN (VALUES {values})",
        snapshot_column = snapshot_column,
        values = values,
    ))
}

fn collect_row_snapshot_keys(rows: &[Value]) -> Vec<ArtefactSnapshotKey> {
    let mut keys = BTreeSet::new();
    for row in rows {
        let Some(artefact_id) = row
            .get("artefact_id")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            continue;
        };
        let Some(snapshot_id) = row.get("blob_sha").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if artefact_id.is_empty() || snapshot_id.is_empty() {
            continue;
        }
        keys.insert((artefact_id.to_string(), snapshot_id.to_string()));
    }
    keys.into_iter().collect()
}

fn parse_representation_values(value: Option<&Value>) -> BTreeSet<String> {
    let mut parsed = BTreeSet::new();
    let Some(value) = value else {
        return parsed;
    };

    let raw_values = if let Some(items) = value.as_array() {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    } else if let Some(raw) = value.as_str() {
        serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
    } else {
        Vec::new()
    };

    for raw in raw_values {
        if let Some(kind) = canonical_representation_kind(&raw) {
            parsed.insert(kind.to_string());
        }
    }
    parsed
}

fn ordered_representation_kinds(kinds: BTreeSet<String>) -> Vec<String> {
    let mut ordered = kinds.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|kind| match kind.as_str() {
        "identity" => 0,
        "architecture" => 1,
        "code" => 2,
        "summary" => 3,
        _ => 9,
    });
    ordered
}

fn canonical_representation_kind(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "identity" | "locator" => Some("identity"),
        "architecture" => Some("architecture"),
        "code" | "baseline" | "enriched" => Some("code"),
        "summary" => Some("summary"),
        _ => None,
    }
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn missing_relation_error(message: &str, relation: &str) -> bool {
    message.contains(&format!("no such table: {relation}"))
        || message.contains(&format!("relation \"{relation}\" does not exist"))
        || message.contains(&format!("relation '{relation}' does not exist"))
        || message.contains(&format!("relation {relation} does not exist"))
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use serde_json::Value;
    use tempfile::tempdir;

    use super::{hydrate_artefact_rows_for_storage_ownership, resolve_current_activity_snapshots};
    use crate::artefact_query_planner::{
        ArtefactActivityFilter, ArtefactQuerySpec, ArtefactScope, ArtefactStructuralFilter,
        ArtefactTemporalScope,
    };
    use crate::host::devql::RelationalStorage;

    #[tokio::test]
    async fn resolve_current_activity_snapshots_reads_shared_checkpoint_files() {
        let temp = tempdir().expect("tempdir");
        let sqlite_path = temp.path().join("relational.sqlite");
        File::create(&sqlite_path).expect("create sqlite file");
        let relational = RelationalStorage::local_only(sqlite_path.clone());
        relational
            .exec(
                "CREATE TABLE checkpoint_files (repo_id TEXT, agent TEXT, event_time TEXT, path_after TEXT, blob_sha_after TEXT)",
            )
            .await
            .expect("create checkpoint_files");
        relational
            .exec(
                "INSERT INTO checkpoint_files (repo_id, agent, event_time, path_after, blob_sha_after) VALUES \
                 ('repo-1', 'codex', '2026-03-20T00:00:00Z', 'src/lib.rs', 'blob-1'), \
                 ('repo-1', 'codex', '2026-03-21T00:00:00Z', 'src/lib.rs', 'blob-1'), \
                 ('repo-1', 'other', '2026-03-21T00:00:00Z', 'src/skip.rs', 'blob-2')",
            )
            .await
            .expect("seed checkpoint_files");

        let spec = resolve_current_activity_snapshots(
            &relational,
            ArtefactQuerySpec {
                repo_id: "repo-1".to_string(),
                branch: Some("main".to_string()),
                historical_path_blob_sha: None,
                scope: ArtefactScope::default(),
                temporal_scope: ArtefactTemporalScope::Current,
                structural_filter: ArtefactStructuralFilter::default(),
                activity_filter: Some(ArtefactActivityFilter {
                    agent: Some("codex".to_string()),
                    since: Some("2026-03-20T00:00:00Z".to_string()),
                }),
                current_activity_snapshots: None,
                pagination: None,
            },
        )
        .await
        .expect("resolve snapshots");

        let snapshots = spec
            .current_activity_snapshots
            .expect("resolved snapshots should exist");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].path, "src/lib.rs");
        assert_eq!(snapshots[0].snapshot_id, "blob-1");
    }

    #[tokio::test]
    async fn hydrate_current_rows_merges_current_and_shared_semantic_state() {
        let temp = tempdir().expect("tempdir");
        let sqlite_path = temp.path().join("relational.sqlite");
        File::create(&sqlite_path).expect("create sqlite file");
        let relational = RelationalStorage::local_only(sqlite_path.clone());
        relational
            .exec_batch_transactional(&[
                "CREATE TABLE symbol_semantics_current (artefact_id TEXT, repo_id TEXT, content_id TEXT, summary TEXT)".to_string(),
                "CREATE TABLE symbol_semantics (artefact_id TEXT, repo_id TEXT, blob_sha TEXT, summary TEXT)".to_string(),
                "CREATE TABLE symbol_embeddings_current (artefact_id TEXT, repo_id TEXT, content_id TEXT, representation_kind TEXT)".to_string(),
                "CREATE TABLE symbol_embeddings (artefact_id TEXT, repo_id TEXT, blob_sha TEXT, representation_kind TEXT)".to_string(),
                "INSERT INTO symbol_semantics_current (artefact_id, repo_id, content_id, summary) VALUES ('artefact-1', 'repo-1', 'blob-1', 'current summary')".to_string(),
                "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, summary) VALUES ('artefact-2', 'repo-1', 'blob-2', 'shared summary')".to_string(),
                "INSERT INTO symbol_embeddings_current (artefact_id, repo_id, content_id, representation_kind) VALUES ('artefact-1', 'repo-1', 'blob-1', 'identity')".to_string(),
                "INSERT INTO symbol_embeddings_current (artefact_id, repo_id, content_id, representation_kind) VALUES ('artefact-1', 'repo-1', 'blob-1', 'architecture')".to_string(),
                "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, representation_kind) VALUES ('artefact-1', 'repo-1', 'blob-1', 'summary')".to_string(),
            ])
            .await
            .expect("seed semantic tables");

        let rows = hydrate_artefact_rows_for_storage_ownership(
            &relational,
            "repo-1",
            false,
            vec![
                serde_json::json!({
                    "artefact_id": "artefact-1",
                    "blob_sha": "blob-1",
                    "summary": Value::Null,
                    "embedding_representations": ["baseline"]
                }),
                serde_json::json!({
                    "artefact_id": "artefact-2",
                    "blob_sha": "blob-2",
                    "summary": Value::Null,
                    "embedding_representations": "[]"
                }),
            ],
        )
        .await
        .expect("hydrate rows");

        assert_eq!(
            rows[0]["summary"],
            Value::String("current summary".to_string())
        );
        assert_eq!(
            rows[0]["embedding_representations"],
            serde_json::json!(["identity", "architecture", "code", "summary"])
        );
        assert_eq!(
            rows[1]["summary"],
            Value::String("shared summary".to_string())
        );
        assert_eq!(rows[1]["embedding_representations"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn hydrate_current_rows_handles_more_than_sqlite_expression_depth() {
        const ROW_COUNT: usize = 1_100;

        let temp = tempdir().expect("tempdir");
        let sqlite_path = temp.path().join("relational.sqlite");
        File::create(&sqlite_path).expect("create sqlite file");
        let relational = RelationalStorage::local_only(sqlite_path.clone());
        relational
            .exec_batch_transactional(&[
                "CREATE TABLE symbol_semantics_current (artefact_id TEXT, repo_id TEXT, content_id TEXT, summary TEXT)".to_string(),
                "CREATE TABLE symbol_semantics (artefact_id TEXT, repo_id TEXT, blob_sha TEXT, summary TEXT)".to_string(),
                "CREATE TABLE symbol_embeddings_current (artefact_id TEXT, repo_id TEXT, content_id TEXT, representation_kind TEXT)".to_string(),
                "CREATE TABLE symbol_embeddings (artefact_id TEXT, repo_id TEXT, blob_sha TEXT, representation_kind TEXT)".to_string(),
            ])
            .await
            .expect("create semantic tables");

        let statements = (0..ROW_COUNT)
            .flat_map(|idx| {
                [
                    format!(
                        "INSERT INTO symbol_semantics_current (artefact_id, repo_id, content_id, summary) VALUES ('artefact-{idx}', 'repo-1', 'blob-{idx}', 'summary {idx}')"
                    ),
                    format!(
                        "INSERT INTO symbol_embeddings_current (artefact_id, repo_id, content_id, representation_kind) VALUES ('artefact-{idx}', 'repo-1', 'blob-{idx}', 'identity')"
                    ),
                ]
            })
            .collect::<Vec<_>>();
        relational
            .exec_batch_transactional(&statements)
            .await
            .expect("seed many semantic rows");

        let rows = (0..ROW_COUNT)
            .map(|idx| {
                serde_json::json!({
                    "artefact_id": format!("artefact-{idx}"),
                    "blob_sha": format!("blob-{idx}"),
                    "summary": Value::Null,
                    "embedding_representations": "[]"
                })
            })
            .collect::<Vec<_>>();

        let hydrated =
            hydrate_artefact_rows_for_storage_ownership(&relational, "repo-1", false, rows)
                .await
                .expect("hydrate many rows without exceeding SQLite expression depth");

        assert_eq!(hydrated.len(), ROW_COUNT);
        assert_eq!(
            hydrated[ROW_COUNT - 1]["summary"],
            Value::String(format!("summary {}", ROW_COUNT - 1))
        );
        assert_eq!(
            hydrated[ROW_COUNT - 1]["embedding_representations"],
            serde_json::json!(["identity"])
        );
    }
}
