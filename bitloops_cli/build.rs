use serde::Deserialize;
use std::env;
use std::fs;
use std::path::Path;

const DASHBOARD_CONFIG_PATH: &str = "config/dashboard_urls.json";
const EXAMPLE_CONFIG_PATH: &str = "config/dashboard_urls.template.json";
const GENERATED_FILE_NAME: &str = "dashboard_env.rs";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DashboardUrlsConfig {
    dashboard_cdn_base_url: String,
    dashboard_manifest_url: String,
}

fn validate_url(name: &str, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        panic!("{name} must not be empty");
    }

    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        panic!("{name} must start with http:// or https://");
    }

    if let Err(err) = reqwest::Url::parse(trimmed) {
        panic!("{name} must be a valid URL: {err}");
    }
}

fn main() {
    println!("cargo:rerun-if-changed={DASHBOARD_CONFIG_PATH}");
    println!("cargo:rerun-if-changed=build.rs");

    let raw = fs::read_to_string(DASHBOARD_CONFIG_PATH).unwrap_or_else(|err| {
        panic!(
            "failed to read {DASHBOARD_CONFIG_PATH}: {err}. Copy {EXAMPLE_CONFIG_PATH} to {DASHBOARD_CONFIG_PATH} and set environment-specific values."
        )
    });

    let cfg: DashboardUrlsConfig = serde_json::from_str(&raw).unwrap_or_else(|err| {
        panic!("invalid JSON in {DASHBOARD_CONFIG_PATH}: {err}");
    });

    validate_url("dashboard_cdn_base_url", &cfg.dashboard_cdn_base_url);
    validate_url("dashboard_manifest_url", &cfg.dashboard_manifest_url);

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is not set");
    let out_path = Path::new(&out_dir).join(GENERATED_FILE_NAME);

    let generated = format!(
        "pub const DASHBOARD_CDN_BASE_URL: &str = {cdn:?};\n\
         pub const DASHBOARD_MANIFEST_URL: &str = {manifest:?};\n",
        cdn = cfg.dashboard_cdn_base_url.trim(),
        manifest = cfg.dashboard_manifest_url.trim(),
    );

    fs::write(&out_path, generated)
        .unwrap_or_else(|err| panic!("failed writing {}: {err}", out_path.display()));
}
