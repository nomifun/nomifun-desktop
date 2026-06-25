//! Real-machine check for Start-Menu app-name resolution (the `launch` action's
//! fix for "找不到" on bare app names like "QQ音乐").
//!
//! Run:  cargo run -p nomi-computer --example appresolve -- "QQ音乐" qqmusic 网易云音乐 notepad
//! Prints the resolved launch target for each name WITHOUT launching anything.

#[cfg(target_os = "windows")]
fn main() {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let names = if names.is_empty() {
        vec!["QQ音乐".to_string(), "qqmusic".to_string(), "notepad".to_string()]
    } else {
        names
    };
    for name in names {
        match nomi_computer::launch::resolve_app_for_diagnostics(&name) {
            Some(t) => println!("{name:?} -> {t:?}"),
            None => println!("{name:?} -> (not found in Start Menu)"),
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("appresolve is a Windows-only example.");
}
