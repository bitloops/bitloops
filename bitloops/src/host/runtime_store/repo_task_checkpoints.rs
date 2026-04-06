use anyhow::{Context, Result};

use super::blob_keys::task_artefact_blob_key;
use super::types::{RepoSqliteRuntimeStore, RuntimeMetadataBlobType, TaskCheckpointArtefact};

impl RepoSqliteRuntimeStore {
    pub fn save_task_checkpoint_artefact(&self, artefact: &TaskCheckpointArtefact) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for task checkpoint save")?;
        let (storage_backend, storage_path, content_hash, size_bytes) = self.write_runtime_blob(
            &task_artefact_blob_key(&self.repo_id, artefact),
            &artefact.payload,
        )?;

        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO task_checkpoint_artefacts (
                    artefact_id, session_id, repo_id, tool_use_id, agent_id,
                    checkpoint_uuid, artefact_kind, incremental_sequence, incremental_type,
                    is_incremental, storage_backend, storage_path, content_hash, size_bytes,
                    created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13, ?14,
                    datetime('now')
                )",
                rusqlite::params![
                    artefact.artefact_id,
                    artefact.session_id,
                    self.repo_id.as_str(),
                    artefact.tool_use_id,
                    artefact.agent_id,
                    artefact.checkpoint_uuid,
                    artefact.kind.as_str(),
                    artefact.incremental_sequence.map(i64::from),
                    artefact.incremental_type,
                    if artefact.is_incremental {
                        1_i64
                    } else {
                        0_i64
                    },
                    storage_backend,
                    storage_path,
                    content_hash,
                    size_bytes,
                ],
            )
            .context("inserting task_checkpoint_artefacts row")?;
            Ok(())
        })
    }

    pub fn load_task_checkpoint_artefacts(
        &self,
        session_id: &str,
        tool_use_id: &str,
    ) -> Result<Vec<TaskCheckpointArtefact>> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for task checkpoint load")?;
        let blob_store = self.open_repo_blob_store()?;
        let rows = sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT artefact_id, agent_id, checkpoint_uuid, artefact_kind,
                        incremental_sequence, incremental_type, is_incremental, storage_path
                 FROM task_checkpoint_artefacts
                 WHERE repo_id = ?1 AND session_id = ?2 AND tool_use_id = ?3
                 ORDER BY created_at ASC, artefact_id ASC",
            )?;
            let mut rows = stmt.query(rusqlite::params![
                self.repo_id.as_str(),
                session_id,
                tool_use_id
            ])?;
            let mut values = Vec::new();
            while let Some(row) = rows.next()? {
                values.push((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                ));
            }
            Ok::<_, anyhow::Error>(values)
        })?;

        let mut artefacts = Vec::new();
        for (
            artefact_id,
            agent_id,
            checkpoint_uuid,
            kind_raw,
            incremental_sequence,
            incremental_type,
            is_incremental,
            storage_path,
        ) in rows
        {
            let Some(kind) = RuntimeMetadataBlobType::from_str(&kind_raw) else {
                continue;
            };
            let payload = blob_store
                .store
                .read(&storage_path)
                .with_context(|| format!("reading runtime task blob `{storage_path}`"))?;
            artefacts.push(TaskCheckpointArtefact {
                artefact_id,
                session_id: session_id.to_string(),
                tool_use_id: tool_use_id.to_string(),
                agent_id,
                checkpoint_uuid,
                kind,
                incremental_sequence: incremental_sequence
                    .and_then(|value| u32::try_from(value).ok()),
                incremental_type,
                is_incremental: is_incremental != 0,
                payload,
            });
        }

        Ok(artefacts)
    }

    pub fn next_task_incremental_sequence(
        &self,
        session_id: &str,
        tool_use_id: &str,
    ) -> Result<u32> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite
            .initialise_runtime_checkpoint_schema()
            .context("initialising runtime schema for task checkpoint sequencing")?;
        let max_sequence = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT MAX(incremental_sequence)
                 FROM task_checkpoint_artefacts
                 WHERE repo_id = ?1
                   AND session_id = ?2
                   AND tool_use_id = ?3
                   AND is_incremental = 1
                   AND artefact_kind = ?4",
                rusqlite::params![
                    self.repo_id.as_str(),
                    session_id,
                    tool_use_id,
                    RuntimeMetadataBlobType::IncrementalCheckpoint.as_str(),
                ],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(anyhow::Error::from)
        })?;
        Ok(max_sequence
            .and_then(|value| u32::try_from(value).ok())
            .map(|value| value.saturating_add(1))
            .unwrap_or(1))
    }
}
