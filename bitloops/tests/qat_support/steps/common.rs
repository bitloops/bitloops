use crate::qat_support::world::QatWorld;
use cucumber::codegen::LocalBoxFuture;
use regex::Regex;

pub(super) fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|err| panic!("invalid step regex `{pattern}`: {err}"))
}

pub(super) fn step_fn(
    f: for<'a> fn(&'a mut QatWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()>,
) -> for<'a> fn(&'a mut QatWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()> {
    f
}

pub(super) fn run_step(step_name: &str, result: anyhow::Result<()>) {
    if let Err(err) = result {
        panic!("{step_name} failed: {err:#}");
    }
}
