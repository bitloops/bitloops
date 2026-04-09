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

pub fn build_devql_hook_augmentation() -> HookAugmentation {
    HookAugmentation {
        additional_context: generic_devql_guidance(),
        targeted: false,
    }
}

fn generic_devql_guidance() -> String {
    format!(
        "<EXTREMELY_IMPORTANT>\nYou have DevQL available in this repo. Use it for repo-aware requests when it can replace repeated grep/file reads.\n\nStart with one of:\n- `bitloops devql query '{{ selectArtefacts(by: {{ symbolFqn: \"<symbol-fqn>\" }}) {{ summary }} }}'`\n- `bitloops devql query '{{ selectArtefacts(by: {{ path: \"<repo-relative-path>\" }}) {{ summary }} }}'`\n- `bitloops devql query '{{ selectArtefacts(by: {{ path: \"<repo-relative-path>\", lines: {{ start: <start>, end: <end> }} }}) {{ summary }} }}'`\n\nThen:\n- inspect returned `schema` only if needed\n- query `items(first: ...)` on the relevant stage for typed rows\n- use `bitloops devql schema` or `bitloops devql schema --global` for SDL discovery\n</EXTREMELY_IMPORTANT>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_guidance_uses_current_devql_surface() {
        let augmentation = build_devql_session_start_augmentation();

        assert!(!augmentation.targeted);
        assert!(augmentation.additional_context.contains("<EXTREMELY_IMPORTANT>"));
        assert!(
            augmentation
                .additional_context
                .contains("You have DevQL available in this repo. Use it for repo-aware requests when it can replace repeated grep/file reads.")
        );
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql query '{ selectArtefacts(by: { symbolFqn: \"<symbol-fqn>\" }) { summary } }'")
        );
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\" }) { summary } }'")
        );
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\", lines: { start: <start>, end: <end> } }) { summary } }'")
        );
        assert!(
            augmentation
                .additional_context
                .contains("inspect returned `schema` only if needed")
        );
        assert!(augmentation.additional_context.contains("items(first:"));
        assert!(augmentation.additional_context.contains("bitloops devql schema --global"));
        assert!(!augmentation.additional_context.contains("src/main.rs"));
        assert!(!augmentation.additional_context.contains("tracked.txt"));
    }

    #[test]
    fn generic_guidance_uses_current_devql_surface() {
        let augmentation = build_devql_hook_augmentation();

        assert!(!augmentation.targeted);
        assert!(augmentation.additional_context.contains("<EXTREMELY_IMPORTANT>"));
        assert!(
            augmentation
                .additional_context
                .contains("You have DevQL available in this repo. Use it for repo-aware requests when it can replace repeated grep/file reads.")
        );
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql query '{ selectArtefacts(by: { symbolFqn: \"<symbol-fqn>\" }) { summary } }'")
        );
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\" }) { summary } }'")
        );
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\", lines: { start: <start>, end: <end> } }) { summary } }'")
        );
        assert!(
            augmentation
                .additional_context
                .contains("inspect returned `schema` only if needed")
        );
        assert!(augmentation.additional_context.contains("items(first:"));
        assert!(augmentation.additional_context.contains("bitloops devql schema --global"));
        assert!(!augmentation.additional_context.contains("src/main.rs"));
        assert!(!augmentation.additional_context.contains("tracked.txt"));
    }
}
