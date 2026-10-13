#[cfg(target_os = "macos")]
#[path = "../src/macos_development_launch.rs"]
mod macos_development_launch;

#[cfg(target_os = "macos")]
fn main() {
    let result = std::env::args_os().nth(1).map(std::path::PathBuf::from)
        .ok_or_else(|| std::io::Error::other("private launch descriptor is required"))
        .and_then(|path| macos_development_launch::controller_main(&path));
    match result {
        Ok(code) => std::process::exit(code),
        Err(_) => {
            // Descriptor environment and nonce are never diagnostics.
            eprintln!("macOS LaunchServices controller failed; exact application cleanup is unconfirmed");
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() { eprintln!("macOS application controller requires macOS"); std::process::exit(1); }
