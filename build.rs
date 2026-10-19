fn main() {
    println!("cargo:rerun-if-env-changed=SLINT_STYLE");
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let style = std::env::var("SLINT_STYLE").unwrap_or_else(|_| {
        match target.as_str() {
            "macos" => "cupertino",
            "windows" => "fluent",
            _ => "cosmic",
        }
        .into()
    });
    slint_build::compile_with_config(
        "ui/commander.slint",
        slint_build::CompilerConfiguration::new()
            .with_style(style)
            // Debug tests inspect the actual accessibility tree; release resources
            // remain compact and do not carry element debug metadata.
            .with_debug_info(std::env::var("PROFILE").is_ok_and(|p| p == "debug"))
            .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles),
    )
    .expect("compile Rust Commander Slint interface");
}
