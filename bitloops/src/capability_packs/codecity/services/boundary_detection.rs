mod builder;
mod detection;
mod entry_points;
mod hierarchy;
mod implicit;
mod manifest;
mod model;
mod naming;
mod runtime;

#[cfg(test)]
mod tests;

pub use detection::detect_boundaries;
pub use model::CodeCityBoundaryDetectionResult;
