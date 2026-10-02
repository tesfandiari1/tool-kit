use std::{future::Future, sync::Arc, time::Duration};

use thiserror::Error;
use tokio::{
    sync::{watch, Mutex, Notify, Semaphore},
    task::JoinHandle,
    time::{timeout_at, Instant},
};

use crate::{
    conversion::{ConversionExecutionError, ConversionService},
    persistence::RepositoryError,
};

mod recovery;

pub(crate) use recovery::StartupRecovery;

const MAX_FORCE_CANCEL_WAIT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunnerStatus {
    Running,
    Stopped,
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct JobRuntime {
    inner: Arc<JobRuntimeInner>,
}

#[derive(Debug)]
struct JobRuntimeInner {
    stop: watch::Sender<bool>,
    cancel: watch::Sender<bool>,
    wake: Arc<Notify>,
    claim_gate: Arc<Semaphore>,
    status: watch::Sender<RunnerStatus>,
    idle: watch::Sender<bool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl JobRuntime {
    pub(crate) fn spawn(service: ConversionService, poll_interval: Duration) -> Self {
        let (stop, stop_rx) = watch::channel(false);
        let (cancel, cancel_rx) = watch::channel(false);
        let (status, _) = watch::channel(RunnerStatus::Running);
        let (idle, _) = watch::channel(false);
        let wake = service.work_notification();
        let claim_gate = Arc::new(Semaphore::new(1));
        let task_status = status.clone();
        let task_wake = Arc::clone(&wake);
        let task_idle = idle.clone();
        let task_claim_gate = Arc::clone(&claim_gate);
        let handle = tokio::spawn(async move {
            match run(
                service,
                task_wake,
                poll_interval,
                stop_rx,
                cancel_rx,
                task_idle,
                task_claim_gate,
            )
            .await
            {
                Ok(()) => {
                    task_status.send_replace(RunnerStatus::Stopped);
                }
                Err(error) => {
                    tracing::error!(%error, "single conversion job runner stopped after a fatal execution error");
                    task_status.send_replace(RunnerStatus::Failed);
                }
            }
        });
        Self {
            inner: Arc::new(JobRuntimeInner {
                stop,
                cancel,
                wake,
                claim_gate,
                status,
                idle,
                handle: Mutex::new(Some(handle)),
            }),
        }
    }

    pub(crate) fn stop_claiming(&self) {
        self.inner.stop.send_replace(true);
        self.inner.claim_gate.close();
        self.inner.wake.notify_waiters();
    }

    pub(crate) fn force_cancel(&self) {
        self.stop_claiming();
        self.inner.cancel.send_replace(true);
        self.inner.wake.notify_waiters();
    }

    pub(crate) fn failed(&self) -> bool {
        if *self.inner.status.borrow() == RunnerStatus::Running
            && self
                .inner
                .handle
                .try_lock()
                .ok()
                .and_then(|handle| handle.as_ref().map(JoinHandle::is_finished))
                .unwrap_or(false)
        {
            self.mark_failed_if_running();
        }
        *self.inner.status.borrow() == RunnerStatus::Failed
    }

    pub(crate) fn stopped(&self) -> bool {
        *self.inner.status.borrow() == RunnerStatus::Stopped
    }

    pub(crate) async fn wait_until_stopped(&self) -> bool {
        let mut status = self.inner.status.subscribe();
        loop {
            match *status.borrow_and_update() {
                RunnerStatus::Failed => return true,
                RunnerStatus::Stopped => return false,
                RunnerStatus::Running => {}
            }
            if self
                .inner
                .handle
                .lock()
                .await
                .as_ref()
                .is_some_and(JoinHandle::is_finished)
            {
                self.mark_failed_if_running();
                return *self.inner.status.borrow() == RunnerStatus::Failed;
            }
            tokio::select! {
                changed = status.changed() => {
                    if changed.is_err() {
                        return true;
                    }
                }
                () = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    }

    pub(crate) async fn wait_until_idle(&self) {
        let mut idle = self.inner.idle.subscribe();
        loop {
            if *idle.borrow_and_update() {
                return;
            }
            if idle.changed().await.is_err() {
                return;
            }
        }
    }

    pub(crate) async fn shutdown(&self, grace: Duration) {
        self.shutdown_until(Instant::now() + grace).await;
    }

    pub(crate) async fn shutdown_until(&self, deadline: Instant) {
        self.stop_claiming();
        let Some(handle) = self.inner.handle.lock().await.take() else {
            return;
        };
        let mut handle = AbortOnDropJoinHandle(handle);
        let grace = deadline.saturating_duration_since(Instant::now());
        let force_wait = grace.min(MAX_FORCE_CANCEL_WAIT);
        let graceful_deadline = deadline.checked_sub(force_wait).unwrap_or(deadline);
        match timeout_at(graceful_deadline, &mut handle.0).await {
            Ok(Ok(())) => return,
            Ok(Err(error)) => {
                tracing::error!(%error, "conversion job runner task failed during shutdown");
                self.inner.status.send_replace(RunnerStatus::Failed);
                return;
            }
            Err(_) => {}
        }

        self.force_cancel();
        match timeout_at(deadline, &mut handle.0).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::error!(%error, "conversion job runner task failed during shutdown");
                self.inner.status.send_replace(RunnerStatus::Failed);
            }
            Err(_) => {
                tracing::error!("conversion job runner did not stop after forced cancellation");
                self.inner.status.send_replace(RunnerStatus::Failed);
            }
        }
    }

    fn mark_failed_if_running(&self) {
        self.inner.status.send_if_modified(|status| {
            if *status == RunnerStatus::Running {
                *status = RunnerStatus::Failed;
                true
            } else {
                false
            }
        });
    }
}

struct AbortOnDropJoinHandle(JoinHandle<()>);

impl Drop for AbortOnDropJoinHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Drop for JobRuntimeInner {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        self.cancel.send_replace(true);
        self.claim_gate.close();
        self.wake.notify_waiters();
        if let Some(handle) = self.handle.get_mut().take() {
            handle.abort();
        }
    }
}

async fn run(
    service: ConversionService,
    wake: Arc<Notify>,
    poll_interval: Duration,
    mut stop: watch::Receiver<bool>,
    cancel: watch::Receiver<bool>,
    idle: watch::Sender<bool>,
    claim_gate: Arc<Semaphore>,
) -> Result<(), JobRunnerError> {
    loop {
        if *stop.borrow() {
            return Ok(());
        }

        match stop_aware_claim(claim_gate.as_ref(), &stop, service.claim_next_queued()).await? {
            ClaimDecision::Stop => return Ok(()),
            ClaimDecision::Raced(job) => {
                tracing::info!(
                    job_id = %job.id,
                    attempt_id = %job.active_attempt.id,
                    "stop raced with a durable claim; preserving conversion for recovery"
                );
                return Ok(());
            }
            ClaimDecision::Claimed(job) => {
                idle.send_replace(false);
                service.execute_claimed(job, cancel.clone()).await?;
                continue;
            }
            ClaimDecision::Idle => {
                idle.send_replace(true);
                tokio::select! {
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow() {
                            return Ok(());
                        }
                    }
                    () = wake.notified() => {}
                    () = tokio::time::sleep(poll_interval) => {}
                }
                idle.send_replace(false);
            }
        }
    }
}

