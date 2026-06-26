//! Seqr mailbox service entry point. All logic lives in the library
//! (`seqr_mailbox`); this binary only wires configuration, logging, and the
//! TCP listener with graceful shutdown.

use std::sync::Arc;

use seqr_mailbox::config::Config;
use seqr_mailbox::store::Store;
use seqr_mailbox::{build_router, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "seqr_mailbox=info".into()),
        )
        .init();

    let config = Config::from_env();
    let store = Store::new(&config.data_dir).expect("create data dir");
    let bind = config.bind.clone();
    let state = Arc::new(AppState { store, config });

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&bind).await.expect("bind");
    tracing::info!(%bind, "seqr-mailbox listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server");
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}
