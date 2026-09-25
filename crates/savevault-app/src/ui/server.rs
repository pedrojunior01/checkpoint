//! Servidor HTTP leve e embutido para servir a interface DEV-UI-01 e seus endpoints REST.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::assets::INDEX_HTML;
use super::state::{DevUiEventLevel, DevUiState};

#[derive(Debug, Deserialize)]
struct RestorePayload {
    snapshot_id: Uuid,
    expected_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScanPayload {
    location_id: Option<Uuid>,
}

/// Executa o loop principal do servidor HTTP atendendo conexões concorrentes até receber sinal de shutdown.
pub async fn run_dev_ui_server(
    state: Arc<DevUiState>,
    listener: TcpListener,
    mut shutdown_rx: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let local_addr = listener.local_addr()?;
    info!("Servidor DEV-UI ativo em http://{}", local_addr);

    state
        .record_event(
            DevUiEventLevel::Info,
            format!("Servidor DEV-UI iniciado em http://{}", local_addr),
            None,
        )
        .await;

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Sinal de shutdown recebido no servidor DEV-UI.");
                    break;
                }
            }
            conn_res = listener.accept() => {
                match conn_res {
                    Ok((stream, peer)) => {
                        debug!("Conexão recebida de {}", peer);
                        let state_clone = state.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(state_clone, stream).await {
                                debug!("Erro ao processar requisição de {}: {}", peer, e);
                            }
                        });
                    }
                    Err(e) => {
                        warn!("Erro ao aceitar conexão TCP: {}", e);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        }
    }

    info!("Servidor DEV-UI finalizado com sucesso.");
    Ok(())
}

