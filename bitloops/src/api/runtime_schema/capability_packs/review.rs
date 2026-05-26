use std::collections::BTreeSet;

use serde_json::Value;

use super::models::CapabilityPackReviewGroupObject;

pub(super) fn build_review_groups(
    daemon_current: &Value,
    daemon_proposed: &Value,
) -> Vec<CapabilityPackReviewGroupObject> {
    let mut groups = Vec::new();
    let daemon_items = diff_value(String::new(), daemon_current, daemon_proposed);
    if !daemon_items.is_empty() {
        groups.push(CapabilityPackReviewGroupObject {
            key: "daemon".to_string(),
            title: "Daemon config changes".to_string(),
            items: daemon_items,
        });
    }
    groups
}

fn diff_value(prefix: String, current: &Value, proposed: &Value) -> Vec<String> {
    if current == proposed {
        return Vec::new();
    }
    match (current, proposed) {
        (Value::Object(current_map), Value::Object(proposed_map)) => {
            let mut keys = current_map
                .keys()
                .chain(proposed_map.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut items = Vec::new();
            for key in keys.split_off("") {
                let next_prefix = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                items.extend(diff_value(
                    next_prefix,
                    current_map.get(&key).unwrap_or(&Value::Null),
                    proposed_map.get(&key).unwrap_or(&Value::Null),
                ));
            }
            items
        }
        _ => vec![format!(
            "{} = {}",
            if prefix.is_empty() {
                "<root>"
            } else {
                prefix.as_str()
            },
            preview_value(proposed)
        )],
    }
}

fn preview_value(value: &Value) -> String {
    match value {
        Value::Null => "unset".to_string(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}
