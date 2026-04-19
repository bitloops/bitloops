use anyhow::{Result, bail};

use super::session::PythonEmbeddingsSessionConfig;

#[cfg(test)]
pub(crate) type PlatformRuntimeAuthEnvironmentHook = dyn Fn(&str) -> Result<Vec<(String, String)>>;
#[cfg(test)]
pub(crate) type PlatformRuntimeAuthEnvironmentHookCell =
    std::cell::RefCell<Option<std::rc::Rc<PlatformRuntimeAuthEnvironmentHook>>>;

#[cfg(test)]
thread_local! {
    pub(crate) static PLATFORM_RUNTIME_AUTH_ENVIRONMENT_HOOK: PlatformRuntimeAuthEnvironmentHookCell =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn with_platform_runtime_auth_environment_hook<T>(
    hook: impl Fn(&str) -> Result<Vec<(String, String)>> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    PLATFORM_RUNTIME_AUTH_ENVIRONMENT_HOOK.with(|cell| {
        let previous = cell.replace(Some(std::rc::Rc::new(hook)));
        let output = f();
        cell.replace(previous);
        output
    })
}

fn platform_runtime_api_key_env(args: &[String]) -> &str {
    args.windows(2)
        .find(|window| window[0] == "--api-key-env")
        .and_then(|window| {
            let value = window[1].trim();
            (!value.is_empty()).then_some(value)
        })
        .unwrap_or(crate::daemon::PLATFORM_GATEWAY_TOKEN_ENV)
}

pub(crate) fn platform_runtime_auth_environment(
    config: &PythonEmbeddingsSessionConfig,
) -> Vec<(String, String)> {
    let api_key_env = platform_runtime_api_key_env(&config.args);

    #[cfg(test)]
    if let Some(result) = PLATFORM_RUNTIME_AUTH_ENVIRONMENT_HOOK
        .with(|cell| cell.borrow().clone())
        .map(|hook| hook(api_key_env))
    {
        return result.unwrap_or_else(|err| {
            log::debug!("skipping platform gateway auth injection via test hook: {err:#}");
            Vec::new()
        });
    }

    if let Ok(token) = std::env::var(api_key_env)
        && !token.trim().is_empty()
    {
        return vec![(api_key_env.to_string(), token)];
    }

    match crate::daemon::platform_gateway_bearer_token() {
        Ok(Some(token)) => vec![(api_key_env.to_string(), token)],
        Ok(None) => Vec::new(),
        Err(err) => {
            log::debug!("skipping platform gateway auth injection: {err:#}");
            Vec::new()
        }
    }
}

pub(crate) fn ensure_platform_runtime_auth_environment_available(
    config: &PythonEmbeddingsSessionConfig,
) -> Result<()> {
    if !config.platform_backed {
        return Ok(());
    }

    if !platform_runtime_auth_environment(config).is_empty() {
        return Ok(());
    }

    bail!(
        "platform-backed embeddings profile requires an authenticated Bitloops session or `{}` to be set",
        crate::daemon::PLATFORM_GATEWAY_TOKEN_ENV
    );
}
