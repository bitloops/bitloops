use std::fs;
use std::path::{Path, PathBuf};

fn collect_markdown_files(root: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root).unwrap_or_else(|err| {
        panic!(
            "reading documentation directory {} failed: {err}",
            root.display()
        )
    });
    for entry in entries {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            files.push(path);
        }
    }
}

#[test]
fn production_docs_do_not_reference_removed_json_configs_or_repo_local_runtime_paths() {
    let docs_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("documentation")
        .join("docs");
    let upgrade_note = docs_root
        .join("reference")
        .join("upgrading-to-the-daemon-architecture.md");

    let banned_patterns = [
        "config.json",
        "config.local.json",
        "settings.json",
        "settings.local.json",
        "~/.bitloops/dashboard/bundle",
        ".bitloops/stores",
        ".bitloops/embeddings",
        ".bitloops/tmp",
        ".bitloops/metadata",
        "bitloops init --agent",
        "bitloops enable --local",
        "bitloops enable --project",
    ];

    let mut markdown_files = Vec::new();
    collect_markdown_files(&docs_root, &mut markdown_files);

    for path in markdown_files {
        if path == upgrade_note {
            continue;
        }

        let content = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("reading {} failed: {err}", path.display()));
        for pattern in banned_patterns {
            assert!(
                !content.contains(pattern),
                "documentation file {} still contains banned pattern `{pattern}`",
                path.display()
            );
        }
    }
}

#[test]
fn upgrade_note_is_linked_from_core_reference_pages() {
    let docs_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("documentation")
        .join("docs");
    let required_links = [
        docs_root.join("reference").join("configuration.md"),
        docs_root.join("reference").join("cli-commands.md"),
        docs_root.join("getting-started").join("quickstart.md"),
    ];

    for path in required_links {
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("reading {} failed: {err}", path.display()));
        assert!(
            content.contains("upgrading-to-the-daemon-architecture.md"),
            "{} should link to the daemon architecture upgrade note",
            path.display()
        );
    }
}
