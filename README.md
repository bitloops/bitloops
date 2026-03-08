# Bitloops CLI

## Quick Start

```bash
cd bitloops_cli
cargo run -- status      # run in dev
cargo build              # compile → target/debug/bitloops_cli
cargo build --release    # compile → target/release/bitloops_cli
cargo check              # type-check only (fast, like tsc --noEmit)
cargo clippy             # lint
cargo fmt                # format
```

## DevQL MVP

The CLI now includes an MVP DevQL ingestion/query flow:

```bash
# 1) Configure backends (persisted config)
mkdir -p ~/.bitloops
cat > ~/.bitloops/config.json <<'JSON'
{
  "devql": {
    "postgres_dsn": "postgres://user:pass@localhost:5432/bitloops",
    "clickhouse_url": "http://localhost:8123",
    "clickhouse_database": "default",
    "clickhouse_user": "default",
    "clickhouse_password": ""
  }
}
JSON

# Optional: environment variables override ~/.bitloops/config.json
# export BITLOOPS_DEVQL_PG_DSN='postgres://user:pass@localhost:5432/bitloops'
# export BITLOOPS_DEVQL_CH_URL='http://localhost:8123'
# export BITLOOPS_DEVQL_CH_DATABASE='default'
# export BITLOOPS_DEVQL_CH_USER='default'
# export BITLOOPS_DEVQL_CH_PASSWORD='...'

# 2) Verify backend connectivity
cargo run -- --connection-status
# equivalent:
cargo run -- devql connection-status
# outputs a DB Status table with statuses:
# Connected / Could not authenticate / Could not reach DB / Not configured

# 3) Create schema
cargo run -- devql init

# 4) Backfill checkpoints/events + file artefacts
cargo run -- devql ingest

# 5) Query with DevQL (MVP pipeline syntax)
cargo run -- devql query 'repo("bitloops-cli")->checkpoints()->limit(20)'
cargo run -- devql query 'repo("bitloops-cli")->asOf(ref:"main")->file("bitloops_cli/src/main.rs")->artefacts()->limit(20)'

# 6) Start dashboard (runs DB startup health checks and keeps pooled connections)
cargo run -- dashboard
# startup uses the same DB Status table as --connection-status
# live DB health endpoint:
# GET http://127.0.0.1:5667/api/db/health
```

## Parity Tracking

- Root command parity matrix: [docs/root-command-parity-matrix.md](docs/root-command-parity-matrix.md)

## E2E Test Suites

```bash
# Claude-focused E2E scenarios
cargo test --quiet --test e2e_scenario_groups -- --test-threads=1

# Cursor-focused E2E scenarios
cargo test --quiet --test cursor_e2e_scenarios -- --test-threads=1
```

## Hello World Example

The minimal subcommand pattern — how `status` works end to end:

**`src/commands/status.rs`**

```rust
use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub struct StatusArgs {
    #[arg(long, help = "Show verbose output")]
    pub verbose: bool,
}

pub async fn run(args: StatusArgs) -> Result<()> {
    if args.verbose {
        println!("Status: OK (verbose mode)");
    } else {
        println!("Status: OK");
    }
    Ok(())
}
```

**`src/commands/mod.rs`** — wire it in:

```rust
pub mod status;                          // 1. declare module

#[derive(Subcommand)]
pub enum Commands {
    Status(status::StatusArgs),          // 2. add variant
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Status(args) => status::run(args).await,   // 3. dispatch
    }
}
```

Run it:

```bash
cargo run -- status
cargo run -- status --verbose
cargo run -- --help
```

## Project Structure

```
bitloops_cli/
├── Cargo.toml           → package.json
├── Cargo.lock           → package-lock.json
└── src/
    ├── lib.rs           → library boundary exporting `engine`
    ├── main.rs          → binary entrypoint: parse args, dispatch commands/server
    ├── commands/
    │   ├── mod.rs       → Cli struct, Commands enum, run() — like commands/index.ts
    │   └── status.rs    → one file per subcommand — like commands/status.ts
    ├── server/
    │   └── dashboard/   → HTTP API surface
    └── engine/          → shared runtime/domain logic
```

### Adding a new subcommand

1. Create `src/commands/new_cmd.rs`:

   ```rust
   use anyhow::Result;
   use clap::Args;

   #[derive(Args)]
   pub struct NewCmdArgs {}

   pub async fn run(_args: NewCmdArgs) -> Result<()> {
       println!("new-cmd!");
       Ok(())
   }
   ```

2. In `src/commands/mod.rs` add:
   ```rust
   pub mod new_cmd;
   // in Commands enum:
   NewCmd(new_cmd::NewCmdArgs),
   // in run() match:
   Commands::NewCmd(args) => new_cmd::run(args).await,
   ```
