//! Windows: embed Monet's icon and version info in monet.exe and give the main thread the same
//! 8 MB stack the other platforms get (a no-op elsewhere).
fn main() {
    println!("cargo::rustc-check-cfg=cfg(windows_slim_errors)");
    println!("cargo::rerun-if-changed=../../assets/app-icon/monet.ico");
    #[cfg(windows)]
    windows();
}

#[cfg(windows)]
fn windows() {
    let mut res = winresource::WindowsResource::new();
    let ico = "../../assets/app-icon/monet.ico";
    if std::path::Path::new(ico).exists() {
        res.set_icon(ico);
    }
    res.set("ProductName", "Monet");
    res.set("FileDescription", "Monet");
    res.set("CompanyName", "Monet");
    if let Err(e) = res.compile() {
        println!("cargo::warning=winresource: {e}");
    }
    // Windows starts the main thread with 1 MB of stack, Linux and macOS with 8 MB. Layers nest
    // up to 256 deep in an export, and that recursion needs the same room everywhere.
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    println!("cargo::rustc-link-arg-bins={}", if msvc { "/STACK:8388608" } else { "-Wl,--stack,8388608" });
}
