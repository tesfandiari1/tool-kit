//! Shared child-process supervision for the engine workers.
//!
//! One implementation of the deadline/cancellation semantics the job runner
//! proved in M2, so a second engine cannot drift from them: a cancelled or
//! timed-out worker is always killed and reaped, never left behind.

use std::process::ExitStatus;

use tokio::{
    process::Child,
    sync::watch,
    time::{sleep, Duration},
};

use super::EngineFailure;

/// Waits for a worker child under a hard deadline and a cancellation watch.
pub(crate) async fn wait_for_child(
    child: &mut Child,
    timeout: Duration,
    mut cancellation: watch::Receiver<bool>,
) -> Result<ExitStatus, EngineFailure> {
    let mut deadline = Box::pin(sleep(timeout));
    let status = loop {
        tokio::select! {
            biased;
            changed = cancellation.changed() => {
                let interrupted = changed.is_err() || *cancellation.borrow_and_update();
                if interrupted {
                    kill_and_reap(child).await;
                    return Err(EngineFailure::Interrupted);
                }
            }
            _ = &mut deadline => {
                kill_and_reap(child).await;
                return Err(EngineFailure::Timeout);
            }
            result = child.wait() => {
                match result {
                    Ok(status) => break status,
                    Err(_) => {
                        kill_and_reap(child).await;
                        return Err(EngineFailure::Crashed);
                    }
                }
            }
        }
    };
    if !status.success() {
        return Err(EngineFailure::Crashed);
    }
    if *cancellation.borrow() {
        return Err(EngineFailure::Interrupted);
    }
    Ok(status)
}

pub(crate) async fn kill_and_reap(child: &mut Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}
