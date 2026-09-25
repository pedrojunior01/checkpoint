use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use savevault_cloud::auth::AuthService;
use savevault_cloud::google_drive::GoogleDriveProvider;
use savevault_cloud::CloudProvider;

// ---------------------------------------------------------------------------
// Mock HTTP Server
// ---------------------------------------------------------------------------

struct MockResponse {
    status: u16,
    status_text: &'static str,
    content_type: &'static str,
    body: String,
}

struct MockHttpServer {
    base_url: String,
    _shutdown_tx: oneshot::Sender<()>,
}

impl MockHttpServer {
    async fn spawn<F>(handler: F) -> Self
    where
        F: Fn(&str, &str, &str) -> MockResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{}", addr);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let handler = Arc::new(handler);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accept_res = listener.accept() => {
                        let (mut socket, _) = match accept_res {
                            Ok(s) => s,
                            Err(_) => break,
                        };
                        let h = handler.clone();
                        tokio::spawn(async move {
                            let mut buf = vec![0u8; 8192];
                            let n = socket.read(&mut buf).await.unwrap_or(0);
                            if n == 0 { return; }
                            let req_str = String::from_utf8_lossy(&buf[..n]);
                            let first_line = req_str.lines().next().unwrap_or("");
                            let mut parts = first_line.split_whitespace();
                            let method = parts.next().unwrap_or("GET");
                            let path = parts.next().unwrap_or("/");

                            let resp = h(method, path, &req_str);
                            let body_bytes = resp.body.as_bytes();
                            let head = format!(
                                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                resp.status, resp.status_text, resp.content_type, body_bytes.len()
                            );
                            let _ = socket.write_all(head.as_bytes()).await;
                            let _ = socket.write_all(body_bytes).await;
                        });
                    }
                }
            }
        });

        Self {
            base_url,
            _shutdown_tx: shutdown_tx,
        }
    }
}

// ---------------------------------------------------------------------------
// Adversarial / Mock HTTP Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_upload_success() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("mimeType") {
            // Retorna pastas existentes para SaveVault, game e file
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[{"id":"folder-123","name":"mock"}]}"#.to_string(),
            }
        } else if method == "GET" && path.contains("/files?q=") {
            // Checagem de arquivo existente (idempotência): por padrão não existe antes do upload
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[]}"#.to_string(),
            }
        } else if method == "POST" && path.contains("uploadType=multipart") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"id":"uploaded-drive-id-999","name":"save.dat","size":"1234"}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Not Found".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("mock-bearer-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"save game binary payload").unwrap();

    let res = provider.upload(tmp.path(), "game_uuid_1/file_uuid_2/save.dat").await;
    assert!(res.is_ok(), "Upload deve ter sucesso contra mock HTTP: {:?}", res);
    let drive_file_id = res.unwrap();
    assert_eq!(drive_file_id, "uploaded-drive-id-999");
}

#[tokio::test]
async fn test_upload_401_unauthorized() {
    let server = MockHttpServer::spawn(|_, _, _| {
        MockResponse {
            status: 401,
            status_text: "Unauthorized",
            content_type: "application/json",
            body: r#"{"error":{"code":401,"message":"Invalid Credentials"}}"#.to_string(),
        }
    }).await;

    let auth = AuthService::with_fixed_token("expired-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"data").unwrap();

    let err = provider.upload(tmp.path(), "game/file/save.dat").await.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("401"), "Erro deve reportar código 401: {}", err_str);
    assert!(err_str.contains("Invalid Credentials"), "Erro deve reportar mensagem: {}", err_str);
}

#[tokio::test]
async fn test_upload_403_forbidden() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("mimeType") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[{"id":"f1","name":"mock"}]}"#.to_string(),
            }
        } else if method == "GET" && path.contains("/files?q=") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[]}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 403,
                status_text: "Forbidden",
                content_type: "application/json",
                body: r#"{"error":{"code":403,"message":"Insufficient Permission"}}"#.to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"data").unwrap();

    let err = provider.upload(tmp.path(), "game/file/save.dat").await.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("403"), "Erro deve reportar 403: {}", err_str);
    assert!(err_str.contains("Insufficient Permission"), "Erro deve reportar mensagem: {}", err_str);
}

#[tokio::test]
async fn test_upload_429_rate_limited() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("mimeType") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[{"id":"f1","name":"mock"}]}"#.to_string(),
            }
        } else if method == "GET" && path.contains("/files?q=") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[]}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 429,
                status_text: "Too Many Requests",
                content_type: "application/json",
                body: r#"{"error":{"code":429,"message":"Rate Limit Exceeded"}}"#.to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"data").unwrap();

    let err = provider.upload(tmp.path(), "game/file/save.dat").await.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("429"), "Erro deve reportar 429: {}", err_str);
    assert!(err_str.contains("Rate Limit Exceeded"), "Erro deve reportar mensagem: {}", err_str);
}

