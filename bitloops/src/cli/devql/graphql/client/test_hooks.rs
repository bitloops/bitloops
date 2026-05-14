use std::future::Future;
use std::path::Path;

use anyhow::Result;

use crate::devql_transport::SlimCliRepoScope;

#[cfg(test)]
type TaskDaemonBootstrapHook = dyn Fn(&Path) -> Result<()> + 'static;

#[cfg(test)]
type TaskDaemonBootstrapHookCell = std::cell::RefCell<Option<std::rc::Rc<TaskDaemonBootstrapHook>>>;

#[cfg(test)]
thread_local! {
    static TASK_DAEMON_BOOTSTRAP_HOOK: TaskDaemonBootstrapHookCell =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
thread_local! {
    static GRAPHQL_EXECUTOR_HOOK: std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
type GraphqlExecutorHook =
    dyn Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static;

#[cfg(test)]
thread_local! {
    static SCHEMA_SDL_FETCH_HOOK: std::cell::RefCell<Option<std::rc::Rc<SchemaSdlFetchHook>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
type SchemaSdlFetchHook =
    dyn Fn(&str, Option<&SlimCliRepoScope>) -> Result<(u16, String)> + 'static;

#[cfg(test)]
struct ThreadLocalHookGuard(fn());

#[cfg(test)]
impl Drop for ThreadLocalHookGuard {
    fn drop(&mut self) {
        (self.0)();
    }
}

#[cfg(test)]
pub(crate) fn with_task_daemon_bootstrap_hook<T>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    TASK_DAEMON_BOOTSTRAP_HOOK.with(|cell: &TaskDaemonBootstrapHookCell| {
        assert!(
            cell.borrow().is_none(),
            "task daemon hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let _guard = ThreadLocalHookGuard(clear_task_daemon_bootstrap_hook);
    f()
}

#[cfg(test)]
pub(crate) async fn with_task_daemon_bootstrap_hook_async<T, Fut>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> Fut,
) -> T
where
    Fut: Future<Output = T>,
{
    TASK_DAEMON_BOOTSTRAP_HOOK.with(|cell: &TaskDaemonBootstrapHookCell| {
        assert!(
            cell.borrow().is_none(),
            "task daemon hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let _guard = ThreadLocalHookGuard(clear_task_daemon_bootstrap_hook);
    f().await
}

#[cfg(test)]
pub(crate) fn with_ingest_daemon_bootstrap_hook<T>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    with_task_daemon_bootstrap_hook(hook, f)
}

#[cfg(test)]
pub(crate) async fn with_ingest_daemon_bootstrap_hook_async<T, Fut>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> Fut,
) -> T
where
    Fut: Future<Output = T>,
{
    with_task_daemon_bootstrap_hook_async(hook, f).await
}

#[cfg(test)]
pub(super) fn maybe_bootstrap_daemon_via_hook(repo_root: &Path) -> Option<Result<()>> {
    TASK_DAEMON_BOOTSTRAP_HOOK.with(|hook: &TaskDaemonBootstrapHookCell| {
        hook.borrow().as_ref().map(|hook| hook(repo_root))
    })
}

#[cfg(test)]
fn clear_task_daemon_bootstrap_hook() {
    TASK_DAEMON_BOOTSTRAP_HOOK.with(|cell: &TaskDaemonBootstrapHookCell| {
        *cell.borrow_mut() = None;
    });
}

#[cfg(test)]
pub(crate) fn with_schema_sdl_fetch_hook<T>(
    hook: impl Fn(&str, Option<&SlimCliRepoScope>) -> Result<(u16, String)> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    SCHEMA_SDL_FETCH_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<SchemaSdlFetchHook>>>| {
            assert!(
                cell.borrow().is_none(),
                "schema SDL fetch hook already installed"
            );
            *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
        },
    );
    let _guard = ThreadLocalHookGuard(clear_schema_sdl_fetch_hook);
    f()
}

#[cfg(test)]
pub(super) fn maybe_fetch_schema_sdl_via_hook(
    endpoint_path: &str,
    scope: Option<&SlimCliRepoScope>,
) -> Option<Result<(u16, String)>> {
    SCHEMA_SDL_FETCH_HOOK.with(
        |hook: &std::cell::RefCell<Option<std::rc::Rc<SchemaSdlFetchHook>>>| {
            hook.borrow()
                .as_ref()
                .map(|hook| hook(endpoint_path, scope))
        },
    )
}

#[cfg(test)]
fn clear_schema_sdl_fetch_hook() {
    SCHEMA_SDL_FETCH_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<SchemaSdlFetchHook>>>| {
            *cell.borrow_mut() = None;
        },
    );
}

#[cfg(test)]
pub(crate) fn with_graphql_executor_hook<T>(
    hook: impl Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            assert!(
                cell.borrow().is_none(),
                "graphql executor hook already installed"
            );
            *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
        },
    );
    let _guard = ThreadLocalHookGuard(clear_graphql_executor_hook);
    f()
}

#[cfg(test)]
pub(crate) async fn with_graphql_executor_hook_async<T, Fut>(
    hook: impl Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static,
    f: impl FnOnce() -> Fut,
) -> T
where
    Fut: Future<Output = T>,
{
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            assert!(
                cell.borrow().is_none(),
                "graphql executor hook already installed"
            );
            *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
        },
    );
    let _guard = ThreadLocalHookGuard(clear_graphql_executor_hook);
    f().await
}

#[cfg(test)]
pub(super) fn maybe_execute_devql_graphql_via_hook(
    repo_root: &Path,
    query: &str,
    variables: &serde_json::Value,
) -> Option<Result<serde_json::Value>> {
    GRAPHQL_EXECUTOR_HOOK.with(
        |hook: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            hook.borrow()
                .as_ref()
                .map(|hook| hook(repo_root, query, variables))
        },
    )
}
#[cfg(test)]
pub(super) fn graphql_executor_hook_installed() -> bool {
    GRAPHQL_EXECUTOR_HOOK.with(
        |hook: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            hook.borrow().is_some()
        },
    )
}

#[cfg(test)]
fn clear_graphql_executor_hook() {
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            *cell.borrow_mut() = None;
        },
    );
}
