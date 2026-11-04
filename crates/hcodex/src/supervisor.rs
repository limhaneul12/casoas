//! Bounded ownership and cooperative cancellation for background work.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Semaphore, watch};
use tokio::task::{AbortHandle, JoinSet};

tokio::task_local! {
    static CANCELLATION: watch::Receiver<bool>;
    static JOB_ID: String;
}

#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<Inner>,
}

struct Inner {
    permits: Arc<Semaphore>,
    tasks: Mutex<JoinSet<()>>,
    controls: Mutex<BTreeMap<String, TaskControl>>,
}

struct TaskControl {
    cancel: watch::Sender<bool>,
    abort: AbortHandle,
}

impl Supervisor {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                permits: Arc::new(Semaphore::new(capacity)),
                tasks: Mutex::new(JoinSet::new()),
                controls: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    pub async fn spawn<F>(&self, job_id: String, future: F) -> crate::Result<()>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.reap().await?;
        // Same lock order as reap/shutdown. Hold registration until the control
        // exists, so a fast task cannot remove it before it is inserted.
        let mut tasks = self.inner.tasks.lock().await;
        let mut controls = self.inner.controls.lock().await;
        if controls.contains_key(&job_id) {
            return Err(format!("job is already supervised: {job_id}").into());
        }
        let permit = Arc::clone(&self.inner.permits)
            .try_acquire_owned()
            .map_err(|_| "background task capacity reached")?;
        let (cancel, receiver) = watch::channel(false);
        let task_cancel = cancel.clone();
        let parent_cancel = CANCELLATION.try_with(Clone::clone).ok();
        let inner = Arc::clone(&self.inner);
        let task_id = job_id.clone();
        let scope_id = job_id.clone();
        let wrapped = JOB_ID.scope(
            scope_id,
            CANCELLATION.scope(receiver, async move {
                let _permit = permit;
                tokio::pin!(future);
                if let Some(mut parent) = parent_cancel {
                    tokio::select! {
                        () = &mut future => {}
                        () = wait_for_cancel(&mut parent) => {
                            let _sent = task_cancel.send(true);
                            future.await;
                        }
                    }
                } else {
                    future.await;
                }
                inner.controls.lock().await.remove(&task_id);
            }),
        );
        let abort = tasks.spawn(wrapped);
        controls.insert(job_id, TaskControl { cancel, abort });
        Ok(())
    }

    pub async fn cancel(&self, job_id: &str) -> bool {
        self.inner
            .controls
            .lock()
            .await
            .get(job_id)
            .is_some_and(|control| control.cancel.send(true).is_ok())
    }

    pub async fn active(&self) -> usize {
        self.inner.controls.lock().await.len()
    }

    pub async fn shutdown(&self, grace: Duration) -> crate::Result<()> {
        {
            let controls = self.inner.controls.lock().await;
            for control in controls.values() {
                let _sent = control.cancel.send(true);
            }
        }
        let drain = async {
            let mut tasks = self.inner.tasks.lock().await;
            while let Some(joined) = tasks.join_next().await {
                joined?;
            }
            Ok::<_, crate::Error>(())
        };
        if tokio::time::timeout(grace, drain).await.is_err() {
            for control in self.inner.controls.lock().await.values() {
                control.abort.abort();
            }
            let mut tasks = self.inner.tasks.lock().await;
            tasks.abort_all();
            while let Some(joined) = tasks.join_next().await {
                if let Err(error) = joined
                    && !error.is_cancelled()
                {
                    return Err(error.into());
                }
            }
        }
        self.inner.controls.lock().await.clear();
        Ok(())
    }

    async fn reap(&self) -> crate::Result<()> {
        let mut tasks = self.inner.tasks.lock().await;
        while let Some(joined) = tasks.try_join_next() {
            if let Err(error) = joined
                && !error.is_cancelled()
            {
                return Err(error.into());
            }
        }
        Ok(())
    }
}

pub fn current_job_id() -> Option<String> {
    JOB_ID.try_with(Clone::clone).ok()
}

