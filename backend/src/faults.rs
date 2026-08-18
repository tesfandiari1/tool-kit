//! Deterministic crash barriers for recovery tests.
//!
//! Production never arms one. Nothing in the configuration, the HTTP surface, or
//! the environment can reach `arm`, so the only cost on the live path is one
//! relaxed atomic load per barrier.

use std::sync::atomic::{AtomicU8, Ordering};

use tokio::sync::Notify;

/// A gap between two committed transitions where a test can freeze the worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FaultPoint {
    AfterClaim = 1,
    AfterFinalizing = 2,
    AfterPublish = 3,
    BeforeSuccessCommit = 4,
}

/// Discriminants start at one so the `Default` zero means "disarmed".
#[derive(Debug, Default)]
pub struct FaultBarrier {
    armed: AtomicU8,
    reached: Notify,
}

impl FaultBarrier {
    /// Arms one point. Only tests call this.
    pub fn arm(&self, point: FaultPoint) {
        self.armed.store(point as u8, Ordering::Relaxed);
    }

    /// Resolves once the armed point has been reached.
    pub async fn wait_reached(&self) {
        self.reached.notified().await;
    }

    /// Parks forever when `point` is armed. A parked task never unwinds and never
    /// writes again, so the durable state stays exactly as a killed process would
    /// have left it.
    pub(crate) async fn hold(&self, point: FaultPoint) {
        if self.armed.load(Ordering::Relaxed) != point as u8 {
            return;
        }
        // `notify_one` stores a permit when nobody is waiting yet, so a test that reaches
        // `wait_reached` after the worker parks still sees the signal. `notify_waiters`
        // drops it and the test hangs, which is why this must not be "simplified".
        self.reached.notify_one();
        std::future::pending::<()>().await
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::{FaultBarrier, FaultPoint};

    #[tokio::test]
    async fn a_disarmed_barrier_never_holds() {
        let barrier = FaultBarrier::default();
        tokio::time::timeout(
            Duration::from_secs(5),
            barrier.hold(FaultPoint::BeforeSuccessCommit),
        )
        .await
        .expect("a disarmed barrier must return immediately");
    }

    #[tokio::test]
    async fn an_unrelated_armed_point_never_holds() {
        let barrier = FaultBarrier::default();
        barrier.arm(FaultPoint::AfterClaim);
        tokio::time::timeout(
            Duration::from_secs(5),
            barrier.hold(FaultPoint::AfterPublish),
        )
        .await
        .expect("only the armed point holds");
    }

    #[tokio::test]
    async fn a_late_waiter_still_sees_the_signal() {
        let barrier = Arc::new(FaultBarrier::default());
        barrier.arm(FaultPoint::AfterPublish);

        let held = Arc::clone(&barrier);
        tokio::spawn(async move { held.hold(FaultPoint::AfterPublish).await });

        // Give the spawned task time to park before anybody waits on it.
        tokio::time::sleep(Duration::from_millis(50)).await;
        tokio::time::timeout(Duration::from_secs(5), barrier.wait_reached())
            .await
            .expect("the stored permit must outlive the moment it was issued");
    }
}
