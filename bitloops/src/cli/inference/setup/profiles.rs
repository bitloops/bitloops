use std::path::Path;

use anyhow::{Context, Result, bail};
use toml_edit::{DocumentMut, Item, Table};

use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, InferenceTask, SemanticSummaryMode,
    resolve_inference_capability_config_for_repo, resolve_preferred_daemon_config_path_for_repo,
};
use crate::host::inference::{BITLOOPS_INFERENCE_RUNTIME_ID, BITLOOPS_PLATFORM_CHAT_DRIVER};

use super::constants::{
    DEFAULT_CONTEXT_GUIDANCE_MAX_OUTPUT_TOKENS, DEFAULT_CONTEXT_GUIDANCE_PROFILE_NAME,
    DEFAULT_OLLAMA_CHAT_BASE_URL, DEFAULT_PLATFORM_CONTEXT_GUIDANCE_API_KEY_ENV,
    DEFAULT_PLATFORM_CONTEXT_GUIDANCE_MODEL, DEFAULT_PLATFORM_CONTEXT_GUIDANCE_PROFILE_NAME,
    DEFAULT_PLATFORM_SUMMARY_API_KEY, DEFAULT_PLATFORM_SUMMARY_MAX_OUTPUT_TOKENS,
    DEFAULT_PLATFORM_SUMMARY_MODEL, DEFAULT_PLATFORM_SUMMARY_PROFILE_NAME,
    DEFAULT_SUMMARY_MAX_OUTPUT_TOKENS, DEFAULT_SUMMARY_PROFILE_NAME, DEFAULT_SUMMARY_TEMPERATURE,
    OLLAMA_CHAT_DRIVER, PLATFORM_CHAT_COMPLETIONS_URL_ENV, PLATFORM_GATEWAY_URL_ENV,
    TEXT_GENERATION_TASK,
};

#[cfg(test)]
type SummaryGenerationConfiguredHook = dyn Fn(&Path) -> bool;
#[cfg(test)]
type SummaryGenerationConfiguredHookCell =
    std::cell::RefCell<Option<std::rc::Rc<SummaryGenerationConfiguredHook>>>;

#[cfg(test)]
thread_local! {
    static SUMMARY_GENERATION_CONFIGURED_HOOK: SummaryGenerationConfiguredHookCell =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
type ContextGuidanceGenerationConfiguredHook = dyn Fn(&Path) -> bool;
#[cfg(test)]
type ContextGuidanceGenerationConfiguredHookCell =
    std::cell::RefCell<Option<std::rc::Rc<ContextGuidanceGenerationConfiguredHook>>>;

#[cfg(test)]
thread_local! {
    static CONTEXT_GUIDANCE_GENERATION_CONFIGURED_HOOK: ContextGuidanceGenerationConfiguredHookCell =
        std::cell::RefCell::new(None);
}

pub(crate) fn summary_generation_configured(repo_root: &Path) -> bool {
    #[cfg(test)]
    if let Some(hook) = SUMMARY_GENERATION_CONFIGURED_HOOK.with(|cell| cell.borrow().clone()) {
        return hook(repo_root);
    }

    let capability = resolve_inference_capability_config_for_repo(repo_root);
    if capability.semantic_clones.summary_mode == SemanticSummaryMode::Off {
        return false;
    }
    let Some(profile_name) = capability
        .semantic_clones
        .inference
        .summary_generation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    text_generation_profile_configured(&capability, profile_name)
}

pub(crate) fn context_guidance_generation_configured(repo_root: &Path) -> bool {
    #[cfg(test)]
    if let Some(hook) =
        CONTEXT_GUIDANCE_GENERATION_CONFIGURED_HOOK.with(|cell| cell.borrow().clone())
    {
        return hook(repo_root);
    }

    let capability = resolve_inference_capability_config_for_repo(repo_root);
    let Some(profile_name) = capability
        .context_guidance
        .inference
        .guidance_generation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    text_generation_profile_configured(&capability, profile_name)
}

fn text_generation_profile_configured(
    capability: &crate::config::InferenceCapabilityConfig,
    profile_name: &str,
) -> bool {
    let Some(profile) = capability.inference.profiles.get(profile_name) else {
        return false;
    };
    let Some(runtime_name) = profile
        .runtime
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(runtime) = capability.inference.runtimes.get(runtime_name) else {
        return false;
    };

    let driver = profile.driver.trim();

    profile.task == InferenceTask::TextGeneration
        && profile
            .model
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        && profile
            .runtime
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        && !runtime.command.trim().is_empty()
        && (driver == BITLOOPS_PLATFORM_CHAT_DRIVER
            || profile
                .base_url
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty()))
        && profile
            .temperature
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        && profile.max_output_tokens.is_some_and(|value| value > 0)
}

