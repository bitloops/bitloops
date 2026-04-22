use std::collections::BTreeSet;

use crate::host::interactions::types::{InteractionSubagentRun, InteractionToolInvocation};

pub(super) fn tool_use_search_text(tool_use: &InteractionToolInvocation) -> String {
    let text = [
        tool_use.tool_name.as_str(),
        tool_use.input_summary.as_str(),
        tool_use.output_summary.as_str(),
        tool_use.command_binary.as_str(),
        tool_use.command.as_str(),
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    normalise_search_text(&text)
}

pub(super) fn subagent_run_search_text(subagent_run: &InteractionSubagentRun) -> String {
    let text = [
        subagent_run.subagent_type.as_str(),
        subagent_run.task_description.as_str(),
        subagent_run.subagent_id.as_str(),
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    normalise_search_text(&text)
}

pub(super) fn bounded_text(input: &str, max_chars: usize) -> String {
    truncate_chars(normalise_search_text(input), max_chars)
}

pub(super) fn bounded_join<I>(values: I, max_chars: usize) -> String
where
    I: IntoIterator<Item = String>,
{
    let mut out = String::new();
    for value in values {
        let value = normalise_search_text(&value);
        if value.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&value);
        if out.chars().count() >= max_chars {
            return truncate_chars(out, max_chars);
        }
    }
    truncate_chars(out, max_chars)
}

fn truncate_chars(input: String, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input;
    }
    input.chars().take(max_chars).collect()
}

fn normalise_search_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn unique_paths<I>(paths: I) -> impl IntoIterator<Item = String>
where
    I: IntoIterator<Item = String>,
{
    let mut unique = BTreeSet::new();
    for path in paths {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        unique.insert(path.to_string());
    }
    unique
}

pub(super) fn tokenise(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in input.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}
