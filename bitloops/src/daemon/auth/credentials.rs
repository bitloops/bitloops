use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(test)]
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(test)]
use std::sync::Mutex;

use super::types::{StoredWorkosTokens, WorkosCredentialKey};

static FALLBACK_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const FILE_CREDENTIAL_STORE_VERSION: u8 = 1;

pub(super) trait SecureCredentialStore: Send + Sync {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>>;
    fn save_tokens(&self, key: &WorkosCredentialKey, tokens: &StoredWorkosTokens) -> Result<()>;
    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileCredentialRecord {
    version: u8,
    service: String,
    account: String,
    tokens: StoredWorkosTokens,
}

#[derive(Debug, Clone)]
pub(super) struct SecureStoreUnavailable {
    operation: &'static str,
    message: String,
}

impl SecureStoreUnavailable {
    pub(super) fn new(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SecureStoreUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for SecureStoreUnavailable {}

#[derive(Debug, Clone)]
pub(super) struct FileCredentialStore {
    dir: PathBuf,
}

impl FileCredentialStore {
    pub(super) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn from_default_state_dir() -> Result<Self> {
        Ok(Self::new(
            crate::utils::platform_dirs::bitloops_state_dir()?
                .join("auth")
                .join("credentials"),
        ))
    }

    #[cfg(test)]
    pub(super) fn credential_file_path_for_test(&self, key: &WorkosCredentialKey) -> PathBuf {
        self.credential_file_path(key)
    }

    fn credential_file_path(&self, key: &WorkosCredentialKey) -> PathBuf {
        self.dir.join(format!("{}.json", credential_key_hash(key)))
    }

    fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir).with_context(|| {
            format!(
                "creating credential fallback directory {}",
                self.dir.display()
            )
        })?;
        set_dir_private(&self.dir)?;
        Ok(())
    }

    fn write_record_atomically(&self, path: &Path, record: &FileCredentialRecord) -> Result<()> {
        self.ensure_dir()?;
        let counter = FALLBACK_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = path.with_extension(format!("json.tmp.{}.{}", std::process::id(), counter));

        let write_result = (|| -> Result<()> {
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                options.mode(0o600);
            }
            let mut file = options.open(&tmp_path).with_context(|| {
                format!("creating temporary credential file {}", tmp_path.display())
            })?;
            serde_json::to_writer(&mut file, record)
                .context("serialising fallback credential payload")?;
            file.write_all(b"\n")
                .context("finalising fallback credential payload")?;
            file.flush()
                .context("flushing fallback credential payload")?;
            file.sync_all()
                .context("syncing fallback credential payload")?;
            set_file_private(&tmp_path)?;
            fs::rename(&tmp_path, path).with_context(|| {
                format!("installing fallback credential file {}", path.display())
            })?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        write_result
    }
}

impl SecureCredentialStore for FileCredentialStore {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>> {
        let path = self.credential_file_path(key);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("opening fallback credential file {}", path.display())
                });
            }
        };
        let record: FileCredentialRecord = serde_json::from_reader(file)
            .with_context(|| format!("parsing fallback credential file {}", path.display()))?;
        if record.version != FILE_CREDENTIAL_STORE_VERSION {
            bail!(
                "fallback credential file {} has unsupported version {}",
                path.display(),
                record.version
            );
        }
        if record.service != key.service || record.account != key.account {
            bail!(
                "fallback credential file {} does not match requested credential key",
                path.display()
            );
        }
        Ok(Some(record.tokens))
    }

    fn save_tokens(&self, key: &WorkosCredentialKey, tokens: &StoredWorkosTokens) -> Result<()> {
        let path = self.credential_file_path(key);
        let record = FileCredentialRecord {
            version: FILE_CREDENTIAL_STORE_VERSION,
            service: key.service.clone(),
            account: key.account.clone(),
            tokens: tokens.clone(),
        };
        self.write_record_atomically(&path, &record)
    }

    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()> {
        let path = self.credential_file_path(key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err)
                .with_context(|| format!("deleting fallback credential file {}", path.display())),
        }
    }
}

fn credential_key_hash(key: &WorkosCredentialKey) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.service.as_bytes());
    hasher.update([0]);
    hasher.update(key.account.as_bytes());
    hex::encode(hasher.finalize())
}

fn set_dir_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("setting private permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn set_file_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting private permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn keyring_error(operation: &'static str, err: keyring::Error) -> anyhow::Error {
    match err {
        keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_) => {
            SecureStoreUnavailable::new(operation, err.to_string()).into()
        }
        other => anyhow!(other).context(operation),
    }
}

fn is_secure_store_unavailable(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.is::<SecureStoreUnavailable>())
}

pub(super) struct KeyringCredentialStore;

