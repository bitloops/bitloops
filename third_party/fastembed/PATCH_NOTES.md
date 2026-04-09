# Fastembed Patch Notes

This directory is a temporary `[patch.crates-io]` override for `fastembed 5.13.0`.

Why it exists:

- Bitloops runs local embeddings in a child runtime process.
- Upstream `fastembed` initializes ONNX sessions with the machine-wide parallelism count.
- On laptops this lets a single local embedding worker fan out across many cores and peg CPU.

What Bitloops changed:

- Added `FASTEMBED_THREADS` support in `src/common.rs`.
- Routed ONNX session creation through that helper in:
  - `src/text_embedding/impl.rs`
  - `src/image_embedding/impl.rs`
  - `src/reranking/impl.rs`
  - `src/sparse_text_embedding/impl.rs`
- Bitloops sets `FASTEMBED_THREADS=1` for the local runtime child in:
  - `/Users/elli/Projects/Bitloops/bitloops/bitloops/src/adapters/model_providers/embeddings.rs`

Intended end state:

- Upstream the `FASTEMBED_THREADS` support to `fastembed`.
- Remove the `[patch.crates-io] fastembed = { path = "third_party/fastembed" }` override.

How to regenerate the upstream patch:

```bash
mkdir -p /tmp/fastembed-upstream
tar -xzf "$HOME/.cargo/registry/cache/index.crates.io-1949cf8c6b5b557f/fastembed-5.13.0.crate" -C /tmp/fastembed-upstream
diff -ru /tmp/fastembed-upstream/fastembed-5.13.0 third_party/fastembed > third_party/patches/fastembed-5.13.0-thread-cap.patch
```
