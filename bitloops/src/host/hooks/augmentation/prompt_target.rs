use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTarget {
    pub path: String,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
}

pub fn extract_primary_prompt_target(repo_root: &Path, prompt: &str) -> Option<PromptTarget> {
    let targets = extract_candidate_prompt_targets(prompt);
    normalize_first_target_for_repo(repo_root, targets)
}

fn extract_candidate_prompt_targets(prompt: &str) -> Vec<PromptTarget> {
    static COLON_LINE_RE: OnceLock<Regex> = OnceLock::new();
    static HASH_LINE_RE: OnceLock<Regex> = OnceLock::new();
    static LINES_PARENS_RE: OnceLock<Regex> = OnceLock::new();
    static FILE_ONLY_RE: OnceLock<Regex> = OnceLock::new();

    let colon_line_re = COLON_LINE_RE.get_or_init(|| {
        Regex::new(r"(?P<path>[A-Za-z0-9_\-./]+\.[A-Za-z0-9_]+):(?P<start>\d+)(?:-(?P<end>\d+))?")
            .expect("valid colon line anchor regex")
    });
    let hash_line_re = HASH_LINE_RE.get_or_init(|| {
        Regex::new(
            r"(?P<path>[A-Za-z0-9_\-./]+\.[A-Za-z0-9_]+)#L(?P<start>\d+)(?:-L?(?P<end>\d+))?",
        )
        .expect("valid hash line anchor regex")
    });
    let lines_parens_re = LINES_PARENS_RE.get_or_init(|| {
        Regex::new(
            r"(?P<path>[A-Za-z0-9_\-./]+\.[A-Za-z0-9_]+)\s*\(lines?\s*(?P<start>\d+)(?:\s*-\s*(?P<end>\d+))?\)",
        )
        .expect("valid lines() anchor regex")
    });
    let file_only_re = FILE_ONLY_RE.get_or_init(|| {
        Regex::new(r"(?P<path>[A-Za-z0-9_\-./]+\.[A-Za-z0-9_]+)").expect("valid file-only regex")
    });

    let mut targets = Vec::<PromptTarget>::new();
    for caps in colon_line_re.captures_iter(prompt) {
        insert_target(
            &mut targets,
            caps.name("path").map(|m| m.as_str()).unwrap_or_default(),
            caps.name("start")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
            caps.name("end")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
        );
    }
    for caps in hash_line_re.captures_iter(prompt) {
        insert_target(
            &mut targets,
            caps.name("path").map(|m| m.as_str()).unwrap_or_default(),
            caps.name("start")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
            caps.name("end")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
        );
    }
    for caps in lines_parens_re.captures_iter(prompt) {
        insert_target(
            &mut targets,
            caps.name("path").map(|m| m.as_str()).unwrap_or_default(),
            caps.name("start")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
            caps.name("end")
                .and_then(|m| m.as_str().parse::<i32>().ok()),
        );
    }

    if targets.is_empty() {
        for caps in file_only_re.captures_iter(prompt) {
            insert_target(
                &mut targets,
                caps.name("path").map(|m| m.as_str()).unwrap_or_default(),
                None,
                None,
            );
        }
    }

    targets
}

fn insert_target(
    targets: &mut Vec<PromptTarget>,
    path: &str,
    start_line: Option<i32>,
    end_line: Option<i32>,
) {
    let Some(path) = sanitize_target_path(path) else {
        return;
    };

    let final_end = end_line.or(start_line);
    if let Some(existing) = targets.iter_mut().find(|target| target.path == path) {
        if existing.start_line.is_none() && start_line.is_some() {
            existing.start_line = start_line;
            existing.end_line = final_end;
        }
        return;
    }

    targets.push(PromptTarget {
        path,
        start_line,
        end_line: final_end,
    });
}

fn sanitize_target_path(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let path = path.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | ',' | ';' | ')' | ']'));
    if path.is_empty() || path.starts_with("http://") || path.starts_with("https://") {
        return None;
    }
    Some(path.to_string())
}

fn normalize_first_target_for_repo(
    repo_root: &Path,
    targets: Vec<PromptTarget>,
) -> Option<PromptTarget> {
    targets.into_iter().find_map(|target| {
        normalize_target_for_repo(repo_root, &target.path).and_then(|repo_relative_path| {
            if repo_root.join(&repo_relative_path).is_file() {
                let mut normalized = target;
                normalized.path = repo_relative_path;
                Some(normalized)
            } else {
                None
            }
        })
    })
}

