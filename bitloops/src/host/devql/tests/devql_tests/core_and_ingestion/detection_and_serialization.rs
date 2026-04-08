use super::*;

#[test]
fn sql_helpers_escape_nullable_and_json_values() {
    assert_eq!(sql_nullable_text(None), "NULL");
    assert_eq!(sql_nullable_text(Some("O'Reilly")), "'O''Reilly'");
    assert_eq!(
        sql_jsonb_text_array(&["O'Reilly".to_string(), "plain".to_string()]),
        r#"'["O''Reilly","plain"]'::jsonb"#
    );
}

#[test]
fn detect_language_prefers_registered_language_pack_profiles() {
    assert_eq!(detect_language("src/main.ts"), "typescript");
    assert_eq!(detect_language("src/component.tsx"), "typescript");
    assert_eq!(detect_language("src/main.js"), "javascript");
    assert_eq!(detect_language("src/component.jsx"), "javascript");
    assert_eq!(detect_language("src/lib.rs"), "rust");
    assert_eq!(detect_language("src/main.py"), "python");
    assert_eq!(detect_language("src/Main.java"), "java");
    assert_eq!(detect_language("src/main.cs"), "csharp");
    assert_eq!(detect_language("src/readme.custom"), "custom");
    assert_eq!(detect_language("README"), "text");
}

#[tokio::test]
async fn upsert_current_state_for_unregistered_language_keeps_file_level_state_only() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let path = "src/readme.custom";
    let file_symbol = file_symbol_id(path);

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-a",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-a",
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path,
            blob_sha: "blob-custom",
        },
        "export const ignored = 1;\n",
    )
    .await
    .expect("upsert unsupported-language current state");

    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let file_row: (String, String) = conn
        .query_row(
            "SELECT language, canonical_kind FROM artefacts_current WHERE repo_id = ?1 AND symbol_id = ?2",
            rusqlite::params![cfg.repo.repo_id, file_symbol],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current file row");
    assert_eq!(file_row.0, "custom");
    assert_eq!(file_row.1, "file");

    let symbol_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2 AND symbol_id != ?3",
            rusqlite::params![cfg.repo.repo_id, path, file_symbol_id(path)],
            |row| row.get(0),
        )
        .expect("count non-file artefacts");
    assert_eq!(symbol_count, 0);

    let edge_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![cfg.repo.repo_id, path],
            |row| row.get(0),
        )
        .expect("count dependency edges");
    assert_eq!(edge_count, 0);
}

#[tokio::test]
async fn upsert_current_state_for_java_persists_symbols_and_edges() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let path = "src/com/acme/Greeter.java";
    let content = r#"package com.acme;

import java.util.List;

class Base {}
interface Runner {}

class Greeter extends Base implements Runner {
    private int count;

    Greeter() {}

    void helper() {}

    void greet(List<String> names) {
        helper();
        System.out.println(names.size());
        new Base();
    }
}
"#;

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-java",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-java",
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path,
            blob_sha: "blob-java",
        },
        content,
    )
    .await
    .expect("upsert java current state");

    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let file_row: (String, String) = conn
        .query_row(
            "SELECT language, canonical_kind FROM artefacts_current WHERE repo_id = ?1 AND symbol_id = ?2",
            rusqlite::params![cfg.repo.repo_id, file_symbol_id(path)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current java file row");
    assert_eq!(file_row.0, "java");
    assert_eq!(file_row.1, "file");

    let symbol_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2 AND symbol_id != ?3",
            rusqlite::params![cfg.repo.repo_id, path, file_symbol_id(path)],
            |row| row.get(0),
        )
        .expect("count java symbols");
    assert!(symbol_count >= 5);

    let edge_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![cfg.repo.repo_id, path],
            |row| row.get(0),
        )
        .expect("count java edges");
    assert!(edge_count >= 4);
}

#[tokio::test]
async fn upsert_current_state_for_csharp_persists_symbols_and_edges() {
    let cfg = test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("relational.db");
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let path = "src/UserService.cs";
    let content = r#"using System.Collections.Generic;

public interface IRepository {}
public class BaseService {}
public class User {}

public class UserService : BaseService, IRepository
{
    private readonly Helper _helper;
    private readonly List<User> _users;

    public UserService(Helper helper)
    {
        _helper = helper;
        _users = new List<User>();
    }

    public User GetUser()
    {
        return _helper.Load();
    }
}

public class Helper
{
    public User Load()
    {
        return new User();
    }
}
"#;

    upsert_current_state_for_content(
        &cfg,
        &relational,
        &FileRevision {
            commit_sha: "commit-csharp",
            revision: TemporalRevisionRef {
                kind: TemporalRevisionKind::Commit,
                id: "commit-csharp",
                temp_checkpoint_id: None,
            },
            commit_unix: 100,
            path,
            blob_sha: "blob-csharp",
        },
        content,
    )
    .await
    .expect("upsert csharp current state");

    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    let file_row: (String, String) = conn
        .query_row(
            "SELECT language, canonical_kind FROM artefacts_current WHERE repo_id = ?1 AND symbol_id = ?2",
            rusqlite::params![cfg.repo.repo_id, file_symbol_id(path)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current csharp file row");
    assert_eq!(file_row.0, "csharp");
    assert_eq!(file_row.1, "file");

    let symbol_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2 AND symbol_id != ?3",
            rusqlite::params![cfg.repo.repo_id, path, file_symbol_id(path)],
            |row| row.get(0),
        )
        .expect("count csharp symbols");
    assert!(symbol_count >= 6);

    let edge_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![cfg.repo.repo_id, path],
            |row| row.get(0),
        )
        .expect("count csharp edges");
    assert!(edge_count >= 3);
}
