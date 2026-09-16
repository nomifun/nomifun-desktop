//! One managed process per native picker. IFileDialog::Close returning S_OK
//! is not exit proof; cancellation waits for the exact helper process tree.
use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    io::{Read, Write},
    path::PathBuf,
    process::{ExitCode, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, watch},
};
use tokio_util::sync::CancellationToken;
use windows::{
    Win32::{
        Foundation::{ERROR_CANCELLED, HWND},
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            Ole::IOleWindow,
        },
        UI::{
            Shell::{
                FOS_ALLOWMULTISELECT, FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
                FOS_NOCHANGEDIR, FOS_NOREADONLYRETURN, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
                FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IFileSaveDialog,
                IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, HWND_MESSAGE, IsWindowVisible, KillTimer, SetTimer,
                WINDOW_EX_STYLE, WINDOW_STYLE,
            },
        },
    },
    core::{HSTRING, Interface, PCWSTR, w},
};

const HELPER_ARG: &str = "--browser-file-picker";
const MAX_REQUEST: usize = 64 * 1024;
const MAX_REPLY: usize = 1024 * 1024;
type Outcome = Result<Option<Vec<PathBuf>>, String>;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Options {
    pub title: String,
    pub initial_directory: PathBuf,
    pub mode: PickerMode,
    pub extensions: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PickerMode {
    Open { multiple: bool },
    Save { filename: String },
}
impl PickerMode {
    fn multiple(&self) -> bool {
        matches!(self, Self::Open { multiple: true })
    }
}

pub(crate) fn valid_save_name(name: &str) -> bool {
    if name.is_empty()
        || name.encode_utf16().count() > 255
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    options: Options,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Reply {
    Opened,
    Result { paths: Option<Vec<PathBuf>> },
    Failed,
}
#[derive(Clone, Default)]
struct State {
    opened: bool,
    outcome: Option<Outcome>,
}
struct Control {
    cancel: CancellationToken,
    state: watch::Sender<State>,
    process: Mutex<Option<ManagedChildProcess>>,
}
struct Owner(Arc<Control>);
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.cancel.cancel();
    }
}
#[derive(Clone)]
pub(crate) struct NativeFilePicker {
    owner: Arc<Owner>,
}

