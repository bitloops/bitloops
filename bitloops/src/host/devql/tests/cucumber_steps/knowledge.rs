use crate::host::devql::cucumber_world::DevqlBddWorld;
use cucumber::{codegen::LocalBoxFuture, step::Collection};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|err| panic!("invalid step regex `{pattern}`: {err}"))
}

fn step_fn(
    f: for<'a> fn(&'a mut DevqlBddWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()>,
) -> for<'a> fn(&'a mut DevqlBddWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()> {
    f
}

fn table_rows(ctx: &cucumber::step::Context) -> Vec<Vec<String>> {
    ctx.step
        .table
        .as_ref()
        .map(|table| table.rows.clone())
        .expect("step table should be present")
}

fn table_row_maps(ctx: &cucumber::step::Context) -> Vec<HashMap<String, String>> {
    let rows = table_rows(ctx);
    let (header, values) = rows
        .split_first()
        .expect("table should include a header row");
    values
        .iter()
        .map(|row| {
            header
                .iter()
                .cloned()
                .zip(row.iter().cloned())
                .collect::<HashMap<_, _>>()
        })
        .collect()
}

fn key_value_table(ctx: &cucumber::step::Context) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for row in table_rows(ctx) {
        assert!(
            row.len() >= 2,
            "key/value row should contain at least two columns"
        );
        values.insert(row[0].trim().to_string(), row[1].trim().to_string());
    }
    values
}

fn normalize_expected_id(world: &DevqlBddWorld, raw: &str) -> String {
    let resolved = world.resolve_placeholders(raw);
    if resolved == "HEAD" {
        return world
            .knowledge
            .as_ref()
            .expect("knowledge harness should be initialized")
            .head_commit();
    }
    resolved
}

fn remember_ingest_ids(
    world: &mut DevqlBddWorld,
    ingest: &crate::capability_packs::knowledge::IngestKnowledgeResult,
) {
    world.remember_id("item_id", ingest.knowledge_item_id.clone());
    world.remember_id("version_id", ingest.knowledge_item_version_id.clone());
}

fn given_knowledge_workspace(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        world.init_knowledge_harness();
    })
}

fn given_provider_returns(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let provider = &ctx.matches[1].1;
        let url = &ctx.matches[2].1;
        let kv = key_value_table(&ctx);
        let title = kv.get("title").map(String::as_str).unwrap_or("Untitled");
        let body = kv.get("body").map(String::as_str).unwrap_or("");
        let updated_at = kv.get("updated_at").map(String::as_str);

        world
            .knowledge_harness_mut()
            .stub_success(provider, url, title, body, updated_at)
            .expect("stub success response");
    })
}

fn given_provider_returns_sequence(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let provider = &ctx.matches[1].1;
        let url = &ctx.matches[2].1;
        let rows = table_row_maps(&ctx);
        let sequence = rows
            .iter()
            .map(|row| {
                (
                    row.get("title")
                        .cloned()
                        .expect("title column should exist"),
                    row.get("body").cloned().expect("body column should exist"),
                )
            })
            .collect::<Vec<_>>();

        world
            .knowledge_harness_mut()
            .stub_success_sequence(provider, url, sequence.as_slice())
            .expect("stub success sequence");
    })
}

fn given_provider_fails(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let provider = &ctx.matches[1].1;
        let url = &ctx.matches[2].1;
        let message = &ctx.matches[3].1;

        world
            .knowledge_harness_mut()
            .stub_failure(provider, url, message)
            .expect("stub failure response");
    })
}

fn given_valid_head_commit(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let head_sha = world.knowledge_harness_mut().head_commit();
        world.remember_id("head_sha", head_sha);
    })
}

fn given_two_valid_commits(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let first = world.knowledge_harness_mut().head_commit();
        let second = world
            .knowledge_harness_mut()
            .create_empty_commit("bdd-second-commit");
        world.remember_id("first_commit_sha", first);
        world.remember_id("second_commit_sha", second.clone());
        world.remember_id("head_sha", second);
    })
}

