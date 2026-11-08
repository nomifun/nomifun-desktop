//! Application installation lifetime. Independent of voice input, USB devices,
//! model selection and optional speech feature initialization.
pub(crate) mod platform;
use nomifun_desktop_maintenance::{desktop_control, maintenance::StartupAdmission};
use std::sync::Mutex;
use tauri::Manager;

pub(crate) struct StartupAdmissionState(Mutex<Option<StartupAdmission>>);
impl StartupAdmissionState {
    pub(crate) fn new(admission: Option<StartupAdmission>) -> Self { Self(Mutex::new(admission)) }
}

/// Legacy device retirement may disable voice input, but must not make the
/// workbench unavailable. The application lifetime endpoint remains independent
/// so installers can still stop the GUI and verify old owners themselves.
pub(crate) fn start(
    app: &tauri::AppHandle,
    ready: impl FnOnce(tauri::AppHandle, Option<String>) + Send + 'static,
) -> anyhow::Result<()> {
    let app = app.clone();
    // Legacy OS queries and teardown may take seconds. Retain startup admission
    // until the endpoint exists, while keeping the workbench main loop usable.
    std::thread::Builder::new().name("nomifun-app-maintenance-init".into()).spawn(move || {
        let mut voice_error = platform::retire_legacy_service().err().map(|error| {
            tracing::warn!(%error, "legacy input retirement incomplete; voice input disabled");
            format!("旧版语音输入资源清理未完成，语音功能已暂停：{error:#}")
        });
        let stop_app = app.clone();
        let exit_app = app.clone();
        let guard = desktop_control::register(
            move || tauri::async_runtime::block_on(crate::keyboard::prepare_external_maintenance(&stop_app))
                .map_err(anyhow::Error::msg),
            move || exit_app.exit(0),
        );
        match guard {
            Ok(guard) => {
                app.manage(guard);
                if let Some(admission) = app.try_state::<StartupAdmissionState>() {
                    drop(admission.0.lock().unwrap_or_else(|error| error.into_inner()).take());
                }
            }
            Err(error) => {
                // Keep the shared admission held: no installer may infer that a
                // running application with a broken endpoint is absent.
                tracing::error!(%error, "application maintenance endpoint unavailable");
                voice_error = Some(format!("应用维护端点不可用，语音功能已暂停：{error:#}"));
            }
        }
        let callback_app = app.clone();
        if let Err(error) = app.run_on_main_thread(move || ready(callback_app, voice_error)) {
            tracing::error!(%error, "voice input initialization dispatch failed");
        }
    })?;
    Ok(())
}

fn command(args: &[std::ffi::OsString]) -> Option<anyhow::Result<bool>> {
    let unregister = match args.first()?.to_str()? {
        "--app-maintenance-stop" | "--keyboard-stop" => false,
        "--app-maintenance-unregister" | "--keyboard-unregister" => true,
        "--keyboard-service" => return Some(Err(anyhow::anyhow!("standalone keyboard service is retired"))),
        _ => return None,
    };
    Some(if args.len() == 1 { Ok(unregister) } else { Err(anyhow::anyhow!("maintenance command does not accept additional arguments")) })
}

pub(crate) fn early_entry() -> Option<std::process::ExitCode> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = command(&args)?.and_then(platform::stop_application);
    Some(match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("NomiFun application maintenance: {error:#}");
            std::process::ExitCode::FAILURE
        }
    })
}

#[cfg(test)]
mod tests {
    use super::command;
    fn args(values: &[&str]) -> Vec<std::ffi::OsString> { values.iter().map(std::ffi::OsString::from).collect() }
    #[test]
    fn retired_and_malformed_maintenance_launches_cannot_fall_through_to_gui() {
        for values in [vec!["--keyboard-service"], vec!["--keyboard-service", "--extra"], vec!["--app-maintenance-stop", "--extra"], vec!["--keyboard-unregister", "--extra"]] {
            assert!(command(&args(&values)).unwrap().is_err());
        }
        assert!(!command(&args(&["--app-maintenance-stop"])).unwrap().unwrap());
        assert!(command(&args(&["--app-maintenance-unregister"])).unwrap().unwrap());
    }
    #[test]
    fn regular_launches_and_other_headless_entries_remain_available() {
        for values in [vec![], vec!["mcp-requirement-stdio"], vec!["--unregister-system-execution"]] {
            assert!(command(&args(&values)).is_none());
        }
    }
}
