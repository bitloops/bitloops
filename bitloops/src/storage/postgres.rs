use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::runtime::{Builder, Runtime};
use tokio_postgres::NoTls;

thread_local! {
    static POSTGRES_SYNC_RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone)]
pub struct PostgresSyncConnection {
    dsn: String,
}

impl PostgresSyncConnection {
    pub fn connect(dsn: impl Into<String>) -> Result<Self> {
        let dsn = dsn.into();
        if dsn.trim().is_empty() {
            bail!("Postgres DSN is empty");
        }

        Ok(Self { dsn })
    }

    pub fn initialise_checkpoint_schema(&self) -> Result<()> {
        self.initialise_relational_checkpoint_schema()
    }

    pub fn initialise_relational_checkpoint_schema(&self) -> Result<()> {
        self.execute_batch(crate::host::devql::checkpoint_relational_schema_sql_postgres())
            .context("initialising Postgres relational checkpoint schema")
    }

    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        self.block_on(async {
            let client = connect_postgres_client(&self.dsn).await?;
            tokio::time::timeout(Duration::from_secs(30), client.batch_execute(sql))
                .await
                .context("Postgres statement timeout after 30s")?
                .context("executing Postgres statements")?;
            Ok(())
        })
    }

    pub fn ping(&self) -> Result<()> {
        self.block_on(async {
            let client = connect_postgres_client(&self.dsn).await?;
            let row =
                tokio::time::timeout(Duration::from_secs(30), client.query_one("SELECT 1", &[]))
                    .await
                    .context("Postgres query timeout after 30s")?
                    .context("running Postgres health query")?;
            let value: i32 = row
                .try_get(0)
                .context("reading Postgres health query result")?;
            if value != 1 {
                bail!("unexpected Postgres health query result: {value}");
            }

            Ok(())
        })
    }

    pub fn with_client<T>(
        &self,
        operation: impl for<'a> FnOnce(
            &'a mut tokio_postgres::Client,
        ) -> Pin<Box<dyn Future<Output = Result<T>> + 'a>>,
    ) -> Result<T> {
        self.block_on(async {
            let mut client = connect_postgres_client(&self.dsn).await?;
            operation(&mut client).await
        })
    }

    pub fn with_client_timeout<T>(
        &self,
        timeout: Duration,
        operation: impl for<'a> FnOnce(
            &'a mut tokio_postgres::Client,
        ) -> Pin<Box<dyn Future<Output = Result<T>> + 'a>>,
    ) -> Result<T> {
        self.block_on(async {
            let mut client = connect_postgres_client(&self.dsn).await?;
            run_postgres_operation_with_timeout(
                timeout,
                operation(&mut client),
                "Postgres client operation",
            )
            .await
        })
    }

    fn block_on<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        with_postgres_runtime(|runtime| runtime.block_on(future))
    }
}

pub(crate) async fn connect_postgres_client(dsn: &str) -> Result<tokio_postgres::Client> {
    let mut pg_cfg: tokio_postgres::Config = dsn.parse().context("parsing Postgres DSN")?;
    pg_cfg.connect_timeout(Duration::from_secs(10));

    let (client, connection) = tokio::time::timeout(Duration::from_secs(10), pg_cfg.connect(NoTls))
        .await
        .context("Postgres connect timeout after 10s")?
        .context("connecting to Postgres")?;

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::warn!("Postgres sync wrapper connection task ended: {err:#}");
        }
    });

    Ok(client)
}

fn with_postgres_runtime<T>(operation: impl FnOnce(&Runtime) -> Result<T>) -> Result<T> {
    POSTGRES_SYNC_RUNTIME.with(|runtime_slot| {
        if runtime_slot.borrow().is_none() {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| {
                    anyhow!(
                        "creating thread-local tokio runtime for Postgres sync wrapper: {err:#}"
                    )
                })?;
            *runtime_slot.borrow_mut() = Some(runtime);
        }

        let runtime_borrow = runtime_slot.borrow();
        let runtime = runtime_borrow
            .as_ref()
            .ok_or_else(|| anyhow!("thread-local Postgres runtime was not initialised"))?;
        operation(runtime)
    })
}

async fn run_postgres_operation_with_timeout<T>(
    timeout: Duration,
    operation: impl Future<Output = Result<T>>,
    label: &str,
) -> Result<T> {
    tokio::time::timeout(timeout, operation)
        .await
        .with_context(|| format!("{label} timeout after {}ms", timeout.as_millis()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn postgres_sync_connection_rejects_empty_dsn() {
        let result = PostgresSyncConnection::connect("   ");
        assert!(result.is_err());
    }

    #[test]
    fn postgres_client_operation_timeout_errors_when_future_runs_too_long() {
        let err = with_postgres_runtime(|runtime| {
            runtime.block_on(run_postgres_operation_with_timeout(
                Duration::from_millis(10),
                async {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok::<(), anyhow::Error>(())
                },
                "test Postgres operation",
            ))
        })
        .expect_err("long-running operation should time out");

        assert!(
            err.to_string()
                .contains("test Postgres operation timeout after 10ms")
        );
    }
}
