fn main() {
    tauri_build::build();

    // En Windows, tauri-build enlaza el recurso con el manifest (Common Controls v6) solo en el
    // binario de la app. Sin él, `cargo test` falla con STATUS_ENTRYPOINT_NOT_FOUND: se enlaza
    // también en los tests, solo cuando se pide (CP_TEST_MANIFEST=1),
    // porque enlazarlo dos veces en el binario rompe el build con MSVC.
    println!("cargo:rerun-if-env-changed=CP_TEST_MANIFEST");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CP_TEST_MANIFEST").is_ok() {
        let out = std::env::var("OUT_DIR").unwrap();
        let gnu = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
        let res = std::path::Path::new(&out).join(if gnu { "libresource.a" } else { "resource.lib" });
        if res.exists() {
            println!("cargo:rustc-link-arg={}", res.display());
        }
    }
}
