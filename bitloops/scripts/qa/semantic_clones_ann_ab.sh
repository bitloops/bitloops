#!/usr/bin/env bash
set -euo pipefail

MIN_NEIGHBORS=1
MAX_NEIGHBORS=50
DEFAULT_NEIGHBORS=5
DEFAULT_ITERATIONS=7
DEFAULT_WARMUP=1
DEFAULT_REPO_NAME="bitloops"
DEFAULT_REQUIRE_NONEMPTY=1
DEFAULT_BOOTSTRAP_SYNC=1
DEFAULT_LOCAL_EMBEDDING_MODEL="jinaai/jina-embeddings-v2-base-code"
DEFAULT_ENRICHMENT_IDLE_TIMEOUT_SECS=600
DEFAULT_ENRICHMENT_POLL_INTERVAL_SECS=2
DEFAULT_EMBEDDINGS_STARTUP_TIMEOUT_SECS=180
DEFAULT_EMBEDDINGS_REQUEST_TIMEOUT_SECS=180

DISABLE_ANN_ENV="BITLOOPS_SEMANTIC_CLONES_DISABLE_ANN"
MOCK_ENV="BITLOOPS_ANN_AB_MOCK"
MOCK_QUERY_ROWS_ENV="BITLOOPS_ANN_AB_MOCK_QUERY_ROWS"
MOCK_QUERY_NESTED_EMPTY_ENV="BITLOOPS_ANN_AB_MOCK_QUERY_NESTED_EMPTY"
MOCK_INVALID_QUERY_JSON_ENV="BITLOOPS_ANN_AB_MOCK_INVALID_QUERY_JSON"
MOCK_INGEST_ERROR_ENV="BITLOOPS_ANN_AB_MOCK_INGEST_ERROR"

ORT_VERSION="1.20.1"
ORT_RELEASE_BASE_URL="https://github.com/microsoft/onnxruntime/releases/download"

LAST_CMD_STATUS=0
LAST_CMD_ELAPSED_MS=0
LAST_ENRICHMENT_WAIT_MS=0

usage() {
  cat <<'USAGE'
Usage:
  semantic_clones_ann_ab.sh --repo <repo-path> --symbol-fqn <symbol-fqn> [options]

Required:
  --repo <path>              Repository root to benchmark.
  --symbol-fqn <fqn>         Single source symbol FQN used for clones(neighbors:...).

Optional:
  --iterations <n>           Measured iterations per mode (default: 7).
  --warmup <n>               Warmup iterations per mode, excluded from stats (default: 1).
  --neighbors <k>            Neighbors override for query (default: 5, clamped 1..50).
  --repo-name <name>         DevQL repo(...) name (default: bitloops).
  --binary <path|name>       Bitloops binary (default: bitloops in PATH).
  --ort-dylib-path <path>    Explicit ONNX Runtime dylib path.
  --require-nonempty         Require query to return non-empty clone rows (default).
  --allow-empty              Disable non-empty requirement.
  --bootstrap-sync           Run devql sync + sync --status before measurements (default).
  --skip-bootstrap-sync      Skip pre-measure sync bootstrap.
  --output <path>            Explicit output JSON path.
  --help                     Show this help.

Notes:
  - Mode A (ANN ON) runs default Stage 3 behavior.
  - Mode B (ANN OFF) sets BITLOOPS_SEMANTIC_CLONES_DISABLE_ANN=1.
  - ONNX Runtime is resolved in this order: --ort-dylib-path, ORT_DYLIB_PATH,
    repo cache under target/qa/tools/onnxruntime, then download to that cache.
  - Benchmark config forces local deterministic semantic-clones embeddings in isolated app-state.
  - Set BITLOOPS_ANN_AB_MOCK=1 for fast smoke validation without real CLI calls.
USAGE
}

fail() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

is_integer() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

to_json_array() {
  local values=("$@")
  local out=""
  local value
  for value in "${values[@]}"; do
    if [[ -n "$out" ]]; then
      out+=", "
    fi
    out+="$value"
  done
  printf '[%s]' "$out"
}

now_ms() {
  perl -MTime::HiRes=time -e 'printf "%.0f\n", time()*1000'
}

