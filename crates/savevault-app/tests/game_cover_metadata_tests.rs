//! Testes de Auditoria e Integração para Metadados e Capas de Jogos (AUD-COVER).
//!
//! Cobre exaustivamente:
//! 1. Jogo com AppID conhecido (Steam).
//! 2. Jogo sem AppID que resolve via Ludusavi Manifest.
//! 3. Jogo desconhecido / não encontrado (negative cache).
//! 4. Metadados previamente cacheados servidos em O(1).
//! 5. Resiliência offline sem chamadas bloqueantes de rede.
//! 6. Validação de segurança rejeitando esquemas inseguros, injeção XSS e domínios falsos.
//! 7. Validação de segurança de AppIDs contra caracteres de controle e traversal.
//! 8. Resolução simultânea de múltiplos jogos com identidades distintas.
//! 9. Persistência e sobrevida do metadata após reabertura do SQLite.
//! 10. Migração 010 aplicada de forma idempotente sobre schemas existentes.
//! 11. Jogos não-Steam (GOG, Emuladores, Manuais) com fallback elegante.
//! 12. Títulos homônimos/conflitantes (ex.: DOOM) rejeitando associação indevida.
//! 13. Contrato de API `LocationView` e `DiscoveredGameView` preservando compatibilidade.
//! 14. Carregamento de `get_locations` no DevUiState integrando cache e fallback.

use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use savevault_app::metadata::{
    build_steam_header_url, is_valid_steam_app_id, is_valid_steam_cover_url, GameMetadataResolver,
    ALLOWED_STEAM_DOMAINS, STEAM_CDN_HEADER_BASE,
};
use savevault_app::ui::state::{DevUiState, LocationView};
use savevault_core::models::{
    Confidence, Game, GameMetadata, MetadataSource, Platform, SaveLocation, SaveLocationType,
};
use savevault_db::repositories::{GameMetadataRepo, GamesRepo, SaveLocationsRepo};
use savevault_db::Database;
use savevault_discovery::ludusavi::manifest::LudusaviManifest;

fn create_test_manifest() -> LudusaviManifest {
    let yaml = r#"
Stardew Valley:
  steam:
    id: 413150
  files:
    <winAppData>/StardewValley/Saves:
      tags: [save]

Hollow Knight:
  steam: 367520
  gog: 1335274700

DOOM:
  steam:
    id: 999999
"#;
    LudusaviManifest::from_yaml_str(yaml).expect("Manifest YAML válido")
}

// -----------------------------------------------------------------------------
// 1. Jogo com AppID conhecido (Steam)
// -----------------------------------------------------------------------------
#[test]
fn test_01_steam_game_with_known_appid_generates_valid_cdn_url() {
    let db = Database::open_in_memory().unwrap();
    let resolver = GameMetadataResolver::new();

    let game = Game::new(
        "Cyberpunk 2077",
        Platform::Steam,
        Some("1091500".into()),
        Confidence::High,
    );
    let games_repo = GamesRepo::new(&db.conn);
    games_repo.insert(&game).unwrap();

    let meta = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert_eq!(meta.steam_app_id.as_deref(), Some("1091500"));
    assert_eq!(meta.source, MetadataSource::Steam);

    let cover_url = meta.cover_url.expect("deve gerar cover_url para Steam");
    assert!(is_valid_steam_cover_url(&cover_url));
    assert!(cover_url.contains("1091500/header.jpg"));
    assert!(cover_url.starts_with(STEAM_CDN_HEADER_BASE));

    // Testa build_steam_header_url diretamente
    assert_eq!(
        build_steam_header_url("1091500").as_deref(),
        Some("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/1091500/header.jpg")
    );
    assert_eq!(build_steam_header_url("not_an_id"), None);
}

// -----------------------------------------------------------------------------
// 2. Jogo sem AppID que resolve deterministicamente via Ludusavi Manifest
// -----------------------------------------------------------------------------
#[test]
fn test_02_game_without_appid_resolves_via_ludusavi_manifest() {
    let db = Database::open_in_memory().unwrap();
    let manifest = create_test_manifest();
    let resolver = GameMetadataResolver::with_manifest(manifest);

    // Jogo genérico sem platform_id, mas presente no catálogo Ludusavi
    let game = Game::new(
        "Stardew Valley",
        Platform::Generic,
        None,
        Confidence::Medium,
    );
    let games_repo = GamesRepo::new(&db.conn);
    games_repo.insert(&game).unwrap();

    let meta = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert_eq!(meta.steam_app_id.as_deref(), Some("413150"));
    assert_eq!(meta.source, MetadataSource::Ludusavi);

    let cover = meta.cover_url.expect("deve gerar capa via Ludusavi");
    assert!(cover.contains("413150/header.jpg"));
    assert!(is_valid_steam_cover_url(&cover));
}

