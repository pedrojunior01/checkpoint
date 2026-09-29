//! Testes de Integração do Cache de Manifesto Ludusavi (SV-ADV-005).
//!
//! Valida que respostas HTTP inválidas (429, 503, erros de rede), YAMLs malformados
//! ou payloads sem estrutura de manifesto válida jamais sobrescrevem ou envenenam o cache local.

use std::path::Path;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use savevault_discovery::manifest::{get_manifest_path_with_options, validate_manifest_content};

/// Inicializa um servidor HTTP real local em porta dinâmica para testar cenários adversariais de rede.
async fn spawn_mock_http_server(
    status_line: &'static str,
    headers: &'static str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Falha ao ligar TcpListener em porta dinâmica");
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}/manifest.yaml", addr);

    let handle = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 {}\r\n{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_line,
                headers,
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
        }
    });

    (url, handle)
}

const OLD_VALID_MANIFEST: &str = r#""Original Protected Game":
  files:
    "<home>/Documents/My Games/Original/save.dat":
      tags:
        - save
  steam:
    id: 111111
"#;

const NEW_VALID_MANIFEST: &str = r#""New Updated Game":
  files:
    "<home>/Saved Games/New/slot1.sav":
      tags:
        - save
  steam:
    id: 222222
"#;

fn setup_old_cache(cache_dir: &Path) {
    let manifest_path = cache_dir.join("manifest.yaml");
    std::fs::write(&manifest_path, OLD_VALID_MANIFEST).expect("Falha ao escrever cache inicial");
}

#[tokio::test]
async fn test_manifest_cache_http_200_valid_yaml_updates_cache() {
    let dir = tempdir().unwrap();
    setup_old_cache(dir.path());

    let (url, _server) =
        spawn_mock_http_server("200 OK", "Content-Type: text/yaml", NEW_VALID_MANIFEST).await;

    // max_age_secs = 0 força tentativa de atualização do cache
    let res = get_manifest_path_with_options(dir.path(), &url, 0).await;
    assert!(
        res.is_ok(),
        "Deve atualizar com sucesso com HTTP 200 e YAML válido"
    );

    let saved_content = std::fs::read_to_string(dir.path().join("manifest.yaml")).unwrap();
    assert!(
        saved_content.contains("New Updated Game"),
        "O cache deve ter sido atualizado com o novo conteúdo"
    );
    assert!(
        !saved_content.contains("Original Protected Game"),
        "O cache antigo deve ter sido substituído"
    );
}

#[tokio::test]
async fn test_manifest_cache_http_429_preserves_old_cache() {
    let dir = tempdir().unwrap();
    setup_old_cache(dir.path());

    let (url, _server) = spawn_mock_http_server(
        "429 Too Many Requests",
        "Content-Type: text/plain",
        "Rate limit exceeded. Try again in 60 seconds.",
    )
    .await;

    let res = get_manifest_path_with_options(dir.path(), &url, 0).await;
    assert!(
        res.is_ok(),
        "Deve fazer fallback gracioso para o cache anterior"
    );

    let saved_content = std::fs::read_to_string(dir.path().join("manifest.yaml")).unwrap();
    assert!(
        saved_content.contains("Original Protected Game"),
        "O cache antigo DEVE ser preservado intacto em caso de HTTP 429"
    );
    assert!(
        !saved_content.contains("Rate limit"),
        "A mensagem de erro HTTP NÃO pode ter envenenado o arquivo de cache"
    );
}

#[tokio::test]
async fn test_manifest_cache_http_503_preserves_old_cache() {
    let dir = tempdir().unwrap();
    setup_old_cache(dir.path());

    let (url, _server) = spawn_mock_http_server(
        "503 Service Unavailable",
        "Content-Type: text/html",
        "<html><body><h1>503 Service Unavailable</h1></body></html>",
    )
    .await;

    let res = get_manifest_path_with_options(dir.path(), &url, 0).await;
    assert!(
        res.is_ok(),
        "Deve fazer fallback gracioso para o cache anterior"
    );

    let saved_content = std::fs::read_to_string(dir.path().join("manifest.yaml")).unwrap();
    assert!(
        saved_content.contains("Original Protected Game"),
        "O cache antigo DEVE ser preservado intacto em caso de HTTP 503"
    );
    assert!(
        !saved_content.contains("503 Service Unavailable"),
        "O HTML de erro 503 NÃO pode ser salvo no cache"
    );
}

