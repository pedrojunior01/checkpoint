//! Testes de integração e testes adversariais para o OneDriveProvider usando Mock HTTP Server.
//!
//! Cobre:
//! - Upload de snapshot com criação em approot
//! - Idempotência no upload (reuso de arquivo já existente)
//! - Download atômico com staging temporário
//! - Exclusão de itens e tolerância a 404
//! - Renovação de token OAuth (refresh_token)
//! - Cache de access token eliminando requisições redundantes
//! - Invariante de Segurança SEC-01 contra path traversal e injeção de caracteres

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use savevault_cloud::onedrive::OneDriveProvider;
use savevault_cloud::onedrive_auth::OneDriveAuthService;
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
// Testes do OneDriveProvider
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_onedrive_upload_success() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("special/approot:") {
            // Checagem de idempotência: ainda não existe
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "application/json",
                body: r#"{"error":{"code":"itemNotFound","message":"Item not found"}}"#.to_string(),
            }
        } else if method == "PUT" && path.contains("/content") {
            MockResponse {
                status: 201,
                status_text: "Created",
                content_type: "application/json",
                body: r#"{"id":"onedrive-item-999","name":"save.dat","size":128}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 400,
                status_text: "Bad Request",
                content_type: "text/plain",
                body: "Requisição inesperada".to_string(),
            }
        }
    })
    .await;

    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let provider = OneDriveProvider::with_custom_endpoints(auth, server.base_url);

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"dados_do_save_game").unwrap();

    let remote_id = provider
        .upload(tmp.path(), "game-1/file-1/v1_save.dat")
        .await
        .unwrap();

    assert_eq!(remote_id, "onedrive-item-999");
}

#[tokio::test]
async fn test_onedrive_upload_idempotency_reuses_remote_file_if_already_exists() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("special/approot:") {
            // Arquivo já existe no OneDrive
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"id":"existing-item-555","name":"v1_save.dat","size":256}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 500,
                status_text: "Internal Error",
                content_type: "text/plain",
                body: "Não deveria fazer upload se já existe".to_string(),
            }
        }
    })
    .await;

    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let provider = OneDriveProvider::with_custom_endpoints(auth, server.base_url);

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"dados_identicos").unwrap();

    let remote_id = provider
        .upload(tmp.path(), "game-1/file-1/v1_save.dat")
        .await
        .unwrap();

    assert_eq!(remote_id, "existing-item-555");
}

#[tokio::test]
async fn test_onedrive_download_success() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "GET" && path.contains("/items/item-xyz/content") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/octet-stream",
                body: "conteudo_recuperado_do_onedrive".to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: "Item não encontrado".to_string(),
            }
        }
    })
    .await;

    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let provider = OneDriveProvider::with_custom_endpoints(auth, server.base_url);

    let dir = tempfile::tempdir().unwrap();
    let dest_file = dir.path().join("restored.sav");

    provider.download("item-xyz", &dest_file).await.unwrap();

    assert!(dest_file.exists());
    let content = std::fs::read_to_string(&dest_file).unwrap();
    assert_eq!(content, "conteudo_recuperado_do_onedrive");
}

#[tokio::test]
async fn test_onedrive_delete_success() {
    let server = MockHttpServer::spawn(|method, path, _| {
        if method == "DELETE" && path.contains("/items/target-to-delete") {
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
                body: "".to_string(),
            }
        }
    })
    .await;

    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let provider = OneDriveProvider::with_custom_endpoints(auth, server.base_url);

    let result = provider.delete("target-to-delete").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_onedrive_delete_tolerates_404() {
    let server = MockHttpServer::spawn(|method, _, _| {
        if method == "DELETE" {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "application/json",
                body: r#"{"error":{"code":"itemNotFound","message":"Item not found"}}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 400,
                status_text: "Bad Request",
                content_type: "text/plain",
                body: "".to_string(),
            }
        }
    })
    .await;

    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let provider = OneDriveProvider::with_custom_endpoints(auth, server.base_url);

    // 404 deve ser tratado como sucesso idempotente (item já não existe)
    let result = provider.delete("already-gone-item").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_onedrive_token_refresh_flow() {
    let server = MockHttpServer::spawn(|method, path, req| {
        if method == "POST" && path.contains("/token") {
            assert!(req.contains("grant_type=refresh_token"));
            assert!(req.contains("refresh_token=mock-rt-123"));
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"access_token":"new-fresh-access-token","refresh_token":"rotated-rt","expires_in":3600}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 400,
                status_text: "Bad Request",
                content_type: "text/plain",
                body: "Invalid".to_string(),
            }
        }
    })
    .await;

    let auth = OneDriveAuthService::with_custom_endpoints(
        "my-client-id".to_string(),
        None,
        format!("{}/auth", server.base_url),
        format!("{}/token", server.base_url),
        8086,
    )
    .with_mock_refresh_token("mock-rt-123".to_string());

    let token = auth.get_fresh_access_token().await.unwrap();
    assert_eq!(token, "new-fresh-access-token");
}

