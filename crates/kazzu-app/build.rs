fn main() {
    // APK e IPA usam a interface mobile nova (ui-mobile-v6); o desktop continua na árvore v5.
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let mobile = os == "android" || os == "ios";
    let entry = if mobile { "ui-mobile-v6/main.slint" } else { "ui-v5/app.slint" };
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_edition((1, 19));
    slint_build::compile_with_config(entry, config).expect("falha ao compilar a UI Slint");
    println!("cargo:rerun-if-changed=ui-mobile-v6");
    println!("cargo:rerun-if-changed=ui-v5");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
}
