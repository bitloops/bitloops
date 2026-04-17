use crate::config::{
    DEFAULT_SEMANTIC_CLONES_ENRICHMENT_WORKERS, resolve_semantic_clones_config_for_repo,
};

const SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV: &str =
    "BITLOOPS_SEMANTIC_CLONES_ENRICHMENT_WORKERS";
const MAX_ENRICHMENT_WORKER_COUNT: usize = 32;

pub(super) fn configured_enrichment_worker_count() -> usize {
    if let Some(override_count) = parse_enrichment_worker_count(
        std::env::var(SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV)
            .ok()
            .as_deref(),
    ) {
        return override_count.clamp(1, MAX_ENRICHMENT_WORKER_COUNT);
    }

    let configured = std::env::current_dir()
        .ok()
        .map(|cwd| resolve_semantic_clones_config_for_repo(&cwd).enrichment_workers)
        .unwrap_or(DEFAULT_SEMANTIC_CLONES_ENRICHMENT_WORKERS);
    configured.clamp(1, MAX_ENRICHMENT_WORKER_COUNT)
}

#[cfg(test)]
fn resolve_enrichment_worker_count(raw_value: Option<&str>) -> usize {
    parse_enrichment_worker_count(raw_value)
        .unwrap_or(DEFAULT_SEMANTIC_CLONES_ENRICHMENT_WORKERS)
        .clamp(1, MAX_ENRICHMENT_WORKER_COUNT)
}

fn parse_enrichment_worker_count(raw_value: Option<&str>) -> Option<usize> {
    raw_value
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|count| *count > 0)
}

#[cfg(test)]
mod tests {
    use super::{
        SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV, configured_enrichment_worker_count,
        resolve_enrichment_worker_count,
    };
    use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;
    use crate::test_support::process_state::enter_process_state;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn enrichment_worker_count_defaults_to_two_for_missing_or_invalid_values() {
        assert_eq!(resolve_enrichment_worker_count(None), 2);
        assert_eq!(resolve_enrichment_worker_count(Some("")), 2);
        assert_eq!(resolve_enrichment_worker_count(Some("0")), 2);
        assert_eq!(resolve_enrichment_worker_count(Some("-1")), 2);
        assert_eq!(resolve_enrichment_worker_count(Some("nope")), 2);
    }

    #[test]
    fn enrichment_worker_count_respects_valid_values_and_caps_large_values() {
        assert_eq!(resolve_enrichment_worker_count(Some("4")), 4);
        assert_eq!(resolve_enrichment_worker_count(Some(" 8 ")), 8);
        assert_eq!(resolve_enrichment_worker_count(Some("999")), 32);
    }

    #[test]
    fn configured_worker_count_prefers_semantic_clones_env_override() {
        let temp = tempdir().expect("temp dir");
        let _guard = enter_process_state(
            Some(temp.path()),
            &[(SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV, Some("7"))],
        );
        assert_eq!(configured_enrichment_worker_count(), 7);
    }

    #[test]
    fn configured_worker_count_ignores_legacy_env_when_repo_config_is_present() {
        let temp = tempdir().expect("temp dir");
        let config_path = temp.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).expect("create config dir");
        }
        fs::write(&config_path, "[semantic_clones]\nenrichment_workers = 5\n")
            .expect("write semantic clones config");

        let _guard = enter_process_state(
            Some(temp.path()),
            &[
                (SEMANTIC_CLONES_ENRICHMENT_WORKER_COUNT_ENV, None),
                ("BITLOOPS_ENRICHMENT_WORKERS", Some("3")),
            ],
        );

        assert_eq!(configured_enrichment_worker_count(), 5);
    }
}
