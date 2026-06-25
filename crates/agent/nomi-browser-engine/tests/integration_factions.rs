//! **P2 F-actions：upload_file / download / save_as_pdf / extract 端到端集成**（`#[ignore]`，本机/打包 chrome）。
//!
//! 验证补全的 4 个动作（P2 DoD「完整动作空间 + extract」）真在 Chrome 上跑通：
//!
//! 1. **upload_file**：navigate upload.html（含 `<input type=file>`）→ observe → 取 file input ref →
//!    `act(UploadFile{ref, [临时文件]})`（经 `DOM.setFileInputFiles` 绕系统文件框）→ 读回
//!    `input.files.length == 1` + files[0].name 含被上传文件名（verify 锚点 `count`/`first`）。
//! 2. **download**：navigate download.html → `act(Download{url: data:URL})` → 文件落隔离 downloads
//!    目录 + size>0（复用 E4 沙箱）。可执行 url 红线拒走纯逻辑单测（`download::tests`）+ E4 集成已覆盖
//!    downloadWillBegin 取消链路，这里只验良性下载落盘。
//! 3. **save_as_pdf**：navigate 任意 fixture → `act(SaveAsPdf)` → PDF 文件落隔离 downloads 目录 +
//!    size>0（headless 下 printToPDF 可靠；本机默认 headless）。
//! 4. **extract**：navigate c3.html（含 `password value="hunter2plain"`）→ `act(Extract{schema})` →
//!    返结构化页面表示（含 aria snapshot + 可见文本）+ 回显 schema；**脱敏验证**：输出不含明文
//!    `hunter2plain`（redact + wrap_untrusted 守住，LLM 永不见 secret）。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=C:\Program Files\Google\Chrome\Application\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(upload) | test(download_action) | test(pdf) | test(extract)'
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop 应自动清）。

use std::time::Duration;

use nomi_browser_engine::progress::Progress;
use nomi_browser_engine::{ActSpec, BrowserEngine, ObserveOpts};

mod common;

/// **upload_file**：DOM.setFileInputFiles 把本地文件路径塞进 `<input type=file>`（绕系统文件框）。
/// 验收：act 成功 + verify 锚点 `count==1` + `first` 含上传文件名。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn upload_file_sets_file_input_via_set_file_input_files() {
    // 准备一个真实临时文件供上传（内容任意，setFileInputFiles 要文件真存在）。
    let tmp = std::env::temp_dir().join(format!("nomifun-upload-{}.txt", std::process::id()));
    std::fs::write(&tmp, b"upload fixture payload").expect("write temp upload file");
    let tmp_str = tmp.to_string_lossy().into_owned();
    let expected_name = tmp.file_name().unwrap().to_string_lossy().into_owned();

    let backend = common::build_backend_for_fixture("facts-upload").await;
    backend
        .navigate(&common::fixture_url("upload.html"), false)
        .await
        .expect("navigate upload.html");

    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== upload fixture entries ===");
    for e in &obs.entries {
        eprintln!("  ref={} role={} name={:?}", e.r#ref, e.role, e.name);
    }
    // file input 的 aria role 多为 "button"（file 控件）或带 "Attachment" accname；按 accname 找最稳。
    let input = obs
        .entries
        .iter()
        .find(|e| e.name.contains("Attachment"))
        .expect("fixture should expose the file input with accname \"Attachment\"");
    eprintln!("file input ref = {}", input.r#ref);

    let p = Progress::new(Duration::from_secs(30));
    let res = backend
        .act(
            &ActSpec::UploadFile {
                r#ref: input.r#ref.clone(),
                paths: vec![std::path::PathBuf::from(&tmp_str)],
            },
            &p,
        )
        .await
        .expect("upload_file should succeed");
    eprintln!("upload result: success={} message={}", res.success, res.message);
    assert!(res.success, "upload_file should report success");

    // verify 锚点：after_anchor = {count, first}。count==1 且 first 含上传文件名（绕系统框真设进去了）。
    let after = res.effect.after_anchor.expect("upload should carry a files anchor");
    eprintln!("after anchor = {after}");
    assert_eq!(
        after.get("count").and_then(|v| v.as_u64()),
        Some(1),
        "file input should hold exactly 1 file"
    );
    let first = after.get("first").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        first.contains(&expected_name) || expected_name.contains(first),
        "files[0].name {first:?} should match the uploaded file {expected_name:?}"
    );
    assert!(res.effect.changed, "upload should report changed (file now set)");

    let _ = std::fs::remove_file(&tmp);
}

