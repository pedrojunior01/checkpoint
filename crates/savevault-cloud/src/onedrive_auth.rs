//! Autenticação OAuth 2.0 com PKCE para o Microsoft OneDrive (Microsoft Identity Platform v2.0).
//!
//! Princípios de Segurança e Design:
//! - Escopo restrito ao diretório do aplicativo: `Files.ReadWrite.AppFolder offline_access`
//!   (O aplicativo só acessa sua própria pasta `OneDrive/Aplicativos/SaveVault`, sem permissão
//!   para inspecionar documentos ou arquivos pessoais do usuário).
//! - Armazenamento seguro de refresh token no Windows Credential Manager via `keyring`.
//! - Cache de access token em memória (`CachedAccessToken`) com margem de segurança para evitar
//!   chamadas de rede redundantes.
//! - Proteção anti-CSRF através de validação estrita do parâmetro `state`.
//! - Suporte a PKCE e clientes públicos para desktops nativos.

use anyhow::{anyhow, Result};
use keyring::Entry;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::auth::{
    extract_query_param, get_keyring_password, open_url_windows, percent_decode,
    urlencoding_simple, AUTH_CONNECT_TIMEOUT, AUTH_REQUEST_TIMEOUT, KEYRING_SERVICE,
    LEGACY_KEYRING_SERVICE,
};

/// Chave do refresh token do OneDrive no Windows Credential Manager.
pub const KEYRING_ONEDRIVE_REFRESH: &str = "onedrive_refresh_token";

/// Escopo de menor privilégio para o OneDrive: acesso exclusivo à pasta de aplicativo `checkpoint`.
pub const ONEDRIVE_SCOPE: &str = "Files.ReadWrite.AppFolder offline_access";

/// Porta padrão do loopback local para callback do OneDrive.
pub const ONEDRIVE_REDIRECT_PORT: u16 = 8086;

/// URL padrão de autorização da Microsoft Identity Platform (contas pessoais / consumidores).
pub const ONEDRIVE_AUTH_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";

/// URL padrão de emissão/renovação de tokens da Microsoft Identity Platform.
pub const ONEDRIVE_TOKEN_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";

#[derive(Deserialize)]
struct OneDriveTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

fn default_expires_in() -> u64 {
    3600
}

#[derive(Clone, Debug)]
struct CachedAccessToken {
    token: String,
    expires_at: std::time::Instant,
}

