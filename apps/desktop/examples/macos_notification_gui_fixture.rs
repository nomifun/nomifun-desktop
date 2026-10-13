//! Independent native notification component acceptance; never a product route.
//!
//! Root builds this example and macos_application_controller, packages a real
//! App with com.nomifun.desktop.audit.notifications, then uses the existing
//! DesktopDevelopment LaunchServices transport with --report /NEW/report.json.
//! That transport name does not make this the formal NomiFun Dev send path.
//! No permission request or notification is issued before a trusted UI click.
//! The report parent must already be an owned 0700 directory. Reports are new
//! 0600 files; they contain no environment, launch nonce, credentials or user data.

#[cfg(target_os = "macos")]
#[path = "../src/macos_appkit_main_loop.rs"]
mod macos_appkit_main_loop;
#[cfg(target_os = "macos")]
#[path = "../src/macos_notifications.rs"]
mod macos_notifications;
#[cfg(target_os = "macos")]
#[path = "../src/native_api_plugins.rs"]
mod native_api_plugins;
#[cfg(target_os = "macos")]
#[path = "../src/macos_development_launch.rs"]
mod application_launch;

#[cfg(target_os = "macos")]
mod fixture {
    use std::{
        borrow::Cow,
        fs::{File, OpenOptions},
        io::{Seek, SeekFrom, Write},
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::PathBuf,
        ptr::NonNull,
        sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}},
        time::Duration,
    };

    use anyhow::{Context as _, ensure};
    use block2::RcBlock;
    use objc2_foundation::{NSArray, NSBundle, NSString};
    use objc2_user_notifications::{UNNotification, UNNotificationRequest, UNUserNotificationCenter};
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};
    use tauri::{AppHandle, Assets, Manager, Runtime, State, WebviewUrl, WebviewWindow,
        utils::assets::{AssetKey, AssetsIter, CspHash}};
    use tokio::sync::oneshot;

    const BUNDLE_ID: &str = "com.nomifun.desktop.audit.notifications";
    const PRODUCT_NAME: &str = "NomiFun Notification Audit";
    const MAX_REPORT_BYTES: usize = 128 * 1024;
    const MAX_OPERATIONS: usize = 64;
    static ASSET_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
    static NAVIGATION_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

    const HTML: &str = r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>NomiFun Notification Audit</title>
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
body{font:16px system-ui;margin:28px;color:#18202b;background:#fafbfc}h1{font-size:24px}
p{max-width:800px;line-height:1.55}button{padding:10px 14px;margin:4px;font:inherit}
pre{padding:16px;background:#edf0f4;white-space:pre-wrap;overflow-wrap:anywhere;max-height:300px;overflow:auto}
.note{color:#5a2630}.status{font-weight:600}code{font-size:14px}
</style></head><body>
<h1>NomiFun 通知组件验收 / Notification Audit</h1>
<p class="note">这是独立测试 App，不是正式 NomiFun Dev 的发送链。仅查询真实系统状态；请求权限和发送通知必须点击对应按钮。通知提交被系统接受，不代表横幅已显示。记录失败时可关闭此测试窗口正常中止；原生 ⌘Q 路径尚未验收。</p>
<p>真实应用身份：<code id="identity">读取中</code><br>本次投递标记：<code id="nonce">读取中</code></p>
<div>
<button id="query" disabled>查询权限 / Query</button>
<button id="request" disabled>请求权限 / Request</button>
<button id="notify" disabled>发送一次测试通知 / Send</button>
<button id="delivered" disabled>读取本次投递证据 / Delivered</button>
<button id="cleanup" disabled>清理本次通知 / Clear this test</button>
<button id="finish" disabled>完成并正常退出 / Finish</button>
</div>
<p class="status" id="status">初始化</p><pre id="output" aria-live="polite"></pre>
<script>
(() => {
  const invoke = (name, args = {}) => window.__TAURI_INTERNALS__.invoke(name, args);
  const buttons = [...document.querySelectorAll('button')];
  const status = document.getElementById('status');
  const output = document.getElementById('output');
  let busy = true, initialized = false, retired = false, info;
  const display = value => { output.textContent = JSON.stringify(value, null, 2); };
  const enable = () => buttons.forEach(button => { button.disabled = busy || retired || !initialized; });
  const errorText = error => typeof error === 'string' ? error : error instanceof Error ? error.message : JSON.stringify(error);
  const run = async (action, trusted) => {
    if (busy || retired || !initialized) return;
    busy = true; enable();
    status.textContent = `${action}: 等待真实 IPC 完成`;
    let job;
    try {
      job = await invoke('fixture_begin', { action, trusted });
      let result, error, ok = false;
      try {
        if (action === 'query') result = await invoke('plugin:notification|is_permission_granted');
        else if (action === 'request') result = await invoke('plugin:notification|request_permission');
        else if (action === 'notify') result = await invoke('plugin:notification|notify', { options: { title: info.title, body: info.body } });
        else if (action === 'delivered') result = await invoke('fixture_delivered', { job });
        else result = await invoke('fixture_cleanup', { job });
        result = result === undefined ? null : result;
        ok = true;
      } catch (cause) { error = errorText(cause); }
      const receipt = { job, action, ok, result: ok ? result : null, error: ok ? null : error };
      // Record only after the actual IPC promise settled. A recorder failure
      // leaves a pending job or a sticky persistence failure; never cleanup PASS.
      await invoke('fixture_settle', receipt);
      display(receipt);
      status.textContent = ok ? `${action}: IPC 已返回` : `${action}: IPC 已拒绝`;
    } catch (cause) {
      display({ recorder_error: errorText(cause), job: job ?? null, action });
      status.textContent = '验收记录失败；无法确认报告落盘';
    } finally { busy = false; enable(); }
  };
  for (const action of ['query', 'request', 'notify', 'delivered', 'cleanup']) {
    document.getElementById(action).addEventListener('click', event => {
      if (event.isTrusted) void run(action, true);
    });
  }
  document.getElementById('finish').addEventListener('click', async event => {
    if (!event.isTrusted || busy || retired || !initialized) return;
    busy = true; retired = true; enable();
    status.textContent = '等待正常退出';
    try { await invoke('fixture_finish', { trusted: true }); }
    catch (cause) {
      retired = false; busy = false; enable();
      display({ finish_error: errorText(cause) }); status.textContent = '退出未获准';
    }
  });
  void (async () => {
    try {
      info = await invoke('fixture_info');
      document.getElementById('identity').textContent = `${info.identity.bundle_id} · PID ${info.identity.pid}`;
      document.getElementById('nonce').textContent = info.nonce;
      initialized = true; busy = false; enable();
      // The only automatic operation is a read-only native settings query.
      await run('query', false);
    } catch (cause) {
      display({ initialization_error: errorText(cause) }); status.textContent = '初始化失败';
    }
  })();
})();
</script></body></html>"#;

    struct FixtureAssets;
    impl<R: Runtime> Assets<R> for FixtureAssets {
        fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
            let key = key.as_ref();
            let present = matches!(key, "/index.html" | "/");
            if ASSET_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 8 {
                let key_class = match key { "/index.html" => "index", "/" => "root", _ => "other" };
                eprintln!("MACOS_NOTIFICATION_FIXTURE_ASSET key_class={key_class} present={present}");
            }
            present.then_some(Cow::Borrowed(HTML.as_bytes()))
        }
        fn iter(&self) -> Box<AssetsIter<'_>> {
            Box::new(std::iter::once((Cow::Borrowed("/index.html"), Cow::Borrowed(HTML.as_bytes()))))
        }
        fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
            Box::new(std::iter::empty())
        }
    }

    #[derive(Clone, Serialize)]
    struct NativeIdentity { pid: u32, bundle_id: String, bundle_path: String }

    fn identity() -> Result<NativeIdentity, String> {
        native_guard(|| {
            let bundle = NSBundle::mainBundle();
            let bundle_id = bundle.bundleIdentifier().map(|value| value.to_string())
                .ok_or("The fixture must run from its real App bundle")?;
            let bundle_path = bundle.bundlePath().to_string();
            if bundle_id != BUNDLE_ID || !PathBuf::from(&bundle_path).is_absolute()
                || !bundle_path.ends_with(".app") {
                return Err("This fixture requires the exact independent notification audit bundle".into());
            }
            Ok(NativeIdentity { pid: std::process::id(), bundle_id, bundle_path })
        })
    }

    fn native_guard<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        match catch_unwind(AssertUnwindSafe(|| objc2::exception::catch(AssertUnwindSafe(work)))) {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Notification fixture native exception; completion is unverified".into()),
            Err(_) => Err("Notification fixture native panic; completion is unverified".into()),
        }
    }

    #[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "lowercase")]
    enum Action { Query, Request, Notify, Delivered, Cleanup }

    #[derive(Clone, Serialize)]
    struct Pending { job: u64, action: Action, trusted_click: bool }

    #[derive(Serialize)]
    struct Receipt {
        job: u64, action: Action, trusted_click: bool, ok: bool,
        raw_result: Value, raw_error: Option<String>,
        native_completion_unverified: bool,
    }

    #[derive(Serialize)]
    struct Report {
        scope: &'static str,
        formal_dev_sender_verified: bool,
        automatic_permission_request: bool,
        automatic_notification_send: bool,
        identity: NativeIdentity,
        final_identity: Option<NativeIdentity>,
        nonce: String,
        title: String,
        body: String,
        pending: Option<Pending>,
        operations: Vec<Receipt>,
        finish_requested: bool,
        run_return_exit_code: Option<i32>,
        component_cleanup_verified: bool,
        cleanup_scope: &'static str,
        notification_cleanup_verified: bool,
        native_system_quit_verified: bool,
        acceptance_requires_separate_gui_review: bool,
    }

    struct Owner { report: Report, file: File, persistence_failed: bool }

    impl Owner {
        fn persist(&mut self) -> Result<(), String> {
            if self.persistence_failed { return Err("The first report persistence failure remains unresolved".into()); }
            let result = (|| -> anyhow::Result<()> {
                let bytes = serde_json::to_vec_pretty(&self.report)?;
                ensure!(bytes.len() < MAX_REPORT_BYTES, "Report exceeded its fixed bound");
                self.file.seek(SeekFrom::Start(0))?;
                self.file.write_all(&bytes)?;
                self.file.write_all(b"\n")?;
                self.file.set_len(bytes.len() as u64 + 1)?;
                self.file.sync_all()?;
                Ok(())
            })();
            if result.is_err() { self.persistence_failed = true; }
            result.map_err(|_| "Could not persist the independent notification fixture report".into())
        }
    }

    struct Shared(Mutex<Owner>);

    fn fixture_document(url: &url::Url) -> bool {
        // Unlike an HTTP URL, Tauri's public Mac base tauri://localhost has
        // an empty path. Tauri deliberately keeps that base for index.html.
        url.scheme() == "tauri" && url.host_str() == Some("localhost")
            && matches!(url.path(), "" | "/" | "/index.html")
            && url.username().is_empty() && url.password().is_none() && url.port().is_none()
            && url.query().is_none() && url.fragment().is_none()
    }

    fn fixture_navigation(url: &url::Url) -> bool {
        let allowed = fixture_document(url);
        if NAVIGATION_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 8 {
            let scheme_class = match url.scheme() { "tauri" => "tauri", "about" => "about", "http" => "http", "https" => "https", _ => "other" };
            let host_class = if url.host_str() == Some("localhost") { "localhost" } else { "other" };
            let path_class = match url.path() { "" => "empty", "/" => "root", "/index.html" => "index", _ => "other" };
            // Never print arbitrary URLs, query values, fragments or data.
            eprintln!("MACOS_NOTIFICATION_FIXTURE_NAV scheme_class={scheme_class} host_class={host_class} path_class={path_class} allowed={allowed}");
        }
        allowed
    }

    fn first_party(window: &WebviewWindow) -> Result<(), String> {
        if window.label() != "main" { return Err("Only the fixture main view may record operations".into()); }
        let url = window.url().map_err(|_| "Fixture main document URL is unavailable")?;
        if !fixture_document(&url) {
            return Err("Only the fixture's embedded first-party document may record operations".into());
        }
        Ok(())
    }

    #[tauri::command]
    fn fixture_info(window: WebviewWindow, state: State<'_, Shared>) -> Result<Value, String> {
        first_party(&window)?;
        let owner = state.0.lock().map_err(|_| "Fixture owner is poisoned")?;
        Ok(json!({ "identity": owner.report.identity, "nonce": owner.report.nonce,
            "title": owner.report.title, "body": owner.report.body }))
    }

    #[tauri::command]
    fn fixture_begin(window: WebviewWindow, state: State<'_, Shared>, action: Action, trusted: bool) -> Result<u64, String> {
        first_party(&window)?;
        if matches!(action, Action::Request | Action::Notify | Action::Cleanup) && !trusted {
            return Err("A trusted user click is required for notification requests, sends and exact test cleanup".into());
        }
        let mut owner = state.0.lock().map_err(|_| "Fixture owner is poisoned")?;
        if owner.report.finish_requested || owner.report.pending.is_some() || owner.persistence_failed {
            return Err("The fixture is retired, busy, or unable to retain evidence".into());
        }
        if owner.report.operations.len() >= MAX_OPERATIONS { return Err("Fixture operation bound reached".into()); }
        let job = owner.report.operations.len() as u64 + 1;
        if action == Action::Notify { owner.report.notification_cleanup_verified = false; }
        owner.report.pending = Some(Pending { job, action, trusted_click: trusted });
        owner.persist()?;
        Ok(job)
    }

    #[tauri::command]
    fn fixture_settle(window: WebviewWindow, state: State<'_, Shared>, job: u64, action: Action,
        ok: bool, result: Value, error: Option<String>) -> Result<(), String> {
        first_party(&window)?;
        if error.as_ref().is_some_and(|error| error.len() > 4096) { return Err("Native error exceeded report bound; original pending retained".into()); }
        if ok == error.is_some() { return Err("A settled result must contain either success or its original error".into()); }
        if ok {
            let valid = match action {
                Action::Query => result.is_null() || result.is_boolean(),
                Action::Request => result.as_str().is_some_and(|value| matches!(value, "granted" | "denied" | "prompt" | "prompt-with-rationale")),
                Action::Notify => result.is_null(),
                Action::Delivered => result.is_array() && result.as_array().is_some_and(|array| array.len() <= 16),
                Action::Cleanup => serde_json::from_value::<CleanupResult>(result.clone()).is_ok(),
            };
            if !valid { return Err("Native IPC result has an unexpected shape; original pending retained".into()); }
        } else if !result.is_null() { return Err("Rejected IPC results must not invent a return value".into()); }
        let mut owner = state.0.lock().map_err(|_| "Fixture owner is poisoned")?;
        let pending = owner.report.pending.as_ref().ok_or("There is no pending operation to settle")?;
        if pending.job != job || pending.action != action || owner.report.finish_requested {
            return Err("This settlement does not own the current operation".into());
        }
        let trusted_click = pending.trusted_click;
        // Settled JavaScript errors do not prove completion of a timed-out or
        // exceptional native callback. Keep that distinction for cleanup.
        let native_completion_unverified = error.as_ref().is_some_and(|error|
            ["unverified", "timed out", "completion was lost", "panicked", "Could not dispatch"]
                .iter().any(|marker| error.contains(marker)));
        owner.report.operations.push(Receipt { job, action, trusted_click, ok,
            raw_result: result.clone(), raw_error: error, native_completion_unverified });
        if action == Action::Cleanup && ok {
            let cleanup: CleanupResult = serde_json::from_value(result).map_err(|_| "Cleanup result shape changed")?;
            owner.report.notification_cleanup_verified = cleanup.current_inventory_clear;
        }
        owner.report.pending = None;
        owner.persist()
    }

    #[derive(Deserialize, Serialize)]
    struct Delivered { identifier: String, title: String, body: String }

    #[derive(Deserialize, Serialize)]
    struct CleanupResult {
        delivered_before: Vec<Delivered>,
        pending_before: Vec<Delivered>,
        removed_delivered_identifiers: Vec<String>,
        removed_pending_identifiers: Vec<String>,
        removal_invocation_is_not_an_acknowledgement: bool,
        delivered_after: Vec<Delivered>,
        pending_after: Vec<Delivered>,
        final_delivered_after: Vec<Delivered>,
        current_inventory_clear: bool,
    }

    type DeliveryReply = Arc<Mutex<Option<oneshot::Sender<Result<Vec<Delivered>, String>>>>>;
    fn complete_delivery(reply: &DeliveryReply, result: Result<Vec<Delivered>, String>) {
        if let Ok(mut reply) = reply.lock() { if let Some(sender) = reply.take() { let _ = sender.send(result); } }
    }

    #[tauri::command]
    async fn fixture_delivered(window: WebviewWindow, state: State<'_, Shared>, job: u64) -> Result<Vec<Delivered>, String> {
        first_party(&window)?;
        let (title, body) = matching_job(&state, job, Action::Delivered)?;
        query_delivered(title, body).await
    }

    fn matching_job(state: &Shared, job: u64, action: Action) -> Result<(String, String), String> {
        {
            let owner = state.0.lock().map_err(|_| "Fixture owner is poisoned")?;
            let pending = owner.report.pending.as_ref().ok_or("Delivered evidence requires its own pending job")?;
            if pending.job != job || pending.action != action || owner.report.finish_requested {
                return Err("Delivered evidence does not own the current job".into());
            }
            Ok((owner.report.title.clone(), owner.report.body.clone()))
        }
    }

    async fn query_delivered(title: String, body: String) -> Result<Vec<Delivered>, String> {
        let (sender, receiver) = oneshot::channel();
        let reply: DeliveryReply = Arc::new(Mutex::new(Some(sender)));
        super::macos_appkit_main_loop::post_to_main(move || {
            let result = native_guard(|| {
                identity()?;
                let callback_reply = reply.clone();
                let completion = RcBlock::new(move |notifications: NonNull<NSArray<UNNotification>>| {
                    let result = native_guard(|| {
                        let notifications = unsafe { notifications.as_ref() };
                        if notifications.len() > 4096 { return Err("Delivered notification inventory exceeded bound".into()); }
                        let mut witnesses = Vec::new();
                        for notification in notifications {
                            let request = notification.request();
                            let content = request.content();
                            let actual_title = content.title().to_string();
                            let actual_body = content.body().to_string();
                            if actual_title == title && actual_body == body {
                                if witnesses.len() == 16 { return Err("Exact nonce matched more than 16 notifications".into()); }
                                witnesses.push(Delivered { identifier: request.identifier().to_string(), title: actual_title, body: actual_body });
                            }
                        }
                        Ok(witnesses)
                    });
                    complete_delivery(&callback_reply, result);
                });
                UNUserNotificationCenter::currentNotificationCenter()
                    .getDeliveredNotificationsWithCompletionHandler(&completion);
                Ok(())
            });
            if let Err(error) = result { complete_delivery(&reply, Err(error)); }
        }).map_err(|_| "Could not dispatch delivered evidence; completion is unverified")?;
        match tokio::time::timeout(Duration::from_secs(15), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Delivered evidence completion was lost; completion is unverified".into()),
            Err(_) => Err("Delivered evidence timed out; completion is unverified".into()),
        }
    }

    async fn query_pending(title: String, body: String) -> Result<Vec<Delivered>, String> {
        let (sender, receiver) = oneshot::channel();
        let reply: DeliveryReply = Arc::new(Mutex::new(Some(sender)));
        super::macos_appkit_main_loop::post_to_main(move || {
            let result = native_guard(|| {
                identity()?;
                let callback_reply = reply.clone();
                let completion = RcBlock::new(move |requests: NonNull<NSArray<UNNotificationRequest>>| {
                    let result = native_guard(|| {
                        let requests = unsafe { requests.as_ref() };
                        if requests.len() > 4096 { return Err("Pending notification inventory exceeded bound".into()); }
                        let mut witnesses = Vec::new();
                        for request in requests {
                            let content = request.content();
                            let actual_title = content.title().to_string();
                            let actual_body = content.body().to_string();
                            if actual_title == title && actual_body == body {
                                if witnesses.len() == 16 { return Err("Exact nonce matched more than 16 pending notifications".into()); }
                                witnesses.push(Delivered { identifier: request.identifier().to_string(), title: actual_title, body: actual_body });
                            }
                        }
                        Ok(witnesses)
                    });
                    complete_delivery(&callback_reply, result);
                });
                UNUserNotificationCenter::currentNotificationCenter()
                    .getPendingNotificationRequestsWithCompletionHandler(&completion);
                Ok(())
            });
            if let Err(error) = result { complete_delivery(&reply, Err(error)); }
        }).map_err(|_| "Could not dispatch pending evidence; completion is unverified")?;
        match tokio::time::timeout(Duration::from_secs(15), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Pending evidence completion was lost; completion is unverified".into()),
            Err(_) => Err("Pending evidence timed out; completion is unverified".into()),
        }
    }

    async fn remove_owned(delivered: Vec<String>, pending: Vec<String>) -> Result<(), String> {
        let (sender, receiver) = oneshot::channel();
        super::macos_appkit_main_loop::post_to_main(move || {
            let result = native_guard(|| {
                identity()?;
                let center = UNUserNotificationCenter::currentNotificationCenter();
                if !delivered.is_empty() {
                    let identifiers = delivered.iter().map(|id| NSString::from_str(id)).collect::<Vec<_>>();
                    center.removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(&identifiers));
                }
                if !pending.is_empty() {
                    let identifiers = pending.iter().map(|id| NSString::from_str(id)).collect::<Vec<_>>();
                    center.removePendingNotificationRequestsWithIdentifiers(&NSArray::from_retained_slice(&identifiers));
                }
                // These public removal methods are void. This only proves an
                // invocation attempt; fresh OS inventory must establish clear.
                Ok(())
            });
            let _ = sender.send(result);
        }).map_err(|_| "Could not dispatch exact notification removal; completion is unverified")?;
        match tokio::time::timeout(Duration::from_secs(15), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Exact removal dispatch completion was lost; completion is unverified".into()),
            Err(_) => Err("Exact removal dispatch timed out; completion is unverified".into()),
        }
    }

    #[tauri::command]
    async fn fixture_cleanup(window: WebviewWindow, state: State<'_, Shared>, job: u64) -> Result<CleanupResult, String> {
        first_party(&window)?;
        let (title, body) = matching_job(&state, job, Action::Cleanup)?;
        let delivered_before = query_delivered(title.clone(), body.clone()).await?;
        let pending_before = query_pending(title.clone(), body.clone()).await?;
        let removed_delivered_identifiers = delivered_before.iter().map(|value| value.identifier.clone()).collect::<Vec<_>>();
        let removed_pending_identifiers = pending_before.iter().map(|value| value.identifier.clone()).collect::<Vec<_>>();
        remove_owned(removed_delivered_identifiers.clone(), removed_pending_identifiers.clone()).await?;
        let mut delivered_after = query_delivered(title.clone(), body.clone()).await?;
        let mut pending_after = query_pending(title.clone(), body.clone()).await?;
        let mut final_delivered_after = query_delivered(title.clone(), body.clone()).await?;
        // Allow public asynchronous removal to settle, without adding another
        // removal request or calling removeAll. Every retry is read-only.
        for _ in 0..8 {
            if delivered_after.is_empty() && pending_after.is_empty() && final_delivered_after.is_empty() { break; }
            tokio::time::sleep(Duration::from_millis(250)).await;
            delivered_after = query_delivered(title.clone(), body.clone()).await?;
            pending_after = query_pending(title.clone(), body.clone()).await?;
            final_delivered_after = query_delivered(title.clone(), body.clone()).await?;
        }
        let current_inventory_clear = delivered_after.is_empty() && pending_after.is_empty() && final_delivered_after.is_empty();
        Ok(CleanupResult { delivered_before, pending_before, removed_delivered_identifiers, removed_pending_identifiers,
            removal_invocation_is_not_an_acknowledgement: true, delivered_after, pending_after, final_delivered_after,
            current_inventory_clear })
    }

    #[tauri::command]
    fn fixture_finish(window: WebviewWindow, app: AppHandle, state: State<'_, Shared>, trusted: bool) -> Result<(), String> {
        first_party(&window)?;
        if !trusted { return Err("Finish requires a trusted user click".into()); }
        {
            let mut owner = state.0.lock().map_err(|_| "Fixture owner is poisoned")?;
            if owner.report.pending.is_some() || owner.report.operations.is_empty() || owner.report.finish_requested {
                return Err("Finish requires all operations settled and no pending job".into());
            }
            owner.report.finish_requested = true;
            owner.persist()?;
        }
        app.exit(0);
        Ok(())
    }

    fn report_file() -> anyhow::Result<File> {
        let mut args = std::env::args_os().skip(1);
        let mut report_path = None;
        while let Some(argument) = args.next() {
            if argument == "--report" && report_path.is_none() { report_path = args.next().map(PathBuf::from); }
            else if argument == super::application_launch::DESCRIPTOR_ARGUMENT || argument == "-ApplePersistenceIgnoreState" {
                args.next().context("Native launch argument is missing its value")?;
            } else { anyhow::bail!("Use --report /ABS/NEW/report.json from the independent notification App"); }
        }
        let path = report_path.context("--report is required")?;
        ensure!(path.is_absolute() && path.file_name().is_some(), "Report requires an absolute new file path");
        let parent = path.parent().context("Report parent is required")?;
        let parent_metadata = std::fs::symlink_metadata(parent)?;
        ensure!(parent_metadata.is_dir() && parent_metadata.uid() == unsafe { libc::geteuid() }
            && parent_metadata.mode() & 0o777 == 0o700, "Report parent must be an owned private 0700 directory");
        let file = OpenOptions::new().write(true).create_new(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file() && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o777 == 0o600, "Report must be a new owned 0600 regular file");
        Ok(file)
    }

    pub fn run() -> anyhow::Result<i32> {
        // Reuse only the existing exact application transport. No product
        // launch guard, identity eligibility policy or scope is broadened.
        let mut bootstrap = super::application_launch::prepare_application(
            super::application_launch::ApplicationScope::DesktopDevelopment)?;
        let native_identity = identity().map_err(anyhow::Error::msg)?;
        let file = report_file()?;
        let nonce = uuid::Uuid::now_v7().to_string();
        let mut owner = Owner {
            report: Report {
                scope: "standalone_component_notification_fixture", formal_dev_sender_verified: false,
                automatic_permission_request: false, automatic_notification_send: false,
                identity: native_identity, final_identity: None,
                title: format!("MAC45 通知验收 {nonce}"), body: format!("独立组件 中文 Ω😀 {nonce}"), nonce,
                pending: None, operations: Vec::new(), finish_requested: false, run_return_exit_code: None,
                component_cleanup_verified: false,
                cleanup_scope: "explicit_fixture_jobs_settled_and_normal_run_return_only",
                notification_cleanup_verified: false, native_system_quit_verified: false,
                acceptance_requires_separate_gui_review: true,
            }, file, persistence_failed: false,
        };
        owner.persist().map_err(anyhow::Error::msg)?;
        let mut context = tauri::generate_context!();
        context.config_mut().identifier = BUNDLE_ID.into();
        context.config_mut().product_name = Some(PRODUCT_NAME.into());
        context.config_mut().build.dev_url = None;
        context.config_mut().app.windows.clear();
        context.set_assets(Box::new(FixtureAssets));
        let app = tauri::Builder::default()
            .manage(Shared(Mutex::new(owner)))
            .plugin(super::native_api_plugins::notification())
            .invoke_handler(tauri::generate_handler![fixture_info, fixture_begin, fixture_settle, fixture_delivered, fixture_cleanup, fixture_finish])
            .setup(|app| {
                let window = tauri::WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("NomiFun 通知组件验收 — Notification Audit")
                    .inner_size(1000.0, 700.0).min_inner_size(880.0, 600.0)
                    .on_navigation(fixture_navigation)
                    .build()?;
                let close_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        // Explicit native Close is a normal abort, not a
                        // successful Finish or a forced process termination.
                        api.prevent_close();
                        close_handle.exit(1);
                    }
                });
                Ok(())
            })
            .build(context)?;
        let handle = app.handle().clone();
        // The explicit Finish and Close handlers use normal app.exit paths.
        // This example does not install the production AppKit termination
        // bridge, so native System Quit reaching run_return is not established.
        // Do not add success-only guards that trap a failed explicit Close.
        let code = app.run_return(|_, _| {});
        let state = handle.state::<Shared>();
        let mut owner = state.0.lock().map_err(|_| anyhow::anyhow!("Fixture owner was poisoned at exit"))?;
        owner.report.final_identity = identity().ok();
        owner.report.run_return_exit_code = Some(code);
        let cleanup = code == 0 && owner.report.finish_requested && owner.report.pending.is_none()
            && !owner.persistence_failed && !owner.report.operations.is_empty()
            && owner.report.operations.iter().all(|receipt| !receipt.native_completion_unverified)
            && owner.report.final_identity.as_ref().is_some_and(|final_identity|
                final_identity.pid == owner.report.identity.pid && final_identity.bundle_id == owner.report.identity.bundle_id
                && final_identity.bundle_path == owner.report.identity.bundle_path);
        owner.report.component_cleanup_verified = cleanup;
        let persisted = owner.persist().is_ok();
        let cleanup = cleanup && persisted;
        drop(owner);
        if let Some(bootstrap) = &mut bootstrap { bootstrap.report_exit(if cleanup { 0 } else { 1 }, cleanup)?; }
        eprintln!("MACOS_NOTIFICATION_GUI_FIXTURE_EXIT component_cleanup_verified={cleanup} formal_dev_sender_verified=false");
        Ok(if cleanup { 0 } else { 1 })
    }
}

#[cfg(target_os = "macos")]
fn main() {
    match fixture::run() {
        Ok(code) => std::process::exit(code),
        Err(error) => { eprintln!("Independent notification fixture failed: {error}"); std::process::exit(1); }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("The independent notification GUI fixture requires macOS and a real App bundle");
    std::process::exit(1);
}
