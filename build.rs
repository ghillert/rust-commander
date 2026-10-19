fn main() {
    println!("cargo:rerun-if-env-changed=SLINT_STYLE");
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target == "windows" {
        println!("cargo:rerun-if-changed=assets/icons/rust-commander.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/icons/rust-commander.ico")
            .set("ProductName", "Rust Commander")
            .set("FileDescription", "Rust Commander dual-pane file manager")
            .set("OriginalFilename", "rust-commander.exe")
            .set("CompanyName", "Gunnar Hillert")
            .set("FileVersion", env!("CARGO_PKG_VERSION"))
            .set("ProductVersion", env!("CARGO_PKG_VERSION"));
        resource
            .compile()
            .expect("compile Windows application resources");
    }
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