fn validate(options: &Options) -> Result<(), String> {
    if options.title.is_empty()
        || options.title.len() > 512
        || options.title.chars().any(char::is_control)
        || !options.initial_directory.is_absolute()
        || options
            .initial_directory
            .to_str()
            .is_none_or(|path| path.contains('\0'))
        || !super::file_accept::valid_extensions(&options.extensions)
        || matches!(&options.mode, PickerMode::Save { filename } if !valid_save_name(filename))
    {
        return Err("Invalid browser file picker options".into());
    }
    Ok(())
}
fn validate_paths(paths: &Option<Vec<PathBuf>>, multiple: bool) -> Result<(), String> {
    if paths.as_ref().is_some_and(|paths| {
        paths.is_empty()
            || paths.len() > if multiple { 256 } else { 1 }
            || paths.iter().any(|path| {
                !path.is_absolute() || path.to_str().is_none_or(|path| path.contains('\0'))
            })
    }) {
        return Err("Invalid native picker file result".into());
    }
    Ok(())
}
impl NativeFilePicker {
    pub(crate) fn start(options: Options) -> Result<Self, String> {
        validate(&options)?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "Browser picker requires a live host runtime")?;
        let (state, _) = watch::channel(State::default());
        let control = Arc::new(Control {
            cancel: CancellationToken::new(),
            state,
            process: Mutex::new(None),
        });
        let picker = Self {
            owner: Arc::new(Owner(control.clone())),
        };
        let running_control = control.clone();
        let worker = runtime.spawn(async move { run(options, &running_control).await });
        runtime.spawn(async move {
            let outcome = match worker.await {
                Ok(outcome) => outcome,
                Err(_) => Err("Browser picker worker failed".into()),
            };
            // All exits, including failures before the reply transaction and
            // worker panics, retain ownership until cleanup has been attempted.
            let outcome = match cleanup(&control).await {
                Ok(()) => outcome,
                Err(error) => Err(error),
            };
            control
                .state
                .send_modify(|state| state.outcome = Some(outcome));
        });
        Ok(picker)
    }
    pub(crate) fn cancel(&self) {
        self.owner.0.cancel.cancel();
    }
    pub(crate) async fn finished(&self) -> Outcome {
        let mut state = self.owner.0.state.subscribe();
        loop {
            if let Some(outcome) = state.borrow().outcome.clone() {
                return outcome;
            }
            state
                .changed()
                .await
                .map_err(|_| "Browser picker completion was lost")?;
        }
    }
    /// Failed cleanup retains the managed process for an explicit retry.
    pub(crate) async fn close(&self) -> Result<(), String> {
        self.cancel();
        let _ = self.finished().await;
        cleanup(&self.owner.0).await
    }
    pub(crate) async fn exit_receipt(
        &self,
    ) -> Result<nomi_process_runtime::ChildProcessCleanup, String> {
        self.owner
            .0
            .process
            .lock()
            .await
            .as_ref()
            .and_then(|process| process.cleanup_receipt())
            .ok_or_else(|| "Picker has no active process receipt".into())
    }
    pub(crate) async fn opened(&self) -> Result<(), String> {
        let mut state = self.owner.0.state.subscribe();
        loop {
            if state.borrow().opened {
                return Ok(());
            }
            if state.borrow().outcome.is_some() {
                return Err("Picker ended before showing its native window".into());
            }
            state
                .changed()
                .await
                .map_err(|_| "Browser picker worker stopped")?;
        }
    }
}
async fn cleanup(control: &Control) -> Result<(), String> {
    let mut process = control.process.lock().await;
    if let Some(child) = process.as_mut() {
        child
            .shutdown()
            .await
            .map_err(|_| "Browser picker process-tree exit was not proven")?;
        process.take();
    }
    Ok(())
}
fn necessary_environment(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_string_lossy().to_ascii_uppercase().as_str(),
        "SYSTEMROOT"
            | "WINDIR"
            | "SYSTEMDRIVE"
            | "USERPROFILE"
            | "LOCALAPPDATA"
            | "APPDATA"
            | "TEMP"
            | "TMP"
            | "HOMEDRIVE"
            | "HOMEPATH"
            | "PROGRAMDATA"
            | "PROGRAMFILES"
            | "PROGRAMFILES(X86)"
            | "PROGRAMW6432"
    )
}
async fn run(options: Options, control: &Arc<Control>) -> Outcome {
    if control.cancel.is_cancelled() {
        return Ok(None);
    }
    let request = serde_json::to_vec(&Request {
        version: 2,
        options: options.clone(),
    })
    .map_err(|_| "Browser picker request could not be encoded")?;
    if request.len() > MAX_REQUEST {
        return Err("Browser picker request exceeds its limit".into());
    }
    let executable =
        std::env::current_exe().map_err(|_| "Browser picker executable is unavailable")?;
    let directory = executable
        .parent()
        .ok_or("Picker executable has no parent directory")?;
    let mut builder = ChildProcessBuilder::new(&executable);
    builder
        .arg(HELPER_ARG)
        // A project directory must not become a DLL search location for Shell.
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Shell extensions must not inherit provider credentials or app tokens.
    for (name, _) in std::env::vars_os() {
        if !necessary_environment(&name) {
            builder.env_remove(name);
        }
    }
    let child = builder
        .spawn_managed()
        .map_err(|_| "Browser picker process could not start")?;
    let (mut input, output) = {
        let mut owned = control.process.lock().await;
        *owned = Some(child);
        let child = owned.as_mut().unwrap().child_mut();
        (
            child
                .stdin
                .take()
                .ok_or("Browser picker input pipe is unavailable")?,
            child
                .stdout
                .take()
                .ok_or("Browser picker output pipe is unavailable")?,
        )
    };
    let transaction = async {
        input
            .write_all(&request)
            .await
            .map_err(|_| "Browser picker request failed")?;
        input
            .shutdown()
            .await
            .map_err(|_| "Browser picker request did not finish")?;
        drop(input);
        let mut output = BufReader::new(output);
        let mut opened = false;
        loop {
            let mut bytes = vec![];
            loop {
                let available = output
                    .fill_buf()
                    .await
                    .map_err(|_| "Browser picker response failed")?;
                if available.is_empty() {
                    return Err("Browser picker exited without a result".to_owned());
                }
                let count = available
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(available.len(), |index| index + 1);
                if bytes.len() + count > MAX_REPLY {
                    return Err("Browser picker response exceeds its limit".into());
                }
                let ended = available[count - 1] == b'\n';
                bytes.extend_from_slice(&available[..count]);
                output.consume(count);
                if ended {
                    break;
                }
            }
            let reply: Reply =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid browser picker response")?;
            match reply {
                Reply::Opened if !opened => {
                    opened = true;
                    control.state.send_modify(|state| state.opened = true);
                }
                Reply::Opened => return Err("Duplicate browser picker window notification".into()),
                Reply::Failed => return Err("Native browser file picker failed".into()),
                Reply::Result { paths } => {
                    validate_paths(&paths, options.mode.multiple())?;
                    return Ok(paths);
                }
            }
        }
    };
    let outcome =
        tokio::select! {biased;_=control.cancel.cancelled()=>Ok(None),result=transaction=>result};
    // A reply is not exit proof; cleanup also closes helper-owned child modals.
    cleanup(control).await?;
    if control.cancel.is_cancelled() {
        Ok(None)
    } else {
        outcome
    }
}

