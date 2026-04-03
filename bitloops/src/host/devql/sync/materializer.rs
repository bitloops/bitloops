#![cfg_attr(not(test), allow(dead_code))]

mod derive;
mod persist;
mod sql;
#[cfg(test)]
mod tests;
mod types;

#[cfg(test)]
use self::derive::parse_cached_language_kind;
pub(crate) use self::derive::prepare_materialization_rows;
#[cfg(test)]
pub(crate) use self::persist::{materialize_path, remove_path};
pub(crate) use self::persist::{persist_prepared_materialisation_tx, remove_paths_tx};
pub(crate) use self::types::PreparedMaterialisationRows;
