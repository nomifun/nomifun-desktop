//! Production tool -> exact run -> observed native click -> HTTP download ->
//! validated workspace publication. No OS Save dialog or fake downloaded bytes.
use nomi_tools::Tool;
use nomifun_agent_kernel::ActiveCapabilitySetSnapshot;
use nomifun_browser_platform::{downloads::BrowserDownloadScope, runtime::*, workspace::BrowserWorkspaceService};
use serde_json::{Value, json};
use std::sync::Arc;
use sha2::{Digest, Sha256};
use tauri::Manager;

async fn element(tool: &dyn Tool, label: &str) -> Result<Value, String> {
    let result = tool.execute(json!({"operation":"observe"})).await;
    if result.is_error { return Err(result.content); }
    let value: Value = serde_json::from_str(&result.content).map_err(|e| e.to_string())?;
    value["elements"].as_array().and_then(|elements| elements.iter().find(|element| element["name"] == label))
        .map(|element| element["reference"].clone()).ok_or_else(|| format!("Missing observed download link: {label}"))
}
pub(super) async fn verify(app: &tauri::AppHandle, url: &str) -> Result<Value, String> {
    let root = tempfile::tempdir().map_err(|e| e.to_string())?;
    let service = BrowserWorkspaceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let key = BrowserWorkspaceKey { user_id: "agent-download-fixture".into(), conversation_id: "agent-download-fixture".into() };
    let workspace = service.ensure(key.clone(), "fixture".into(), BrowserProfile::Ephemeral).await.map_err(|e| e.to_string())?;
    let slot = super::browser_lifecycle::NativeBrowserTurnSlot::default();
    let result = async {
        workspace.user_command(BrowserTabCommand::Create { url: url.into() }).await.map_err(|e| e.to_string())?;
        let target = super::wait_workspace_page(&workspace, url).await?;
        let view = app.get_webview(&target.tab_id).ok_or("Missing native download tab")?;
        workspace.set_surface(BrowserSurfaceBounds { x: 20., y: 60., width: 1000., height: 600. }, true, Default::default()).await.map_err(|e| e.to_string())?;
        slot.begin(workspace.clone()).await.map_err(|e| e.to_string())?;
        let tool = Arc::new(super::browser_tool::ConversationBrowserTool::new(slot.clone(), ActiveCapabilitySetSnapshot {
            resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef { snapshot_id: "download-fixture".into(), snapshot_digest: "download-fixture".into() },
            generation: 1, active: ["browser.observe", "browser.act", "browser.download"].into_iter().map(Into::into).collect(),
        }).with_download_scope(Arc::new(BrowserDownloadScope::open(root.path()).map_err(|e| e.to_string())?)));
        let mut published = vec![];
        for index in 0..5 {
            let reference = element(tool.as_ref(), "Download local fixture").await?;
            let downloaded = tool.execute(json!({"operation":"download", "element":reference})).await;
            if downloaded.is_error { return Err(format!("download {index}: {}", downloaded.content)); }
            let value: Value = serde_json::from_str(&downloaded.content).map_err(|e| e.to_string())?;
            let path = value["download"]["path"].as_str().ok_or_else(|| format!("Missing publication proof: {value}"))?;
            let actual = std::fs::read(root.path().join(path)).map_err(|e| e.to_string())?;
            if actual != "Native WebView download 中文\n".as_bytes() || value["download"]["bytes"] != actual.len() || value["download"]["sha256"] != format!("{:x}", Sha256::digest(&actual)) {
                return Err("Published download does not match native HTTP bytes/proof".into());
            }
            if published.iter().any(|old| old==path) { return Err("Repeated download overwrote a previous output".into()); }
            published.push(path.to_owned());
            if super::windows::user_downloads::inspect(&view).await?.0.is_some() { return Err("Agent download opened a user Save picker".into()); }
        }
        for (label, expected) in [("Download redirected fixture", "Native WebView download 中文\n"), ("Download Blob export", "name,value\n中文,42\n"), ("Download empty Blob export", ""), ("Download popup attachment", "Native WebView download 中文\n")] {
            eprintln!("BROWSER_AGENT_DOWNLOAD_EXPORT {label}");
            let reference = element(tool.as_ref(), label).await?;
            let downloaded = tool.execute(json!({"operation":"download", "element":reference})).await;
            if downloaded.is_error { return Err(format!("{label}: {}", downloaded.content)); }
            let value: Value = serde_json::from_str(&downloaded.content).map_err(|e|e.to_string())?;
            let path = value["download"]["path"].as_str().ok_or("Missing export publication")?;
            let actual = std::fs::read(root.path().join(path)).map_err(|e|e.to_string())?;
            if actual != expected.as_bytes() || value["download"]["bytes"] != actual.len() || value["download"]["sha256"] != format!("{:x}", Sha256::digest(&actual)) { return Err(format!("{label}: native export bytes/proof mismatch")); }
            if published.iter().any(|old|old==path) { return Err("Export overwrote an earlier download".into()); }
            published.push(path.to_owned());
            let runtime = workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing export runtime")?;
            let opener = runtime.tabs.iter().find(|tab|tab.target.tab_id==target.tab_id).ok_or("Download lost its opener")?;
            let activated=tool.execute(json!({"operation":"tab","command":{"command":"activate","target":opener.target}})).await;
            if activated.is_error {return Err(format!("Restore opener after {label}: {}",activated.content));}
            if label=="Download popup attachment" {
                let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
                loop {
                    let runtime=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing runtime after popup download")?;
                    if runtime.tabs.len()==1 && runtime.tabs[0].target.tab_id==target.tab_id {break;}
                    if tokio::time::Instant::now()>=deadline {return Err("Download-only popup was not retired after native completion".into());}
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        }
        for label in ["Download disguised fixture", "Download executable fixture", "Download oversized fixture", "Download interrupted fixture", "Download disguised Blob export"] {
            eprintln!("BROWSER_AGENT_DOWNLOAD_REJECTION {label}");
            let previous = workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.and_then(|runtime| runtime.downloads.last().map(|entry| entry.id.clone()));
            let reference = element(tool.as_ref(), label).await?;
            let download_tool = tool.clone();
            let mut pending = tokio::spawn(async move { download_tool.execute(json!({"operation":"download", "element":reference})).await });
            let rejected = match tokio::time::timeout(std::time::Duration::from_secs(15), &mut pending).await {
                Ok(result) => result.map_err(|e| e.to_string())?,
                Err(_) => {
                    let observed = super::windows::user_downloads::inspect_native(&view).await?;
                    eprintln!("BROWSER_AGENT_DOWNLOAD_TIMEOUT {label}: {observed:?}");
                    slot.cancel();
                    let stopped = pending.await.map_err(|e| e.to_string())?;
                    eprintln!("BROWSER_AGENT_DOWNLOAD_STOP_RESULT {}: {:?}", stopped.content, super::windows::user_downloads::inspect_native(&view).await?);
                    return Err(format!("Download rejection did not settle: {label}: native={observed:?}; stop={}", stopped.content));
                },
            };
            if !rejected.is_error { return Err(format!("Unsafe download was accepted: {label}")); }
            let rejected_value: Value = serde_json::from_str(&rejected.content).map_err(|e|e.to_string())?;
            let expected_code = if label == "Download oversized fixture" { "BROWSER_DOWNLOAD_LIMIT" } else { "BROWSER_DOWNLOAD_DENIED" };
            if rejected_value["code"] != expected_code { return Err(format!("Unexpected download rejection: {label}: {}", rejected.content)); }
            let runtime=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing download runtime")?;
            let last=runtime.downloads.last().ok_or("Missing rejected download state")?;
            if Some(&last.id)==previous.as_ref() || last.state!=BrowserDownloadState::Failed || last.can_cancel { return Err(format!("Failed download did not finish its own cleanup: {label}: {:?}",last.state)); }
        }
        let reference = element(tool.as_ref(), "Download local fixture").await?;
        let recovered = tool.execute(json!({"operation":"download", "element":reference})).await;
        if recovered.is_error { return Err(format!("Download after network failure did not recover: {}", recovered.content)); }
        let value: Value = serde_json::from_str(&recovered.content).map_err(|e|e.to_string())?;
        let path = value["download"]["path"].as_str().ok_or("Missing recovery artifact")?;
        let bytes = std::fs::read(root.path().join(path)).map_err(|e|e.to_string())?;
        if bytes != "Native WebView download 中文\n".as_bytes() || value["download"]["sha256"] != format!("{:x}",Sha256::digest(&bytes)) || published.iter().any(|old|old==path) { return Err("Recovery download has invalid content or overwrote a file".into()); }
        published.push(path.to_owned());
        let reference = element(tool.as_ref(), "Download pending fixture").await?;
        let pending_tool = tool.clone();
        let pending = tokio::spawn(async move { pending_tool.execute(json!({"operation":"download", "element":reference})).await });
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            if super::windows::user_downloads::inspect(&view).await?.3 > 0 { break; }
            if tokio::time::Instant::now() > deadline { slot.cancel(); let _=pending.await; return Err("Slow native download never became active".into()); }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        slot.cancel();
        slot.settle().await.map_err(|e| e.to_string())?;
        if !pending.await.map_err(|e| e.to_string())?.is_error { return Err("Stop claimed a completed slow download".into()); }
        slot.finish().await.map_err(|e| e.to_string())?;
        if super::windows::user_downloads::inspect(&view).await?.1 != 0 { return Err("Stop returned before native download cleanup".into()); }
        if std::fs::read_dir(root.path().join("downloads")).map_err(|e| e.to_string())?.count() != published.len() { return Err("Rejected download left an output or partial file".into()); }
        Ok(json!({"real_native_click_and_http_bytes":true,"no_save_picker":true,"unique_publications":published.len(),"redirect_blob_and_empty_exports":true,"executable_and_size_rejection":true,"network_failure_cleanup_and_recovery":true,"stop_settles_before_unlock":true}))
    }.await;
    slot.cancel();
    let settled = slot.settle().await.map_err(|e| e.to_string());
    let finished = if settled.is_ok() { slot.finish().await.map_err(|e| e.to_string()) } else { settled.clone() };
    let closed = service.close(&key).await.map_err(|e| e.to_string());
    if let Err(error) = &result { eprintln!("BROWSER_AGENT_DOWNLOAD_FAILURE {error}; settle={settled:?}; close={closed:?}"); }
    settled?; finished?; closed?;
    drop(workspace);
    root.close().map_err(|e| e.to_string())?;
    result
}
