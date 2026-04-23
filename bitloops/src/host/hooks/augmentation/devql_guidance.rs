pub fn build_session_bootstrap(surface_path: &str) -> String {
    format!(
        "<EXTREMELY_IMPORTANT>\n\
This repo has DevQL guidance available.\n\
The repo-local DevQL-capable guidance surface is installed at `{surface_path}`.\n\
When DevQL is available in this session, use DevQL first for repo-aware understanding questions.\n\
That includes questions about architecture, symbol lookup, path/line resolution, callers/usages/imports/tests/dependencies, and \"what does this repo do\".\n\
Start with the most specific selector available: `path`, `path + lines`, `symbolFqn`, or `search`.\n\
Use `search` for approximate or conceptual lookup, then use `overview` once the selection is concrete.\n\
When DevQL returns a follow-up hint such as `expandHint`, use that hint to choose the next query.\n\
If DevQL is unavailable in this session or returns no useful artefacts, fall back to targeted repo search or file reads.\n\
Read the repo-local guidance at `{surface_path}` for the full workflow.\n\
</EXTREMELY_IMPORTANT>"
    )
}

pub fn build_turn_guidance(surface_path: &str) -> String {
    format!(
        "Use DevQL first for this repo-aware request when it is available in this session. \
This applies even to repo-overview questions like \"what does this repo do?\". \
Prefer `path`, `path + lines`, or `symbolFqn` when the target is exact, and `search` when it is approximate or conceptual. \
Once the selection is concrete, use `overview` before broader repo search. \
If the response includes `expandHint`, use it to drive the next query. \
If DevQL is unavailable or returns no useful artefacts, fall back to targeted repo search or file reads. \
Read `{surface_path}` before broad search."
    )
}

pub fn prompt_warrants_devql(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let repo_understanding_terms = [
        "what does this repo do",
        "understand",
        "explain",
        "architecture",
        "where is",
        "find",
        "inspect",
        "caller",
        "usage",
        "import",
        "dependency",
        "test covering",
    ];
    let execution_terms = [
        "fix ",
        "implement ",
        "edit ",
        "write ",
        "run ",
        "build ",
        "test ",
        "format ",
    ];
    let looks_like_code_reference = prompt.contains('/')
        || prompt.contains("::")
        || prompt.contains('`')
        || prompt.contains(':');
    let looks_like_edit_or_execution = execution_terms.iter().any(|needle| lower.contains(needle));
    let looks_like_repo_understanding = repo_understanding_terms
        .iter()
        .any(|needle| lower.contains(needle));

    (looks_like_code_reference || looks_like_repo_understanding) && !looks_like_edit_or_execution
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_session_bootstrap_mentions_search_overview_and_response_hints() {
        let text = build_session_bootstrap(".opencode/skills/bitloops/using-devql/SKILL.md");

        assert!(text.contains("This repo has DevQL guidance available."));
        assert!(text.contains("DevQL-capable guidance surface"));
        assert!(text.contains("When DevQL is available in this session"));
        assert!(text.contains("search"));
        assert!(text.contains("overview"));
        assert!(text.contains("expandHint"));
        assert!(text.contains("fall back to targeted repo search or file reads"));
        assert!(!text.contains("fuzzyName"));
        assert!(!text.contains("naturalLanguage"));
        assert!(!text.contains("semanticQuery"));
    }

    #[test]
    fn build_turn_guidance_mentions_search_overview_and_skill_path() {
        let guidance = build_turn_guidance(".claude/skills/bitloops/using-devql/SKILL.md");

        assert!(guidance.contains("when it is available in this session"));
        assert!(guidance.contains("what does this repo do?"));
        assert!(guidance.contains("path + lines"));
        assert!(guidance.contains("search"));
        assert!(guidance.contains("overview"));
        assert!(guidance.contains("expandHint"));
        assert!(guidance.contains(".claude/skills/bitloops/using-devql/SKILL.md"));
        assert!(guidance.contains("fall back to targeted repo search or file reads"));
        assert!(!guidance.contains("fuzzyName"));
        assert!(!guidance.contains("naturalLanguage"));
        assert!(!guidance.contains("semanticQuery"));
    }

    #[test]
    fn prompt_warrants_devql_accepts_repo_overview_prompts_and_rejects_execution_prompts() {
        assert!(prompt_warrants_devql("What does this repo do?"));
        assert!(prompt_warrants_devql(
            "Help me understand src/payments/invoice.ts:42"
        ));
        assert!(!prompt_warrants_devql("Run cargo fmt"));
        assert!(!prompt_warrants_devql("Implement payment retry handling"));
    }
}
