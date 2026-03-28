//! Local HTTPS: mkcert leaf certs plus a shared `rustls::ServerConfig` for the dashboard.

use anyhow::{Context, Result, anyhow, bail};
use rustls::ServerConfig;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::CertificateDer;
use std::fs::{self, File};
use std::io::BufReader;
use std::net::{IpAddr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::OnceLock;
use x509_parser::extensions::GeneralName;
use x509_parser::parse_x509_certificate;

fn eprint_mkcert_not_found_on_path() {
    eprintln!();
    eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    eprintln!("  Dashboard HTTPS — mkcert not found");
    eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    eprintln!();
    eprintln!("  Step 1: install mkcert and put it on your PATH.");
    eprintln!("  Step 2: run `mkcert -install`.");
    eprintln!();
    eprintln!("  macOS:  brew install mkcert nss");
    eprintln!("  Then:   mkcert -install");
    eprintln!();
    eprintln!("  {}", mkcert_hint());
    eprintln!();
}

fn mkcert_output_indicates_untrusted_ca(output: &str) -> bool {
    output.contains(r#"Run "mkcert -install" for certificates to be trusted automatically"#)
        || output.contains("not installed in the system trust store")
}

fn ensure_mkcert_trust_ready(mkcert: &Path) -> Result<()> {
    let probe_root = std::env::temp_dir().join(format!(
        "bitloops-mkcert-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&probe_root).with_context(|| format!("create {}", probe_root.display()))?;
    let cert_path = probe_root.join("cert.pem");
    let key_path = probe_root.join("key.pem");

    let out = Command::new(mkcert)
        .arg("-cert-file")
        .arg(&cert_path)
        .arg("-key-file")
        .arg(&key_path)
        .arg("localhost")
        .arg("127.0.0.1")
        .arg("::1")
        .output()
        .with_context(|| format!("running mkcert trust probe via {}", mkcert.display()))?;

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stderr}\n{stdout}");

    let _ = fs::remove_file(&cert_path);
    let _ = fs::remove_file(&key_path);
    let _ = fs::remove_dir(&probe_root);

    if out.status.success() && mkcert_output_indicates_untrusted_ca(&combined) {
        bail!(
            "mkcert local CA is not trusted by this machine/browser yet.\n\
             Run `mkcert -install`, then retry dashboard."
        );
    }

    if !out.status.success() {
        bail!(
            "mkcert trust probe failed (exit {}).\n\
             stderr:\n{stderr}\n\
             stdout:\n{stdout}\n\
             {}",
            out.status,
            mkcert_hint()
        );
    }

    Ok(())
}

/// Ensures `mkcert` is on `PATH` before any certificate work.
pub fn require_mkcert_binary() -> Result<PathBuf> {
    match which::which("mkcert") {
        Ok(p) => Ok(p),
        Err(_) => {
            eprint_mkcert_not_found_on_path();
            bail!("mkcert not found on PATH");
        }
    }
}

pub fn mkcert_on_path() -> bool {
    which::which("mkcert").is_ok()
}

fn ensure_rustls_crypto_provider() -> Result<()> {
    static INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let init = INIT.get_or_init(|| {
        if CryptoProvider::get_default().is_none() {
            return rustls::crypto::aws_lc_rs::default_provider()
                .install_default()
                .map_err(|e| format!("install rustls aws_lc_rs crypto provider: {e:?}"));
        }
        Ok(())
    });
    match init {
        Ok(()) => Ok(()),
        Err(msg) => Err(anyhow!("{msg}")),
    }
}

fn cert_root_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".bitloops")
        .join("certs")
}

/// Deterministic directory name for a browser host (no collisions between IPv4/IPv6/DNS).
pub fn host_id_for_path(browser_host: &str) -> String {
    if let Ok(ip) = browser_host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(v4) => format!("ipv4-{}", v4).replace('.', "-"),
            IpAddr::V6(v6) => {
                if v6 == Ipv6Addr::LOCALHOST {
                    "ipv6-localhost".to_string()
                } else {
                    format!("ipv6-{}", v6).replace(':', "-")
                }
            }
        }
    } else {
        browser_host
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
    }
}

