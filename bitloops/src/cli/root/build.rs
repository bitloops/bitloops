pub(crate) fn build_version() -> &'static str {
    option_env!("BITLOOPS_BUILD_VERSION").unwrap_or("dev")
}

pub(crate) fn build_commit() -> &'static str {
    option_env!("BITLOOPS_BUILD_COMMIT").unwrap_or("unknown")
}

pub(crate) fn build_target() -> &'static str {
    option_env!("BITLOOPS_BUILD_TARGET")
        .or(option_env!("TARGET"))
        .unwrap_or("unknown")
}

pub(crate) fn build_date() -> &'static str {
    option_env!("BITLOOPS_BUILD_DATE").unwrap_or("unknown")
}
