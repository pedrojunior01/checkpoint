//! Google Drive provider — implementação real via OAuth 2.0 + API v3.
//!
//! Organização dos arquivos no Drive:
//! ```text
//! SaveVault/                          <- pasta raiz criada pelo app
//!   <game-uuid>/                      <- pasta por jogo
//!     <file-uuid>/                    <- pasta por arquivo de save
//!       v1__<filename>.<ext>          <- snapshots versionados
//!       v2__<filename>.<ext>
//! ```

use anyhow::{anyhow, Result};
use reqwest::header::AUTHORIZATION;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::auth::AuthService;
use crate::CloudProvider;

/// Timeout padrão para estabelecimento de conexão TCP/TLS (DNS + Handshake).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout padrão por requisição HTTP para operações com o Google Drive.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

fn build_default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .timeout(DEFAULT_REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Valida se um `remote_file_id` é um identificador remoto opaco seguro.
///
/// Invariante de Segurança (SEC-01):
/// - Rejeita caracteres de caminho (`/`, `\`), prevenindo path traversal;
/// - Rejeita componentes relativos (`..`, `...`, ou strings compostas unicamente por pontos);
/// - Rejeita delimitadores de URL (`?`, `#`, `&`), prevenindo query/fragment injection;
/// - Rejeita caracteres de controle ASCII (`c.is_control()`) e espaços em branco;
/// - Rejeita identificador vazio.
///
/// Não utiliza uma whitelist artificial para não quebrar compatibilidade com IDs
/// opacos válidos gerados pela Google Drive API.
pub fn validate_drive_file_id(remote_file_id: &str) -> Result<()> {
    let trimmed = remote_file_id.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Identificador remoto não pode ser vazio"));
    }
    if remote_file_id.contains('/')
        || remote_file_id.contains('\\')
        || remote_file_id.contains('?')
        || remote_file_id.contains('#')
        || remote_file_id.contains('&')
        || remote_file_id
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(anyhow!(
            "Identificador remoto inválido: '{}'. Esperado Google Drive File ID opaco, não caminho ou parâmetros de URL.",
            remote_file_id
        ));
    }
    // Rejeita tentativas de path traversal puramente relativas como '.', '..', '...', etc.
    if remote_file_id.chars().all(|c| c == '.') {
        return Err(anyhow!(
            "Identificador remoto inválido: '{}'. Tentativa de path traversal detectada.",
            remote_file_id
        ));
    }
    Ok(())
}

/// Nome da pasta raiz criada no Google Drive do usuário.
const DRIVE_ROOT_FOLDER: &str = "SaveVault";
/// URL base da Google Drive API v3.
const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
/// URL base para uploads multipart na Drive API.
const DRIVE_UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

// ---------------------------------------------------------------------------
// GoogleDriveProvider
// ---------------------------------------------------------------------------

/// Limiar padrão a partir do qual é feito Resumable Streaming Upload (5 MB, padrão oficial da Google Drive API v3).
pub const RESUMABLE_UPLOAD_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024;

/// Provedor Google Drive com autenticação OAuth 2.0.
pub struct GoogleDriveProvider {
    auth: AuthService,
    client: reqwest::Client,
    api_base: String,
    upload_base: String,
    resumable_threshold_bytes: u64,
}

