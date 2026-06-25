//! Real-machine smoke test for the Windows UIA backend.
//!
//! Run with:  cargo run -p nomi-a11y --example winsmoke
//! Optional:  cargo run -p nomi-a11y --example winsmoke -- <pid>
//!
//! Opens Notepad on a temp file with known (Chinese) content, then exercises
//! the engine: observe → numbered element list + bounds + app_name → read the
//! document's text back via the Text pattern (validates the TextPattern value
//! path) → attempt SetValue → stale-generation guard. Prints a numbered
//! Set-of-Marks-style listing so coordinates can be eyeballed against the window.

#[cfg(target_os = "windows")]
fn main() {
    use std::{thread::sleep, time::Duration};

    use nomi_a11y::{ElementAction, ObserveOpts, Snapshot, Target};

    fn dump(snap: &Snapshot) {
        println!(
            "  app={:?}  window={:?}  pid={:?}  entries={}  truncated={}",
            snap.app_name,
            snap.window_title,
            snap.pid,
            snap.entries.len(),
            snap.truncated
        );
        for e in &snap.entries {
            let b = e.bounds;
            println!(
                "    [{:>2}] {:<11} name={:?} value={:?} states={:?}  @ ({:.0},{:.0}) {:.0}x{:.0}",
                e.r#ref, e.role, e.name, e.value, e.states, b.x, b.y, b.w, b.h
            );
        }
    }

    let arg_pid: Option<i32> = std::env::args().nth(1).and_then(|s| s.parse().ok());

    // A temp file with known content lets us verify the TextPattern read path
    // (the Win11 RichEdit Notepad exposes text via TextPattern, not ValuePattern).
    const MARKER: &str = "你好世界";
    let tmp = std::env::temp_dir().join("nomi_winsmoke.txt");
    let _ = std::fs::write(&tmp, format!("NomiFun TextPattern 验证 Hello {MARKER}\n第二行 line two\n"));

    let mut child = None;
    let target_pid = match arg_pid {
        Some(p) => {
            println!("== using provided pid {p} ==");
            Some(p)
        }
        None => {
            println!("== launching notepad.exe on {} ==", tmp.display());
            match std::process::Command::new("notepad.exe").arg(&tmp).spawn() {
                Ok(c) => {
                    let p = c.id() as i32;
                    println!("   spawned notepad, launcher pid = {p}");
                    child = Some(c);
                    sleep(Duration::from_millis(2500)); // allow the file to load
                    Some(p)
                }
                Err(e) => {
                    println!("   failed to launch notepad: {e}");
                    None
                }
            }
        }
    };

    let engine = match nomi_a11y::create_engine() {
        Ok(e) => e,
        Err(e) => {
            println!("FATAL: create_engine failed: {e}");
            return;
        }
    };
    println!("capabilities: {:?}", engine.capabilities());

    println!("\n== observe(foreground) ==");
    let t0 = std::time::Instant::now();
    let fg = engine.observe(&ObserveOpts::default());
    let elapsed = t0.elapsed();
    match &fg {
        Ok(s) => {
            println!(
                "  observe latency: {:?}  ({} entries, truncated={})",
                elapsed,
                s.entries.len(),
                s.truncated
            );
            dump(s);
            println!("\n  -- semantic tree (snap.text) --");
            for line in s.text.lines() {
                println!("  {line}");
            }
        }
        Err(e) => println!("  observe(foreground) error: {e}  (after {elapsed:?})"),
    }

    let mut pid_snap = None;
    if let Some(pid) = target_pid {
        println!("\n== observe(pid={pid}) ==");
        match engine.observe(&ObserveOpts {
            pid: Some(pid),
            ..Default::default()
        }) {
            Ok(s) => {
                dump(&s);
                pid_snap = Some(s);
            }
            Err(e) => {
                println!("  observe(pid) error: {e} (Store Notepad reparents; using foreground)")
            }
        }
    }

    let snap = pid_snap
        .filter(|s| !s.entries.is_empty())
        .or_else(|| fg.ok().filter(|s| !s.entries.is_empty()));

    let Some(snap) = snap else {
        println!("\nNo usable snapshot with elements; aborting actuation phase.");
        if let Some(mut c) = child {
            let _ = c.kill();
        }
        let _ = std::fs::remove_file(&tmp);
        return;
    };

    // --- TextPattern value read: the document should show the file's text ---
    let doc = snap
        .entries
        .iter()
        .find(|e| matches!(e.role.as_str(), "edit" | "document"));
    println!("\n== TextPattern value read ==");
    match doc.and_then(|e| e.value.clone()) {
        Some(v) => println!(
            "  TEXTPATTERN READ: {} (value={:?})",
            if v.contains(MARKER) { "PASS ✔" } else { "got text but no marker" },
            v
        ),
        None => println!("  document value = None (no text read)"),
    }

    // --- SetValue actuation (RichEdit Notepad often no-ops ValuePattern.SetValue;
    //     the tool layer types instead — we just confirm the call is honest) ---
    if let Some(e) = doc {
        let text = "NomiFun SetValue 测试".to_string();
        println!("\n== invoke SetValue on [{}] {} ==", e.r#ref, e.role);
        match engine.invoke(
            &Target::Ref(e.r#ref),
            snap.generation,
            ElementAction::SetValue(text),
        ) {
            Ok(eff) => println!("  ok: {}", eff.message),
            Err(err) => println!("  SetValue error (data, not a panic): {err}"),
        }
    }

    // --- press_chain demonstration: focus then activate a button via the chain ---
    if let Some(btn) = snap.entries.iter().find(|e| e.role == "button") {
        println!("\n== invoke Focus on [{}] button {:?} ==", btn.r#ref, btn.name);
        match engine.invoke(&Target::Ref(btn.r#ref), snap.generation, ElementAction::Focus) {
            Ok(eff) => println!("  ok: {}", eff.message),
            Err(err) => println!("  error: {err}"),
        }
    }

    // --- stale-ref guard ---
    println!("\n== stale generation guard ==");
    let stale = engine.invoke(
        &Target::Ref(1),
        nomi_a11y::SnapshotGen(snap.generation.0.wrapping_sub(1)),
        ElementAction::Focus,
    );
    println!("  invoke with old generation → {stale:?}");

    if let Some(mut c) = child {
        sleep(Duration::from_millis(300));
        let _ = c.kill();
        println!("\n(killed spawned notepad)");
    }
    let _ = std::fs::remove_file(&tmp);
    println!("\n== done ==");
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("winsmoke is a Windows-only example.");
}
