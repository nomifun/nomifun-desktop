//! Windows native witness for the production global-shortcut controller.
//! Opens only a disposable native window: no backend, WebView, microphone,
//! serial port, provider, user profile or installed NomiFun is initialized.
//! Run with --owned-native-fixture [--report ABSOLUTE_JSON_PATH].
#[cfg(windows)]
#[path = "../src/keyboard/shortcuts.rs"]
mod shortcuts;

#[cfg(windows)]
mod native {
    use super::shortcuts;
    use anyhow::{Context, Result, ensure};
    use nomifun_keyboard::document::Settings;
    use serde_json::json;
    use std::{ffi::c_void, mem::size_of, path::PathBuf, sync::{Arc, Mutex}, time::{Duration, Instant}};

    type Hwnd = *mut c_void;
    const CONTROL: u16 = 0x11;
    const SHIFT: u16 = 0x10;
    const ALT: u16 = 0x12;
    const F23: u16 = 0x86;
    const F24: u16 = 0x87;
    const MODIFIERS: u32 = 0x0001 | 0x0002 | 0x0004 | 0x4000;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct KeyboardInput { key: u16, scan: u16, flags: u32, time: u32, extra: usize }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MouseInput { x: i32, y: i32, data: u32, flags: u32, time: u32, extra: usize }
    #[repr(C)]
    union InputData { keyboard: KeyboardInput, mouse: MouseInput }
    #[repr(C)]
    struct Input { kind: u32, data: InputData }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> Hwnd;
        fn GetWindowThreadProcessId(window: Hwnd, pid: *mut u32) -> u32;
        fn GetAsyncKeyState(key: i32) -> i16;
        fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
        fn RegisterHotKey(window: Hwnd, id: i32, modifiers: u32, key: u32) -> i32;
        fn UnregisterHotKey(window: Hwnd, id: i32) -> i32;
    }

    struct Reservation(i32);
    impl Reservation {
        fn acquire(id: i32, key: u16) -> Result<Self> {
            ensure!(unsafe { RegisterHotKey(std::ptr::null_mut(), id, MODIFIERS, u32::from(key)) } != 0,
                "reserve fixture shortcut: {}", std::io::Error::last_os_error());
            Ok(Self(id))
        }
    }
    impl Drop for Reservation {
        fn drop(&mut self) { unsafe { UnregisterHotKey(std::ptr::null_mut(), self.0); } }
    }

    fn owned_foreground(window: usize) -> Result<()> {
        let foreground = unsafe { GetForegroundWindow() };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(foreground, &mut pid); }
        ensure!(foreground as usize == window && pid == std::process::id(),
            "fixture does not own foreground; no input was injected (foreground PID {pid})");
        Ok(())
    }

    fn chord(window: usize, key: u16) -> Result<()> {
        owned_foreground(window)?;
        for key in [CONTROL, SHIFT, ALT, F23, F24] {
            ensure!(unsafe { GetAsyncKeyState(i32::from(key)) } & i16::MIN == 0,
                "a test modifier/function key is already held; no input was injected");
        }
        // One serialized SendInput packet includes all down/up edges. No held
        // modifier is left behind while waiting for the plugin's callbacks.
        let keys = [(CONTROL, 0), (SHIFT, 0), (ALT, 0), (key, 0),
            (key, 2), (ALT, 2), (SHIFT, 2), (CONTROL, 2)];
        let inputs: Vec<_> = keys.into_iter().map(|(key, flags)| Input { kind: 1,
            data: InputData { keyboard: KeyboardInput { key, scan: 0, flags, time: 0, extra: 0x4e4f4d49 } } }).collect();
        ensure!(unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<Input>() as i32) } == inputs.len() as u32,
            "fixture input packet was not fully accepted: {}", std::io::Error::last_os_error());
        Ok(())
    }

    fn until(stage: &str, mut ready: impl FnMut() -> bool) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !ready() {
            ensure!(Instant::now() < deadline, "native shortcut witness timed out: {stage}");
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }

    fn validate(app: &tauri::AppHandle, window: usize) -> Result<serde_json::Value> {
        until("owned window foreground", || owned_foreground(window).is_ok())?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let collect = events.clone();
        let mut controller = shortcuts::Controller::new(app.clone(), move |action, pressed| {
            collect.lock().unwrap().push((format!("{action:?}"), pressed));
        });
        let result = (|| -> Result<_> {
            // Exercise the actual shipping defaults as well as isolated F-key
            // chords: a default reserved by Windows must fail this witness.
            // No default shortcut is injected into the user's desktop.
            let defaults = Settings::default();
            ensure!(controller.reconfigure(&defaults).map_err(anyhow::Error::msg)? == 4,
                "all four shipping default shortcuts must register with the OS");
            controller.shutdown().map_err(anyhow::Error::msg)?;
            ensure!(controller.registered() == 0, "default shortcuts were not released before F-key tests");
            ensure!(events.lock().unwrap().is_empty(), "unexpected user input while reserving defaults");
            let mut settings = Settings::default();
            settings.enabled = true;
            settings.shortcuts.toggle.clear(); settings.shortcuts.beautify.clear(); settings.shortcuts.cancel.clear();
            settings.shortcuts.dictate = "Control+Alt+Shift+F23".into();
            ensure!(controller.reconfigure(&settings).map_err(anyhow::Error::msg)? == 1, "initial registration count");
            let reservation = Reservation::acquire(0x6e01, F24)?;
            let mut changed = settings.clone();
            changed.shortcuts.dictate = "Control+Alt+Shift+F24".into();
            let conflict = controller.reconfigure(&changed).expect_err("OS-reserved F24 must conflict");
            ensure!(controller.registered() == 1, "conflict must preserve the original registration");
            chord(window, F23)?;
            until("original F23 dictation press/release", || events.lock().unwrap().len() >= 2)?;
            ensure!(*events.lock().unwrap() == vec![("Dictate".into(), true), ("Dictate".into(), false)],
                "original binding did not retain its native press/release edges");
            drop(reservation);
            changed.shortcuts.toggle = "Control+Alt+Shift+F23".into();
            ensure!(controller.reconfigure(&changed).map_err(anyhow::Error::msg)? == 2, "updated registration count");
            chord(window, F23)?;
            until("rebound F23 toggle press/release", || events.lock().unwrap().len() >= 4)?;
            chord(window, F24)?;
            until("F24 dictation press/release", || events.lock().unwrap().len() >= 6)?;
            ensure!(*events.lock().unwrap() == vec![("Dictate".into(), true), ("Dictate".into(), false),
                ("Toggle".into(), true), ("Toggle".into(), false), ("Dictate".into(), true), ("Dictate".into(), false)],
                "rebinding did not route native edges to the intended actions");
            controller.shutdown().map_err(anyhow::Error::msg)?;
            ensure!(controller.registered() == 0, "shutdown retained an active shortcut");
            let free23 = Reservation::acquire(0x6e02, F23)?;
            let free24 = Reservation::acquire(0x6e03, F24)?;
            drop((free23, free24));
            chord(window, F23)?; chord(window, F24)?;
            std::thread::sleep(Duration::from_millis(150));
            ensure!(events.lock().unwrap().len() == 6, "events continued after shutdown");
            Ok(json!({"ok":true,"events":*events.lock().unwrap(),"conflict":conflict,
                "defaultPlanRegistered":4,"defaultPlanReleased":true,"defaultBindings":defaults.shortcuts,
                "actualOsRegistration":true,"actualPressedAndReleased":true,"rebindingVerified":true,
                "osRegistrationsReleased":true,"foregroundPid":std::process::id(),"testKeys":["Control+Alt+Shift+F23","Control+Alt+Shift+F24"],
                "microphoneOpened":false,"serialOpened":false,"providerCalled":false,"productionProfileUsed":false}))
        })();
        let cleanup = controller.shutdown().map_err(anyhow::Error::msg);
        match result { Ok(value) => { cleanup?; Ok(value) }, Err(error) => { let _ = cleanup; Err(error) } }
    }

    pub fn run() -> Result<()> {
        let mut args = std::env::args_os().skip(1);
        ensure!(args.next().as_deref() == Some(std::ffi::OsStr::new("--owned-native-fixture")),
            "run with --owned-native-fixture [--report ABSOLUTE_JSON_PATH]");
        let private = tempfile::Builder::new().prefix("nomifun-voice-shortcut-").tempdir()?.keep();
        let report = match args.next() {
            None => private.join("report.json"),
            Some(flag) if flag == "--report" => PathBuf::from(args.next().context("missing report path")?),
            Some(_) => anyhow::bail!("unsupported fixture argument"),
        };
        ensure!(args.next().is_none() && report.is_absolute(), "absolute report path required");
        println!("{}", json!({"fixtureRoot":private,"report":report}));
        tauri::Builder::default().plugin(shortcuts::plugin()).setup(move |app| {
            let window = tauri::window::WindowBuilder::new(app, "voice-shortcut-owned-fixture")
                .title("NomiFun voice shortcut fixture — no microphone or hardware")
                .inner_size(880.0, 600.0).min_inner_size(880.0, 600.0).focused(true).build()?;
            window.set_focus()?;
            let window = window.hwnd()?.0 as usize;
            let app = app.handle().clone();
            std::thread::spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| validate(&app, window)));
                let value = match outcome { Ok(Ok(value)) => value, Ok(Err(error)) => json!({"ok":false,"error":format!("{error:#}")}),
                    Err(_) => json!({"ok":false,"error":"native shortcut fixture panicked"}) };
                let _ = std::fs::create_dir_all(report.parent().unwrap());
                let _ = std::fs::write(&report, serde_json::to_vec_pretty(&value).unwrap());
                println!("{}", json!({"ok":value["ok"],"report":report}));
                app.exit(if value["ok"] == true { 0 } else { 1 });
            });
            Ok(())
        }).run(tauri::generate_context!()).context("run owned shortcut fixture")?;
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> { native::run() }
#[cfg(not(windows))]
fn main() { eprintln!("This native shortcut witness requires Windows."); std::process::exit(2); }
