use anyhow::Result;

#[cfg(test)]
type CloudLoginStatusHook = dyn Fn() -> Result<bool>;
#[cfg(test)]
type CloudLoginStatusHookCell = std::cell::RefCell<Option<std::rc::Rc<CloudLoginStatusHook>>>;

#[cfg(test)]
thread_local! {
    static CLOUD_LOGIN_STATUS_HOOK: CloudLoginStatusHookCell = std::cell::RefCell::new(None);
}

pub(super) async fn resolve_cloud_logged_in_for_optional_setup() -> Result<bool> {
    #[cfg(test)]
    if let Some(hook) = CLOUD_LOGIN_STATUS_HOOK.with(|cell| cell.borrow().clone()) {
        return hook();
    }

    Ok(crate::daemon::resolve_workos_session_status()
        .await?
        .is_some())
}

#[cfg(test)]
pub(crate) fn with_cloud_login_status_hook<T>(
    hook: impl Fn() -> Result<bool> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    CLOUD_LOGIN_STATUS_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "cloud login status hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let result = f();
    CLOUD_LOGIN_STATUS_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}