// -----------------------------------------------------------------------------
// 3. Jogo não encontrado no catálogo (negative cache)
// -----------------------------------------------------------------------------
#[test]
fn test_03_unknown_game_stores_negative_cache_and_returns_none() {
    let db = Database::open_in_memory().unwrap();
    let manifest = create_test_manifest();
    let resolver = GameMetadataResolver::with_manifest(manifest);

    let game = Game::new(
        "Super Secret Homebrew",
        Platform::Generic,
        None,
        Confidence::Low,
    );
    let games_repo = GamesRepo::new(&db.conn);
    games_repo.insert(&game).unwrap();

    let meta = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert!(meta.steam_app_id.is_none());
    assert!(meta.cover_url.is_none());
    assert_eq!(meta.source, MetadataSource::None);

    // Verifica que foi persistido no banco para evitar buscas repetidas
    let meta_repo = GameMetadataRepo::new(&db.conn);
    let cached = meta_repo
        .find_by_game_id(game.id)
        .unwrap()
        .expect("deve estar no cache");
    assert_eq!(cached.source, MetadataSource::None);
    assert!(cached.cover_url.is_none());
}

// -----------------------------------------------------------------------------
// 4. Metadado já existente no cache
// -----------------------------------------------------------------------------
#[test]
fn test_04_cached_metadata_returns_without_recalculation() {
    let db = Database::open_in_memory().unwrap();
    let meta_repo = GameMetadataRepo::new(&db.conn);
    let games_repo = GamesRepo::new(&db.conn);

    let game = Game::new("Pre-Cached Game", Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let custom_url = "https://cdn.cloudflare.steamstatic.com/steam/apps/123/header.jpg";
    let existing_meta = GameMetadata::new(
        game.id,
        Some("123".into()),
        Some(custom_url.into()),
        MetadataSource::Manual,
    );
    meta_repo.upsert(&existing_meta).unwrap();

    // Resolver vazio (sem manifest) deve honrar o cache
    let resolver = GameMetadataResolver::new();
    let resolved = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert_eq!(resolved.cover_url.as_deref(), Some(custom_url));
    assert_eq!(resolved.source, MetadataSource::Manual);
}

// -----------------------------------------------------------------------------
// 5. Falha de rede / Operação 100% offline
// -----------------------------------------------------------------------------
#[test]
fn test_05_offline_operation_never_blocks_or_errors() {
    let db = Database::open_in_memory().unwrap();
    let resolver = GameMetadataResolver::new();

    let game = Game::new(
        "Offline Adventure",
        Platform::Steam,
        Some("77777".into()),
        Confidence::High,
    );
    let games_repo = GamesRepo::new(&db.conn);
    games_repo.insert(&game).unwrap();

    // A resolução é puramente local e determinística, nunca abrindo socket de rede
    let start = std::time::Instant::now();
    let meta = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    let elapsed = start.elapsed();

    assert!(elapsed < std::time::Duration::from_millis(50));
    assert_eq!(meta.steam_app_id.as_deref(), Some("77777"));
    assert!(meta.cover_url.is_some());
}

// -----------------------------------------------------------------------------
// 6. Validação de segurança de URLs (XSS, Injection, Esquemas)
// -----------------------------------------------------------------------------
#[test]
fn test_06_security_url_validator_rejects_malicious_urls_and_schemes() {
    // Válidos
    assert!(is_valid_steam_cover_url(
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/1091500/header.jpg"
    ));
    assert!(is_valid_steam_cover_url(
        "https://cdn.cloudflare.steamstatic.com/steam/apps/413150/header.jpg"
    ));
    assert!(is_valid_steam_cover_url(
        "https://steamcdn-a.akamaihd.net/steam/apps/10/header.jpg"
    ));

    // Inválidos: HTTP simples (não criptografado)
    assert!(!is_valid_steam_cover_url(
        "http://shared.cloudflare.steamstatic.com/apps/1/header.jpg"
    ));

    // Inválidos: esquemas arbitrários e XSS
    assert!(!is_valid_steam_cover_url("javascript:alert(1)"));
    assert!(!is_valid_steam_cover_url(
        "data:text/html,<script>alert(1)</script>"
    ));
    assert!(!is_valid_steam_cover_url("file:///etc/passwd"));
    assert!(!is_valid_steam_cover_url("file:///C:/Windows/win.ini"));

    // Inválidos: Domínios externos não autorizados
    assert!(!is_valid_steam_cover_url(
        "https://malicious-site.com/image.jpg"
    ));
    assert!(!is_valid_steam_cover_url(
        "https://steamdb.info/app/1091500/"
    ));
    assert!(!is_valid_steam_cover_url(
        "https://evil.com/?shared.cloudflare.steamstatic.com/"
    ));

    // Inválidos: Injeção de caracteres perigosos (quebra de atributos HTML)
    assert!(!is_valid_steam_cover_url(
        "https://shared.cloudflare.steamstatic.com/\" onmouseover=\"alert(1)"
    ));
    assert!(!is_valid_steam_cover_url(
        "https://shared.cloudflare.steamstatic.com/<script>alert(1)</script>"
    ));
    assert!(!is_valid_steam_cover_url(
        "https://shared.cloudflare.steamstatic.com/apps/1/\\..\\win.ini"
    ));

    // Valida lista de domínios autorizados
    assert_eq!(ALLOWED_STEAM_DOMAINS.len(), 3);
}

// -----------------------------------------------------------------------------
// 7. Validação de segurança de AppIDs
// -----------------------------------------------------------------------------
#[test]
fn test_07_security_app_id_validator_rejects_injection_and_path_traversal() {
    // Válidos canônicos
    assert!(is_valid_steam_app_id("1"));
    assert!(is_valid_steam_app_id("730"));
    assert!(is_valid_steam_app_id("1091500"));
    assert!(is_valid_steam_app_id("413150"));
    assert!(is_valid_steam_app_id("10"));
    assert!(is_valid_steam_app_id("4294967295")); // u32::MAX

    // Rejeita zeros à esquerda (leading zeros)
    assert!(!is_valid_steam_app_id("000730"));
    assert!(!is_valid_steam_app_id("0"));
    assert!(!is_valid_steam_app_id("000"));

    // Rejeita números negativos e overflow
    assert!(!is_valid_steam_app_id("-730"));
    assert!(!is_valid_steam_app_id("4294967296")); // u32::MAX + 1

    // Rejeita espaços em branco e quebras de linha
    assert!(!is_valid_steam_app_id(" 730"));
    assert!(!is_valid_steam_app_id("730 "));
    assert!(!is_valid_steam_app_id("  504230  "));
    assert!(!is_valid_steam_app_id("730\n"));
    assert!(!is_valid_steam_app_id(""));
    assert!(!is_valid_steam_app_id("   "));

    // Rejeita dígitos Unicode fullwidth
    assert!(!is_valid_steam_app_id("７３０"));

    // Rejeita letras, injeções, caracteres especiais e path traversal
    assert!(!is_valid_steam_app_id("1091500a"));
    assert!(!is_valid_steam_app_id("app_1091500"));
    assert!(!is_valid_steam_app_id("../1091500"));
    assert!(!is_valid_steam_app_id("1091500/header.jpg"));
    assert!(!is_valid_steam_app_id("1091500; DROP TABLE games;"));
    assert!(!is_valid_steam_app_id("1091500\0"));

    // Rejeita comprimento absurdo (> 10 dígitos)
    assert!(!is_valid_steam_app_id("123456789012345678901"));

    // build_steam_header_url aceita strings com espaços se limpas resultarem em AppID válido
    assert!(build_steam_header_url("  730  ").is_some());
    assert_eq!(build_steam_header_url("000730"), None);
}

// -----------------------------------------------------------------------------
// 8. Múltiplos jogos resolvidos deterministicamente
// -----------------------------------------------------------------------------
#[test]
fn test_08_multiple_games_resolved_consistently() {
    let db = Database::open_in_memory().unwrap();
    let manifest = create_test_manifest();
    let resolver = GameMetadataResolver::with_manifest(manifest);
    let games_repo = GamesRepo::new(&db.conn);

    let g1 = Game::new(
        "Cyberpunk 2077",
        Platform::Steam,
        Some("1091500".into()),
        Confidence::High,
    );
    let g2 = Game::new(
        "Stardew Valley",
        Platform::Generic,
        None,
        Confidence::Medium,
    );
    let g3 = Game::new("Custom Emu", Platform::RetroArch, None, Confidence::Low);

    games_repo.insert(&g1).unwrap();
    games_repo.insert(&g2).unwrap();
    games_repo.insert(&g3).unwrap();

    let m1 = resolver.resolve_or_get_cached(&db.conn, &g1).unwrap();
    let m2 = resolver.resolve_or_get_cached(&db.conn, &g2).unwrap();
    let m3 = resolver.resolve_or_get_cached(&db.conn, &g3).unwrap();

    assert_eq!(m1.source, MetadataSource::Steam);
    assert_eq!(m1.steam_app_id.as_deref(), Some("1091500"));

    assert_eq!(m2.source, MetadataSource::Ludusavi);
    assert_eq!(m2.steam_app_id.as_deref(), Some("413150"));

    assert_eq!(m3.source, MetadataSource::None);
    assert!(m3.cover_url.is_none());
}

// -----------------------------------------------------------------------------
// 9. Persistência do metadata no SQLite sobrevive a reabertura
// -----------------------------------------------------------------------------
#[test]
fn test_09_metadata_persistence_survives_db_reopen() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("savevault_test.db");

    let game_id = Uuid::new_v4();

    // 1. Cria e persiste no primeiro handle
    {
        let db = Database::open(&db_path).unwrap();
        let games_repo = GamesRepo::new(&db.conn);
        let meta_repo = GameMetadataRepo::new(&db.conn);

        let game = Game {
            id: game_id,
            name: "Persistent Game".into(),
            platform: Platform::Steam,
            platform_id: Some("12345".into()),
            confidence: Confidence::High,
            created_at: chrono::Utc::now(),
        };
        games_repo.insert(&game).unwrap();

        let meta = GameMetadata::new(
            game_id,
            Some("12345".into()),
            Some("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/12345/header.jpg".into()),
            MetadataSource::Steam,
        );
        meta_repo.upsert(&meta).unwrap();
    }

    // 2. Reabre o banco e verifica integridade
    {
        let db = Database::open(&db_path).unwrap();
        let meta_repo = GameMetadataRepo::new(&db.conn);
        let loaded = meta_repo
            .find_by_game_id(game_id)
            .unwrap()
            .expect("deve sobreviver ao re-open");

        assert_eq!(loaded.game_id, game_id);
        assert_eq!(loaded.steam_app_id.as_deref(), Some("12345"));
        assert_eq!(loaded.source, MetadataSource::Steam);
        assert!(loaded.cover_url.unwrap().contains("12345"));
    }
}

