//! Autenticação OAuth 2.0 para o Google Drive.
//!
//! Implementa o fluxo "Authorization Code with PKCE via Loopback IP" recomendado
//! pelo Google para aplicações desktop:
//!
//! 1. Inicia um servidor TCP local em 127.0.0.1:8085.
//! 2. Abre a URL de consentimento no navegador padrão do Windows.
//! 3. O Google redireciona para http://127.0.0.1:8085/oauth/callback?code=...
//! 4. Troca o `code` por access_token + refresh_token.
//! 5. Persiste o refresh_token de forma segura no Windows Credential Manager.

use anyhow::{anyhow, Result};
use keyring::Entry;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

/// Timeout padrão para handshake de conexão OAuth.
pub const AUTH_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout padrão para requisição de token OAuth.
pub const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn build_auth_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(AUTH_CONNECT_TIMEOUT)
        .timeout(AUTH_REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Nome do serviço registrado no Windows Credential Manager.
const KEYRING_SERVICE: &str = "SaveVault";
/// Chave do refresh token do Google Drive.
const KEYRING_GOOGLE_REFRESH: &str = "google_drive_refresh_token";
/// Escopo de acesso: apenas arquivos criados pelo próprio SaveVault.
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
/// Porta do servidor local para receber o callback OAuth.
const REDIRECT_PORT: u16 = 8085;

// ---------------------------------------------------------------------------
use sha2::{Digest, Sha256};

/// Codificação Base64URL sem padding (RFC 7636).
pub fn base64url_encode(bytes: &[u8]) -> String {
    const URL_SAFE_CHARS: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut res = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };

        res.push(URL_SAFE_CHARS[(b0 >> 2) as usize] as char);
        res.push(URL_SAFE_CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            res.push(URL_SAFE_CHARS[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        }
        if i + 2 < bytes.len() {
            res.push(URL_SAFE_CHARS[(b2 & 0x3F) as usize] as char);
        }
        i += 3;
    }
    res
}

/// Gera o par de chaves PKCE (code_verifier, code_challenge) com SHA-256 (RFC 7636).
pub fn generate_pkce_pair() -> (String, String) {
    let mut random_bytes = [0u8; 32];
    for chunk in random_bytes.chunks_mut(16) {
        let u = uuid::Uuid::new_v4();
        chunk.copy_from_slice(&u.into_bytes()[..chunk.len()]);
    }
    let verifier = base64url_encode(&random_bytes);
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = base64url_encode(&hash);
    (verifier, challenge)
}

/// Gera um parâmetro `state` criptograficamente imprevisível para mitigar ataques CSRF no OAuth 2.0.
pub fn generate_oauth_state() -> String {
    let u1 = uuid::Uuid::new_v4().simple().to_string();
    let u2 = uuid::Uuid::new_v4().simple().to_string();
    format!("{}{}", u1, u2)
}

/// Valida uma requisição de callback HTTP extraindo o código e comprovando a correspondência do `state`.
pub fn validate_callback_request(raw_request: &str, expected_state: &str) -> Result<String> {
    let first_line = raw_request
        .lines()
        .next()
        .ok_or_else(|| anyhow!("Requisição HTTP de callback vazia"))?;
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow!("Requisição HTTP de callback malformada"))?;

    if !path.starts_with("/oauth/callback") {
        return Err(anyhow!("Endpoint de callback inválido: {}", path));
    }

    let state = extract_query_param(raw_request, "state")
        .ok_or_else(|| anyhow!("Parâmetro 'state' ausente no callback OAuth (possível CSRF)"))?;

    if state != expected_state {
        return Err(anyhow!(
            "OAuth state mismatch detectado! Esperado: '{}', Recebido: '{}'. Abortando por segurança (CSRF)",
            expected_state,
            state
        ));
    }

    let code = extract_query_param(raw_request, "code").ok_or_else(|| {
        let error =
            extract_query_param(raw_request, "error").unwrap_or_else(|| "desconhecido".to_string());
        anyhow!("Autorização recusada pelo provedor OAuth: {}", error)
    })?;

    Ok(code)
}

// ---------------------------------------------------------------------------
// Tipos internos de resposta da API do Google
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[allow(dead_code)]
    expires_in: u64,
}

// ---------------------------------------------------------------------------
// AuthService
// ---------------------------------------------------------------------------

/// Serviço de autenticação OAuth 2.0 para o Google Drive.
#[derive(Clone)]
pub struct AuthService {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
    token_url: String,
    fixed_token: Option<String>,
    mock_refresh_token: Option<String>,
}