impl GoogleDriveProvider {
    /// Cria um novo provider usando as credenciais do Google Cloud Console.
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            auth: AuthService::new(client_id, client_secret),
            client: build_default_http_client(),
            api_base: DRIVE_API_BASE.to_string(),
            upload_base: DRIVE_UPLOAD_BASE.to_string(),
            resumable_threshold_bytes: RESUMABLE_UPLOAD_THRESHOLD_BYTES,
        }
    }

    /// Cria um provider para testes apontando para endpoints mockados.
    #[doc(hidden)]
    pub fn with_custom_endpoints(auth: AuthService, api_base: String, upload_base: String) -> Self {
        Self {
            auth,
            client: build_default_http_client(),
            api_base,
            upload_base,
            resumable_threshold_bytes: RESUMABLE_UPLOAD_THRESHOLD_BYTES,
        }
    }

    /// Permite customizar o limiar a partir do qual é feito upload resumable em vez de multipart (útil para testes unitários).
    pub fn with_resumable_threshold(mut self, bytes: u64) -> Self {
        self.resumable_threshold_bytes = bytes;
        self
    }

    /// Permite injetar um cliente HTTP configurado especificamente (ex: com timeouts reduzidos para testes).
    #[doc(hidden)]
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Tenta criar um provider a partir das variáveis de ambiente.
    /// Retorna `None` se as variáveis não estiverem definidas.
    pub fn from_env() -> Option<Self> {
        let client_id = std::env::var("SAVEVAULT_GOOGLE_CLIENT_ID").ok()?;
        let client_secret = std::env::var("SAVEVAULT_GOOGLE_CLIENT_SECRET").ok()?;
        Some(Self::new(client_id, client_secret))
    }

    /// Retorna `true` se o usuário já realizou o login (há um refresh token salvo).
    pub fn is_authenticated(&self) -> bool {
        self.auth.is_authenticated()
    }

    /// Inicia o fluxo interativo de login via navegador.
    pub async fn login(&self) -> Result<()> {
        self.auth.login_interactive().await
    }

    /// Remove as credenciais salvas (logout).
    pub fn logout(&self) -> Result<()> {
        self.auth.logout()
    }

    // -----------------------------------------------------------------------
    // Helpers internos
    // -----------------------------------------------------------------------

    /// Faz uma requisição autenticada e devolve o JSON de resposta.
    async fn get_json(&self, url: &str, token: &str) -> Result<serde_json::Value> {
        let res = self
            .client
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let body: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            let error_msg = body["error"]["message"]
                .as_str()
                .unwrap_or(if text.is_empty() {
                    "desconhecido"
                } else {
                    &text
                });
            return Err(anyhow!("Drive API GET erro {}: {}", status, error_msg));
        }

        if body.is_null() && !text.is_empty() {
            return Err(anyhow!("Resposta JSON inválida da Drive API: {}", text));
        }

        Ok(body)
    }

    /// Busca ou cria uma pasta no Drive com o nome especificado.
    /// Retorna o ID da pasta.
    async fn get_or_create_folder(
        &self,
        token: &str,
        name: &str,
        parent_id: Option<&str>,
    ) -> Result<String> {
        // Monta a query de busca
        let parent_clause = parent_id
            .map(|id| format!(" and '{}' in parents", id))
            .unwrap_or_default();

        let query = format!(
            "name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false{}",
            name, parent_clause
        );

        let url = format!(
            "{}/files?q={}&fields=files(id,name)&spaces=drive",
            self.api_base,
            urlencoding_query(&query),
        );

        let response = self.get_json(&url, token).await?;

        if let Some(files) = response["files"].as_array() {
            if let Some(first) = files.first() {
                if let Some(id) = first["id"].as_str() {
                    debug!("Pasta '{}' encontrada no Drive: {}", name, id);
                    return Ok(id.to_string());
                }
            }
        }

        // Não encontrou: cria a pasta
        let mut metadata = serde_json::json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder"
        });

        if let Some(parent) = parent_id {
            metadata["parents"] = serde_json::json!([parent]);
        }

        let res = self
            .client
            .post(format!("{}/files", self.api_base))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .json(&metadata)
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let body: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            let error_msg = body["error"]["message"]
                .as_str()
                .unwrap_or(if text.is_empty() {
                    "erro desconhecido"
                } else {
                    &text
                });
            return Err(anyhow!(
                "Falha ao criar pasta '{}': HTTP {}: {}",
                name,
                status,
                error_msg
            ));
        }

        let id = body["id"]
            .as_str()
            .ok_or_else(|| anyhow!("Resposta inválida ao criar pasta"))?
            .to_string();

        info!("Pasta '{}' criada no Drive: {}", name, id);
        Ok(id)
    }

    /// Garante que existe a hierarquia de pastas no Drive para um backup:
    /// `SaveVault/<game_uuid>/<file_uuid>/`
    ///
    /// Retorna o ID da pasta folha (file_uuid).
    async fn ensure_backup_path(
        &self,
        token: &str,
        game_uuid: &str,
        file_uuid: &str,
    ) -> Result<String> {
        let root_id = self
            .get_or_create_folder(token, DRIVE_ROOT_FOLDER, None)
            .await?;

        let game_id = self
            .get_or_create_folder(token, game_uuid, Some(&root_id))
            .await?;

        let file_id = self
            .get_or_create_folder(token, file_uuid, Some(&game_id))
            .await?;

        Ok(file_id)
    }

    /// Constrói com segurança a URL para uma operação em `/files/{drive_file_id}`
    /// utilizando `reqwest::Url` e manipulação segura de segmentos de caminho.
    pub fn build_file_url(
        &self,
        remote_file_id: &str,
        query: Option<&[(&str, &str)]>,
    ) -> Result<reqwest::Url> {
        validate_drive_file_id(remote_file_id)?;

        let mut url = reqwest::Url::parse(&self.api_base)
            .map_err(|e| anyhow!("Base URL da API inválida '{}': {}", self.api_base, e))?;

        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                anyhow!(
                    "Não é possível manipular segmentos de caminho na URL base: {}",
                    self.api_base
                )
            })?;
            segments.pop_if_empty().push("files").push(remote_file_id);
        }

        if let Some(params) = query {
            url.query_pairs_mut().extend_pairs(params.iter().copied());
        }

        Ok(url)
    }

    /// Consulta o status atual de uma sessão de upload resumable (conforme protocolo Google Drive API v3).
    pub async fn query_resumable_session_status(
        &self,
        session_uri: &str,
        file_size: u64,
    ) -> Result<ResumableSessionStatus> {
        let res = self
            .client
            .put(session_uri)
            .header("Content-Range", format!("bytes */{}", file_size))
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .send()
            .await?;

        let status = res.status();
        if status.as_u16() == 308 {
            let range_hdr = res
                .headers()
                .get(reqwest::header::RANGE)
                .and_then(|h| h.to_str().ok())
                .unwrap_or("");
            let last_byte = parse_range_header_last_byte(range_hdr);
            let next_offset = match last_byte {
                Some(b) => b + 1,
                None => 0,
            };
            Ok(ResumableSessionStatus::Incomplete { next_offset })
        } else if status.is_success() {
            let text = res.text().await.unwrap_or_default();
            let upload_resp: DriveUploadResponse = serde_json::from_str(&text).map_err(|e| {
                anyhow!(
                    "Resposta inválida ao consultar status: {}. Body: {}",
                    e,
                    text
                )
            })?;
            Ok(ResumableSessionStatus::Completed(upload_resp))
        } else if status.as_u16() == 404 || status.as_u16() == 410 {
            Ok(ResumableSessionStatus::Expired)
        } else {
            Err(anyhow!(
                "Consulta de status resumable falhou com HTTP {}",
                status
            ))
        }
    }

    /// Realiza upload via sessão Resumable com streaming assíncrono para arquivos grandes,
    /// tratando HTTP 308 (Resume Incomplete), reconexão e validação de tamanho final.
    pub async fn upload_resumable(
        &self,
        local_path: &Path,
        filename: &str,
        folder_id: &str,
        token: &str,
        file_size: u64,
        mime: &str,
    ) -> Result<String> {
        let init_url = format!("{}/files?uploadType=resumable", self.upload_base);
        let metadata = serde_json::json!({
            "name": filename,
            "parents": [folder_id]
        });

        let init_res = self
            .client
            .post(&init_url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=UTF-8",
            )
            .header("X-Upload-Content-Type", mime)
            .header("X-Upload-Content-Length", file_size.to_string())
            .json(&metadata)
            .send()
            .await?;

        let init_status = init_res.status();
        let init_headers = init_res.headers().clone();
        let init_text = init_res.text().await.unwrap_or_default();

        if !init_status.is_success() {
            let error_msg = serde_json::from_str::<serde_json::Value>(&init_text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| {
                    if init_text.is_empty() {
                        "erro desconhecido".to_string()
                    } else {
                        init_text
                    }
                });
            return Err(anyhow!(
                "Falha ao iniciar upload resumable para '{}': HTTP {}: {}",
                filename,
                init_status,
                error_msg
            ));
        }

        let session_uri = init_headers
            .get(reqwest::header::LOCATION)
            .and_then(|val| val.to_str().ok())
            .ok_or_else(|| {
                anyhow!("Google Drive não retornou header Location para resumable upload")
            })?
            .to_string();

        let mut offset: u64 = 0;
        let mut attempts: u32 = 0;
        const MAX_RESUME_ATTEMPTS: u32 = 5;

        while offset < file_size && attempts < MAX_RESUME_ATTEMPTS {
            attempts += 1;

            use tokio::io::AsyncSeekExt;
            let mut file = tokio::fs::File::open(local_path).await.map_err(|e| {
                anyhow!(
                    "Falha ao abrir arquivo '{}' para streaming: {}",
                    local_path.display(),
                    e
                )
            })?;

            if offset > 0 {
                file.seek(std::io::SeekFrom::Start(offset))
                    .await
                    .map_err(|e| {
                        anyhow!("Falha ao posicionar offset {} no arquivo: {}", offset, e)
                    })?;
            }

            let remaining_bytes = file_size - offset;
            let stream = tokio_util::io::ReaderStream::new(file);
            let body = reqwest::Body::wrap_stream(stream);

            let upload_res_result = self
                .client
                .put(&session_uri)
                .header(
                    "Content-Range",
                    format!("bytes {}-{}/{}", offset, file_size - 1, file_size),
                )
                .header(reqwest::header::CONTENT_LENGTH, remaining_bytes.to_string())
                .header(reqwest::header::CONTENT_TYPE, mime)
                .body(body)
                .send()
                .await;

            let upload_res = match upload_res_result {
                Ok(res) => res,
                Err(err) => {
                    warn!(
                        "Interrupção de rede durante upload de '{}' no offset {}: {}. Consultando status da sessão...",
                        filename, offset, err
                    );
                    match self
                        .query_resumable_session_status(&session_uri, file_size)
                        .await
                    {
                        Ok(ResumableSessionStatus::Completed(resp)) => return Ok(resp.id),
                        Ok(ResumableSessionStatus::Incomplete { next_offset }) => {
                            if next_offset > offset {
                                offset = next_offset;
                                continue;
                            }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            continue;
                        }
                        Ok(ResumableSessionStatus::Expired) => {
                            return Err(anyhow!("Sessão de upload resumable expirou no Drive"));
                        }
                        Err(q_err) => {
                            return Err(anyhow!(
                                "Falha no upload ({}) e na consulta de status ({})",
                                err,
                                q_err
                            ));
                        }
                    }
                }
            };

            let status = upload_res.status();

            if status.as_u16() == 308 {
                let range_hdr = upload_res
                    .headers()
                    .get(reqwest::header::RANGE)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("");
                if let Some(last_byte) = parse_range_header_last_byte(range_hdr) {
                    offset = last_byte + 1;
                    debug!(
                        "HTTP 308 recebido: bytes 0-{} gravados. Próximo offset: {}",
                        last_byte, offset
                    );
                    if offset >= file_size {
                        match self
                            .query_resumable_session_status(&session_uri, file_size)
                            .await?
                        {
                            ResumableSessionStatus::Completed(resp) => return Ok(resp.id),
                            _ => {
                                return Err(anyhow!(
                                    "Upload indicou término mas sessão não retornou ID"
                                ))
                            }
                        }
                    }
                    continue;
                } else {
                    match self
                        .query_resumable_session_status(&session_uri, file_size)
                        .await?
                    {
                        ResumableSessionStatus::Completed(resp) => return Ok(resp.id),
                        ResumableSessionStatus::Incomplete { next_offset } => {
                            offset = next_offset;
                            continue;
                        }
                        ResumableSessionStatus::Expired => {
                            return Err(anyhow!("Sessão de upload resumable expirou"));
                        }
                    }
                }
            }

            let text = upload_res.text().await.unwrap_or_default();
            if !status.is_success() {
                let error_msg = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| {
                        if text.is_empty() {
                            "erro desconhecido".to_string()
                        } else {
                            text
                        }
                    });
                return Err(anyhow!(
                    "Falha ao enviar stream de dados para '{}': HTTP {}: {}",
                    filename,
                    status,
                    error_msg
                ));
            }

            let upload_resp: DriveUploadResponse = serde_json::from_str(&text).map_err(|e| {
                anyhow!(
                    "Resposta de upload inválida do Google Drive: {}. Body: {}",
                    e,
                    text
                )
            })?;

            if upload_resp.id.trim().is_empty() {
                return Err(anyhow!("Google Drive retornou upload com campo 'id' vazio"));
            }

            // Validação de integridade do tamanho retornado pelo Drive
            if let Some(ref sz_str) = upload_resp.size {
                if let Ok(returned_size) = sz_str.parse::<u64>() {
                    if returned_size != file_size {
                        return Err(anyhow!(
                            "Inconsistência de tamanho pós-upload para '{}': remoto={} bytes, esperado={} bytes",
                            filename,
                            returned_size,
                            file_size
                        ));
                    }
                }
            }

            info!(
                "Upload Resumable concluído com sucesso: {} (drive_file_id: {})",
                filename, upload_resp.id
            );
            return Ok(upload_resp.id);
        }

        Err(anyhow!(
            "Número máximo de tentativas ({}) de retomada resumable excedido para '{}'",
            MAX_RESUME_ATTEMPTS,
            filename
        ))
    }

    /// Faz o download de um arquivo do Google Drive com verificação rigorosa de integridade criptográfica.
    ///
    /// Invariantes de Integridade (SV-ADV-007):
    /// 1. Cria o staging no diretório de destino sob nome temporário único (`.{filename}.download_tmp.{uuid}`);
    /// 2. Faz streaming delimitando o consumo de RAM;
    /// 3. Se a conexão cair, falhar ou for interrompida, remove o staging imediatamente sem deixar órfãos;
    /// 4. Valida se o total de bytes baixados confere com o header `Content-Length` (se fornecido);
    /// 5. Calcula simultaneamente hashes SHA-256 e MD5 durante o streaming;
    /// 6. Se um `expected_hash` (MD5 ou SHA-256) foi fornecido, valida antes de promover;
    /// 7. Apenas quando todas as validações forem aprovadas promove atomicamente via rename.
    pub async fn download_with_integrity(
        &self,
        remote_file_id: &str,
        local_path: &Path,
        expected_hash: Option<&str>,
    ) -> Result<()> {
        let url = self.build_file_url(remote_file_id, Some(&[("alt", "media")]))?;
        let token = self.auth.get_fresh_access_token().await?;

        let mut res = self
            .client
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!(
                "Falha ao baixar arquivo do Drive (id: {}): HTTP {}",
                remote_file_id,
                status
            ));
        }

        let expected_content_length = res.content_length();

        let parent = local_path.parent().unwrap_or_else(|| Path::new("."));
        tokio::fs::create_dir_all(parent).await?;

        let filename = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download.tmp");
        let tmp_path = parent.join(format!(
            ".{}.download_tmp.{}",
            filename,
            uuid::Uuid::new_v4()
        ));

        use md5::Digest;
        let mut md5_hasher = md5::Md5::new();
        let mut sha256_hasher = sha2::Sha256::new();
        let mut total_bytes: u64 = 0;

        {
            use tokio::io::AsyncWriteExt;
            let mut file = match tokio::fs::File::create(&tmp_path).await {
                Ok(f) => f,
                Err(e) => {
                    return Err(anyhow!(
                        "Falha ao criar staging '{}': {}",
                        tmp_path.display(),
                        e
                    ))
                }
            };

            loop {
                let chunk_res = res.chunk().await;
                match chunk_res {
                    Ok(Some(chunk)) => {
                        md5_hasher.update(&chunk);
                        sha256_hasher.update(&chunk);
                        total_bytes += chunk.len() as u64;
                        if let Err(e) = file.write_all(&chunk).await {
                            let _ = tokio::fs::remove_file(&tmp_path).await;
                            return Err(anyhow!(
                                "Falha de I/O ao gravar staging '{}': {}",
                                tmp_path.display(),
                                e
                            ));
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                        return Err(anyhow!(
                            "Conexão HTTP interrompida durante download do Drive: {}",
                            e
                        ));
                    }
                }
            }

            if let Err(e) = file.flush().await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(anyhow!("Falha ao fazer flush do staging: {}", e));
            }
            if let Err(e) = file.sync_all().await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(anyhow!("Falha ao fazer sync do staging: {}", e));
            }
        }

        // Validação 1: Content-Length se informado pelo servidor
        if let Some(expected_len) = expected_content_length {
            if total_bytes != expected_len {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(anyhow!(
                    "Download truncado/incompleto para '{}': recebidos {} bytes, esperado {} bytes",
                    remote_file_id,
                    total_bytes,
                    expected_len
                ));
            }
        }

        let computed_md5 = hex_encode(&md5_hasher.finalize());
        let computed_sha256 = hex_encode(&sha256_hasher.finalize());

        // Validação 2: Expected hash (se fornecido pelo caller)
        if let Some(expected) = expected_hash {
            let expected_clean = expected.trim().to_lowercase();
            if expected_clean != computed_md5 && expected_clean != computed_sha256 {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(anyhow!(
                    "Integridade criptográfica violada pós-download para '{}': hash recebido (SHA256: {}, MD5: {}) não coincide com o esperado ({})",
                    remote_file_id,
                    computed_sha256,
                    computed_md5,
                    expected
                ));
            }
        }

        // Promoção atômica do staging para o arquivo definitivo
        if let Err(e) = tokio::fs::rename(&tmp_path, local_path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(anyhow!(
                "Falha ao renomear arquivo temporário para '{}': {}",
                local_path.display(),
                e
            ));
        }

        info!(
            "Download concluído com integridade verificada: Drive/{} → '{}' ({} bytes, sha256: {})",
            remote_file_id,
            local_path.display(),
            total_bytes,
            computed_sha256
        );

        Ok(())
    }
}

