use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
struct ArtefactSeed {
    artefact_id: &'static str,
    symbol_id: Option<&'static str>,
    path: &'static str,
    language: &'static str,
    canonical_kind: &'static str,
    language_kind: Option<&'static str>,
    symbol_fqn: Option<&'static str>,
    parent_artefact_id: Option<&'static str>,
    start_line: i64,
    end_line: i64,
    signature: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct SeedStats {
    pub artefacts: usize,
}

pub fn seed_database(conn: &mut Connection, commit_sha: &str) -> Result<SeedStats> {
    let tx = conn
        .transaction()
        .context("failed to start seed transaction")?;

    tx.execute(
        r#"
INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
VALUES (?1, 'local', 'local', 'testlens-fixture', 'main')
ON CONFLICT(repo_id) DO NOTHING
"#,
        params![FIXTURE_REPO_ID],
    )
    .context("failed seeding repository")?;
    tx.execute(
        r#"
INSERT INTO commits (commit_sha, repo_id, committed_at)
VALUES (?1, ?2, '2026-03-18T00:00:00Z')
ON CONFLICT(commit_sha) DO UPDATE SET
  repo_id = excluded.repo_id,
  committed_at = excluded.committed_at
"#,
        params![commit_sha, FIXTURE_REPO_ID],
    )
    .context("failed seeding commit")?;

    let mut parent_symbol_ids = HashMap::new();
    for artefact in FIXTURE_PRODUCTION_ARTEFACTS {
        let symbol_id = artefact
            .symbol_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("seed-symbol:{}", artefact.path));
        parent_symbol_ids.insert(artefact.artefact_id, symbol_id);
    }

    let mut artefact_count = 0usize;
    for artefact in FIXTURE_PRODUCTION_ARTEFACTS {
        let symbol_id = artefact
            .symbol_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("seed-symbol:{}", artefact.path));
        let parent_symbol_id = artefact
            .parent_artefact_id
            .and_then(|parent| parent_symbol_ids.get(parent).cloned());
        let blob_sha = format!("seed-blob:{}", artefact.path);

        tx.execute(
            r#"
INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(repo_id, commit_sha, path) DO UPDATE SET
  blob_sha = excluded.blob_sha
"#,
            params![FIXTURE_REPO_ID, commit_sha, artefact.path, blob_sha],
        )
        .with_context(|| format!("failed seeding file_state {}", artefact.path))?;
        tx.execute(
            r#"
INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at)
VALUES (?1, ?2, ?3, ?4, '2026-03-18T00:00:00Z')
ON CONFLICT(repo_id, path) DO UPDATE SET
  commit_sha = excluded.commit_sha,
  blob_sha = excluded.blob_sha,
  committed_at = excluded.committed_at
"#,
            params![FIXTURE_REPO_ID, artefact.path, commit_sha, blob_sha],
        )
        .with_context(|| format!("failed seeding current_file_state {}", artefact.path))?;
        tx.execute(
            r#"
INSERT INTO artefacts (
  artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte,
  end_byte, signature, modifiers, docstring, content_hash
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13, ?14, '[]', NULL, NULL
)
ON CONFLICT(artefact_id) DO UPDATE SET
  symbol_id = excluded.symbol_id,
  repo_id = excluded.repo_id,
  blob_sha = excluded.blob_sha,
  path = excluded.path,
  language = excluded.language,
  canonical_kind = excluded.canonical_kind,
  language_kind = excluded.language_kind,
  symbol_fqn = excluded.symbol_fqn,
  parent_artefact_id = excluded.parent_artefact_id,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  end_byte = excluded.end_byte,
  signature = excluded.signature
"#,
            params![
                artefact.artefact_id,
                symbol_id,
                FIXTURE_REPO_ID,
                blob_sha,
                artefact.path,
                artefact.language,
                artefact.canonical_kind,
                artefact.language_kind,
                artefact.symbol_fqn,
                artefact.parent_artefact_id,
                artefact.start_line,
                artefact.end_line,
                artefact.end_line * 10,
                artefact.signature,
            ],
        )
        .with_context(|| format!("failed seeding artefact {}", artefact.artefact_id))?;
        tx.execute(
            r#"
INSERT INTO artefacts_current (
  repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
  start_byte, end_byte, signature, modifiers, docstring, content_hash
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, ?15, ?16, '[]', NULL, NULL
)
ON CONFLICT(repo_id, symbol_id) DO UPDATE SET
  artefact_id = excluded.artefact_id,
  commit_sha = excluded.commit_sha,
  blob_sha = excluded.blob_sha,
  path = excluded.path,
  language = excluded.language,
  canonical_kind = excluded.canonical_kind,
  language_kind = excluded.language_kind,
  symbol_fqn = excluded.symbol_fqn,
  parent_symbol_id = excluded.parent_symbol_id,
  parent_artefact_id = excluded.parent_artefact_id,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  end_byte = excluded.end_byte,
  signature = excluded.signature
"#,
            params![
                FIXTURE_REPO_ID,
                symbol_id,
                artefact.artefact_id,
                commit_sha,
                blob_sha,
                artefact.path,
                artefact.language,
                artefact.canonical_kind,
                artefact.language_kind,
                artefact.symbol_fqn,
                parent_symbol_id,
                artefact.parent_artefact_id,
                artefact.start_line,
                artefact.end_line,
                artefact.end_line * 10,
                artefact.signature
            ],
        )
        .with_context(|| format!("failed seeding current artefact {}", artefact.artefact_id))?;
        artefact_count += 1;
    }

    tx.commit().context("failed to commit seed transaction")?;

    Ok(SeedStats {
        artefacts: artefact_count,
    })
}