fn given_checkpoint_exists(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let checkpoint_id = &ctx.matches[1].1;
        world
            .knowledge_harness_mut()
            .seed_checkpoint(checkpoint_id)
            .expect("seed checkpoint");
    })
}

fn given_artefact_exists(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let artefact_id = &ctx.matches[1].1;
        world
            .knowledge_harness_mut()
            .seed_artefact(artefact_id)
            .expect("seed artefact");
    })
}

fn given_added_knowledge(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let url = &ctx.matches[1].1;
        let result = world.knowledge_harness_mut().add(url, None).await;
        match result {
            Ok((ingest, association)) => {
                remember_ingest_ids(world, &ingest);
                world.knowledge_last_error = None;
                world.knowledge_last_ingest = Some(ingest);
                world.knowledge_last_association = association;
            }
            Err(err) => {
                world.knowledge_last_ingest = None;
                world.knowledge_last_association = None;
                world.knowledge_last_error = Some(err);
            }
        }
    })
}

fn given_added_knowledge_as_alias(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let url = &ctx.matches[1].1;
        let alias = &ctx.matches[2].1;
        let (ingest, association) = world
            .knowledge_harness_mut()
            .add(url, None)
            .await
            .expect("pre-add knowledge");

        world.remember_id(
            format!("{alias}_item_id").as_str(),
            ingest.knowledge_item_id.clone(),
        );
        world.remember_id(
            format!("{alias}_item_version_id").as_str(),
            ingest.knowledge_item_version_id.clone(),
        );

        remember_ingest_ids(world, &ingest);
        world.knowledge_last_error = None;
        world.knowledge_last_ingest = Some(ingest);
        world.knowledge_last_association = association;
    })
}

fn given_added_two_versions(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let url = &ctx.matches[1].1;
        let (first, _) = world
            .knowledge_harness_mut()
            .add(url, None)
            .await
            .expect("first pre-add");
        let (second, _) = world
            .knowledge_harness_mut()
            .add(url, None)
            .await
            .expect("second pre-add");

        world.remember_id("item_id", second.knowledge_item_id.clone());
        world.remember_id("first_version_id", first.knowledge_item_version_id.clone());
        world.remember_id(
            "second_version_id",
            second.knowledge_item_version_id.clone(),
        );
        world.remember_id("version_id", second.knowledge_item_version_id.clone());

        world.knowledge_last_error = None;
        world.knowledge_last_ingest = Some(second);
        world.knowledge_last_association = None;
    })
}

fn given_added_two_versions_as_alias(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let url = &ctx.matches[1].1;
        let alias = &ctx.matches[2].1;
        let (first, _) = world
            .knowledge_harness_mut()
            .add(url, None)
            .await
            .expect("first pre-add");
        let (second, _) = world
            .knowledge_harness_mut()
            .add(url, None)
            .await
            .expect("second pre-add");

        world.remember_id(
            format!("{alias}_item_id").as_str(),
            second.knowledge_item_id.clone(),
        );
        world.remember_id(
            format!("{alias}_first_version_id").as_str(),
            first.knowledge_item_version_id.clone(),
        );
        world.remember_id(
            format!("{alias}_second_version_id").as_str(),
            second.knowledge_item_version_id.clone(),
        );
        world.remember_id(
            format!("{alias}_item_version_id").as_str(),
            second.knowledge_item_version_id.clone(),
        );

        world.knowledge_last_error = None;
        world.knowledge_last_ingest = Some(second);
        world.knowledge_last_association = None;
    })
}

fn when_add(world: &mut DevqlBddWorld, ctx: cucumber::step::Context) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let url = &ctx.matches[1].1;
        let result = world.knowledge_harness_mut().add(url, None).await;
        match result {
            Ok((ingest, association)) => {
                remember_ingest_ids(world, &ingest);
                world.knowledge_last_error = None;
                world.knowledge_last_ingest = Some(ingest);
                world.knowledge_last_association = association;
            }
            Err(err) => {
                world.knowledge_last_ingest = None;
                world.knowledge_last_association = None;
                world.knowledge_last_error = Some(err);
            }
        }
    })
}