/// Status de uma sessão resumable consultada no Google Drive.
#[derive(Debug)]
pub enum ResumableSessionStatus {
    Completed(DriveUploadResponse),
    Incomplete { next_offset: u64 },
    Expired,
}

fn parse_range_header_last_byte(range_hdr: &str) -> Option<u64> {
    let clean = range_hdr.trim();
    let after_eq = clean
        .split('=')
        .nth(1)
        .or_else(|| clean.split_whitespace().nth(1))?;
    let range_part = after_eq.split('/').next()?.trim();
    let end_str = range_part.split('-').nth(1)?.trim();
    end_str.parse::<u64>().ok()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Estrutura tipada de resposta de upload/metadados da Google Drive API.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct DriveUploadResponse {
    pub id: String,
    pub name: Option<String>,
    pub size: Option<String>,
}

// ---------------------------------------------------------------------------
// Implementação da trait CloudProvider
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl CloudProvider for GoogleDriveProvider {
    /// Faz o upload de um arquivo local para o Google Drive.
    ///
    /// `cloud_path` deve ter o formato: `<game_uuid>/<file_uuid>/<filename>`
    /// Retorna o `drive_file_id` remoto (Google Drive File ID alfanumérico opaco).
    ///
    /// Idempotência: antes de fazer o upload, verifica se o arquivo já existe na pasta
    /// de destino do Drive (por exemplo, após falha do processo logo após upload anterior).
    /// Se já existir, reutiliza o ID existente sem criar duplicatas remotas.
    async fn upload(&self, local_path: &Path, cloud_path: &str) -> Result<String> {
        let token = self.auth.get_fresh_access_token().await?;

        // Decompõe o cloud_path em partes
        let parts: Vec<&str> = cloud_path.splitn(3, '/').collect();
        let (game_uuid, file_uuid, filename) = match parts.as_slice() {
            [g, f, n] => (*g, *f, *n),
            _ => {
                return Err(anyhow!(
                    "cloud_path inválido: '{}'. Esperado: '<game_uuid>/<file_uuid>/<filename>'",
                    cloud_path
                ))
            }
        };

        let folder_id = self
            .ensure_backup_path(&token, game_uuid, file_uuid)
            .await?;

        // Verificação de idempotência: se o arquivo já existe no Drive na mesma pasta, reutiliza o ID
        let check_query = format!(
            "name='{}' and '{}' in parents and trashed=false",
            filename, folder_id
        );
        let check_url = format!(
            "{}/files?q={}&fields=files(id,name,size)&spaces=drive",
            self.api_base,
            urlencoding_query(&check_query)
        );
        if let Ok(existing_resp) = self.get_json(&check_url, &token).await {
            if let Some(files) = existing_resp["files"].as_array() {
                if let Some(first) = files.first() {
                    if let Some(id) = first["id"].as_str() {
                        if !id.trim().is_empty() {
                            info!(
                                "Arquivo '{}' já existente no Drive (id: {}). Reutilizando ID remoto com garantia de idempotência.",
                                filename, id
                            );
                            return Ok(id.to_string());
                        }
                    }
                }
            }
        }

        let metadata = tokio::fs::metadata(local_path).await.map_err(|e| {
            anyhow!(
                "Falha ao obter metadados do arquivo local '{}': {}",
                local_path.display(),
                e
            )
        })?;
        let file_size = metadata.len();
        let mime = mime_from_extension(local_path);

        // Se o tamanho do arquivo for maior ou igual ao limiar configurado (default 5MB),
        // utiliza Resumable Streaming Upload para garantir baixo consumo de RAM.
        if file_size >= self.resumable_threshold_bytes {
            info!(
                "Arquivo '{}' ({:.2} MB) excede o limiar de {:.2} MB. Utilizando Resumable Streaming Upload.",
                filename,
                file_size as f64 / 1_048_576.0,
                self.resumable_threshold_bytes as f64 / 1_048_576.0
            );
            return self
                .upload_resumable(local_path, filename, &folder_id, &token, file_size, mime)
                .await;
        }

        let file_bytes = tokio::fs::read(local_path).await.map_err(|e| {
            anyhow!(
                "Falha ao ler arquivo local '{}': {}",
                local_path.display(),
                e
            )
        })?;

        // Metadados do arquivo no Drive
        let metadata = serde_json::json!({
            "name": filename,
            "parents": [folder_id]
        });

        // Upload Multipart: metadados JSON + binário do arquivo numa única requisição
        let metadata_part = reqwest::multipart::Part::text(metadata.to_string())
            .mime_str("application/json; charset=UTF-8")?;

        let file_part = reqwest::multipart::Part::bytes(file_bytes)
            .mime_str(mime)?
            .file_name(filename.to_string());

        let form = reqwest::multipart::Form::new()
            .part("metadata", metadata_part)
            .part("file", file_part);

        let res = self
            .client
            .post(format!(
                "{}/files?uploadType=multipart&fields=id,name,size",
                self.upload_base
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            let error_msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| {
                    if text.is_empty() {
                        "erro desconhecido".to_string()
                    } else {
                        text
                    }
                });
            return Err(anyhow!(
                "Falha no upload de '{}' para o Drive: HTTP {}: {}",
                filename,
                status,
                error_msg
            ));
        }

        let upload_resp: DriveUploadResponse = serde_json::from_str(&text).map_err(|e| {
            anyhow!(
                "Resposta de upload inválida do Google Drive: {}. Body: {}",
                e,
                text
            )
        })?;

        if upload_resp.id.trim().is_empty() {
            return Err(anyhow!("Google Drive retornou upload com campo 'id' vazio"));
        }

        info!(
            "Upload concluído: '{}' → Drive/{} (drive_file_id: {})",
            local_path.display(),
            cloud_path,
            upload_resp.id
        );

        Ok(upload_resp.id)
    }

    /// Faz o download de um arquivo do Google Drive para o disco local em streaming contínuo.
    ///
    /// `remote_file_id` deve ser o **Google Drive File ID** opaco alfanumérico.
    async fn download(&self, remote_file_id: &str, local_path: &Path) -> Result<()> {
        self.download_with_integrity(remote_file_id, local_path, None)
            .await
    }

    /// Remove um arquivo do Google Drive pelo seu File ID.
    async fn delete(&self, remote_file_id: &str) -> Result<()> {
        let url = self.build_file_url(remote_file_id, None)?;

        let token = self.auth.get_fresh_access_token().await?;

        let res = self
            .client
            .delete(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();

        // 204 No Content = sucesso; 404 = já não existe (toleramos)
        if status.is_success() || status.as_u16() == 404 {
            if status.as_u16() == 404 {
                warn!(
                    "Arquivo Drive/{} não encontrado (já foi deletado?)",
                    remote_file_id
                );
            } else {
                info!("Arquivo Drive/{} removido.", remote_file_id);
            }
            return Ok(());
        }

        Err(anyhow!(
            "Falha ao deletar arquivo Drive/{}: HTTP {}",
            remote_file_id,
            status
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Retorna o MIME type baseado na extensão do arquivo.
fn mime_from_extension(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("zip") => "application/zip",
        Some("json") => "application/json",
        Some("toml") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Percent-encoding simples para uso em parâmetros de query da URL.
fn urlencoding_query(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
