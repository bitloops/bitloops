use std::collections::{HashMap, HashSet};

use crate::capability_packs::test_harness::event_handlers::ExistingTestArtefactIdentityRow;
use crate::capability_packs::test_harness::identity::{
    stable_test_identity_key, test_duplicate_aware_symbol_id, test_edge_id,
    test_revision_artefact_id,
};
use crate::models::{ProductionArtefact, TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DraftTestArtefactId(pub(crate) usize);

#[derive(Debug, Clone)]
pub(crate) struct DraftTestArtefact {
    pub(crate) draft_id: DraftTestArtefactId,
    pub(crate) parent_draft_id: Option<DraftTestArtefactId>,
    pub(crate) record: TestArtefactCurrentRecord,
}

#[derive(Debug, Clone)]
pub(crate) struct DraftTestEdge {
    pub(crate) from_draft_id: DraftTestArtefactId,
    pub(crate) production: ProductionArtefact,
    pub(crate) path: String,
    pub(crate) content_id: String,
    pub(crate) language: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedTestIdentityOutput {
    pub(crate) test_artefacts: Vec<TestArtefactCurrentRecord>,
    pub(crate) test_edges: Vec<TestArtefactEdgeCurrentRecord>,
}

pub(crate) fn resolve_test_identities(
    repo_id: &str,
    existing: &[ExistingTestArtefactIdentityRow],
    drafts: Vec<DraftTestArtefact>,
    draft_edges: Vec<DraftTestEdge>,
) -> ResolvedTestIdentityOutput {
    let mut suite_drafts = Vec::new();
    let mut scenario_drafts = Vec::new();
    for draft in drafts {
        if draft.record.canonical_kind == "test_suite" {
            suite_drafts.push(draft);
        } else {
            scenario_drafts.push(draft);
        }
    }

    let mut test_artefacts = Vec::new();
    let mut final_index_by_draft_id = HashMap::new();

    resolve_draft_groups(
        repo_id,
        existing,
        suite_drafts,
        &mut test_artefacts,
        &mut final_index_by_draft_id,
        true,
    );

    for draft in &mut scenario_drafts {
        if let Some(parent_draft_id) = draft.parent_draft_id
            && let Some(parent_index) = final_index_by_draft_id.get(&parent_draft_id)
        {
            let parent = &test_artefacts[*parent_index];
            draft.record.parent_symbol_id = Some(parent.symbol_id.clone());
            draft.record.parent_artefact_id = Some(parent.artefact_id.clone());
        }
    }

    resolve_draft_groups(
        repo_id,
        existing,
        scenario_drafts,
        &mut test_artefacts,
        &mut final_index_by_draft_id,
        false,
    );

    let test_edges = resolve_edges(
        repo_id,
        &test_artefacts,
        &final_index_by_draft_id,
        draft_edges,
    );

    ResolvedTestIdentityOutput {
        test_artefacts,
        test_edges,
    }
}

fn resolve_draft_groups(
    repo_id: &str,
    existing: &[ExistingTestArtefactIdentityRow],
    drafts: Vec<DraftTestArtefact>,
    test_artefacts: &mut Vec<TestArtefactCurrentRecord>,
    final_index_by_draft_id: &mut HashMap<DraftTestArtefactId, usize>,
    collapse_duplicate_source_suites: bool,
) {
    let mut groups: HashMap<String, Vec<DraftTestArtefact>> = HashMap::new();
    for draft in drafts {
        groups
            .entry(base_key_for_record(&draft.record))
            .or_default()
            .push(draft);
    }

    let mut ordered_groups = groups.into_values().collect::<Vec<_>>();
    ordered_groups.sort_by(|left, right| {
        let left_record = &left[0].record;
        let right_record = &right[0].record;
        (
            left_record.path.as_str(),
            left_record.start_line,
            left[0].draft_id.0,
        )
            .cmp(&(
                right_record.path.as_str(),
                right_record.start_line,
                right[0].draft_id.0,
            ))
    });

    for mut group in ordered_groups {
        group.sort_by_key(|draft| (draft.record.start_line, draft.draft_id.0));

        if collapse_duplicate_source_suites
            && group.len() > 1
            && group
                .iter()
                .all(|draft| draft.record.discovery_source == "source")
        {
            let mut record = group[0].record.clone();
            record.start_line = group
                .iter()
                .map(|draft| draft.record.start_line)
                .min()
                .unwrap_or(record.start_line);
            record.end_line = group
                .iter()
                .map(|draft| draft.record.end_line)
                .max()
                .unwrap_or(record.end_line);
            finalize_record(repo_id, &mut record, None);

            let final_index = test_artefacts.len();
            test_artefacts.push(record);
            for draft in group {
                final_index_by_draft_id.insert(draft.draft_id, final_index);
            }
            continue;
        }

        let duplicate_tokens = duplicate_tokens_for_group(existing, &group);
        for (draft, duplicate_token) in group.into_iter().zip(duplicate_tokens) {
            let mut record = draft.record;
            finalize_record(repo_id, &mut record, duplicate_token.as_deref());
            let final_index = test_artefacts.len();
            test_artefacts.push(record);
            final_index_by_draft_id.insert(draft.draft_id, final_index);
        }
    }
}

fn duplicate_tokens_for_group(
    existing: &[ExistingTestArtefactIdentityRow],
    group: &[DraftTestArtefact],
) -> Vec<Option<String>> {
    if group.len() == 1 {
        return vec![None];
    }

    let base_key = base_key_for_record(&group[0].record);
    let mut matching_existing = existing
        .iter()
        .enumerate()
        .filter(|(_, row)| base_key_for_existing_row(row) == base_key)
        .filter(|(_, row)| row.discovery_source == group[0].record.discovery_source)
        .filter_map(|(index, row)| {
            duplicate_index_for_existing_row(row).map(|duplicate_index| {
                ExistingDuplicateCandidate {
                    index,
                    duplicate_index,
                    start_line: row.start_line,
                    end_line: row.end_line,
                }
            })
        })
        .collect::<Vec<_>>();
    matching_existing.sort_by_key(|row| (row.start_line, row.end_line, row.index));

    let mut assigned = vec![None; group.len()];
    let mut used_duplicate_indexes = HashSet::new();

    let mut next_draft_index = 0;
    for existing_row in &matching_existing {
        if next_draft_index >= group.len() {
            break;
        }
        if used_duplicate_indexes.insert(existing_row.duplicate_index) {
            assigned[next_draft_index] = Some(existing_row.duplicate_index);
            next_draft_index += 1;
        }
    }

    let mut next_duplicate_index = 0;
    assigned
        .into_iter()
        .map(|duplicate_index| {
            let duplicate_index = duplicate_index.unwrap_or_else(|| {
                while used_duplicate_indexes.contains(&next_duplicate_index) {
                    next_duplicate_index += 1;
                }
                let allocated = next_duplicate_index;
                used_duplicate_indexes.insert(allocated);
                next_duplicate_index += 1;
                allocated
            });
            Some(format!("duplicate:{duplicate_index}"))
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct ExistingDuplicateCandidate {
    index: usize,
    duplicate_index: usize,
    start_line: i64,
    end_line: i64,
}

fn duplicate_index_for_existing_row(row: &ExistingTestArtefactIdentityRow) -> Option<usize> {
    (0..1024).find(|duplicate_index| {
        let token = format!("duplicate:{duplicate_index}");
        test_duplicate_aware_symbol_id(
            &row.path,
            &row.canonical_kind,
            row.language_kind.as_deref(),
            row.parent_symbol_id.as_deref(),
            &row.name,
            row.signature.as_deref(),
            Some(&token),
        ) == row.symbol_id
    })
}

fn finalize_record(
    repo_id: &str,
    record: &mut TestArtefactCurrentRecord,
    duplicate_token: Option<&str>,
) {
    record.symbol_id = test_duplicate_aware_symbol_id(
        &record.path,
        &record.canonical_kind,
        record.language_kind.as_deref(),
        record.parent_symbol_id.as_deref(),
        &record.name,
        record.signature.as_deref(),
        duplicate_token,
    );
    record.artefact_id = test_revision_artefact_id(repo_id, &record.content_id, &record.symbol_id);
}

fn resolve_edges(
    repo_id: &str,
    test_artefacts: &[TestArtefactCurrentRecord],
    final_index_by_draft_id: &HashMap<DraftTestArtefactId, usize>,
    draft_edges: Vec<DraftTestEdge>,
) -> Vec<TestArtefactEdgeCurrentRecord> {
    let mut test_edges = Vec::new();
    let mut link_keys = HashSet::new();

    for draft_edge in draft_edges {
        let Some(final_index) = final_index_by_draft_id.get(&draft_edge.from_draft_id) else {
            continue;
        };
        let from = &test_artefacts[*final_index];
        let link_key = format!(
            "{}::{}::tests",
            from.symbol_id, draft_edge.production.symbol_id
        );
        if !link_keys.insert(link_key) {
            continue;
        }

        test_edges.push(TestArtefactEdgeCurrentRecord {
            edge_id: test_edge_id(
                repo_id,
                &from.symbol_id,
                "tests",
                &draft_edge.production.symbol_id,
            ),
            repo_id: repo_id.to_string(),
            content_id: draft_edge.content_id,
            path: draft_edge.path,
            from_artefact_id: from.artefact_id.clone(),
            from_symbol_id: from.symbol_id.clone(),
            to_artefact_id: Some(draft_edge.production.artefact_id),
            to_symbol_id: Some(draft_edge.production.symbol_id),
            to_symbol_ref: None,
            edge_kind: "tests".to_string(),
            language: draft_edge.language,
            start_line: Some(from.start_line),
            end_line: Some(from.end_line),
            metadata:
                r#"{"confidence":0.6,"link_source":"static_analysis","linkage_status":"resolved"}"#
                    .to_string(),
        });
    }

    test_edges
}

fn base_key_for_record(record: &TestArtefactCurrentRecord) -> String {
    stable_test_identity_key(
        &record.path,
        &record.canonical_kind,
        record.language_kind.as_deref(),
        record.parent_symbol_id.as_deref(),
        &record.name,
        record.signature.as_deref(),
    )
}

fn base_key_for_existing_row(row: &ExistingTestArtefactIdentityRow) -> String {
    stable_test_identity_key(
        &row.path,
        &row.canonical_kind,
        row.language_kind.as_deref(),
        row.parent_symbol_id.as_deref(),
        &row.name,
        row.signature.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{DraftTestArtefact, DraftTestArtefactId, resolve_test_identities};
    use crate::capability_packs::test_harness::event_handlers::ExistingTestArtefactIdentityRow;
    use crate::capability_packs::test_harness::identity::{
        test_duplicate_aware_symbol_id, test_revision_artefact_id, test_structural_symbol_id,
    };
    use crate::models::TestArtefactCurrentRecord;

    #[test]
    fn resolver_leaves_unique_base_identity_unchanged() {
        let draft = scenario_draft(0, "unique case", 4, 6);
        let expected = test_structural_symbol_id(
            "tests/dupes.spec.ts",
            "test_scenario",
            None,
            Some("suite-symbol"),
            "unique case",
            Some("unique case"),
        );

        let resolved = resolve_test_identities("repo-1", &[], vec![draft], Vec::new());

        assert_eq!(resolved.test_artefacts.len(), 1);
        assert_eq!(resolved.test_artefacts[0].symbol_id, expected);
    }

    #[test]
    fn resolver_disambiguates_duplicate_scenarios_without_line_span() {
        let resolved = resolve_test_identities(
            "repo-1",
            &[],
            vec![
                scenario_draft(0, "options with multilines", 4, 6),
                scenario_draft(1, "options with multilines", 8, 10),
            ],
            Vec::new(),
        );

        let symbol_ids = resolved
            .test_artefacts
            .iter()
            .map(|artefact| artefact.symbol_id.as_str())
            .collect::<HashSet<_>>();
        let artefact_ids = resolved
            .test_artefacts
            .iter()
            .map(|artefact| artefact.artefact_id.as_str())
            .collect::<HashSet<_>>();

        assert_eq!(symbol_ids.len(), 2);
        assert_eq!(artefact_ids.len(), 2);
        let expected = HashSet::from([
            duplicate_symbol_id("duplicate:0"),
            duplicate_symbol_id("duplicate:1"),
        ]);
        assert_eq!(
            symbol_ids
                .into_iter()
                .map(str::to_string)
                .collect::<HashSet<_>>(),
            expected
        );
    }

    #[test]
    fn resolver_keeps_duplicate_symbol_ids_when_lines_shift() {
        let original = duplicate_symbol_ids(4, 8);
        let shifted = duplicate_symbol_ids(9, 13);

        assert_eq!(original, shifted);
    }

    #[test]
    fn resolver_reuses_existing_duplicate_symbol_ids_by_source_order_after_line_shift() {
        let duplicate_zero = duplicate_symbol_id("duplicate:0");
        let duplicate_one = duplicate_symbol_id("duplicate:1");
        let existing = vec![
            existing_row(&duplicate_zero, 4),
            existing_row(&duplicate_one, 8),
        ];

        let resolved = resolve_test_identities(
            "repo-1",
            &existing,
            vec![
                scenario_draft(0, "options with multilines", 9, 11),
                scenario_draft(1, "options with multilines", 13, 15),
            ],
            Vec::new(),
        );

        let by_source_order = resolved
            .test_artefacts
            .iter()
            .map(|artefact| (artefact.start_line, artefact.symbol_id.as_str()))
            .collect::<Vec<_>>();

        assert_eq!(
            by_source_order,
            vec![(9, duplicate_zero.as_str()), (13, duplicate_one.as_str())]
        );
    }

    fn duplicate_symbol_ids(first_line: i64, second_line: i64) -> Vec<String> {
        let resolved = resolve_test_identities(
            "repo-1",
            &[],
            vec![
                scenario_draft(0, "options with multilines", first_line, first_line + 2),
                scenario_draft(1, "options with multilines", second_line, second_line + 2),
            ],
            Vec::new(),
        );
        let mut symbol_ids = resolved
            .test_artefacts
            .into_iter()
            .map(|artefact| artefact.symbol_id)
            .collect::<Vec<_>>();
        symbol_ids.sort();
        symbol_ids
    }

    fn scenario_draft(id: usize, name: &str, start_line: i64, end_line: i64) -> DraftTestArtefact {
        let symbol_id = test_structural_symbol_id(
            "tests/dupes.spec.ts",
            "test_scenario",
            None,
            Some("suite-symbol"),
            name,
            Some(name),
        );
        DraftTestArtefact {
            draft_id: DraftTestArtefactId(id),
            parent_draft_id: None,
            record: TestArtefactCurrentRecord {
                artefact_id: test_revision_artefact_id("repo-1", "content-1", &symbol_id),
                symbol_id,
                repo_id: "repo-1".to_string(),
                content_id: "content-1".to_string(),
                path: "tests/dupes.spec.ts".to_string(),
                language: "typescript".to_string(),
                canonical_kind: "test_scenario".to_string(),
                language_kind: None,
                symbol_fqn: Some(format!("parse positives.{name}")),
                name: name.to_string(),
                parent_artefact_id: Some("suite-artefact".to_string()),
                parent_symbol_id: Some("suite-symbol".to_string()),
                start_line,
                end_line,
                start_byte: None,
                end_byte: None,
                signature: Some(name.to_string()),
                modifiers: "[]".to_string(),
                docstring: None,
                discovery_source: "source".to_string(),
            },
        }
    }

    fn duplicate_symbol_id(token: &str) -> String {
        test_duplicate_aware_symbol_id(
            "tests/dupes.spec.ts",
            "test_scenario",
            None,
            Some("suite-symbol"),
            "options with multilines",
            Some("options with multilines"),
            Some(token),
        )
    }

    fn existing_row(symbol_id: &str, start_line: i64) -> ExistingTestArtefactIdentityRow {
        ExistingTestArtefactIdentityRow {
            path: "tests/dupes.spec.ts".to_string(),
            symbol_id: symbol_id.to_string(),
            canonical_kind: "test_scenario".to_string(),
            language_kind: None,
            name: "options with multilines".to_string(),
            parent_symbol_id: Some("suite-symbol".to_string()),
            start_line,
            end_line: start_line + 2,
            signature: Some("options with multilines".to_string()),
            discovery_source: "source".to_string(),
        }
    }
}
