//! Estado e Controladores da Interface de Desenvolvimento (DEV-UI-01).
//!
//! Desacopla a camada visual das operações do backend, expondo uma API limpa
//! e testável sem dependência de rendering para consumo pela interface gráfica e testes.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use savevault_core::models::{Confidence, Game, Platform, SaveLocation, SaveLocationType};
use savevault_db::repositories::{FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo};
use savevault_monitor::scanner::SaveScanner;

use crate::reconciler::{Reconciler, ReconciliationError, ReconciliationReport};
use crate::restore::{restore_snapshot_with_db, RestoreError, RestoreRequest};

/// Nível de severidade de um evento registrado na DEV-UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DevUiEventLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Registro de evento para o log visual da DEV-UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevUiEvent {
    pub id: u64,
    pub timestamp: String,
    pub level: DevUiEventLevel,
    pub message: String,
    pub details: Option<String>,
}

/// Resumo geral do sistema para o Dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSummary {
    pub app_status: String,
    pub db_status: String,
    pub db_path: String,
    pub backup_root: String,
    pub save_locations_count: usize,
    pub monitored_files_count: usize,
    pub snapshots_count: usize,
    pub sync_pending_count: usize,
    pub watcher_status: String,
    pub reconciler_status: String,
    pub reconciliation_interval_secs: u64,
    pub last_reconciliation_time: Option<String>,
    pub next_reconciliation_time: Option<String>,
    pub last_event: Option<DevUiEvent>,
    pub last_error: Option<String>,
}

/// Visão de um SaveLocation cadastrado.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationView {
    pub id: Uuid,
    pub game_id: Uuid,
    pub game_name: String,
    pub platform: String,
    pub path: String,
    pub location_type: String,
    pub exists_on_disk: bool,
    pub monitored_files_count: usize,
    pub last_check: String,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub last_backup_time: Option<String>,
}

/// Arquivo físico descoberto durante varredura manual.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredFileView {
    pub path: String,
    pub filename: String,
    pub size_bytes: u64,
    pub size_formatted: String,
    pub modified_at: String,
}

/// Resultado do scan sob demanda de um SaveLocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanLocationResult {
    pub location_id: Uuid,
    pub files_found: Vec<DiscoveredFileView>,
    pub scanned_at: String,
}

/// Visão estruturada de um Snapshot para a UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotView {
    pub id: Uuid,
    pub file_id: Uuid,
    pub game_name: String,
    pub file_path: String,
    pub file_name: String,
    pub version: u32,
    pub size_bytes: u64,
    pub size_formatted: String,
    pub hash: String,
    pub short_hash: String,
    pub created_at: String,
    pub physical_path: String,
    pub physical_exists: bool,
    pub sync_status: String,
}

/// Informações de um Safety Backup individual.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyBackupItem {
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
    pub size_formatted: String,
    pub modified_at: String,
}

/// Grupo de Safety Backups associados a um arquivo original.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyBackupGroup {
    pub original_file_id: Uuid,
    pub original_path: String,
    pub original_name: String,
    pub game_name: String,
    pub backups: Vec<SafetyBackupItem>,
}

/// Visão detalhada do status de Reconciliação.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationView {
    pub status: String,
    pub interval_secs: u64,
    pub is_running: bool,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub locations_scanned: usize,
    pub files_seen: usize,
    pub files_added: usize,
    pub files_changed: usize,
    pub files_removed: usize,
    pub errors: usize,
    pub duration_ms: u64,
}

/// Resposta de uma operação de restauração para a UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreApiResponse {
    pub success: bool,
    pub snapshot_id: Option<Uuid>,
    pub version: Option<u32>,
    pub dest_path: Option<String>,
    pub safety_backup_path: Option<String>,
    pub restored_hash: Option<String>,
    pub bytes_restored: Option<u64>,
    pub error: Option<String>,
    pub details: Option<String>,
}

/// Resultado da otimização preventiva e limpeza do banco SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationResult {
    pub purged_sync_items: usize,
    pub db_size_bytes: u64,
    pub db_size_formatted: String,
}

/// Resultado da exportação do pacote de backup em arquivo .ZIP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub export_path: String,
    pub file_size_bytes: u64,
    pub file_size_formatted: String,
}

/// DTO para transferência de configurações com o frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDto {
    pub max_versions: u32,
    pub debounce_ms: u64,
    pub reconciliation_interval_secs: u64,
    pub log_level: String,
    pub backup_dir: String,
    pub database_path: String,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub cloud_sync_path: Option<String>,
}

/// Configurações detalhadas da nuvem para a UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfigView {
    pub provider: String,
    pub local_sync_path: String,
    pub enabled: bool,
    pub detected_folders: Vec<savevault_cloud::DetectedCloudFolder>,
}

