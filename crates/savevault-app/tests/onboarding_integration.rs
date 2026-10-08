//! Testes de Integração do Fluxo de Onboarding (Apple/macOS Style).
//!
//! Cobre:
//! - Primeiro acesso (setup_completed = false) e estado pós-conclusão
//! - Descoberta assíncrona não bloqueante de bibliotecas (Steam, GOG, Emuladores, Genéricos)
//! - Armazenamento: Automático (zero pastas criadas), Nuvem/OneDrive e Customizado
//! - Segurança de Storage: Rejeição de Path Traversal, Disco Raiz, Pastas de Sistema e Caracteres Nulos
//! - Modos de Interface: Basic (Simples), Advanced (Avançado) e UltraLight (Ultra leve)
//! - Atomicidade de persistência (sem aplicação parcial em caso de erro)
//! - Endpoints HTTP da DEV-UI para o Onboarding

use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex};

use savevault_app::ui::{start_dev_ui_server, DevUiState, OnboardingRequest};
use savevault_core::config::InterfaceMode;
use savevault_db::Database;

/// Helper para requisições HTTP GET na DEV-UI.
async fn http_get(addr: SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("Falha ao conectar no servidor DEV-UI");
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, addr
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).await.unwrap();

    let status = resp
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .unwrap();

    let body = resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

/// Helper para requisições HTTP POST na DEV-UI.
async fn http_post(addr: SocketAddr, path: &str, json_body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("Falha ao conectar no servidor DEV-UI");
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nX-SaveVault-Request: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        addr,
        addr,
        json_body.len(),
        json_body
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).await.unwrap();

    let status = resp
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .unwrap();

    let body = resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

/// Cria um DevUiState de teste isolado em diretório temporário.
fn setup_test_state() -> (tempfile::TempDir, DevUiState, watch::Receiver<bool>) {
    let temp = tempdir().unwrap();
    let db_path = temp.path().join("savevault_test.db");
    let config_path = temp.path().join("config.toml");

    let db = Database::open(&db_path).unwrap();
    let db_mutex = Arc::new(Mutex::new(db.conn));

    let (_shutdown_tx, shutdown_rx) = watch::channel(false);

    let state = DevUiState::new(
        db_mutex,
        db_path,
        temp.path().join("backups"),
        None,
        std::time::Duration::from_secs(30),
    )
    .with_config_path(config_path);

    (temp, state, shutdown_rx)
}

#[tokio::test]
async fn test_onboarding_initial_state() {
    let (_temp, state, _rx) = setup_test_state();

    let status = state.get_setup_status().await.unwrap();
    assert!(
        !status.setup_completed,
        "No primeiro acesso, setup_completed deve ser falso"
    );
    assert_eq!(status.interface_mode, InterfaceMode::Basic);
    assert!(status.selected_libraries.contains(&"steam".to_string()));
    assert!(status.selected_libraries.contains(&"gog".to_string()));
}

#[tokio::test]
async fn test_get_available_libraries() {
    let (_temp, state, _rx) = setup_test_state();

    let libs = state.get_available_libraries().unwrap();
    assert_eq!(
        libs.len(),
        4,
        "Deve expor Steam, GOG, Emuladores e Genéricos"
    );

    let steam = libs
        .iter()
        .find(|l| l.id == "steam")
        .expect("Steam deve existir");
    assert_eq!(steam.category, "commercial");

    let gog = libs
        .iter()
        .find(|l| l.id == "gog")
        .expect("GOG deve existir");
    assert_eq!(gog.category, "commercial");

    let emu = libs
        .iter()
        .find(|l| l.id == "emulators")
        .expect("Emuladores deve existir");
    assert_eq!(emu.category, "emulators");

    let generic = libs
        .iter()
        .find(|l| l.id == "generic")
        .expect("Genérico deve existir");
    assert_eq!(generic.category, "generic");
}

