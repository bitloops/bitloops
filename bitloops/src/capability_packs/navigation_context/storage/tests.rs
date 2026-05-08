use super::*;
use crate::capability_packs::navigation_context::schema::navigation_context_sqlite_schema_sql;
use crate::host::devql::RelationalStorage;
use tempfile::tempdir;

fn primitive(kind: NavigationPrimitiveKind, id: &str, hash: &str) -> NavigationPrimitiveFact {
    NavigationPrimitiveFact {
        repo_id: "repo".to_string(),
        primitive_id: id.to_string(),
        primitive_kind: kind.as_str().to_string(),
        identity_key: id.to_string(),
        label: id.to_string(),
        path: Some(format!("src/{id}.rs")),
        artefact_id: None,
        symbol_id: None,
        source_kind: "TEST".to_string(),
        confidence: 1.0,
        primitive_hash: hash.to_string(),
        properties: json!({}),
        provenance: json!({}),
        last_observed_generation: Some(1),
    }
}

#[test]
fn stable_hash_changes_when_input_changes() {
    assert_eq!(stable_hash(&["a", "b"]), stable_hash(&["a", "b"]));
    assert_ne!(stable_hash(&["a", "b"]), stable_hash(&["a", "c"]));
}

#[test]
fn view_materialisation_marks_changed_signature_stale() {
    let primitives = vec![primitive(
        NavigationPrimitiveKind::Symbol,
        "symbol-1",
        "new",
    )];
    let mut existing_views = BTreeMap::new();
    existing_views.insert(
        "architecture_map".to_string(),
        ExistingViewState {
            accepted_signature: "old-signature".to_string(),
        },
    );
    let mut existing_deps = BTreeMap::new();
    existing_deps.insert(
        "architecture_map".to_string(),
        BTreeMap::from([(
            "symbol-1".to_string(),
            ExistingViewDependency {
                primitive_hash: "old".to_string(),
                primitive_kind: NavigationPrimitiveKind::Symbol.as_str().to_string(),
                label: Some("symbol-1".to_string()),
                path: Some("src/symbol-1.rs".to_string()),
                source_kind: Some("TEST".to_string()),
            },
        )]),
    );

    let view = build_view_materialisation(
        NAVIGATION_VIEW_DEFINITIONS
            .iter()
            .find(|view| view.view_id == "architecture_map")
            .expect("architecture map view definition"),
        &primitives,
        &existing_views,
        &existing_deps,
    );

    assert_eq!(view.status, "stale");
    assert_eq!(
        view.stale_reason["changedPrimitiveIds"],
        json!(["symbol-1"])
    );
    assert_eq!(view.stale_reason["changeCount"], json!(1));
    assert_eq!(
        view.stale_reason["changedPrimitives"][0],
        json!({
            "primitiveId": "symbol-1",
            "primitiveKind": "SYMBOL",
            "label": "symbol-1",
            "path": "src/symbol-1.rs",
            "sourceKind": "TEST",
            "changeKind": "hash_changed",
            "previousHash": "old",
            "currentHash": "new",
        })
    );
}

#[tokio::test]
async fn accept_navigation_context_view_rebaselines_current_signature() -> Result<()> {
    let temp = tempdir()?;
    let sqlite_path = temp.path().join("navigation.sqlite");
    rusqlite::Connection::open(&sqlite_path)?;
    let relational = RelationalStorage::local_only(sqlite_path);
    relational
        .exec(navigation_context_sqlite_schema_sql())
        .await?;
    relational
            .exec(
                "INSERT INTO navigation_context_views_current (
                    repo_id, view_id, view_kind, label, view_query_version, dependency_query_json,
                    accepted_signature, current_signature, status, stale_reason_json, materialised_ref,
                    last_observed_generation, updated_at
                ) VALUES (
                    'repo', 'architecture_map', 'ARCHITECTURE_MAP', 'Architecture map', '1', '{}',
                    'old-signature', 'new-signature', 'stale', '{}', NULL, 7, '2026-05-03T00:00:00Z'
                );",
            )
            .await?;

    let accepted = accept_navigation_context_view(
        &relational,
        "repo",
        "architecture_map",
        Some("new-signature"),
        Some("test"),
        Some("reviewed"),
        Some("docs/navigation/architecture.md"),
    )
    .await?
    .expect("view should exist");

    assert!(
        accepted
            .acceptance_id
            .starts_with("navigation-context-acceptance-")
    );
    assert_eq!(accepted.previous_accepted_signature, "old-signature");
    assert_eq!(accepted.accepted_signature, "new-signature");
    assert_eq!(
        accepted.materialised_ref.as_deref(),
        Some("docs/navigation/architecture.md")
    );
    let rows = relational
        .query_rows(
            "SELECT accepted_signature, current_signature, status, materialised_ref \
                 FROM navigation_context_views_current \
                 WHERE repo_id = 'repo' AND view_id = 'architecture_map'",
        )
        .await?;
    assert_eq!(rows[0]["accepted_signature"], json!("new-signature"));
    assert_eq!(rows[0]["current_signature"], json!("new-signature"));
    assert_eq!(rows[0]["status"], json!("fresh"));
    assert_eq!(
        rows[0]["materialised_ref"],
        json!("docs/navigation/architecture.md")
    );

    let history = relational
            .query_rows(
                "SELECT view_id, previous_accepted_signature, accepted_signature, \
                        current_signature, expected_current_signature, source, reason, materialised_ref \
                 FROM navigation_context_view_acceptance_history \
                 WHERE repo_id = 'repo' AND view_id = 'architecture_map'",
            )
            .await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["view_id"], json!("architecture_map"));
    assert_eq!(
        history[0]["previous_accepted_signature"],
        json!("old-signature")
    );
    assert_eq!(history[0]["accepted_signature"], json!("new-signature"));
    assert_eq!(
        history[0]["expected_current_signature"],
        json!("new-signature")
    );
    assert_eq!(history[0]["source"], json!("test"));
    assert_eq!(history[0]["reason"], json!("reviewed"));
    assert_eq!(
        history[0]["materialised_ref"],
        json!("docs/navigation/architecture.md")
    );
    Ok(())
}

