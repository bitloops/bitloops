#![cfg_attr(not(test), allow(dead_code))]

#[path = "content_cache/lookup.rs"]
mod lookup;
#[path = "content_cache/persist.rs"]
mod persist;
#[path = "content_cache/sql.rs"]
mod sql;
#[cfg(test)]
#[path = "content_cache/tests.rs"]
mod tests;
#[path = "content_cache/types.rs"]
mod types;

#[allow(unused_imports)]
pub(crate) use self::lookup::{lookup_cached_content, lookup_cached_content_with_connection};
#[allow(unused_imports)]
pub(crate) use self::persist::{
    deduped_cached_content_parts, persist_cached_content_tx, promote_cached_content_to_git_backed,
    promote_to_git_backed, store_cached_content, touch_cache_entries_tx,
};
pub(crate) use self::types::{CacheKey, CachedArtefact, CachedEdge, CachedExtraction};
