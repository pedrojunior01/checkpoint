//! Módulo da interface visual provisória para testes e validação manual (DEV-UI-01).

pub mod assets;
pub mod server;
pub mod state;

#[cfg(feature = "desktop")]
pub mod tauri_app;

pub use state::{
    DashboardSummary, DevUiEvent, DevUiEventLevel, DevUiState, DiscoveredFileView, LocationView,
    ReconciliationView, RestoreApiResponse, SafetyBackupGroup, SafetyBackupItem,
    ScanLocationResult, SnapshotView,
};

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::info;

/// Inicia o servidor HTTP da DEV-UI na porta preferencial informada (ex.: 4040)
/// com fallback automático para porta dinâmica caso a porta desejada esteja ocupada.
pub async fn start_dev_ui_server(
    state: Arc<DevUiState>,
    preferred_port: u16,
    shutdown_rx: watch::Receiver<bool>,
) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<anyhow::Result<()>>)> {
    let addr = format!("127.0.0.1:{}", preferred_port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(
                "Porta {} ocupada ou indisponível ({}), alocando porta livre automaticamente...",
                preferred_port,
                e
            );
            TcpListener::bind("127.0.0.1:0").await?
        }
    };

    let bound_addr = listener.local_addr()?;
    info!("DEV-UI-01 pronta para conexões em http://{}", bound_addr);

    let server_handle = tokio::spawn(server::run_dev_ui_server(state, listener, shutdown_rx));

    Ok((bound_addr, server_handle))
}