#[tokio::test]
async fn materialise_navigation_context_view_stores_snapshot_and_updates_ref() -> Result<()> {
    let temp = tempdir()?;
    let sqlite_path = temp.path().join("navigation.sqlite");
    rusqlite::Connection::open(&sqlite_path)?;
    let relational = RelationalStorage::local_only(sqlite_path);
    relational
        .exec(navigation_context_sqlite_schema_sql())
        .await?;
    relational
            .exec(
                "INSERT INTO navigation_context_views_current (
                    repo_id, view_id, view_kind, label, view_query_version, dependency_query_json,
                    accepted_signature, current_signature, status, stale_reason_json, materialised_ref,
                    last_observed_generation, updated_at
                ) VALUES (
                    'repo', 'architecture_map', 'ARCHITECTURE_MAP', 'Architecture map', '1',
                    '{\"primitiveKinds\":[\"SYMBOL\"]}', 'old-signature', 'new-signature',
                    'stale', '{\"reason\":\"dependency_signature_changed\"}', NULL, 7,
                    '2026-05-03T00:00:00Z'
                );
                INSERT INTO navigation_context_primitives_current (
                    repo_id, primitive_id, primitive_kind, identity_key, label, path, artefact_id,
                    symbol_id, source_kind, confidence, primitive_hash, hash_version,
                    properties_json, provenance_json, last_observed_generation, updated_at
                ) VALUES (
                    'repo', 'symbol-1', 'SYMBOL', 'symbol:render', 'render', 'src/render.rs',
                    'artefact-1', 'symbol-id-1', 'TEST', 1.0, 'primitive-hash',
                    'navigation-context-v1', '{\"signature\":\"fn render()\"}',
                    '{\"source\":\"test\"}', 7, '2026-05-03T00:00:00Z'
                );
                INSERT INTO navigation_context_view_dependencies_current (
                    repo_id, view_id, primitive_id, primitive_kind, primitive_hash, dependency_role, updated_at
                ) VALUES (
                    'repo', 'architecture_map', 'symbol-1', 'SYMBOL', 'primitive-hash',
                    'signature_input', '2026-05-03T00:00:00Z'
                );",
            )
            .await?;

    let materialised = materialise_navigation_context_view(
        &relational,
        "repo",
        "architecture_map",
        Some("new-signature"),
    )
    .await?
    .expect("view should exist");

    assert_eq!(materialised.view_id, "architecture_map");
    assert_eq!(materialised.current_signature, "new-signature");
    assert_eq!(materialised.primitive_count, 1);
    assert_eq!(materialised.edge_count, 0);
    assert!(
        materialised
            .materialised_ref
            .starts_with("navigation-context://materialisations/")
    );
    assert_eq!(
        materialised.payload["primitives"][0]["properties"],
        json!({"signature": "fn render()"})
    );
    assert!(materialised.rendered_text.contains("# Architecture map"));
    assert!(
        materialised
            .rendered_text
            .contains("SYMBOL: render (src/render.rs)")
    );

    let rows = relational
        .query_rows(
            "SELECT materialised_ref FROM navigation_context_views_current \
                 WHERE repo_id = 'repo' AND view_id = 'architecture_map'",
        )
        .await?;
    assert_eq!(
        rows[0]["materialised_ref"],
        json!(materialised.materialised_ref)
    );

    let snapshots = relational
            .query_rows(
                "SELECT materialisation_id, materialised_ref, current_signature, primitive_count, edge_count, payload_json \
                 FROM navigation_context_materialised_views \
                 WHERE repo_id = 'repo' AND view_id = 'architecture_map'",
            )
            .await?;
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0]["materialisation_id"],
        json!(materialised.materialisation_id)
    );
    assert_eq!(
        snapshots[0]["materialised_ref"],
        json!(materialised.materialised_ref)
    );
    assert_eq!(snapshots[0]["current_signature"], json!("new-signature"));
    assert_eq!(snapshots[0]["primitive_count"], json!(1));
    assert_eq!(snapshots[0]["edge_count"], json!(0));
    let payload: Value = serde_json::from_str(
        snapshots[0]["payload_json"]
            .as_str()
            .expect("payload JSON should be stored as text"),
    )?;
    assert_eq!(payload["view"]["viewId"], json!("architecture_map"));
    Ok(())
}
