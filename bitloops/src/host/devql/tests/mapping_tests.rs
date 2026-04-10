use super::*;
use crate::adapters::languages::go::canonical::{
    GO_CANONICAL_MAPPINGS, GO_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::go::extraction::extract_go_artefacts;
use crate::adapters::languages::java::canonical::{
    JAVA_CANONICAL_MAPPINGS, JAVA_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::java::extraction::extract_java_artefacts;
use crate::adapters::languages::python::canonical::{
    PYTHON_CANONICAL_MAPPINGS, PYTHON_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::python::extraction::extract_python_artefacts;
use crate::adapters::languages::rust::canonical::{
    RUST_CANONICAL_MAPPINGS, RUST_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::rust::extraction::extract_rust_artefacts;
use crate::adapters::languages::ts_js::canonical::{
    TS_JS_CANONICAL_MAPPINGS, TS_JS_SUPPORTED_LANGUAGE_KINDS,
};
use crate::adapters::languages::ts_js::extraction::extract_js_ts_artefacts;
use crate::host::language_adapter::{
    GoKind, JavaKind, LanguageKind, PythonKind, RustKind, TsJsKind,
};
use crate::host::language_adapter::{is_supported_language_kind, resolve_canonical_kind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn isolated_test_repo_root() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("bitloops-devql-mapping-test-{id}"))
}

#[path = "mapping_tests/canonical_mappings.rs"]
mod canonical_mappings;
#[path = "mapping_tests/lifecycle.rs"]
mod lifecycle;
#[path = "mapping_tests/registry_and_resolution.rs"]
mod registry_and_resolution;

fn extension_runtime_cfg() -> DevqlConfig {
    let repo_root = isolated_test_repo_root();
    DevqlConfig {
        daemon_config_root: repo_root.clone(),
        repo_root,
        repo: RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "temp2".to_string(),
            identity: "github/bitloops/temp2".to_string(),
            repo_id: deterministic_uuid("repo://github/bitloops/temp2"),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
    }
}

fn canonical_kind(artefact: &LanguageArtefact) -> Option<&str> {
    artefact.canonical_kind.as_deref()
}

fn artefact_by_language_kind(
    artefacts: &[LanguageArtefact],
    language_kind: LanguageKind,
) -> &LanguageArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.language_kind == language_kind)
        .unwrap_or_else(|| panic!("missing artefact with language_kind {}", language_kind))
}

fn artefact_by_name_and_language_kind<'a>(
    artefacts: &'a [LanguageArtefact],
    language_kind: LanguageKind,
    name: &str,
) -> &'a LanguageArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.language_kind == language_kind && artefact.name == name)
        .unwrap_or_else(|| {
            panic!(
                "missing artefact {name} with language_kind {}",
                language_kind
            )
        })
}