fn when_add_with_commit(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let url = &ctx.matches[1].1;
        let commit_ref = world.resolve_placeholders(&ctx.matches[2].1);
        let result = world
            .knowledge_harness_mut()
            .add(url, Some(commit_ref.as_str()))
            .await;
        match result {
            Ok((ingest, association)) => {
                remember_ingest_ids(world, &ingest);
                world.knowledge_last_error = None;
                world.knowledge_last_ingest = Some(ingest);
                world.knowledge_last_association = association;
            }
            Err(err) => {
                world.knowledge_last_ingest = None;
                world.knowledge_last_association = None;
                world.knowledge_last_error = Some(err);
            }
        }
    })
}

fn when_associate(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let source_ref = world.resolve_placeholders(&ctx.matches[1].1);
        let target_ref = world.resolve_placeholders(&ctx.matches[2].1);

        let result = world
            .knowledge_harness_mut()
            .associate(source_ref.as_str(), target_ref.as_str())
            .await;

        match result {
            Ok(association) => {
                world.knowledge_last_error = None;
                world.knowledge_last_association = Some(association);
            }
            Err(err) => {
                world.knowledge_last_association = None;
                world.knowledge_last_error = Some(err);
            }
        }
    })
}

fn then_last_operation_succeeds(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        if let Some(err) = world.knowledge_last_error.as_ref() {
            panic!("expected success, got error: {err:#}");
        }
    })
}

fn then_operation_fails_with_message(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let err = world
            .knowledge_last_error
            .as_ref()
            .expect("expected operation error");
        assert!(
            err.to_string().contains(expected),
            "expected error containing `{expected}`, got `{err}`"
        );
    })
}

fn then_exactly_knowledge_items_exist(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected: i64 = ctx.matches[1]
            .1
            .parse()
            .expect("knowledge item count should be numeric");
        let actual = world
            .knowledge_harness_mut()
            .sqlite_row_count("knowledge_items")
            .expect("count knowledge_items");
        assert_eq!(actual, expected, "unexpected knowledge item count");
    })
}

fn then_exactly_versions_exist(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected: i64 = ctx.matches[1]
            .1
            .parse()
            .expect("version count should be numeric");
        let actual = world
            .knowledge_harness_mut()
            .duckdb_document_count()
            .expect("count knowledge_document_versions");
        assert_eq!(
            actual, expected,
            "unexpected knowledge document version count"
        );
    })
}

fn then_exactly_relations_exist(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected: i64 = ctx.matches[1]
            .1
            .parse()
            .expect("relation count should be numeric");
        let actual = world
            .knowledge_harness_mut()
            .sqlite_row_count("knowledge_relation_assertions")
            .expect("count knowledge_relation_assertions");
        assert_eq!(actual, expected, "unexpected relation assertion count");
    })
}

fn then_last_ingest_provider_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let ingest = world
            .knowledge_last_ingest
            .as_ref()
            .expect("expected last ingest result");
        assert_eq!(ingest.provider, *expected, "unexpected ingest provider");
    })
}

fn then_last_ingest_source_kind_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let ingest = world
            .knowledge_last_ingest
            .as_ref()
            .expect("expected last ingest result");
        assert_eq!(
            ingest.source_kind, *expected,
            "unexpected ingest source kind"
        );
    })
}

fn then_relation_target_type_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let relation = world
            .knowledge_harness_mut()
            .latest_relation()
            .expect("read latest relation")
            .expect("latest relation should exist");
        assert_eq!(
            relation.target_type, *expected,
            "unexpected relation target type"
        );
    })
}

fn then_relation_target_id_equals(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = normalize_expected_id(world, &ctx.matches[1].1);
        let relation = world
            .knowledge_harness_mut()
            .latest_relation()
            .expect("read latest relation")
            .expect("latest relation should exist");
        assert_eq!(
            relation.target_id, expected,
            "unexpected relation target id"
        );
    })
}