resolve_dyld_fallback_path() {
  local inherited="$1"
  local binary="$2"
  if [[ -n "$inherited" ]]; then
    echo "$inherited"
    return 0
  fi

  if [[ "$binary" == */* ]]; then
    local binary_dir
    binary_dir="$(dirname "$binary")"
    local candidate="$binary_dir/deps"
    if [[ -d "$candidate" ]]; then
      echo "$candidate"
      return 0
    fi
  fi

  echo ""
}

reserve_local_port() {
  local reserved
  reserved="$(
    perl -MIO::Socket::INET -e '
    my $sock = IO::Socket::INET->new(
      Listen    => 1,
      LocalAddr => "127.0.0.1",
      LocalPort => 0,
      Proto     => "tcp",
      ReuseAddr => 1
    ) or die "unable to reserve local TCP port\n";
    print $sock->sockport();
  ' 2>/dev/null || true
  )"
  if [[ -n "$reserved" ]]; then
    echo "$reserved"
    return 0
  fi
  echo $((30000 + (RANDOM % 20000)))
}

calc_stats() {
  if [[ "$#" -eq 0 ]]; then
    printf '0 0\n'
    return 0
  fi

  printf '%s\n' "$@" | LC_ALL=C sort -n | awk '
    { values[NR] = $1 }
    END {
      count = NR
      if (count == 0) {
        printf "0 0\n"
        exit
      }

      if (count % 2 == 1) {
        median = values[(count + 1) / 2]
      } else {
        median = int((values[count / 2] + values[(count / 2) + 1]) / 2)
      }

      p95_index = int((95 * count + 99) / 100)
      if (p95_index < 1) {
        p95_index = 1
      }
      if (p95_index > count) {
        p95_index = count
      }
      p95 = values[p95_index]

      printf "%d %d\n", median, p95
    }'
}

speedup_percent() {
  local baseline_ms="$1"
  local improved_ms="$2"
  LC_ALL=C awk -v baseline="$baseline_ms" -v improved="$improved_ms" '
    BEGIN {
      if (baseline <= 0) {
        printf "0.00"
        exit
      }
      printf "%.2f", ((baseline - improved) / baseline) * 100
    }'
}

mock_duration_ms() {
  local mode="$1"
  local metric="$2"
  local iteration="$3"
  case "${mode}:${metric}" in
    ann_on:ingest) echo $((780 + iteration * 3)) ;;
    ann_on:query) echo $((95 + iteration * 2)) ;;
    ann_off:ingest) echo $((1280 + iteration * 4)) ;;
    ann_off:query) echo $((185 + iteration * 2)) ;;
    *) echo $((100 + iteration)) ;;
  esac
}

ensure_binary() {
  local binary="$1"
  if [[ -n "${!MOCK_ENV:-}" && "${!MOCK_ENV}" != "0" ]]; then
    return 0
  fi
  if [[ "$binary" == */* ]]; then
    [[ -x "$binary" ]] || fail "binary is not executable: $binary"
    return 0
  fi
  command -v "$binary" >/dev/null 2>&1 || fail "binary not found in PATH: $binary"
}

