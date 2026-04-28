mod hash;
mod identity;
mod input;
mod rows;
mod text;
mod types;

#[cfg(test)]
mod tests;

pub use self::hash::{build_symbol_embedding_input_hash, symbol_embeddings_require_reindex};
pub use self::input::build_symbol_embedding_inputs;
pub use self::rows::{
    build_symbol_embedding_row, build_symbol_embedding_rows, resolve_embedding_setup,
};
pub use self::text::build_symbol_embedding_text;
pub use self::types::{
    ActiveEmbeddingRepresentationState, EmbeddingRepresentationKind, EmbeddingSetup,
    SymbolEmbeddingIndexState, SymbolEmbeddingIngestionStats, SymbolEmbeddingInput,
    SymbolEmbeddingRow,
};
