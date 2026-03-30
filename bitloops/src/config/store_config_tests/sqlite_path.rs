use super::super::store_config_utils::expand_home_prefix_with;
use super::*;

#[test]
fn sqlite_path_resolution_uses_explicit_path() {
    let resolved = resolve_sqlite_db_path(Some("/tmp/bitloops-relational.sqlite"))
        .expect("explicit sqlite path should resolve");
    assert_eq!(resolved, PathBuf::from("/tmp/bitloops-relational.sqlite"));
}

#[test]
fn sqlite_path_resolution_resolves_relative_path_against_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    with_cwd(temp.path(), || {
        let resolved = resolve_sqlite_db_path(Some("data/relational.sqlite"))
            .expect("relative sqlite path should resolve");
        assert!(
            resolved.ends_with(Path::new("data").join("relational.sqlite")),
            "expected repo-relative sqlite path, got {}",
            resolved.display()
        );
    });
}

#[test]
fn sqlite_path_resolution_expands_tilde_prefix() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("create fake home");
    let home_str = home.to_string_lossy().into_owned();
    let _guard = enter_process_state(
        None,
        &[("HOME", Some(home_str.as_str())), ("USERPROFILE", None)],
    );

    let resolved =
        resolve_sqlite_db_path(Some("~/devql.sqlite")).expect("tilde sqlite path should resolve");
    assert_eq!(resolved, home.join("devql.sqlite"));
}

#[test]
fn sqlite_path_resolution_expands_windows_tilde_prefix_with_windows_home() {
    let windows_home = Path::new(r"C:\Users\bitloops");

    let expanded = expand_home_prefix_with(
        r"~\.bitloops\stores\relational\relational.db",
        Some(windows_home),
    )
    .expect("windows-style tilde sqlite path should resolve");

    assert_eq!(
        PathBuf::from(expanded),
        windows_home.join(r".bitloops\stores\relational\relational.db")
    );
}