#[tokio::test]
async fn test_upload_500_server_error() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("mimeType") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[{"id":"f1","name":"mock"}]}"#.to_string(),
            }
        } else if method == "GET" && path.contains("/files?q=") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[]}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 500,
                status_text: "Internal Server Error",
                content_type: "text/html",
                body: "<html>500 Internal Server Error</html>".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"data").unwrap();

    let err = provider.upload(tmp.path(), "game/file/save.dat").await.unwrap_err();
    let err_str = err.to_string();
    assert!(err_str.contains("500"), "Erro deve reportar 500 mesmo com body HTML: {}", err_str);
}

#[tokio::test]
async fn test_download_success() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("/files/drive-file-abc?alt=media") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/octet-stream",
                body: "downloaded cloud save content".to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Not Found".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("restored_save.dat");

    let res = provider.download("drive-file-abc", &dest).await;
    assert!(res.is_ok(), "Download deve ter sucesso: {:?}", res);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "downloaded cloud save content");
}

#[tokio::test]
async fn test_delete_success() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "DELETE" && path.contains("/files/drive-to-delete-123") {
            MockResponse {
                status: 204,
                status_text: "No Content",
                content_type: "text/plain",
                body: "".to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Not Found".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let res = provider.delete("drive-to-delete-123").await;
    assert!(res.is_ok(), "Delete deve retornar Ok para status 204: {:?}", res);

    // 404 também deve ser tolerado
    let res_404 = provider.delete("nonexistent-file").await;
    assert!(res_404.is_ok(), "Delete deve tolerar 404 (já deletado): {:?}", res_404);
}

#[tokio::test]
async fn test_invalid_json_response_handling() {
    let server = MockHttpServer::spawn(|_, _, _| {
        MockResponse {
            status: 200,
            status_text: "OK",
            content_type: "application/json",
            body: "BROKEN_JSON_NOT_VALID".to_string(),
        }
    }).await;

    let auth = AuthService::with_fixed_token("token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"data").unwrap();

    let err = provider.upload(tmp.path(), "game/file/save.dat").await.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("JSON inválida") || err_str.contains("BROKEN_JSON"),
        "Deve reportar erro de JSON inválido: {}", err_str
    );
}

#[tokio::test]
async fn test_token_refresh_flow() {
    let server = MockHttpServer::spawn(|method, path, req| {
        if method == "POST" && path == "/oauth2/token" {
            assert!(req.contains("grant_type=refresh_token"));
            assert!(req.contains("refresh_token=valid_refresh_token_12345"));
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"access_token":"fresh_access_token_xyz987","expires_in":3600}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 400,
                status_text: "Bad Request",
                content_type: "text/plain",
                body: "Bad Request".to_string(),
            }
        }
    }).await;

    let token_endpoint = format!("{}/oauth2/token", server.base_url);
    let auth = AuthService::with_mock_refresh(
        "test-client-id".to_string(),
        "test-client-secret".to_string(),
        token_endpoint,
        "valid_refresh_token_12345".to_string(),
    );

    let token = auth.get_fresh_access_token().await.unwrap();
    assert_eq!(token, "fresh_access_token_xyz987", "Token renovado com sucesso");
}

#[tokio::test]
async fn test_case_a_upload_returns_file_id() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("mimeType") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[{"id":"folder-root","name":"mock"}]}"#.to_string(),
            }
        } else if method == "GET" && path.contains("/files?q=") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[]}"#.to_string(),
            }
        } else if method == "POST" && path.contains("uploadType=multipart") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"id":"drive-file-123","name":"v1_save.dat","size":"2048"}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Not Found".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("mock-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"save content").unwrap();

    let res = provider.upload(tmp.path(), "location_uuid/file_uuid/v1_save.dat").await;
    assert!(res.is_ok(), "Upload deve ter sucesso: {:?}", res);
    let drive_file_id = res.unwrap();
    assert_eq!(drive_file_id, "drive-file-123", "Upload DEVE retornar o File ID retornado pelo Google Drive");
}

#[tokio::test]
async fn test_case_b_download_rejects_cloud_path_and_uses_file_id() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("/files/drive-file-123?alt=media") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/octet-stream",
                body: "binary payload".to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Not Found".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("mock-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("downloaded.dat");

    // 1. Tentar passar um cloud_path (com barras) DEVE falhar imediatamente sem fazer chamada HTTP
    let err = provider.download("location_uuid/file_uuid/v1_save.dat", &dest).await.unwrap_err();
    assert!(
        err.to_string().contains("Identificador remoto inválido"),
        "Download com cloud_path deve falhar com erro de rejeição semântica: {}",
        err
    );

    // 2. Passar o drive_file_id real DEVE funcionar
    let ok = provider.download("drive-file-123", &dest).await;
    assert!(ok.is_ok(), "Download com File ID real deve ter sucesso: {:?}", ok);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "binary payload");
}

