use super::prompt_target::PromptTarget;

const GENERIC_SUMMARY_QUERY: &str =
    "bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\" }) { summary } }'";
const GENERIC_LINES_SELECTOR: &str = "lines: { start: <start>, end: <end> }";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAugmentation {
    pub additional_context: String,
    pub targeted: bool,
}

pub fn build_devql_session_start_augmentation() -> HookAugmentation {
    HookAugmentation {
        additional_context: format!(
            "DevQL is available in this repo. For file-specific requests, start with `{GENERIC_SUMMARY_QUERY}` by replacing `<repo-relative-path>` with the relevant repo path. To narrow the query to a region, add `{GENERIC_LINES_SELECTOR}` inside the `by` selector. Read stage `schema` only if needed, then query `items(first: ...)` for typed rows. Use `bitloops devql schema` or `bitloops devql schema --global` for SDL discovery.",
        ),
        targeted: false,
    }
}

pub fn build_devql_hook_augmentation(target: Option<&PromptTarget>) -> HookAugmentation {
    let targeted = target.is_some();
    let additional_context = match target {
        Some(target) => {
            let query = targeted_summary_query(target);
            format!(
                "DevQL is available in this repo. Start with `{query}` to get a compact summary of what Bitloops knows. Read the returned `schema` only if you need the drill-down shape, then query `items(first: ...)` on the relevant stage for typed rows. Use `bitloops devql schema` when you need the slim SDL and `bitloops devql schema --global` when you need the full global SDL."
            )
        }
        None => {
            let query = String::from(GENERIC_SUMMARY_QUERY);
            format!(
                "DevQL is available in this repo. Start with `{query}` to get a compact summary of what Bitloops knows. If the request is file-specific, replace `<repo-relative-path>` with the relevant path before running it. To narrow the query to a region, add `{GENERIC_LINES_SELECTOR}` inside the `by` selector. Read the returned `schema` only if you need the drill-down shape, then query `items(first: ...)` on the relevant stage for typed rows. Use `bitloops devql schema` when you need the slim SDL and `bitloops devql schema --global` when you need the full global SDL."
            )
        }
    };

    HookAugmentation {
        additional_context,
        targeted,
    }
}

fn targeted_summary_query(target: &PromptTarget) -> String {
    let escaped_path = target.path.replace('"', "\\\"");
    match (target.start_line, target.end_line) {
        (Some(start), Some(end)) => format!(
            "bitloops devql query '{{ selectArtefacts(by: {{ path: \"{escaped_path}\", lines: {{ start: {start}, end: {end} }} }}) {{ summary }} }}'"
        ),
        _ => format!(
            "bitloops devql query '{{ selectArtefacts(by: {{ path: \"{escaped_path}\" }}) {{ summary }} }}'"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_guidance_uses_current_devql_surface() {
        let augmentation = build_devql_session_start_augmentation();

        assert!(!augmentation.targeted);
        assert!(augmentation.additional_context.contains("<repo-relative-path>"));
        assert!(
            augmentation
                .additional_context
                .contains("lines: { start: <start>, end: <end> }")
        );
        assert!(augmentation.additional_context.contains("selectArtefacts"));
        assert!(augmentation.additional_context.contains("summary"));
        assert!(augmentation.additional_context.contains("schema"));
        assert!(augmentation.additional_context.contains("items(first:"));
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql schema")
        );
        assert!(!augmentation.additional_context.contains("availableInfo"));
        assert!(!augmentation.additional_context.contains("menu"));
        assert!(!augmentation.additional_context.contains("src/main.rs"));
    }

    #[test]
    fn generic_guidance_uses_current_devql_surface() {
        let augmentation = build_devql_hook_augmentation(None);

        assert!(!augmentation.targeted);
        assert!(augmentation.additional_context.contains("<repo-relative-path>"));
        assert!(
            augmentation
                .additional_context
                .contains("lines: { start: <start>, end: <end> }")
        );
        assert!(augmentation.additional_context.contains("selectArtefacts"));
        assert!(augmentation.additional_context.contains("summary"));
        assert!(augmentation.additional_context.contains("schema"));
        assert!(augmentation.additional_context.contains("items(first:"));
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql schema")
        );
        assert!(!augmentation.additional_context.contains("availableInfo"));
        assert!(!augmentation.additional_context.contains("menu"));
        assert!(!augmentation.additional_context.contains("src/main.rs"));
    }

    #[test]
    fn targeted_guidance_uses_path_and_lines_example() {
        let target = PromptTarget {
            path: "src/main.rs".to_string(),
            start_line: Some(6),
            end_line: Some(10),
        };

        let augmentation = build_devql_hook_augmentation(Some(&target));

        assert!(augmentation.targeted);
        assert!(augmentation.additional_context.contains("src/main.rs"));
        assert!(augmentation.additional_context.contains("start: 6"));
        assert!(augmentation.additional_context.contains("end: 10"));
        assert!(augmentation.additional_context.contains("selectArtefacts"));
        assert!(augmentation.additional_context.contains("summary"));
        assert!(!augmentation.additional_context.contains("<repo-relative-path>"));
    }
}