impl SecureCredentialStore for KeyringCredentialStore {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>> {
        let entry = keyring::Entry::new(&key.service, &key.account)
            .context("opening secure credential entry")?;
        let secret = match entry.get_secret() {
            Ok(secret) => secret,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(err) => return Err(keyring_error("reading secure credentials", err)),
        };
        serde_json::from_slice(&secret)
            .context("parsing secure credential payload")
            .map(Some)
    }

    fn save_tokens(&self, key: &WorkosCredentialKey, tokens: &StoredWorkosTokens) -> Result<()> {
        let entry = keyring::Entry::new(&key.service, &key.account)
            .context("opening secure credential entry")?;
        let payload =
            serde_json::to_vec(tokens).context("serialising secure credential payload")?;
        entry
            .set_secret(&payload)
            .map_err(|err| keyring_error("writing secure credentials", err))
    }

    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()> {
        let entry = keyring::Entry::new(&key.service, &key.account)
            .context("opening secure credential entry")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(keyring_error("deleting secure credentials", err)),
        }
    }
}

pub(super) struct FallbackCredentialStore {
    primary: Arc<dyn SecureCredentialStore>,
    fallback: Arc<dyn SecureCredentialStore>,
}

impl FallbackCredentialStore {
    pub(super) fn new(
        primary: Arc<dyn SecureCredentialStore>,
        fallback: Arc<dyn SecureCredentialStore>,
    ) -> Self {
        Self { primary, fallback }
    }
}

impl SecureCredentialStore for FallbackCredentialStore {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>> {
        match self.primary.load_tokens(key) {
            Ok(Some(tokens)) => Ok(Some(tokens)),
            Ok(None) => self.fallback.load_tokens(key),
            Err(err) if is_secure_store_unavailable(&err) => self.fallback.load_tokens(key),
            Err(err) => Err(err),
        }
    }

    fn save_tokens(&self, key: &WorkosCredentialKey, tokens: &StoredWorkosTokens) -> Result<()> {
        match self.primary.save_tokens(key, tokens) {
            Ok(()) => {
                self.fallback
                    .delete_tokens(key)
                    .context("clearing fallback credentials after secure credential write")?;
                Ok(())
            }
            Err(err) if is_secure_store_unavailable(&err) => self
                .fallback
                .save_tokens(key, tokens)
                .with_context(|| format!("falling back after secure store failure: {err:#}")),
            Err(err) => Err(err),
        }
    }

    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()> {
        let primary_result = self.primary.delete_tokens(key);
        let fallback_result = self.fallback.delete_tokens(key);

        if let Err(err) = primary_result
            && !is_secure_store_unavailable(&err)
        {
            return Err(err);
        }

        fallback_result
    }
}

pub(super) fn default_secure_store() -> Arc<dyn SecureCredentialStore> {
    match FileCredentialStore::from_default_state_dir() {
        Ok(fallback) => Arc::new(FallbackCredentialStore::new(
            Arc::new(KeyringCredentialStore),
            Arc::new(fallback),
        )),
        Err(err) => {
            log::warn!(
                "file credential fallback is unavailable; using platform secure store only: {err:#}"
            );
            Arc::new(KeyringCredentialStore)
        }
    }
}

pub(super) async fn load_tokens(
    store: Arc<dyn SecureCredentialStore>,
    key: WorkosCredentialKey,
) -> Result<Option<StoredWorkosTokens>> {
    tokio::task::spawn_blocking(move || store.load_tokens(&key))
        .await
        .context("joining secure credential read task")?
}

pub(super) async fn save_tokens(
    store: Arc<dyn SecureCredentialStore>,
    key: WorkosCredentialKey,
    tokens: StoredWorkosTokens,
) -> Result<()> {
    tokio::task::spawn_blocking(move || store.save_tokens(&key, &tokens))
        .await
        .context("joining secure credential write task")?
}

pub(super) async fn delete_tokens(
    store: Arc<dyn SecureCredentialStore>,
    key: WorkosCredentialKey,
) -> Result<()> {
    tokio::task::spawn_blocking(move || store.delete_tokens(&key))
        .await
        .context("joining secure credential delete task")?
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct MemoryCredentialStore {
    inner: Mutex<HashMap<(String, String), StoredWorkosTokens>>,
}

#[cfg(test)]
impl SecureCredentialStore for MemoryCredentialStore {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>> {
        Ok(self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&(key.service.clone(), key.account.clone()))
            .cloned())
    }

    fn save_tokens(&self, key: &WorkosCredentialKey, tokens: &StoredWorkosTokens) -> Result<()> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert((key.service.clone(), key.account.clone()), tokens.clone());
        Ok(())
    }

    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&(key.service.clone(), key.account.clone()));
        Ok(())
    }
}