impl std::fmt::Debug for AuthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthService")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .field("token_url", &self.token_url)
            .field(
                "fixed_token",
                &self.fixed_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "mock_refresh_token",
                &self.mock_refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl AuthService {
    /// Cria uma nova instância com as credenciais do Google Cloud Console.
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
            http: build_auth_http_client(),
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            fixed_token: None,
            mock_refresh_token: None,
        }
    }

    /// Cria uma instância para testes com token fixo injetado.
    #[doc(hidden)]
    pub fn with_fixed_token(token: String) -> Self {
        Self {
            client_id: "test-client".to_string(),
            client_secret: "test-secret".to_string(),
            http: build_auth_http_client(),
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            fixed_token: Some(token),
            mock_refresh_token: None,
        }
    }

    /// Cria uma instância para testes de refresh contra um endpoint customizado.
    #[doc(hidden)]
    pub fn with_mock_refresh(
        client_id: String,
        client_secret: String,
        token_url: String,
        refresh_token: String,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            http: build_auth_http_client(),
            token_url,
            fixed_token: None,
            mock_refresh_token: Some(refresh_token),
        }
    }

    /// Verifica se já existe um refresh token salvo (ou se foi injetado para testes).
    pub fn is_authenticated(&self) -> bool {
        if self.fixed_token.is_some() || self.mock_refresh_token.is_some() {
            return true;
        }
        Entry::new(KEYRING_SERVICE, KEYRING_GOOGLE_REFRESH)
            .and_then(|e| e.get_password())
            .is_ok()
    }

    /// Inicia o fluxo interativo de login:
    /// - Gera credenciais PKCE (RFC 7636) e parâmetro `state` anti-CSRF.
    /// - Abre o navegador com os desafios criptográficos.
    /// - Aguarda e valida o callback OAuth.
    /// - Salva o refresh token de forma segura.
    pub async fn login_interactive(&self) -> Result<()> {
        let (code_verifier, code_challenge) = generate_pkce_pair();
        let state = generate_oauth_state();

        let redirect_uri = format!("http://127.0.0.1:{}/oauth/callback", REDIRECT_PORT);
        let auth_url = format!(
            "https://accounts.google.com/o/oauth2/v2/auth\
            ?client_id={}\
            &redirect_uri={}\
            &response_type=code\
            &scope={}\
            &access_type=offline\
            &prompt=consent\
            &state={}\
            &code_challenge={}\
            &code_challenge_method=S256",
            self.client_id,
            urlencoding_simple(&redirect_uri),
            urlencoding_simple(DRIVE_SCOPE),
            urlencoding_simple(&state),
            urlencoding_simple(&code_challenge),
        );

        // Inicia o listener antes de abrir o navegador para não perder o callback
        let listener = TcpListener::bind(format!("127.0.0.1:{}", REDIRECT_PORT))
            .await
            .map_err(|e| {
                anyhow!(
                    "Falha ao abrir porta {}. Verifique se está em uso: {}",
                    REDIRECT_PORT,
                    e
                )
            })?;

        info!("Aguardando callback do Google na porta {}", REDIRECT_PORT);
        println!("\nAbrindo navegador para autenticação no Google Drive...");
        println!(
            "Caso o navegador não abra, acesse manualmente:\n{}\n",
            auth_url
        );

        // Abre a URL no navegador padrão do Windows
        open_url_windows(&auth_url);

        // Aguarda a conexão do navegador com o callback
        let (mut socket, addr) = listener.accept().await?;
        debug!("Conexão recebida de {}", addr);

        let mut buffer = vec![0u8; 4096];
        let n = socket.read(&mut buffer).await?;
        let request = String::from_utf8_lossy(&buffer[..n]);

        // Valida o callback, o endpoint e o parâmetro state anti-CSRF
        let code = match validate_callback_request(&request, &state) {
            Ok(c) => c,
            Err(e) => {
                let err_html = format!(
                    "HTTP/1.1 403 Forbidden\r\n\
                    Content-Type: text/html; charset=utf-8\r\n\
                    Connection: close\r\n\r\n\
                    <!DOCTYPE html><html><head><meta charset='utf-8'><title>SaveVault - Erro de Autenticação</title></head>\
                    <body style='font-family:sans-serif;padding:2rem'>\
                    <h2>❌ Falha de Segurança no OAuth</h2>\
                    <p>A solicitação de autenticação foi rejeitada por falha de integridade (CSRF ou state inválido).</p>\
                    <p style='color:red'>{}</p>\
                    </body></html>",
                    e
                );
                let _ = socket.write_all(err_html.as_bytes()).await;
                return Err(e);
            }
        };

        // Resposta visual na aba do navegador
        let html = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "Connection: close\r\n\r\n",
            "<!DOCTYPE html><html><head><meta charset='utf-8'>",
            "<title>SaveVault</title></head><body style='font-family:sans-serif;padding:2rem'>",
            "<h2>✅ SaveVault autenticado com sucesso!</h2>",
            "<p>Você pode fechar esta aba e voltar para o SaveVault.</p>",
            "</body></html>",
        );
        let _ = socket.write_all(html.as_bytes()).await;

        // Troca o código pelos tokens enviando o code_verifier PKCE
        self.exchange_code_for_tokens(&code, &redirect_uri, Some(&code_verifier))
            .await?;
        info!("Login no Google Drive concluído com sucesso.");
        Ok(())
    }

    /// Troca um `authorization_code` por tokens e persiste o `refresh_token`.
    pub async fn exchange_code_for_tokens(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: Option<&str>,
    ) -> Result<()> {
        let mut params = HashMap::new();
        params.insert("client_id", self.client_id.as_str());
        params.insert("client_secret", self.client_secret.as_str());
        params.insert("code", code);
        params.insert("grant_type", "authorization_code");
        params.insert("redirect_uri", redirect_uri);
        if let Some(cv) = code_verifier {
            params.insert("code_verifier", cv);
        }

        let res = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await?;

        if !res.status().is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("Falha ao obter tokens do Google: {}", body));
        }

        let token_resp: TokenResponse = res.json().await?;

        match token_resp.refresh_token {
            Some(ref rt) => {
                let entry = Entry::new(KEYRING_SERVICE, KEYRING_GOOGLE_REFRESH)
                    .map_err(|e| anyhow!("Erro ao acessar o cofre de senhas: {}", e))?;
                entry
                    .set_password(rt)
                    .map_err(|e| anyhow!("Erro ao salvar refresh token: {}", e))?;
                info!("Refresh token salvo com segurança no Windows Credential Manager.");
            }
            None => {
                warn!(
                    "Google não retornou refresh_token. \
                    Isso pode ocorrer se o usuário já havia autorizado antes. \
                    Tente revogar o acesso em myaccount.google.com/permissions e refaça o login."
                );
            }
        }

        Ok(())
    }

    /// Obtém um access_token válido renovando via refresh_token armazenado.
    /// Deve ser chamado antes de qualquer operação na API do Google Drive.
    pub async fn get_fresh_access_token(&self) -> Result<String> {
        if let Some(ref fixed) = self.fixed_token {
            return Ok(fixed.clone());
        }

        let refresh_token = if let Some(ref rt) = self.mock_refresh_token {
            rt.clone()
        } else {
            let entry = Entry::new(KEYRING_SERVICE, KEYRING_GOOGLE_REFRESH)
                .map_err(|e| anyhow!("Erro ao acessar o cofre de senhas: {}", e))?;

            entry.get_password().map_err(|_| {
                anyhow!(
                    "SaveVault não está autenticado no Google Drive.\n\
                    Execute: cargo run --bin savevault -- auth login"
                )
            })?
        };

        let mut params = HashMap::new();
        params.insert("client_id", self.client_id.as_str());
        params.insert("client_secret", self.client_secret.as_str());
        params.insert("refresh_token", refresh_token.as_str());
        params.insert("grant_type", "refresh_token");

        let res = self.http.post(&self.token_url).form(&params).send().await?;

        if !res.status().is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("Falha ao renovar access token: {}", body));
        }

        let token_resp: TokenResponse = res.json().await?;
        debug!("Access token renovado com sucesso.");
        Ok(token_resp.access_token)
    }

    /// Remove o refresh token salvo (logout).
    pub fn logout(&self) -> Result<()> {
        let entry = Entry::new(KEYRING_SERVICE, KEYRING_GOOGLE_REFRESH)
            .map_err(|e| anyhow!("Erro ao acessar o cofre de senhas: {}", e))?;
        entry
            .delete_credential()
            .map_err(|e| anyhow!("Erro ao remover credenciais: {}", e))?;
        info!("Credenciais do Google Drive removidas.");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers internos
// ---------------------------------------------------------------------------

/// Abre uma URL no navegador padrão do Windows.
fn open_url_windows(url: &str) {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

/// Extrai o valor de um parâmetro de query string de um request HTTP cru.
/// Ex: extrai "abc" de "GET /oauth/callback?code=abc&state=... HTTP/1.1"
fn extract_query_param(raw_request: &str, param: &str) -> Option<String> {
    let first_line = raw_request.lines().next()?;
    let path = first_line.split_whitespace().nth(1)?;
    let query = path.split('?').nth(1)?;

    for pair in query.split('&') {
        let mut kv = pair.split('=');
        let key = kv.next()?;
        let value = kv.next()?;
        if key == param {
            // Decodificação simples de %XX e substituição de + por espaço
            return Some(percent_decode(value));
        }
    }
    None
}

/// Encoding mínimo de URL (apenas os caracteres que a RFC exige para query params).
fn urlencoding_simple(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ':' => "%3A".to_string(),
            '/' => "%2F".to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

/// Decodificação simples de percent-encoding (%XX → char).
fn percent_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next().unwrap_or('0');
            let h2 = chars.next().unwrap_or('0');
            let hex = format!("{}{}", h1, h2);
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Testes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_code_from_get_request() {
        let request =
            "GET /oauth/callback?code=abc123&scope=drive HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(
            extract_query_param(request, "code"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn extract_error_from_get_request() {
        let request = "GET /oauth/callback?error=access_denied HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(
            extract_query_param(request, "error"),
            Some("access_denied".to_string())
        );
    }

    #[test]
    fn extract_returns_none_for_missing_param() {
        let request = "GET /oauth/callback?state=xyz HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(extract_query_param(request, "code"), None);
    }

    #[test]
    fn percent_decode_handles_encoded_chars() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("foo+bar"), "foo bar");
    }

    #[test]
    fn urlencoding_simple_encodes_special_chars() {
        let encoded = urlencoding_simple("https://accounts.google.com/o/auth");
        assert!(!encoded.contains("://"));
        assert!(encoded.contains("%3A%2F%2F"));
    }
}
