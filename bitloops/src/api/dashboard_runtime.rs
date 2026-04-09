use super::{
    DashboardReadyInfo, DashboardRuntimeOptions, DashboardServerConfig, DashboardStartupMode,
    DashboardState, DashboardTransport, FALLBACK_LOCAL_HOST, LocalDashboardDiscovery, ServeMode,
    db, router, tls,
};
use crate::config::{persist_dashboard_tls_hint, resolve_dashboard_config_for_repo};
use crate::graphql;
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, bitloops_wordmark, color_hex};
use crate::utils::paths;
use anyhow::{Context, Result, anyhow, bail};
use std::env;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::dashboard_paths::{
    browser_host_for_url, format_dashboard_url, has_bundle_index, normalized_host,
    resolve_bind_addr, resolve_bundle_dir,
};

fn is_dev_mode() -> bool {
    env::var("BITLOOPS_DEV").is_ok()
}

pub(super) async fn run(
    config: DashboardServerConfig,
    options: DashboardRuntimeOptions,
) -> Result<()> {
    let mut startup_warnings: Vec<String> = Vec::new();
    let mut discovery = LocalDashboardDiscovery::default();
    let config_root = options
        .config_root
        .clone()
        .or_else(|| paths::repo_root().ok())
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let dashboard_cfg = resolve_dashboard_config_for_repo(&config_root);
    let local_dashboard_cfg = dashboard_cfg.local_dashboard.as_ref();
    let explicit_host = config
        .host
        .as_deref()
        .and_then(normalized_host)
        .map(str::to_string);
    let startup_mode = select_startup_mode(&config, local_dashboard_cfg, explicit_host.as_deref())?;
    log::info!("dashboard startup phase: initialising database pools");
    let db_init = db::init_dashboard_db().await;
    if db_init.startup_health.has_failures() {
        bail!(
            "dashboard database startup health check failed; run `bitloops --connection-status` for details"
        );
    }

    let selected_host = match startup_mode {
        DashboardStartupMode::FastHttpLoopback => FALLBACK_LOCAL_HOST.to_string(),
        DashboardStartupMode::FastConfiguredHttps | DashboardStartupMode::SlowProbe => {
            explicit_host
                .clone()
                .unwrap_or_else(|| FALLBACK_LOCAL_HOST.to_string())
        }
    };

    let bind_addr = resolve_bind_addr(&selected_host, config.port)?;

    let listener = TcpListener::bind(bind_addr).await.with_context(|| {
        format!(
            "Binding dashboard server to {selected_host}:{}",
            config.port
        )
    })?;
    let local_addr = listener
        .local_addr()
        .context("Reading dashboard listener address")?;
    let browser_host = browser_host_for_url(&selected_host, local_addr);
    let (transport, tls_acceptor) = match startup_mode {
        DashboardStartupMode::FastHttpLoopback => (DashboardTransport::Http, None),
        DashboardStartupMode::FastConfiguredHttps => {
            let tls_material = tls::load_existing_dashboard_tls_material(&browser_host)
                .with_context(|| {
                    format!(
                        "dashboard fast TLS path failed for host {browser_host}; \
                         run `bitloops daemon start --recheck-local-dashboard-net` once"
                    )
                })?;
            log::debug!(
                "Dashboard TLS (fast path): cert={} key={}",
                tls_material.cert_path.display(),
                tls_material.key_path.display()
            );
            (
                DashboardTransport::Https,
                Some(TlsAcceptor::from(tls_material.server_config.clone())),
            )
        }
        DashboardStartupMode::SlowProbe => {
            if !tls::mkcert_on_path() {
                startup_warnings.push(
                    "Warning: `mkcert` is not on PATH. Falling back to local HTTP.\n\
                     See https://bitloops.com/docs/guides/dashboard-local-https-setup for local TLS setup instructions."
                        .to_string(),
                );
                (DashboardTransport::Http, None)
            } else {
                match tls::ensure_dashboard_tls_material(&browser_host) {
                    Ok(tls_material) => {
                        discovery.tls = true;
                        log::debug!(
                            "Dashboard TLS: cert={} key={}",
                            tls_material.cert_path.display(),
                            tls_material.key_path.display()
                        );
                        (
                            DashboardTransport::Https,
                            Some(TlsAcceptor::from(tls_material.server_config.clone())),
                        )
                    }
                    Err(err) => {
                        startup_warnings.push(format!(
                            "Warning: Dashboard HTTPS setup failed ({err:#}).\n\
                             Falling back to local HTTP.\n\
                             See https://bitloops.com/docs/guides/dashboard-local-https-setup for local TLS setup instructions."
                        ));
                        (DashboardTransport::Http, None)
                    }
                }
            }
        }
    };

    let bundle_dir = resolve_bundle_dir(
        config.bundle_dir.as_deref(),
        dashboard_cfg.bundle_dir.as_deref(),
    );
    let serve_mode = if has_bundle_index(&bundle_dir) {
        ServeMode::Bundle(bundle_dir.clone())
    } else {
        ServeMode::HelloWorld
    };
    let repo_root = config_root.clone();

    if options.bootstrap_devql_schema
        // Bootstrap DevQL schema on startup (idempotent).
        && let Ok(repo_identity) = crate::host::devql::resolve_repo_identity(&repo_root)
        && let Err(err) = crate::host::devql::ensure_relational_and_events_schema(
            &config_root,
            &repo_root,
            repo_identity,
        )
        .await
    {
        log::warn!("DevQL schema bootstrap on startup failed: {err:#}");
        startup_warnings.push(format!("Warning: DevQL schema bootstrap failed: {err:#}"));
    }

    if matches!(startup_mode, DashboardStartupMode::SlowProbe)
        && discovery.tls
        && let Err(err) = persist_local_dashboard_discovery(&repo_root, discovery)
    {
        startup_warnings.push(format!(
            "Warning: failed to persist local dashboard network hints: {err:#}"
        ));
    }

    let url = format_dashboard_url(transport, &browser_host, local_addr.port());
    let devql_schema = graphql::build_global_schema_template();
    let devql_slim_schema = graphql::build_slim_schema_template();

    if options.print_ready_banner {
        println!();
        println!("{}", color_hex(&bitloops_wordmark(), BITLOOPS_PURPLE_HEX));
        println!();
        print!("📊 {} ", options.ready_subject);
        print!("{}", color_hex("ready ", "#22c55e"));
        print!("at ");
        println!("{}", color_hex(&clickable_url(&url), BITLOOPS_PURPLE_HEX));
        if !startup_warnings.is_empty() {
            eprintln!();
        }
    }
    for warning in &startup_warnings {
        print_warning_message(warning);
    }
    if options.print_ready_banner {
        match &serve_mode {
            ServeMode::HelloWorld => {
                println!(
                    "Bundle not found. Bundle expected at {}",
                    bundle_dir.display()
                );
            }
            ServeMode::Bundle(path) => {
                log::debug!("Serving dashboard bundle from {}", path.display());
                if is_dev_mode() {
                    println!("Serving dashboard bundle from {}", path.display());
                }
                println!();
                println!("To exit, press Ctrl+C");
            }
        }
    }

    let ready_info = DashboardReadyInfo {
        url: url.clone(),
        host: browser_host.clone(),
        port: local_addr.port(),
        bundle_dir: bundle_dir.clone(),
        repo_root: repo_root.clone(),
    };

    if let Some(on_ready) = options.on_ready.as_ref() {
        on_ready(&ready_info)?;
    }

    if options.open_browser
        && !config.no_open
        && let Err(err) = open_in_default_browser(&url)
    {
        print_warning_message(&format!("Warning: failed to open default browser: {err:#}"));
    }

    let state = DashboardState {
        config_root: config_root.clone(),
        repo_root,
        repo_registry_path: options.repo_registry_path.clone(),
        mode: serve_mode,
        db: db_init.pools,
        bundle_dir,
        bundle_source_overrides: super::DashboardBundleSourceOverrides::default(),
        subscription_hub: graphql::SubscriptionHub::new_arc(),
        devql_schema,
        devql_slim_schema,
    };
    crate::daemon::activate_sync_worker(state.subscription_hub());

    match (transport, tls_acceptor) {
        (DashboardTransport::Https, Some(acceptor)) => {
            serve_until_shutdown_tls(listener, acceptor, state, options.shutdown_message).await
        }
        (DashboardTransport::Http, _) => {
            serve_until_shutdown_http(listener, state, options.shutdown_message).await
        }
        (DashboardTransport::Https, None) => {
            Err(anyhow!("dashboard HTTPS selected without a TLS acceptor"))
        }
    }?;

    if let Some(on_shutdown) = options.on_shutdown.as_ref() {
        on_shutdown();
    }

    Ok(())
}

