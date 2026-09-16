//! Explicit localhost developer expressions. Never used as an input fallback.
use super::native::{self, View};
use nomifun_browser_platform::{run_guard::RunAdmissionError, runtime::*};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

fn local_origin(raw: &str) -> Result<String, WorkspaceError> {
    let url = url::Url::parse(raw).map_err(|_| WorkspaceError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some()
        || !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
        return Err(WorkspaceError::UnsupportedAction);
    }
    Ok(url.origin().ascii_serialization())
}

async fn cdp(view: &View, method: &str, params: Value) -> Result<Value, WorkspaceError> {
    native::protocol_call(view, method, params).await.map_err(|_| WorkspaceError::NativeCommandFailed)
}

pub(super) async fn run(
    view: &View,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    request: BrowserEvaluation,
    cancel: &CancellationToken,
) -> Result<BrowserEvaluationResult, WorkspaceError> {
    if request.expression.is_empty() || request.expression.len() > 65536 { return Err(WorkspaceError::ObservationLimit); }
    let current = || -> Result<String, WorkspaceError> {
        if cancel.is_cancelled() { return Err(RunAdmissionError::Cancelled.into()); }
        let current = metadata.lock().unwrap_or_else(|e| e.into_inner());
        if current.target != request.target { return Err(WorkspaceError::StaleTarget); }
        local_origin(&current.url)
    };
    let origin = current()?;
    let tree = cdp(view, "Page.getFrameTree", json!({})).await?;
    let frame = &tree["frameTree"]["frame"];
    if local_origin(frame["url"].as_str().ok_or(WorkspaceError::StaleTarget)?)? != origin { return Err(WorkspaceError::StaleTarget); }
    current()?;
    let frame_id = frame["id"].as_str().ok_or(WorkspaceError::StaleTarget)?;
    let world = cdp(view, "Page.createIsolatedWorld", json!({"frameId":frame_id,"worldName":"nomifun-developer-evaluation"})).await?;
    let context = world["executionContextId"].as_i64().ok_or(WorkspaceError::StaleTarget)?;
    current()?;
    // A fresh, single-slot nonce prevents numeric context-id reuse from applying
    // the expression to a replacement world. It never shares the semantic world.
    let nonce = uuid::Uuid::now_v7().to_string();
    let seed = format!("globalThis.__nomifunDeveloperCall = {}", json!(nonce));
    let seeded = cdp(view, "Runtime.evaluate", json!({"expression":seed,"contextId":context,"returnByValue":true,"timeout":5000,"disableBreaks":true,"includeCommandLineAPI":false})).await?;
    if seeded.get("exceptionDetails").is_some() { return Err(WorkspaceError::NativeCommandFailed); }
    current()?;
    let expression = format!(r#"(() => {{
        if (globalThis.__nomifunDeveloperCall !== {nonce} || location.origin !== {origin}) return JSON.stringify({{error:'Document changed before evaluation'}});
        delete globalThis.__nomifunDeveloperCall;
        try {{
            const value = (0, eval)({source});
            if (value && typeof value.then === 'function') return JSON.stringify({{error:'Return synchronous JSON data, not a Promise'}});
            const text = JSON.stringify({{value:value === undefined ? null : value}});
            return text.length > 131072 ? JSON.stringify({{error:'Evaluation result exceeds 128 KiB'}}) : text;
        }} catch (error) {{ return JSON.stringify({{error:String(error).slice(0,2048)}}); }}
    }})()"#, nonce=json!(nonce), origin=json!(origin), source=json!(request.expression));
    // Browser-scoped timeout, not Runtime.terminateExecution (which can arm a
    // termination for a later unrelated script). The owner awaits the callback.
    let response = cdp(view, "Runtime.evaluate", json!({"expression":expression,"contextId":context,"returnByValue":true,"awaitPromise":false,"timeout":5000,"disableBreaks":true,"includeCommandLineAPI":false,"userGesture":false,"allowUnsafeEvalBlockedByCSP":false})).await;
    current()?;
    let response = match response {
        Ok(response) => response,
        // WebView2 can report execution timeout via the COM command status,
        // rather than JS exceptionDetails. Do not claim a successful script or
        // guess the exact cause from elapsed time; preserve an explicit failure.
        Err(_) => return Ok(BrowserEvaluationResult {
            target: request.target, execution_kind: "developer_script",
            outcome: BrowserEvaluationOutcome::ScriptError { message: "The browser did not confirm script completion; it may have exceeded the execution deadline. Effects are not rolled back. Observe the page before continuing.".into() },
        }),
    };
    let outcome = if response.get("exceptionDetails").is_some() {
        BrowserEvaluationOutcome::ScriptError { message: "Script evaluation failed or exceeded its browser execution deadline; effects are not rolled back. Observe the current page before continuing.".into() }
    } else {
        let text = response["result"]["value"].as_str().filter(|text|text.len() <= 131072).ok_or(WorkspaceError::ObservationLimit)?;
        let value: Value = serde_json::from_str(text).map_err(|_| WorkspaceError::NativeCommandFailed)?;
        if let Some(error) = value["error"].as_str() {
            BrowserEvaluationOutcome::ScriptError { message: error.chars().take(2048).collect() }
        } else { BrowserEvaluationOutcome::Completed { value: value.get("value").cloned().ok_or(WorkspaceError::NativeCommandFailed)? } }
    };
    Ok(BrowserEvaluationResult { target: request.target, execution_kind: "developer_script", outcome })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn developer_expressions_are_limited_to_explicit_loopback_web_origins() {
        for url in ["http://localhost:3000/app", "http://127.0.0.1:5173/", "https://[::1]/"] { assert!(local_origin(url).is_ok()); }
        for url in ["https://example.com/", "http://localhost.example/", "http://192.168.1.1/", "file:///app.html", "about:blank", "http://u:p@localhost/"] { assert!(local_origin(url).is_err()); }
    }
}