fn then_relation_source_version_equals(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = normalize_expected_id(world, &ctx.matches[1].1);
        let relation = world
            .knowledge_harness_mut()
            .latest_relation()
            .expect("read latest relation")
            .expect("latest relation should exist");
        assert_eq!(
            relation.source_knowledge_item_version_id, expected,
            "unexpected relation source version"
        );
    })
}

fn then_relation_type_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let relation = world
            .knowledge_harness_mut()
            .latest_relation()
            .expect("read latest relation")
            .expect("latest relation should exist");
        assert_eq!(
            relation.relation_type, *expected,
            "unexpected relation type"
        );
    })
}

fn then_association_method_is(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = &ctx.matches[1].1;
        let relation = world
            .knowledge_harness_mut()
            .latest_relation()
            .expect("read latest relation")
            .expect("latest relation should exist");
        assert_eq!(
            relation.association_method, *expected,
            "unexpected association method"
        );
    })
}

fn then_last_two_ingests_reuse_item_id(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let harness = world.knowledge_harness_mut();
        assert!(
            harness.ingest_history.len() >= 2,
            "need at least two ingests in history"
        );
        let last = &harness.ingest_history[harness.ingest_history.len() - 1];
        let prev = &harness.ingest_history[harness.ingest_history.len() - 2];
        assert_eq!(
            last.knowledge_item_id, prev.knowledge_item_id,
            "expected same knowledge item id"
        );
    })
}

fn then_last_two_ingests_reuse_version_id(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let harness = world.knowledge_harness_mut();
        assert!(
            harness.ingest_history.len() >= 2,
            "need at least two ingests in history"
        );
        let last = &harness.ingest_history[harness.ingest_history.len() - 1];
        let prev = &harness.ingest_history[harness.ingest_history.len() - 2];
        assert_eq!(
            last.knowledge_item_version_id, prev.knowledge_item_version_id,
            "expected same knowledge item version id"
        );
    })
}

fn then_last_two_ingests_have_different_versions(
    world: &mut DevqlBddWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let harness = world.knowledge_harness_mut();
        assert!(
            harness.ingest_history.len() >= 2,
            "need at least two ingests in history"
        );
        let last = &harness.ingest_history[harness.ingest_history.len() - 1];
        let prev = &harness.ingest_history[harness.ingest_history.len() - 2];
        assert_ne!(
            last.knowledge_item_version_id, prev.knowledge_item_version_id,
            "expected different knowledge item version ids"
        );
    })
}

fn then_all_source_rows_stamped_for(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected_operation = &ctx.matches[1].1;
        let rows = world
            .knowledge_harness_mut()
            .source_provenance_rows()
            .expect("read source provenance rows");
        assert!(!rows.is_empty(), "expected source provenance rows");

        for row in rows {
            let value: Value = serde_json::from_str(row.as_str()).expect("parse source provenance");
            assert_eq!(value["capability"], Value::String("knowledge".to_string()));
            assert_eq!(
                value["capability_version"],
                json!(crate::capability_packs::knowledge::descriptor::KNOWLEDGE_DESCRIPTOR.version)
            );
            assert_eq!(
                value["api_version"],
                json!(
                    crate::capability_packs::knowledge::descriptor::KNOWLEDGE_DESCRIPTOR
                        .api_version
                )
            );
            assert_eq!(
                value["operation"],
                Value::String(expected_operation.to_string())
            );
            if let Some(id) = value.get("ingester_id").and_then(Value::as_str) {
                assert_eq!(id, expected_operation.as_str());
            }
            if let Some(cap) = value.get("invoking_capability_id").and_then(Value::as_str) {
                assert_eq!(cap, "knowledge");
            }
        }
    })
}