pub(crate) fn platform_summary_gateway_url_override() -> Option<String> {
    let explicit = read_non_empty_env_value(PLATFORM_CHAT_COMPLETIONS_URL_ENV);
    if explicit.is_some() {
        return explicit;
    }

    read_non_empty_env_value(PLATFORM_GATEWAY_URL_ENV).map(|base_url| {
        let trimmed = base_url.trim_end_matches('/');
        format!("{trimmed}/v1/chat/completions")
    })
}

pub(crate) fn platform_context_guidance_gateway_url_override(
    explicit: Option<&str>,
) -> Option<String> {
    explicit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(platform_summary_gateway_url_override)
}

pub(super) fn write_summary_profile(repo_root: &Path, model_name: &str) -> Result<()> {
    let config_path = resolve_preferred_daemon_config_path_for_repo(repo_root)?;
    let mut doc = read_daemon_config_document(&config_path)?;

    let profile_name = {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        select_summary_profile_name(profiles)
    };

    {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        let profile = ensure_child_table(profiles, &profile_name);
        profile["task"] = Item::Value(TEXT_GENERATION_TASK.into());
        profile["runtime"] = Item::Value(BITLOOPS_INFERENCE_RUNTIME_ID.into());
        profile["driver"] = Item::Value(OLLAMA_CHAT_DRIVER.into());
        profile["model"] = Item::Value(model_name.into());
        profile["base_url"] = Item::Value(DEFAULT_OLLAMA_CHAT_BASE_URL.into());
        profile["temperature"] = Item::Value(DEFAULT_SUMMARY_TEMPERATURE.into());
        profile["max_output_tokens"] = Item::Value(DEFAULT_SUMMARY_MAX_OUTPUT_TOKENS.into());
        profile.remove("api_key");
        profile.remove("cache_dir");
    }

    update_summary_generation_binding(&mut doc, &profile_name);
    write_daemon_config_document(&config_path, &doc)?;
    maybe_sync_repo_local_summary_mode(repo_root, "auto")
}

pub(super) fn write_context_guidance_profile(repo_root: &Path, model_name: &str) -> Result<()> {
    let config_path = resolve_preferred_daemon_config_path_for_repo(repo_root)?;
    let mut doc = read_daemon_config_document(&config_path)?;

    let profile_name = {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        select_profile_name(
            profiles,
            DEFAULT_CONTEXT_GUIDANCE_PROFILE_NAME,
            is_managed_summary_profile,
        )
    };

    {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        let profile = ensure_child_table(profiles, &profile_name);
        profile["task"] = Item::Value(TEXT_GENERATION_TASK.into());
        profile["runtime"] = Item::Value(BITLOOPS_INFERENCE_RUNTIME_ID.into());
        profile["driver"] = Item::Value(OLLAMA_CHAT_DRIVER.into());
        profile["model"] = Item::Value(model_name.into());
        profile["base_url"] = Item::Value(DEFAULT_OLLAMA_CHAT_BASE_URL.into());
        profile["temperature"] = Item::Value(DEFAULT_SUMMARY_TEMPERATURE.into());
        profile["max_output_tokens"] =
            Item::Value(DEFAULT_CONTEXT_GUIDANCE_MAX_OUTPUT_TOKENS.into());
        profile.remove("api_key");
        profile.remove("cache_dir");
    }

    update_context_guidance_generation_binding(&mut doc, &profile_name);
    write_daemon_config_document(&config_path, &doc)
}