const FIXTURE_REPO_ID: &str = "testlens-fixture";

const FIXTURE_PRODUCTION_ARTEFACTS: &[ArtefactSeed] = &[
    ArtefactSeed {
        artefact_id: "prod_file_models_user",
        symbol_id: None,
        path: "src/models/User.ts",
        language: "typescript",
        canonical_kind: "file",
        language_kind: Some("source_file"),
        symbol_fqn: Some("src/models/User.ts"),
        parent_artefact_id: None,
        start_line: 1,
        end_line: 10,
        signature: None,
    },
    ArtefactSeed {
        artefact_id: "prod_interface_user",
        symbol_id: Some("sym_user_interface"),
        path: "src/models/User.ts",
        language: "typescript",
        canonical_kind: "interface",
        language_kind: Some("interface_declaration"),
        symbol_fqn: Some("User"),
        parent_artefact_id: Some("prod_file_models_user"),
        start_line: 3,
        end_line: 8,
        signature: Some("export interface User"),
    },
    ArtefactSeed {
        artefact_id: "prod_type_user_id",
        symbol_id: Some("sym_user_id"),
        path: "src/models/User.ts",
        language: "typescript",
        canonical_kind: "type",
        language_kind: Some("type_alias_declaration"),
        symbol_fqn: Some("UserId"),
        parent_artefact_id: Some("prod_file_models_user"),
        start_line: 1,
        end_line: 1,
        signature: Some("export type UserId = string"),
    },
    ArtefactSeed {
        artefact_id: "prod_const_max_name_length",
        symbol_id: Some("sym_max_name_length"),
        path: "src/models/User.ts",
        language: "typescript",
        canonical_kind: "constant",
        language_kind: Some("variable_declaration"),
        symbol_fqn: Some("MAX_NAME_LENGTH"),
        parent_artefact_id: Some("prod_file_models_user"),
        start_line: 10,
        end_line: 10,
        signature: Some("export const MAX_NAME_LENGTH = 64"),
    },
    ArtefactSeed {
        artefact_id: "prod_file_user_repository",
        symbol_id: None,
        path: "src/repositories/UserRepository.ts",
        language: "typescript",
        canonical_kind: "file",
        language_kind: Some("source_file"),
        symbol_fqn: Some("src/repositories/UserRepository.ts"),
        parent_artefact_id: None,
        start_line: 1,
        end_line: 54,
        signature: None,
    },
    ArtefactSeed {
        artefact_id: "prod_class_user_repository",
        symbol_id: Some("sym_user_repository_class"),
        path: "src/repositories/UserRepository.ts",
        language: "typescript",
        canonical_kind: "class",
        language_kind: Some("class_declaration"),
        symbol_fqn: Some("UserRepository"),
        parent_artefact_id: Some("prod_file_user_repository"),
        start_line: 3,
        end_line: 54,
        signature: Some("export class UserRepository"),
    },
    ArtefactSeed {
        artefact_id: "prod_method_user_repository_find_by_id",
        symbol_id: Some("sym_repo_find_by_id"),
        path: "src/repositories/UserRepository.ts",
        language: "typescript",
        canonical_kind: "method",
        language_kind: Some("method_definition"),
        symbol_fqn: Some("UserRepository.findById"),
        parent_artefact_id: Some("prod_class_user_repository"),
        start_line: 18,
        end_line: 29,
        signature: Some("findById(id: UserId): User | null"),
    },
    ArtefactSeed {
        artefact_id: "prod_method_user_repository_find_by_email",
        symbol_id: Some("sym_repo_find_by_email"),
        path: "src/repositories/UserRepository.ts",
        language: "typescript",
        canonical_kind: "method",
        language_kind: Some("method_definition"),
        symbol_fqn: Some("UserRepository.findByEmail"),
        parent_artefact_id: Some("prod_class_user_repository"),
        start_line: 31,
        end_line: 42,
        signature: Some("findByEmail(email: string): User | null"),
    },
    ArtefactSeed {
        artefact_id: "prod_method_user_repository_delete",
        symbol_id: Some("sym_repo_delete"),
        path: "src/repositories/UserRepository.ts",
        language: "typescript",
        canonical_kind: "method",
        language_kind: Some("method_definition"),
        symbol_fqn: Some("UserRepository.delete"),
        parent_artefact_id: Some("prod_class_user_repository"),
        start_line: 44,
        end_line: 53,
        signature: Some("delete(id: UserId): boolean"),
    },
    ArtefactSeed {
        artefact_id: "prod_file_user_service",
        symbol_id: None,
        path: "src/services/UserService.ts",
        language: "typescript",
        canonical_kind: "file",
        language_kind: Some("source_file"),
        symbol_fqn: Some("src/services/UserService.ts"),
        parent_artefact_id: None,
        start_line: 1,
        end_line: 42,
        signature: None,
    },
    ArtefactSeed {
        artefact_id: "prod_class_user_service",
        symbol_id: Some("sym_user_service_class"),
        path: "src/services/UserService.ts",
        language: "typescript",
        canonical_kind: "class",
        language_kind: Some("class_declaration"),
        symbol_fqn: Some("UserService"),
        parent_artefact_id: Some("prod_file_user_service"),
        start_line: 11,
        end_line: 42,
        signature: Some("export class UserService"),
    },
    ArtefactSeed {
        artefact_id: "prod_method_user_service_create_user",
        symbol_id: Some("sym_service_create_user"),
        path: "src/services/UserService.ts",
        language: "typescript",
        canonical_kind: "method",
        language_kind: Some("method_definition"),
        symbol_fqn: Some("UserService.createUser"),
        parent_artefact_id: Some("prod_class_user_service"),
        start_line: 14,
        end_line: 33,
        signature: Some("createUser(input: CreateUserInput): User"),
    },
    ArtefactSeed {
        artefact_id: "prod_method_user_service_get_user",
        symbol_id: Some("sym_service_get_user"),
        path: "src/services/UserService.ts",
        language: "typescript",
        canonical_kind: "method",
        language_kind: Some("method_definition"),
        symbol_fqn: Some("UserService.getUser"),
        parent_artefact_id: Some("prod_class_user_service"),
        start_line: 35,
        end_line: 37,
        signature: Some("getUser(id: UserId): User | null"),
    },
    ArtefactSeed {
        artefact_id: "prod_method_user_service_delete_user",
        symbol_id: Some("sym_service_delete_user"),
        path: "src/services/UserService.ts",
        language: "typescript",
        canonical_kind: "method",
        language_kind: Some("method_definition"),
        symbol_fqn: Some("UserService.deleteUser"),
        parent_artefact_id: Some("prod_class_user_service"),
        start_line: 39,
        end_line: 41,
        signature: Some("deleteUser(id: UserId): boolean"),
    },
    ArtefactSeed {
        artefact_id: "prod_file_auth_service",
        symbol_id: None,
        path: "src/services/AuthService.ts",
        language: "typescript",
        canonical_kind: "file",
        language_kind: Some("source_file"),
        symbol_fqn: Some("src/services/AuthService.ts"),
        parent_artefact_id: None,
        start_line: 1,
        end_line: 13,
        signature: None,
    },
    ArtefactSeed {
        artefact_id: "prod_function_validate_token",
        symbol_id: Some("sym_validate_token"),
        path: "src/services/AuthService.ts",
        language: "typescript",
        canonical_kind: "function",
        language_kind: Some("function_declaration"),
        symbol_fqn: Some("validateToken"),
        parent_artefact_id: Some("prod_file_auth_service"),
        start_line: 1,
        end_line: 3,
        signature: Some("validateToken(token: string): boolean"),
    },
    ArtefactSeed {
        artefact_id: "prod_function_hash_password",
        symbol_id: Some("sym_hash_password"),
        path: "src/services/AuthService.ts",
        language: "typescript",
        canonical_kind: "function",
        language_kind: Some("function_declaration"),
        symbol_fqn: Some("hashPassword"),
        parent_artefact_id: Some("prod_file_auth_service"),
        start_line: 6,
        end_line: 13,
        signature: Some("hashPassword(raw: string): string"),
    },
];
