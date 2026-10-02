//! Servidor HTTP leve e embutido para servir a interface DEV-UI-01 e seus endpoints REST.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::assets::{APP_JS, INDEX_HTML};
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

#[derive(Debug, Deserialize)]
struct SetupLoginPayload {
    provider: Option<String>,
    local_sync_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetupCompletePayload {
    provider: Option<String>,
    autostart: Option<bool>,
    notifications: Option<bool>,
    local_sync_path: Option<String>,
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
    // Lê os dados da requisição até encontrar o fim dos headers (\r\n\r\n)
    let mut buffer = Vec::with_capacity(4096);
    let mut header_end = None;

    loop {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);

        if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = Some(pos);
            break;
        }

        if buffer.len() > 65536 {
            // Headers excessivamente grandes -> rejeita
            return Ok(());
        }
    }

    let header_end_pos = match header_end {
        Some(pos) => pos,
        None => return Ok(()),
    };

    let header_str = String::from_utf8_lossy(&buffer[..header_end_pos]).into_owned();
    let mut lines = header_str.lines();
    let request_line = match lines.next() {
        Some(l) => l,
        None => return Ok(()),
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let uri = parts.next().unwrap_or("/");

    // Extrai e valida headers (Host, Origin, Content-Length, Headers Anti-CSRF)
    let mut content_length = 0usize;
    let mut origin_valid = true;
    let mut has_local_origin = false;
    let mut host_valid = true;
    let mut has_host = false;
    let mut has_custom_header = false;

    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if k == "content-length" {
                content_length = v.parse::<usize>().unwrap_or(0);
            } else if k == "host" {
                has_host = true;
                if !is_valid_local_host(v) {
                    host_valid = false;
                }
            } else if k == "origin" {
                // Origem permitida apenas de localhost/127.0.0.1 com porta opcional
                if !v.is_empty() {
                    if is_valid_local_origin(v) {
                        has_local_origin = true;
                    } else {
                        origin_valid = false;
                    }
                }
            } else if k == "x-savevault-request" || k == "x-requested-with" {
                has_custom_header = true;
            }
        }
    }

    if (has_host && !host_valid) || !origin_valid {
        warn!("Tentativa de requisição com Host ou Origin externa bloqueada por segurança (DNS Rebinding / CORS)");
        send_response(
            &mut stream,
            403,
            "Forbidden",
            "application/json",
            r#"{"error":"Forbidden Host or Origin"}"#,
        )
        .await?;
        return Ok(());
    }

    // Proteção Anti-CSRF para requisições com mutação de estado (POST)
    if method == "POST" && !has_local_origin && !has_custom_header {
        warn!("Tentativa de requisição POST sem validação Anti-CSRF bloqueada");
        send_response(
            &mut stream,
            403,
            "Forbidden",
            "application/json",
            r#"{"error":"CSRF Protection: Missing required local origin or X-SaveVault-Request header"}"#,
        )
        .await?;
        return Ok(());
    }

    // Suporte a OPTIONS (CORS preflight)
    if method == "OPTIONS" {
        send_response(&mut stream, 204, "No Content", "text/plain", "").await?;
        return Ok(());
    }

    // Lê o restante do corpo até atingir content_length (limite máximo de 2MB para evitar DoS)
    let body_start = header_end_pos + 4;
    let max_body_size = 2 * 1024 * 1024;
    if content_length > max_body_size {
        send_response(
            &mut stream,
            413,
            "Payload Too Large",
            "application/json",
            r#"{"error":"Payload too large"}"#,
        )
        .await?;
        return Ok(());
    }

    while buffer.len() < body_start + content_length {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
    }

    let body_bytes = if buffer.len() >= body_start {
        let available = buffer.len() - body_start;
        let to_take = available.min(content_length);
        &buffer[body_start..body_start + to_take]
    } else {
        &[]
    };

    let body_owned = String::from_utf8_lossy(body_bytes);
    let body = body_owned.as_ref();

    // Separa path de query string
    let (path, query) = match uri.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (uri, None),
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

        ("GET", "/app.js") => {
            send_response(
                &mut stream,
                200,
                "OK",
                "application/javascript; charset=utf-8",
                APP_JS,
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
        ("POST", "/api/reconcile-now") | ("POST", "/api/reconcile") => {
            match state.trigger_reconcile().await {
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
            }
        }

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
        // REST: Autostart (Windows Registry)
        // -------------------------------------------------------------
        ("GET", "/api/autostart") => {
            let status = savevault_core::autostart::status().unwrap_or(
                savevault_core::autostart::AutostartStatus {
                    enabled: false,
                    command: None,
                    executable_path: None,
                },
            );
            let json = serde_json::to_string(&status)?;
            send_response(&mut stream, 200, "OK", "application/json", &json).await?;
        }

        ("POST", "/api/autostart/toggle") => {
            let is_on = savevault_core::autostart::is_enabled().unwrap_or(false);
            let res = if is_on {
                savevault_core::autostart::disable()
            } else {
                savevault_core::autostart::enable(None)
            };

            match res {
                Ok(new_status) => {
                    let json = serde_json::to_string(&new_status)?;
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
            }
        }

        // -------------------------------------------------------------
        // REST: Assistente de Primeiro Acesso (Setup Wizard)
        // -------------------------------------------------------------
        ("GET", "/api/setup/status") => match state.get_setup_status().await {
            Ok(status) => {
                let json = serde_json::to_string(&status)?;
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

        ("POST", "/api/setup/scan") => match state.run_setup_discovery().await {
            Ok(games) => {
                let json = serde_json::json!({
                    "success": true,
                    "games": games
                })
                .to_string();
                send_response(&mut stream, 200, "OK", "application/json", &json).await?;
            }
            Err(e) => {
                let err_json =
                    serde_json::json!({ "success": false, "error": e.to_string() }).to_string();
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

        ("GET", "/api/setup/cloud-folders") => {
            let folders = savevault_cloud::detect_cloud_folders();
            let json = serde_json::to_string(&folders)?;
            send_response(&mut stream, 200, "OK", "application/json", &json).await?;
        }

        ("POST", "/api/setup/login-cloud") => {
            let payload: SetupLoginPayload =
                serde_json::from_str(body).unwrap_or(SetupLoginPayload {
                    provider: None,
                    local_sync_path: None,
                });

            let mut config = state.get_config();

            let target_path = if let Some(ref lsp) = payload.local_sync_path {
                let trimmed = lsp.trim();
                if !trimmed.is_empty() {
                    let p = std::path::PathBuf::from(trimmed);
                    let _ = std::fs::create_dir_all(&p);
                    config.cloud.local_sync_path = Some(p.clone());
                    Some(p)
                } else {
                    None
                }
            } else {
                config.cloud.local_sync_path.clone().or_else(|| {
                    savevault_cloud::detect_cloud_folders().first().map(|d| {
                        let _ = std::fs::create_dir_all(&d.savevault_folder);
                        config.cloud.local_sync_path = Some(d.savevault_folder.clone());
                        d.savevault_folder.clone()
                    })
                })
            };

            let chosen_provider = payload.provider.as_deref().unwrap_or("local_folder");
            config.cloud.provider = chosen_provider.to_string();
            let _ = state.save_config(&config);

            let path_display = target_path
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Pasta local".into());
            let json = serde_json::json!({
                "success": true,
                "status": "connected",
                "provider": "local_folder",
                "path": path_display,
                "message": "Pasta sincronizada configurada com sucesso!"
            })
            .to_string();
            send_response(&mut stream, 200, "OK", "application/json", &json).await?;
        }

        ("POST", "/api/setup/complete") => {
            let payload: SetupCompletePayload =
                serde_json::from_str(body).unwrap_or(SetupCompletePayload {
                    provider: None,
                    autostart: None,
                    notifications: None,
                    local_sync_path: None,
                });
            match state
                .complete_setup(
                    payload.provider,
                    payload.autostart,
                    payload.notifications,
                    payload.local_sync_path,
                )
                .await
            {
                Ok(status) => {
                    let json = serde_json::to_string(&status)?;
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
            }
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
         Access-Control-Allow-Origin: http://127.0.0.1:4040\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type, X-SaveVault-Request, X-Requested-With\r\n\
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

/// Valida se uma origem pertence exclusivamente ao localhost (IPv4 ou nome de host local)
/// rejeitando qualquer sufixo como domínios de invasores (ex: http://127.0.0.1.attacker.com).
pub fn is_valid_local_origin(origin: &str) -> bool {
    let lower = origin.trim().to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("http://127.0.0.1") {
        r
    } else if let Some(r) = lower.strip_prefix("http://localhost") {
        r
    } else {
        return false;
    };

    if rest.is_empty() {
        return true;
    }

    if let Some(port) = rest.strip_prefix(':') {
        !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

/// Valida se o cabeçalho Host pertence exclusivamente ao localhost (IPv4 ou nome de host local)
/// rejeitando ataques de DNS Rebinding.
pub fn is_valid_local_host(host: &str) -> bool {
    let lower = host.trim().to_ascii_lowercase();
    let name_part = lower.split(':').next().unwrap_or("");
    name_part == "127.0.0.1" || name_part == "localhost"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_local_host() {
        assert!(is_valid_local_host("127.0.0.1:4040"));
        assert!(is_valid_local_host("localhost:4040"));
        assert!(is_valid_local_host("127.0.0.1"));
        assert!(is_valid_local_host("localhost"));

        // Ataques de DNS Rebinding ou hosts externos
        assert!(!is_valid_local_host("attacker.com"));
        assert!(!is_valid_local_host("evil.com:4040"));
        assert!(!is_valid_local_host("127.0.0.1.attacker.com"));
        assert!(!is_valid_local_host("localhost.attacker.com:4040"));
    }

    #[test]
    fn test_is_valid_local_origin() {
        assert!(is_valid_local_origin("http://127.0.0.1:4040"));
        assert!(is_valid_local_origin("http://localhost:4040"));
        assert!(is_valid_local_origin("http://127.0.0.1"));
        assert!(is_valid_local_origin("http://localhost"));

        // Origens forjadas
        assert!(!is_valid_local_origin("http://127.0.0.1.attacker.com"));
        assert!(!is_valid_local_origin("http://localhost.attacker.com"));
        assert!(!is_valid_local_origin("https://evil.org"));
    }
}
