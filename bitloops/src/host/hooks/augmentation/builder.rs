use super::prompt_target::PromptTarget;

const GENERIC_SUMMARY_QUERY: &str =
    "bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\" }) { summary } }'";
const GENERIC_LINES_QUERY: &str =
    "bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\", lines: { start: <start>, end: <end> } }) { summary } }'";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAugmentation {
    pub additional_context: String,
    pub targeted: bool,
}

pub fn build_devql_session_start_augmentation() -> HookAugmentation {
    HookAugmentation {
        additional_context: generic_devql_guidance(),
        targeted: false,
    }
}

pub fn build_devql_hook_augmentation(target: Option<&PromptTarget>) -> HookAugmentation {
    let targeted = target.is_some();
    let additional_context = match target {
        Some(target) => {
            let query = targeted_summary_query(target);
            targeted_devql_guidance(&query)
        }
        None => generic_devql_guidance(),
    };

    HookAugmentation {
        additional_context,
        targeted,
    }
}

fn generic_devql_guidance() -> String {
    format!(
        "<EXTREMELY_IMPORTANT>\nYou have DevQL available in this repo. You should leverage it for repo-aware requests.\n\nStart with:\n`{GENERIC_SUMMARY_QUERY}`\n\nOptional line targeting:\n`{GENERIC_LINES_QUERY}`\n\nThen:\n- read returned `schema` only if needed\n- query `items(first: ...)` for typed rows\n- use `bitloops devql schema` or `bitloops devql schema --global` for SDL discovery\n</EXTREMELY_IMPORTANT>"
    )
}

fn targeted_devql_guidance(query: &str) -> String {
    format!(
        "<EXTREMELY_IMPORTANT>\nYou should leverage DevQL for this repo-aware request.\n\nStart with:\n`{query}`\n\nThen:\n- read returned `schema` only if needed\n- query `items(first: ...)` for typed rows\n- use `bitloops devql schema` or `bitloops devql schema --global` for SDL discovery\n</EXTREMELY_IMPORTANT>"
    )
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
        assert!(augmentation.additional_context.contains("<EXTREMELY_IMPORTANT>"));
        assert!(augmentation.additional_context.contains("<repo-relative-path>"));
        assert!(
            augmentation
                .additional_context
                .contains("You have DevQL available in this repo. You should leverage it for repo-aware requests.")
        );
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
        assert!(augmentation.additional_context.contains("<EXTREMELY_IMPORTANT>"));
        assert!(augmentation.additional_context.contains("<repo-relative-path>"));
        assert!(
            augmentation
                .additional_context
                .contains("You have DevQL available in this repo. You should leverage it for repo-aware requests.")
        );
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
        assert!(augmentation.additional_context.contains("<EXTREMELY_IMPORTANT>"));
        assert!(
            augmentation
                .additional_context
                .contains("You should leverage DevQL for this repo-aware request.")
        );
        assert!(augmentation.additional_context.contains("src/main.rs"));
        assert!(augmentation.additional_context.contains("start: 6"));
        assert!(augmentation.additional_context.contains("end: 10"));
        assert!(augmentation.additional_context.contains("selectArtefacts"));
        assert!(augmentation.additional_context.contains("summary"));
        assert!(augmentation.additional_context.contains("schema"));
        assert!(!augmentation.additional_context.contains("<repo-relative-path>"));
    }
}