pub fn resolve_tls_paths_for_host(browser_host: &str) -> (PathBuf, PathBuf) {
    let id = host_id_for_path(browser_host);
    let dir = cert_root_dir().join(id);
    (dir.join("cert.pem"), dir.join("key.pem"))
}

fn mkcert_hint() -> &'static str {
    "More: https://github.com/FiloSottile/mkcert — macOS: brew install mkcert nss; Linux/Windows: see releases."
}

/// Build `mkcert` arguments after `-cert-file` / `-key-file` per plan.
fn mkcert_identity_args(browser_host: &str) -> Vec<String> {
    match browser_host {
        "bitloops.local" => vec![
            "bitloops.local".into(),
            "localhost".into(),
            "127.0.0.1".into(),
            "::1".into(),
        ],
        "localhost" => vec!["localhost".into(), "127.0.0.1".into(), "::1".into()],
        "127.0.0.1" => vec!["127.0.0.1".into(), "localhost".into(), "::1".into()],
        "::1" => vec!["::1".into(), "localhost".into(), "127.0.0.1".into()],
        other => {
            vec![
                other.to_string(),
                "localhost".into(),
                "127.0.0.1".into(),
                "::1".into(),
            ]
        }
    }
}

fn run_mkcert(cert_path: &Path, key_path: &Path, browser_host: &str) -> Result<()> {
    let mkcert = require_mkcert_binary()?;
    if let Some(parent) = cert_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let mut cmd = Command::new(&mkcert);
    cmd.arg("-cert-file")
        .arg(cert_path)
        .arg("-key-file")
        .arg(key_path);
    for a in mkcert_identity_args(browser_host) {
        cmd.arg(a);
    }

    let out = cmd
        .output()
        .with_context(|| format!("running {:?}", cmd.get_program()))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() {
        let combined = format!("{stderr}\n{stdout}");
        if mkcert_output_indicates_untrusted_ca(&combined) {
            bail!(
                "mkcert generated certificate but local CA trust is not installed.\n\
                 Run the instructions below, then retry launching the dashboard."
            );
        }
        return Ok(());
    }
    bail!(
        "mkcert failed (exit {}).\n\
         stderr:\n{stderr}\n\
         stdout:\n{stdout}\n\
         {}",
        out.status,
        mkcert_hint()
    )
}

fn load_server_config(cert_path: &Path, key_path: &Path) -> Result<Arc<ServerConfig>> {
    let cert_file =
        File::open(cert_path).with_context(|| format!("open cert {}", cert_path.display()))?;
    let key_file =
        File::open(key_path).with_context(|| format!("open key {}", key_path.display()))?;

    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("read certificate PEM chain")?;

    let mut key_reader = BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .context("read private key PEM")?
        .ok_or_else(|| anyhow!("no private key found in {}", key_path.display()))?;

    ensure_rustls_crypto_provider()?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("build rustls ServerConfig (check key matches certificate)")?;

    Ok(Arc::new(config))
}

fn load_leaf_certificate(cert_path: &Path) -> Result<CertificateDer<'static>> {
    let cert_file =
        File::open(cert_path).with_context(|| format!("open cert {}", cert_path.display()))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("read certificate PEM chain")?;
    certs
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no certificate found in {}", cert_path.display()))
}

