mod core;
mod knowledge;

use crate::engine::devql::cucumber_world::DevqlBddWorld;
use cucumber::step::Collection;

pub(super) fn collection() -> Collection<DevqlBddWorld> {
    let collection = core::collection();
    knowledge::register(collection)
}