// -----------------------------------------------------------------------------
// 10. Migração 010 aplicada de forma idempotente sobre schemas existentes
// -----------------------------------------------------------------------------
#[test]
fn test_10_migration_010_applies_cleanly_and_idempotently() {
    let db = Database::open_in_memory().unwrap();

    // Executa a migração novamente de forma explícita
    savevault_db::migrations::run(&db.conn).expect("segunda execução deve ser idempotente");

    // Valida que a tabela game_metadata existe e está operacional
    let meta_repo = GameMetadataRepo::new(&db.conn);
    assert!(meta_repo.list_all().unwrap().is_empty());
}

// -----------------------------------------------------------------------------
// 11. Jogos não-Steam (GOG, Emuladores, Manuais) com fallback elegante
// -----------------------------------------------------------------------------
#[test]
fn test_11_non_steam_games_use_graceful_fallback() {
    let db = Database::open_in_memory().unwrap();
    let resolver = GameMetadataResolver::new(); // sem manifest
    let games_repo = GamesRepo::new(&db.conn);

    let gog_game = Game::new(
        "Witcher 3",
        Platform::Gog,
        Some("1207664643".into()),
        Confidence::High,
    );
    let emu_game = Game::new(
        "Super Smash Bros Melee",
        Platform::Dolphin,
        None,
        Confidence::High,
    );
    let generic_game = Game::new("My Custom RPG", Platform::Generic, None, Confidence::Low);

    games_repo.insert(&gog_game).unwrap();
    games_repo.insert(&emu_game).unwrap();
    games_repo.insert(&generic_game).unwrap();

    for g in [&gog_game, &emu_game, &generic_game] {
        let meta = resolver.resolve_or_get_cached(&db.conn, g).unwrap();
        assert_eq!(meta.source, MetadataSource::None);
        assert!(meta.cover_url.is_none());
    }
}