fn then_all_item_rows_stamped_for(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected_operation = &ctx.matches[1].1;
        let rows = world
            .knowledge_harness_mut()
            .item_provenance_rows()
            .expect("read item provenance rows");
        assert!(!rows.is_empty(), "expected item provenance rows");

        for row in rows {
            let value: Value = serde_json::from_str(row.as_str()).expect("parse item provenance");
            assert_eq!(value["capability"], Value::String("knowledge".to_string()));
            assert_eq!(
                value["capability_version"],
                json!(crate::capability_packs::knowledge::descriptor::KNOWLEDGE_DESCRIPTOR.version)
            );
            assert_eq!(
                value["api_version"],
                json!(
                    crate::capability_packs::knowledge::descriptor::KNOWLEDGE_DESCRIPTOR
                        .api_version
                )
            );
            assert_eq!(
                value["operation"],
                Value::String(expected_operation.to_string())
            );
            if let Some(id) = value.get("ingester_id").and_then(Value::as_str) {
                assert_eq!(id, expected_operation.as_str());
            }
            if let Some(cap) = value.get("invoking_capability_id").and_then(Value::as_str) {
                assert_eq!(cap, "knowledge");
            }
        }
    })
}

fn then_latest_relation_provenance_has_fields(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let relation = world
            .knowledge_harness_mut()
            .latest_relation()
            .expect("read latest relation")
            .expect("latest relation should exist");
        let provenance: Value = serde_json::from_str(relation.provenance_json.as_str())
            .expect("parse relation provenance");

        for row in table_row_maps(&ctx) {
            let key = row
                .get("key")
                .map(String::as_str)
                .expect("key column should exist");
            let expected = normalize_expected_id(
                world,
                row.get("value")
                    .map(String::as_str)
                    .expect("value column should exist"),
            );
            let actual = provenance
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            assert_eq!(
                actual, expected,
                "unexpected provenance value for key `{key}`"
            );
        }
    })
}

fn then_relation_target_types_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let relations = world
            .knowledge_harness_mut()
            .all_relations()
            .expect("read all relations");
        for row in table_row_maps(&ctx) {
            let expected = row
                .get("target_type")
                .map(String::as_str)
                .expect("target_type column should exist");
            assert!(
                relations
                    .iter()
                    .any(|relation| relation.target_type == expected),
                "expected target_type `{expected}` in relation set"
            );
        }
    })
}

fn then_relation_target_ids_include(
    world: &mut DevqlBddWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let relations = world
            .knowledge_harness_mut()
            .all_relations()
            .expect("read all relations");
        for row in table_row_maps(&ctx) {
            let expected = normalize_expected_id(
                world,
                row.get("target_id")
                    .map(String::as_str)
                    .expect("target_id column should exist"),
            );
            assert!(
                relations
                    .iter()
                    .any(|relation| relation.target_id == expected),
                "expected target_id `{expected}` in relation set"
            );
        }
    })
}

