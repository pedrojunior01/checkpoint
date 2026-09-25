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
        Self {
            appdata: std::env::var("APPDATA")
                .unwrap_or_else(|_| format!("{}\\AppData\\Roaming", home)),
            local_appdata: std::env::var("LOCALAPPDATA")
                .unwrap_or_else(|_| format!("{}\\AppData\\Local", home)),
            documents: format!("{}\\Documents", home),
            home: home.clone(),
            game_install_path: install_path
                .map(|p| p.to_string_lossy().to_string()),
        }
    }
}

/// Resolve um template de caminho do Ludusavi Manifest em um PathBuf concreto.
///
/// Retorna `None` se o template contiver placeholders que não puderam ser resolvidos.
pub fn resolve_path(template: &str, ctx: &PathContext) -> Option<PathBuf> {
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

    // Ignorar placeholders específicos de outras plataformas
    if path.contains("<xdg")
        || path.contains("<mac")
        || path.contains("<linux")
    {
        return None;
    }

    // Se ainda restam placeholders não resolvidos, descartar
    if path.contains('<') && path.contains('>') {
        return None;
    }

    if path.is_empty() {
        return None;
    }

    // Normalizar separadores para Windows
    let path = path.replace('/', "\\");

    // Remover wildcards — queremos o diretório pai
    let path = path
        .trim_end_matches("\\**")
        .trim_end_matches("\\*")
        .to_string();

    Some(PathBuf::from(path))
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
        let result = resolve_path("<winAppData>/StardewValley/Saves", &ctx).unwrap();
        assert_eq!(
            result,
            PathBuf::from(r"C:\Users\pedro\AppData\Roaming\StardewValley\Saves")
        );
    }

    #[test]
    fn resolves_root_placeholder() {
        let ctx = dummy_ctx();
        let result = resolve_path("<root>/saves", &ctx).unwrap();
        assert_eq!(result, PathBuf::from(r"C:\Games\Stardew Valley\saves"));
    }

    #[test]
    fn strips_wildcards() {
        let ctx = dummy_ctx();
        let result = resolve_path("<winAppData>/Game/saves/**", &ctx).unwrap();
        assert!(
            !result.to_string_lossy().contains("**"),
            "Wildcards devem ser removidos"
        );
    }

    #[test]
    fn returns_none_for_unresolved_placeholder() {
        let ctx = PathContext {
            game_install_path: None,
            ..dummy_ctx()
        };
        let result = resolve_path("<root>/saves", &ctx);
        assert!(result.is_none(), "Sem install path, <root> deve retornar None");
    }

    #[test]
    fn returns_none_for_linux_placeholder() {
        let ctx = dummy_ctx();
        let result = resolve_path("<xdgData>/game/saves", &ctx);
        assert!(result.is_none(), "Placeholders Linux devem retornar None");
    }
}
