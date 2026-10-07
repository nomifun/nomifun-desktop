//! Interactive native conformance only; never linked into the product UI.
use crate::windows::user_file_picker::{NativeFilePicker, Options, PickerMode};

pub(super) async fn verify_save() -> Result<serde_json::Value, String> {
    let root = tempfile::Builder::new().prefix("nomifun-save-picker-").tempdir()
        .map_err(|error| error.to_string())?;
    let name = "下载 验收.txt";
    let picker = NativeFilePicker::start(Options {
        title: "NomiFun save picker conformance".into(),
        initial_directory: root.path().into(),
        mode: PickerMode::Save { filename: name.into() },
        extensions: vec!["txt".into()],
    })?;
    let ready = tokio::time::timeout(std::time::Duration::from_secs(8), picker.opened()).await;
    if !matches!(ready, Ok(Ok(()))) {
        picker.close().await?;
        return Err("Native save picker did not open".into());
    }
    let receipt = picker.exit_receipt().await?;
    eprintln!("BROWSER_SAVE_PICKER_READY {}", serde_json::json!({"directory":root.path(),"filename":name}));
    let selected = tokio::time::timeout(std::time::Duration::from_secs(180), picker.finished()).await;
    picker.close().await?;
    receipt.wait().await.map_err(|error| error.to_string())?;
    let paths = selected.map_err(|_| "Native save selection timed out")??
        .ok_or("Native save selection was cancelled")?;
    if paths.len() != 1 || paths[0].file_name() != Some(std::ffi::OsStr::new(name))
        || paths[0].parent().map(std::fs::canonicalize).transpose().map_err(|e|e.to_string())?
            != Some(std::fs::canonicalize(root.path()).map_err(|e|e.to_string())?) {
        return Err("Native save returned a path outside its exact fixture destination".into());
    }
    // This checks the selected local path, not browser download completion.
    // Never overwrite any existing file selected accidentally during UI QA.
    let path = root.path().join(name);
    let text = "Local save-picker fixture 中文";
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|e| e.to_string())?;
    drop(file);
    if std::fs::read_to_string(path).map_err(|e|e.to_string())? != text {
        return Err("Save fixture bytes differ".into());
    }
    root.close().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"native_save_selection":true,"unicode_filename":true,"exact_destination":true,"helper_exited":true,"local_fixture_cleanup":true}))
}

pub(super) async fn verify() -> Result<serde_json::Value, String> {
    let root = tempfile::Builder::new()
        .prefix("nomifun-picker-selection-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let names = ["附件 空格.txt", "second.txt"];
    for name in names {
        std::fs::write(root.path().join(name), "Native picker conformance fixture")
            .map_err(|error| error.to_string())?;
    }
    for (phase, multiple, expected) in [
        ("single", false, vec![names[0]]),
        ("multiple", true, names.to_vec()),
        ("cancel", false, vec![]),
    ] {
        let picker = NativeFilePicker::start(Options {
            title: format!("NomiFun picker conformance: {phase}"),
            initial_directory: root.path().into(),
            mode: PickerMode::Open { multiple },
            extensions: vec![],
        })?;
        picker.opened().await?;
        let receipt = picker.exit_receipt().await?;
        eprintln!("BROWSER_PICKER_SELECTION_READY {phase}");
        let selected = picker.finished().await?;
        receipt.wait().await.map_err(|error| error.to_string())?;
        picker.close().await?;
        if expected.is_empty() {
            if selected.is_some() {
                return Err("Native Cancel returned selected files".into());
            }
        } else {
            let paths = selected.ok_or("Native selection was cancelled")?;
            let mut actual = paths
                .iter()
                .map(|path| path.canonicalize().map_err(|error| error.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            let mut expected = expected
                .iter()
                .map(|name| {
                    root.path()
                        .join(name)
                        .canonicalize()
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            actual.sort();
            expected.sort();
            if actual != expected {
                return Err(format!("Native {phase} selection returned incorrect paths"));
            }
        }
    }
    root.close().map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "scope":"picker-selection-only", "native_single":true,
        "native_multiple":true, "unicode_and_spaces":true,
        "native_cancel":true, "helper_exit_proven":true,
        "selection_fixture_cleanup":true
    }))
}
