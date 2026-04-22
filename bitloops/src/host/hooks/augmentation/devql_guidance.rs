use std::path::Path;

use super::prompt_target::extract_primary_prompt_target;

pub fn build_turn_guidance(repo_root: &Path, prompt: &str) -> String {
    if let Some(target) = extract_primary_prompt_target(repo_root, prompt) {
        if let (Some(start), Some(end)) = (target.start_line, target.end_line) {
            return format!(
                "<EXTREMELY_IMPORTANT>\n\
Use DevQL first for this request.\n\
\n\
Suggested command:\n\
```bash\n\
bitloops devql query '{{ selectArtefacts(by: {{ path: \"{}\", lines: {{ start: {}, end: {} }} }}) {{ summary }} }}'\n\
```\n\
\n\
Run this before broad repo search.\n\
</EXTREMELY_IMPORTANT>",
                target.path, start, end
            );
        }

        return format!(
            "<EXTREMELY_IMPORTANT>\n\
Use DevQL first for this request.\n\
\n\
Suggested command:\n\
```bash\n\
bitloops devql query '{{ selectArtefacts(by: {{ path: \"{}\" }}) {{ summary }} }}'\n\
```\n\
\n\
Run this before broad repo search.\n\
</EXTREMELY_IMPORTANT>",
            target.path
        );
    }

    String::from(
        "<EXTREMELY_IMPORTANT>\n\
Use DevQL first for code understanding in this repo.\n\
\n\
Quick-start commands:\n\
```bash\n\
bitloops devql query '{ selectArtefacts(by: { path: \"<repo-relative-path>\" }) { summary } }'\n\
bitloops devql query '{ selectArtefacts(by: { search: \"<natural-language request or approx symbol>\" }) { artefacts(first: 10) { path symbolFqn } } }'\n\
bitloops devql query '{ selectArtefacts(by: { symbolFqn: \"<symbol-fqn>\" }) { summary } }'\n\
bitloops devql schema --global\n\
```\n\
\n\
Run DevQL before broad repo search.\n\
</EXTREMELY_IMPORTANT>",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_turn_guidance_emits_line_scoped_command_for_targeted_prompt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("tracked.txt");
        std::fs::write(&file, "one\n").expect("write tracked file");

        let guidance = build_turn_guidance(dir.path(), "Explain tracked.txt:1");

        assert!(guidance.contains("Use DevQL first for this request."));
        assert!(guidance.contains("tracked.txt"));
        assert!(guidance.contains("start: 1"));
        assert!(guidance.contains("end: 1"));
        assert!(!guidance.contains("<repo-relative-path>"));
    }

    #[test]
    fn build_turn_guidance_includes_fuzzy_lookup_in_generic_guidance() {
        let dir = tempfile::tempdir().expect("tempdir");

        let guidance = build_turn_guidance(dir.path(), "Help me find payLatr()");

        assert!(guidance.contains("search"));
        assert!(guidance.contains("<natural-language request or approx symbol>"));
    }
}