/// Capture the parent's task-local ownership before moving a future into JoinSet.
pub(crate) fn inherit_context<F>(future: F) -> impl Future<Output = F::Output>
where
    F: Future,
{
    let receiver = CANCELLATION.try_with(Clone::clone).ok();
    let job_id = current_job_id();
    async move {
        match (receiver, job_id) {
            (Some(receiver), Some(job_id)) => {
                JOB_ID
                    .scope(job_id, CANCELLATION.scope(receiver, future))
                    .await
            }
            _ => future.await,
        }
    }
}

pub(crate) fn is_cancelled() -> bool {
    CANCELLATION
        .try_with(|receiver| *receiver.borrow())
        .unwrap_or(false)
}

pub async fn cancelled() {
    let receiver = CANCELLATION.try_with(Clone::clone);
    match receiver {
        Ok(mut receiver) => {
            if *receiver.borrow() {
                return;
            }
            while receiver.changed().await.is_ok() {
                if *receiver.borrow() {
                    return;
                }
            }
        }
        Err(_) => std::future::pending::<()>().await,
    }
}

async fn wait_for_cancel(receiver: &mut watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    while receiver.changed().await.is_ok() {
        if *receiver.borrow() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn cancel_and_shutdown_own_every_task() -> crate::Result<()> {
        let supervisor = Supervisor::new(2);
        let observed = Arc::new(AtomicBool::new(false));
        let child_observed = Arc::clone(&observed);
        supervisor
            .spawn("job-1".to_owned(), async move {
                cancelled().await;
                child_observed.store(true, Ordering::SeqCst);
            })
            .await?;
        assert_eq!(supervisor.active().await, 1);
        assert!(supervisor.cancel("job-1").await);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !observed.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        supervisor.shutdown(Duration::from_secs(1)).await?;
        assert_eq!(supervisor.active().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn bounded_admission_rejects_excess_work() -> crate::Result<()> {
        let supervisor = Supervisor::new(1);
        supervisor
            .spawn("job-1".to_owned(), async { cancelled().await })
            .await?;
        assert!(
            supervisor
                .spawn("job-2".to_owned(), async { cancelled().await })
                .await
                .is_err()
        );
        supervisor.shutdown(Duration::from_secs(10)).await?;
        Ok(())
    }

    #[tokio::test]
    async fn campaign_cancellation_reaches_joinset_children() -> crate::Result<()> {
        let supervisor = Supervisor::new(1);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        supervisor
            .spawn("campaign".to_owned(), async move {
                let mut nodes = JoinSet::new();
                nodes.spawn(inherit_context(async move {
                    let _ready = ready_tx.send(current_job_id());
                    cancelled().await;
                    is_cancelled()
                }));
                let result = nodes.join_next().await;
                let _result = result_tx.send(result);
            })
            .await?;
        assert_eq!(ready_rx.await?, Some("campaign".to_owned()));
        assert!(supervisor.cancel("campaign").await);
        let joined = tokio::time::timeout(Duration::from_secs(10), result_rx).await??;
        assert!(joined.ok_or("node was not joined")??);
        supervisor.shutdown(Duration::from_secs(10)).await?;
        assert_eq!(supervisor.active().await, 0);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_registration_has_one_owner_and_fast_tasks_leave_no_controls()
    -> crate::Result<()> {
        let supervisor = Supervisor::new(2);
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let mut callers = JoinSet::new();
        for _ in 0..2 {
            let supervisor = supervisor.clone();
            let barrier = Arc::clone(&barrier);
            callers.spawn(async move {
                barrier.wait().await;
                supervisor
                    .spawn("same-id".to_owned(), async { cancelled().await })
                    .await
                    .is_ok()
            });
        }
        barrier.wait().await;
        let mut accepted = 0;
        while let Some(result) = callers.join_next().await {
            accepted += usize::from(result?);
        }
        assert_eq!(accepted, 1);
        supervisor.shutdown(Duration::from_secs(10)).await?;
        for index in 0..100 {
            supervisor.spawn(format!("fast-{index}"), async {}).await?;
            // Await each control's removal; no timing assumption or detached task.
            tokio::time::timeout(Duration::from_secs(10), async {
                while supervisor.active().await != 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await?;
        }
        supervisor.shutdown(Duration::from_secs(10)).await?;
        assert_eq!(supervisor.active().await, 0);
        Ok(())
    }
}