pub(super) fn register(collection: Collection<DevqlBddWorld>) -> Collection<DevqlBddWorld> {
    collection
        .given(
            None,
            regex(r"^a Knowledge test workspace with configured providers$"),
            step_fn(given_knowledge_workspace),
        )
        .given(
            None,
            regex(r#"^(GitHub|Jira|Confluence) knowledge for "([^"]+)" returns:$"#),
            step_fn(given_provider_returns),
        )
        .given(
            None,
            regex(r#"^(GitHub|Jira|Confluence) knowledge for "([^"]+)" returns in sequence:$"#),
            step_fn(given_provider_returns_sequence),
        )
        .given(
            None,
            regex(r#"^(GitHub|Jira|Confluence) knowledge for "([^"]+)" fails with "([^"]+)"$"#),
            step_fn(given_provider_fails),
        )
        .given(
            None,
            regex(r"^the current repository has a valid HEAD commit$"),
            step_fn(given_valid_head_commit),
        )
        .given(
            None,
            regex(r"^the repository has two valid commits$"),
            step_fn(given_two_valid_commits),
        )
        .given(
            None,
            regex(r#"^a checkpoint "([^"]+)" exists$"#),
            step_fn(given_checkpoint_exists),
        )
        .given(
            None,
            regex(r#"^an artefact "([^"]+)" exists$"#),
            step_fn(given_artefact_exists),
        )
        .given(
            None,
            regex(r#"^the developer has already added knowledge from "([^"]+)"$"#),
            step_fn(given_added_knowledge),
        )
        .given(
            None,
            regex(r#"^the developer has already added knowledge from "([^"]+)" as "([^"]+)"$"#),
            step_fn(given_added_knowledge_as_alias),
        )
        .given(
            None,
            regex(r#"^the developer has already added two versions from "([^"]+)"$"#),
            step_fn(given_added_two_versions),
        )
        .given(
            None,
            regex(r#"^the developer has already added two versions from "([^"]+)" as "([^"]+)"$"#),
            step_fn(given_added_two_versions_as_alias),
        )
        .when(
            None,
            regex(r#"^the developer adds knowledge from "([^"]+)"$"#),
            step_fn(when_add),
        )
        .when(
            None,
            regex(r#"^the developer adds knowledge from "([^"]+)" and attaches it to "([^"]+)"$"#),
            step_fn(when_add_with_commit),
        )
        .when(
            None,
            regex(r#"^the developer associates "([^"]+)" to "([^"]+)"$"#),
            step_fn(when_associate),
        )
        .then(
            None,
            regex(r"^the last operation succeeds$"),
            step_fn(then_last_operation_succeeds),
        )
        .then(
            None,
            regex(r#"^the operation fails with message containing "([^"]+)"$"#),
            step_fn(then_operation_fails_with_message),
        )
        .then(
            None,
            regex(r"^exactly (\d+) knowledge items exist$"),
            step_fn(then_exactly_knowledge_items_exist),
        )
        .then(
            None,
            regex(r"^exactly (\d+) knowledge document versions exist$"),
            step_fn(then_exactly_versions_exist),
        )
        .then(
            None,
            regex(r"^exactly (\d+) knowledge relation assertions exist$"),
            step_fn(then_exactly_relations_exist),
        )
        .then(
            None,
            regex(r#"^the last ingest provider is "([^"]+)"$"#),
            step_fn(then_last_ingest_provider_is),
        )
        .then(
            None,
            regex(r#"^the last ingest source kind is "([^"]+)"$"#),
            step_fn(then_last_ingest_source_kind_is),
        )
        .then(
            None,
            regex(r#"^the relation target type is "([^"]+)"$"#),
            step_fn(then_relation_target_type_is),
        )
        .then(
            None,
            regex(r#"^the relation target id equals "([^"]+)"$"#),
            step_fn(then_relation_target_id_equals),
        )
        .then(
            None,
            regex(r#"^the relation source version equals "([^"]+)"$"#),
            step_fn(then_relation_source_version_equals),
        )
        .then(
            None,
            regex(r#"^the relation type is "([^"]+)"$"#),
            step_fn(then_relation_type_is),
        )
        .then(
            None,
            regex(r#"^the association method is "([^"]+)"$"#),
            step_fn(then_association_method_is),
        )
        .then(
            None,
            regex(r"^the last two ingests reuse the same knowledge item id$"),
            step_fn(then_last_two_ingests_reuse_item_id),
        )
        .then(
            None,
            regex(r"^the last two ingests reuse the same knowledge item version id$"),
            step_fn(then_last_two_ingests_reuse_version_id),
        )
        .then(
            None,
            regex(r"^the last two ingests have different knowledge item version ids$"),
            step_fn(then_last_two_ingests_have_different_versions),
        )
        .then(
            None,
            regex(r#"^all knowledge source rows are stamped for "([^"]+)"$"#),
            step_fn(then_all_source_rows_stamped_for),
        )
        .then(
            None,
            regex(r#"^all knowledge item rows are stamped for "([^"]+)"$"#),
            step_fn(then_all_item_rows_stamped_for),
        )
        .then(
            None,
            regex(r"^the latest relation provenance has fields:$"),
            step_fn(then_latest_relation_provenance_has_fields),
        )
        .then(
            None,
            regex(r"^relation target types include:$"),
            step_fn(then_relation_target_types_include),
        )
        .then(
            None,
            regex(r"^relation target ids include:$"),
            step_fn(then_relation_target_ids_include),
        )
}
