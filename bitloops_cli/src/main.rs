use clap::Parser;

pub use bitloops_cli::engine;
mod commands;
mod devql_config;
mod server;
mod terminal;

#[cfg(test)]
pub(crate) mod test_support;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cmd = commands::Cli::parse();

    tokio::select! {
        result = commands::run(cmd) => {
            if let Err(e) = result {
                // SilentError: command already printed the error, skip duplicate output
                if e.downcast_ref::<commands::SilentError>().is_none() {
                    eprintln!("Error: {e:#}");
                }
                std::process::exit(1);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            // Interrupted — exit cleanly
        }
    }
}