pub(super) fn write_platform_summary_profile(
    repo_root: &Path,
    gateway_url_override: Option<&str>,
) -> Result<()> {
    let config_path = resolve_preferred_daemon_config_path_for_repo(repo_root)?;
    let mut doc = read_daemon_config_document(&config_path)?;

    let profile_name = {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        select_profile_name(
            profiles,
            DEFAULT_PLATFORM_SUMMARY_PROFILE_NAME,
            is_managed_platform_summary_profile,
        )
    };

    {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        let profile = ensure_child_table(profiles, &profile_name);
        profile["task"] = Item::Value(TEXT_GENERATION_TASK.into());
        profile["runtime"] = Item::Value(BITLOOPS_INFERENCE_RUNTIME_ID.into());
        profile["driver"] = Item::Value(BITLOOPS_PLATFORM_CHAT_DRIVER.into());
        profile["model"] = Item::Value(DEFAULT_PLATFORM_SUMMARY_MODEL.into());
        profile["api_key"] = Item::Value(DEFAULT_PLATFORM_SUMMARY_API_KEY.into());
        if let Some(gateway_url_override) = gateway_url_override {
            profile["base_url"] = Item::Value(gateway_url_override.into());
        } else {
            profile.remove("base_url");
        }
        profile["temperature"] = Item::Value(DEFAULT_SUMMARY_TEMPERATURE.into());
        profile["max_output_tokens"] =
            Item::Value(DEFAULT_PLATFORM_SUMMARY_MAX_OUTPUT_TOKENS.into());
        profile.remove("cache_dir");
    }

    update_summary_generation_binding(&mut doc, &profile_name);
    write_daemon_config_document(&config_path, &doc)?;
    maybe_sync_repo_local_summary_mode(repo_root, "auto")
}

pub(super) fn write_platform_context_guidance_profile(
    repo_root: &Path,
    gateway_url_override: Option<&str>,
    api_key_env: &str,
) -> Result<()> {
    let api_key_env = api_key_env.trim();
    if api_key_env.is_empty() {
        bail!("context guidance platform API key environment variable cannot be empty");
    }

    let config_path = resolve_preferred_daemon_config_path_for_repo(repo_root)?;
    let mut doc = read_daemon_config_document(&config_path)?;

    let profile_name = {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        select_profile_name(
            profiles,
            DEFAULT_PLATFORM_CONTEXT_GUIDANCE_PROFILE_NAME,
            is_managed_platform_context_guidance_profile,
        )
    };

    {
        let inference = ensure_table(&mut doc, "inference");
        let profiles = ensure_child_table(inference, "profiles");
        let profile = ensure_child_table(profiles, &profile_name);
        profile["task"] = Item::Value(TEXT_GENERATION_TASK.into());
        profile["runtime"] = Item::Value(BITLOOPS_INFERENCE_RUNTIME_ID.into());
        profile["driver"] = Item::Value(BITLOOPS_PLATFORM_CHAT_DRIVER.into());
        profile["model"] = Item::Value(DEFAULT_PLATFORM_CONTEXT_GUIDANCE_MODEL.into());
        profile["api_key"] = Item::Value(env_placeholder(api_key_env).into());
        if let Some(gateway_url_override) = gateway_url_override {
            profile["base_url"] = Item::Value(gateway_url_override.into());
        } else {
            profile.remove("base_url");
        }
        profile["temperature"] = Item::Value(DEFAULT_SUMMARY_TEMPERATURE.into());
        profile["max_output_tokens"] =
            Item::Value(DEFAULT_CONTEXT_GUIDANCE_MAX_OUTPUT_TOKENS.into());
        profile.remove("cache_dir");
    }

    update_context_guidance_generation_binding(&mut doc, &profile_name);
    write_daemon_config_document(&config_path, &doc)
}

fn read_daemon_config_document(config_path: &Path) -> Result<DocumentMut> {
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("reading Bitloops daemon config {}", config_path.display())
            });
        }
    };

    if contents.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        contents
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing Bitloops daemon config {}", config_path.display()))
    }
}

fn write_daemon_config_document(config_path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating Bitloops config directory {}", parent.display()))?;
    }
    std::fs::write(config_path, doc.to_string())
        .with_context(|| format!("writing Bitloops daemon config {}", config_path.display()))
}

fn maybe_sync_repo_local_summary_mode(repo_root: &Path, mode: &str) -> Result<()> {
    let repo_config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    if !repo_config_path.exists() {
        return Ok(());
    }
    write_summary_mode(&repo_config_path, mode)
}

fn write_summary_mode(config_path: &Path, mode: &str) -> Result<()> {
    let mut doc = read_daemon_config_document(config_path)?;
    let semantic_clones = ensure_table(&mut doc, "semantic_clones");
    semantic_clones["summary_mode"] = Item::Value(mode.into());
    write_daemon_config_document(config_path, &doc)
}

fn update_summary_generation_binding(doc: &mut DocumentMut, profile_name: &str) {
    {
        let semantic_clones = ensure_table(doc, "semantic_clones");
        semantic_clones["summary_mode"] = Item::Value("auto".into());
    }
    update_text_generation_binding(doc, "semantic_clones", "summary_generation", profile_name);
}

