mod core;
mod knowledge;
mod resilience;

use crate::host::devql::cucumber_world::DevqlBddWorld;
use cucumber::step::Collection;

pub(super) fn collection() -> Collection<DevqlBddWorld> {
    let collection = core::collection();
    let collection = knowledge::register(collection);
    resilience::register(collection)
}
