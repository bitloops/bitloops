pub const USING_DEVQL_SKILL: &str = include_str!("skills/using-devql/SKILL.md");

pub fn using_devql_skill_body() -> &'static str {
    USING_DEVQL_SKILL
        .splitn(3, "---")
        .nth(2)
        .map(str::trim_start)
        .unwrap_or(USING_DEVQL_SKILL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn using_devql_skill_body_strips_skill_frontmatter() {
        let body = using_devql_skill_body();
        assert!(body.contains("# Using DevQL"));
        assert!(!body.starts_with("---"));
        assert!(body.contains("bitloops devql query"));
    }

    #[test]
    fn using_devql_skill_mentions_running_bitloops_devql_outside_sandbox() {
        let body = using_devql_skill_body();
        assert!(body.contains("outside the sandbox"));
        assert!(body.contains("bitloops devql"));
    }
}
