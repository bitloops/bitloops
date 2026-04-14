use std::fs;
use std::path::Path;

use tempfile::tempdir;

use crate::config::REPO_POLICY_LOCAL_FILE_NAME;
use crate::host::devql::PLAIN_TEXT_LANGUAGE_ID;

use super::classifier::TRACK_ONLY_LANGUAGE_ID;
use super::{AnalysisMode, FileRole, ProjectAwareClassifier, TextIndexMode};

fn classifier_for(repo: &Path, paths: &[&str]) -> ProjectAwareClassifier {
    ProjectAwareClassifier::discover_for_worktree(repo, paths, "parser-v1", "extractor-v1")
        .expect("build classifier")
}

#[test]
fn contextless_typescript_file_defaults_to_track_only() {
    let dir = tempdir().expect("temp dir");
    let classifier = classifier_for(dir.path(), &["src/main.ts"]);

    let classification = classifier
        .classify_repo_relative_path("src/main.ts", false)
        .expect("classify path");

    assert_eq!(classification.analysis_mode, AnalysisMode::TrackOnly);
    assert_eq!(classification.file_role, FileRole::SourceCode);
    assert_eq!(classification.text_index_mode, TextIndexMode::None);
    assert_eq!(classification.language, TRACK_ONLY_LANGUAGE_ID);
    assert_eq!(
        classification.classification_reason,
        "contextless_code_like"
    );
}

#[test]
fn markdown_and_manifests_classify_as_text() {
    let dir = tempdir().expect("temp dir");
    let classifier = classifier_for(dir.path(), &["README.md", "Cargo.toml"]);

    let readme = classifier
        .classify_repo_relative_path("README.md", false)
        .expect("classify README");
    let cargo = classifier
        .classify_repo_relative_path("Cargo.toml", false)
        .expect("classify Cargo.toml");

    assert_eq!(readme.analysis_mode, AnalysisMode::Text);
    assert_eq!(cargo.analysis_mode, AnalysisMode::Text);
    assert_eq!(readme.file_role, FileRole::Documentation);
    assert_eq!(readme.text_index_mode, TextIndexMode::Embed);
    assert_eq!(cargo.file_role, FileRole::ProjectManifest);
    assert_eq!(cargo.text_index_mode, TextIndexMode::StoreOnly);
    assert_eq!(readme.language, PLAIN_TEXT_LANGUAGE_ID);
    assert_eq!(cargo.language, PLAIN_TEXT_LANGUAGE_ID);
}

#[test]
fn typescript_context_activates_near_tsconfig() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("web/src")).expect("create web/src");
    fs::write(dir.path().join("web/tsconfig.json"), "{}").expect("write tsconfig");

    let classifier = classifier_for(dir.path(), &["web/tsconfig.json", "web/src/app.tsx"]);
    let classification = classifier
        .classify_repo_relative_path("web/src/app.tsx", false)
        .expect("classify tsx");

    assert_eq!(classification.analysis_mode, AnalysisMode::Code);
    assert_eq!(classification.file_role, FileRole::SourceCode);
    assert_eq!(classification.text_index_mode, TextIndexMode::None);
    assert_eq!(classification.language, "typescript");
    assert_eq!(classification.dialect.as_deref(), Some("tsx"));
    assert_eq!(
        classification.primary_context_id.as_deref(),
        Some("auto:typescript:web")
    );
}

#[test]
fn scope_include_as_text_promotes_otherwise_track_only_paths() {
    let dir = tempdir().expect("temp dir");
    fs::write(
        dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[scope]
include_as_text = ["notes/**/*.foo"]
"#,
    )
    .expect("write policy");

    let classifier = classifier_for(dir.path(), &["notes/tmp/example.foo"]);
    let classification = classifier
        .classify_repo_relative_path("notes/tmp/example.foo", false)
        .expect("classify");

    assert_eq!(classification.analysis_mode, AnalysisMode::Text);
    assert_eq!(classification.file_role, FileRole::Configuration);
    assert_eq!(classification.text_index_mode, TextIndexMode::StoreOnly);
    assert_eq!(classification.classification_reason, "manual_text");
}

#[test]
fn manual_context_promotes_contextless_code() {
    let dir = tempdir().expect("temp dir");
    fs::write(
        dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[[contexts]]
id = "scripts"
kind = "standalone"
root = "tools"
profile = "typescript-standard"
include = ["**/*.ts"]
"#,
    )
    .expect("write policy");
    fs::create_dir_all(dir.path().join("tools")).expect("create tools");

    let classifier = classifier_for(dir.path(), &["tools/run.ts"]);
    let classification = classifier
        .classify_repo_relative_path("tools/run.ts", false)
        .expect("classify");

    assert_eq!(classification.analysis_mode, AnalysisMode::Code);
    assert_eq!(classification.file_role, FileRole::SourceCode);
    assert_eq!(classification.text_index_mode, TextIndexMode::None);
    assert_eq!(
        classification.primary_context_id.as_deref(),
        Some("scripts")
    );
    assert_eq!(classification.language, "typescript");
}

#[test]
fn auto_scope_project_root_and_include_constrain_auto_detection() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("packages/app/src")).expect("create app/src");
    fs::create_dir_all(dir.path().join("packages/other/src")).expect("create other/src");
    fs::write(
        dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[scope]
project_root = "packages/app"
include = ["src/**"]
"#,
    )
    .expect("write policy");
    fs::write(dir.path().join("packages/app/tsconfig.json"), "{}").expect("write app tsconfig");
    fs::write(dir.path().join("packages/other/tsconfig.json"), "{}").expect("write other tsconfig");

    let classifier = classifier_for(
        dir.path(),
        &[
            "packages/app/tsconfig.json",
            "packages/app/src/app.ts",
            "packages/other/tsconfig.json",
            "packages/other/src/skip.ts",
        ],
    );

    let allowed = classifier
        .classify_repo_relative_path("packages/app/src/app.ts", false)
        .expect("classify allowed");
    let denied = classifier
        .classify_repo_relative_path("packages/other/src/skip.ts", false)
        .expect("classify denied");

    assert_eq!(allowed.analysis_mode, AnalysisMode::Code);
    assert_eq!(denied.analysis_mode, AnalysisMode::TrackOnly);
}
