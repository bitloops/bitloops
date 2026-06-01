use super::*;

const OLD_BLOB: &str = "1111111111111111111111111111111111111111";
const NEW_BLOB: &str = "2222222222222222222222222222222222222222";
const ADDED_BLOB: &str = "3333333333333333333333333333333333333333";
const DELETED_BLOB: &str = "4444444444444444444444444444444444444444";
const BINARY_OLD_BLOB: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BINARY_NEW_BLOB: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn parse_commit_hunks_from_git_show_captures_added_deleted_and_modified_lines() {
    let diff = format!(
        r#"diff --git a/src/lib.rs b/src/lib.rs
index {OLD_BLOB}..{NEW_BLOB} 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1,2 @@
-pub fn old() {{}}
+pub fn new() {{}}
+pub fn added() {{}}
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
index 0000000000000000000000000000000000000000..{ADDED_BLOB}
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,2 @@
+pub fn new_file() {{}}
+pub fn second() {{}}
diff --git a/src/deleted.rs b/src/deleted.rs
deleted file mode 100644
index {DELETED_BLOB}..0000000000000000000000000000000000000000
--- a/src/deleted.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-pub fn gone() {{}}
-pub fn also_gone() {{}}
"#
    );

    let parsed =
        parse_commit_hunks_from_git_show("repo-1", "commit-1", &diff).expect("parse git show diff");

    assert_eq!(parsed.file_deltas.len(), 3);
    assert_eq!(parsed.hunks.len(), 3);

    let modified = parsed
        .file_deltas
        .iter()
        .find(|delta| delta.path_after.as_deref() == Some("src/lib.rs"))
        .expect("modified delta");
    assert_eq!(modified.change_kind, "modified");
    assert_eq!(modified.old_blob_sha.as_deref(), Some(OLD_BLOB));
    assert_eq!(modified.new_blob_sha.as_deref(), Some(NEW_BLOB));
    assert!(!modified.is_binary);

    let modified_hunk = parsed
        .hunks
        .iter()
        .find(|hunk| hunk.delta_id == modified.delta_id)
        .expect("modified hunk");
    assert_eq!(modified_hunk.old_start, 1);
    assert_eq!(modified_hunk.old_line_count, 1);
    assert_eq!(modified_hunk.new_start, 1);
    assert_eq!(modified_hunk.new_line_count, 2);
    assert_eq!(modified_hunk.deleted_lines[0].line_number, 1);
    assert_eq!(modified_hunk.deleted_lines[0].content, "pub fn old() {}");
    assert_eq!(modified_hunk.added_lines[0].line_number, 1);
    assert_eq!(modified_hunk.added_lines[0].content, "pub fn new() {}");
    assert_eq!(modified_hunk.added_lines[1].line_number, 2);
    assert_eq!(modified_hunk.added_lines[1].content, "pub fn added() {}");

    let added = parsed
        .file_deltas
        .iter()
        .find(|delta| delta.path_after.as_deref() == Some("src/new.rs"))
        .expect("added delta");
    assert_eq!(added.change_kind, "added");
    assert_eq!(added.old_blob_sha, None);
    assert_eq!(added.new_blob_sha.as_deref(), Some(ADDED_BLOB));
    let added_hunk = parsed
        .hunks
        .iter()
        .find(|hunk| hunk.delta_id == added.delta_id)
        .expect("added hunk");
    assert!(added_hunk.deleted_lines.is_empty());
    assert_eq!(added_hunk.added_lines.len(), 2);

    let deleted = parsed
        .file_deltas
        .iter()
        .find(|delta| delta.path_before.as_deref() == Some("src/deleted.rs"))
        .expect("deleted delta");
    assert_eq!(deleted.change_kind, "deleted");
    assert_eq!(deleted.old_blob_sha.as_deref(), Some(DELETED_BLOB));
    assert_eq!(deleted.new_blob_sha, None);
    let deleted_hunk = parsed
        .hunks
        .iter()
        .find(|hunk| hunk.delta_id == deleted.delta_id)
        .expect("deleted hunk");
    assert!(deleted_hunk.added_lines.is_empty());
    assert_eq!(deleted_hunk.deleted_lines.len(), 2);
}

#[test]
fn parse_commit_hunks_from_git_show_captures_rename_only_and_binary_deltas() {
    let diff = format!(
        r#"diff --git a/src/old.rs b/src/new.rs
similarity index 100%
rename from src/old.rs
rename to src/new.rs
diff --git a/assets/logo.png b/assets/logo.png
index {BINARY_OLD_BLOB}..{BINARY_NEW_BLOB} 100644
Binary files a/assets/logo.png and b/assets/logo.png differ
"#
    );

    let parsed =
        parse_commit_hunks_from_git_show("repo-1", "commit-2", &diff).expect("parse git show diff");

    assert_eq!(parsed.file_deltas.len(), 2);
    assert!(parsed.hunks.is_empty());

    let rename = parsed
        .file_deltas
        .iter()
        .find(|delta| delta.path_after.as_deref() == Some("src/new.rs"))
        .expect("rename delta");
    assert_eq!(rename.change_kind, "renamed");
    assert_eq!(rename.path_before.as_deref(), Some("src/old.rs"));
    assert_eq!(rename.path_after.as_deref(), Some("src/new.rs"));
    assert!(!rename.is_binary);

    let binary = parsed
        .file_deltas
        .iter()
        .find(|delta| delta.path_after.as_deref() == Some("assets/logo.png"))
        .expect("binary delta");
    assert_eq!(binary.change_kind, "modified");
    assert!(binary.is_binary);
    assert_eq!(binary.old_blob_sha.as_deref(), Some(BINARY_OLD_BLOB));
    assert_eq!(binary.new_blob_sha.as_deref(), Some(BINARY_NEW_BLOB));
}