#[tokio::test]
async fn test_onedrive_access_token_caching_eliminates_redundant_refreshes() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    let server = MockHttpServer::spawn(move |method, path, _| {
        if method == "POST" && path.contains("/token") {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"access_token":"cached-token-abc","expires_in":3600}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 400,
                status_text: "Bad Request",
                content_type: "text/plain",
                body: "".to_string(),
            }
        }
    })
    .await;

    let auth = OneDriveAuthService::with_custom_endpoints(
        "my-client-id".to_string(),
        None,
        format!("{}/auth", server.base_url),
        format!("{}/token", server.base_url),
        8086,
    )
    .with_mock_refresh_token("mock-rt-123".to_string());

    // 1ª chamada: vai na rede
    let t1 = auth.get_fresh_access_token().await.unwrap();
    assert_eq!(t1, "cached-token-abc");
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // 2ª chamada: reutiliza o cache em memória sem chamada de rede
    let t2 = auth.get_fresh_access_token().await.unwrap();
    assert_eq!(t2, "cached-token-abc");
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "Segunda chamada deve vir do cache em memória"
    );
}

#[tokio::test]
async fn test_onedrive_sec_01_opaque_id_adversarial_rejection() {
    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let provider = OneDriveProvider::with_custom_endpoints(auth, "http://localhost".to_string());

    let malicious_ids = [
        "../path/traversal",
        r"..\..\windows\system32",
        "item/subitem",
        r"item\subitem",
        "item?param=injected",
        "item#fragment",
        "item&foo=bar",
        "   ",
        "",
        "item with spaces",
        "item\x00null",
    ];

    for id in malicious_ids {
        assert!(
            provider
                .download(id, std::path::Path::new("target"))
                .await
                .is_err(),
            "ID malicioso '{}' deveria ser rejeitado pelo download",
            id
        );
        assert!(
            provider.delete(id).await.is_err(),
            "ID malicioso '{}' deveria ser rejeitado pelo delete",
            id
        );
    }
}

#[tokio::test]
async fn test_onedrive_sec_02_valid_opaque_id_accepted() {
    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let provider =
        OneDriveProvider::with_custom_endpoints(auth, "http://localhost:8080".to_string());

    let valid_ids = [
        "01BY27U2DWNPBYA7N225A625V5N2I2T6F2",
        "item-123-abc_XYZ.456",
        "A1B2C3D4",
    ];

    for id in valid_ids {
        let url = provider.build_item_url(id, Some("content")).unwrap();
        assert!(url.as_str().contains(id));
    }
}

#[tokio::test]
async fn test_onedrive_upload_401_unauthorized() {
    let server = MockHttpServer::spawn(|_, _, _| MockResponse {
        status: 401,
        status_text: "Unauthorized",
        content_type: "application/json",
        body: r#"{"error":{"code":"unauthenticated","message":"Token expired or invalid"}}"#
            .to_string(),
    })
    .await;

    let auth = OneDriveAuthService::with_fixed_token("expired-token".to_string());
    let provider = OneDriveProvider::with_custom_endpoints(auth, server.base_url);

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"data").unwrap();

    let result = provider.upload(tmp.path(), "game/file/save.dat").await;
    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("HTTP 401")
            || err_str.contains("unauthenticated")
            || err_str.contains("Token expired")
    );
}

#[tokio::test]
async fn test_onedrive_timeout_handling() {
    let server = MockHttpServer::spawn(|_, _, _| {
        // Simula travamento/delay infinito
        std::thread::sleep(Duration::from_millis(500));
        MockResponse {
            status: 200,
            status_text: "OK",
            content_type: "text/plain",
            body: "Too late".to_string(),
        }
    })
    .await;

    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap();

    let provider =
        OneDriveProvider::with_custom_endpoints(auth, server.base_url).with_client(client);

    let tmp = NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"data").unwrap();

    let result = provider.upload(tmp.path(), "game/file/save.dat").await;
    assert!(
        result.is_err(),
        "Requisição que excede o timeout deve falhar explicitamente"
    );
}