/// Estado compartilhado e thread-safe da DEV-UI.
pub struct DevUiState {
    pub db: Arc<Mutex<Connection>>,
    pub db_path: PathBuf,
    backup_root: std::sync::RwLock<PathBuf>,
    pub reconciler: Option<Arc<Reconciler>>,
    pub reconciliation_interval: Duration,
    pub watcher_active: AtomicBool,
    pub reconciler_active: AtomicBool,
    event_counter: AtomicU64,
    recent_events: Mutex<VecDeque<DevUiEvent>>,
    last_error: Mutex<Option<String>>,
    last_reconciliation_report: Mutex<Option<ReconciliationReport>>,
    last_reconcile_time: Mutex<Option<DateTime<Utc>>>,
    pub sync_worker: Mutex<Option<Arc<crate::sync_worker::SyncWorker>>>,
    pub steam_cover_cache: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

/// Mapeia nomes conhecidos de jogos, abreviações ou nomes de projeto UE para seus respectivos AppIDs Steam.
pub fn map_game_name_to_steam_id(name: &str) -> Option<&'static str> {
    let lower = name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "kofxv" | "kof xv" | "the king of fighters xv" => Some("1498570"),
        "mortalshell2" | "mortal shell ii" | "mortal shell 2" => Some("2584270"),
        "mortalshell" | "mortal shell" => Some("1110910"),
        "rsdragonwilds" | "dragonwilds" | "runescape: dragonwilds" | "runescape dragonwilds" => Some("1374490"),
        "pioneergame" | "pioneer" | "arc raiders" => Some("1808500"),
        "dysmantle" => Some("846770"),
        "sekiro" | "sekiro: shadows die twice" | "sekiro shadows die twice" => Some("814380"),
        "elden ring" | "eldenring" => Some("1245620"),
        "no man's sky" | "no mans sky" | "nomanssky" | "nms" => Some("275850"),
        "warhammer 40,000: space marine 2" | "space marine 2" | "spacemarine2" => Some("2183900"),
        "street fighter 6" | "streetfighter6" | "sf6" => Some("1364780"),
        "the witcher 3" | "the witcher 3: wild hunt" | "witcher 3" | "witcher3" => Some("292030"),
        "armored core vi" | "armored core 6" | "armored core vi fires of rubicon" | "armoredcorevi" => Some("1888160"),
        "aion 2" | "aion2" => Some("3393110"),
        "iron nest" | "iron nest: heavy turret simulator" | "iron nest heavy turret simulator" => Some("2950790"),
        "wallpaper engine" => Some("431960"),
        "albion online" => Some("761890"),
        _ => None,
    }
}

impl DevUiState {
    pub fn new(
        db: Arc<Mutex<Connection>>,
        db_path: PathBuf,
        backup_root: PathBuf,
        reconciler: Option<Arc<Reconciler>>,
        reconciliation_interval: Duration,
    ) -> Self {
        let initial_covers = {
            let cache_file = db_path.parent().unwrap_or(std::path::Path::new(".")).join("steam_covers.json");
            if let Ok(c) = std::fs::read_to_string(&cache_file) {
                serde_json::from_str(&c).unwrap_or_default()
            } else {
                std::collections::HashMap::new()
            }
        };

        Self {
            db,
            db_path,
            backup_root: std::sync::RwLock::new(backup_root),
            reconciler,
            reconciliation_interval,
            watcher_active: AtomicBool::new(true),
            reconciler_active: AtomicBool::new(true),
            event_counter: AtomicU64::new(1),
            recent_events: Mutex::new(VecDeque::with_capacity(100)),
            last_error: Mutex::new(None),
            last_reconciliation_report: Mutex::new(None),
            last_reconcile_time: Mutex::new(None),
            sync_worker: Mutex::new(None),
            steam_cover_cache: std::sync::Mutex::new(initial_covers),
        }
    }

    /// Retorna o diretório raiz de backups configurado atualmente.
    pub fn get_backup_root(&self) -> PathBuf {
        self.backup_root.read().map(|b| b.clone()).unwrap_or_default()
    }

    /// Atualiza dinamicamente o diretório raiz de backups e notifica o reconciliador.
    pub fn update_backup_root(&self, new_root: PathBuf) {
        if let Ok(mut lock) = self.backup_root.write() {
            *lock = new_root.clone();
        }
        if let Some(ref rec) = self.reconciler {
            rec.update_backup_root(new_root);
        }
    }

    /// Define a instância ativa do SyncWorker para comunicação em tempo real com a UI.
    pub async fn set_sync_worker(&self, worker: Arc<crate::sync_worker::SyncWorker>) {
        let mut lock = self.sync_worker.lock().await;
        *lock = Some(worker);
    }

    /// Salva o cache de capas no disco de forma resiliente (config_dir/steam_covers.json)
    pub fn persist_cover_cache(&self) {
        let lock = self.steam_cover_cache.lock().unwrap_or_else(|e| e.into_inner());
        let cache_file = self.db_path.parent().unwrap_or(std::path::Path::new(".")).join("steam_covers.json");
        if let Ok(json) = serde_json::to_string_pretty(&*lock) {
            let _ = std::fs::write(cache_file, json);
        }
    }

    /// Resolve a capa oficial de um jogo Steam via cache ou consulta a API publica do Steam
    pub async fn resolve_steam_cover(&self, app_id: &str) -> Option<String> {
        if app_id.is_empty() || !app_id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }

        // 1. Verifica cache em memória
        {
            let lock = self.steam_cover_cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(cached) = lock.get(app_id) {
                return Some(cached.clone());
            }
        }

