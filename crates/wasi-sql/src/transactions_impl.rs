use std::mem;
use std::sync::Arc;

use omnia_core::HostView;
use tokio::sync::{mpsc, oneshot};
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::database_impl::{DatabaseHandle, Selected, statement};
use crate::host::generated::omnia::sql::transactions::{
    Host, HostTransaction, HostTransactionWithStore, HostWithStore,
};
use crate::host::generated::omnia::sql::types::{ResultSet, Value};
use crate::host::{Connection, Error, Result, SqlClient, WasiSql};

// The host's side of a guest's `transaction` handle. Its calls queue for the
// one task that owns the transaction's connection, which runs them in the
// order made and rolls back once the queue closes on an open transaction.
//
// Requires a tokio runtime, which every `omnia` deployment provides.
#[derive(Clone, Debug)]
pub struct TransactionHandle {
    calls: mpsc::UnboundedSender<Call>,
}

impl TransactionHandle {
    // The task begins the transaction, so a guest that stops awaiting `begin`
    // leaves it to roll back what began.
    async fn begin(connection: Arc<dyn Connection>) -> Result<Self> {
        let (calls, queue) = mpsc::unbounded_channel();
        let (began, outcome) = oneshot::channel();
        tokio::spawn(Task::run(connection, began, queue));
        outcome.await.map_err(|_closed| stopped())??;
        Ok(Self { calls })
    }

    fn lookup<T>(accessor: &Accessor<T, WasiSql>, resource: &Resource<Self>) -> Result<Self> {
        Ok(omnia_core::get_cloned(accessor, resource)?)
    }

    // The call queues before the first await, so calls run in the order the
    // guest made them, and it runs to its end once queued.
    async fn call<R>(&self, call: impl FnOnce(oneshot::Sender<Result<R>>) -> Call) -> Result<R> {
        let (reply, replied) = oneshot::channel();
        self.calls.send(call(reply)).map_err(|_closed| stopped())?;
        replied.await.map_err(|_closed| stopped())?
    }

    // a failure the host meets after the task's reply still ends the transaction
    fn abort(&self) {
        let (reply, _) = oneshot::channel();
        drop(self.calls.send(Call::Rollback(reply)));
    }
}

// The task ends only by a panic, or by its runtime shutting down.
fn stopped() -> Error {
    Error::Other("the transaction's task has stopped".to_owned())
}

impl<T> HostWithStore<T> for WasiSql {
    async fn begin(
        accessor: &Accessor<T, Self>, database: Resource<DatabaseHandle>,
    ) -> Result<Resource<TransactionHandle>> {
        let connection = DatabaseHandle::connection(accessor, &database).await?;
        let handle = TransactionHandle::begin(connection).await?;
        Ok(accessor.with(|mut store| store.get().table.push(handle))?)
    }
}

impl<T> HostTransactionWithStore<T> for WasiSql {
    async fn query(
        accessor: &Accessor<T, Self>, self_: Resource<TransactionHandle>, sql: String,
        params: Vec<Option<Value>>,
    ) -> Result<ResultSet> {
        let handle = TransactionHandle::lookup(accessor, &self_)?;
        let selected = handle.call(|reply| Call::Query { sql, params, reply }).await?;
        selected.into_result_set(accessor).inspect_err(|_| handle.abort())
    }

    async fn execute(
        accessor: &Accessor<T, Self>, self_: Resource<TransactionHandle>, sql: String,
        params: Vec<Option<Value>>,
    ) -> Result<u64> {
        let handle = TransactionHandle::lookup(accessor, &self_)?;
        handle.call(|reply| Call::Execute { sql, params, reply }).await
    }

    async fn commit(
        accessor: &Accessor<T, Self>, self_: Resource<TransactionHandle>,
    ) -> Result<()> {
        TransactionHandle::lookup(accessor, &self_)?.call(Call::Commit).await
    }

    async fn rollback(
        accessor: &Accessor<T, Self>, self_: Resource<TransactionHandle>,
    ) -> Result<()> {
        TransactionHandle::lookup(accessor, &self_)?.call(Call::Rollback).await
    }

    fn drop(
        mut host: Access<'_, T, Self>, rep: Resource<TransactionHandle>,
    ) -> wasmtime::Result<()> {
        host.get().table.delete(rep)?;
        Ok(())
    }
}

enum Call {
    Query { sql: String, params: Vec<Option<Value>>, reply: oneshot::Sender<Result<Selected>> },
    Execute { sql: String, params: Vec<Option<Value>>, reply: oneshot::Sender<Result<u64>> },
    Commit(oneshot::Sender<Result<()>>),
    Rollback(oneshot::Sender<Result<()>>),
}

