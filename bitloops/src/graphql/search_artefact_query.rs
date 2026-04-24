mod lexical;
mod scoring;
mod selection;
mod storage;
mod types;

pub(crate) use self::selection::select_search_artefacts;

pub(super) const SEARCH_RESULT_LIMIT: usize = 20;
pub(super) const SEARCH_BREAKDOWN_LIMIT: usize = 10;
pub(super) const SEARCH_CANDIDATE_LIMIT: usize = 100;
pub(super) const POSTGRES_SIMILARITY_THRESHOLD: f64 = 0.2;
