// build.rs do savevault-app
// Prepara os recursos do Windows (ícone da aplicação, ícone do tray e manifesto)
// e os módulos da feature 'desktop' (Tauri).

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

    #[cfg(all(windows, not(feature = "desktop")))]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        res.set_resource_file("savevault.rc");
        let _ = res.compile();
    }
}
