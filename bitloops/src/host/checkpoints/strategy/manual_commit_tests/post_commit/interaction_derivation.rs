//! Post-commit checkpoint derivation from interaction event sources (tests).

#[path = "interaction_derivation/derive_failure.rs"]
mod derive_failure;
#[path = "interaction_derivation/derive_session_state.rs"]
mod derive_session_state;
#[path = "interaction_derivation/derive_success.rs"]
mod derive_success;
#[path = "interaction_derivation/derive_transcript.rs"]
mod derive_transcript;
#[path = "interaction_derivation/fakes.rs"]
mod fakes;
#[path = "interaction_derivation/fixtures.rs"]
mod fixtures;
#[path = "interaction_derivation/integration_clickhouse.rs"]
mod integration_clickhouse;
#[path = "interaction_derivation/integration_duckdb.rs"]
mod integration_duckdb;

pub(crate) use derive_failure::*;
pub(crate) use derive_session_state::*;
pub(crate) use derive_success::*;
pub(crate) use derive_transcript::*;
pub(crate) use integration_clickhouse::*;
pub(crate) use integration_duckdb::*;
