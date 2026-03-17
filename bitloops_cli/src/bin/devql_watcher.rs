fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if let Err(err) = bitloops_cli::engine::devql::watch::run_process_from_cli() {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    }
}
