//! Capability workplane mailbox intent upserts.

use anyhow::{Context, Result};
use rusqlite::params;

use super::util::{sql_i64, unix_timestamp_now};
use crate::host::runtime_store::types::RepoSqliteRuntimeStore;

impl RepoSqliteRuntimeStore {
    pub fn set_capability_workplane_mailbox_intents<'a>(
        &self,
        capability_id: &str,
        mailbox_names: impl IntoIterator<Item = &'a str>,
        active: bool,
        source: Option<&str>,
    ) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_write_connection(|conn| {
            let now = unix_timestamp_now();
            let mut stmt = conn.prepare(
                "INSERT INTO capability_workplane_mailbox_intents (
                    repo_id, capability_id, mailbox_name, active, source, updated_at_unix
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (repo_id, capability_id, mailbox_name)
                 DO UPDATE SET
                    active = excluded.active,
                    source = excluded.source,
                    updated_at_unix = excluded.updated_at_unix",
            )?;
            for mailbox_name in mailbox_names {
                stmt.execute(params![
                    &self.repo_id,
                    capability_id,
                    mailbox_name,
                    if active { 1 } else { 0 },
                    source,
                    sql_i64(now)?,
                ])
                .with_context(|| {
                    format!(
                        "upserting capability workplane mailbox intent `{mailbox_name}` for repo `{}`",
                        self.repo_id
                    )
                })?;
            }
            Ok(())
        })
    }
}
