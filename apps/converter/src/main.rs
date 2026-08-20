use std::{
    error::Error,
    future::{pending, Future, IntoFuture},
    io,
    time::Duration,
};

use tokio::net::TcpListener;
use tokio::{sync::oneshot, time::Instant};
use tool_kit_converter::{config::Settings, router, AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let settings = Settings::from_env()?;
    init_tracing(&settings.log_filter)?;
    let app_state = AppState::initialize(&settings).await?;
    let listener = TcpListener::bind(settings.bind_address).await?;
    // The bound address, never the configured one. Port 0 asks the kernel for a
    // free port, so the configured value names no port at all and this line is
    // the only place the real one appears.
    let bound_address = listener.local_addr()?;

    tracing::info!(
        bind_address = %bound_address,
        service_version = env!("CARGO_PKG_VERSION"),
        "conversion service listening"
    );

    if serve_until_shutdown(
        listener,
        app_state,
        settings.shutdown_grace,
        shutdown_signal(settings.shutdown_on_stdin_eof),
    )
    .await?
    {
        return Err(std::io::Error::other("conversion job runner failed").into());
    }

    Ok(())
}

async fn serve_until_shutdown<F>(
    listener: TcpListener,
    app_state: AppState,
    shutdown_grace: Duration,
    shutdown: F,
) -> Result<bool, Box<dyn Error>>
where
    F: Future<Output = ()> + Send,
{
    let runner_state = app_state.clone();
    let shutdown_state = app_state.clone();
    let (http_shutdown, http_shutdown_rx) = oneshot::channel::<()>();
    let server = axum::serve(listener, router(app_state))
        .with_graceful_shutdown(async move {
            let _ = http_shutdown_rx.await;
        })
        .into_future();
    tokio::pin!(server);
    tokio::pin!(shutdown);

    let runner_failed = tokio::select! {
        () = &mut shutdown => false,
        failed = runner_state.wait_for_job_runner_exit() => {
            if failed {
                tracing::error!("conversion job runner exited unexpectedly");
            }
            failed
        }
        result = &mut server => {
            let deadline = Instant::now() + shutdown_grace;
            shutdown_state.stop_job_claiming();
            shutdown_state.shutdown_jobs_until(deadline).await;
            result?;
            return Ok(shutdown_state.job_runner_failed());
        }
    };

    let deadline = Instant::now() + shutdown_grace;
    shutdown_state.stop_job_claiming();
    let _ = http_shutdown.send(());
    let server_result = drain_with_deadline(
        &mut server,
        shutdown_state.shutdown_jobs_until(deadline),
        deadline,
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "HTTP shutdown grace elapsed"))?;
    server_result?;

    Ok(runner_failed || shutdown_state.job_runner_failed())
}

async fn drain_with_deadline<S, J, E>(
    server: S,
    job_shutdown: J,
    deadline: Instant,
) -> Result<Result<(), E>, tokio::time::error::Elapsed>
where
    S: Future<Output = Result<(), E>>,
    J: Future<Output = ()>,
{
    tokio::time::timeout_at(deadline, async {
        let (server_result, ()) = tokio::join!(server, job_shutdown);
        server_result
    })
    .await
}

fn init_tracing(raw_filter: &str) -> Result<(), Box<dyn Error>> {
    let filter = EnvFilter::try_new(raw_filter)?;

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();

    Ok(())
}

async fn shutdown_signal(shutdown_on_stdin_eof: bool) {
    let interrupt = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to install interrupt handler");
            pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "failed to install terminate handler");
                pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = pending::<()>();

    let reason = tokio::select! {
        () = interrupt => "interrupt",
        () = terminate => "terminate",
        () = stdin_eof(shutdown_on_stdin_eof) => "stdin_eof",
    };

    tracing::info!(reason, "shutdown signal received");
}

/// Resolves when stdin reaches EOF, and stays pending forever when the knob is
/// off. A supervised sidecar reads its parent's death off that pipe: the write
/// end closes even when the parent was killed outright and could signal
/// nothing, which is the one orphan case a signal handler cannot cover.
///
/// The read sits on a detached OS thread, not on the blocking pool. Dropping
/// the runtime waits for every blocking task that already started, so a pool
/// read of a pipe nobody closes makes SIGTERM inert: the service drains, `main`
/// returns, and the process then hangs forever on that read. A detached thread
/// does not hold up process exit, so the two shutdown paths stay independent.
async fn stdin_eof(enabled: bool) {
    if !enabled {
        pending::<()>().await;
    }

    let (closed, wait) = oneshot::channel();
    std::thread::spawn(move || {
        let outcome = io::copy(&mut io::stdin().lock(), &mut io::sink());
        // A read error leaves no way to notice the parent going away, so treat
        // it as the pipe closing rather than run on as a possible orphan.
        if let Err(error) = outcome {
            tracing::error!(%error, "failed to read stdin, treating it as closed");
        }
        // The receiver is gone whenever another arm of the select won, which is
        // the ordinary case and not a failure.
        let _ = closed.send(());
    });

    if wait.await.is_err() {
        tracing::error!("the stdin reader stopped, treating it as closed");
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::pending,
        io,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };

    use tokio::time::{timeout, Instant};

    use super::{drain_with_deadline, stdin_eof};

    #[tokio::test]
    async fn http_drain_and_job_shutdown_start_together_under_one_deadline() {
        let jobs_started = Arc::new(AtomicBool::new(false));
        let task_jobs_started = Arc::clone(&jobs_started);
        let deadline = Instant::now() + Duration::from_millis(50);
        let started_at = Instant::now();

        let result = timeout(
            Duration::from_millis(250),
            drain_with_deadline(
                pending::<Result<(), io::Error>>(),
                async move {
                    task_jobs_started.store(true, Ordering::SeqCst);
                    pending::<()>().await;
                },
                deadline,
            ),
        )
        .await
        .expect("shutdown orchestration exceeded its outer safety bound");

        assert!(
            result.is_err(),
            "the pending HTTP drain must hit the deadline"
        );
        assert!(jobs_started.load(Ordering::SeqCst));
        assert!(started_at.elapsed() < Duration::from_millis(200));
    }

    #[tokio::test]
    async fn stdin_is_not_a_shutdown_trigger_unless_the_knob_is_on() {
        // The container inherits whatever stdin Docker hands it, so an off knob
        // has to leave the process untouched even when that stdin is at EOF.
        let resolved = timeout(Duration::from_millis(50), stdin_eof(false)).await;

        assert!(resolved.is_err(), "the disabled reader must stay pending");
    }
}