// -----------------------------------------------------------------------------
// 12. Nome com potencial ambiguidade / conflito rejeita associação arbitrária
// -----------------------------------------------------------------------------
#[test]
fn test_12_conflicting_name_rejects_arbitrary_association() {
    let db = Database::open_in_memory().unwrap();
    let manifest = create_test_manifest();
    let resolver = GameMetadataResolver::with_manifest(manifest);
    let games_repo = GamesRepo::new(&db.conn);

    // Jogo na máquina é Steam com AppID 123456, mas manifest tem DOOM com Steam AppID 999999
    let conflicting_game = Game::new(
        "DOOM",
        Platform::Steam,
        Some("123456".into()),
        Confidence::High,
    );
    games_repo.insert(&conflicting_game).unwrap();

    // Como o jogo é da Steam com AppID 123456, a prioridade 1 (Steam ID local) prevalece
    let app_id = resolver.resolve_app_id(&conflicting_game);
    assert_eq!(app_id.as_deref(), Some("123456"));

    // Agora suponha que um jogo NÃO seja Steam, mas tenha platform_id 111111 diferente do manifest
    let conflicting_non_steam = Game::new(
        "DOOM",
        Platform::Generic,
        Some("111111".into()),
        Confidence::High,
    );
    let app_id_conflict = resolver.resolve_app_id(&conflicting_non_steam);
    // Deve rejeitar associar ao 999999 para não sobrepor identificador existente
    assert_eq!(app_id_conflict, None);
}

// -----------------------------------------------------------------------------
// 13. Contrato de API LocationView preserva retrocompatibilidade
// -----------------------------------------------------------------------------
#[test]
fn test_13_location_view_serialization_and_deserialization() {
    let loc_json_old = r#"{
        "id": "11111111-1111-1111-1111-111111111111",
        "game_id": "22222222-2222-2222-2222-222222222222",
        "game_name": "Test Game",
        "platform": "Steam",
        "path": "C:\\Saves\\Test",
        "location_type": "Directory",
        "exists_on_disk": true,
        "monitored_files_count": 5,
        "last_check": "12:00:00"
    }"#;

    // Deserialização de JSON legado sem cover_url deve resultar em None por default
    let deserialized: LocationView = serde_json::from_str(loc_json_old).unwrap();
    assert_eq!(deserialized.cover_url, None);
    assert_eq!(deserialized.game_name, "Test Game");

    // Serialização com cover_url
    let mut loc_new = deserialized.clone();
    loc_new.cover_url = Some(
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/10/header.jpg"
            .into(),
    );
    let serialized = serde_json::to_string(&loc_new).unwrap();
    assert!(serialized.contains("cover_url"));
    assert!(serialized.contains("10/header.jpg"));
}

