use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result as AnyhowResult;
use serde_json::Value;

use super::catalog::pack_metadata;
use super::models::{PackMetadata, PackSelectionState, ResolvedSelection};
use super::values::read_string_list;

pub(super) fn resolve_selection(
    repo_root: &Path,
    daemon_value: &Value,
    explicit_enabled_override: &[String],
    explicit_disabled_override: &[String],
) -> AnyhowResult<ResolvedSelection> {
    let metadata = pack_metadata(repo_root)?;
    let by_id = metadata
        .iter()
        .map(|item| (item.id.clone(), item.clone()))
        .collect::<BTreeMap<_, _>>();

    let policy_enabled = if explicit_enabled_override.is_empty() {
        read_string_list(
            daemon_value,
            &["runtime", "capability_policy", "explicit_enabled"],
        )
    } else {
        explicit_enabled_override
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    };
    let policy_disabled = if explicit_disabled_override.is_empty() {
        read_string_list(
            daemon_value,
            &["runtime", "capability_policy", "explicit_disabled"],
        )
    } else {
        explicit_disabled_override
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    };

    let explicit_enabled = if policy_enabled.is_empty() && policy_disabled.is_empty() {
        metadata
            .iter()
            .filter(|item| item.default_enabled)
            .map(|item| item.id.clone())
            .collect::<BTreeSet<_>>()
    } else {
        policy_enabled.into_iter().collect::<BTreeSet<_>>()
    };
    let explicit_disabled = policy_disabled.into_iter().collect::<BTreeSet<_>>();

    let mut states = BTreeMap::new();
    let mut blockers = Vec::new();
    let mut visited = BTreeSet::new();
    let mut active_stack = Vec::<String>::new();

    struct SelectionVisitContext<'a> {
        by_id: &'a BTreeMap<String, PackMetadata>,
        explicit_disabled: &'a BTreeSet<String>,
        states: &'a mut BTreeMap<String, PackSelectionState>,
        visited: &'a mut BTreeSet<String>,
        active_stack: &'a mut Vec<String>,
        blockers: &'a mut Vec<String>,
    }

    fn visit(id: &str, ctx: &mut SelectionVisitContext<'_>, source: &str, reason: Option<String>) {
        if ctx.visited.contains(id) {
            if let Some(existing) = ctx.states.get_mut(id)
                && existing.selection_source != "explicit"
                && source == "explicit"
            {
                existing.selection_source = "explicit".to_string();
                existing.dependency_reason = None;
            }
            return;
        }
        if ctx.active_stack.iter().any(|entry| entry == id) {
            let mut chain = ctx.active_stack.clone();
            chain.push(id.to_string());
            ctx.blockers
                .push(format!("dependency cycle detected: {}", chain.join(" -> ")));
            return;
        }
        let Some(metadata) = ctx.by_id.get(id) else {
            ctx.blockers.push(format!("missing dependency `{id}`"));
            return;
        };
        if ctx.explicit_disabled.contains(id) && source != "explicit" {
            ctx.blockers.push(format!(
                "dependency `{id}` is explicitly disabled but required by another pack"
            ));
            return;
        }
        let display_name = metadata.display_name.clone();
        let dependencies = metadata.dependencies.clone();
        ctx.active_stack.push(id.to_string());
        for dependency in &dependencies {
            visit(
                dependency.pack_id.as_str(),
                ctx,
                "dependency",
                Some(format!("required by {display_name}")),
            );
        }
        ctx.active_stack.pop();
        ctx.visited.insert(id.to_string());
        ctx.states.insert(
            id.to_string(),
            PackSelectionState {
                enabled: true,
                selection_source: source.to_string(),
                dependency_reason: reason,
            },
        );
    }

    {
        let mut visit_ctx = SelectionVisitContext {
            by_id: &by_id,
            explicit_disabled: &explicit_disabled,
            states: &mut states,
            visited: &mut visited,
            active_stack: &mut active_stack,
            blockers: &mut blockers,
        };
        for id in &explicit_enabled {
            visit(id, &mut visit_ctx, "explicit", None);
        }
    }

    for id in by_id.keys() {
        states.entry(id.clone()).or_insert(PackSelectionState {
            enabled: false,
            selection_source: "default".to_string(),
            dependency_reason: None,
        });
    }

    Ok(ResolvedSelection { states, blockers })
}
