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
    assert_eq!(detect_language("src/main.go"), "go");
    assert_eq!(detect_language("src/Main.java"), "java");
    assert_eq!(detect_language("src/main.cs"), "csharp");
    assert_eq!(detect_language("src/readme.custom"), "plain_text");
    assert_eq!(detect_language("README"), "plain_text");
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
    assert_eq!(file_row.0, "plain_text");
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

namespace MyApp.Services;

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

    let artefacts = {
        let mut stmt = conn
            .prepare(
                "SELECT symbol_fqn, canonical_kind \
                 FROM artefacts_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY symbol_fqn",
            )
            .expect("prepare csharp artefact query");
        stmt.query_map(rusqlite::params![cfg.repo.repo_id, path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .expect("query csharp artefacts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect csharp artefacts")
    };

    assert!(artefacts.iter().any(|(symbol_fqn, canonical_kind)| {
        symbol_fqn == "src/UserService.cs::ns::MyApp.Services" && canonical_kind.is_none()
    }));
    assert!(artefacts.iter().any(|(symbol_fqn, canonical_kind)| {
        symbol_fqn == "src/UserService.cs::IRepository"
            && canonical_kind.as_deref() == Some("interface")
    }));
    assert!(artefacts.iter().any(|(symbol_fqn, canonical_kind)| {
        symbol_fqn == "src/UserService.cs::UserService" && canonical_kind.as_deref() == Some("type")
    }));
    assert!(artefacts.iter().any(|(symbol_fqn, canonical_kind)| {
        symbol_fqn == "src/UserService.cs::UserService::_helper"
            && canonical_kind.as_deref() == Some("variable")
    }));
    assert!(artefacts.iter().any(|(symbol_fqn, canonical_kind)| {
        symbol_fqn == "src/UserService.cs::UserService::UserService"
            && canonical_kind.as_deref() == Some("method")
    }));
    assert!(artefacts.iter().any(|(symbol_fqn, canonical_kind)| {
        symbol_fqn == "src/UserService.cs::UserService::GetUser"
            && canonical_kind.as_deref() == Some("method")
    }));
    assert!(artefacts.iter().any(|(symbol_fqn, canonical_kind)| {
        symbol_fqn == "src/UserService.cs::using::System.Collections.Generic@1"
            && canonical_kind.as_deref() == Some("import")
    }));

    let edges = {
        let mut stmt = conn
            .prepare(
                "SELECT src.symbol_fqn, e.edge_kind, dst.symbol_fqn, e.to_symbol_ref \
                 FROM artefact_edges_current e \
                 JOIN artefacts_current src \
                   ON src.repo_id = e.repo_id AND src.symbol_id = e.from_symbol_id \
                 LEFT JOIN artefacts_current dst \
                   ON dst.repo_id = e.repo_id AND dst.symbol_id = e.to_symbol_id \
                 WHERE e.repo_id = ?1 AND e.path = ?2 \
                 ORDER BY src.symbol_fqn, e.edge_kind, COALESCE(dst.symbol_fqn, e.to_symbol_ref, '')",
            )
            .expect("prepare csharp edge query");
        stmt.query_map(rusqlite::params![cfg.repo.repo_id, path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .expect("query csharp edges")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect csharp edges")
    };

    assert!(edges.iter().any(
        |(from_symbol_fqn, edge_kind, to_symbol_fqn, to_symbol_ref)| {
            from_symbol_fqn == path
                && edge_kind == "imports"
                && to_symbol_fqn.is_none()
                && to_symbol_ref.as_deref() == Some("System.Collections.Generic")
        }
    ));
    assert!(
        edges
            .iter()
            .any(|(from_symbol_fqn, edge_kind, to_symbol_fqn, _)| {
                from_symbol_fqn == "src/UserService.cs::UserService"
                    && edge_kind == "extends"
                    && to_symbol_fqn.as_deref() == Some("src/UserService.cs::BaseService")
            })
    );
    assert!(
        edges
            .iter()
            .any(|(from_symbol_fqn, edge_kind, to_symbol_fqn, _)| {
                from_symbol_fqn == "src/UserService.cs::UserService"
                    && edge_kind == "implements"
                    && to_symbol_fqn.as_deref() == Some("src/UserService.cs::IRepository")
            })
    );
    assert!(
        edges
            .iter()
            .any(|(from_symbol_fqn, edge_kind, to_symbol_fqn, _)| {
                from_symbol_fqn == "src/UserService.cs::UserService::_helper"
                    && edge_kind == "references"
                    && to_symbol_fqn.as_deref() == Some("src/UserService.cs::Helper")
            })
    );
    assert!(edges.iter().any(
        |(from_symbol_fqn, edge_kind, to_symbol_fqn, to_symbol_ref)| {
            from_symbol_fqn == "src/UserService.cs::UserService::GetUser"
                && edge_kind == "calls"
                && to_symbol_fqn.is_none()
                && to_symbol_ref.as_deref() == Some("src/UserService.cs::member::_helper::Load")
        }
    ));
}