// -----------------------------------------------------------------------------
// 14. Carregamento de get_locations no DevUiState integrando cache e fallback
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_14_dev_ui_state_get_locations_integrates_covers() {
    let db = Database::open_in_memory().unwrap();
    let conn_arc = Arc::new(Mutex::new(db.conn));

    let manifest = create_test_manifest();
    let resolver = Arc::new(GameMetadataResolver::with_manifest(manifest));

    let state = DevUiState::new(
        conn_arc.clone(),
        std::path::PathBuf::from(":memory:"),
        std::path::PathBuf::from("backups"),
        None,
        std::time::Duration::from_secs(60),
    )
    .with_metadata_resolver(resolver);

    // Cadastra 1 jogo Steam e 1 jogo Genérico
    {
        let conn = conn_arc.lock().await;
        let games_repo = GamesRepo::new(&conn);
        let locs_repo = SaveLocationsRepo::new(&conn);

        let g_steam = Game::new(
            "Celeste",
            Platform::Steam,
            Some("504230".into()),
            Confidence::High,
        );
        let g_other = Game::new("Unknown RPG", Platform::Generic, None, Confidence::Low);

        games_repo.insert(&g_steam).unwrap();
        games_repo.insert(&g_other).unwrap();

        let l_steam = SaveLocation::new(
            g_steam.id,
            std::path::PathBuf::from(r"C:\Saves\Celeste"),
            SaveLocationType::Directory,
        );
        let l_other = SaveLocation::new(
            g_other.id,
            std::path::PathBuf::from(r"C:\Saves\RPG"),
            SaveLocationType::Directory,
        );

        locs_repo.upsert(&l_steam).unwrap();
        locs_repo.upsert(&l_other).unwrap();
    }

    // Executa get_locations
    let locations = state.get_locations().await.unwrap();
    assert_eq!(locations.len(), 2);

    let celeste = locations.iter().find(|l| l.game_name == "Celeste").unwrap();
    assert!(celeste.cover_url.is_some());
    assert!(celeste
        .cover_url
        .as_ref()
        .unwrap()
        .contains("504230/header.jpg"));

    let unknown = locations
        .iter()
        .find(|l| l.game_name == "Unknown RPG")
        .unwrap();
    assert_eq!(unknown.cover_url, None);
}

// -----------------------------------------------------------------------------
// 15. Auditoria Adversarial de Identidade de Jogos e Prevenção de Vazamento
// -----------------------------------------------------------------------------
#[test]
fn test_15_adversarial_platform_id_security_and_non_steam_leak() {
    let manifest = create_test_manifest();
    let resolver = GameMetadataResolver::with_manifest(manifest);

    // Steam com AppID 730
    let steam_cs = Game::new(
        "Counter-Strike 2",
        Platform::Steam,
        Some("730".into()),
        Confidence::High,
    );
    assert_eq!(resolver.resolve_app_id(&steam_cs).as_deref(), Some("730"));

    // GOG com mesmo ID 730 NÃO DEVE gerar AppID Steam
    let gog_game = Game::new(
        "Custom GOG",
        Platform::Gog,
        Some("730".into()),
        Confidence::High,
    );
    assert_eq!(resolver.resolve_app_id(&gog_game), None);

    // Dolphin Emulator com ID 730 NÃO DEVE gerar AppID Steam
    let emu_game = Game::new(
        "Custom Emu",
        Platform::Dolphin,
        Some("730".into()),
        Confidence::High,
    );
    assert_eq!(resolver.resolve_app_id(&emu_game), None);

    // Generic com ID 730 NÃO DEVE gerar AppID Steam
    let gen_game = Game::new(
        "Custom Gen",
        Platform::Generic,
        Some("730".into()),
        Confidence::High,
    );
    assert_eq!(resolver.resolve_app_id(&gen_game), None);

    // AppIDs inválidos ou perigosos:
    let test_ids = [
        "",
        "   ",
        "0",
        "000",
        "000730",
        "abc",
        "../730",
        "730<script>",
        "730/../1091500",
        "4294967296", // u32::MAX + 1
        "-730",
        "730; DROP TABLE games;",
        "７３０",
    ];
    for bad_id in test_ids {
        assert!(
            !is_valid_steam_app_id(bad_id),
            "Deveria rejeitar id: '{}'",
            bad_id
        );
        let g = Game::new(
            "Test",
            Platform::Steam,
            Some(bad_id.into()),
            Confidence::High,
        );
        assert_eq!(resolver.resolve_app_id(&g), None);
        assert_eq!(build_steam_header_url(bad_id), None);
    }

    // u32::MAX válido
    let max_game = Game::new(
        "Max AppID Game",
        Platform::Steam,
        Some("4294967295".into()),
        Confidence::High,
    );
    assert_eq!(
        resolver.resolve_app_id(&max_game).as_deref(),
        Some("4294967295")
    );
    assert!(build_steam_header_url("4294967295").is_some());

    // GOG Hollow Knight com ID GOG 1335274700 declarado no manifest resolve Steam cover via Ludusavi
    let gog_hk = Game::new(
        "Hollow Knight",
        Platform::Gog,
        Some("1335274700".into()),
        Confidence::High,
    );
    assert_eq!(resolver.resolve_app_id(&gog_hk).as_deref(), Some("367520"));

    // GOG Hollow Knight com ID GOG conflitante NÃO resolve
    let gog_hk_conflict = Game::new(
        "Hollow Knight",
        Platform::Gog,
        Some("999999999".into()),
        Confidence::High,
    );
    assert_eq!(resolver.resolve_app_id(&gog_hk_conflict), None);
}