#[tokio::test]
async fn test_contract_onedrive_rejects_manifest_filename_as_file_id() {
    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let provider = OneDriveProvider::with_custom_endpoints(auth, "http://127.0.0.1:9".to_string());

    let tmp = NamedTempFile::new().unwrap();
    let res = provider
        .download("savevault_manifest.json", tmp.path())
        .await;

    assert!(
        matches!(res, Err(savevault_cloud::CloudError::InvalidRemoteId(_))),
        "download() com '.json' deve ser rejeitado estritamente como InvalidRemoteId no OneDrive, obtido: {:?}",
        res
    );
}

#[tokio::test]
async fn test_contract_onedrive_manifest_lifecycle() {
    let manifest_saved = Arc::new(std::sync::Mutex::new(None::<String>));
    let manifest_clone = manifest_saved.clone();

    let server = MockHttpServer::spawn(move |method, path, req| {
        // 1. Salvar manifesto via PUT
        if method == "PUT" && path.contains("approot:/savevault_manifest.json:/content") {
            *manifest_clone.lock().unwrap() = Some(req.to_string());
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"id":"onedrive_manifest_item_id_888","name":"savevault_manifest.json"}"#.to_string(),
            }
        }
        // 2. Download do manifesto via GET /content
        else if method == "GET" && path.contains("approot:/savevault_manifest.json:/content") {
            let cur = manifest_clone.lock().unwrap().clone();
            if cur.is_some() {
                let json = serde_json::json!({
                    "schema_version": 1,
                    "updated_at": "2026-10-02T12:00:00Z",
                    "entries": [
                        {
                            "game_name": "Cyberpunk 2077",
                            "platform": "steam",
                            "save_location_path": "C:\\Saves",
                            "relative_file_path": "manualsave.dat",
                            "file_hash": "cp77hash",
                            "version": 1,
                            "cloud_file_id": "onedrive_item_cp77",
                            "cloud_path": "loc/file/v1_manualsave.dat",
                            "created_at": "2026-10-02T12:00:00Z"
                        }
                    ]
                });
                MockResponse {
                    status: 200,
                    status_text: "OK",
                    content_type: "application/json",
                    body: json.to_string(),
                }
            } else {
                MockResponse {
                    status: 404,
                    status_text: "Not Found",
                    content_type: "application/json",
                    body: r#"{"error":{"code":"itemNotFound","message":"Item not found"}}"#.to_string(),
                }
            }
        }
        // 3. Busca de item pelo path/nome
        else if method == "GET" && path.contains("approot:/savevault_manifest.json:") {
            MockResponse {
                status: 200,
                status_text: "OK",
                content_type: "application/json",
                body: r#"{"id":"onedrive_manifest_item_id_888","name":"savevault_manifest.json","size":512}"#.to_string(),
            }
        } else {
            MockResponse {
                status: 404,
                status_text: "Not Found",
                content_type: "text/plain",
                body: format!("Not Found: {} {}", method, path),
            }
        }
    })
    .await;

    let auth = OneDriveAuthService::with_fixed_token("fake-token".to_string());
    let provider = OneDriveProvider::with_custom_endpoints(auth, server.base_url);

    // 1. Antes de salvar, manifesto não existe (retorna None)
    let initial = provider.get_manifest().await.unwrap();
    assert_eq!(initial, None);

    // 2. Salva manifesto
    let manifest = savevault_cloud::CloudCatalogManifest {
        schema_version: 1,
        updated_at: "2026-10-02T12:00:00Z".to_string(),
        entries: vec![],
    };
    provider
        .save_manifest(&manifest)
        .await
        .expect("save_manifest deve salvar via PUT no approot");

    // 3. Busca arquivo pelo nome no approot
    let found = provider
        .find_file_by_name("savevault_manifest.json", None)
        .await
        .unwrap()
        .expect("find_file_by_name deve encontrar no approot");
    assert_eq!(found.id, "onedrive_manifest_item_id_888");

    // 4. Download e parse do manifesto
    let loaded = provider
        .get_manifest()
        .await
        .unwrap()
        .expect("get_manifest deve carregar manifesto");
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].game_name, "Cyberpunk 2077");
}
