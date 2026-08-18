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

    tracing::info!(
        bind_address = %settings.bind_address,
        service_version = env!("CARGO_PKG_VERSION"),
        "conversion service listening"
    );

    if serve_until_shutdown(
        listener,
        app_state,
        settings.shutdown_grace,
        shutdown_signal(),
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

async fn shutdown_signal() {
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

    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }

    tracing::info!("shutdown signal received");
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

    use super::drain_with_deadline;

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
}
