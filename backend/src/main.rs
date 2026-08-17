use std::{error::Error, future::pending};

use tokio::net::TcpListener;
use tool_kit_converter::{config::Settings, router, AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let settings = Settings::from_env()?;
    init_tracing(&settings.log_filter)?;
    let app_state = AppState::initialize(&settings)?;
    let listener = TcpListener::bind(settings.bind_address).await?;

    tracing::info!(
        bind_address = %settings.bind_address,
        service_version = env!("CARGO_PKG_VERSION"),
        "conversion service listening"
    );

    axum::serve(listener, router(app_state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
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