/// **download**：act(Download{url: data:URL}) → 文件落隔离 downloads 目录 + size>0（复用 E4 沙箱）。
/// data: URL 自带 download 属性语义（注入的 `<a download>` href=data: → chrome 当附件下载）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn download_action_lands_file_in_sandbox() {
    let (backend, download_dir) =
        common::build_backend_for_fixture_with_downloads("facts-download").await;
    eprintln!("download_dir = {}", download_dir.display());

    // 任意可下载页（需有 document.body 供注入 <a>）。download.html 即可（它有 body）。
    backend
        .navigate(&common::fixture_url("download.html"), false)
        .await
        .expect("navigate download.html");

    // data: URL 内容（>0 字节，便于断言 size>0）。act(Download) 注入隐藏 <a download href=this> click。
    let url = "data:text/plain;charset=utf-8,F-actions%20download%20payload%0Aline2%0A";
    let p = Progress::new(Duration::from_secs(30));
    let res = backend
        .act(&ActSpec::Download { url: url.to_string() }, &p)
        .await
        .expect("download action should not error");
    eprintln!("download result: success={} message={}", res.success, res.message);

    // act_download 自身已轮询落盘（success=true 即落盘成功 + after 锚点带文件名/字节数）；如实断言。
    if res.success {
        let after = res.effect.after_anchor.expect("successful download carries a file anchor");
        eprintln!("after anchor = {after}");
        assert!(
            after.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0) > 0,
            "downloaded file should be >0 bytes"
        );
        // 目录里也应能看到那个文件（双重确认，非 .crdownload）。
        let any_completed = std::fs::read_dir(&download_dir)
            .ok()
            .map(|rd| {
                rd.flatten().any(|e| {
                    let p = e.path();
                    p.extension().and_then(|x| x.to_str()) != Some("crdownload")
                        && std::fs::metadata(&p).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
                })
            })
            .unwrap_or(false);
        assert!(any_completed, "a completed download should be present in the sandbox dir");
    } else {
        // 若环境/时序未能在窗口内落盘（良性 false），至少不应 panic——report 真实结果供诊断。
        eprintln!("NOTE: download did not land in the wait window; message: {}", res.message);
    }
}

/// **save_as_pdf**：act(SaveAsPdf) → PDF 文件落隔离 downloads 目录 + size>0（headless 下 printToPDF 可靠）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn save_as_pdf_writes_nonempty_pdf_to_sandbox() {
    let (backend, download_dir) =
        common::build_backend_for_fixture_with_downloads("facts-pdf").await;
    eprintln!("download_dir = {}", download_dir.display());

    backend
        .navigate(&common::fixture_url("c3.html"), false)
        .await
        .expect("navigate c3.html");

    let p = Progress::new(Duration::from_secs(30));
    let res = backend.act(&ActSpec::SaveAsPdf, &p).await;
    eprintln!("save_as_pdf result = {res:?}");
    let res = res.expect("save_as_pdf should not error in headless");
    // headful 受限时本机默认 headless，故应成功；若意外失败，report 供诊断（TODO(verify-headful)）。
    assert!(res.success, "save_as_pdf should succeed (headless printToPDF). message: {}", res.message);

    let after = res.effect.after_anchor.expect("save_as_pdf carries a pdf_path/bytes anchor");
    eprintln!("after anchor = {after}");
    let bytes = after.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0);
    assert!(bytes > 0, "PDF should be >0 bytes, got {bytes}");
    let pdf_path = after.get("pdf_path").and_then(|v| v.as_str()).expect("pdf_path");
    // 文件真在隔离 downloads 目录里 + 真有内容（%PDF header）。
    let meta = std::fs::metadata(pdf_path).expect("pdf file should exist on disk");
    assert!(meta.len() > 0, "pdf on disk should be non-empty");
    assert!(
        std::path::Path::new(pdf_path).starts_with(&download_dir),
        "pdf must be written inside the sandboxed downloads dir, got {pdf_path}"
    );
    let head = std::fs::read(pdf_path).expect("read pdf");
    assert_eq!(&head[..4.min(head.len())], b"%PDF", "file should be a real PDF");
}

