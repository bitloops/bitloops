use std::io::Cursor;

use tempfile::TempDir;

use crate::cli::inference::{
    OllamaAvailability, SummarySetupOutcome, configure_local_summary_generation,
    summary_generation_configured, with_managed_inference_install_hook, with_ollama_probe_hook,
};
use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;

#[test]
fn summary_setup_skips_profile_when_ollama_is_missing() {
    let repo = TempDir::new().expect("tempdir");
    let repo_root = repo.path().to_path_buf();
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config parent");
    std::fs::write(&config_path, "").expect("write config");
    let mut out = Vec::new();
    let mut input = Cursor::new(Vec::<u8>::new());
    let install_root = repo_root.clone();
    let configure_root = repo_root.clone();

    let outcome = with_managed_inference_install_hook(
        move |_repo_root| {
            Ok(
                crate::cli::inference::ManagedInferenceBinaryInstallOutcome {
                    version: "v1.2.3".to_string(),
                    binary_path: install_root.join("bitloops-inference"),
                    freshly_installed: true,
                },
            )
        },
        || {
            with_ollama_probe_hook(
                || Ok(OllamaAvailability::MissingCli),
                || configure_local_summary_generation(&configure_root, &mut out, &mut input, false),
            )
        },
    )
    .expect("summary setup outcome");

    assert_eq!(outcome, SummarySetupOutcome::InstalledRuntimeOnly);
    assert!(!summary_generation_configured(&repo_root));
}

#[test]
fn summary_setup_prefers_ministral_3_3b_when_available() {
    let repo = TempDir::new().expect("tempdir");
    let repo_root = repo.path().to_path_buf();
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config parent");
    std::fs::write(&config_path, "").expect("write config");
    let mut out = Vec::new();
    let mut input = Cursor::new(Vec::<u8>::new());
    let install_root = repo_root.clone();
    let configure_root = repo_root.clone();

    let outcome = with_managed_inference_install_hook(
        move |_repo_root| {
            Ok(
                crate::cli::inference::ManagedInferenceBinaryInstallOutcome {
                    version: "v1.2.3".to_string(),
                    binary_path: install_root.join("bitloops-inference"),
                    freshly_installed: true,
                },
            )
        },
        || {
            with_ollama_probe_hook(
                || {
                    Ok(OllamaAvailability::Running {
                        models: vec!["ministral-3:8b".to_string(), "ministral-3:3b".to_string()],
                    })
                },
                || configure_local_summary_generation(&configure_root, &mut out, &mut input, false),
            )
        },
    )
    .expect("summary setup outcome");

    assert_eq!(
        outcome,
        SummarySetupOutcome::Configured {
            model_name: "ministral-3:3b".to_string()
        }
    );
    assert!(summary_generation_configured(&repo_root));
}