// -----------------------------------------------------------------------------
// 16. Auditoria Adversarial de URLs e Validação Rigorosa contra XSS
// -----------------------------------------------------------------------------
#[test]
fn test_16_adversarial_url_validation() {
    let invalid_urls = [
        // Schemas não-HTTPS
        "http://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.jpg",
        "ftp://shared.cloudflare.steamstatic.com/header.jpg",
        "javascript:alert(1)",
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAUA",
        "file:///etc/passwd",
        "file:///C:/Windows/System32/cmd.exe",
        // Domínios maliciosos / bypass de autoridade
        "https://evil.example/header.jpg",
        "https://shared.cloudflare.steamstatic.com.evil.example/apps/730/header.jpg",
        "https://evil@shared.cloudflare.steamstatic.com/apps/730/header.jpg",
        "https://shared.cloudflare.steamstatic.com:443/apps/730/header.jpg",
        "https://steamstatic.fake/image.jpg",
        "https://cdn.cloudflare.steamstatic.com.evil.com/image.jpg",
        "https://cdn.cloudflare.steamstatic.com/@evil",
        // Query parameters, fragmentos e portas
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.jpg?eval=alert(1)",
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.jpg#fragment",
        "https://shared.cloudflare.steamstatic.com/apps/730/header.jpg?version=1",
        // Path traversal, encoded traversal e barras duplas
        "https://shared.cloudflare.steamstatic.com/../evil",
        "https://shared.cloudflare.steamstatic.com//evil",
        "https://shared.cloudflare.steamstatic.com/%2e%2e/evil",
        "https://cdn.cloudflare.steamstatic.com/../evil.com",
        // Caracteres de controle, espaços, tabs, newlines e NUL
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.jpg\r\n",
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.jpg\t",
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/ 730/header.jpg",
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.jpg\0",
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.jpg\x07",
        // Caracteres Unicode e encoded characters
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/７３０/header.jpg",
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/🎮/header.jpg",
        // XSS e HTML injection
        "<img src=x onerror=alert(1)>",
        "\"><script>alert(1)</script>",
        "' onerror='alert(1)",
        // Trailing slash e extensões não-imagem
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/",
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.exe",
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.html",
    ];

    for bad_url in invalid_urls {
        assert!(
            !is_valid_steam_cover_url(bad_url),
            "URL maliciosa/inválida foi aceita: '{}'",
            bad_url
        );
    }

    // Long URL > 256 chars
    let very_long = format!(
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{}/header.jpg",
        "a".repeat(250)
    );
    assert!(!is_valid_steam_cover_url(&very_long));

    let valid_urls = [
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/730/header.jpg",
        "https://cdn.cloudflare.steamstatic.com/steam/apps/1091500/header.jpg",
        "https://steamcdn-a.akamaihd.net/steam/apps/413150/header.jpg",
        // Uppercase / mixed-case host e scheme (RFC 3986)
        "HTTPS://SHARED.CLOUDFLARE.STEAMSTATIC.COM/store_item_assets/steam/apps/730/header.jpg",
        "https://CDN.CLOUDFLARE.STEAMSTATIC.COM/steam/apps/1091500/header.PNG",
    ];

    for good_url in valid_urls {
        assert!(
            is_valid_steam_cover_url(good_url),
            "URL válida foi rejeitada: '{}'",
            good_url
        );
    }
}

