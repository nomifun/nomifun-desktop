//! Dedicated native install fixture. No user microphone, serial port, provider
//! credentials, database, browser profile or UI is opened by this executable.
use anyhow::Result;
use nomifun_keyboard::{desktop_control, service};
use std::{fs::OpenOptions, io::Write, time::Duration};

fn event(mode: &str) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true)
        .open(service::runtime_dir()?.join("events.log"))?;
    writeln!(file, "{mode} {}", std::process::id())?;
    Ok(())
}

fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "gui".into());
    if mode == "--keyboard-service" || mode == "service" {
        event("service")?;
        return service::run_maintenance_fixture();
    }
    let fail = mode == "gui-fail";
    let _guard = desktop_control::register(move || {
        event("stop")?;
        anyhow::ensure!(!fail, "fixture rejected native stop");
        Ok(())
    }, || std::process::exit(0))?;
    event(if fail { "gui-fail" } else { "gui" })?;
    loop { std::thread::sleep(Duration::from_millis(50)); }
}
