use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::blob_keys::session_snapshot_blob_key;
use super::types::{RepoSqliteRuntimeStore, RuntimeMetadataBlobType, SessionMetadataSnapshot};

impl RepoSqliteRuntimeStore {
    pub fn save_session_metadata_snapshot(&self, snapshot: &SessionMetadataSnapshot) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        let prompt_text = snapshot.bundle.prompt_text();
        let entries = [
            (
                RuntimeMetadataBlobType::Transcript,
                snapshot.bundle.transcript.clone(),
            ),
            (
                RuntimeMetadataBlobType::Prompts,
                prompt_text.as_bytes().to_vec(),
            ),
            (
                RuntimeMetadataBlobType::Summary,
                snapshot.bundle.summary.as_bytes().to_vec(),
            ),
            (
                RuntimeMetadataBlobType::Context,
                snapshot.bundle.context.clone(),
            ),
        ];

        for (blob_type, payload) in entries {
            if payload.is_empty() {
                continue;
            }
            let (storage_backend, storage_path, content_hash, size_bytes) = self
                .write_runtime_blob(
                    &session_snapshot_blob_key(
                        &self.repo_id,
                        &snapshot.session_id,
                        &snapshot.snapshot_id,
                        blob_type,
                    ),
                    &payload,
                )?;
            sqlite.with_connection(|conn| {
                conn.execute(
                    "INSERT INTO session_metadata_snapshots (
                        snapshot_id, session_id, repo_id, turn_id, transcript_identifier,
                        transcript_path, blob_type, storage_backend, storage_path,
                        content_hash, size_bytes, created_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5,
                        ?6, ?7, ?8, ?9,
                        ?10, ?11, datetime('now')
                    )
                    ON CONFLICT(repo_id, snapshot_id, blob_type) DO UPDATE SET
                        session_id = excluded.session_id,
                        turn_id = excluded.turn_id,
                        transcript_identifier = excluded.transcript_identifier,
                        transcript_path = excluded.transcript_path,
                        storage_backend = excluded.storage_backend,
                        storage_path = excluded.storage_path,
                        content_hash = excluded.content_hash,
                        size_bytes = excluded.size_bytes",
                    rusqlite::params![
                        snapshot.snapshot_id,
                        snapshot.session_id,
                        self.repo_id.as_str(),
                        snapshot.turn_id,
                        snapshot.transcript_identifier,
                        snapshot.transcript_path,
                        blob_type.as_str(),
                        storage_backend,
                        storage_path,
                        content_hash,
                        size_bytes,
                    ],
                )
                .context("upserting session_metadata_snapshots row")?;
                Ok(())
            })?;
        }

        Ok(())
    }

    pub fn load_latest_session_metadata_snapshot(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionMetadataSnapshot>> {
        let sqlite = self.connect_repo_sqlite()?;
        let header = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT snapshot_id,
                        COALESCE(MAX(turn_id), ''),
                        COALESCE(MAX(transcript_identifier), ''),
                        COALESCE(MAX(transcript_path), '')
                 FROM session_metadata_snapshots
                 WHERE repo_id = ?1 AND session_id = ?2
                 GROUP BY snapshot_id
                 ORDER BY MAX(created_at) DESC, snapshot_id DESC
                 LIMIT 1",
                rusqlite::params![self.repo_id.as_str(), session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })?;
        let Some((snapshot_id, turn_id, transcript_identifier, transcript_path)) = header else {
            return Ok(None);
        };

        let blob_store = self.open_repo_blob_store()?;
        let rows = sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT blob_type, storage_path
                 FROM session_metadata_snapshots
                 WHERE repo_id = ?1 AND session_id = ?2 AND snapshot_id = ?3",
            )?;
            let mut rows = stmt.query(rusqlite::params![
                self.repo_id.as_str(),
                session_id,
                snapshot_id.as_str()
            ])?;
            let mut values = Vec::new();
            while let Some(row) = rows.next()? {
                values.push((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
            }
            Ok::<_, anyhow::Error>(values)
        })?;

        let mut bundle =
            crate::host::checkpoints::transcript::metadata::SessionMetadataBundle::default();
        for (blob_type, storage_path) in rows {
            let payload = blob_store
                .store
                .read(&storage_path)
                .with_context(|| format!("reading runtime metadata blob `{storage_path}`"))?;
            match RuntimeMetadataBlobType::from_str(&blob_type) {
                Some(RuntimeMetadataBlobType::Transcript) => bundle.transcript = payload,
                Some(RuntimeMetadataBlobType::Prompts) => {
                    bundle.prompts = String::from_utf8_lossy(&payload)
                        .split("\n\n---\n\n")
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                Some(RuntimeMetadataBlobType::Summary) => {
                    bundle.summary = String::from_utf8_lossy(&payload).to_string();
                }
                Some(RuntimeMetadataBlobType::Context) => bundle.context = payload,
                _ => {}
            }
        }

        Ok(Some(SessionMetadataSnapshot {
            snapshot_id,
            session_id: session_id.to_string(),
            turn_id,
            transcript_identifier,
            transcript_path,
            bundle,
        }))
    }
}