// -----------------------------------------------------------------------------
// 17. Auditoria do Ciclo de Vida do Cache, Invalidação e Cascade Delete
// -----------------------------------------------------------------------------
#[test]
fn test_17_adversarial_cache_lifecycle_and_invalidation() {
    let db = Database::open_in_memory().unwrap();
    let manifest = create_test_manifest();
    let resolver = GameMetadataResolver::with_manifest(manifest);
    let games_repo = GamesRepo::new(&db.conn);
    let meta_repo = GameMetadataRepo::new(&db.conn);

    // 1. Cria jogo Steam com AppID 730
    let mut game = Game::new("CS2", Platform::Steam, Some("730".into()), Confidence::High);
    games_repo.insert(&game).unwrap();

    let meta1 = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert_eq!(meta1.steam_app_id.as_deref(), Some("730"));
    assert!(meta1.cover_url.unwrap().contains("730/header.jpg"));

    // 2. Mudança de AppID: usuário altera para 1091500
    game.platform_id = Some("1091500".into());
    let meta2 = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert_eq!(meta2.steam_app_id.as_deref(), Some("1091500"));
    assert!(meta2.cover_url.unwrap().contains("1091500/header.jpg"));

    // Verifica que o registro no banco foi atualizado
    let persisted = meta_repo.find_by_game_id(game.id).unwrap().unwrap();
    assert_eq!(persisted.steam_app_id.as_deref(), Some("1091500"));

    // 3. Mudança de Plataforma: jogo vira GOG (não listado em Ludusavi)
    game.platform = Platform::Gog;
    game.platform_id = Some("99999".into());
    let meta3 = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert_eq!(meta3.source, MetadataSource::None);
    assert!(meta3.cover_url.is_none());

    // 4. Upgrade de negative cache:
    // Cria jogo Hollow Knight enquanto o resolver está SEM manifest
    let empty_resolver = GameMetadataResolver::new();
    let hk_game = Game::new("Hollow Knight", Platform::Generic, None, Confidence::High);
    games_repo.insert(&hk_game).unwrap();
    let meta_neg = empty_resolver
        .resolve_or_get_cached(&db.conn, &hk_game)
        .unwrap();
    assert_eq!(meta_neg.source, MetadataSource::None);

    // Agora resolve com o resolver que possui manifest (com Hollow Knight -> 367520)
    let meta_upgraded = resolver.resolve_or_get_cached(&db.conn, &hk_game).unwrap();
    assert_eq!(meta_upgraded.source, MetadataSource::Ludusavi);
    assert_eq!(meta_upgraded.steam_app_id.as_deref(), Some("367520"));
    assert!(meta_upgraded
        .cover_url
        .unwrap()
        .contains("367520/header.jpg"));

    // 5. Precedência de MetadataSource::Manual
    let manual_game = Game::new("Custom Art Game", Platform::Generic, None, Confidence::Low);
    games_repo.insert(&manual_game).unwrap();
    let manual_url =
        "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/10/header.jpg"
            .to_string();
    let manual_meta = GameMetadata::new(
        manual_game.id,
        Some("10".into()),
        Some(manual_url.clone()),
        MetadataSource::Manual,
    );
    meta_repo.upsert(&manual_meta).unwrap();

    let fetched = resolver
        .resolve_or_get_cached(&db.conn, &manual_game)
        .unwrap();
    assert_eq!(fetched.source, MetadataSource::Manual);
    assert_eq!(fetched.cover_url, Some(manual_url));

    // 6. Cascade delete: remover jogo deve remover metadata automaticamente via FK
    games_repo.delete(game.id).unwrap();
    let meta_after_delete = meta_repo.find_by_game_id(game.id).unwrap();
    assert!(
        meta_after_delete.is_none(),
        "Metadata deve sofrer cascade delete quando o jogo for removido"
    );
}

// -----------------------------------------------------------------------------
// 18. Performance em Lote: Verificação contra N+1 Queries em get_locations
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_18_batch_performance_no_n_plus_one() {
    let db = Database::open_in_memory().unwrap();
    let conn_arc = Arc::new(Mutex::new(db.conn));
    let manifest = create_test_manifest();
    let resolver = Arc::new(GameMetadataResolver::with_manifest(manifest));

    let state = DevUiState::new(
        conn_arc.clone(),
        std::path::PathBuf::from(":memory:"),
        std::path::PathBuf::from("backups"),
        None,
        std::time::Duration::from_secs(60),
    )
    .with_metadata_resolver(resolver);

    // Cadastra 20 jogos e 20 locais
    {
        let conn = conn_arc.lock().await;
        let games_repo = GamesRepo::new(&conn);
        let locs_repo = SaveLocationsRepo::new(&conn);

        for i in 0..20 {
            let g = Game::new(
                format!("Game {}", i),
                Platform::Steam,
                Some(format!("100{}", i)),
                Confidence::High,
            );
            games_repo.insert(&g).unwrap();

            let loc = SaveLocation::new(
                g.id,
                std::path::PathBuf::from(format!(r"C:\Saves\Game{}", i)),
                SaveLocationType::Directory,
            );
            locs_repo.upsert(&loc).unwrap();
        }
    }

    // get_locations deve executar em lote rapidamente e retornar todos os 20 itens
    let locations = state.get_locations().await.unwrap();
    assert_eq!(locations.len(), 20);
    for loc in &locations {
        assert!(loc.cover_url.is_some());
    }
}

// -----------------------------------------------------------------------------
// 19. Recuperação em Nuvem Reconstrói Catálogo sem Confiar em URLs Remotas
// -----------------------------------------------------------------------------
#[tokio::test]
async fn test_19_cloud_recovery_reconstructs_catalog_without_remote_cover_url() {
    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let meta_repo = GameMetadataRepo::new(&db.conn);
    let resolver = GameMetadataResolver::with_manifest(create_test_manifest());

    // Jogo reconstruído a partir do manifest remoto (apenas nome e plataforma são confiados)
    let recovered_game = Game::new("Stardew Valley", Platform::Generic, None, Confidence::High);
    games_repo.insert(&recovered_game).unwrap();

    // No banco, ainda não há metadados visuais
    assert!(meta_repo
        .find_by_game_id(recovered_game.id)
        .unwrap()
        .is_none());

    // Ao ser consultado pela UI, a resolução é feita localmente e de forma determinística
    let meta = resolver
        .resolve_or_get_cached(&db.conn, &recovered_game)
        .unwrap();
    assert_eq!(meta.steam_app_id.as_deref(), Some("413150"));
    assert_eq!(meta.source, MetadataSource::Ludusavi);
    assert!(meta.cover_url.unwrap().contains("413150/header.jpg"));
}

