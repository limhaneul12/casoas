//! Bounded ownership and cooperative cancellation for background work.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, Semaphore, watch};
use tokio::task::JoinSet;

tokio::task_local! {
    static CANCELLATION: watch::Receiver<bool>;
    static JOB_ID: String;
}

#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<Inner>,
}

struct Inner {
    closed: AtomicBool,
    shutdown: Mutex<()>,
    permits: Arc<Semaphore>,
    tasks: Mutex<JoinSet<()>>,
    controls: Mutex<BTreeMap<String, TaskControl>>,
}

struct TaskControl {
    cancel: watch::Sender<bool>,
}

impl Supervisor {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                closed: AtomicBool::new(false),
                shutdown: Mutex::new(()),
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
        if self.inner.closed.load(Ordering::Acquire) {
            return Err("supervisor admission is closed".into());
        }
        self.reap().await?;
        // Same lock order as reap/shutdown. Hold registration until the control
        // exists, so a fast task cannot remove it before it is inserted.
        let mut tasks = self.inner.tasks.lock().await;
        if self.inner.closed.load(Ordering::Acquire) {
            return Err("supervisor admission is closed".into());
        }
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
        let _abort = tasks.spawn(wrapped);
        controls.insert(job_id, TaskControl { cancel });
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
        // Serialize complete shutdowns so a second caller cannot report success
        // while the first still owns draining tasks. Callers must await shutdown.
        let _shutdown = self.inner.shutdown.lock().await;
        let mut tasks = {
            let mut tasks = self.inner.tasks.lock().await;
            self.inner.closed.store(true, Ordering::Release);
            let controls = self.inner.controls.lock().await;
            for control in controls.values() {
                let _sent = control.cancel.send(true);
            }
            std::mem::replace(&mut *tasks, JoinSet::new())
        };
        let mut first_error: Option<crate::Error> = None;
        let drain = async {
            while let Some(joined) = tasks.join_next().await {
                if let Err(error) = joined {
                    first_error.get_or_insert_with(|| error.into());
                }
            }
        };
        if tokio::time::timeout(grace, drain).await.is_err() {
            tasks.abort_all();
            first_error.get_or_insert_with(|| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "supervisor grace expired; owned tasks were forcibly aborted",
                )
                .into()
            });
            while let Some(joined) = tasks.join_next().await {
                if let Err(error) = joined
                    && !error.is_cancelled()
                {
                    first_error.get_or_insert_with(|| error.into());
                }
            }
        }
        self.inner.controls.lock().await.clear();
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
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

pub(crate) async fn scope_cancellation<F: Future>(
    receiver: watch::Receiver<bool>,
    future: F,
) -> F::Output {
    CANCELLATION.scope(receiver, future).await
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
    async fn shutdown_closes_admission_before_draining() -> crate::Result<()> {
        let supervisor = Supervisor::new(2);
        let (cancelled_tx, cancelled_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        supervisor
            .spawn("draining".to_owned(), async move {
                cancelled().await;
                let _ = cancelled_tx.send(());
                let _ = finish_rx.await;
            })
            .await?;
        let shutting_down = supervisor.clone();
        let task =
            tokio::spawn(async move { shutting_down.shutdown(Duration::from_secs(5)).await });
        tokio::time::timeout(Duration::from_secs(5), cancelled_rx).await??;
        assert!(supervisor.spawn("late".to_owned(), async {}).await.is_err());
        let _ = finish_tx.send(());
        task.await??;
        assert_eq!(supervisor.active().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn panicked_child_does_not_skip_owned_sibling_cleanup() -> crate::Result<()> {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let supervisor = Supervisor::new(2);
        let (panic_tx, panic_rx) = tokio::sync::oneshot::channel();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&dropped);
        supervisor
            .spawn("panic".to_owned(), async move {
                let _ = panic_rx.await;
                std::panic::resume_unwind(Box::new("intentional supervised test panic"));
            })
            .await?;
        supervisor
            .spawn("sibling".to_owned(), async move {
                let _drop = Dropped(observed);
                let _ = ready_tx.send(());
                std::future::pending::<()>().await;
            })
            .await?;
        ready_rx.await?;
        let _ = panic_tx.send(());
        assert!(
            supervisor
                .shutdown(Duration::from_millis(50))
                .await
                .is_err()
        );
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(supervisor.active().await, 0);
        assert!(supervisor.spawn("late".to_owned(), async {}).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn forced_shutdown_joins_tasks_but_reports_grace_expiry() -> crate::Result<()> {
        let supervisor = Supervisor::new(1);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        supervisor
            .spawn("uncooperative".to_owned(), async move {
                let _ = ready_tx.send(());
                std::future::pending::<()>().await;
            })
            .await?;
        ready_rx.await?;
        let error = supervisor
            .shutdown(Duration::from_millis(20))
            .await
            .err()
            .ok_or("forced shutdown must report grace expiry")?;
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::TimedOut)
        );
        assert_eq!(supervisor.active().await, 0);
        assert!(supervisor.spawn("late".to_owned(), async {}).await.is_err());
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
        let supervisor = Supervisor::new(2);
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