pub(super) fn select_startup_mode(
    config: &DashboardServerConfig,
    local_dashboard_cfg: Option<&crate::config::DashboardLocalDashboardConfig>,
    explicit_host: Option<&str>,
) -> Result<DashboardStartupMode> {
    if config.force_http {
        if explicit_host != Some(FALLBACK_LOCAL_HOST) {
            bail!("fast HTTP mode requires both `--http` and `--host {FALLBACK_LOCAL_HOST}`");
        }
        return Ok(DashboardStartupMode::FastHttpLoopback);
    }

    if !config.recheck_local_dashboard_net
        && local_dashboard_cfg.and_then(|cfg| cfg.tls) == Some(true)
    {
        return Ok(DashboardStartupMode::FastConfiguredHttps);
    }

    Ok(DashboardStartupMode::SlowProbe)
}

fn persist_local_dashboard_discovery(
    repo_root: &Path,
    discovery: LocalDashboardDiscovery,
) -> Result<()> {
    let _ = repo_root;
    if !discovery.tls {
        return Ok(());
    }
    persist_dashboard_tls_hint(true).map(|_| ())
}

async fn serve_until_shutdown_tls(
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
    state: DashboardState,
    shutdown_message: Option<String>,
) -> Result<()> {
    use hyper::server::conn::http1::Builder as Http1Builder;
    use hyper_util::rt::TokioIo;
    use hyper_util::service::TowerToHyperService;

    let app = router::build_dashboard_router(state);
    serve_until_shutdown_with_handler(listener, shutdown_message, move |stream| {
        let tls_acceptor = tls_acceptor.clone();
        let app = app.clone();
        async move {
            let tls_stream = match tls_acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => {
                    let detail = e.to_string();
                    if detail.contains("CertificateUnknown") || detail.contains("UnknownIssuer") {
                        log::warn!(
                            "TLS handshake failed: {e} \
                             (the client rejected the certificate — run `mkcert -install` once, quit Chrome fully, retry)"
                        );
                    } else {
                        log::warn!("TLS handshake failed: {e}");
                    }
                    return;
                }
            };
            let io = TokioIo::new(tls_stream);
            let service = TowerToHyperService::new(app);
            if let Err(e) = Http1Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                log::warn!("HTTP connection error: {e}");
            }
        }
    })
    .await
}

