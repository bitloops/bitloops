use super::*;

#[test]
fn blob_local_path_resolution_defaults_under_current_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = enter_process_state(Some(temp.path()), &[]);

    let resolved = resolve_blob_local_path(None).expect("default blob path");
    let rendered = resolved.to_string_lossy();

    assert!(
        rendered.ends_with(".bitloops-test-state/data/stores/blob")
            || rendered.ends_with(".bitloops-test-state\\data\\stores\\blob")
    );
}

#[test]
fn blob_storage_local_path_or_default_uses_current_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = BlobStorageConfig {
        local_path: None,
        s3_bucket: None,
        s3_region: None,
        s3_access_key_id: None,
        s3_secret_access_key: None,
        gcs_bucket: None,
        gcs_credentials_path: None,
    };
    let _guard = enter_process_state(Some(temp.path()), &[]);

    let resolved = config
        .local_path_or_default()
        .expect("default local blob path");

    let rendered = resolved.to_string_lossy();
    assert!(
        rendered.ends_with(".bitloops-test-state/data/stores/blob")
            || rendered.ends_with(".bitloops-test-state\\data\\stores\\blob")
    );
}

#[test]
fn blob_local_path_resolution_uses_explicit_path() {
    let resolved =
        resolve_blob_local_path(Some("/tmp/bitloops-blobs")).expect("explicit blob path");
    assert_eq!(resolved, PathBuf::from("/tmp/bitloops-blobs"));
}

#[test]
fn blob_local_path_resolution_expands_tilde_prefix() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("create fake home");
    let home_str = home.to_string_lossy().into_owned();
    let _guard = enter_process_state(
        None,
        &[("HOME", Some(home_str.as_str())), ("USERPROFILE", None)],
    );

    let resolved =
        resolve_blob_local_path(Some("~/blob-storage")).expect("tilde blob path should resolve");
    assert_eq!(resolved, home.join("blob-storage"));
}

#[test]
fn blob_local_path_resolution_defaults_under_test_state_store_directory() {
    let blobs = BlobStorageConfig {
        local_path: None,
        s3_bucket: None,
        s3_region: None,
        s3_access_key_id: None,
        s3_secret_access_key: None,
        gcs_bucket: None,
        gcs_credentials_path: None,
    };

    let resolved = blobs
        .local_path_or_default()
        .expect("default local blob path");
    let rendered = resolved.to_string_lossy();
    assert!(
        rendered.ends_with(".bitloops-test-state/data/stores/blob")
            || rendered.ends_with(".bitloops-test-state\\data\\stores\\blob")
    );
}
