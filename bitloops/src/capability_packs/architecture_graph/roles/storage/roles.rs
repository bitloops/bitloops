use anyhow::{Context, Result};
use serde_json::Value;

use crate::host::devql::{RelationalStorage, sql_json_value, sql_now};

use super::rows::{role_from_row, sql_text};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    ArchitectureRole, RoleLifecycle,
};

pub async fn upsert_classification_role(
    relational: &RelationalStorage,
    role: &ArchitectureRole,
) -> Result<()> {
    let canonical_key = format!("{}:{}", role.family, role.slug);
    let sql = format!(
        "INSERT INTO architecture_roles (
            repo_id, role_id, family, canonical_key, display_name, description,
            lifecycle_status, provenance_json, updated_at
         ) VALUES (
            {repo_id}, {role_id}, {family}, {canonical_key}, {display_name}, {description},
            {lifecycle}, {provenance}, {now}
         )
         ON CONFLICT(repo_id, role_id) DO UPDATE SET
            display_name = excluded.display_name,
            description = excluded.description,
            lifecycle_status = excluded.lifecycle_status,
            provenance_json = excluded.provenance_json,
            updated_at = {now};",
        repo_id = sql_text(&role.repo_id),
        role_id = sql_text(&role.role_id),
        family = sql_text(&role.family),
        canonical_key = sql_text(&canonical_key),
        display_name = sql_text(&role.display_name),
        description = sql_text(&role.description),
        lifecycle = sql_text(role.lifecycle.as_db()),
        provenance = sql_json_value(relational, &role.provenance),
        now = sql_now(relational),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("upserting architecture role")
}

pub async fn rename_role(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
    display_name: &str,
    provenance: &Value,
) -> Result<()> {
    let sql = format!(
        "UPDATE architecture_roles
         SET display_name = {display_name}, provenance_json = {provenance}, updated_at = {now}
         WHERE repo_id = {repo_id} AND role_id = {role_id};",
        repo_id = sql_text(repo_id),
        role_id = sql_text(role_id),
        display_name = sql_text(display_name),
        provenance = sql_json_value(relational, provenance),
        now = sql_now(relational),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("renaming architecture role")
}

pub async fn set_role_lifecycle(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
    lifecycle: RoleLifecycle,
) -> Result<()> {
    let sql = format!(
        "UPDATE architecture_roles
         SET lifecycle_status = {lifecycle}, updated_at = {now}
         WHERE repo_id = {repo_id} AND role_id = {role_id};",
        repo_id = sql_text(repo_id),
        role_id = sql_text(role_id),
        lifecycle = sql_text(lifecycle.as_db()),
        now = sql_now(relational),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("updating architecture role lifecycle")
}

pub async fn load_roles(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<ArchitectureRole>> {
    let sql = format!(
        "SELECT repo_id,
                role_id,
                family,
                CASE
                    WHEN instr(canonical_key, ':') > 0 THEN substr(canonical_key, instr(canonical_key, ':') + 1)
                    ELSE canonical_key
                END AS slug,
                display_name,
                description,
                lifecycle_status AS lifecycle,
                provenance_json
         FROM architecture_roles
         WHERE repo_id = {}
         ORDER BY family ASC, canonical_key ASC",
        sql_text(repo_id)
    );
    relational
        .query_rows(&sql)
        .await
        .context("loading architecture roles")?
        .into_iter()
        .map(role_from_row)
        .collect()
}