async fn serve_until_shutdown_http(
    listener: TcpListener,
    state: DashboardState,
    shutdown_message: Option<String>,
) -> Result<()> {
    use hyper::server::conn::http1::Builder as Http1Builder;
    use hyper_util::rt::TokioIo;
    use hyper_util::service::TowerToHyperService;

    let app = router::build_dashboard_router(state);
    serve_until_shutdown_with_handler(listener, shutdown_message, move |stream| {
        let app = app.clone();
        async move {
            let io = TokioIo::new(stream);
            let service = TowerToHyperService::new(app);
            if let Err(e) = Http1Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                log::warn!("HTTP connection error: {e}");
            }
        }
    })
    .await
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        let ctrl_c = async {
            if tokio::signal::ctrl_c().await.is_err() {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            _ = ctrl_c => {}
            _ = async {
                if let Some(sigterm) = sigterm.as_mut() {
                    sigterm.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }

    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn serve_until_shutdown_with_handler<F, Fut>(
    listener: TcpListener,
    shutdown_message: Option<String>,
    mut on_stream: F,
) -> Result<()>
where
    F: FnMut(tokio::net::TcpStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let shutdown = wait_for_shutdown_signal();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                break;
            }
            accept = listener.accept() => {
                let (stream, _) = accept.context("accepting dashboard TCP connection")?;
                tokio::spawn(on_stream(stream));
            }
        }
    }

    drop(listener);
    tokio::time::sleep(Duration::from_secs(5)).await;
    if let Some(message) = shutdown_message {
        println!("{message}");
    }
    Ok(())
}

fn clickable_url(url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
}

fn terminal_supports_color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

pub(super) fn warning_block_lines(warning: &str, use_color: bool) -> Vec<String> {
    let lines: Vec<&str> = warning.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    // Brown background close to terminal selection tone in user screenshot.
    const WARN_STYLE: &str = "30;48;2;107;79;59";
    let max_content_width = lines.iter().map(|line| line.len()).max().unwrap_or(0);
    let total_width = max_content_width + 4; // two spaces left + two spaces right
    let blank = " ".repeat(total_width);

    if use_color {
        let mut out = Vec::with_capacity(lines.len() + 3);
        out.push(format!("\x1b[{WARN_STYLE}m{blank}\x1b[K\x1b[0m"));
        let icon_line = format!("  \x1b[33m⚠\x1b[{WARN_STYLE}m  ");
        out.push(format!("\x1b[{WARN_STYLE}m{icon_line}\x1b[K\x1b[0m"));
        for line in &lines {
            let content = format!("  {line:<width$}  ", width = max_content_width);
            out.push(format!("\x1b[{WARN_STYLE}m{content}\x1b[K\x1b[0m"));
        }
        out.push(format!("\x1b[{WARN_STYLE}m{blank}\x1b[K\x1b[0m"));
        return out;
    }

    let mut out = Vec::with_capacity(lines.len() + 3);
    out.push(String::new());
    out.push("  ⚠".to_string());
    for line in &lines {
        out.push(format!("  {line}"));
    }
    out.push(String::new());
    out
}

fn print_warning_message(warning: &str) {
    for line in warning_block_lines(warning, terminal_supports_color()) {
        eprintln!("{line}");
    }
}

pub(super) fn open_in_default_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command.arg(url);
        command
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        return Err(anyhow!(
            "opening the browser is not supported on this platform"
        ));
    }

    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Running browser opener for {url}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod dashboard_runtime_unit_tests {
    use super::{clickable_url, warning_block_lines};

    #[test]
    fn clickable_url_inserts_osc8_hyperlink_sequence() {
        let url = "https://example.test/path";
        let rendered = clickable_url(url);
        assert!(rendered.contains(url));
        assert!(rendered.contains("\x1b]8;;"));
    }

    #[test]
    fn warning_block_lines_returns_nothing_for_empty_warning() {
        assert!(warning_block_lines("", false).is_empty());
    }

    #[test]
    fn warning_block_lines_plain_mode_includes_message_lines() {
        let lines = warning_block_lines("line one\nline two", false);
        assert!(lines.iter().any(|l| l.contains('⚠')));
        assert!(lines.iter().any(|l| l.contains("line one")));
        assert!(lines.iter().any(|l| l.contains("line two")));
    }
}
