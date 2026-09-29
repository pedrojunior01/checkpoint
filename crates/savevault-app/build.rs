use std::path::PathBuf;

fn main() {
    tauri_build::build();

    // No Windows GNU, garante que os binários de teste também recebam o manifesto de compatibilidade
    // e tenham acesso ao WebView2Loader.dll no diretório target/debug/deps.
    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        let resource_lib = PathBuf::from(&out_dir).join("libresource.a");
        if resource_lib.exists() {
            println!("cargo:rustc-link-arg-tests={}", resource_lib.display());
        }

        if let Some(target_dir) = PathBuf::from(&out_dir).ancestors().nth(3) {
            let src_dll = target_dir.join("WebView2Loader.dll");
            let deps_dll = target_dir.join("deps").join("WebView2Loader.dll");
            if src_dll.exists() {
                let _ = std::fs::copy(&src_dll, &deps_dll);
            }
        }
    }
}