fn browser_host_has_matching_san(
    cert_path: &Path,
    leaf_cert: &CertificateDer<'_>,
    browser_host: &str,
) -> Result<()> {
    let (_, cert) = parse_x509_certificate(leaf_cert.as_ref())
        .map_err(|e| anyhow!("parse X.509 certificate {}: {e}", cert_path.display()))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let nb = cert.validity().not_before.timestamp();
    let na = cert.validity().not_after.timestamp();
    if now < nb || now > na {
        bail!(
            "certificate at {} is outside its validity window (not_before={}, not_after={})",
            cert_path.display(),
            cert.validity().not_before,
            cert.validity().not_after
        );
    }

    let san = cert
        .subject_alternative_name()
        .map_err(|e| {
            anyhow!(
                "read SubjectAlternativeName from {}: {e}",
                cert_path.display()
            )
        })?
        .ok_or_else(|| {
            anyhow!(
                "certificate at {} has no SubjectAlternativeName extension",
                cert_path.display()
            )
        })?;

    if let Ok(ip) = browser_host.parse::<IpAddr>() {
        let matches = san.value.general_names.iter().any(|name| match (ip, name) {
            (IpAddr::V4(expected), GeneralName::IPAddress(raw)) => {
                raw.len() == 4 && *raw == expected.octets().as_slice()
            }
            (IpAddr::V6(expected), GeneralName::IPAddress(raw)) => {
                raw.len() == 16 && *raw == expected.octets().as_slice()
            }
            _ => false,
        });
        if matches {
            return Ok(());
        }
        bail!(
            "certificate at {} is missing IP SAN for {}",
            cert_path.display(),
            browser_host
        );
    }

    let matches = san.value.general_names.iter().any(|name| match name {
        GeneralName::DNSName(dns_name) => dns_name.eq_ignore_ascii_case(browser_host),
        _ => false,
    });
    if matches {
        return Ok(());
    }

    bail!(
        "certificate at {} is missing DNS SAN for {}",
        cert_path.display(),
        browser_host
    )
}

