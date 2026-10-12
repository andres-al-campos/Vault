fn main() {
    // The in-app player links libmpv from Homebrew, wherever this machine's
    // prefix is.
    let prefix = std::process::Command::new("brew")
        .args(["--prefix", "mpv"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|p| std::path::Path::new(p).join("lib").is_dir())
        .expect("mpv isn't installed, and Vault's player is built on it. Run `brew install mpv`, then build again.");
    println!("cargo:rustc-link-search=native={prefix}/lib");
    println!("cargo:rustc-link-lib=dylib=mpv");
    tauri_build::build()
}