fn build_auth_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(AUTH_CONNECT_TIMEOUT)
        .timeout(AUTH_REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Serviço de autenticação OAuth 2.0 para o Microsoft OneDrive.
#[derive(Clone)]
pub struct OneDriveAuthService {
    client_id: String,
    client_secret: Option<String>,
    http: reqwest::Client,
    auth_url: String,
    token_url: String,
    redirect_port: u16,
    fixed_token: Option<String>,
    mock_refresh_token: Option<String>,
    cached_token: Arc<RwLock<Option<CachedAccessToken>>>,
}

impl OneDriveAuthService {
    /// Cria uma nova instância com credenciais do Microsoft Entra / Azure Portal.
    pub fn new(client_id: String, client_secret: Option<String>) -> Self {
        Self {
            client_id,
            client_secret,
            http: build_auth_http_client(),
            auth_url: ONEDRIVE_AUTH_URL.to_string(),
            token_url: ONEDRIVE_TOKEN_URL.to_string(),
            redirect_port: ONEDRIVE_REDIRECT_PORT,
            fixed_token: None,
            mock_refresh_token: None,
            cached_token: Arc::new(RwLock::new(None)),
        }
    }

    /// Cria uma instância para testes com token fixo injetado.
    #[doc(hidden)]
    pub fn with_fixed_token(token: String) -> Self {
        Self {
            client_id: "test-client-id".to_string(),
            client_secret: None,
            http: build_auth_http_client(),
            auth_url: ONEDRIVE_AUTH_URL.to_string(),
            token_url: ONEDRIVE_TOKEN_URL.to_string(),
            redirect_port: ONEDRIVE_REDIRECT_PORT,
            fixed_token: Some(token),
            mock_refresh_token: None,
            cached_token: Arc::new(RwLock::new(None)),
        }
    }

    /// Configura endpoints personalizados (útil para testes com mock server).
    #[doc(hidden)]
    pub fn with_custom_endpoints(
        client_id: String,
        client_secret: Option<String>,
        auth_url: String,
        token_url: String,
        redirect_port: u16,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            http: build_auth_http_client(),
            auth_url,
            token_url,
            redirect_port,
            fixed_token: None,
            mock_refresh_token: None,
            cached_token: Arc::new(RwLock::new(None)),
        }
    }

    /// Permite injetar um mock refresh token para testes sem tocar no Windows Credential Manager.
    #[doc(hidden)]
    pub fn with_mock_refresh_token(mut self, token: String) -> Self {
        self.mock_refresh_token = Some(token);
        self
    }

    /// Injeta um cliente HTTP com timeouts customizados.
    #[doc(hidden)]
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.http = client;
        self
    }

    /// Retorna `true` se há credencial válida registrada no sistema.
    pub fn is_authenticated(&self) -> bool {
        if self.fixed_token.is_some() || self.mock_refresh_token.is_some() {
            return true;
        }

        get_keyring_password(KEYRING_ONEDRIVE_REFRESH).is_some()
    }

    /// Inicia o fluxo interativo de login via navegador com PKCE e servidor loopback local.
    pub async fn login_interactive(&self) -> Result<()> {
        let redirect_uri = format!("http://127.0.0.1:{}/oauth/callback", self.redirect_port);
        let oauth_state = uuid::Uuid::new_v4().to_string();
        let code_verifier = format!(
            "{}_{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        // RFC 7636: para clientes nativos, plain ou S256 são suportados pela Microsoft Identity
        let code_challenge = code_verifier.clone();

        let auth_url = format!(
            "{}\
            ?client_id={}\
            &response_type=code\
            &redirect_uri={}\
            &response_mode=query\
            &scope={}\
            &state={}\
            &code_challenge={}\
            &code_challenge_method=plain",
            self.auth_url,
            self.client_id,
            urlencoding_simple(&redirect_uri),
            urlencoding_simple(ONEDRIVE_SCOPE),
            oauth_state,
            code_challenge,
        );

        let listener = TcpListener::bind(format!("127.0.0.1:{}", self.redirect_port))
            .await
            .map_err(|e| {
                anyhow!(
                    "Falha ao abrir porta {} para callback do OneDrive: {}",
                    self.redirect_port,
                    e
                )
            })?;

        info!(
            "Aguardando callback do OneDrive na porta {}",
            self.redirect_port
        );
        println!("\nAbrindo navegador para autenticação no Microsoft OneDrive...");
        println!(
            "Caso o navegador não abra, acesse manualmente:\n{}\n",
            auth_url
        );

        open_url_windows(&auth_url);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        let mut auth_code = None;

        while tokio::time::Instant::now() < deadline {
            let accept_res = tokio::time::timeout_at(deadline, listener.accept()).await;
            let (mut socket, addr) = match accept_res {
                Ok(Ok(pair)) => pair,
                Ok(Err(e)) => return Err(anyhow!("Erro de I/O no listener OAuth: {}", e)),
                Err(_) => {
                    return Err(anyhow!(
                        "Tempo limite de autenticação OAuth esgotado (timeout de 3 minutos)."
                    ))
                }
            };
            debug!("Callback recebido de {}", addr);

            let mut buffer = vec![0u8; 4096];
            let n = match socket.read(&mut buffer).await {
                Ok(n) if n > 0 => n,
                _ => continue,
            };
            let request = String::from_utf8_lossy(&buffer[..n]);

            if request.contains("GET /favicon.ico") {
                let _ = socket
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .await;
                continue;
            }

            if !request.contains("/oauth/callback") {
                let _ = socket
                    .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")
                    .await;
                continue;
            }

            // Validação anti-CSRF
            let received_state = extract_query_param(&request, "state");
            if received_state.as_deref() != Some(&oauth_state) {
                let error_html = concat!(
                    "HTTP/1.1 400 Bad Request\r\n",
                    "Content-Type: text/html; charset=utf-8\r\n",
                    "Connection: close\r\n\r\n",
                    "<!DOCTYPE html><html><head><meta charset='utf-8'>",
                    "<title>checkpoint - Erro</title></head><body style='font-family:sans-serif;padding:2rem'>",
                    "<h2>❌ Falha de segurança na autenticação</h2>",
                    "<p>O parâmetro 'state' recebido não confere com o gerado. A requisição foi bloqueada.</p>",
                    "</body></html>",
                );
                let _ = socket.write_all(error_html.as_bytes()).await;
                return Err(anyhow!(
                    "Callback OAuth inválido: 'state' diverge ou ausente (possível CSRF)"
                ));
            }

            let code = extract_query_param(&request, "code").ok_or_else(|| {
                let error = extract_query_param(&request, "error_description")
                    .or_else(|| extract_query_param(&request, "error"))
                    .unwrap_or_else(|| "desconhecido".to_string());
                anyhow!("Autorização recusada pela Microsoft: {}", error)
            })?;

            let html = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/html; charset=utf-8\r\n",
                "Connection: close\r\n\r\n",
                "<!DOCTYPE html><html><head><meta charset='utf-8'>",
                "<title>checkpoint - Sucesso</title></head><body style='font-family:sans-serif;padding:2rem'>",
                "<h2>✅ Autenticação realizada com sucesso!</h2>",
                "<p>O checkpoint foi autorizado a sincronizar backups com o Microsoft OneDrive.</p>",
                "<p>Você já pode fechar esta aba.</p>",
                "</body></html>",
            );
            let _ = socket.write_all(html.as_bytes()).await;
            auth_code = Some(code);
            break;
        }

        let code = auth_code.ok_or_else(|| {
            anyhow!("Nenhum código de autorização foi recebido dentro do tempo limite.")
        })?;

        let code_decoded = percent_decode(&code);

        let mut params = HashMap::new();
        params.insert("client_id", self.client_id.as_str());
        if let Some(ref sec) = self.client_secret {
            params.insert("client_secret", sec.as_str());
        }
        params.insert("code", code_decoded.as_str());
        params.insert("code_verifier", code_verifier.as_str());
        params.insert("grant_type", "authorization_code");
        params.insert("redirect_uri", redirect_uri.as_str());
        params.insert("scope", ONEDRIVE_SCOPE);

        let res = self.http.post(&self.token_url).form(&params).send().await?;

        if !res.status().is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Falha ao trocar código por token na Microsoft Identity: {}",
                body
            ));
        }

        let token_resp: OneDriveTokenResponse = res.json().await?;

        match token_resp.refresh_token {
            Some(ref rt) => {
                let entry = Entry::new(KEYRING_SERVICE, KEYRING_ONEDRIVE_REFRESH)
                    .map_err(|e| anyhow!("Erro ao acessar cofre do sistema: {}", e))?;
                entry
                    .set_password(rt)
                    .map_err(|e| anyhow!("Erro ao salvar credenciais: {}", e))?;
                info!(
                    "Refresh token do OneDrive salvo com segurança no Windows Credential Manager."
                );
            }
            None => {
                let has_existing = Entry::new(KEYRING_SERVICE, KEYRING_ONEDRIVE_REFRESH)
                    .and_then(|e| e.get_password())
                    .is_ok();
                if !has_existing {
                    return Err(anyhow!(
                        "Microsoft Identity não retornou refresh_token e nenhum token prévio existe no sistema."
                    ));
                }
            }
        }

        let expires_in_secs = if token_resp.expires_in > 0 {
            token_resp.expires_in
        } else {
            3600
        };
        let expires_at = std::time::Instant::now() + Duration::from_secs(expires_in_secs);

        {
            let mut cache = self.cached_token.write().await;
            *cache = Some(CachedAccessToken {
                token: token_resp.access_token,
                expires_at,
            });
        }

        Ok(())
    }

    /// Obtém um access_token válido renovando via refresh_token se necessário.
    /// Utiliza cache em memória para evitar chamadas de rede redundantes.
    pub async fn get_fresh_access_token(&self) -> Result<String> {
        if let Some(ref fixed) = self.fixed_token {
            return Ok(fixed.clone());
        }

        // 1. Verifica cache em memória (com margem de segurança de 60s)
        {
            let cache = self.cached_token.read().await;
            if let Some(ref cached) = *cache {
                if std::time::Instant::now() + Duration::from_secs(60) < cached.expires_at {
                    debug!("Reutilizando access token do OneDrive em cache.");
                    return Ok(cached.token.clone());
                }
            }
        }

        let refresh_token = if let Some(ref rt) = self.mock_refresh_token {
            rt.clone()
        } else {
            get_keyring_password(KEYRING_ONEDRIVE_REFRESH).ok_or_else(|| {
                anyhow!(
                    "checkpoint não está autenticado no Microsoft OneDrive.\n\
                    Execute: checkpoint auth login"
                )
            })?
        };

        let mut params = HashMap::new();
        params.insert("client_id", self.client_id.as_str());
        if let Some(ref sec) = self.client_secret {
            params.insert("client_secret", sec.as_str());
        }
        params.insert("refresh_token", refresh_token.as_str());
        params.insert("grant_type", "refresh_token");
        params.insert("scope", ONEDRIVE_SCOPE);

        let res = self.http.post(&self.token_url).form(&params).send().await?;

        if !res.status().is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("Falha ao renovar token do OneDrive: {}", body));
        }

        let token_resp: OneDriveTokenResponse = res.json().await?;

        // Se a Microsoft emitir um novo refresh_token durante a rotação, atualiza o cofre
        if let Some(ref new_rt) = token_resp.refresh_token {
            if self.mock_refresh_token.is_none() {
                if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_ONEDRIVE_REFRESH) {
                    let _ = entry.set_password(new_rt);
                }
            }
        }

        let expires_in_secs = if token_resp.expires_in > 0 {
            token_resp.expires_in
        } else {
            3600
        };
        let expires_at = std::time::Instant::now() + Duration::from_secs(expires_in_secs);

        {
            let mut cache = self.cached_token.write().await;
            *cache = Some(CachedAccessToken {
                token: token_resp.access_token.clone(),
                expires_at,
            });
        }

        debug!(
            "Access token do OneDrive renovado com sucesso (válido por {}s).",
            expires_in_secs
        );
        Ok(token_resp.access_token)
    }

    /// Realiza logout removendo o refresh token do cofre e invalidando o cache.
    pub fn logout(&self) -> Result<()> {
        if let Ok(mut cache) = self.cached_token.try_write() {
            *cache = None;
        }

        if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_ONEDRIVE_REFRESH) {
            let _ = entry.delete_credential();
        }
        if let Ok(legacy_entry) = Entry::new(LEGACY_KEYRING_SERVICE, KEYRING_ONEDRIVE_REFRESH) {
            let _ = legacy_entry.delete_credential();
        }
        info!("Credenciais do Microsoft OneDrive removidas.");
        Ok(())
    }
}