        // 2. Consulta API pública do Steam (https://store.steampowered.com/api/appdetails?appids=...)
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(3500))
            .build()
            .ok()?;

        let api_url = format!("https://store.steampowered.com/api/appdetails?appids={}&filters=basic", app_id);
        if let Ok(resp) = client.get(&api_url).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(data) = json.get(app_id).and_then(|v| v.get("data")) {
                    if let Some(header_img) = data.get("header_image").and_then(|v| v.as_str()) {
                        let final_url = header_img.to_string();
                        {
                            let mut lock = self.steam_cover_cache.lock().unwrap_or_else(|e| e.into_inner());
                            lock.insert(app_id.to_string(), final_url.clone());
                        }
                        self.persist_cover_cache();
                        return Some(final_url);
                    }
                }
            }
        }

        // 3. Fallback clássico
        Some(format!(
            "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{}/header.jpg",
            app_id
        ))
    }

    /// Resolve a capa oficial de um jogo pelo nome (dicionário de aliases ou busca no Steam Store).
    pub async fn resolve_cover_by_name(&self, game_name: &str) -> Option<String> {
        let name_trimmed = game_name.trim();
        if name_trimmed.is_empty() {
            return None;
        }

        // 1. Dicionário de aliases conhecidos
        if let Some(app_id) = map_game_name_to_steam_id(name_trimmed) {
            return self.resolve_steam_cover(app_id).await;
        }

        // 2. Cache em memória
        let norm_key = format!("name:{}", name_trimmed.to_ascii_lowercase());
        {
            let lock = self.steam_cover_cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(cached) = lock.get(&norm_key) {
                return Some(cached.clone());
            }
        }

        // 3. Consulta à API de busca da loja do Steam
        let search_term = if name_trimmed.ends_with("Game") && name_trimmed.len() > 4 {
            &name_trimmed[..name_trimmed.len() - 4]
        } else {
            name_trimmed
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(4000))
            .build()
            .ok()?;

        let search_url = "https://store.steampowered.com/api/storesearch/";
        if let Ok(resp) = client
            .get(search_url)
            .query(&[("term", search_term), ("l", "english"), ("cc", "US")])
            .send()
            .await
        {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(items) = json.get("items").and_then(|v| v.as_array()) {
                    if let Some(first) = items.first() {
                        if let Some(id_val) = first.get("id") {
                            let app_id = id_val.to_string();
                            if let Some(cover) = self.resolve_steam_cover(&app_id).await {
                                {
                                    let mut lock = self.steam_cover_cache.lock().unwrap_or_else(|e| e.into_inner());
                                    lock.insert(norm_key, cover.clone());
                                }
                                self.persist_cover_cache();
                                return Some(cover);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Resolve a capa de um jogo por AppID ou Nome do jogo com persistência de cache.
    pub async fn resolve_cover(&self, app_id: Option<&str>, game_name: Option<&str>) -> Option<String> {
        if let Some(id) = app_id {
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                if let Some(cover) = self.resolve_steam_cover(id).await {
                    return Some(cover);
                }
            }
        }

        if let Some(name) = game_name {
            if let Some(cover) = self.resolve_cover_by_name(name).await {
                return Some(cover);
            }
        }

        None
    }

    /// Registra um evento no log em memória da DEV-UI.
    pub async fn record_event(
        &self,
        level: DevUiEventLevel,
        message: impl Into<String>,
        details: Option<String>,
    ) {
        let id = self.event_counter.fetch_add(1, Ordering::SeqCst);
        let event = DevUiEvent {
            id,
            timestamp: Utc::now().format("%H:%M:%S").to_string(),
            level,
            message: message.into(),
            details,
        };

        let mut events = self.recent_events.lock().await;
        if events.len() >= 100 {
            events.pop_front();
        }
        events.push_back(event);
    }

    /// Registra o último erro conhecido.
    pub async fn record_error(&self, err: &str) {
        let mut last_err = self.last_error.lock().await;
        *last_err = Some(err.to_string());
        self.record_event(DevUiEventLevel::Error, err, None).await;
    }

    /// Retorna os eventos recentes armazenados em memória.
    pub async fn get_events(&self) -> Vec<DevUiEvent> {
        let events = self.recent_events.lock().await;
        events.iter().cloned().rev().collect()
    }

    /// Retorna o resumo consolidado para o Dashboard.
    pub async fn get_dashboard(&self) -> Result<DashboardSummary> {
        let (locations_count, monitored_files_count, snapshots_count, sync_pending_count) = {
            let conn = self.db.lock().await;
            let loc_repo = SaveLocationsRepo::new(&conn);
            let files_repo = FilesRepo::new(&conn);
            let queue_repo = SyncQueueRepo::new(&conn);

            let locations = loc_repo.list_all().unwrap_or_default();
            let locations_count = locations.len();

            let mut monitored_files_count = 0usize;
            for loc in &locations {
                let files = files_repo.list_by_location(loc.id).unwrap_or_default();
                monitored_files_count += files.len();
            }

            let snapshots_count: usize = conn
                .query_row("SELECT COUNT(*) FROM backups", [], |r| r.get(0))
                .unwrap_or(0);

            let sync_pending_count = queue_repo.list_pending().unwrap_or_default().len();

            (
                locations_count,
                monitored_files_count,
                snapshots_count,
                sync_pending_count,
            )
        };

        let last_recon = *self.last_reconcile_time.lock().await;
        let last_recon_str = last_recon.map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string());

        let next_recon_str = last_recon.map(|t| {
            let next =
                t + chrono::Duration::from_std(self.reconciliation_interval).unwrap_or_default();
            next.format("%H:%M:%S UTC").to_string()
        });

        let watcher_status = if self.watcher_active.load(Ordering::SeqCst) {
            "Running".to_string()
        } else {
            "Idle".to_string()
        };

        let is_recon = if let Some(ref r) = self.reconciler {
            r.is_reconciling()
        } else {
            false
        };

        let reconciler_status = if is_recon {
            "Reconciling".to_string()
        } else if self.reconciler_active.load(Ordering::SeqCst) {
            "Running".to_string()
        } else {
            "Idle".to_string()
        };

        let last_event = {
            let events = self.recent_events.lock().await;
            events.back().cloned()
        };

        let last_error = self.last_error.lock().await.clone();

        let app_status = if last_error.is_some() {
            "Warning".to_string()
        } else {
            "Online".to_string()
        };

        Ok(DashboardSummary {
            app_status,
            db_status: "Connected".to_string(),
            db_path: self.db_path.to_string_lossy().to_string(),
            backup_root: self.get_backup_root().to_string_lossy().to_string(),
            save_locations_count: locations_count,
            monitored_files_count,
            snapshots_count,
            sync_pending_count,
            watcher_status,
            reconciler_status,
            reconciliation_interval_secs: self.reconciliation_interval.as_secs(),
            last_reconciliation_time: last_recon_str,
            next_reconciliation_time: next_recon_str,
            last_event,
            last_error,
        })
    }

    /// Retorna a lista de SaveLocations para a aba de Localizações.
    pub async fn get_locations(&self) -> Result<Vec<LocationView>> {
        let cached_covers = {
            let lock = self.steam_cover_cache.lock().unwrap_or_else(|e| e.into_inner());
            lock.clone()
        };

        let conn = self.db.lock().await;

        // Auto-correção preventiva: converte entradas de arquivos conhecidos salvas anteriormente como 'directory'
        let _ = conn.execute(
            "UPDATE save_locations 
             SET location_type = 'file' 
             WHERE location_type = 'directory' 
               AND (path LIKE '%.sl2' OR path LIKE '%.sav' OR path LIKE '%.dat' OR path LIKE '%.vdf' OR path LIKE '%.bin')",
            [],
        );

        let loc_repo = SaveLocationsRepo::new(&conn);
        let games_repo = GamesRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);

        // Mapeamento de último backup por save_location_id
        let mut last_backups: std::collections::HashMap<Uuid, String> = std::collections::HashMap::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT f.save_location_id, MAX(b.created_at)
             FROM backups b
             JOIN files f ON b.file_id = f.id
             GROUP BY f.save_location_id"
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                let loc_id_str: String = r.get(0)?;
                let created_at: String = r.get(1)?;
                Ok((loc_id_str, created_at))
            }) {
                for r in rows.flatten() {
                    if let Ok(uid) = Uuid::parse_str(&r.0) {
                        last_backups.insert(uid, r.1);
                    }
                }
            }
        }

        // Fallback: se o arquivo foi modificado/monitorado mas ainda não tem backup registrado
        if let Ok(mut stmt) = conn.prepare(
            "SELECT save_location_id, MAX(modified_at)
             FROM files
             GROUP BY save_location_id"
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                let loc_id_str: String = r.get(0)?;
                let mod_at: String = r.get(1)?;
                Ok((loc_id_str, mod_at))
            }) {
                for r in rows.flatten() {
                    if let Ok(uid) = Uuid::parse_str(&r.0) {
                        last_backups.entry(uid).or_insert(r.1);
                    }
                }
            }
        }

        let locations = loc_repo.list_all().unwrap_or_default();
        let mut views = Vec::new();

        for loc in locations {
            let game = games_repo.find_by_id(loc.game_id).unwrap_or(None);
            let mut game_name = game
                .as_ref()
                .map(|g| g.name.clone())
                .unwrap_or_else(|| "Desconhecido".into());

            // Auto-recuperação caso o nome armazenado anteriormente seja numérico (ex: pasta de slot "0")
            if game_name.chars().all(|c| c.is_ascii_digit()) || game_name == "0" {
                let inferred = savevault_discovery::generic::infer_game_name(&loc.path);
                if !inferred.is_empty() && !inferred.chars().all(|c| c.is_ascii_digit()) {
                    if let Some(ref g) = game {
                        let _ = games_repo.update_name(g.id, &inferred);
                    }
                    game_name = inferred;
                }
            }

            let platform = game
                .as_ref()
                .map(|g| format!("{:?}", g.platform))
                .unwrap_or_else(|| "Generic".into());

            let exists_on_disk = loc.path.exists();
            if loc.path.is_file() && loc.location_type != SaveLocationType::File {
                let _ = conn.execute(
                    "UPDATE save_locations SET location_type = 'file' WHERE id = ?1",
                    [loc.id.to_string()],
                );
            }
            let files = files_repo.list_by_location(loc.id).unwrap_or_default();

            // Resolução de capa do jogo:
            // 1. Se Steam com platform_id numérico, usa diretamente.
            // 2. Se Genérico ou sem platform_id, resolve pelo nome via dicionário de aliases ou cache.
            let steam_id = if let Some(ref g) = game {
                if g.platform == Platform::Steam {
                    g.platform_id
                        .as_deref()
                        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
                        .or_else(|| map_game_name_to_steam_id(&game_name))
                } else {
                    map_game_name_to_steam_id(&game_name)
                }
            } else {
                map_game_name_to_steam_id(&game_name)
            };

            let cover_url = if let Some(id) = steam_id {
                if let Some(cached_url) = cached_covers.get(id) {
                    Some(cached_url.clone())
                } else {
                    Some(format!(
                        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{}/header.jpg",
                        id
                    ))
                }
            } else {
                let norm_key = format!("name:{}", game_name.trim().to_ascii_lowercase());
                cached_covers.get(&norm_key).cloned()
            };

            let last_backup_time = last_backups.get(&loc.id).cloned();

            views.push(LocationView {
                id: loc.id,
                game_id: loc.game_id,
                game_name,
                platform,
                path: loc.path.to_string_lossy().to_string(),
                location_type: format!("{:?}", loc.location_type),
                exists_on_disk,
                monitored_files_count: files.len(),
                last_check: Utc::now().format("%H:%M:%S").to_string(),
                cover_url,
                last_backup_time,
            });
        }

        // Ordena por atividade mais recente (último backup/modificação decrescente)
        // Jogos sem backup ficam no final ordenados alfabeticamente
        views.sort_by(|a, b| {
            match (&b.last_backup_time, &a.last_backup_time) {
                (Some(tb), Some(ta)) => tb.cmp(ta),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.game_name.to_lowercase().cmp(&b.game_name.to_lowercase()),
            }
        });

        Ok(views)
    }

    /// Executa varredura sob demanda de um `SaveLocation` utilizando o `SaveScanner` existente.
    pub async fn scan_location(&self, location_id: Uuid) -> Result<ScanLocationResult> {
        let loc = {
            let conn = self.db.lock().await;
            let loc_repo = SaveLocationsRepo::new(&conn);
            loc_repo
                .find_by_id(location_id)?
                .ok_or_else(|| anyhow::anyhow!("SaveLocation '{}' não encontrado", location_id))?
        };

        let scanner = SaveScanner::default();
        let discovered = scanner.scan(&loc)?;

        let mut files_found = Vec::new();
        for f in discovered {
            let filename = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let size = f.size_bytes;
            let modified: DateTime<Utc> = f.modified_at.into();

            files_found.push(DiscoveredFileView {
                path: f.path.to_string_lossy().to_string(),
                filename,
                size_bytes: size,
                size_formatted: format_bytes(size),
                modified_at: modified.format("%Y-%m-%d %H:%M:%S").to_string(),
            });
        }

        self.record_event(
            DevUiEventLevel::Info,
            format!(
                "Scan manual concluído para location {}: {} arquivo(s) encontrado(s)",
                location_id,
                files_found.len()
            ),
            None,
        )
        .await;

        Ok(ScanLocationResult {
            location_id,
            files_found,
            scanned_at: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        })
    }

    /// Retorna a lista de todos os Snapshots cadastrados no cofre.
    pub async fn get_snapshots(&self) -> Result<Vec<SnapshotView>> {
        let conn = self.db.lock().await;
        let mut stmt = conn.prepare(
            "SELECT b.id, b.file_id, b.version, b.created_at, b.sync_status,
                    f.path, f.hash, f.size_bytes, g.name
             FROM backups b
             JOIN files f ON b.file_id = f.id
             JOIN save_locations sl ON f.save_location_id = sl.id
             JOIN games g ON sl.game_id = g.id
             ORDER BY b.created_at DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            let b_id: String = row.get(0)?;
            let f_id: String = row.get(1)?;
            let version: i64 = row.get(2)?;
            let created_at: String = row.get(3)?;
            let sync_status: String = row.get(4)?;
            let file_path: String = row.get(5)?;
            let hash: String = row.get(6)?;
            let size_bytes: i64 = row.get(7)?;
            let game_name: String = row.get(8)?;

            Ok((
                b_id,
                f_id,
                version,
                created_at,
                sync_status,
                file_path,
                hash,
                size_bytes,
                game_name,
            ))
        })?;

        let mut views = Vec::new();
        for r in rows {
            let (
                b_id,
                f_id,
                version,
                created_at,
                sync_status,
                file_path,
                hash,
                size_bytes,
                game_name,
            ) = r?;
            let snapshot_id = Uuid::parse_str(&b_id).unwrap_or_default();
            let file_id = Uuid::parse_str(&f_id).unwrap_or_default();

            let file_p = PathBuf::from(&file_path);
            let file_name = file_p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| file_path.clone());

            let safe_game_name = savevault_backup::snapshot::sanitize_name(&game_name);
            let physical_p = self
                .get_backup_root()
                .join(&safe_game_name)
                .join("latest")
                .join(&file_name);
            let physical_exists = physical_p.exists();

            let short_hash = if hash.len() > 12 {
                format!("{}...", &hash[..12])
            } else {
                hash.clone()
            };

            views.push(SnapshotView {
                id: snapshot_id,
                file_id,
                game_name,
                file_path: file_path.clone(),
                file_name,
                version: version as u32,
                size_bytes: size_bytes as u64,
                size_formatted: format_bytes(size_bytes as u64),
                hash,
                short_hash,
                created_at,
                physical_path: physical_p.to_string_lossy().to_string(),
                physical_exists,
                sync_status,
            });
        }

        Ok(views)
    }

    /// Descobre e agrupa Safety Backups existentes no disco para cada arquivo monitorado.
    pub async fn get_safety_backups(&self) -> Result<Vec<SafetyBackupGroup>> {
        let conn = self.db.lock().await;
        let mut stmt = conn.prepare(
            "SELECT f.id, f.path, g.name
             FROM files f
             JOIN save_locations sl ON f.save_location_id = sl.id
             JOIN games g ON sl.game_id = g.id",
        )?;

        let rows = stmt.query_map([], |row| {
            let f_id: String = row.get(0)?;
            let path: String = row.get(1)?;
            let game_name: String = row.get(2)?;
            Ok((f_id, path, game_name))
        })?;

        let mut groups = Vec::new();

        for r in rows {
            let (f_id, path_str, game_name) = r?;
            let file_id = Uuid::parse_str(&f_id).unwrap_or_default();
            let orig_path = PathBuf::from(&path_str);
            let orig_name = orig_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let mut backups = Vec::new();
            if let Some(parent) = orig_path.parent() {
                if parent.exists() {
                    if let Ok(entries) = std::fs::read_dir(parent) {
                        for entry in entries.flatten() {
                            let fname = entry.file_name().to_string_lossy().to_string();
                            if fname.starts_with(&orig_name) && fname.contains(".savevault_safety_")
                            {
                                let meta = entry.metadata().ok();
                                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                                let modified = meta
                                    .and_then(|m| m.modified().ok())
                                    .map(|st| {
                                        let dt: DateTime<Utc> = st.into();
                                        dt.format("%Y-%m-%d %H:%M:%S").to_string()
                                    })
                                    .unwrap_or_else(|| "Desconhecido".into());

                                backups.push(SafetyBackupItem {
                                    filename: fname,
                                    path: entry.path().to_string_lossy().to_string(),
                                    size_bytes: size,
                                    size_formatted: format_bytes(size),
                                    modified_at: modified,
                                });
                            }
                        }
                    }
                }
            }

            if !backups.is_empty() {
                groups.push(SafetyBackupGroup {
                    original_file_id: file_id,
                    original_path: path_str,
                    original_name: orig_name,
                    game_name,
                    backups,
                });
            }
        }

        Ok(groups)
    }

    /// Retorna as estatísticas e status atual do Reconciliador.
    pub async fn get_reconciliation_status(&self) -> Result<ReconciliationView> {
        let is_running = if let Some(ref r) = self.reconciler {
            r.is_reconciling()
        } else {
            false
        };

        let last_time = *self.last_reconcile_time.lock().await;
        let last_run = last_time.map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string());
        let next_run = last_time.map(|t| {
            let next =
                t + chrono::Duration::from_std(self.reconciliation_interval).unwrap_or_default();
            next.format("%H:%M:%S UTC").to_string()
        });

        let rep = self
            .last_reconciliation_report
            .lock()
            .await
            .clone()
            .unwrap_or_default();

        let status = if is_running {
            "Running".to_string()
        } else {
            "Idle".to_string()
        };

        Ok(ReconciliationView {
            status,
            interval_secs: self.reconciliation_interval.as_secs(),
            is_running,
            last_run,
            next_run,
            locations_scanned: rep.locations_scanned,
            files_seen: rep.files_seen,
            files_added: rep.files_added,
            files_changed: rep.files_changed,
            files_removed: rep.files_removed,
            errors: rep.errors,
            duration_ms: rep.duration.as_millis() as u64,
        })
    }

    /// Aciona um ciclo de reconciliação imediatamente utilizando o `Reconciler` real.
    pub async fn trigger_reconcile(&self) -> Result<ReconciliationReport, ReconciliationError> {
        let reconciler = self.reconciler.as_ref().ok_or_else(|| {
            ReconciliationError::Pipeline(anyhow::anyhow!("Reconciliador não inicializado"))
        })?;

        self.record_event(
            DevUiEventLevel::Info,
            "Ciclo de reconciliação sob demanda iniciado pela DEV-UI".to_string(),
            None,
        )
        .await;

        let res = reconciler.reconcile().await;
        match res {
            Ok(report) => {
                {
                    let mut last_rep = self.last_reconciliation_report.lock().await;
                    *last_rep = Some(report.clone());
                    let mut last_time = self.last_reconcile_time.lock().await;
                    *last_time = Some(Utc::now());
                }

                self.record_event(
                    DevUiEventLevel::Success,
                    format!(
                        "Reconciliação concluída: {} locais, {} arquivos vistos (+{} novos, ~{} modificados, -{} removidos, {} erros) em {:?}",
                        report.locations_scanned, report.files_seen, report.files_added,
                        report.files_changed, report.files_removed, report.errors, report.duration
                    ),
                    None,
                ).await;

                Ok(report)
            }
            Err(e) => {
                let err_msg = format!("Falha na reconciliação: {}", e);
                self.record_error(&err_msg).await;
                Err(e)
            }
        }
    }

    /// Executa a restauração de um snapshot com safety backup através da API real.
    pub async fn trigger_restore(
        &self,
        snapshot_id: Uuid,
        expected_hash: Option<String>,
    ) -> RestoreApiResponse {
        let req = match expected_hash {
            Some(h) => RestoreRequest::with_expected_hash(snapshot_id, h),
            None => RestoreRequest::new(snapshot_id),
        };

        self.record_event(
            DevUiEventLevel::Info,
            format!(
                "Iniciando restauração do snapshot {} via DEV-UI",
                snapshot_id
            ),
            None,
        )
        .await;

        let res = restore_snapshot_with_db(&self.db, &self.get_backup_root(), &req).await;

        match res {
            Ok(report) => {
                let safety_str = report
                    .safety_backup_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());
                let msg = format!(
                    "Restauração concluída com sucesso: versão {} de {} restaurada em {}",
                    report.version,
                    report.file_id,
                    report.dest_path.display()
                );

                self.record_event(
                    DevUiEventLevel::Success,
                    msg,
                    Some(format!(
                        "Hash: {}, Bytes: {}",
                        report.restored_hash, report.bytes_restored
                    )),
                )
                .await;

                RestoreApiResponse {
                    success: true,
                    snapshot_id: Some(report.snapshot_id),
                    version: Some(report.version),
                    dest_path: Some(report.dest_path.to_string_lossy().to_string()),
                    safety_backup_path: safety_str,
                    restored_hash: Some(report.restored_hash),
                    bytes_restored: Some(report.bytes_restored),
                    error: None,
                    details: None,
                }
            }
            Err(err) => {
                let (short_err, details) = match &err {
                    RestoreError::SnapshotNotFound(id) => (
                        format!("Snapshot '{}' não encontrado no banco de dados.", id),
                        "O identificador UUID informado não existe na tabela backups.".to_string(),
                    ),
                    RestoreError::PhysicalSnapshotMissing(path) => (
                        format!("Arquivo físico do snapshot ausente em '{}'.", path.display()),
                        "O arquivo de backup foi apagado ou movido do cofre físico.".to_string(),
                    ),
                    RestoreError::IntegrityCheckFailed(id, exp, act) => (
                        format!("Integridade violada para o snapshot '{}'!", id),
                        format!("O hash BLAKE3 real ('{}') diverge do esperado ('{}'). Restauração abortada para segurança.", act, exp),
                    ),
                    RestoreError::PathSecurityViolation(reason) => (
                        "Violação de segurança de caminho detectada!".to_string(),
                        format!("A tentativa de restauração tentou escapar do SaveLocation autorizado: {}", reason),
                    ),
                    RestoreError::SafetyBackupFailed(path, e) => (
                        format!("Falha ao criar Safety Backup em '{}'.", path.display()),
                        format!("Não foi possível duplicar o arquivo atual antes da substituição: {}. O save ativo não foi alterado.", e),
                    ),
                    RestoreError::AtomicWriteFailed(path, e) => (
                        format!("Falha na escrita atômica em '{}'.", path.display()),
                        format!("Erro durante staging ou rename: {}. O arquivo original permaneceu intacto.", e),
                    ),
                    other => (
                        "Falha durante a operação de restauração.".to_string(),
                        other.to_string(),
                    ),
                };

                self.record_event(
                    DevUiEventLevel::Error,
                    format!("Restauração falhou: {}", short_err),
                    Some(details.clone()),
                )
                .await;

                RestoreApiResponse {
                    success: false,
                    snapshot_id: Some(snapshot_id),
                    version: None,
                    dest_path: None,
                    safety_backup_path: None,
                    restored_hash: None,
                    bytes_restored: None,
                    error: Some(short_err),
                    details: Some(format!("{}\nO arquivo original foi preservado.", details)),
                }
            }
        }
    }

    fn config_path(&self) -> PathBuf {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        let checkpoint_cfg = PathBuf::from(&appdata).join("checkpoint").join("config.toml");
        if checkpoint_cfg.exists() {
            return checkpoint_cfg;
        }
        let legacy_cfg = PathBuf::from(&appdata).join("SaveVault").join("config.toml");
        if legacy_cfg.exists() {
            return legacy_cfg;
        }
        checkpoint_cfg
    }

    /// Cadastra um jogo e local de save manualmente através da UI.
    pub async fn add_game(&self, name: &str, path_str: &str) -> Result<LocationView> {
        let raw_path = PathBuf::from(path_str);
        let location_type = if raw_path.is_file() {
            SaveLocationType::File
        } else {
            SaveLocationType::Directory
        };

        let (game_id, loc) = {
            let conn = self.db.lock().await;
            let games_repo = GamesRepo::new(&conn);
            let locations_repo = SaveLocationsRepo::new(&conn);

            let (game_id, _game) = if let Ok(Some(existing)) =
                games_repo.find_by_name_and_platform(name, &Platform::Generic)
            {
                (existing.id, existing)
            } else {
                let game = Game::new(name, Platform::Generic, None, Confidence::High);
                games_repo.insert(&game)?;
                (game.id, game)
            };

            let loc = SaveLocation::new(game_id, raw_path.clone(), location_type);
            let saved_loc = locations_repo.upsert(&loc)?;

            (game_id, saved_loc)
        };

        self.record_event(
            DevUiEventLevel::Success,
            format!("Jogo manual '{}' cadastrado com sucesso", name),
            Some(raw_path.display().to_string()),
        )
        .await;

        Ok(LocationView {
            id: loc.id,
            game_id,
            game_name: name.to_string(),
            platform: "Generic".to_string(),
            path: raw_path.display().to_string(),
            location_type: format!("{:?}", loc.location_type),
            exists_on_disk: raw_path.exists(),
            monitored_files_count: 0,
            last_check: Utc::now().format("%H:%M:%S").to_string(),
            cover_url: None,
            last_backup_time: None,
        })
    }

    /// Retorna as configurações atuais de nuvem com auto-detecção de OneDrive e serviços locais.
    pub async fn get_cloud_config(&self) -> Result<CloudConfigView> {
        let config_path = self.config_path();
        let config = savevault_core::config::Config::load(&config_path).unwrap_or_default();
        let detected = savevault_cloud::detect_cloud_folders();
        let local_path = config
            .cloud
            .local_sync_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| {
                // Se não há caminho configurado, usa a pasta do OneDrive detectada automaticamente como default inteligente
                detected
                    .iter()
                    .find(|d| d.provider_type == "onedrive")
                    .or_else(|| detected.first())
                    .map(|d| d.savevault_folder.display().to_string())
                    .unwrap_or_default()
            });

        let effective_provider = if config.cloud.provider == "none" {
            "none".to_string()
        } else if !local_path.is_empty() {
            let lower = local_path.to_lowercase();
            if lower.contains("onedrive") {
                "onedrive".to_string()
            } else if lower.contains("google")
                || lower.contains("drive")
                || lower.contains("meu drive")
                || lower.starts_with("g:")
                || lower.starts_with("h:")
                || detected.iter().any(|d| {
                    d.provider_type == "google_drive"
                        && (lower.starts_with(&d.sync_folder.to_string_lossy().to_lowercase())
                            || lower.starts_with(&d.savevault_folder.to_string_lossy().to_lowercase()))
                })
            {
                "google_drive".to_string()
            } else {
                config.cloud.provider
            }
        } else {
            config.cloud.provider
        };

        let enabled = effective_provider != "none" && !local_path.is_empty();

        Ok(CloudConfigView {
            provider: effective_provider,
            local_sync_path: local_path,
            enabled,
            detected_folders: detected,
        })
    }

    /// Atualiza as configurações de nuvem e notifica o SyncWorker imediatamente.
    pub async fn update_cloud_config(
        &self,
        provider: Option<String>,
        local_sync_path: Option<String>,
        enabled: Option<bool>,
    ) -> Result<CloudConfigView> {
        let config_path = self.config_path();
        let mut config = savevault_core::config::Config::load(&config_path).unwrap_or_default();

        if let Some(p) = provider {
            config.cloud.provider = p;
        }

        if let Some(lsp) = local_sync_path {
            let trimmed = lsp.trim();
            if !trimmed.is_empty() {
                let pb = PathBuf::from(trimmed);
                let _ = std::fs::create_dir_all(&pb);
                savevault_cloud::auto_migrate_legacy_cloud_folder(&pb);
                config.cloud.local_sync_path = Some(pb);
            }
        }

        if let Some(en) = enabled {
            if !en {
                config.cloud.provider = "none".to_string();
            } else if config.cloud.provider == "none" {
                config.cloud.provider = "onedrive".to_string();
            }
        }

        config.save(&config_path)?;

        // Notifica o worker em tempo de execução
        if let Some(ref worker) = *self.sync_worker.lock().await {
            worker.update_backup_root(self.get_backup_root());
            worker
                .update_config(crate::sync_worker::SyncWorkerConfig {
                    provider: config.cloud.provider.clone(),
                    local_sync_path: config.cloud.local_sync_path.clone(),
                    reconciliation_interval: std::time::Duration::from_secs(60),
                    max_attempts: 5,
                    base_backoff_ms: 2000,
                })
                .await;
            worker.notify_new_item();
        }

        self.record_event(
            DevUiEventLevel::Info,
            "Configurações de nuvem atualizadas".to_string(),
            Some(format!(
                "Provedor: {}, Pasta: {:?}",
                config.cloud.provider, config.cloud.local_sync_path
            )),
        )
        .await;

        self.get_cloud_config().await
    }

    /// Atualiza as configurações gerais e de diretórios, garantindo sincronização imediata
    /// com Reconciler e SyncWorker em tempo de execução sem reiniciar o app.
    pub async fn update_config(&self, new_cfg: ConfigDto) -> Result<String> {
        let config_path = self.config_path();
        let mut config = savevault_core::config::Config::load(&config_path).unwrap_or_default();

        config.general.max_versions = new_cfg.max_versions.max(1);
        config.general.debounce_ms = new_cfg.debounce_ms.max(100);
        config.general.reconciliation_interval_secs = new_cfg.reconciliation_interval_secs.max(5);
        config.general.log_level = new_cfg.log_level;

        let trimmed_backup = new_cfg.backup_dir.trim();
        if !trimmed_backup.is_empty() {
            let pb = PathBuf::from(trimmed_backup);
            let _ = std::fs::create_dir_all(&pb);

            // Migração automática transparente de pastas legadas (SaveVault -> checkpoint)
            savevault_cloud::auto_migrate_legacy_cloud_folder(&pb);

            config.paths.backup_dir = pb.clone();
            self.update_backup_root(pb.clone());

            // Se o usuário apontou a pasta de backups para o Google Drive ou OneDrive,
            // sincroniza também a pasta de cloud sync para garantir coerência completa
            let lower = trimmed_backup.to_lowercase();
            let is_cloud_target = lower.contains("google")
                || lower.contains("drive")
                || lower.contains("meu drive")
                || lower.starts_with("g:")
                || lower.starts_with("h:")
                || lower.contains("onedrive");

            if is_cloud_target {
                config.cloud.local_sync_path = Some(pb.clone());
                if lower.contains("onedrive") {
                    config.cloud.provider = "onedrive".to_string();
                } else {
                    config.cloud.provider = "google_drive".to_string();
                }
            }
        }

        if let Some(csp) = new_cfg.cloud_sync_path {
            let trimmed_csp = csp.trim();
            if !trimmed_csp.is_empty() {
                let pb_csp = PathBuf::from(trimmed_csp);
                let _ = std::fs::create_dir_all(&pb_csp);
                savevault_cloud::auto_migrate_legacy_cloud_folder(&pb_csp);
                config.cloud.local_sync_path = Some(pb_csp);
            }
        }

        config.save(&config_path)?;

        if new_cfg.autostart {
            let _ = savevault_core::autostart::enable(None);
        } else {
            let _ = savevault_core::autostart::disable();
        }

        // Atualiza e notifica SyncWorker em tempo de execução
        if let Some(ref worker) = *self.sync_worker.lock().await {
            worker.update_backup_root(self.get_backup_root());
            worker
                .update_config(crate::sync_worker::SyncWorkerConfig {
                    provider: config.cloud.provider.clone(),
                    local_sync_path: config.cloud.local_sync_path.clone(),
                    reconciliation_interval: std::time::Duration::from_secs(60),
                    max_attempts: 5,
                    base_backoff_ms: 2000,
                })
                .await;
            worker.notify_new_item();
        }

        self.record_event(
            DevUiEventLevel::Info,
            "Configurações atualizadas e aplicadas".to_string(),
            Some(format!(
                "Backups: {}, Nuvem: {:?}",
                config.paths.backup_dir.display(),
                config.cloud.local_sync_path
            )),
        )
        .await;

        Ok("Configurações salvas e sincronizadas com sucesso!".to_string())
    }

    /// Executa manutenção preventiva no banco de dados SQLite (pruning de sync_queue antigas e VACUUM).
    pub async fn optimize_database(&self) -> Result<OptimizationResult> {
        let mut purged = 0;
        {
            let db = self.db.lock().await;
            let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
            let deleted = db.execute(
                "DELETE FROM sync_queue WHERE status = 'synced' OR (attempts >= 5 AND created_at < ?1)",
                rusqlite::params![cutoff.to_rfc3339()],
            )?;
            purged += deleted;
            db.execute("VACUUM", [])?;
        }

        let size = std::fs::metadata(&self.db_path).map(|m| m.len()).unwrap_or(0);
        let size_formatted = format_bytes(size);

        self.record_event(
            DevUiEventLevel::Success,
            format!(
                "Otimização do banco concluída. {} itens antigos limpos. Tamanho atual: {}",
                purged, size_formatted
            ),
            None,
        )
        .await;

        Ok(OptimizationResult {
            purged_sync_items: purged,
            db_size_bytes: size,
            db_size_formatted: size_formatted,
        })
    }

    /// Cria um backup completo compactado (.ZIP) de todos os saves monitorados e snapshots.
    pub async fn export_backup_zip(&self) -> Result<ExportResult> {
        let backup_dir = self.get_backup_root();

        if !backup_dir.exists() {
            anyhow::bail!("O diretório de backups ainda não foi criado ou não contém dados.");
        }

        // Determina destino padrão (Downloads do usuário ou diretório corrente)
        let downloads_dir = std::env::var("USERPROFILE")
            .map(|p| std::path::PathBuf::from(p).join("Downloads"))
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());

        if !downloads_dir.exists() {
            let _ = std::fs::create_dir_all(&downloads_dir);
        }

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
        let zip_file_name = format!("checkpoint_backup_{}.zip", timestamp);
        let target_zip = downloads_dir.join(&zip_file_name);

        #[cfg(target_os = "windows")]
        {
            let output = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!(
                        "Compress-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                        backup_dir.display(),
                        target_zip.display()
                    ),
                ])
                .output()?;

            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("Falha ao gerar arquivo ZIP: {}", err);
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            let output = std::process::Command::new("zip")
                .args([
                    "-r",
                    target_zip.to_str().unwrap(),
                    backup_dir.to_str().unwrap(),
                ])
                .output()?;
            if !output.status.success() {
                anyhow::bail!("Falha ao compactar arquivo no Linux/macOS");
            }
        }

        let size = std::fs::metadata(&target_zip).map(|m| m.len()).unwrap_or(0);
        let size_formatted = format_bytes(size);

        self.record_event(
            DevUiEventLevel::Success,
            format!(
                "Backup manual exportado com sucesso: {} ({})",
                zip_file_name, size_formatted
            ),
            Some(target_zip.display().to_string()),
        )
        .await;

        Ok(ExportResult {
            export_path: target_zip.display().to_string(),
            file_size_bytes: size,
            file_size_formatted: size_formatted,
        })
    }
}

/// Formata bytes de forma legível.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