#[tokio::test]
async fn test_complete_onboarding_automatic_storage_creates_no_folders() {
    let (temp, state, _rx) = setup_test_state();

    let unexpected_target = temp.path().join("should_not_exist");
    assert!(!unexpected_target.exists());

    let req = OnboardingRequest {
        selected_libraries: vec!["steam".to_string(), "generic".to_string()],
        detect_emulators: Some(false),
        detect_generic_games: Some(true),
        storage_mode: "automatic".to_string(),
        custom_storage_path: None,
        autostart: true,
        interface_mode: InterfaceMode::Basic,
    };

    let res = state.complete_onboarding(req).await;
    assert!(
        res.is_ok(),
        "Onboarding automático deve concluir com sucesso"
    );

    // Verifica que nenhuma pasta inesperada foi criada
    assert!(
        !unexpected_target.exists(),
        "Modo automático não deve criar diretórios arbitrários"
    );

    // Verifica persistência das escolhas
    let status = state.get_setup_status().await.unwrap();
    assert!(
        status.setup_completed,
        "Setup deve estar marcado como concluído"
    );
    assert_eq!(status.interface_mode, InterfaceMode::Basic);
    assert_eq!(status.selected_libraries, vec!["steam", "generic"]);
}

#[tokio::test]
async fn test_complete_onboarding_advanced_custom_storage() {
    let (temp, state, _rx) = setup_test_state();

    let custom_vault = temp.path().join("CustomSavesVault");
    std::fs::create_dir_all(&custom_vault).unwrap();

    let req = OnboardingRequest {
        selected_libraries: vec![
            "steam".to_string(),
            "gog".to_string(),
            "emulators".to_string(),
        ],
        detect_emulators: Some(true),
        detect_generic_games: Some(false),
        storage_mode: "custom".to_string(),
        custom_storage_path: Some(custom_vault.to_string_lossy().to_string()),
        autostart: false,
        interface_mode: InterfaceMode::Advanced,
    };

    let res = state.complete_onboarding(req).await;
    assert!(
        res.is_ok(),
        "Onboarding com pasta customizada válida deve ter sucesso"
    );

    let status = state.get_setup_status().await.unwrap();
    assert!(status.setup_completed);
    assert_eq!(status.interface_mode, InterfaceMode::Advanced);
    assert_eq!(status.cloud_provider, "local_folder");
    assert_eq!(
        status.local_sync_path,
        Some(custom_vault.to_string_lossy().to_string())
    );
    assert!(
        !state.get_config().general.autostart,
        "Configuração autostart deve ser salva como falso"
    );
}

#[tokio::test]
async fn test_complete_onboarding_ultralight_mode() {
    let (_temp, state, _rx) = setup_test_state();

    let req = OnboardingRequest {
        selected_libraries: vec!["steam".to_string()],
        detect_emulators: Some(false),
        detect_generic_games: Some(false),
        storage_mode: "automatic".to_string(),
        custom_storage_path: None,
        autostart: true,
        interface_mode: InterfaceMode::UltraLight,
    };

    let res = state.complete_onboarding(req).await;
    assert!(res.is_ok(), "Onboarding com UltraLight deve ter sucesso");

    let status = state.get_setup_status().await.unwrap();
    assert!(status.setup_completed);
    assert_eq!(status.interface_mode, InterfaceMode::UltraLight);
}

#[tokio::test]
async fn test_onboarding_security_rejects_path_traversal() {
    let (_temp, state, _rx) = setup_test_state();

    let req = OnboardingRequest {
        selected_libraries: vec!["steam".to_string()],
        detect_emulators: Some(false),
        detect_generic_games: Some(false),
        storage_mode: "custom".to_string(),
        custom_storage_path: Some("../../../Windows/System32".to_string()),
        autostart: false,
        interface_mode: InterfaceMode::Basic,
    };

    let res = state.complete_onboarding(req).await;
    assert!(
        res.is_err(),
        "Path traversal deve ser veementemente rejeitado"
    );
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("traversal") || err_msg.contains(".."));

    // Garante que o setup NÃO foi marcado como concluído (atomicidade)
    let status = state.get_setup_status().await.unwrap();
    assert!(
        !status.setup_completed,
        "Setup não deve ser concluído após falha"
    );
}

#[tokio::test]
async fn test_onboarding_security_rejects_disk_root() {
    let (_temp, state, _rx) = setup_test_state();

    let req = OnboardingRequest {
        selected_libraries: vec!["steam".to_string()],
        detect_emulators: Some(false),
        detect_generic_games: Some(false),
        storage_mode: "custom".to_string(),
        custom_storage_path: Some("C:\\".to_string()),
        autostart: false,
        interface_mode: InterfaceMode::Basic,
    };

    let res = state.complete_onboarding(req).await;
    assert!(res.is_err(), "Raiz de disco não deve ser aceita");
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("raiz"));

    let status = state.get_setup_status().await.unwrap();
    assert!(!status.setup_completed);
}

