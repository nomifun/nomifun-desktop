//! Dedicated native install fixture. No user microphone, serial port, provider
//! credentials, database, browser profile or UI is opened by this executable.
use anyhow::{Result, ensure};
use fs2::FileExt;
use nomifun_keyboard::{desktop_control, ipc};
use serde_json::{Value, json};
use std::{fs::{self, OpenOptions}, io::{BufRead, BufReader, Write}, net::{TcpListener, TcpStream}, time::Duration};

fn event(mode: &str) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true)
        .open(ipc::runtime_dir()?.join("events.log"))?;
    writeln!(file, "{mode} {}", std::process::id())?;
    Ok(())
}

fn receive(stream: &TcpStream) -> Result<Value> {
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();
    ensure!(reader.read_until(b'\n', &mut bytes)? <= 32768 && bytes.last() == Some(&b'\n'), "invalid fixture message");
    Ok(serde_json::from_slice(&bytes)?)
}
fn reply(stream: &mut TcpStream, value: Value) -> Result<()> {
    serde_json::to_writer(&mut *stream, &value)?;
    stream.write_all(b"\n")?;
    Ok(())
}

/// Test-only simulation of an already-installed owner. This is intentionally
/// absent from the production library and never opens a real serial device.
fn legacy_owner() -> Result<()> {
    let admission = nomifun_keyboard::maintenance::try_admit_startup()?.ok_or_else(|| anyhow::anyhow!("maintenance owns startup"))?;
    let dir = ipc::runtime_dir()?;
    let owner = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(dir.join("owner.lock"))?;
    FileExt::try_lock_exclusive(&owner)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let mut random = [0u8;32];
    getrandom::getrandom(&mut random).map_err(|_| anyhow::anyhow!("secure random unavailable"))?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let descriptor = dir.join("owner.json");
    fs::write(&descriptor, serde_json::to_vec(&json!({"port":listener.local_addr()?.port(),"pid":std::process::id(),"token":token}))?)?;
    drop(admission);
    event("legacy-owner")?;
    loop {
        let (mut stream, address) = listener.accept()?;
        ensure!(address.ip().is_loopback(), "fixture only accepts loopback");
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let request = receive(&stream)?;
        ensure!(request["protocol"] == 1 && request["token"] == token && request["mode"] == "control", "fixture authentication rejected");
        let proof = request["desktop_exit_proof"].as_str().ok_or_else(|| anyhow::anyhow!("missing GUI exit proof"))?;
        desktop_control::verify_quiescence(&dir, proof)?;
        reply(&mut stream, json!({"accepted":true}))?;
        let control = receive(&stream)?;
        ensure!(control == json!({"kind":"shutdown"}), "fixture only supports retirement shutdown");
        reply(&mut stream, json!({"completed":true}))?;
        fs::remove_file(&descriptor)?;
        drop(owner);
        return Ok(());
    }
}

fn main() -> Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "gui".into());
    ensure!(mode != "--keyboard-service" && mode != "service", "standalone keyboard service has been retired");
    if mode == "legacy-owner" { return legacy_owner(); }
    let fail = mode == "gui-fail";
    let _guard = desktop_control::register(move || {
        event("stop")?;
        ensure!(!fail, "fixture rejected native stop");
        Ok(())
    }, || std::process::exit(0))?;
    event(if fail { "gui-fail" } else { "gui" })?;
    loop { std::thread::sleep(Duration::from_millis(50)); }
}
