use super::prompt_target::PromptTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAugmentation {
    pub additional_context: String,
    pub targeted: bool,
}

pub fn build_devql_hook_augmentation(target: Option<&PromptTarget>) -> HookAugmentation {
    let query = match target {
        Some(target) => targeted_summary_query(target),
        None => String::from(
            "bitloops devql query '{ selectArtefacts(by: { path: \"src/main.rs\" }) { summary } }'",
        ),
    };

    let additional_context = format!(
        "DevQL is available in this repo. Start with `{query}` to get a compact summary of what Bitloops knows. Read the returned `schema` only if you need the drill-down shape, then query `items(first: ...)` on the relevant stage for typed rows. Use `bitloops devql schema` when you need the slim SDL and `bitloops devql schema --global` when you need the full global SDL."
    );

    HookAugmentation {
        additional_context,
        targeted: target.is_some(),
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
    fn generic_guidance_uses_current_devql_surface() {
        let augmentation = build_devql_hook_augmentation(None);

        assert!(!augmentation.targeted);
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
    }
}
