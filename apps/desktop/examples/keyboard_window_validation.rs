//! Linux X11 integration witness for the production keyboard window adapter.
//! Run with isolated HOME/XDG/NOMIFUN_DATA_DIR and GDK_BACKEND=x11. It opens
//! real GTK/WebKit windows and a separate X11 control application, without
//! starting a NomiFun backend or model session.
#[cfg(target_os = "linux")]
#[path = "../src/keyboard/platform.rs"]
mod platform;

#[cfg(target_os = "linux")]
fn main() {
    use std::{path::PathBuf, time::Duration};
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    let profile = PathBuf::from(
        std::env::var_os("NOMIFUN_DATA_DIR").expect("isolated NOMIFUN_DATA_DIR is required"),
    );
    let report = PathBuf::from(
        std::env::var_os("NOMIFUN_KEYBOARD_VALIDATION_REPORT")
            .expect("validation report path is required"),
    );
    assert!(profile.is_absolute() && report.is_absolute());
    assert_eq!(std::env::var("GDK_BACKEND").as_deref(), Ok("x11"));
    tauri::Builder::default()
        .register_uri_scheme_protocol("keyboard-validation", |_, _| {
            tauri::http::Response::builder().header("Content-Type", "text/html; charset=utf-8")
                .body(b"<!doctype html><html><body></body></html>".to_vec()).unwrap()
        })
        .setup(move |app| {
            let subtitle_ready=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            for (label, title, width, height, visible) in [
                ("main", "NomiFun keyboard witness - main", 880.0, 600.0, false),
                ("keyboard-subtitle", "NomiFun keyboard witness - subtitle", 480.0, 320.0, false),
            ] {
                let ready=subtitle_ready.clone();
                WebviewWindowBuilder::new(app, label, WebviewUrl::External("keyboard-validation://localhost/".parse()?))
                    .title(title).inner_size(width, height).visible(visible).focused(visible)
                    .focusable(label != "keyboard-subtitle")
                    .decorations(label != "keyboard-subtitle")
                    .always_on_top(label == "keyboard-subtitle")
                    .on_page_load(move|window,payload|{if window.label()=="keyboard-subtitle"&&matches!(payload.event(),tauri::webview::PageLoadEvent::Finished){ready.store(true,std::sync::atomic::Ordering::Release);}})
                    .data_directory(profile.join(label)).build()?;
            }
            let app = app.handle().clone();
            let report = report.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(900)).await;
                let outcome = validate(&app,subtitle_ready).await;
                let value = match outcome {
                    Ok(value) => value,
                    Err(error) => serde_json::json!({"ok": false, "environment": environment(), "error": error.to_string()}),
                };
                if let Some(parent) = report.parent() { let _ = std::fs::create_dir_all(parent); }
                let _ = std::fs::write(&report, serde_json::to_vec_pretty(&value).unwrap());
                println!("{}", serde_json::json!({"ok": value["ok"], "report": report}));
                app.exit(if value["ok"] == true { 0 } else { 1 });
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("native X11 window witness failed");
}

#[cfg(target_os = "linux")]
fn x11_command(arguments: &[&str]) -> anyhow::Result<String> {
    let output = std::process::Command::new("timeout")
        .arg("4s")
        .arg("xdotool")
        .args(arguments)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "X11 compositor command {} failed or timed out: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[cfg(target_os = "linux")]
fn native_input(arguments: &[&str]) -> anyhow::Result<()> {
    let mut command = std::process::Command::new("timeout");
    command.arg("4s").arg("xdotool").args(arguments);
    if let Some(display) = std::env::var_os("NOMIFUN_KEYBOARD_VALIDATION_INPUT_DISPLAY") {
        command.env("DISPLAY", display).env_remove("XAUTHORITY");
    }
    anyhow::ensure!(
        command.output()?.status.success(),
        "private native input injection failed"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn pointer(window: &str, x: i32, y: i32, remaining: &[&str]) -> anyhow::Result<()> {
    if let Ok(host) = std::env::var("NOMIFUN_KEYBOARD_VALIDATION_INPUT_WINDOW") {
        let geometry = x11_command(&["getwindowgeometry", "--shell", window])?;
        let coordinate = |name: &str| -> anyhow::Result<i32> {
            Ok(geometry
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .ok_or_else(|| anyhow::anyhow!("native window coordinate missing"))?
                .parse()?)
        };
        let x = (coordinate("X=")? + x).to_string();
        let y = (coordinate("Y=")? + y).to_string();
        let mut arguments = vec![
            "mousemove",
            "--window",
            host.as_str(),
            x.as_str(),
            y.as_str(),
        ];
        arguments.extend_from_slice(remaining);
        native_input(&arguments)
    } else {
        let x = x.to_string();
        let y = y.to_string();
        let mut arguments = vec!["mousemove", "--window", window, x.as_str(), y.as_str()];
        arguments.extend_from_slice(remaining);
        native_input(&arguments)
    }
}

#[cfg(target_os = "linux")]
fn activate_control(window: &str) -> anyhow::Result<()> {
    if std::env::var_os("NOMIFUN_KEYBOARD_VALIDATION_INPUT_WINDOW").is_some() {
        let geometry = x11_command(&["getwindowgeometry", "--shell", window])?;
        let dimension = |name: &str| -> anyhow::Result<i32> {
            Ok(geometry
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .ok_or_else(|| anyhow::anyhow!("native window dimensions missing"))?
                .parse()?)
        };
        pointer(
            window,
            dimension("WIDTH=")? / 2,
            dimension("HEIGHT=")? / 3,
            &["click", "1"],
        )?;
        std::thread::sleep(std::time::Duration::from_millis(150));
        Ok(())
    } else {
        x11_command(&["windowactivate", "--sync", window])?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn environment() -> String {
    std::env::var("NOMIFUN_KEYBOARD_VALIDATION_ENVIRONMENT")
        .unwrap_or_else(|_| "X11 on WSLg".into())
}

#[cfg(target_os = "linux")]
fn active_role() -> anyhow::Result<&'static str> {
    Ok(
        match x11_command(&["getactivewindow", "getwindowname"])?.as_str() {
            "NomiFun keyboard validation - external control" => "control",
            "NomiFun keyboard witness - main" => "main",
            "NomiFun keyboard witness - subtitle" => "subtitle",
            _ => "other", // Never log the title of an unrelated user application.
        },
    )
}

#[cfg(target_os = "linux")]
struct ControlApplication(std::process::Child);
#[cfg(target_os = "linux")]
impl Drop for ControlApplication {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(target_os = "linux")]
fn restore_clipboard(text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    // xclip retains the restored selection after this short-lived validation
    // process exits, until its next real replacement.
    let mut restore = std::process::Command::new("timeout")
        .args(["4s", "xclip", "-selection", "clipboard", "-in"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    restore.stdin.take().unwrap().write_all(text.as_bytes())?;
    anyhow::ensure!(
        restore.wait()?.success(),
        "previous clipboard text could not be restored"
    );
    let read = std::process::Command::new("timeout")
        .args(["4s", "xclip", "-selection", "clipboard", "-out"])
        .stderr(std::process::Stdio::null())
        .output()?;
    anyhow::ensure!(
        read.status.success() && read.stdout == text.as_bytes(),
        "restored clipboard verification failed"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
async fn validate(
    app: &tauri::AppHandle,
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::{Context, ensure};
    use std::time::Duration;
    use tauri::Manager;
    let main = app
        .get_webview_window("main")
        .context("main window missing")?;
    let subtitle = app
        .get_webview_window("keyboard-subtitle")
        .context("subtitle window missing")?;
    const CONTROL_TITLE: &str = "NomiFun keyboard validation - external control";
    let _control = ControlApplication(
        std::process::Command::new("xmessage")
            .args([
                "-name",
                "nomifun-keyboard-control",
                "-title",
                CONTROL_TITLE,
                "-buttons",
                "OK",
                "-timeout",
                "45",
                "-center",
                "External application for window validation",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
    );
    let window_id = x11_command(&["search", "--sync", "--onlyvisible", "--name", CONTROL_TITLE])?
        .lines()
        .next()
        .context("control window missing")?
        .to_owned();
    ensure!(
        window_id.bytes().all(|value| value.is_ascii_digit()),
        "invalid X11 control window identity"
    );
    activate_control(&window_id)?;
    ensure!(
        active_role()? == "control",
        "external control application did not receive focus"
    );
    platform::show_without_activation(&subtitle)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while !ready.load(std::sync::atomic::Ordering::Acquire) {
        ensure!(
            std::time::Instant::now() < deadline,
            "WebKit validation document did not finish loading"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    tokio::time::sleep(Duration::from_millis(350)).await;
    ensure!(subtitle.is_visible()?, "subtitle did not map");
    let text = "实时字幕：中文 👋\nSecond line";
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    subtitle.eval_with_callback(format!("(() => {{ document.body.replaceChildren(); document.body.style.margin='0'; const header=document.createElement('header'); header.setAttribute('data-tauri-drag-region',''); header.style.cssText='height:40px;background:#ddd;user-select:none'; header.textContent='Native subtitle drag'; const text=document.createElement('div'); text.id='text'; text.style.cssText='padding:16px;white-space:pre-wrap'; text.textContent={}; document.body.append(header,text); return text.textContent; }})()", serde_json::to_string(text)?), move |value| { let _ = send.send(value); })?;
    let actual = tokio::task::spawn_blocking(move || receive.recv_timeout(Duration::from_secs(3)))
        .await?
        .context("subtitle text update callback did not complete")?;
    ensure!(
        serde_json::from_str::<String>(&actual)? == text,
        "subtitle text did not actually update in WebKit"
    );
    tokio::time::sleep(Duration::from_millis(250)).await;

    let subtitle_id = x11_command(&[
        "search",
        "--onlyvisible",
        "--name",
        "NomiFun keyboard witness - subtitle",
    ])?
    .lines()
    .next()
    .context("subtitle X11 identity missing")?
    .to_owned();
    pointer(&subtitle_id, 16, 64, &["mousedown", "1"])?;
    tokio::time::sleep(Duration::from_millis(120)).await;
    pointer(&subtitle_id, 260, 64, &["sleep", "0.1", "mouseup", "1"])?;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while active_role()? != "subtitle" {
        if std::time::Instant::now() >= deadline {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let window = subtitle.clone();
            app.run_on_main_thread(move || {
                use gtk::prelude::*;
                let native = window.gtk_window().unwrap();
                let explicit = unsafe {
                    native.data::<std::rc::Rc<std::cell::Cell<bool>>>(
                        "nomifun-subtitle-explicit-focus-v1",
                    )
                }
                .map(|value| unsafe { value.as_ref().get() });
                let _ = tx.send((explicit, native.accepts_focus(), native.is_active()));
            })?;
            let state =
                tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(2)))
                    .await??;
            let geometry = x11_command(&["getwindowgeometry", "--shell", &subtitle_id])?;
            let pointer = x11_command(&["getmouselocation", "--shell"])?;
            anyhow::bail!(
                "explicit pointer press did not activate subtitle (observed: {:?}, accepts focus: {}, native active: {}, active role: {}; geometry: {}; pointer: {})",
                state.0,
                state.1,
                state.2,
                active_role()?,
                geometry.replace('\n', ","),
                pointer.replace('\n', ",")
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    subtitle.eval_with_callback("window.getSelection().toString()", move |value| {
        let _ = send.send(value);
    })?;
    let selection =
        tokio::task::spawn_blocking(move || receive.recv_timeout(Duration::from_secs(3)))
            .await?
            .context("subtitle selection callback did not complete")?;
    ensure!(
        !serde_json::from_str::<String>(&selection)?.is_empty(),
        "native pointer drag could not select subtitle text"
    );
    let before = subtitle.outer_position()?;
    // The production data-tauri-drag-region listener receives the actual
    // WebKit mousedown, then invokes the permitted native drag command. This
    // preserves the compositor input serial and avoids racing an out-of-band
    // Rust start_dragging request ahead of the physical event.
    pointer(&subtitle_id, 420, 20, &["mousedown", "1"])?;
    tokio::time::sleep(Duration::from_millis(120)).await;
    native_input(&[
        "mousemove_relative",
        "--sync",
        "70",
        "60",
        "sleep",
        "0.2",
        "mouseup",
        "1",
    ])?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while subtitle.outer_position()? == before {
        ensure!(
            std::time::Instant::now() < deadline,
            "native subtitle drag did not move the window"
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    activate_control(&window_id)?;

    // A delayed main-thread callback must not resurrect an expired automatic
    // show after its caller has already received the timeout.
    subtitle.hide()?;
    app.run_on_main_thread(|| std::thread::sleep(Duration::from_secs(4)))?;
    ensure!(
        platform::show_without_activation(&subtitle).is_err(),
        "blocked main-thread operation did not expire"
    );
    tokio::time::sleep(Duration::from_millis(1300)).await;
    ensure!(
        !subtitle.is_visible()?,
        "expired automatic show ran late and remapped the window"
    );

    platform::toggle_main(app)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while !main.is_focused()? {
        ensure!(
            std::time::Instant::now() < deadline,
            "K1 request did not activate the main window in this compositor"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ensure!(
        active_role()? == "main",
        "X11 active window did not become the main window"
    );
    platform::toggle_main(app)?;
    ensure!(
        !main.is_visible()?,
        "K1 did not hide the foreground main window"
    );

    let clipboard = if std::env::var("NOMIFUN_KEYBOARD_VALIDATION_ALLOW_CLIPBOARD").as_deref()
        == Ok("1")
    {
        activate_control(&window_id)?;
        platform::show_without_activation(&subtitle)?;
        tokio::task::spawn_blocking(move || -> anyhow::Result<&'static str> {
            let previous = std::process::Command::new("timeout")
                .args(["4s", "xclip", "-selection", "clipboard", "-out"])
                .output()?;
            ensure!(previous.status.success(), "private clipboard seed is unavailable");
            let previous = String::from_utf8(previous.stdout)?;
            let body = format!("中文 👋\n第二行\n{}", "long text · 长文本\n".repeat(2048));
            let result = (|| -> anyhow::Result<()> {
                let editor = format!("(() => {{ document.body.replaceChildren(); const text=document.createElement('textarea'); text.id='text'; text.style.cssText='width:95vw;height:80vh'; document.body.append(text); return true; }})()");
                subtitle.eval(editor)?;
                std::thread::sleep(Duration::from_millis(100));
                let subtitle_id=x11_command(&["search","--onlyvisible","--name","NomiFun keyboard witness - subtitle"])?;
                let subtitle_id=subtitle_id.lines().next().context("subtitle window missing")?;
                pointer(subtitle_id,24,32,&["click","1"])?;
                subtitle.eval(format!("document.getElementById('text').value={}",serde_json::to_string(&body)?))?;
                std::thread::sleep(Duration::from_millis(100));
                native_input(&["key","--clearmodifiers","ctrl+a","ctrl+c"])?;
                let deadline=std::time::Instant::now()+Duration::from_secs(4);
                loop {
                    let output=std::process::Command::new("timeout").args(["4s","xclip","-selection","clipboard","-out"]).output()?;
                    if output.status.success()&&output.stdout==body.as_bytes(){break;}
                    ensure!(std::time::Instant::now()<deadline,"WebKit default copy did not publish the complete selected text");
                    std::thread::sleep(Duration::from_millis(30));
                }
                let (clear_tx, clear_rx) = std::sync::mpsc::sync_channel(1);
                subtitle.eval_with_callback(
                    "(() => { const text = document.getElementById('text'); text.value = ''; return text.value; })()",
                    move |value| { let _ = clear_tx.send(value); },
                )?;
                let cleared = clear_rx.recv_timeout(Duration::from_secs(3))
                    .context("subtitle clear callback did not complete")?;
                ensure!(serde_json::from_str::<String>(&cleared)?.is_empty(), "subtitle did not clear before hiding");
                subtitle.hide()?;
                std::thread::sleep(Duration::from_millis(150));
                let copied=std::process::Command::new("timeout").args(["4s","xclip","-selection","clipboard","-out"]).output()?;
                ensure!(copied.status.success()&&copied.stdout==body.as_bytes(),"system copy was unavailable after clearing and hiding the subtitle");
                Ok(())
            })();
            let restored = restore_clipboard(&previous);
            result?;
            restored?;
            Ok("passed_webkit_default_ctrl_c_selection_after_hide")
        }).await??
    } else {
        "skipped_shared_clipboard_not_enabled"
    };
    Ok(serde_json::json!({
        "ok": true, "environment": environment(), "automaticShowMapped": true,
        "pointerActivatedSubtitle": true, "pointerSelectedText": true,"nativeWindowDragMoved":true,
        "expiredShowDidNotRemap": true, "k1ActivatedMain": true, "k1HidMain": true,
        "clipboard": clipboard, "clipboardPath": "WebKit/GDK system clipboard; X11 or compositor selection bridge",
    }))
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("keyboard_window_validation is a Linux X11 integration witness");
    std::process::exit(2);
}