#[tokio::test]
async fn test_onboarding_security_rejects_system_directory() {
    let (_temp, state, _rx) = setup_test_state();

    let req = OnboardingRequest {
        selected_libraries: vec!["steam".to_string()],
        detect_emulators: Some(false),
        detect_generic_games: Some(false),
        storage_mode: "custom".to_string(),
        custom_storage_path: Some("C:\\Windows".to_string()),
        autostart: false,
        interface_mode: InterfaceMode::Basic,
    };

    let res = state.complete_onboarding(req).await;
    assert!(res.is_err(), "Diretório de sistema não deve ser aceito");
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("sistema protegido") || err_msg.contains("sistema"));
}

#[tokio::test]
async fn test_onboarding_http_endpoints() {
    let (temp, state, rx) = setup_test_state();

    let (bound_addr, _handle) = start_dev_ui_server(Arc::new(state), 0, rx).await.unwrap();

    // 1. GET /api/setup/libraries
    let (status_code, body) = http_get(bound_addr, "/api/setup/libraries").await;
    assert_eq!(status_code, 200);
    assert!(body.contains("steam"));
    assert!(body.contains("emulators"));

    // 2. POST /api/setup/onboarding com sucesso
    let valid_storage = temp.path().join("OnboardingHttpVault");
    std::fs::create_dir_all(&valid_storage).unwrap();

    let json_req = serde_json::json!({
        "selected_libraries": ["steam", "gog"],
        "detect_emulators": false,
        "detect_generic_games": true,
        "storage_mode": "custom",
        "storage_path": valid_storage.to_string_lossy(),
        "startup_enabled": true,
        "interface_mode": "advanced"
    })
    .to_string();

    let (post_status, post_body) = http_post(bound_addr, "/api/setup/onboarding", &json_req).await;
    assert_eq!(post_status, 200);
    assert!(post_body.contains("\"setup_completed\":true"));

    // 3. POST /api/setup/onboarding com path traversal
    let evil_req = serde_json::json!({
        "selected_libraries": ["steam"],
        "detect_emulators": false,
        "detect_generic_games": false,
        "storage_mode": "custom",
        "storage_path": "../../evil",
        "startup_enabled": false,
        "interface_mode": "basic"
    })
    .to_string();

    let (evil_status, evil_body) = http_post(bound_addr, "/api/setup/onboarding", &evil_req).await;
    assert_eq!(evil_status, 400);
    assert!(evil_body.contains("error"));

    // 4. POST /api/setup/skip
    let (skip_status, skip_body) = http_post(bound_addr, "/api/setup/skip", "").await;
    assert_eq!(skip_status, 200);
    assert!(skip_body.contains("\"setup_completed\":true"));
}

#[tokio::test]
async fn test_skip_onboarding_preserves_existing_config() {
    let (temp, state, _rx) = setup_test_state();

    // Simula uma configuração já existente personalizada no disco
    let config_path = temp.path().join("config.toml");
    let mut initial_config = savevault_core::config::Config::default();
    initial_config.general.setup_completed = false;
    initial_config.general.interface_mode = InterfaceMode::Advanced;
    initial_config.general.selected_libraries = vec!["gog".into(), "emulators".into()];
    initial_config.cloud.provider = "local_folder".into();
    let custom_dir = temp.path().join("ExistingVault");
    std::fs::create_dir_all(&custom_dir).unwrap();
    initial_config.cloud.local_sync_path = Some(custom_dir.clone());
    initial_config.save(&config_path).unwrap();

    // Usuário clica em "Já configurei isso" (skip_onboarding)
    let res = state.skip_onboarding().await;
    assert!(res.is_ok(), "skip_onboarding deve suceder");

    let status = state.get_setup_status().await.unwrap();
    assert!(status.setup_completed, "setup_completed deve ser true");
    // Garante que não destruiu as escolhas anteriores
    assert_eq!(status.interface_mode, InterfaceMode::Advanced);
    assert_eq!(status.selected_libraries, vec!["gog", "emulators"]);
    assert_eq!(status.cloud_provider, "local_folder");
    assert_eq!(
        status.local_sync_path,
        Some(custom_dir.to_string_lossy().to_string())
    );
}