// The connection while the transaction is open, released as it ends.
struct Task {
    state: std::result::Result<Arc<dyn Connection>, Ended>,
}

#[derive(Clone, Copy)]
enum Ended {
    Committed,
    RolledBack,
    // a commit failed with `unavailable`, so whether it committed is unknown
    Unknown,
}

impl Task {
    async fn run(
        connection: Arc<dyn Connection>, began: oneshot::Sender<Result<()>>,
        mut queue: mpsc::UnboundedReceiver<Call>,
    ) {
        if let Err(err) = connection.begin().await {
            drop(began.send(Err(err.into())));
            return;
        }
        drop(began.send(Ok(())));

        let mut task = Self {
            state: Ok(connection),
        };
        while let Some(call) = queue.recv().await {
            match call {
                Call::Query { sql, params, reply } => {
                    drop(reply.send(task.query(sql, params).await));
                }
                Call::Execute { sql, params, reply } => {
                    drop(reply.send(task.execute(sql, params).await));
                }
                Call::Commit(reply) => drop(reply.send(task.commit().await)),
                Call::Rollback(reply) => drop(reply.send(task.rollback().await)),
            }
        }

        // the handle and every pending call are gone
        if let Ok(connection) = task.state.as_ref().map(Arc::clone)
            && roll_back(&*connection).await.is_ok()
        {
            drop(task.end(Ended::RolledBack));
        }
    }

    async fn query(&mut self, sql: String, params: Vec<Option<Value>>) -> Result<Selected> {
        let connection = self.connection()?;
        let selected = async {
            let statement = statement(sql, params)?;
            Selected::new(connection.select(statement.sql, statement.params).await?)
        }
        .await;
        self.settle(selected).await
    }

    async fn execute(&mut self, sql: String, params: Vec<Option<Value>>) -> Result<u64> {
        let connection = self.connection()?;
        let executed = async {
            let statement = statement(sql, params)?;
            connection.execute(statement.sql, statement.params).await.map_err(Error::from)
        }
        .await;
        self.settle(executed).await
    }

    // A failed commit is rolled back as well, because the backend may still
    // hold the transaction. The transaction has ended either way.
    async fn commit(&mut self) -> Result<()> {
        let connection = self.connection()?;
        let committed = connection.commit().await.map_err(Error::from);
        match &committed {
            Ok(()) => drop(self.end(Ended::Committed)),
            Err(Error::Unavailable(_)) => drop(self.end(Ended::Unknown)),
            Err(_) => {
                if roll_back(&*connection).await.is_ok() {
                    drop(self.end(Ended::RolledBack));
                }
            }
        }
        committed
    }

    async fn rollback(&mut self) -> Result<()> {
        if matches!(self.state, Err(Ended::RolledBack)) {
            return Ok(());
        }
        let connection = self.connection()?;
        connection.rollback().await?;
        drop(self.end(Ended::RolledBack));
        Ok(())
    }

    fn connection(&self) -> Result<Arc<dyn Connection>> {
        self.state.as_ref().map(Arc::clone).map_err(|ended| Error::from(*ended))
    }

    // a failed call rolls the transaction back and ends it
    async fn settle<R>(&mut self, outcome: Result<R>) -> Result<R> {
        if outcome.is_err()
            && let Ok(connection) = self.connection()
            && roll_back(&*connection).await.is_ok()
        {
            drop(self.end(Ended::RolledBack));
        }
        outcome
    }

    // The connection of a transaction still open, which ends as `ended`.
    fn end(&mut self, ended: Ended) -> Option<Arc<dyn Connection>> {
        if self.state.is_err() {
            return None;
        }
        mem::replace(&mut self.state, Err(ended)).ok()
    }
}

impl From<Ended> for Error {
    fn from(ended: Ended) -> Self {
        Self::Invalid(
            match ended {
                Ended::Committed => "the transaction has committed",
                Ended::RolledBack => "the transaction has rolled back",
                Ended::Unknown => "the transaction's commit failed with an unknown outcome",
            }
            .to_owned(),
        )
    }
}

// The guest sees the failure that ended the transaction, so a rollback's own
// failure is only logged.
async fn roll_back(connection: &dyn Connection) -> Result<()> {
    match connection.rollback().await {
        Ok(()) => Ok(()),
        Err(err) => {
            tracing::warn!("failed to roll back a transaction: {err:#}");
            Err(err.into())
        }
    }
}

impl Host for HostView<'_, dyn SqlClient> {}

impl HostTransaction for HostView<'_, dyn SqlClient> {}
