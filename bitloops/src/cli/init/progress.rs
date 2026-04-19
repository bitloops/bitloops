//! Compact runtime init progress display, split into focused submodules.
//!
//! The active rendering path (used in production) is composed of the
//! [`driver`] loop driving [`renderer::RuntimeInitRenderer`], which in turn
//! delegates to the [`compact`], [`session_status`], [`progress_calc`],
//! [`task_lookup`], [`bars`] and [`viewport`] helpers. The [`legacy`]
//! module retains the older verbose renderer for reference.

mod bars;
mod compact;
mod driver;
mod legacy;
mod progress_calc;
mod renderer;
mod session_status;
mod task_lookup;
mod viewport;

pub(crate) use driver::{InitProgressOptions, run_dual_init_progress};
