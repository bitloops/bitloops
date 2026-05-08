use std::path::Path;

use anyhow::Result;

#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

#[cfg(test)]
type WatcherReconciliationHook = dyn Fn(&Path, bool) -> Result<()> + 'static;

#[cfg(test)]
thread_local! {
    static WATCHER_RECONCILIATION_HOOK: RefCell<Option<Rc<WatcherReconciliationHook>>> =
        RefCell::new(None);
}

#[cfg(test)]
struct WatcherReconciliationHookGuard;

#[cfg(test)]
impl Drop for WatcherReconciliationHookGuard {
    fn drop(&mut self) {
        WATCHER_RECONCILIATION_HOOK.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

pub(crate) fn reconcile_repo_watcher(repo_root: &Path) -> Result<()> {
    #[cfg(test)]
    let watcher_enabled = crate::config::settings::is_enabled_for_hooks(repo_root)
        && crate::config::settings::devql_sync_enabled(repo_root)?;

    #[cfg(test)]
    if let Some(result) = maybe_reconcile_repo_watcher_via_hook(repo_root, watcher_enabled) {
        return result;
    }

    #[cfg(test)]
    {
        Ok(())
    }

    #[cfg(not(test))]
    {
        let daemon_config_root =
            crate::config::resolve_bound_daemon_config_root_for_repo(repo_root)?;
        if crate::config::settings::devql_sync_enabled(repo_root)? {
            crate::host::devql::watch::restart_watcher(repo_root, &daemon_config_root)
        } else {
            crate::host::devql::watch::stop_watcher(repo_root, &daemon_config_root)
        }
    }
}

#[cfg(test)]
pub(crate) fn with_watcher_reconciliation_hook<T>(
    hook: impl Fn(&Path, bool) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    WATCHER_RECONCILIATION_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "watcher reconciliation hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });

    let _guard = WatcherReconciliationHookGuard;
    f()
}

#[cfg(test)]
fn maybe_reconcile_repo_watcher_via_hook(
    repo_root: &Path,
    watcher_enabled: bool,
) -> Option<Result<()>> {
    WATCHER_RECONCILIATION_HOOK.with(|hook: &RefCell<Option<Rc<WatcherReconciliationHook>>>| {
        hook.borrow()
            .as_ref()
            .map(|hook| hook(repo_root, watcher_enabled))
    })
}
