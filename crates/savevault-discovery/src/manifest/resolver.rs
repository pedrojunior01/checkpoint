use std::path::PathBuf;

/// Contexto de caminhos do sistema para resolução de placeholders.
pub struct PathContext {
    pub appdata: String,
    pub local_appdata: String,
    pub documents: String,
    pub home: String,
    /// Diretório de instalação do jogo (para <root> e <base>).
    pub game_install_path: Option<String>,
}

impl PathContext {
    /// Constrói PathContext a partir das variáveis de ambiente do Windows.
    pub fn from_env(install_path: Option<&PathBuf>) -> Self {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\User".into());
        let docs = {
            let onedrive_docs = PathBuf::from(&home).join("OneDrive").join("Documents");
            let onedrive_pt = PathBuf::from(&home).join("OneDrive").join("Documentos");
            let direct = PathBuf::from(&home).join("Documents");
            if onedrive_docs.exists() {
                onedrive_docs.to_string_lossy().to_string()
            } else if onedrive_pt.exists() {
                onedrive_pt.to_string_lossy().to_string()
            } else {
                direct.to_string_lossy().to_string()
            }
        };
        Self {
            appdata: std::env::var("APPDATA")
                .unwrap_or_else(|_| format!("{}\\AppData\\Roaming", home)),
            local_appdata: std::env::var("LOCALAPPDATA")
                .unwrap_or_else(|_| format!("{}\\AppData\\Local", home)),
            documents: docs,
            home: home.clone(),
            game_install_path: install_path.map(|p| p.to_string_lossy().to_string()),
        }
    }
}

/// Resolve um template de caminho do Ludusavi Manifest em um ou mais PathBufs concretos.
///
/// Retorna `vec![]` se o template contiver placeholders não resolvidos.
pub fn resolve_paths(template: &str, ctx: &PathContext) -> Vec<PathBuf> {
    let mut path = template.to_string();

    // Substituições diretas de variáveis de ambiente
    path = path.replace("<winAppData>", &ctx.appdata);
    path = path.replace("<winLocalAppData>", &ctx.local_appdata);
    path = path.replace("<winDocuments>", &ctx.documents);
    path = path.replace("<home>", &ctx.home);

    // Substituições que dependem do diretório de instalação do jogo
    if let Some(ref install) = ctx.game_install_path {
        path = path.replace("<root>", install);
        path = path.replace("<base>", install);
    }

    // IDs dinâmicos: substituímos por curinga (*) para buscar no disco
    path = path.replace("<storeUserId>", "*");
    path = path.replace("<osUserName>", "*");

    // Ignorar placeholders específicos de outras plataformas
    if path.contains("<xdg") || path.contains("<mac") || path.contains("<linux") {
        return vec![];
    }

    // Se ainda restam placeholders não resolvidos, descartar
    if path.contains('<') && path.contains('>') {
        return vec![];
    }

    if path.is_empty() {
        return vec![];
    }

    // Normalizar separadores para Windows
    let path = path.replace('/', "\\");

    // Remover wildcards finais de arquivos (queremos o diretório pai para monitorar)
    let path = path
        .trim_end_matches("\\**")
        .trim_end_matches("\\*")
        .to_string();

    // Se não há curingas dinâmicos, retorna o caminho (mesmo que não exista, para mostrar como Ausente)
    if !path.contains('*') && !path.contains('?') {
        return vec![PathBuf::from(path)];
    }

    // Se há curingas (ex: <storeUserId>), buscar correspondências no disco
    let mut results = Vec::new();
    if let Ok(entries) = glob::glob(&path) {
        for entry in entries.filter_map(|e| e.ok()) {
            results.push(entry);
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_ctx() -> PathContext {
        PathContext {
            appdata: r"C:\Users\pedro\AppData\Roaming".into(),
            local_appdata: r"C:\Users\pedro\AppData\Local".into(),
            documents: r"C:\Users\pedro\Documents".into(),
            home: r"C:\Users\pedro".into(),
            game_install_path: Some(r"C:\Games\Stardew Valley".into()),
        }
    }

    #[test]
    fn resolves_appdata_placeholder() {
        let ctx = dummy_ctx();
        let result = resolve_paths("<winAppData>/StardewValley/Saves", &ctx);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PathBuf::from(r"C:\Users\pedro\AppData\Roaming\StardewValley\Saves")
        );
    }

    #[test]
    fn resolves_root_placeholder() {
        let ctx = dummy_ctx();
        let result = resolve_paths("<root>/saves", &ctx);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], PathBuf::from(r"C:\Games\Stardew Valley\saves"));
    }

    #[test]
    fn strips_wildcards() {
        let ctx = dummy_ctx();
        let result = resolve_paths("<winAppData>/Game/saves/**", &ctx);
        assert_eq!(result.len(), 1);
        assert!(
            !result[0].to_string_lossy().contains("**"),
            "Wildcards devem ser removidos"
        );
    }

    #[test]
    fn returns_none_for_unresolved_placeholder() {
        let ctx = PathContext {
            game_install_path: None,
            ..dummy_ctx()
        };
        let result = resolve_paths("<root>/saves", &ctx);
        assert!(
            result.is_empty(),
            "Sem install path, <root> deve retornar vazio"
        );
    }

    #[test]
    fn returns_none_for_linux_placeholder() {
        let ctx = dummy_ctx();
        let result = resolve_paths("<xdgData>/game/saves", &ctx);
        assert!(result.is_empty(), "Placeholders Linux devem retornar vazio");
    }
}
