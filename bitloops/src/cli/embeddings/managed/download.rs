use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, USER_AGENT};
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MANAGED_RELEASE_DOWNLOAD_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(crate) struct DownloadedManagedAsset {
    path: PathBuf,
    pub(crate) bytes_downloaded: u64,
    pub(crate) bytes_total: Option<u64>,
    pub(crate) sha256_hex: String,
}

impl DownloadedManagedAsset {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DownloadedManagedAsset {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn download_release_asset_to_temp_file(
    client: &Client,
    url: &str,
    user_agent: &str,
    asset_label: &str,
    mut progress: impl FnMut(u64, Option<u64>) -> Result<()>,
) -> Result<DownloadedManagedAsset> {
    let mut response = client
        .get(url)
        .header(ACCEPT, "application/octet-stream")
        .header(USER_AGENT, user_agent)
        .send()
        .with_context(|| format!("downloading {asset_label} from {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading {asset_label} from {url}"))?;
    let mut download = DownloadedManagedAsset {
        path: temporary_download_path(asset_label),
        bytes_downloaded: 0,
        bytes_total: response.content_length(),
        sha256_hex: String::new(),
    };
    let mut file = File::create(download.path()).with_context(|| {
        format!(
            "creating temporary download file {}",
            download.path().display()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut chunk = [0_u8; MANAGED_RELEASE_DOWNLOAD_BUFFER_BYTES];
    progress(download.bytes_downloaded, download.bytes_total)?;
    loop {
        let read = response
            .read(&mut chunk)
            .with_context(|| format!("reading {asset_label} bytes"))?;
        if read == 0 {
            break;
        }
        file.write_all(&chunk[..read]).with_context(|| {
            format!(
                "writing temporary download file {}",
                download.path().display()
            )
        })?;
        hasher.update(&chunk[..read]);
        download.bytes_downloaded = download.bytes_downloaded.saturating_add(read as u64);
        progress(download.bytes_downloaded, download.bytes_total)?;
    }
    file.flush().with_context(|| {
        format!(
            "flushing temporary download file {}",
            download.path().display()
        )
    })?;
    download.sha256_hex = hex::encode(hasher.finalize());
    Ok(download)
}

fn temporary_download_path(asset_label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let safe_label: String = asset_label
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() {
                char.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    env::temp_dir().join(format!(
        "bitloops-{safe_label}.{}.{}.download",
        std::process::id(),
        suffix
    ))
}
