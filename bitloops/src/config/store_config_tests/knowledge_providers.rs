use super::*;

#[test]
fn knowledge_config_providers_defaults_when_block_missing() {
    let value = serde_json::json!({
        "stores": {
            "relational": { "provider": "sqlite" }
        }
    });

    let cfg = resolve_provider_config_for_tests(&value, &[]).expect("provider config");
    assert_eq!(cfg, ProviderConfig::default());
}

#[test]
fn knowledge_config_providers_reads_literal_values() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "github": { "token": "gh-token" },
                "atlassian": {
                    "site_url": "https://shared.atlassian.net",
                    "email": "shared@example.com",
                    "token": "shared-token"
                },
                "jira": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "jira@example.com",
                    "token": "jira-token"
                },
                "confluence": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "docs@example.com",
                    "token": "confluence-token"
                }
            }
        }
    });

    let cfg = resolve_provider_config_for_tests(&value, &[]).expect("provider config");
    assert_eq!(
        cfg.github,
        Some(GithubProviderConfig {
            token: "gh-token".to_string()
        })
    );
    assert_eq!(
        cfg.atlassian,
        Some(AtlassianProviderConfig {
            site_url: "https://shared.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        })
    );
    assert_eq!(
        cfg.jira,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "jira@example.com".to_string(),
            token: "jira-token".to_string(),
        })
    );
    assert_eq!(
        cfg.confluence,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "docs@example.com".to_string(),
            token: "confluence-token".to_string(),
        })
    );
}

#[test]
fn knowledge_config_providers_reads_shared_atlassian_values() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "atlassian": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "shared@example.com",
                    "token": "shared-token"
                }
            }
        }
    });

    let cfg = resolve_provider_config_for_tests(&value, &[]).expect("provider config");
    assert_eq!(
        cfg.atlassian,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        })
    );
    assert_eq!(cfg.jira, None);
    assert_eq!(cfg.confluence, None);
}

#[test]
fn knowledge_config_providers_resolves_env_indirection() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "github": { "token": "${BITLOOPS_GITHUB_TOKEN}" }
            }
        }
    });

    let cfg = resolve_provider_config_for_tests(&value, &[("BITLOOPS_GITHUB_TOKEN", "env-gh")])
        .expect("provider config");
    assert_eq!(
        cfg.github,
        Some(GithubProviderConfig {
            token: "env-gh".to_string()
        })
    );
}

#[test]
fn knowledge_config_providers_shared_atlassian_resolves_env_indirection() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "atlassian": {
                    "site_url": "${BITLOOPS_ATLASSIAN_URL}",
                    "email": "${BITLOOPS_ATLASSIAN_EMAIL}",
                    "token": "${BITLOOPS_ATLASSIAN_TOKEN}"
                }
            }
        }
    });

    let cfg = resolve_provider_config_for_tests(
        &value,
        &[
            ("BITLOOPS_ATLASSIAN_URL", "https://bitloops.atlassian.net"),
            ("BITLOOPS_ATLASSIAN_EMAIL", "shared@example.com"),
            ("BITLOOPS_ATLASSIAN_TOKEN", "shared-token"),
        ],
    )
    .expect("provider config");
    assert_eq!(
        cfg.atlassian,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        })
    );
}

#[test]
fn knowledge_config_providers_rejects_missing_env_value() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "github": { "token": "${BITLOOPS_GITHUB_TOKEN}" }
            }
        }
    });

    let err = resolve_provider_config_for_tests(&value, &[]).expect_err("missing env should fail");
    assert!(err.to_string().contains("knowledge.providers.github.token"));
}

#[test]
fn knowledge_config_providers_rejects_missing_required_shared_atlassian_field() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "atlassian": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "shared@example.com"
                }
            }
        }
    });

    let err = resolve_provider_config_for_tests(&value, &[])
        .expect_err("missing provider field should fail");
    assert!(
        err.to_string()
            .contains("missing `knowledge.providers.atlassian.token`")
    );
}

#[test]
fn knowledge_config_providers_rejects_missing_required_field() {
    let value = serde_json::json!({
        "knowledge": {
            "providers": {
                "jira": {
                    "site_url": "https://bitloops.atlassian.net",
                    "email": "jira@example.com"
                }
            }
        }
    });

    let err = resolve_provider_config_for_tests(&value, &[])
        .expect_err("missing provider field should fail");
    assert!(
        err.to_string()
            .contains("missing `knowledge.providers.jira.token`")
    );
}