/// Processa uma única conexão HTTP/1.1.
async fn handle_connection(state: Arc<DevUiState>, mut stream: TcpStream) -> anyhow::Result<()> {
    let mut buffer = vec![0u8; 16384];
    let mut total_read = 0;

    // Lê os dados iniciais da requisição
    let n = stream.read(&mut buffer).await?;
    if n == 0 {
        return Ok(());
    }
    total_read += n;

    let req_str = String::from_utf8_lossy(&buffer[..total_read]);
    let mut lines = req_str.lines();
    let request_line = match lines.next() {
        Some(l) => l,
        None => return Ok(()),
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let uri = parts.next().unwrap_or("/");

    // Suporte a OPTIONS (CORS preflight)
    if method == "OPTIONS" {
        send_response(&mut stream, 204, "No Content", "text/plain", "").await?;
        return Ok(());
    }

    // Separa path de query string
    let (path, query) = match uri.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (uri, None),
    };

    // Extrai o corpo da requisição se houver
    let body = if let Some(idx) = req_str.find("\r\n\r\n") {
        &req_str[idx + 4..]
    } else {
        ""
    };

    // Roteamento dos endpoints da DEV-UI
    match (method, path) {
        // -------------------------------------------------------------
        // Página Inicial / Assets
        // -------------------------------------------------------------
        ("GET", "/") | ("GET", "/index.html") => {
            send_response(
                &mut stream,
                200,
                "OK",
                "text/html; charset=utf-8",
                INDEX_HTML,
            )
            .await?;
        }

        // -------------------------------------------------------------
        // REST: Dashboard
        // -------------------------------------------------------------
        ("GET", "/api/dashboard") => match state.get_dashboard().await {
            Ok(summary) => {
                let json = serde_json::to_string(&summary)?;
                send_response(&mut stream, 200, "OK", "application/json", &json).await?;
            }
            Err(e) => {
                let err_json = serde_json::json!({ "error": e.to_string() }).to_string();
                send_response(
                    &mut stream,
                    500,
                    "Internal Server Error",
                    "application/json",
                    &err_json,
                )
                .await?;
            }
        },

        // -------------------------------------------------------------
        // REST: Save Locations
        // -------------------------------------------------------------
        ("GET", "/api/locations") => match state.get_locations().await {
            Ok(locations) => {
                let json = serde_json::to_string(&locations)?;
                send_response(&mut stream, 200, "OK", "application/json", &json).await?;
            }
            Err(e) => {
                let err_json = serde_json::json!({ "error": e.to_string() }).to_string();
                send_response(
                    &mut stream,
                    500,
                    "Internal Server Error",
                    "application/json",
                    &err_json,
                )
                .await?;
            }
        },

        // Scan sob demanda de um SaveLocation
        ("POST", "/api/locations/scan") => {
            // Tenta obter o UUID via query (?id=...) ou via body JSON
            let mut loc_id_opt = None;
            if let Some(q) = query {
                for pair in q.split('&') {
                    if let Some((k, v)) = pair.split_once('=') {
                        if k == "id" {
                            loc_id_opt = Uuid::parse_str(v).ok();
                        }
                    }
                }
            }

            if loc_id_opt.is_none() && !body.is_empty() {
                if let Ok(parsed) = serde_json::from_str::<ScanPayload>(body) {
                    loc_id_opt = parsed.location_id;
                }
            }

            match loc_id_opt {
                Some(loc_id) => match state.scan_location(loc_id).await {
                    Ok(result) => {
                        let json = serde_json::to_string(&result)?;
                        send_response(&mut stream, 200, "OK", "application/json", &json).await?;
                    }
                    Err(e) => {
                        let err_json = serde_json::json!({ "error": e.to_string() }).to_string();
                        send_response(
                            &mut stream,
                            400,
                            "Bad Request",
                            "application/json",
                            &err_json,
                        )
                        .await?;
                    }
                },
                None => {
                    let err_json =
                        serde_json::json!({ "error": "Parâmetro 'id' (UUID) é obrigatório." })
                            .to_string();
                    send_response(
                        &mut stream,
                        400,
                        "Bad Request",
                        "application/json",
                        &err_json,
                    )
                    .await?;
                }
            }
        }

        // -------------------------------------------------------------
        // REST: Snapshots
        // -------------------------------------------------------------
        ("GET", "/api/snapshots") => match state.get_snapshots().await {
            Ok(snapshots) => {
                let json = serde_json::to_string(&snapshots)?;
                send_response(&mut stream, 200, "OK", "application/json", &json).await?;
            }
            Err(e) => {
                let err_json = serde_json::json!({ "error": e.to_string() }).to_string();
                send_response(
                    &mut stream,
                    500,
                    "Internal Server Error",
                    "application/json",
                    &err_json,
                )
                .await?;
            }
        },

        // -------------------------------------------------------------
        // REST: Safety Backups
        // -------------------------------------------------------------
        ("GET", "/api/safety-backups") => match state.get_safety_backups().await {
            Ok(groups) => {
                let json = serde_json::to_string(&groups)?;
                send_response(&mut stream, 200, "OK", "application/json", &json).await?;
            }
            Err(e) => {
                let err_json = serde_json::json!({ "error": e.to_string() }).to_string();
                send_response(
                    &mut stream,
                    500,
                    "Internal Server Error",
                    "application/json",
                    &err_json,
                )
                .await?;
            }
        },

        // -------------------------------------------------------------
        // REST: Reconciliação
        // -------------------------------------------------------------
        ("GET", "/api/reconciliation") => match state.get_reconciliation_status().await {
            Ok(recon_view) => {
                let json = serde_json::to_string(&recon_view)?;
                send_response(&mut stream, 200, "OK", "application/json", &json).await?;
            }
            Err(e) => {
                let err_json = serde_json::json!({ "error": e.to_string() }).to_string();
                send_response(
                    &mut stream,
                    500,
                    "Internal Server Error",
                    "application/json",
                    &err_json,
                )
                .await?;
            }
        },

        // Disparo manual de reconciliação
        ("POST", "/api/reconcile-now") => match state.trigger_reconcile().await {
            Ok(report) => {
                let json = serde_json::json!({
                    "success": true,
                    "locations_scanned": report.locations_scanned,
                    "files_seen": report.files_seen,
                    "files_added": report.files_added,
                    "files_changed": report.files_changed,
                    "files_removed": report.files_removed,
                    "errors": report.errors,
                    "duration_ms": report.duration.as_millis(),
                })
                .to_string();
                send_response(&mut stream, 200, "OK", "application/json", &json).await?;
            }
            Err(e) => {
                let err_json = serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                })
                .to_string();
                send_response(
                    &mut stream,
                    500,
                    "Internal Server Error",
                    "application/json",
                    &err_json,
                )
                .await?;
            }
        },

        // -------------------------------------------------------------
        // REST: Restauração com Safety Backup
        // -------------------------------------------------------------
        ("POST", "/api/restore") => match serde_json::from_str::<RestorePayload>(body) {
            Ok(payload) => {
                let resp = state
                    .trigger_restore(payload.snapshot_id, payload.expected_hash)
                    .await;
                let json = serde_json::to_string(&resp)?;
                send_response(&mut stream, 200, "OK", "application/json", &json).await?;
            }
            Err(e) => {
                let err_json = serde_json::json!({
                        "success": false,
                        "error": format!("Payload JSON inválido: {}", e),
                        "details": "Esperado formato: { \"snapshot_id\": \"uuid\", \"expected_hash\": \"opcional\" }"
                    }).to_string();
                send_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    "application/json",
                    &err_json,
                )
                .await?;
            }
        },

        // -------------------------------------------------------------
        // REST: Eventos e Log
        // -------------------------------------------------------------
        ("GET", "/api/events") => {
            let events = state.get_events().await;
            let json = serde_json::to_string(&events)?;
            send_response(&mut stream, 200, "OK", "application/json", &json).await?;
        }

        // -------------------------------------------------------------
        // 404 Não Encontrado
        // -------------------------------------------------------------
        _ => {
            let not_found = serde_json::json!({ "error": "Endpoint não encontrado" }).to_string();
            send_response(
                &mut stream,
                404,
                "Not Found",
                "application/json",
                &not_found,
            )
            .await?;
        }
    }

    Ok(())
}

/// Envia uma resposta HTTP/1.1 formatada e fecha o stream.
async fn send_response(
    stream: &mut TcpStream,
    status_code: u16,
    reason: &str,
    content_type: &str,
    body: &str,
) -> anyhow::Result<()> {
    let response = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        status_code,
        reason,
        content_type,
        body.len(),
        body
    );

    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}