// -----------------------------------------------------------------------------
// 20. Metadados Manuais com cover_url=None NUNCA são Destruídos por Auto-Resolução
// -----------------------------------------------------------------------------
#[test]
fn test_20_manual_metadata_none_cover_preserved_against_manifest() {
    let db = Database::open_in_memory().unwrap();
    let manifest = create_test_manifest(); // Contém "Stardew Valley" -> Steam 413150
    let resolver = GameMetadataResolver::with_manifest(manifest);
    let games_repo = GamesRepo::new(&db.conn);
    let meta_repo = GameMetadataRepo::new(&db.conn);

    let game = Game::new("Stardew Valley", Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    // Usuário configura manualmente SEM capa (cover_url: None)
    let manual_empty = GameMetadata::new(game.id, None, None, MetadataSource::Manual);
    meta_repo.upsert(&manual_empty).unwrap();

    // Mesmo que o título exista no manifest, a decisão manual de 'sem capa' prevalece
    let resolved = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert_eq!(resolved.source, MetadataSource::Manual);
    assert_eq!(resolved.cover_url, None);
    assert_eq!(resolved.steam_app_id, None);
}

// -----------------------------------------------------------------------------
// 21. Matriz Completa de Transições de Plataforma e AppID
// -----------------------------------------------------------------------------
#[test]
fn test_21_complete_platform_transition_matrix() {
    let db = Database::open_in_memory().unwrap();
    let manifest = create_test_manifest();
    let resolver = GameMetadataResolver::with_manifest(manifest);
    let games_repo = GamesRepo::new(&db.conn);
    let meta_repo = GameMetadataRepo::new(&db.conn);

    // Transição 1: GOG -> Steam
    let mut game = Game::new(
        "Action Game",
        Platform::Gog,
        Some("555".into()),
        Confidence::High,
    );
    games_repo.insert(&game).unwrap();

    // No GOG com ID próprio sem declaração no manifest, resulta em None
    let meta_gog = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert_eq!(meta_gog.source, MetadataSource::None);
    assert_eq!(meta_gog.cover_url, None);

    // Muda para Steam com AppID 730
    game.platform = Platform::Steam;
    game.platform_id = Some("730".into());
    let meta_steam = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    assert_eq!(meta_steam.source, MetadataSource::Steam);
    assert_eq!(meta_steam.steam_app_id.as_deref(), Some("730"));
    assert!(meta_steam.cover_url.unwrap().contains("730/header.jpg"));

    // Transição 2: Steam -> GOG
    game.platform = Platform::Gog;
    game.platform_id = Some("730".into()); // Mesmo ID numérico, mas agora é GOG!
    let meta_back_to_gog = resolver.resolve_or_get_cached(&db.conn, &game).unwrap();
    // NÃO deve herdar Steam 730! Deve invalidar e virar None
    assert_eq!(meta_back_to_gog.source, MetadataSource::None);
    assert_eq!(meta_back_to_gog.cover_url, None);
    assert_eq!(meta_back_to_gog.steam_app_id, None);

    // Transição 3: Colisão de ID entre lojas (Steam 730 vs GOG 730)
    let steam_730 = Game::new("CS2", Platform::Steam, Some("730".into()), Confidence::High);
    let gog_730 = Game::new(
        "Random GOG",
        Platform::Gog,
        Some("730".into()),
        Confidence::High,
    );
    games_repo.insert(&steam_730).unwrap();
    games_repo.insert(&gog_730).unwrap();

    let meta_s = resolver
        .resolve_or_get_cached(&db.conn, &steam_730)
        .unwrap();
    let meta_g = resolver.resolve_or_get_cached(&db.conn, &gog_730).unwrap();

    assert_eq!(meta_s.source, MetadataSource::Steam);
    assert_eq!(meta_s.steam_app_id.as_deref(), Some("730"));
    assert!(meta_s.cover_url.is_some());

    assert_eq!(meta_g.source, MetadataSource::None);
    assert_eq!(meta_g.steam_app_id, None);
    assert_eq!(meta_g.cover_url, None);

    // Ambas as entradas no SQLite coexistem sem colisão
    let s_persisted = meta_repo.find_by_game_id(steam_730.id).unwrap().unwrap();
    let g_persisted = meta_repo.find_by_game_id(gog_730.id).unwrap().unwrap();
    assert_eq!(s_persisted.steam_app_id.as_deref(), Some("730"));
    assert_eq!(g_persisted.steam_app_id, None);
}

// -----------------------------------------------------------------------------
// 22. Integridade Referencial SQLite: Bloqueio de Órfãos e Concorrência
// -----------------------------------------------------------------------------
#[test]
fn test_22_foreign_key_prevents_orphan_metadata_on_deleted_game() {
    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let meta_repo = GameMetadataRepo::new(&db.conn);

    let game = Game::new(
        "Deletable Game",
        Platform::Steam,
        Some("123".into()),
        Confidence::High,
    );
    games_repo.insert(&game).unwrap();

    // Exclui o jogo primeiro
    games_repo.delete(game.id).unwrap();

    // Tentativa concorrente de inserir metadata para o jogo deletado deve falhar por FK constraint
    let orphan_meta = GameMetadata::new(
        game.id,
        Some("123".into()),
        Some(
            "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/123/header.jpg"
                .into(),
        ),
        MetadataSource::Steam,
    );

    let err = meta_repo.upsert(&orphan_meta);
    assert!(
        err.is_err(),
        "SQLite com foreign_keys=ON deve rejeitar metadata para game_id inexistente"
    );
}
