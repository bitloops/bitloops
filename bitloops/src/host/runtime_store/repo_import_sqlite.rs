use anyhow::{Context, Result};

use crate::host::interactions::db_store::legacy_interaction_spool_db_path;
use crate::utils::paths::default_repo_runtime_db_path;

use super::sqlite_migrate::{
    all_tables_empty, attach_if_needed, detach_if_needed, execute_copy_if_legacy_table_exists,
    legacy_relational_sqlite_path, table_has_rows_in_attached_db,
};
use super::types::RepoSqliteRuntimeStore;

impl RepoSqliteRuntimeStore {
    pub(crate) fn import_legacy_repo_local_runtime_if_needed(&self) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        let destination_empty = sqlite.with_connection(|conn| {
            all_tables_empty(
                conn,
                &[
                    "sessions",
                    "temporary_checkpoints",
                    "pre_prompt_states",
                    "pre_task_markers",
                    "interaction_sessions",
                    "interaction_turns",
                    "interaction_events",
                    "interaction_spool_queue",
                    "session_metadata_snapshots",
                    "task_checkpoint_artefacts",
                ],
            )
        })?;
        if !destination_empty {
            return Ok(());
        }

        let legacy_path = default_repo_runtime_db_path(&self.repo_root);
        if !legacy_path.is_file() || legacy_path == self.db_path {
            return Ok(());
        }