#[tokio::test]
async fn test_manifest_cache_http_200_invalid_yaml_preserves_old_cache() {
    let dir = tempdir().unwrap();
    setup_old_cache(dir.path());

    let (url, _server) = spawn_mock_http_server(
        "200 OK",
        "Content-Type: text/yaml",
        "corrupted: [yaml: unclosed bracket - {bad_indent",
    )
    .await;

    let res = get_manifest_path_with_options(dir.path(), &url, 0).await;
    assert!(
        res.is_ok(),
        "Deve fazer fallback gracioso para o cache anterior"
    );

    let saved_content = std::fs::read_to_string(dir.path().join("manifest.yaml")).unwrap();
    assert!(
        saved_content.contains("Original Protected Game"),
        "O cache antigo DEVE ser preservado intacto quando o YAML baixado for inválido"
    );
    assert!(
        !saved_content.contains("corrupted"),
        "YAML corrompido não pode substituir o cache válido"
    );
}

#[tokio::test]
async fn test_manifest_cache_http_200_invalid_structure_preserves_old_cache() {
    let dir = tempdir().unwrap();
    setup_old_cache(dir.path());

    // Payload com status 200 mas corpo de erro JSON do GitHub sem dados de jogos
    let (url, _server) = spawn_mock_http_server(
        "200 OK",
        "Content-Type: application/json",
        r#"{"message": "API rate limit exceeded", "documentation_url": "https://docs.github.com"}"#,
    )
    .await;

    let res = get_manifest_path_with_options(dir.path(), &url, 0).await;
    assert!(
        res.is_ok(),
        "Deve fazer fallback gracioso para o cache anterior"
    );

    let saved_content = std::fs::read_to_string(dir.path().join("manifest.yaml")).unwrap();
    assert!(
        saved_content.contains("Original Protected Game"),
        "O cache antigo DEVE ser preservado intacto quando a estrutura não tiver jogos válidos"
    );
    assert!(
        !saved_content.contains("documentation_url"),
        "JSON de erro da API não pode substituir o cache de manifesto"
    );
}

#[tokio::test]
async fn test_manifest_cache_connection_failure_preserves_old_cache() {
    let dir = tempdir().unwrap();
    setup_old_cache(dir.path());

    // Conexão em porta onde nenhum serviço escuta
    let invalid_url = "http://127.0.0.1:1/manifest.yaml";

    let res = get_manifest_path_with_options(dir.path(), invalid_url, 0).await;
    assert!(
        res.is_ok(),
        "Deve fazer fallback gracioso para o cache anterior"
    );

    let saved_content = std::fs::read_to_string(dir.path().join("manifest.yaml")).unwrap();
    assert!(
        saved_content.contains("Original Protected Game"),
        "O cache antigo DEVE ser preservado intacto em caso de falha de conexão"
    );
}

#[tokio::test]
async fn test_manifest_cache_initial_download_fails_without_cache() {
    let dir = tempdir().unwrap();
    // Diretório vazio, sem nenhum cache pré-existente

    let (url, _server) = spawn_mock_http_server(
        "500 Internal Server Error",
        "Content-Type: text/plain",
        "Internal Server Error",
    )
    .await;

    let res = get_manifest_path_with_options(dir.path(), &url, 0).await;
    assert!(
        res.is_err(),
        "Sem cache prévio, uma falha de rede/HTTP deve retornar erro explícito"
    );
    assert!(
        !dir.path().join("manifest.yaml").exists(),
        "Nenhum arquivo manifest.yaml deve ser criado em caso de erro"
    );
}

#[test]
fn test_validate_manifest_content_rejects_empty_or_malformed() {
    assert!(
        validate_manifest_content(b"").is_err(),
        "Vazio deve ser rejeitado"
    );
    assert!(
        validate_manifest_content(b"[]").is_err(),
        "Array vazio deve ser rejeitado"
    );
    assert!(
        validate_manifest_content(b"Simple scalar string").is_err(),
        "Escalar deve ser rejeitado"
    );
    assert!(
        validate_manifest_content(b"message: Not Found").is_err(),
        "Mapa sem definições de jogos deve ser rejeitado"
    );
    assert!(
        validate_manifest_content(NEW_VALID_MANIFEST.as_bytes()).is_ok(),
        "Manifesto válido deve ser aceito"
    );
}