resolve_binary_path() {
  local binary="$1"
  if [[ "$binary" == */* ]]; then
    echo "$binary"
    return 0
  fi
  command -v "$binary"
}

ort_platform_archive_name() {
  local os_name arch_name
  os_name="$(uname -s)"
  arch_name="$(uname -m)"

  case "${os_name}:${arch_name}" in
    Darwin:arm64)
      echo "onnxruntime-osx-arm64-${ORT_VERSION}.tgz"
      ;;
    Darwin:x86_64)
      echo "onnxruntime-osx-x86_64-${ORT_VERSION}.tgz"
      ;;
    Linux:x86_64)
      echo "onnxruntime-linux-x64-${ORT_VERSION}.tgz"
      ;;
    Linux:aarch64)
      echo "onnxruntime-linux-aarch64-${ORT_VERSION}.tgz"
      ;;
    *)
      fail "unsupported platform for ORT auto-download: ${os_name}/${arch_name}"
      ;;
  esac
}

ort_platform_tag() {
  local os_name arch_name
  os_name="$(uname -s)"
  arch_name="$(uname -m)"

  case "${os_name}:${arch_name}" in
    Darwin:arm64) echo "osx-arm64" ;;
    Darwin:x86_64) echo "osx-x86_64" ;;
    Linux:x86_64) echo "linux-x64" ;;
    Linux:aarch64) echo "linux-aarch64" ;;
    *) fail "unsupported platform for ORT cache tag: ${os_name}/${arch_name}" ;;
  esac
}

ort_library_filename() {
  local os_name
  os_name="$(uname -s)"
  case "$os_name" in
    Darwin) echo "libonnxruntime.dylib" ;;
    Linux) echo "libonnxruntime.so" ;;
    *) fail "unsupported platform for ORT library name: ${os_name}" ;;
  esac
}

resolve_repo_cached_ort_path() {
  local repo_path="$1"
  local tag library_name
  tag="$(ort_platform_tag)"
  library_name="$(ort_library_filename)"
  echo "$repo_path/target/qa/tools/onnxruntime/${ORT_VERSION}/${tag}/lib/${library_name}"
}

ensure_repo_local_ort() {
  local repo_path="$1"
  local target_library="$2"

  local tools_root archive_dir archive_name archive_path extract_dir
  tools_root="$repo_path/target/qa/tools/onnxruntime"
  archive_dir="$tools_root/_downloads"
  archive_name="$(ort_platform_archive_name)"
  archive_path="$archive_dir/$archive_name"
  extract_dir="$archive_dir/extracted-${ORT_VERSION}-$$"

  mkdir -p "$archive_dir"

  if [[ ! -f "$archive_path" ]]; then
    local url
    url="$ORT_RELEASE_BASE_URL/v${ORT_VERSION}/${archive_name}"
    curl -fL --retry 2 --connect-timeout 20 -o "$archive_path" "$url" >/dev/null 2>&1 || {
      fail "failed to download ORT archive from ${url}. Provide --ort-dylib-path or ORT_DYLIB_PATH."
    }
  fi

  rm -rf "$extract_dir"
  mkdir -p "$extract_dir"
  tar -xzf "$archive_path" -C "$extract_dir"

  local discovered
  discovered="$(find "$extract_dir" -name "$(ort_library_filename)" -type f | head -n 1)"
  if [[ -z "$discovered" ]]; then
    rm -rf "$extract_dir"
    fail "downloaded ORT archive did not contain $(ort_library_filename)"
  fi

  mkdir -p "$(dirname "$target_library")"
  cp "$discovered" "$target_library"
  rm -rf "$extract_dir"
}

resolve_ort_dylib_path() {
  local repo_path="$1"
  local override_path="$2"

  if [[ -n "$override_path" ]]; then
    [[ -f "$override_path" ]] || fail "--ort-dylib-path not found: $override_path"
    ORT_DYLIB_RESOLVED="$override_path"
    ORT_DYLIB_SOURCE="override"
    return 0
  fi

  if [[ -n "${ORT_DYLIB_PATH:-}" ]]; then
    [[ -f "${ORT_DYLIB_PATH}" ]] || fail "ORT_DYLIB_PATH file not found: ${ORT_DYLIB_PATH}"
    ORT_DYLIB_RESOLVED="${ORT_DYLIB_PATH}"
    ORT_DYLIB_SOURCE="env"
    return 0
  fi

  local repo_cached
  repo_cached="$(resolve_repo_cached_ort_path "$repo_path")"
  if [[ -f "$repo_cached" ]]; then
    ORT_DYLIB_RESOLVED="$repo_cached"
    ORT_DYLIB_SOURCE="repo_cache"
    return 0
  fi

  if [[ -n "${!MOCK_ENV:-}" && "${!MOCK_ENV}" != "0" ]]; then
    ORT_DYLIB_RESOLVED=""
    ORT_DYLIB_SOURCE="mock_skipped"
    return 0
  fi

  ensure_repo_local_ort "$repo_path" "$repo_cached"
  ORT_DYLIB_RESOLVED="$repo_cached"
  ORT_DYLIB_SOURCE="downloaded"
}

run_timed_mode_command() {
  local repo_path="$1"
  local home_dir="$2"
  local disable_ann="$3"
  local daemon_config_path="$4"
  local ort_dylib_path="$5"
  local log_path="$6"
  shift 6
  local -a command=("$@")
  local dyld_fallback
  dyld_fallback="$(resolve_dyld_fallback_path "${DYLD_FALLBACK_LIBRARY_PATH:-}" "${command[0]}")"

  local started ended status
  started="$(now_ms)"

  set +e
  (
    cd "$repo_path"
    env \
      HOME="$home_dir" \
      USERPROFILE="$home_dir" \
      XDG_CONFIG_HOME="$home_dir/xdg-config" \
      XDG_DATA_HOME="$home_dir/xdg-data" \
      XDG_CACHE_HOME="$home_dir/xdg-cache" \
      XDG_STATE_HOME="$home_dir/xdg-state" \
      DYLD_FALLBACK_LIBRARY_PATH="$dyld_fallback" \
      LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-}" \
      ORT_DYLIB_PATH="$ort_dylib_path" \
      BITLOOPS_DAEMON_CONFIG_PATH_OVERRIDE="$daemon_config_path" \
      BITLOOPS_TEST_TTY=0 \
      ACCESSIBLE=1 \
      BITLOOPS_QAT_ACTIVE=1 \
      "$DISABLE_ANN_ENV=$disable_ann" \
      "${command[@]}"
  ) >"$log_path" 2>&1
  status=$?
  set -e

  ended="$(now_ms)"
  LAST_CMD_ELAPSED_MS=$((ended - started))
  LAST_CMD_STATUS=$status
}

ingest_log_has_failure_marker() {
  local log_path="$1"
  grep -Eiq 'failed to load onnx runtime dylib|loading local embedding model|error: loading local embedding model' "$log_path"
}

wait_for_enrichment_idle() {
  local repo_path="$1"
  local home_dir="$2"
  local disable_ann="$3"
  local daemon_config_path="$4"
  local ort_dylib_path="$5"
  local log_path="$6"

  if [[ -n "${!MOCK_ENV:-}" && "${!MOCK_ENV}" != "0" ]]; then
    LAST_ENRICHMENT_WAIT_MS=0
    return 0
  fi

  local started elapsed_secs
  started="$(now_ms)"
  LAST_ENRICHMENT_WAIT_MS=0

  while true; do
    run_timed_mode_command \
      "$repo_path" \
      "$home_dir" \
      "$disable_ann" \
      "$daemon_config_path" \
      "$ort_dylib_path" \
      "$log_path" \
      "$binary_path" daemon status --config "$daemon_config_path"

    if (( LAST_CMD_STATUS != 0 )); then
      return 2
    fi

    local pending running failed
    pending="$(awk -F': *' '/Enrichment pending jobs:/ {print $2; exit}' "$log_path" | tr -d '\r' || true)"
    running="$(awk -F': *' '/Enrichment running jobs:/ {print $2; exit}' "$log_path" | tr -d '\r' || true)"
    failed="$(awk -F': *' '/Enrichment failed jobs:/ {print $2; exit}' "$log_path" | tr -d '\r' || true)"

    is_integer "${pending:-}" || pending="0"
    is_integer "${running:-}" || running="0"
    is_integer "${failed:-}" || failed="0"

    if (( failed > 0 )); then
      LAST_ENRICHMENT_WAIT_MS=$(( $(now_ms) - started ))
      return 3
    fi

    if (( pending == 0 && running == 0 )); then
      LAST_ENRICHMENT_WAIT_MS=$(( $(now_ms) - started ))
      return 0
    fi

    elapsed_secs=$(( ($(now_ms) - started) / 1000 ))
    if (( elapsed_secs >= DEFAULT_ENRICHMENT_IDLE_TIMEOUT_SECS )); then
      LAST_ENRICHMENT_WAIT_MS=$(( $(now_ms) - started ))
      return 1
    fi
    sleep "$DEFAULT_ENRICHMENT_POLL_INTERVAL_SECS"
  done
}

json_clone_row_count_from_file() {
  local json_path="$1"
  perl -MJSON::PP -e '
    local $/;
    my $raw = <>;
    my $decoded = eval { JSON::PP->new->allow_nonref->decode($raw) };
    if ($@) {
      print "INVALID_JSON\n";
      exit 0;
    }
    if (ref($decoded) ne "ARRAY") {
      print "INVALID_SHAPE\n";
      exit 0;
    }

    my $clone_rows = 0;
    foreach my $item (@{$decoded}) {
      if (ref($item) eq "HASH"
          && ref($item->{clones}) eq "HASH"
          && ref($item->{clones}->{edges}) eq "ARRAY") {
        $clone_rows += scalar(@{$item->{clones}->{edges}});
      } else {
        $clone_rows += 1;
      }
    }

    print $clone_rows, "\n";
  ' "$json_path"
}

mock_query_rows() {
  local configured="${!MOCK_QUERY_ROWS_ENV:-2}"
  if ! is_integer "$configured"; then
    echo "2"
    return 0
  fi
  echo "$configured"
}

write_mock_query_log() {
  local log_path="$1"
  local row_count="$2"

  if [[ -n "${!MOCK_INVALID_QUERY_JSON_ENV:-}" && "${!MOCK_INVALID_QUERY_JSON_ENV}" != "0" ]]; then
    printf '{invalid-json]\n' >"$log_path"
    return 0
  fi

  if (( row_count <= 0 )); then
    printf '[]\n' >"$log_path"
    return 0
  fi

  if [[ -n "${!MOCK_QUERY_NESTED_EMPTY_ENV:-}" && "${!MOCK_QUERY_NESTED_EMPTY_ENV}" != "0" ]]; then
    printf '[{"symbolFqn":"mock::source","clones":{"edges":[]}}]\n' >"$log_path"
    return 0
  fi

  local idx
  printf '[{"symbolFqn":"mock::source","clones":{"edges":[' >"$log_path"
  for idx in $(seq 1 "$row_count"); do
    if (( idx > 1 )); then
      printf ',' >>"$log_path"
    fi
    printf '{"node":{"target_symbol_fqn":"mock::target_%d","score":0.9}}' "$idx" >>"$log_path"
  done
  printf ']}}]\n' >>"$log_path"
}

mark_failure() {
  local reason="$1"
  if [[ "$VALID_RUN" == "1" ]]; then
    VALID_RUN="0"
    FAILURE_REASON="$reason"
  fi
}

repo_path=""
source_symbol_fqn=""
iterations="$DEFAULT_ITERATIONS"
warmup="$DEFAULT_WARMUP"
neighbors="$DEFAULT_NEIGHBORS"
repo_name="$DEFAULT_REPO_NAME"
binary="${BITLOOPS_ANN_AB_BINARY:-bitloops}"
ort_dylib_path_arg=""
require_nonempty="$DEFAULT_REQUIRE_NONEMPTY"
bootstrap_sync="$DEFAULT_BOOTSTRAP_SYNC"
output_path=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --repo)
      [[ "$#" -ge 2 ]] || fail "--repo requires a value"
      repo_path="$2"
      shift 2
      ;;
    --symbol-fqn)
      [[ "$#" -ge 2 ]] || fail "--symbol-fqn requires a value"
      source_symbol_fqn="$2"
      shift 2
      ;;
    --iterations)
      [[ "$#" -ge 2 ]] || fail "--iterations requires a value"
      iterations="$2"
      shift 2
      ;;
    --warmup)
      [[ "$#" -ge 2 ]] || fail "--warmup requires a value"
      warmup="$2"
      shift 2
      ;;
    --neighbors)
      [[ "$#" -ge 2 ]] || fail "--neighbors requires a value"
      neighbors="$2"
      shift 2
      ;;
    --repo-name)
      [[ "$#" -ge 2 ]] || fail "--repo-name requires a value"
      repo_name="$2"
      shift 2
      ;;
    --binary)
      [[ "$#" -ge 2 ]] || fail "--binary requires a value"
      binary="$2"
      shift 2
      ;;
    --ort-dylib-path)
      [[ "$#" -ge 2 ]] || fail "--ort-dylib-path requires a value"
      ort_dylib_path_arg="$2"
      shift 2
      ;;
    --require-nonempty)
      require_nonempty="1"
      shift
      ;;
    --allow-empty)
      require_nonempty="0"
      shift
      ;;
    --bootstrap-sync)
      bootstrap_sync="1"
      shift
      ;;
    --skip-bootstrap-sync)
      bootstrap_sync="0"
      shift
      ;;
    --output)
      [[ "$#" -ge 2 ]] || fail "--output requires a value"
      output_path="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

[[ -n "$repo_path" ]] || fail "--repo is required"
[[ -n "$source_symbol_fqn" ]] || fail "--symbol-fqn is required"
[[ -d "$repo_path" ]] || fail "repo path does not exist: $repo_path"
is_integer "$iterations" || fail "--iterations must be an integer"
((iterations > 0)) || fail "--iterations must be > 0"
is_integer "$warmup" || fail "--warmup must be an integer"
((warmup >= 0)) || fail "--warmup must be >= 0"
is_integer "$neighbors" || fail "--neighbors must be an integer"
((neighbors >= MIN_NEIGHBORS)) || neighbors="$MIN_NEIGHBORS"
((neighbors <= MAX_NEIGHBORS)) || neighbors="$MAX_NEIGHBORS"
if [[ "$bootstrap_sync" != "0" && "$bootstrap_sync" != "1" ]]; then
  fail "--bootstrap-sync/--skip-bootstrap-sync produced invalid bootstrap flag"
fi

ensure_binary "$binary"
binary_path="$(resolve_binary_path "$binary")"
resolve_ort_dylib_path "$repo_path" "$ort_dylib_path_arg"

VALID_RUN="1"
FAILURE_REASON=""

timestamp="$(date -u +"%Y%m%dT%H%M%SZ")"
run_id="ann-ab-${timestamp}-$$"
report_root="$repo_path/target/qa"
run_root="$report_root/runs/$run_id"
mkdir -p "$run_root"

if [[ -z "$output_path" ]]; then
  output_path="$report_root/semantic_clones_ann_ab_${run_id}.json"
fi
mkdir -p "$(dirname "$output_path")"

query_dsl="repo(\"$repo_name\")->artefacts(symbol_fqn:\"$source_symbol_fqn\")->clones(neighbors:$neighbors)->limit(20)"

ann_on_ingest_ms=()
ann_on_query_ms=()
ann_off_ingest_ms=()
ann_off_query_ms=()
ann_on_row_counts=()
ann_off_row_counts=()

running_mode_homes=()
running_mode_disable_ann=()
running_mode_daemon_cfg=()
running_mode_ort=()

cleanup_mode_daemons() {
  local index
  for index in "${!running_mode_homes[@]}"; do
    run_timed_mode_command \
      "$repo_path" \
      "${running_mode_homes[$index]}" \
      "${running_mode_disable_ann[$index]}" \
      "${running_mode_daemon_cfg[$index]}" \
      "${running_mode_ort[$index]}" \
      /dev/null \
      "$binary_path" daemon stop --config "${running_mode_daemon_cfg[$index]}" >/dev/null 2>&1 || true
  done
}

trap cleanup_mode_daemons EXIT

for mode in ann_on ann_off; do
  disable_ann="0"
  if [[ "$mode" == "ann_off" ]]; then
    disable_ann="1"
  fi
  mode_failed="0"

  mode_root="$run_root/$mode"
  mode_home="$mode_root/home"
  mode_logs="$mode_root/logs"
  mode_state="$mode_root/state"
  mode_config_dir="$mode_home/xdg-config/bitloops"
  mode_config_path="$mode_config_dir/config.toml"
  sqlite_path="$mode_state/stores/relational/relational.db"
  duckdb_path="$mode_state/stores/event/events.duckdb"
  blob_path="$mode_state/stores/blob"

  mkdir -p \
    "$mode_home/xdg-config" \
    "$mode_home/xdg-data" \
    "$mode_home/xdg-cache" \
    "$mode_home/xdg-state" \
    "$mode_config_dir" \
    "$(dirname "$sqlite_path")" \
    "$(dirname "$duckdb_path")" \
    "$blob_path" \
    "$mode_logs"

  cat >"$mode_config_path" <<CFG
[runtime]
local_dev = false

[stores.relational]
sqlite_path = "$sqlite_path"

[stores.events]
duckdb_path = "$duckdb_path"

[stores.blob]
local_path = "$blob_path"

[semantic_clones]
summary_mode = "off"
embedding_mode = "deterministic"

[semantic_clones.inference]
code_embeddings = "local"
summary_embeddings = "local"

[inference.runtimes.bitloops_embeddings]
command = "bitloops-embeddings"
args = []
startup_timeout_secs = ${DEFAULT_EMBEDDINGS_STARTUP_TIMEOUT_SECS}
request_timeout_secs = ${DEFAULT_EMBEDDINGS_REQUEST_TIMEOUT_SECS}

[inference.profiles.local]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_embeddings"
model = "${DEFAULT_LOCAL_EMBEDDING_MODEL}"
cache_dir = "$mode_state/embeddings/models"
CFG

  daemon_start_log="$mode_logs/daemon_start.log"
  daemon_stop_log="$mode_logs/daemon_stop.log"
  init_log="$mode_logs/devql_init.log"
  sync_log="$mode_logs/devql_sync.log"
  sync_status_log="$mode_logs/devql_sync_status.log"
  enrichment_wait_log="$mode_logs/enrichment_wait.log"

  if [[ -n "${!MOCK_ENV:-}" && "${!MOCK_ENV}" != "0" ]]; then
    printf 'mock mode: skipped daemon start and devql init\n' >"$daemon_start_log"
    printf 'mock mode: skipped devql init\n' >"$init_log"
  else
    daemon_port="$(reserve_local_port)"

    run_timed_mode_command \
      "$repo_path" \
      "$mode_home" \
      "$disable_ann" \
      "$mode_config_path" \
      "$ORT_DYLIB_RESOLVED" \
      "$daemon_start_log" \
      "$binary_path" daemon start --config "$mode_config_path" --no-telemetry --http --host 127.0.0.1 --port "$daemon_port" -d

    if (( LAST_CMD_STATUS != 0 )); then
      mark_failure "${mode}: daemon start failed"
      mode_failed="1"
    fi

    if (( mode_failed == 0 )); then
      running_mode_homes+=("$mode_home")
      running_mode_disable_ann+=("$disable_ann")
      running_mode_daemon_cfg+=("$mode_config_path")
      running_mode_ort+=("$ORT_DYLIB_RESOLVED")
    fi

    if (( mode_failed == 0 )); then
      run_timed_mode_command \
        "$repo_path" \
        "$mode_home" \
        "$disable_ann" \
        "$mode_config_path" \
        "$ORT_DYLIB_RESOLVED" \
        "$init_log" \
        "$binary_path" devql init

      if (( LAST_CMD_STATUS != 0 )); then
        mark_failure "${mode}: devql init failed"
        mode_failed="1"
      fi
    fi

    if (( mode_failed == 0 && bootstrap_sync == 1 )); then
      run_timed_mode_command \
        "$repo_path" \
        "$mode_home" \
        "$disable_ann" \
        "$mode_config_path" \
        "$ORT_DYLIB_RESOLVED" \
        "$sync_log" \
        "$binary_path" devql sync
      if (( LAST_CMD_STATUS != 0 )); then
        mark_failure "${mode}: devql sync bootstrap failed"
        mode_failed="1"
      fi

      if (( mode_failed == 0 )); then
        run_timed_mode_command \
          "$repo_path" \
          "$mode_home" \
          "$disable_ann" \
          "$mode_config_path" \
          "$ORT_DYLIB_RESOLVED" \
          "$sync_status_log" \
          "$binary_path" devql sync --status
        if (( LAST_CMD_STATUS != 0 )); then
          mark_failure "${mode}: devql sync --status bootstrap failed"
          mode_failed="1"
        fi
      fi
    elif (( mode_failed == 0 )); then
      printf 'sync bootstrap skipped by flag\n' >"$sync_log"
      printf 'sync --status bootstrap skipped by flag\n' >"$sync_status_log"
    fi
  fi

  if (( mode_failed == 0 )); then
    total_runs=$((warmup + iterations))
    for run_idx in $(seq 1 "$total_runs"); do

      phase="measure"
      phase_idx="$run_idx"
      if (( run_idx <= warmup )); then
        phase="warmup"
        phase_idx="$run_idx"
      else
        phase_idx=$((run_idx - warmup))
      fi

      ingest_log="$mode_logs/${phase}-ingest-${phase_idx}.log"
      query_log="$mode_logs/${phase}-query-${phase_idx}.log"

    if [[ -n "${!MOCK_ENV:-}" && "${!MOCK_ENV}" != "0" ]]; then
      ingest_ms="$(mock_duration_ms "$mode" "ingest" "$run_idx")"
      query_ms="$(mock_duration_ms "$mode" "query" "$run_idx")"

      if [[ -n "${!MOCK_INGEST_ERROR_ENV:-}" && "${!MOCK_INGEST_ERROR_ENV}" != "0" ]]; then
        printf 'Error: loading local embedding model mock failure\n' >"$ingest_log"
      else
        printf 'DevQL ingest complete (mock)\n' >"$ingest_log"
      fi

      row_count="$(mock_query_rows)"
      write_mock_query_log "$query_log" "$row_count"
    else
      run_timed_mode_command \
        "$repo_path" \
        "$mode_home" \
        "$disable_ann" \
        "$mode_config_path" \
        "$ORT_DYLIB_RESOLVED" \
        "$ingest_log" \
        "$binary_path" devql ingest
      ingest_ms="$LAST_CMD_ELAPSED_MS"

      if (( LAST_CMD_STATUS != 0 )); then
        mark_failure "${mode}: devql ingest failed during ${phase} ${phase_idx}"
        break
      fi

      if wait_for_enrichment_idle \
        "$repo_path" \
        "$mode_home" \
        "$disable_ann" \
        "$mode_config_path" \
        "$ORT_DYLIB_RESOLVED" \
        "$enrichment_wait_log"; then
        ingest_ms=$((ingest_ms + LAST_ENRICHMENT_WAIT_MS))
      else
        wait_status=$?
        case "$wait_status" in
          1) mark_failure "${mode}: enrichment queue did not drain within timeout after ${phase} ingest ${phase_idx}" ;;
          2) mark_failure "${mode}: failed to read daemon status while waiting for enrichment after ${phase} ingest ${phase_idx}" ;;
          3) mark_failure "${mode}: enrichment reported failed jobs after ${phase} ingest ${phase_idx}" ;;
          *) mark_failure "${mode}: unknown enrichment wait failure after ${phase} ingest ${phase_idx}" ;;
        esac
        break
      fi

      run_timed_mode_command \
        "$repo_path" \
        "$mode_home" \
        "$disable_ann" \
        "$mode_config_path" \
        "$ORT_DYLIB_RESOLVED" \
        "$query_log" \
        "$binary_path" devql query "$query_dsl" --compact
      query_ms="$LAST_CMD_ELAPSED_MS"

      if (( LAST_CMD_STATUS != 0 )); then
        mark_failure "${mode}: devql query failed during ${phase} ${phase_idx}"
        break
      fi
    fi

      if ingest_log_has_failure_marker "$ingest_log"; then
        mark_failure "${mode}: embedding runtime failure marker detected in ${phase} ingest ${phase_idx}"
        break
      fi

      parsed_clone_count="$(json_clone_row_count_from_file "$query_log")"
      if [[ "$parsed_clone_count" == "INVALID_JSON" ]]; then
        mark_failure "${mode}: query output was invalid JSON during ${phase} ${phase_idx}"
        break
      fi
      if [[ "$parsed_clone_count" == "INVALID_SHAPE" ]]; then
        mark_failure "${mode}: query output was not a JSON array during ${phase} ${phase_idx}"
        break
      fi
      if ! is_integer "$parsed_clone_count"; then
        mark_failure "${mode}: query clone row count parsing failed during ${phase} ${phase_idx}"
        break
      fi

      if [[ "$phase" == "measure" ]]; then
        if [[ "$mode" == "ann_on" ]]; then
          ann_on_ingest_ms+=("$ingest_ms")
          ann_on_query_ms+=("$query_ms")
          ann_on_row_counts+=("$parsed_clone_count")
        else
          ann_off_ingest_ms+=("$ingest_ms")
          ann_off_query_ms+=("$query_ms")
          ann_off_row_counts+=("$parsed_clone_count")
        fi
      fi

      if (( require_nonempty == 1 && parsed_clone_count == 0 )); then
        mark_failure "${mode}: query returned zero clone rows during ${phase} ${phase_idx}"
        break
      fi
    done
  fi

  if [[ -z "${!MOCK_ENV:-}" || "${!MOCK_ENV}" == "0" ]]; then
    run_timed_mode_command \
      "$repo_path" \
      "$mode_home" \
      "$disable_ann" \
      "$mode_config_path" \
      "$ORT_DYLIB_RESOLVED" \
      "$daemon_stop_log" \
      "$binary_path" daemon stop --config "$mode_config_path" >/dev/null || true
  else
    printf 'mock mode: skipped daemon stop\n' >"$daemon_stop_log"
  fi
done

read -r ann_on_ingest_median ann_on_ingest_p95 <<<"$(calc_stats "${ann_on_ingest_ms[@]}")"
read -r ann_off_ingest_median ann_off_ingest_p95 <<<"$(calc_stats "${ann_off_ingest_ms[@]}")"
read -r ann_on_query_median ann_on_query_p95 <<<"$(calc_stats "${ann_on_query_ms[@]}")"
read -r ann_off_query_median ann_off_query_p95 <<<"$(calc_stats "${ann_off_query_ms[@]}")"
read -r ann_on_row_median _ <<<"$(calc_stats "${ann_on_row_counts[@]}")"
read -r ann_off_row_median _ <<<"$(calc_stats "${ann_off_row_counts[@]}")"

if [[ "$VALID_RUN" == "1" && "$require_nonempty" == "1" ]]; then
  if (( ann_on_row_median == 0 || ann_off_row_median == 0 )); then
    mark_failure "non-empty mode enabled but measured row count was zero"
  elif (( ann_on_row_median != ann_off_row_median )); then
    mark_failure "row-count parity check failed (ann_on=${ann_on_row_median}, ann_off=${ann_off_row_median})"
  fi
fi

ingest_speedup_json="null"
query_speedup_json="null"
if [[ "$VALID_RUN" == "1" ]]; then
  ingest_speedup_json="$(speedup_percent "$ann_off_ingest_median" "$ann_on_ingest_median")"
  query_speedup_json="$(speedup_percent "$ann_off_query_median" "$ann_on_query_median")"
fi

repo_path_json="$(json_escape "$repo_path")"
symbol_fqn_json="$(json_escape "$source_symbol_fqn")"
binary_json="$(json_escape "$binary_path")"
repo_name_json="$(json_escape "$repo_name")"
query_dsl_json="$(json_escape "$query_dsl")"
ort_source_json="$(json_escape "$ORT_DYLIB_SOURCE")"
ort_path_json="$(json_escape "$ORT_DYLIB_RESOLVED")"

failure_reason_json="null"
if [[ "$VALID_RUN" != "1" ]]; then
  failure_reason_json="\"$(json_escape "$FAILURE_REASON")\""
fi

cat >"$output_path" <<JSON
{
  "run_id": "$(json_escape "$run_id")",
  "generated_at_utc": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")",
  "valid_run": $([[ "$VALID_RUN" == "1" ]] && echo "true" || echo "false"),
  "failure_reason": ${failure_reason_json},
  "ann_on_row_count": ${ann_on_row_median},
  "ann_off_row_count": ${ann_off_row_median},
  "config": {
    "repo_path": "$repo_path_json",
    "repo_name": "$repo_name_json",
    "source_symbol_fqn": "$symbol_fqn_json",
    "binary": "$binary_json",
    "iterations": $iterations,
    "warmup": $warmup,
    "neighbors": $neighbors,
    "query_dsl": "$query_dsl_json",
    "require_nonempty": $([[ "$require_nonempty" == "1" ]] && echo "true" || echo "false"),
    "bootstrap_sync": $([[ "$bootstrap_sync" == "1" ]] && echo "true" || echo "false"),
    "ort_version": "${ORT_VERSION}",
    "ort_dylib_path": "$ort_path_json",
    "ort_source": "$ort_source_json",
    "mock_mode": $([[ -n "${!MOCK_ENV:-}" && "${!MOCK_ENV}" != "0" ]] && echo "true" || echo "false")
  },
  "metrics": {
    "ingest_ms": {
      "ann_on": {
        "samples": $(to_json_array "${ann_on_ingest_ms[@]}"),
        "median": $ann_on_ingest_median,
        "p95": $ann_on_ingest_p95
      },
      "ann_off": {
        "samples": $(to_json_array "${ann_off_ingest_ms[@]}"),
        "median": $ann_off_ingest_median,
        "p95": $ann_off_ingest_p95
      },
      "speedup_percent_ann_on_vs_ann_off": $ingest_speedup_json
    },
    "query_ms": {
      "ann_on": {
        "samples": $(to_json_array "${ann_on_query_ms[@]}"),
        "median": $ann_on_query_median,
        "p95": $ann_on_query_p95
      },
      "ann_off": {
        "samples": $(to_json_array "${ann_off_query_ms[@]}"),
        "median": $ann_off_query_median,
        "p95": $ann_off_query_p95
      },
      "speedup_percent_ann_on_vs_ann_off": $query_speedup_json
    },
    "query_row_counts": {
      "ann_on_samples": $(to_json_array "${ann_on_row_counts[@]}"),
      "ann_off_samples": $(to_json_array "${ann_off_row_counts[@]}")
    }
  }
}
JSON

printf '\nStage 3 ANN A/B (report-only)\n'
printf '%-26s %-14s %-14s %-14s\n' "Metric" "ANN ON median" "ANN OFF median" "Speedup %"
printf '%-26s %-14s %-14s %-14s\n' "ingest_ms" "$ann_on_ingest_median" "$ann_off_ingest_median" "$ingest_speedup_json"
printf '%-26s %-14s %-14s %-14s\n' "query_ms" "$ann_on_query_median" "$ann_off_query_median" "$query_speedup_json"
printf '\n'
printf 'ANN ON p95: ingest=%s ms, query=%s ms\n' "$ann_on_ingest_p95" "$ann_on_query_p95"
printf 'ANN OFF p95: ingest=%s ms, query=%s ms\n' "$ann_off_ingest_p95" "$ann_off_query_p95"
printf 'Row parity (median): ann_on=%s ann_off=%s\n' "$ann_on_row_median" "$ann_off_row_median"
printf 'Run valid: %s\n' "$([[ "$VALID_RUN" == "1" ]] && echo "true" || echo "false")"
if [[ "$VALID_RUN" != "1" ]]; then
  printf 'Failure reason: %s\n' "$FAILURE_REASON"
fi
printf 'Run logs: %s\n' "$run_root"
printf 'Report JSON: %s\n' "$output_path"

if [[ "$VALID_RUN" != "1" ]]; then
  exit 1
fi
