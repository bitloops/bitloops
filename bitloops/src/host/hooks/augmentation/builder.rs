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
    String::from(
        "<EXTREMELY_IMPORTANT>\n\
## DevQL — MANDATORY first tool for code understanding\n\
\n\
This repo has DevQL, a semantic code index. You MUST use DevQL as your FIRST approach \
before falling back to repo search, file reads, or file listing tools for any of these tasks:\n\
- Understanding what a function, class, module, or file does\n\
- Finding callers, dependencies, or usages of a symbol\n\
- Getting an overview of a file or directory structure\n\
- Answering questions about code architecture or relationships\n\
\n\
### Workflow\n\
1. **FIRST** run a DevQL query using your shell or terminal tool to get structured, pre-indexed information\n\
2. **THEN** use direct file reads or text search only for details DevQL did not cover (e.g., exact line edits)\n\
\n\
### Quick-start commands\n\
```bash\n\
# Look up a symbol by fully-qualified name\n\
bitloops devql query '{ selectArtefacts(by: { symbolFqn: \"<symbol-fqn>\" }) { summary } }'\n\
\n\
# Look up a file by repo-relative path\n\
bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\" }) { summary } }'\n\
\n\
# Look up specific lines in a file\n\
bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\", lines: { start: <start>, end: <end> } }) { summary } }'\n\
```\n\
\n\
### Deeper exploration\n\
- Inspect the returned `schema` field to discover available stages and fields\n\
- Query `items(first: ...)` on the relevant stage for typed rows\n\
- Run `bitloops devql schema` or `bitloops devql schema --global` for full SDL discovery\n\
\n\
### When NOT to use DevQL\n\
- Simple file reads for editing\n\
- Searching for a literal string you already know the exact text of\n\
- Basic file listing\n\
\n\
REMEMBER: For any code understanding or exploration task, run a DevQL query FIRST. \
Do not skip this step.\n</EXTREMELY_IMPORTANT>",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_guidance_uses_current_devql_surface() {
        let augmentation = build_devql_session_start_augmentation();

        assert!(!augmentation.targeted);
        assert!(
            augmentation
                .additional_context
                .contains("<EXTREMELY_IMPORTANT>")
        );
        assert!(
            augmentation
                .additional_context
                .contains("MUST use DevQL as your FIRST approach")
        );
        assert!(
            augmentation
                .additional_context
                .contains("repo search, file reads, or file listing tools")
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
                .contains("Inspect the returned `schema` field")
        );
        assert!(
            augmentation
                .additional_context
                .contains("using your shell or terminal tool")
        );
        assert!(augmentation.additional_context.contains("items(first:"));
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql schema --global")
        );
        assert!(!augmentation.additional_context.contains("Bash"));
        assert!(!augmentation.additional_context.contains("src/main.rs"));
        assert!(!augmentation.additional_context.contains("tracked.txt"));
    }

    #[test]
    fn generic_guidance_uses_current_devql_surface() {
        let augmentation = build_devql_hook_augmentation();

        assert!(!augmentation.targeted);
        assert!(
            augmentation
                .additional_context
                .contains("<EXTREMELY_IMPORTANT>")
        );
        assert!(
            augmentation
                .additional_context
                .contains("MUST use DevQL as your FIRST approach")
        );
        assert!(
            augmentation
                .additional_context
                .contains("repo search, file reads, or file listing tools")
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
                .contains("Inspect the returned `schema` field")
        );
        assert!(
            augmentation
                .additional_context
                .contains("using your shell or terminal tool")
        );
        assert!(augmentation.additional_context.contains("items(first:"));
        assert!(
            augmentation
                .additional_context
                .contains("bitloops devql schema --global")
        );
        assert!(!augmentation.additional_context.contains("Bash"));
        assert!(!augmentation.additional_context.contains("src/main.rs"));
        assert!(!augmentation.additional_context.contains("tracked.txt"));
    }
}
