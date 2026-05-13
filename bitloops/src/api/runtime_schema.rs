// Slim facade for the runtime DevQL surface.
//
// The runtime schema powers internal init/status/dashboard operational views.
// Each cohesive responsibility lives in its own submodule below; this file only
// declares the modules and re-exports the items required by the rest of the
// crate (the parent `api` module, the router, the dashboard runtime bundle and
// the test scaffolding).

mod config;
mod config_management;
mod debug;
mod events;
mod handlers;
mod init_session;
mod roots;
mod schema;
mod snapshot;
mod start_init;
mod util;
mod watchers;

pub(crate) use handlers::{
    runtime_graphql_handler, runtime_graphql_playground_handler, runtime_graphql_sdl_handler,
    runtime_graphql_ws_handler,
};
pub use schema::runtime_schema_sdl;
pub(crate) use schema::{RuntimeGraphqlSchema, build_runtime_schema_template};