fn normalize_target_for_repo(repo_root: &Path, raw_path: &str) -> Option<String> {
    let repo_root = repo_root.canonicalize().ok()?;
    let input = Path::new(raw_path);
    let candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        repo_root.join(input)
    };
    let candidate = candidate.canonicalize().ok()?;
    if !candidate.starts_with(&repo_root) {
        return None;
    }
    let rel = candidate.strip_prefix(&repo_root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some(rel.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_colon_line_anchor_inside_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path();
        let src_dir = repo_root.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src");
        std::fs::write(src_dir.join("main.rs"), "fn main() {}\n").expect("write file");

        let target = extract_primary_prompt_target(repo_root, "please inspect src/main.rs:42")
            .expect("target");

        assert_eq!(
            target,
            PromptTarget {
                path: "src/main.rs".to_string(),
                start_line: Some(42),
                end_line: Some(42),
            }
        );
    }

    #[test]
    fn extracts_hash_line_range_inside_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path();
        let src_dir = repo_root.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src");
        std::fs::write(src_dir.join("lib.rs"), "pub fn run() {}\n").expect("write file");

        let target =
            extract_primary_prompt_target(repo_root, "check src/lib.rs#L10-L14").expect("target");

        assert_eq!(
            target,
            PromptTarget {
                path: "src/lib.rs".to_string(),
                start_line: Some(10),
                end_line: Some(14),
            }
        );
    }

    #[test]
    fn falls_back_to_file_only_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path();
        let src_dir = repo_root.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src");
        std::fs::write(src_dir.join("mod.rs"), "pub mod feature;\n").expect("write file");

        let target = extract_primary_prompt_target(repo_root, "explain what src/mod.rs is doing")
            .expect("target");

        assert_eq!(
            target,
            PromptTarget {
                path: "src/mod.rs".to_string(),
                start_line: None,
                end_line: None,
            }
        );
    }

    #[test]
    fn extracts_file_only_target_from_trailing_double_colon_prompt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path();
        let src_dir = repo_root.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src");
        std::fs::write(src_dir.join("main.rs"), "fn main() {}\n").expect("write file");

        let target =
            extract_primary_prompt_target(repo_root, "explain src/main.rs::").expect("target");

        assert_eq!(
            target,
            PromptTarget {
                path: "src/main.rs".to_string(),
                start_line: None,
                end_line: None,
            }
        );
    }

    #[test]
    fn normalizes_absolute_paths_inside_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path();
        let src_dir = repo_root.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src");
        let file_path = src_dir.join("service.rs");
        std::fs::write(&file_path, "pub fn run() {}\n").expect("write file");

        let prompt = format!("review {}:12", file_path.display());
        let target = extract_primary_prompt_target(repo_root, &prompt).expect("target");

        assert_eq!(
            target,
            PromptTarget {
                path: "src/service.rs".to_string(),
                start_line: Some(12),
                end_line: Some(12),
            }
        );
    }

    #[test]
    fn collapses_duplicate_targets_to_first_line_anchored_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path();
        let src_dir = repo_root.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src");
        std::fs::write(src_dir.join("shared.rs"), "pub fn run() {}\n").expect("write file");

        let target = extract_primary_prompt_target(
            repo_root,
            "compare src/shared.rs and src/shared.rs:8 before refactoring",
        )
        .expect("target");

        assert_eq!(
            target,
            PromptTarget {
                path: "src/shared.rs".to_string(),
                start_line: Some(8),
                end_line: Some(8),
            }
        );
    }

    #[test]
    fn ignores_urls_and_paths_outside_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path();
        let src_dir = repo_root.join("src");
        std::fs::create_dir_all(&src_dir).expect("create src");
        std::fs::write(src_dir.join("main.rs"), "fn main() {}\n").expect("write file");

        let outside = tempfile::tempdir().expect("outside");
        let outside_file = outside.path().join("secret.rs");
        std::fs::write(&outside_file, "fn nope() {}\n").expect("write outside");

        assert!(
            extract_primary_prompt_target(repo_root, "https://example.com/src/main.rs").is_none()
        );
        assert!(
            extract_primary_prompt_target(repo_root, outside_file.to_string_lossy().as_ref())
                .is_none()
        );
    }
}
