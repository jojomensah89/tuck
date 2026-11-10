fn main() {
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attributes).expect("Tauri build configuration");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64") {
            println!("cargo:rerun-if-env-changed=VULKAN_SDK");
            if let Some(sdk) = std::env::var_os("VULKAN_SDK") {
                println!(
                    "cargo:rustc-link-search=native={}",
                    std::path::PathBuf::from(sdk).join("Lib").display()
                );
            }
            println!("cargo:rustc-link-arg=/DELAYLOAD:vulkan-1.dll");
            println!("cargo:rustc-link-lib=delayimp");
        }
        // Use one linker-generated manifest for app, unit and integration executables.
        // Tauri's resource manifest is disabled above to avoid duplicate resource IDs.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
}
