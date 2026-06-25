//! 真 Chrome 集成测试：验证 per-tab 调试事件捕获（console/errors/network）。
//! 需 `NOMIFUN_CHROME_BINARY` 环境变量指向 Chrome 可执行文件。
//! 运行：`cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(captures_console_error_and_network)'`

use nomi_browser_engine::{create_engine, EngineConfig};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

/// fixture HTML: console.error + throw + fetch（base64 编码避免 data: URL 截断）。
fn fixture_html() -> String {
    let html = r#"<!DOCTYPE html>
<html>
<head><title>Debug Capture Test</title></head>
<body>
<script>
// 1) console.error
console.error("test-debug-error-message", 42);
// 2) uncaught exception
setTimeout(function throwIt() {
    throw new Error("test-uncaught-exception");
}, 50);
// 3) fetch (triggers network activity)
fetch("https://httpbin.org/get?foo=bar").catch(function(){});
// 4) a second fetch to a non-existent domain (will fail)
fetch("https://this-domain-does-not-exist-12345.invalid/api").catch(function(){});
</script>
</body>
</html>"#;
    let encoded = B64.encode(html.as_bytes());
    format!("data:text/html;base64,{encoded}")
}

#[tokio::test]
#[ignore = "需 NOMIFUN_CHROME_BINARY（真 Chrome）：调试事件捕获冒烟"]
async fn captures_console_error_and_network() {
    let engine = create_engine(EngineConfig::default())
        .await
        .expect("engine builds with NOMIFUN_CHROME_BINARY set");

    let url = fixture_html();
    engine.navigate(&url, false).await.expect("navigate");

    // 给 Chrome 一点时间让事件到来（setTimeout + fetch 需要异步完成）。
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let snap = engine.debug_snapshot().await.expect("debug_snapshot");

    // ── Console: 至少有一条 error 级别的 console 消息 ──
    assert!(
        !snap.console.is_empty(),
        "expected at least one console entry, got none"
    );
    let has_error_msg = snap
        .console
        .iter()
        .any(|e| e.text.contains("test-debug-error-message"));
    assert!(
        has_error_msg,
        "expected console.error('test-debug-error-message'), got: {:?}",
        snap.console.iter().map(|e| &e.text).collect::<Vec<_>>()
    );

    // ── Errors: 至少有一条未捕获异常 ──
    // 注意：setTimeout throw 可能需要更长时间才会被捕获
    // 如果还没来，等一下再取
    let snap2 = if snap.errors.is_empty() {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        engine.debug_snapshot().await.expect("debug_snapshot 2")
    } else {
        snap
    };

    assert!(
        !snap2.errors.is_empty(),
        "expected at least one page error (uncaught exception), got none"
    );
    let has_exception = snap2
        .errors
        .iter()
        .any(|e| e.message.contains("test-uncaught-exception"));
    assert!(
        has_exception,
        "expected 'test-uncaught-exception' in errors, got: {:?}",
        snap2.errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );

    // ── Network: 至少有一条网络请求 ──
    assert!(
        !snap2.network.is_empty(),
        "expected at least one network entry (the fetch), got none"
    );
    // httpbin.org 或 the-invalid-domain 应出现
    let has_httpbin = snap2.network.iter().any(|e| e.url.contains("httpbin.org"));
    let has_invalid = snap2
        .network
        .iter()
        .any(|e| e.url.contains("this-domain-does-not-exist"));
    assert!(
        has_httpbin || has_invalid,
        "expected network entries for our fetch URLs, got: {:?}",
        snap2.network.iter().map(|e| &e.url).collect::<Vec<_>>()
    );
}