fn update_context_guidance_generation_binding(doc: &mut DocumentMut, profile_name: &str) {
    update_text_generation_binding(doc, "context_guidance", "guidance_generation", profile_name);
}

fn update_text_generation_binding(
    doc: &mut DocumentMut,
    capability_table: &str,
    inference_key: &str,
    profile_name: &str,
) {
    let capability = ensure_table(doc, capability_table);
    let inference = ensure_child_table(capability, "inference");
    inference[inference_key] = Item::Value(profile_name.into());
}

fn select_summary_profile_name(profiles: &Table) -> String {
    select_profile_name(
        profiles,
        DEFAULT_SUMMARY_PROFILE_NAME,
        is_managed_summary_profile,
    )
}

fn select_profile_name(
    profiles: &Table,
    default_name: &str,
    is_managed_profile: fn(&Table) -> bool,
) -> String {
    match profiles.get(default_name).and_then(Item::as_table) {
        None => default_name.to_string(),
        Some(profile) if is_managed_profile(profile) => default_name.to_string(),
        Some(_) => next_available_profile_name(profiles, default_name),
    }
}

fn next_available_profile_name(profiles: &Table, prefix: &str) -> String {
    let mut suffix = 1usize;
    loop {
        let candidate = format!("{prefix}_{suffix}");
        if !profiles.contains_key(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn is_managed_summary_profile(profile: &Table) -> bool {
    profile
        .get("task")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::trim)
        == Some(TEXT_GENERATION_TASK)
        && profile
            .get("runtime")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            == Some(BITLOOPS_INFERENCE_RUNTIME_ID)
        && profile
            .get("driver")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            == Some(OLLAMA_CHAT_DRIVER)
}

fn is_managed_platform_summary_profile(profile: &Table) -> bool {
    profile
        .get("task")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::trim)
        == Some(TEXT_GENERATION_TASK)
        && profile
            .get("runtime")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            == Some(BITLOOPS_INFERENCE_RUNTIME_ID)
        && profile
            .get("driver")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|driver| {
                driver == BITLOOPS_PLATFORM_CHAT_DRIVER || driver == "openai_chat_completions"
            })
        && profile
            .get("api_key")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_none_or(|api_key| api_key == DEFAULT_PLATFORM_SUMMARY_API_KEY)
}

fn is_managed_platform_context_guidance_profile(profile: &Table) -> bool {
    profile
        .get("task")
        .and_then(Item::as_value)
        .and_then(|value| value.as_str())
        .map(str::trim)
        == Some(TEXT_GENERATION_TASK)
        && profile
            .get("runtime")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            == Some(BITLOOPS_INFERENCE_RUNTIME_ID)
        && profile
            .get("driver")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|driver| {
                driver == BITLOOPS_PLATFORM_CHAT_DRIVER || driver == "openai_chat_completions"
            })
        && profile
            .get("api_key")
            .and_then(Item::as_value)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_none_or(|api_key| {
                api_key == env_placeholder(DEFAULT_PLATFORM_CONTEXT_GUIDANCE_API_KEY_ENV).as_str()
            })
}

fn env_placeholder(env_name: &str) -> String {
    format!("${{{}}}", env_name.trim())
}

fn read_non_empty_env_value(key: &str) -> Option<String> {
    std::env::var(key).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    if !doc.contains_key(key) || !doc[key].is_table() {
        doc[key] = Item::Table(Table::new());
    }
    doc[key].as_table_mut().expect("table inserted above")
}

fn ensure_child_table<'a>(table: &'a mut Table, key: &str) -> &'a mut Table {
    if !table.contains_key(key) || !table[key].is_table() {
        table[key] = Item::Table(Table::new());
    }
    table[key].as_table_mut().expect("table inserted above")
}

#[cfg(test)]
pub(crate) fn with_summary_generation_configured_hook<T>(
    hook: impl Fn(&Path) -> bool + 'static,
    f: impl FnOnce() -> T,
) -> T {
    SUMMARY_GENERATION_CONFIGURED_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "summary generation configured hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let result = f();
    SUMMARY_GENERATION_CONFIGURED_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
pub(crate) fn with_context_guidance_generation_configured_hook<T>(
    hook: impl Fn(&Path) -> bool + 'static,
    f: impl FnOnce() -> T,
) -> T {
    CONTEXT_GUIDANCE_GENERATION_CONFIGURED_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "context guidance generation configured hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let result = f();
    CONTEXT_GUIDANCE_GENERATION_CONFIGURED_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}
