// build.rs do savevault-app
// Quando a feature 'desktop' está ativa, aciona o tauri-build para preparar
// os recursos de janela, ícones e manifesto do Windows.
// Sem a feature, este arquivo não faz nada, mantendo o build padrão CLI intacto.

fn main() {
    #[cfg(feature = "desktop")]
    {
        tauri_build::build();

        // No Windows, copia o WebView2Loader.dll para o diretório deps
        // para que os binários de integração também o encontrem.
        if let Ok(out_dir) = std::env::var("OUT_DIR") {
            let out_path = std::path::PathBuf::from(&out_dir);
            if let Some(target_dir) = out_path.ancestors().nth(3) {
                let src_dll = target_dir.join("WebView2Loader.dll");
                let deps_dll = target_dir.join("deps").join("WebView2Loader.dll");
                if src_dll.exists() && !deps_dll.exists() {
                    let _ = std::fs::copy(&src_dll, &deps_dll);
                }
            }
        }
    }
}
