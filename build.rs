// Embeds the app icon and version info into the Windows executable (release builds only, per
// contract 6: the .exe needs an icon and version metadata). Linux and macOS packaging embed the
// icon a different way (see packaging/linux and packaging/macos), so this is a no-op off Windows.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let icon = "assets/icon.ico";
    if !std::path::Path::new(icon).exists() {
        println!("cargo:warning=assets/icon.ico not found; building without a Windows icon");
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon(icon);
    res.set("ProductName", "SideQuest Companion");
    res.set("FileDescription", "SideQuest Companion");
    res.set("LegalCopyright", "Copyright (c) 2026 SideQuest Guild");
    if let Err(err) = res.compile() {
        println!("cargo:warning=failed to embed Windows resources: {err}");
    }
}
