//! Standalone Hugging Face / sentence-transformers embeddings runtime client.
//!
//! The implementation is split across cohesive submodules:
//!
//! - [`service`] exposes the public [`BitloopsEmbeddingsIpcService`] used by the
//!   inference gateway.
//! - [`shared`] hosts the process-wide registry that pools idle subprocess
//!   sessions across service instances.
//! - [`session`] owns a single Python embeddings subprocess and the JSON line
//!   protocol spoken over its stdio pipes.
//! - [`runtime`] holds environment fingerprinting, cache discovery and timeout
//!   helpers shared by the session and service layers.
//! - [`auth`] resolves the bearer token needed for platform-backed runtimes.

#[path = "embeddings/auth.rs"]
mod auth;
#[path = "embeddings/runtime.rs"]
mod runtime;
#[path = "embeddings/service.rs"]
mod service;
#[path = "embeddings/session.rs"]
mod session;
#[path = "embeddings/shared.rs"]
mod shared;

#[cfg(test)]
#[path = "embeddings/tests.rs"]
mod tests;

pub(super) use service::BitloopsEmbeddingsIpcService;