#[derive(Debug)]
enum ClaimDecision<T> {
    Stop,
    Idle,
    Claimed(T),
    Raced(T),
}

async fn stop_aware_claim<T, E, F>(
    claim_gate: &Semaphore,
    stop: &watch::Receiver<bool>,
    claim: F,
) -> Result<ClaimDecision<T>, E>
where
    F: Future<Output = Result<Option<T>, E>>,
{
    let claim_permit = match claim_gate.acquire().await {
        Ok(permit) => permit,
        Err(_) => return Ok(ClaimDecision::Stop),
    };
    if *stop.borrow() {
        return Ok(ClaimDecision::Stop);
    }
    let claimed = claim.await?;
    drop(claim_permit);

    Ok(match claimed {
        Some(job) if *stop.borrow() => ClaimDecision::Raced(job),
        Some(job) => ClaimDecision::Claimed(job),
        None if *stop.borrow() => ClaimDecision::Stop,
        None => ClaimDecision::Idle,
    })
}

#[derive(Debug, Error)]
enum JobRunnerError {
    #[error(transparent)]
    Persistence(#[from] RepositoryError),
    #[error(transparent)]
    Execution(#[from] ConversionExecutionError),
}

#[cfg(test)]
mod tests {
    use std::{
        future::pending,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant as StdInstant},
    };

    use tokio::{
        sync::{watch, Mutex, Notify, Semaphore},
        task::JoinHandle,
        time::timeout,
    };

    use super::{stop_aware_claim, ClaimDecision, JobRuntime, JobRuntimeInner, RunnerStatus};

    fn runtime_with_handle(handle: JoinHandle<()>) -> JobRuntime {
        let (stop, _) = watch::channel(false);
        let (cancel, _) = watch::channel(false);
        let (status, _) = watch::channel(RunnerStatus::Running);
        let (idle, _) = watch::channel(false);
        JobRuntime {
            inner: Arc::new(JobRuntimeInner {
                stop,
                cancel,
                wake: Arc::new(Notify::new()),
                claim_gate: Arc::new(Semaphore::new(1)),
                status,
                idle,
                handle: Mutex::new(Some(handle)),
            }),
        }
    }

    #[tokio::test]
    async fn stale_finished_observation_cannot_overwrite_a_clean_stop() {
        let runtime = runtime_with_handle(tokio::spawn(pending()));
        runtime.inner.status.send_replace(RunnerStatus::Stopped);

        runtime.mark_failed_if_running();

        assert!(runtime.stopped());
        assert!(!runtime.failed());
    }

    #[tokio::test]
    async fn stop_during_an_inflight_claim_preserves_the_returned_claim() {
        let claim_gate = Arc::new(Semaphore::new(1));
        let (stop, stop_rx) = watch::channel(false);
        let claim_started = Arc::new(Notify::new());
        let finish_claim = Arc::new(Notify::new());
        let task_claim_started = Arc::clone(&claim_started);
        let task_finish_claim = Arc::clone(&finish_claim);
        let task_claim_gate = Arc::clone(&claim_gate);
        let task = tokio::spawn(async move {
            stop_aware_claim(task_claim_gate.as_ref(), &stop_rx, async move {
                task_claim_started.notify_one();
                task_finish_claim.notified().await;
                Ok::<_, ()>(Some(7_u8))
            })
            .await
            .unwrap()
        });

        claim_started.notified().await;
        stop.send_replace(true);
        claim_gate.close();
        finish_claim.notify_one();

        assert!(matches!(task.await.unwrap(), ClaimDecision::Raced(7)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hard_deadline_does_not_await_an_abort_resistant_task() {
        let started = Arc::new(AtomicBool::new(false));
        let task_started = Arc::clone(&started);
        let handle = tokio::spawn(async move {
            task_started.store(true, Ordering::SeqCst);
            let finish_at = StdInstant::now() + Duration::from_millis(250);
            while StdInstant::now() < finish_at {
                std::hint::spin_loop();
            }
        });
        while !started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        let runtime = runtime_with_handle(handle);
        let shutdown_started = StdInstant::now();

        timeout(
            Duration::from_millis(100),
            runtime.shutdown(Duration::from_millis(20)),
        )
        .await
        .expect("shutdown awaited an abort-resistant task past its hard bound");

        assert!(shutdown_started.elapsed() < Duration::from_millis(100));
        assert!(runtime.failed());
    }
}