        sqlite.with_connection(|conn| {
            attach_if_needed(conn, &legacy_path, "legacy_repo_runtime")?;
            let legacy_tables = [
                "sessions",
                "temporary_checkpoints",
                "pre_prompt_states",
                "pre_task_markers",
                "interaction_sessions",
                "interaction_turns",
                "interaction_events",
                "interaction_spool_queue",
                "session_metadata_snapshots",
                "task_checkpoint_artefacts",
            ];
            let any_legacy_rows = legacy_tables.iter().try_fold(false, |found, table| {
                if found {
                    return Ok(true);
                }
                table_has_rows_in_attached_db(conn, "legacy_repo_runtime", table)
            })?;
            if !any_legacy_rows {
                detach_if_needed(conn, "legacy_repo_runtime")?;
                return Ok(());
            }

            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting repo-local runtime import transaction")?;
            let result = (|| {
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "sessions",
                    "INSERT OR IGNORE INTO sessions SELECT * FROM legacy_repo_runtime.sessions",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "temporary_checkpoints",
                    "INSERT OR IGNORE INTO temporary_checkpoints SELECT * FROM legacy_repo_runtime.temporary_checkpoints",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "pre_prompt_states",
                    "INSERT OR IGNORE INTO pre_prompt_states SELECT * FROM legacy_repo_runtime.pre_prompt_states",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "pre_task_markers",
                    "INSERT OR IGNORE INTO pre_task_markers SELECT * FROM legacy_repo_runtime.pre_task_markers",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "interaction_sessions",
                    "INSERT OR IGNORE INTO interaction_sessions SELECT * FROM legacy_repo_runtime.interaction_sessions",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "interaction_turns",
                    "INSERT OR IGNORE INTO interaction_turns SELECT * FROM legacy_repo_runtime.interaction_turns",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "interaction_events",
                    "INSERT OR IGNORE INTO interaction_events SELECT * FROM legacy_repo_runtime.interaction_events",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "interaction_spool_queue",
                    "INSERT OR IGNORE INTO interaction_spool_queue SELECT * FROM legacy_repo_runtime.interaction_spool_queue",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "session_metadata_snapshots",
                    "INSERT OR IGNORE INTO session_metadata_snapshots SELECT * FROM legacy_repo_runtime.session_metadata_snapshots",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_repo_runtime",
                    "task_checkpoint_artefacts",
                    "INSERT OR IGNORE INTO task_checkpoint_artefacts SELECT * FROM legacy_repo_runtime.task_checkpoint_artefacts",
                )?;
                conn.execute_batch("COMMIT;")
                    .context("committing repo-local runtime import transaction")?;
                Ok(())
            })();
            if result.is_err() {
                let _ = conn.execute_batch("ROLLBACK;");
            }
            detach_if_needed(conn, "legacy_repo_runtime")?;
            result
        })
    }

    pub(crate) fn import_legacy_checkpoint_runtime_if_needed(&self) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        let destination_empty = sqlite.with_connection(|conn| {
            all_tables_empty(
                conn,
                &[
                    "sessions",
                    "temporary_checkpoints",
                    "pre_prompt_states",
                    "pre_task_markers",
                ],
            )
        })?;
        if !destination_empty {
            return Ok(());
        }

        let legacy_path = legacy_relational_sqlite_path(&self.repo_root)?;
        if !legacy_path.is_file() || legacy_path == self.db_path {
            return Ok(());
        }

        sqlite.with_connection(|conn| {
            attach_if_needed(conn, &legacy_path, "legacy_runtime")?;
            let legacy_tables = [
                "sessions",
                "temporary_checkpoints",
                "pre_prompt_states",
                "pre_task_markers",
            ];
            let any_legacy_rows = legacy_tables.iter().try_fold(false, |found, table| {
                if found {
                    return Ok(true);
                }
                table_has_rows_in_attached_db(conn, "legacy_runtime", table)
            })?;
            if !any_legacy_rows {
                detach_if_needed(conn, "legacy_runtime")?;
                return Ok(());
            }

            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting runtime checkpoint import transaction")?;
            let result = (|| {
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_runtime",
                    "sessions",
                    "INSERT OR IGNORE INTO sessions SELECT * FROM legacy_runtime.sessions",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_runtime",
                    "temporary_checkpoints",
                    "INSERT OR IGNORE INTO temporary_checkpoints SELECT * FROM legacy_runtime.temporary_checkpoints",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_runtime",
                    "pre_prompt_states",
                    "INSERT OR IGNORE INTO pre_prompt_states SELECT * FROM legacy_runtime.pre_prompt_states",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_runtime",
                    "pre_task_markers",
                    "INSERT OR IGNORE INTO pre_task_markers SELECT * FROM legacy_runtime.pre_task_markers",
                )?;
                conn.execute_batch("COMMIT;")
                    .context("committing runtime checkpoint import transaction")?;
                Ok(())
            })();
            if result.is_err() {
                let _ = conn.execute_batch("ROLLBACK;");
            }
            detach_if_needed(conn, "legacy_runtime")?;
            result
        })
    }

    pub(crate) fn import_legacy_interaction_spool_if_needed(&self) -> Result<()> {
        let sqlite = self.connect_repo_sqlite()?;
        let destination_empty = sqlite.with_connection(|conn| {
            all_tables_empty(
                conn,
                &[
                    "interaction_sessions",
                    "interaction_turns",
                    "interaction_events",
                    "interaction_spool_queue",
                ],
            )
        })?;
        if !destination_empty {
            return Ok(());
        }

        let legacy_path = legacy_interaction_spool_db_path(&self.repo_root)
            .context("resolving legacy interaction spool path")?;
        if !legacy_path.is_file() || legacy_path == self.db_path {
            return Ok(());
        }

        sqlite.with_connection(|conn| {
            attach_if_needed(conn, &legacy_path, "legacy_spool")?;
            let legacy_tables = [
                "interaction_sessions",
                "interaction_turns",
                "interaction_events",
                "interaction_spool_queue",
            ];
            let any_legacy_rows = legacy_tables.iter().try_fold(false, |found, table| {
                if found {
                    return Ok(true);
                }
                table_has_rows_in_attached_db(conn, "legacy_spool", table)
            })?;
            if !any_legacy_rows {
                detach_if_needed(conn, "legacy_spool")?;
                return Ok(());
            }

            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting interaction spool import transaction")?;
            let result = (|| {
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_spool",
                    "interaction_sessions",
                    "INSERT OR IGNORE INTO interaction_sessions SELECT * FROM legacy_spool.interaction_sessions",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_spool",
                    "interaction_turns",
                    "INSERT OR IGNORE INTO interaction_turns SELECT * FROM legacy_spool.interaction_turns",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_spool",
                    "interaction_events",
                    "INSERT OR IGNORE INTO interaction_events SELECT * FROM legacy_spool.interaction_events",
                )?;
                execute_copy_if_legacy_table_exists(
                    conn,
                    "legacy_spool",
                    "interaction_spool_queue",
                    "INSERT OR IGNORE INTO interaction_spool_queue SELECT * FROM legacy_spool.interaction_spool_queue",
                )?;
                conn.execute_batch("COMMIT;")
                    .context("committing interaction spool import transaction")?;
                Ok(())
            })();
            if result.is_err() {
                let _ = conn.execute_batch("ROLLBACK;");
            }
            detach_if_needed(conn, "legacy_spool")?;
            result
        })
    }
}