/// Validated TLS material for the dashboard (single shared server config).
pub struct DashboardTlsMaterial {
    pub server_config: Arc<ServerConfig>,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

/// Load existing TLS material without running trust probes or mkcert generation.
///
/// This is the fast-path loader used when local dashboard config explicitly
/// declares that TLS is already provisioned.
pub fn load_existing_dashboard_tls_material(browser_host: &str) -> Result<DashboardTlsMaterial> {
    let (cert_path, key_path) = resolve_tls_paths_for_host(browser_host);
    if !cert_path.is_file() || !key_path.is_file() {
        bail!(
            "dashboard TLS fast path requested for host {browser_host}, but certificate files are missing at {} and {}",
            cert_path.display(),
            key_path.display()
        );
    }

    let leaf_cert = load_leaf_certificate(&cert_path)?;
    browser_host_has_matching_san(&cert_path, &leaf_cert, browser_host).with_context(|| {
        format!(
            "dashboard TLS fast path validation failed for host {browser_host}; \
             run `bitloops daemon start --recheck-local-dashboard-net` once to refresh local dashboard network hints"
        )
    })?;
    let server_config = load_server_config(&cert_path, &key_path)?;

    Ok(DashboardTlsMaterial {
        server_config,
        cert_path,
        key_path,
    })
}

/// Ensure PEMs exist and are valid for `browser_host`, then build [`ServerConfig`] once.
pub fn ensure_dashboard_tls_material(browser_host: &str) -> Result<DashboardTlsMaterial> {
    let mkcert = require_mkcert_binary()?;
    ensure_mkcert_trust_ready(&mkcert)?;
    let (cert_path, key_path) = resolve_tls_paths_for_host(browser_host);

    if !cert_path.is_file() || !key_path.is_file() {
        run_mkcert(&cert_path, &key_path, browser_host)?;
    }

    let server_config = match load_leaf_certificate(&cert_path).and_then(|leaf_cert| {
        browser_host_has_matching_san(&cert_path, &leaf_cert, browser_host)?;
        load_server_config(&cert_path, &key_path)
    }) {
        Ok(server_config) => server_config,
        Err(validation_error) => {
            log::warn!(
                "dashboard TLS material invalid for host {browser_host}, regenerating: {validation_error:#}"
            );
            try_remove_pair(&cert_path, &key_path);
            run_mkcert(&cert_path, &key_path, browser_host)?;
            let leaf_cert = load_leaf_certificate(&cert_path)?;
            browser_host_has_matching_san(&cert_path, &leaf_cert, browser_host).with_context(
                || {
                    format!(
                        "regenerated certificate for {browser_host} is still invalid; \
                         refusing to open the dashboard browser URL"
                    )
                },
            )?;
            load_server_config(&cert_path, &key_path).with_context(|| {
                format!(
                    "regenerated TLS material for {browser_host} is unusable; \
                     refusing to open the dashboard browser URL"
                )
            })?
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(&key_path) {
            let mut mode = meta.permissions();
            mode.set_mode(0o600);
            let _ = fs::set_permissions(&key_path, mode);
        }
    }

    Ok(DashboardTlsMaterial {
        server_config,
        cert_path,
        key_path,
    })
}

fn try_remove_pair(cert_path: &Path, key_path: &Path) {
    let _ = fs::remove_file(cert_path);
    let _ = fs::remove_file(key_path);
}

// Inline implementation to avoid an extra dependency.
mod which {
    use std::path::PathBuf;

    pub fn which(name: &str) -> std::result::Result<PathBuf, ()> {
        let sep = std::env::var_os("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&sep) {
            let p = dir.join(name);
            if p.is_file() {
                return Ok(p);
            }
            #[cfg(windows)]
            {
                let p_exe = dir.join(format!("{name}.exe"));
                if p_exe.is_file() {
                    return Ok(p_exe);
                }
            }
        }
        Err(())
    }
}

#[cfg(test)]
mod tls_tests {
    use super::*;

    #[test]
    fn host_id_for_path_normalizes_hosts() {
        assert_eq!(host_id_for_path("127.0.0.1"), "ipv4-127-0-0-1");
        assert_eq!(host_id_for_path("::1"), "ipv6-localhost");
        assert_eq!(host_id_for_path("bitloops.local"), "bitloops-local");
    }

    #[test]
    fn mkcert_identity_args_include_loopback_identities() {
        assert_eq!(
            mkcert_identity_args("localhost"),
            vec!["localhost", "127.0.0.1", "::1"]
        );
        assert_eq!(
            mkcert_identity_args("bitloops.local"),
            vec!["bitloops.local", "localhost", "127.0.0.1", "::1"]
        );
    }

    #[test]
    fn mkcert_identity_args_have_no_duplicate_sans() {
        let ipv4 = mkcert_identity_args("127.0.0.1");
        assert_eq!(ipv4, vec!["127.0.0.1", "localhost", "::1"]);

        let ipv6 = mkcert_identity_args("::1");
        assert_eq!(ipv6, vec!["::1", "localhost", "127.0.0.1"]);
    }

    #[test]
    fn mkcert_identity_args_for_custom_host_include_loopback_fallbacks() {
        assert_eq!(
            mkcert_identity_args("dev.internal"),
            vec!["dev.internal", "localhost", "127.0.0.1", "::1"]
        );
    }

    #[test]
    fn resolve_tls_paths_for_host_uses_host_specific_directory() {
        let (cert, key) = resolve_tls_paths_for_host("bitloops.local");
        assert!(cert.ends_with("bitloops-local/cert.pem"));
        assert!(key.ends_with("bitloops-local/key.pem"));
    }

    #[test]
    fn mkcert_output_untrusted_ca_detection() {
        let text = r#"Note: the local CA is not installed in the system trust store.
Run "mkcert -install" for certificates to be trusted automatically ⚠️"#;
        assert!(mkcert_output_indicates_untrusted_ca(text));
        assert!(!mkcert_output_indicates_untrusted_ca(
            "Created a new certificate"
        ));
    }
}
