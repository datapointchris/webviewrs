// Emits `gtk` on the platforms where wry builds its webview on GTK. The same five are
// named in Cargo.toml's target dependency table, which cannot read a build script's cfg.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(gtk)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(os.as_str(), "linux" | "dragonfly" | "freebsd" | "netbsd" | "openbsd") {
        println!("cargo::rustc-cfg=gtk");
    }
}
