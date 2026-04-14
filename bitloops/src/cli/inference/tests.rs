use std::io::Cursor;

use tempfile::TempDir;
use toml_edit::{DocumentMut, Item};

use crate::cli::inference::{
    OllamaAvailability, SummarySetupOutcome, configure_local_summary_generation,
    summary_generation_configured, with_managed_inference_install_hook, with_ollama_probe_hook,
};
use crate::config::{BITLOOPS_CONFIG_RELATIVE_PATH, resolve_inference_capability_config_for_repo};

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

#[test]
fn summary_generation_configured_rejects_legacy_hosted_profile_without_runtime() {
    let repo = TempDir::new().expect("tempdir");
    let repo_root = repo.path().to_path_buf();
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config parent");
    std::fs::write(
        &config_path,
        r#"
[semantic_clones.inference]
summary_generation = "summary_remote"

[inference.profiles.summary_remote]
task = "text_generation"
driver = "openai"
model = "gpt-4.1-mini"
api_key = "sk-test"
base_url = "https://api.openai.com/v1/chat/completions"
temperature = "0.1"
max_output_tokens = 200
"#,
    )
    .expect("write config");

    assert!(
        !summary_generation_configured(&repo_root),
        "legacy hosted text-generation profile should require migration"
    );
}

#[test]
fn summary_setup_preserves_existing_summary_local_profile() {
    let repo = TempDir::new().expect("tempdir");
    let repo_root = repo.path().to_path_buf();
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config parent");
    std::fs::write(
        &config_path,
        r#"
[inference.profiles.summary_local]
task = "text_generation"
driver = "openai"
model = "gpt-4.1-mini"
api_key = "sk-test"
base_url = "https://api.openai.com/v1/chat/completions"
temperature = "0.1"
max_output_tokens = 200
"#,
    )
    .expect("write config");
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
                        models: vec!["ministral-3:3b".to_string()],
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

    let rendered = std::fs::read_to_string(&config_path).expect("read config");
    let doc = rendered
        .parse::<DocumentMut>()
        .expect("parse updated config");
    let summary_generation = doc["semantic_clones"]["inference"]["summary_generation"]
        .as_value()
        .and_then(|value| value.as_str())
        .expect("summary generation binding");
    assert_eq!(summary_generation, "summary_local_1");

    let legacy_profile = doc["inference"]["profiles"]["summary_local"]
        .as_table()
        .expect("legacy profile table");
    assert_eq!(
        legacy_profile
            .get("driver")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str()),
        Some("openai")
    );
    assert_eq!(
        legacy_profile
            .get("api_key")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str()),
        Some("sk-test")
    );

    let managed_profile = doc["inference"]["profiles"]["summary_local_1"]
        .as_table()
        .expect("managed profile table");
    assert_eq!(
        managed_profile
            .get("runtime")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str()),
        Some("bitloops_inference")
    );
    assert_eq!(
        managed_profile
            .get("driver")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str()),
        Some("ollama_chat")
    );
    assert_eq!(
        managed_profile
            .get("model")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str()),
        Some("ministral-3:3b")
    );
    assert_eq!(
        managed_profile
            .get("base_url")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str()),
        Some("http://127.0.0.1:11434/api/chat")
    );
    assert_eq!(
        managed_profile
            .get("temperature")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str()),
        Some("0.1")
    );
    assert_eq!(
        managed_profile
            .get("max_output_tokens")
            .and_then(Item::as_integer),
        Some(200)
    );
}

#[test]
fn summary_setup_writes_profiles_to_the_repo_bound_daemon_config() {
    let repo = TempDir::new().expect("tempdir");
    let repo_root = repo.path().to_path_buf();
    let local_config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    std::fs::create_dir_all(local_config_path.parent().expect("config parent"))
        .expect("create config parent");
    std::fs::write(
        &local_config_path,
        r#"
[semantic_clones.inference]
summary_generation = "local_summary"

[inference.profiles.local_summary]
task = "text_generation"
driver = "openai"
model = "gpt-4.1-mini"
api_key = "sk-test"
base_url = "https://api.openai.com/v1/chat/completions"
temperature = "0.1"
max_output_tokens = 200
"#,
    )
    .expect("write local config");

    let bound_config_path = repo_root.join("daemon").join(BITLOOPS_CONFIG_RELATIVE_PATH);
    std::fs::create_dir_all(bound_config_path.parent().expect("bound config parent"))
        .expect("create bound config parent");
    std::fs::write(&bound_config_path, "").expect("write bound config");
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &bound_config_path,
    )
    .expect("write repo daemon binding");

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
                        models: vec!["ministral-3:3b".to_string()],
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
    assert!(
        summary_generation_configured(&repo_root),
        "summary generation should resolve from the bound daemon config"
    );

    let capability = resolve_inference_capability_config_for_repo(&repo_root);
    assert_eq!(
        capability
            .semantic_clones
            .inference
            .summary_generation
            .as_deref(),
        Some("summary_local"),
        "capability resolution should prefer the bound daemon config"
    );

    let bound_rendered = std::fs::read_to_string(&bound_config_path).expect("read bound config");
    let bound_doc = bound_rendered
        .parse::<DocumentMut>()
        .expect("parse bound config");
    assert_eq!(
        bound_doc["semantic_clones"]["inference"]["summary_generation"]
            .as_value()
            .and_then(|value| value.as_str()),
        Some("summary_local")
    );
    assert_eq!(
        bound_doc["inference"]["profiles"]["summary_local"]["runtime"]
            .as_value()
            .and_then(|value| value.as_str()),
        Some("bitloops_inference")
    );

    let local_rendered = std::fs::read_to_string(&local_config_path).expect("read local config");
    assert!(
        local_rendered.contains("local_summary"),
        "local config should remain unchanged:\n{local_rendered}"
    );
    assert!(
        !local_rendered.contains("bitloops_inference"),
        "summary setup should not rewrite the nearest daemon config:\n{local_rendered}"
    );
}
