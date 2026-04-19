//! Managed runtime asset download pipeline.
//!
//! The dispatcher in [`entry`] selects between a parallel range-based download
//! and a serial fallback, while [`parallel`], [`serial`], [`http`], [`types`]
//! and [`config`] host the supporting machinery. Tests live alongside in
//! [`tests`] and exercise the public dispatcher against an in-process mock
//! HTTP server.

mod config;
mod entry;
mod http;
mod parallel;
mod serial;
mod types;

pub(crate) use entry::download_release_asset_to_temp_file;
#[allow(unused_imports)]
pub(crate) use types::DownloadedManagedAsset;

#[cfg(test)]
mod tests;