#[tokio::test]
async fn test_case_c_delete_rejects_cloud_path_and_uses_file_id() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "DELETE" && path.contains("/files/drive-file-123") {
            MockResponse {
                status: 204,
                status_text: "No Content",
                content_type: "text/plain",
                body: "".to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Not Found".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("mock-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    // 1. Tentar deletar passando cloud_path DEVE falhar
    let err = provider.delete("location_uuid/file_uuid/v1_save.dat").await.unwrap_err();
    assert!(
        err.to_string().contains("Identificador remoto inválido"),
        "Delete com cloud_path deve falhar com rejeição semântica: {}",
        err
    );

    // 2. Deletar com drive_file_id DEVE suceder
    let ok = provider.delete("drive-file-123").await;
    assert!(ok.is_ok(), "Delete com File ID deve suceder: {:?}", ok);
}

#[tokio::test]
async fn test_case_e_retry_after_transient_error_429() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let server = MockHttpServer::spawn(move |method, path, _| {
        if method == "GET" && path.contains("mimeType") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[{"id":"folder-root","name":"mock"}]}"#.to_string(),
            }
        } else if method == "GET" && path.contains("/files?q=") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[]}"#.to_string(),
            }
        } else if method == "POST" && path.contains("uploadType=multipart") {
            let n = attempts_clone.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // Primeira tentativa: 429
                MockResponse {
                    status: 429,
                    status_text: "Too Many Requests",
                    content_type: "application/json",
                    body: r#"{"error":{"code":429,"message":"Rate Limit Exceeded"}}"#.to_string(),
                }
            } else {
                // Segunda tentativa: 200 com drive_file_id
                MockResponse {
                    status: 200,
                    status_text: "OK",
                    content_type: "application/json",
                    body: r#"{"id":"drive-file-retry-success","name":"save.dat"}"#.to_string(),
                }
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Not Found".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("mock-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"save content").unwrap();

    // Primeira tentativa falha com 429
    let err1 = provider.upload(tmp.path(), "loc/file/save.dat").await.unwrap_err();
    assert!(err1.to_string().contains("429"), "Primeira tentativa deve falhar com 429: {}", err1);

    // Segunda tentativa (retry) sucede com 200 + drive_file_id
    let res2 = provider.upload(tmp.path(), "loc/file/save.dat").await;
    assert!(res2.is_ok(), "Segunda tentativa deve ter sucesso: {:?}", res2);
    assert_eq!(res2.unwrap(), "drive-file-retry-success");
}

#[tokio::test]
async fn test_case_f_upload_idempotency_reuses_remote_file_if_already_exists() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("mimeType") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[{"id":"folder-root","name":"mock"}]}"#.to_string(),
            }
        } else if method == "GET" && path.contains("/files?q=") {
            // O arquivo JÁ existe na pasta remota (ex: upload anterior teve sucesso mas local DB falhou)
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[{"id":"existing-remote-drive-id-888","name":"v1_save.dat"}]}"#.to_string(),
            }
        } else if method == "POST" {
            // Se chamar POST, falha o teste! A idempotência deve reutilizar o arquivo sem re-upload!
            MockResponse {
                status: 500,
                status_text: "Internal Error",
                content_type: "text/plain",
                body: "POST não deveria ter sido chamado para arquivo já existente!".to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Not Found".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("mock-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"save binary payload").unwrap();

    let res = provider.upload(tmp.path(), "location_uuid/file_uuid/v1_save.dat").await;
    assert!(res.is_ok(), "Upload idempotente deve reutilizar o arquivo: {:?}", res);
    assert_eq!(
        res.unwrap(),
        "existing-remote-drive-id-888",
        "Deve retornar o ID do arquivo remoto já existente"
    );
}

#[tokio::test]
async fn test_case_g_file_id_missing_in_response_fails_explicitly() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("mimeType") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[{"id":"folder-root","name":"mock"}]}"#.to_string(),
            }
        } else if method == "GET" && path.contains("/files?q=") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"files":[]}"#.to_string(),
            }
        } else if method == "POST" && path.contains("uploadType=multipart") {
            // Retorna HTTP 200, mas JSON SEM campo "id"
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"name":"save.dat","size":"1024"}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Not Found".to_string(),
            }
        }
    }).await;

    let auth = AuthService::with_fixed_token("mock-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"payload").unwrap();

    let err = provider.upload(tmp.path(), "loc/file/save.dat").await.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("missing field `id`") || err_str.contains("Resposta JSON inválida"),
        "Deve falhar explicitamente pela ausência do campo id na resposta do Drive: {}",
        err_str
    );
}