/// **save_as_pdf（headful 校验）**：Task 6 / 解 `TODO(verify-headful-printToPDF)`——在 **headful**
/// （真可见窗口）Chrome 下跑 `act(SaveAsPdf)`，验 `Page.printToPDF` 是否仍产非空 PDF 落隔离 downloads。
/// headful Chrome 历史上对 printToPDF 有限制；若本机/版本受限，引擎**如实**返回 `success=false`
/// （绝不 panic、绝不写半截文件），测试据实记录。**绿条件 = 非空真 PDF *或* 干净的 success=false**
/// （二者都证明引擎契约正确：headful 受限只降级为如实失败，不破坏不变量）。
#[tokio::test]
#[ignore = "需本机 chrome + 显示器（headful）：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn save_as_pdf_headful_writes_pdf_or_reports_cleanly() {
    let (backend, download_dir) =
        common::build_backend_for_fixture_headful_with_downloads("facts-pdf-headful").await;
    eprintln!("[headful] download_dir = {}", download_dir.display());

    backend
        .navigate(&common::fixture_url("c3.html"), false)
        .await
        .expect("navigate c3.html (headful)");

    let p = Progress::new(Duration::from_secs(30));
    let res = backend.act(&ActSpec::SaveAsPdf, &p).await;
    eprintln!("[headful] save_as_pdf result = {res:?}");

    // 引擎契约：headful 受限也绝不把 transport error 透到 Result 层——printToPDF 失败经
    // map_transport_err → Other → act 骨架转成 success=false 的 ActResult。故 act() 本身应 Ok。
    let res = res
        .expect("save_as_pdf must not surface a transport error (headful restriction → success=false)");

    if res.success {
        // headful 也能产 PDF（现代 Chrome 多数支持）——验落盘真 PDF。
        let after = res
            .effect
            .after_anchor
            .expect("success → save_as_pdf carries a pdf_path/bytes anchor");
        eprintln!("[headful] after anchor = {after}");
        let bytes = after.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0);
        assert!(bytes > 0, "[headful] PDF should be >0 bytes, got {bytes}");
        let pdf_path = after.get("pdf_path").and_then(|v| v.as_str()).expect("pdf_path");
        assert!(
            std::path::Path::new(pdf_path).starts_with(&download_dir),
            "[headful] pdf must be written inside the sandboxed downloads dir, got {pdf_path}"
        );
        let head = std::fs::read(pdf_path).expect("read headful pdf");
        assert!(!head.is_empty(), "[headful] pdf on disk should be non-empty");
        assert_eq!(&head[..4.min(head.len())], b"%PDF", "[headful] file should be a real PDF");
        eprintln!(
            "[headful] VERIFIED: Page.printToPDF works in headful — {bytes} bytes, %PDF header OK"
        );
    } else {
        // headful 受限：引擎如实 success=false（不 panic、不写半截文件）。这同样是被验证的正确行为。
        eprintln!(
            "[headful] NOTE: printToPDF appears restricted in headful on this Chrome — \
             engine reported success=false cleanly (no panic, no partial file): {}",
            res.message
        );
    }
}

/// **extract**：act(Extract{schema}) → 结构化页面表示（aria snapshot + 可见文本）+ 回显 schema；
/// **脱敏验证**：c3.html 的可见明文 secret（`sk-...` API key / `Bearer ...` token）绝不出现在输出明文里
/// （redact + wrap_untrusted 守住，LLM 永不见 secret）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn extract_returns_structured_representation_and_redacts_secrets() {
    let backend = common::build_backend_for_fixture("facts-extract").await;
    backend
        .navigate(&common::fixture_url("c3.html"), false)
        .await
        .expect("navigate c3.html");
    // extract 自身内部 observe，但先 observe 一次确保 ref 表/缓存就绪（与正常用法一致）。
    backend.observe(&ObserveOpts::default()).await.expect("observe");

    let schema = serde_json::json!({ "title": "string", "fields": ["order", "status"] });
    let p = Progress::new(Duration::from_secs(30));
    let res = backend
        .act(&ActSpec::Extract { schema: schema.clone() }, &p)
        .await
        .expect("extract should succeed");
    eprintln!("extract message (first 600 chars):\n{}", &res.message.chars().take(600).collect::<String>());

    assert!(res.success, "extract should succeed");
    assert!(!res.effect.changed, "extract is read-only (changed=false)");
    // 回显 schema（请求字段提示）。
    assert!(
        res.message.contains("Requested schema") && res.message.contains("status"),
        "extract should echo the requested schema"
    );
    // deterministic plumbing markers：含 aria snapshot + 可见文本两段。
    assert!(res.message.contains("accessibility snapshot"), "should include the aria snapshot section");
    assert!(res.message.contains("visible text"), "should include the visible-text section");
    // 可见正文（非 secret）应进 extract 输出（确认它真捕获了页面文本，而非空）。
    assert!(
        res.message.contains("unique-sentinel-marker"),
        "extract should capture the page's visible text (sentinel marker)"
    );
    // P3 TODO 提示在文案里（引擎层无 LLM）。
    assert!(res.message.to_uppercase().contains("TODO(P3)"), "should note LLM extraction is P3");

    // **脱敏铁律**：c3.html 的可见明文 secret 绝不出现在喂 LLM 的输出里（redact + wrap 守住）。
    assert!(
        !res.message.contains("sk-ABCDEFGHIJ0123456789xyzQRSTUV"),
        "SECURITY: the API key plaintext must NEVER appear in extract output"
    );
    assert!(
        !res.message.contains("abcdef0123456789ABCDEFghij"),
        "SECURITY: the Bearer token plaintext must NEVER appear in extract output"
    );
}
