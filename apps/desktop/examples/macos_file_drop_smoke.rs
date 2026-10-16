//! Native file-drop regression through the real Wry view and Tauri event bridge.
//! Run: cargo run -p nomifun-desktop --example macos_file_drop_smoke
//! Uses hidden disposable WebViews and a private pasteboard; no backend or user data.
//! The JSON report records the actual display scale. Run on a Retina display to
//! additionally exercise its real 2x backing store; a scaled child view is always tested.

#[cfg(target_os = "macos")]
mod native {
    use std::{os::unix::fs::MetadataExt, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};

    use anyhow::{Context as _, ensure};
    use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained, runtime::ProtocolObject};
    use objc2_app_kit::{NSDragOperation, NSPasteboard, NSPasteboardTypeString, NSPasteboardWriting, NSView};
    use objc2_foundation::{MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSSize, NSString, NSURL};
    use serde::Serialize;
    use tauri::{DragDropEvent, Manager, WebviewEvent, WindowEvent};

    struct DragInfoIvars {
        pasteboard: Retained<NSPasteboard>,
        window_point: NSPoint,
    }

    // Wry reads exactly these two NSDraggingInfo selectors. Sending the real
    // destination selectors exercises production dispatch without synthesizing
    // an OS drag, changing the general clipboard, or requiring input permission.
    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = DragInfoIvars]
        struct FileDropInfo;

        unsafe impl NSObjectProtocol for FileDropInfo {}

        impl FileDropInfo {
            #[unsafe(method(draggingLocation))]
            fn dragging_location(&self) -> NSPoint { self.ivars().window_point }

            #[unsafe(method_id(draggingPasteboard))]
            fn dragging_pasteboard(&self) -> Retained<NSPasteboard> { self.ivars().pasteboard.clone() }
        }
    );

    #[derive(Serialize)]
    struct Surface {
        label: &'static str,
        display_scale: f64,
        flipped: bool,
        offset: [f64; 2],
        bounds_origin: [f64; 2],
        bounds_scale: f64,
        expected_position: [i32; 2],
    }

    #[derive(Serialize)]
    struct Observation {
        label: &'static str,
        content: &'static str,
        phase: &'static str,
        position: Option<[i32; 2]>,
        paths: Option<Vec<PathBuf>>,
    }

    #[derive(Default, Serialize)]
    struct Report {
        surfaces: Vec<Surface>,
        observations: Vec<Observation>,
        failures: Vec<String>,
    }

    fn observe(app: &tauri::AppHandle, report: &Arc<Mutex<Report>>, paths: &[PathBuf], label: &'static str, event: &DragDropEvent) {
        let mut report = report.lock().unwrap();
        let (phase, position, actual_paths) = match event {
            DragDropEvent::Enter { paths, position } => ("enter", Some([position.x as i32, position.y as i32]), Some(paths)),
            DragDropEvent::Over { position } => ("over", Some([position.x as i32, position.y as i32]), None),
            DragDropEvent::Drop { paths, position } => ("drop", Some([position.x as i32, position.y as i32]), Some(paths)),
            DragDropEvent::Leave => ("leave", None, None),
            _ => return,
        };
        let index = report.observations.iter().filter(|entry| entry.label == label).count();
        let content = ["files", "url", "text"].get(index / 4).copied().unwrap_or("unexpected");
        if ["enter", "over", "drop", "leave"].get(index % 4) != Some(&phase) {
            report.failures.push(format!("{label}: unexpected {phase} at index {index}"));
        }
        let expected_paths = if content == "files" { paths } else { &[] };
        // NSURL and the filesystem can return decomposed Unicode names on
        // macOS. Verify each actual path resolves to its expected real file,
        // preserving order and count without mistaking normalization for loss.
        if actual_paths.is_some_and(|actual| actual.len() != expected_paths.len() || !actual.iter().zip(expected_paths).all(|(actual, expected)| {
            match (std::fs::metadata(actual), std::fs::metadata(expected)) {
                (Ok(actual), Ok(expected)) => actual.is_file() && expected.is_file()
                    && actual.dev() == expected.dev() && actual.ino() == expected.ino(),
                _ => false,
            }
        })) {
            report.failures.push(format!("{label}: native paths {actual_paths:?} != {expected_paths:?}"));
        }
        let expected = report.surfaces.iter().find(|surface| surface.label == label).map(|surface| surface.expected_position);
        if position.is_some() && position != expected {
            report.failures.push(format!("{label}: {phase} position {position:?} != {expected:?}"));
        }
        report.observations.push(Observation { label, content, phase, position, paths: actual_paths.cloned() });
        let done = report.observations.len() == 24;
        let failed = !report.failures.is_empty();
        drop(report);
        if done { app.exit(i32::from(failed)); }
    }

    fn drive(view: &NSView, report: &Arc<Mutex<Report>>, paths: &[PathBuf], label: &'static str, bounds_scale: f64) -> anyhow::Result<()> {
        let mtm = MainThreadMarker::new().context("file drop must run on the AppKit main thread")?;
        let frame = view.frame();
        let display_scale = view.window().context("test WebView has no NSWindow")?.backingScaleFactor();
        view.setBoundsOrigin(NSPoint::new(5.0, 7.0));
        view.setBoundsSize(NSSize::new(frame.size.width / bounds_scale, frame.size.height / bounds_scale));
        let bounds = view.bounds();
        let flipped = view.isFlipped();
        let local_distance = NSPoint::new(81.25, 123.5);
        let local_point = NSPoint::new(bounds.origin.x + local_distance.x,
            if flipped { bounds.origin.y + local_distance.y } else { bounds.origin.y + bounds.size.height - local_distance.y });
        let window_point = view.convertPoint_toView(local_point, None);
        let expected_position = [
            (local_distance.x * bounds_scale * display_scale).round() as i32,
            (local_distance.y * bounds_scale * display_scale).round() as i32,
        ];
        report.lock().unwrap().surfaces.push(Surface {
            label, display_scale, flipped, offset: [frame.origin.x, frame.origin.y],
            bounds_origin: [bounds.origin.x, bounds.origin.y], bounds_scale, expected_position,
        });
        for content in ["files", "url", "text"] {
            let pasteboard = NSPasteboard::pasteboardWithUniqueName();
            pasteboard.clearContents();
            match content {
                "files" => {
                    let urls = paths.iter().map(|path| NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()))).collect::<Vec<_>>();
                    let objects = urls.iter().map(|url| ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&**url)).collect::<Vec<_>>();
                    ensure!(pasteboard.writeObjects(&NSArray::from_slice(&objects)), "private file pasteboard could not be populated");
                }
                "url" => {
                    let url = NSURL::URLWithString(&NSString::from_str("https://example.invalid/drop-fixture")).unwrap();
                    let object = ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&*url);
                    ensure!(pasteboard.writeObjects(&NSArray::arrayWithObject(object)), "private URL pasteboard could not be populated");
                }
                _ => ensure!(pasteboard.setString_forType(&NSString::from_str("普通文本"), unsafe { NSPasteboardTypeString }), "private text pasteboard could not be populated"),
            }
            let info = FileDropInfo::alloc(mtm).set_ivars(DragInfoIvars { pasteboard: pasteboard.clone(), window_point });
            let info: Retained<FileDropInfo> = unsafe { msg_send![super(info), init] };
            unsafe {
                let entered: NSDragOperation = msg_send![view, draggingEntered: &*info];
                let updated: NSDragOperation = msg_send![view, draggingUpdated: &*info];
                let dropped: bool = msg_send![view, performDragOperation: &*info];
                let _: () = msg_send![view, draggingExited: &*info];
                ensure!(entered == NSDragOperation::Copy && updated == NSDragOperation::Copy && dropped,
                    "native drag was rejected");
            }
            pasteboard.clearContents();
        }
        Ok(())
    }

    pub fn run() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let paths = vec![directory.path().join("会话 附件.txt"), directory.path().join("résumé🦀.png")];
        for path in &paths { std::fs::write(path, b"native file drop fixture")?; }
        let report = Arc::new(Mutex::new(Report::default()));
        // Only configuration is synthetic. Builder::default uses the real
        // Wry runtime, and both events and native view handles are real.
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().identifier = "com.nomifun.desktop.component.filedrop".into();
        let app = tauri::Builder::default()
            .setup(move |app| {
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External("about:blank".parse()?))
                    .title("NomiFun native file-drop regression").inner_size(880.0, 600.0)
                    .min_inner_size(880.0, 600.0).visible(false).incognito(true).build()?;
                Ok(())
            })
            .build(context)?;
        let timed_out = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let timeout_completed = completed.clone();
        let timeout_flag = timed_out.clone();
        let timeout_app = app.handle().clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(10));
            if !timeout_completed.load(Ordering::Acquire) {
                timeout_flag.store(true, Ordering::Release);
                timeout_app.exit(1);
            }
        });
        let run_report = report.clone();
        let exit_code = app.run_return(move |app, event| {
            match &event {
                tauri::RunEvent::WindowEvent { label, event: WindowEvent::DragDrop(event), .. } if label == "main" => {
                    observe(app, &run_report, &paths, "main", event);
                }
                tauri::RunEvent::WebviewEvent { label, event: WebviewEvent::DragDrop(event), .. } => {
                    if label == "main" { observe(app, &run_report, &paths, "main", event); }
                    if label == "drop-child" { observe(app, &run_report, &paths, "child", event); }
                }
                _ => {}
            }
            if !matches!(event, tauri::RunEvent::Ready) { return; }
            let app = app.clone();
            let run_report = run_report.clone();
            let paths = paths.clone();
            // Creating a child waits for UI dispatch; the Tauri event callback
            // must return while a worker waits for that creation to complete.
            std::thread::spawn(move || {
                let result = (|| -> anyhow::Result<()> {
                    app.get_window("main").context("fixture window")?.add_child(
                        tauri::webview::WebviewBuilder::new("drop-child", tauri::WebviewUrl::External("about:blank".parse()?)).incognito(true),
                        tauri::LogicalPosition::new(113.0, 97.0), tauri::LogicalSize::new(500.0, 300.0),
                    )?;
                    for (label, report_label, scale) in [("main", "main", 1.0), ("drop-child", "child", 2.0)] {
                        let driver_report = run_report.clone();
                        let driver_paths = paths.clone();
                        let handle = app.clone();
                        app.get_webview(label).context("fixture WebView")?.with_webview(move |platform| {
                            let view = unsafe { &*platform.inner().cast::<NSView>() };
                            if let Err(error) = drive(view, &driver_report, &driver_paths, report_label, scale) {
                                driver_report.lock().unwrap().failures.push(error.to_string());
                                handle.exit(1);
                            }
                        })?;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    run_report.lock().unwrap().failures.push(error.to_string());
                    app.exit(1);
                }
            });
        });
        completed.store(true, Ordering::Release);
        let mut report = report.lock().unwrap();
        if timed_out.load(Ordering::Acquire) { report.failures.push("native file-drop events timed out".into()); }
        if report.observations.len() != 24 { report.failures.push("expected four native events for three drag types on each WebView".into()); }
        println!("{}", serde_json::to_string_pretty(&*report)?);
        ensure!(exit_code == 0 && report.failures.is_empty(), "MACOS_FILE_DROP_SMOKE_FAIL");
        println!("MACOS_FILE_DROP_SMOKE_PASS");
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> { native::run() }

#[cfg(not(target_os = "macos"))]
fn main() { eprintln!("macos_file_drop_smoke requires macOS"); }
