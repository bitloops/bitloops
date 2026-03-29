use anyhow::{Context, Result};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_postgres::NoTls;

#[derive(Clone)]
pub(super) struct PostgresPool {
    inner: Arc<PostgresPoolInner>,
}

struct PostgresPoolInner {
    clients: Vec<tokio_postgres::Client>,
    next: AtomicUsize,
}

impl fmt::Debug for PostgresPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgresPool")
            .field("size", &self.inner.clients.len())
            .finish()
    }
}

impl PostgresPool {
    pub(super) async fn connect(dsn: &str, size: usize) -> Result<Self> {
        let size = size.max(1);
        let mut clients = Vec::with_capacity(size);

        for index in 0..size {
            let (client, connection) = tokio_postgres::connect(dsn, NoTls)
                .await
                .with_context(|| format!("connecting Postgres pool slot {}", index + 1))?;
            tokio::spawn(async move {
                if let Err(err) = connection.await {
                    log::warn!("dashboard Postgres connection task ended: {err:#}");
                }
            });
            clients.push(client);
        }

        Ok(Self {
            inner: Arc::new(PostgresPoolInner {
                clients,
                next: AtomicUsize::new(0),
            }),
        })
    }

    fn pick_client(&self) -> &tokio_postgres::Client {
        let len = self.inner.clients.len();
        let idx = self.inner.next.fetch_add(1, Ordering::Relaxed) % len;
        &self.inner.clients[idx]
    }

    pub(super) async fn ping(&self) -> Result<i32> {
        let row = self
            .pick_client()
            .query_one("SELECT 1", &[])
            .await
            .context("running Postgres health query `SELECT 1`")?;
        let value: i32 = row
            .try_get(0)
            .context("reading Postgres health query result")?;
        Ok(value)
    }
}