#[tokio::test]
async fn test_case_sec_01_opaque_drive_file_id_adversarial_rejection() {
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let req_counter = requests.clone();

    let server = MockHttpServer::spawn(move |_method, _path, _body| {
        req_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        MockResponse {
            status: 200,
            status_text: "OK",
            content_type: "application/json",
            body: "{}".to_string(),
        }
    }).await;

    let auth = AuthService::with_fixed_token("mock-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();

    let adversarial_ids = [
        "foo/bar",
        r"foo\bar",
        "../foo",
        "..",
        "...",
        "foo?alt=media",
        "foo#fragment",
        "foo&bar",
        "foo\r\nbar",
        "foo\0bar",
        "   ",
        "",
    ];

    for invalid_id in adversarial_ids {
        // Testa download
        let err_dl = provider.download(invalid_id, tmp.path()).await.unwrap_err();
        let err_dl_str = err_dl.to_string();
        assert!(
            err_dl_str.contains("Identificador remoto inválido") || err_dl_str.contains("vazio"),
            "Download com ID '{}' deve ser rejeitado sem enviar requisição: {}",
            invalid_id, err_dl_str
        );

        // Testa delete
        let err_del = provider.delete(invalid_id).await.unwrap_err();
        let err_del_str = err_del.to_string();
        assert!(
            err_del_str.contains("Identificador remoto inválido") || err_del_str.contains("vazio"),
            "Delete com ID '{}' deve ser rejeitado sem enviar requisição: {}",
            invalid_id, err_del_str
        );
    }

    // Invariante de Segurança: NENHUMA requisição deve ter chegado ao servidor remoto!
    assert_eq!(
        requests.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "IDs inválidos/adversariais NUNCA devem gerar requisições HTTP para a API remota"
    );
}

#[tokio::test]
async fn test_case_sec_02_valid_opaque_id_accepted() {
    let requested_paths = Arc::new(std::sync::Mutex::new(Vec::new()));
    let paths_clone = requested_paths.clone();

    let server = MockHttpServer::spawn(move |_method, path, _body| {
        paths_clone.lock().unwrap().push(path.to_string());
        MockResponse {
            status: 200,
            status_text: "OK",
            content_type: "application/octet-stream",
            body: "file content".to_string(),
        }
    }).await;

    let auth = AuthService::with_fixed_token("mock-token".to_string());
    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        server.base_url.clone(),
        server.base_url.clone(),
    );

    let tmp = NamedTempFile::new().unwrap();

    let valid_ids = [
        "1a2B3c-4D5_E",
        "1-a_B.c~",
        "ABCDEF1234567890",
    ];

    for valid_id in valid_ids {
        provider.download(valid_id, tmp.path()).await.unwrap();
    }

    let paths = requested_paths.lock().unwrap();
    assert_eq!(paths.len(), 3);
    assert_eq!(paths[0], "/files/1a2B3c-4D5_E?alt=media");
    assert_eq!(paths[1], "/files/1-a_B.c~?alt=media");
    assert_eq!(paths[2], "/files/ABCDEF1234567890?alt=media");
}

#[tokio::test]
async fn test_case_net_01_http_timeout_triggers_error_and_does_not_hang() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    // Servidor que aceita mas atrasa propositalmente antes de responder
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0u8; 1024];
            let _ = socket.read(&mut buf).await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
        }
    });

    let auth = AuthService::with_fixed_token("mock-token".to_string());
    let timeout_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(50))
        .build()
        .unwrap();

    let provider = GoogleDriveProvider::with_custom_endpoints(
        auth,
        base_url.clone(),
        base_url,
    ).with_client(timeout_client);

    let start = std::time::Instant::now();
    let res = provider.delete("file_123").await;
    let elapsed = start.elapsed();

    assert!(res.is_err(), "Operação com timeout estourado deve retornar erro");
    assert!(
        elapsed < std::time::Duration::from_millis(400),
        "Timeout de 50ms deve retornar rapidamente, levou {:?}",
        elapsed
    );
    let err = res.unwrap_err();
    let is_timeout = err
        .downcast_ref::<reqwest::Error>()
        .map(|e| e.is_timeout())
        .unwrap_or(false)
        || format!("{:#}", err).to_lowercase().contains("time");
    assert!(
        is_timeout,
        "Erro deve indicar timeout HTTP: {:#}",
        err
    );
}