/// Dispatch before app runtime/data initialization or single-instance routing.
pub(crate) fn helper_entry() -> Option<ExitCode> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(HELPER_ARG)) {
        return None;
    }
    let result = (|| -> Result<(), String> {
        if std::env::args_os().count() != 2 {
            return Err("Invalid picker command".into());
        }
        let mut bytes = vec![];
        std::io::stdin()
            .take((MAX_REQUEST + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Picker request read failed")?;
        if bytes.len() > MAX_REQUEST {
            return Err("Picker request exceeds its limit".into());
        }
        let request: Request =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid picker request")?;
        if request.version != 2 {
            return Err("Unsupported picker protocol".into());
        }
        validate(&request.options)?;
        emit(&Reply::Result {
            paths: show(request.options)?,
        })
    })();
    if result.is_err() {
        let _ = emit(&Reply::Failed);
    }
    Some(if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
fn emit(reply: &Reply) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(reply).map_err(|_| "Picker response encoding failed")?;
    if bytes.len() > MAX_REPLY {
        return Err("Picker response exceeds its limit".into());
    }
    bytes.push(b'\n');
    let mut output = std::io::stdout().lock();
    output
        .write_all(&bytes)
        .and_then(|()| output.flush())
        .map_err(|_| "Picker response write failed".into())
}
thread_local! {static DIALOG:RefCell<Option<IFileDialog>>=RefCell::default();}
static ANNOUNCED: AtomicBool = AtomicBool::new(false);
unsafe extern "system" fn tick(_: HWND, _: u32, _: usize, _: u32) {
    let dialog = DIALOG.with(|dialog| dialog.borrow().clone());
    if let Some(dialog) = dialog {
        if let Ok(window) = dialog
            .cast::<IOleWindow>()
            .and_then(|window| unsafe { window.GetWindow() })
        {
            if unsafe { IsWindowVisible(window) }.as_bool()
                && !ANNOUNCED.swap(true, Ordering::AcqRel)
            {
                let _ = emit(&Reply::Opened);
            }
        }
    }
}
struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
struct DialogWindow(HWND);
impl Drop for DialogWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.0), 1);
            let _ = DestroyWindow(self.0);
        }
        DIALOG.with(|dialog| dialog.borrow_mut().take());
    }
}
fn show(options: Options) -> Outcome {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|_| "Picker COM initialization failed")?;
    let _apartment = ComApartment;
    let dialog: IFileDialog = match &options.mode {
        PickerMode::Open { .. } => unsafe {
            CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
        }
        .and_then(|dialog| dialog.cast()),
        PickerMode::Save { .. } => unsafe {
            CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
        }
        .and_then(|dialog| dialog.cast()),
    }
    .map_err(|_| "Native file picker is unavailable")?;
    let mut flags = FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR | FOS_DONTADDTORECENT;
    if options.mode.multiple() {
        flags |= FOS_ALLOWMULTISELECT;
    }
    match &options.mode {
        PickerMode::Open { .. } => flags |= FOS_FILEMUSTEXIST,
        PickerMode::Save { filename } => {
            flags |= FOS_OVERWRITEPROMPT | FOS_NOREADONLYRETURN;
            unsafe { dialog.SetFileName(PCWSTR(HSTRING::from(filename).as_ptr())) }
                .map_err(|_| "Download filename is unavailable")?;
        }
    }
    let filter_pattern = (!options.extensions.is_empty()).then(|| {
        HSTRING::from(
            options
                .extensions
                .iter()
                .map(|extension| format!("*.{extension}"))
                .collect::<Vec<_>>()
                .join(";"),
        )
    });
    unsafe {
        dialog
            .SetOptions(flags)
            .map_err(|_| "Picker options failed")?;
        if let Some(pattern) = &filter_pattern {
            let filters = [
                windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC {
                    pszName: PCWSTR(pattern.as_ptr()),
                    pszSpec: PCWSTR(pattern.as_ptr()),
                },
                windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC {
                    pszName: w!("*.*"),
                    pszSpec: w!("*.*"),
                },
            ];
            dialog
                .SetFileTypes(&filters)
                .map_err(|_| "Native file type filter failed")?;
            dialog
                .SetFileTypeIndex(1)
                .map_err(|_| "Native file type selection failed")?;
        }
        dialog
            .SetTitle(PCWSTR(HSTRING::from(&options.title).as_ptr()))
            .map_err(|_| "Picker title failed")?;
        let directory = options
            .initial_directory
            .to_str()
            .ok_or("Picker directory is not Unicode")?;
        let item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR(HSTRING::from(directory).as_ptr()), None)
                .map_err(|_| "Picker directory is unavailable")?;
        dialog
            .SetFolder(&item)
            .map_err(|_| "Picker directory failed")?;
    }
    let window = DialogWindow(
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .map_err(|_| "Picker observation window failed")?,
    );
    DIALOG.with(|slot| *slot.borrow_mut() = Some(dialog.clone()));
    if unsafe { SetTimer(Some(window.0), 1, 25, Some(tick)) } == 0 {
        return Err("Picker observation timer failed".into());
    }
    // An isolated helper must not leave a cross-process owner HWND disabled.
    if let Err(error) = unsafe { dialog.Show(None) } {
        return if error.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0) {
            Ok(None)
        } else {
            Err("Native file picker failed".into())
        };
    }
    let items = if options.mode.multiple() {
        let open = dialog
            .cast::<IFileOpenDialog>()
            .map_err(|_| "Open picker is unavailable")?;
        let selection =
            unsafe { open.GetResults() }.map_err(|_| "File selection is unavailable")?;
        let count = unsafe { selection.GetCount() }.map_err(|_| "File selection count failed")?;
        if count > 256 {
            return Err("File selection exceeds its limit".into());
        }
        (0..count)
            .map(|index| {
                unsafe { selection.GetItemAt(index) }
                    .map_err(|_| "Selected item is unavailable".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![unsafe { dialog.GetResult() }.map_err(|_| "File selection is unavailable")?]
    };
    let mut paths = Vec::with_capacity(items.len());
    for item in items {
        let raw = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }
            .map_err(|_| "Selected item is not a filesystem file")?;
        let allocation = webview2_com::CoTaskMemPWSTR::from(raw);
        if raw.is_null() {
            return Err("Selected path is unavailable".into());
        }
        let wide = unsafe { raw.as_wide() };
        if wide.len() > 32768 {
            return Err("Selected path exceeds its limit".into());
        }
        let path = String::from_utf16(wide).map_err(|_| "Selected path is not Unicode")?;
        drop(allocation);
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("Selected path is not absolute".into());
        }
        paths.push(path);
    }
    let result = Some(paths);
    validate_paths(&result, options.mode.multiple())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_mode_has_one_result_and_rejects_path_or_device_names() {
        for filename in ["下载 空格.txt", "report.csv", "image.png"] {
            assert!(valid_save_name(filename));
            let mode = PickerMode::Save {
                filename: filename.into(),
            };
            assert!(!mode.multiple());
            assert!(
                validate(&Options {
                    title: "Save download".into(),
                    initial_directory: PathBuf::from("C:/work"),
                    mode,
                    extensions: vec![],
                })
                .is_ok()
            );
        }
        for filename in [
            "",
            ".",
            "..",
            "../file",
            "a\\b",
            "file:stream",
            "NUL.txt",
            "COM1",
            "LPT².log",
            "con .txt",
            "file.",
            "file ",
            "a\n.txt",
        ] {
            assert!(
                !valid_save_name(filename),
                "accepted unsafe suggested name {filename:?}"
            );
        }
        assert!(!valid_save_name(&"x".repeat(256)));
        assert!(
            serde_json::from_value::<PickerMode>(serde_json::json!({
                "kind":"save","filename":"file.txt","multiple":true
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<Options>(serde_json::json!({
                "title":"picker","initial_directory":"C:/work","multiple":false,"extensions":[]
            }))
            .is_err(),
            "retired protocol shape has no compatibility alias"
        );
    }
    #[test]
    fn result_preserves_cancel_and_requested_selection_cardinality() {
        let files = Some(vec![PathBuf::from("C:/work/附件 空格.txt")]);
        assert!(validate_paths(&None, false).is_ok());
        assert!(validate_paths(&files, false).is_ok());
        assert!(validate_paths(&Some(vec![]), true).is_err());
        assert!(validate_paths(&Some(vec![PathBuf::from("relative.txt")]), true).is_err());
        assert!(validate_paths(&Some(vec![PathBuf::from("C:/file\0.txt")]), true).is_err());
        let pair = Some(vec![PathBuf::from("C:/a.txt"), PathBuf::from("C:/b.txt")]);
        assert!(validate_paths(&pair, false).is_err());
        assert!(validate_paths(&pair, true).is_ok());
        assert!(validate_paths(&Some(vec![PathBuf::from("C:/a.txt"); 257]), true).is_err());
        assert!(
            validate(&Options {
                title: "picker".into(),
                initial_directory: PathBuf::from("C:/work\0"),
                mode: PickerMode::Open { multiple: false },
                extensions: vec![],
            })
            .is_err()
        );
    }
    #[test]
    fn helper_protocol_is_strict_and_does_not_inherit_provider_environment() {
        assert!(serde_json::from_str::<Request>(r#"{"version":2,"options":{"title":"picker","initial_directory":"C:/work","mode":{"kind":"open","multiple":false},"extensions":[]},"token":"forged"}"#).is_err());
        assert!(
            validate(&Options {
                title: "picker".into(),
                initial_directory: PathBuf::from("C:/work"),
                mode: PickerMode::Open { multiple: false },
                extensions: vec!["txt;*.*".into()],
            })
            .is_err()
        );
        assert!(!necessary_environment(std::ffi::OsStr::new(
            "OPENAI_API_KEY"
        )));
        assert!(!necessary_environment(std::ffi::OsStr::new(
            "NOMIFUN_LOCAL_TRUST"
        )));
        assert!(necessary_environment(std::ffi::OsStr::new("SystemRoot")));
    }
}