#[test]
fn knowledge_config_providers_jira_and_confluence_fall_back_to_shared_atlassian() {
    let cfg = ProviderConfig {
        github: None,
        atlassian: Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        }),
        jira: None,
        confluence: None,
    };

    assert_eq!(cfg.jira_config(), cfg.atlassian.as_ref());
    assert_eq!(cfg.confluence_config(), cfg.atlassian.as_ref());
}

#[test]
fn knowledge_config_providers_product_overrides_win_over_shared_atlassian() {
    let cfg = ProviderConfig {
        github: None,
        atlassian: Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        }),
        jira: Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "jira@example.com".to_string(),
            token: "jira-token".to_string(),
        }),
        confluence: Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "docs@example.com".to_string(),
            token: "docs-token".to_string(),
        }),
    };

    assert_eq!(cfg.jira_config(), cfg.jira.as_ref());
    assert_eq!(cfg.confluence_config(), cfg.confluence.as_ref());
}

#[test]
fn knowledge_config_providers_defaults_when_repo_config_missing() {
    let temp = tempfile::tempdir().expect("temp dir");

    let cfg = resolve_provider_config_for_repo(temp.path()).expect("provider config");

    assert_eq!(cfg, ProviderConfig::default());
}

#[test]
fn knowledge_config_providers_reads_values_from_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "knowledge": {
                "providers": {
                    "github": { "token": "gh-token" },
                    "jira": {
                        "site_url": "https://bitloops.atlassian.net",
                        "email": "jira@example.com",
                        "token": "jira-token"
                    }
                }
            }
        }),
    );

    let cfg = resolve_provider_config_for_repo(temp.path()).expect("provider config");

    assert_eq!(
        cfg.github,
        Some(GithubProviderConfig {
            token: "gh-token".to_string(),
        })
    );
    assert_eq!(cfg.atlassian, None);
    assert_eq!(
        cfg.jira,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "jira@example.com".to_string(),
            token: "jira-token".to_string(),
        })
    );
    assert_eq!(cfg.confluence, None);
}

#[test]
fn knowledge_config_providers_resolve_from_current_repo_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "knowledge": {
                "providers": {
                    "confluence": {
                        "site_url": "https://bitloops.atlassian.net",
                        "email": "docs@example.com",
                        "token": "docs-token"
                    }
                }
            }
        }),
    );

    let _guard = enter_process_state(Some(temp.path()), &[]);
    let cfg = resolve_provider_config().expect("provider config");

    assert_eq!(
        cfg.confluence,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "docs@example.com".to_string(),
            token: "docs-token".to_string(),
        })
    );
    assert_eq!(cfg.github, None);
    assert_eq!(cfg.atlassian, None);
    assert_eq!(cfg.jira, None);
}

#[test]
fn knowledge_config_providers_reads_shared_atlassian_values_from_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "knowledge": {
                "providers": {
                    "atlassian": {
                        "site_url": "https://bitloops.atlassian.net",
                        "email": "shared@example.com",
                        "token": "shared-token"
                    }
                }
            }
        }),
    );

    let cfg = resolve_provider_config_for_repo(temp.path()).expect("provider config");

    assert_eq!(
        cfg.atlassian,
        Some(AtlassianProviderConfig {
            site_url: "https://bitloops.atlassian.net".to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        })
    );
    assert_eq!(cfg.github, None);
    assert_eq!(cfg.jira, None);
    assert_eq!(cfg.confluence, None);
}

#[test]
fn knowledge_config_providers_resolve_env_indirection_from_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "knowledge": {
                "providers": {
                    "github": { "token": "${BITLOOPS_GITHUB_TOKEN}" }
                }
            }
        }),
    );

    let _guard = enter_process_state(None, &[("BITLOOPS_GITHUB_TOKEN", Some("env-gh-from-file"))]);
    let cfg = resolve_provider_config_for_repo(temp.path()).expect("provider config");

    assert_eq!(
        cfg.github,
        Some(GithubProviderConfig {
            token: "env-gh-from-file".to_string(),
        })
    );
}

#[test]
fn resolve_provider_config_defaults_without_repo_config_via_public_function() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = enter_process_state(Some(temp.path()), &[]);

    let cfg = resolve_provider_config().expect("provider config");

    assert_eq!(cfg, ProviderConfig::default());
}
