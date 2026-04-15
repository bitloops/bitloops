use std::sync::Arc;

use anyhow::{Context, Result};

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

use super::types::{StoredWorkosTokens, WorkosCredentialKey};

pub(super) trait SecureCredentialStore: Send + Sync {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>>;
    fn save_tokens(&self, key: &WorkosCredentialKey, tokens: &StoredWorkosTokens) -> Result<()>;
    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()>;
}

pub(super) struct KeyringCredentialStore;

impl SecureCredentialStore for KeyringCredentialStore {
    fn load_tokens(&self, key: &WorkosCredentialKey) -> Result<Option<StoredWorkosTokens>> {
        let entry = keyring::Entry::new(&key.service, &key.account)
            .context("opening secure credential entry")?;
        let secret = match entry.get_secret() {
            Ok(secret) => secret,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(err) => return Err(err).context("reading secure credentials"),
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
            .context("writing secure credentials")
    }

    fn delete_tokens(&self, key: &WorkosCredentialKey) -> Result<()> {
        let entry = keyring::Entry::new(&key.service, &key.account)
            .context("opening secure credential entry")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).context("deleting secure credentials"),
        }
    }
}

pub(super) fn default_secure_store() -> Arc<dyn SecureCredentialStore> {
    Arc::new(KeyringCredentialStore)
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
