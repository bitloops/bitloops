# Build (from the bitloops directory)

cd /Users/markos/code/bitloops/cli/bitloops

# Required once per environment: build-time dashboard URL config

cp config/dashboard_urls.template.json config/dashboard_urls.json

# edit config/dashboard_urls.json with real values

# build script validation runs during check/build

cargo check

cargo build

# Then run it from ANY directory

cd /path/to/some-other-repo
/Users/markos/code/bitloops/cli/bitloops/target/debug/bitloops init
/Users/markos/code/bitloops/cli/bitloops/target/debug/bitloops enable

# OR INSTEAD, BETTER

cargo install --path . --force

# Make sure cargo is in your PATH

# this will make the `bitloops` command available globally, so you can just run

bitloops --version

# Follow these steps

1. git init
2. create + commit a tiny initial file (README.md)
3. bitloops init
4. bitloops enable
5. chat with Claude (so hooks run and stop snapshots)
6. git commit → Bitloops now stores checkpoint-to-commit mappings in relational state

# Stage 3 ANN A/B QA benchmark (report-only)

Run this from the `bitloops` directory to compare Stage 3 with ANN ON vs ANN OFF
using the same binary and a real repo snapshot. The benchmark uses isolated app-state
per mode, 1 warmup run (excluded), and 7 measured iterations by default:

```bash
./scripts/qa/semantic_clones_ann_ab.sh \
  --repo /absolute/path/to/repo \
  --symbol-fqn "src/services/orders.ts::OrderService.create" \
  --iterations 7 \
  --warmup 1 \
  --neighbors 5
```

Inputs:
- `--repo` (required): target repository root
- `--symbol-fqn` (required): one source symbol for `clones(neighbors:...)`
- `--iterations` (optional, default `7`)
- `--warmup` (optional, default `1`)
- `--neighbors` (optional, default `5`, clamped to `1..50`)
- `--ort-dylib-path` (optional): explicit local ONNX Runtime library path
- `--require-nonempty` / `--allow-empty` (optional): enforce non-empty query rows (default: required)
- `--bootstrap-sync` / `--skip-bootstrap-sync` (optional): run `devql sync` + `devql sync --status` before timed iterations (default: enabled)

ORT provisioning behavior:
- resolve in order: `--ort-dylib-path` -> `ORT_DYLIB_PATH` -> repo cache under
  `target/qa/tools/onnxruntime/...` -> auto-download to repo cache.
- the library path is exported only to benchmark subprocesses, with no global install side effects.
- each benchmark mode uses isolated app-state config with deterministic local semantic-clones embeddings (bind `code_embeddings = "local_code"` and `summary_embeddings = "local_code"`).

Outputs:
- terminal summary table (median/p95 + ANN speedup % for ingest/query)
- machine-readable JSON report under `target/qa/semantic_clones_ann_ab_*.json`
- per-run logs under `target/qa/runs/<run-id>/`
- report validity fields: `valid_run`, `failure_reason`, `ann_on_row_count`, `ann_off_row_count`

Interpretation:
- Positive speedup % means ANN ON is faster than ANN OFF.
- `ingest_ms` reflects rebuild-inclusive `bitloops devql ingest`.
- `query_ms` reflects on-demand `clones(neighbors:...)` latency and is meaningful only when
  both modes return non-empty rows and comparable counts.
- invalid runs still keep measured timing samples captured before the failing validation to aid diagnosis.
- If validity gates fail (ingest/runtime errors, invalid JSON, or empty rows with non-empty enforcement),
  the script exits non-zero and marks `valid_run: false`.
- The runner starts/stops an isolated daemon per mode using per-run app state.
