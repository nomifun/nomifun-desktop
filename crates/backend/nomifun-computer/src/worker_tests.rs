use super::*;
use serde_json::json;
use std::future::Future;

fn fixture() -> (tempfile::TempDir, WorkerConfig) {
    let root = tempfile::tempdir().unwrap();
    let mut config = WorkerConfig::new("bun", root.path());
    config.prefix_args = vec![
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fake-worker.mjs")
            .into_os_string(),
    ];
    config.startup_timeout = Duration::from_secs(5);
    config.shutdown_timeout = Duration::from_millis(80);
    config.call_timeout = Some(Duration::from_secs(2));
    (root, config)
}
fn all() -> BTreeSet<String> {
    surface::manifest()
        .tools
        .iter()
        .map(|tool| tool.action_id.clone())
        .collect()
}
async fn call(
    computer: Arc<Computer>,
    session: &str,
    args: Value,
    control: InvocationControl,
) -> Result<DriverResult, CallError> {
    computer
        .call(
            session,
            session,
            // These are transport/owner tests with a fake worker, not native
            // input tests. The real guide traverses the same dispatch queue
            // without requiring the test runner's macOS Accessibility grant.
            "computer/get_guide",
            args,
            &all(),
            true,
            control,
        )
        .await
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fixture barrier");
}
fn calls(root: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("computer-driver/calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("bounded host cleanup")
}
async fn admitted(
    future: impl Future<Output = Result<DriverResult, CallError>> + Send + 'static,
) -> tokio::task::JoinHandle<Result<DriverResult, CallError>> {
    let (sender, receiver) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut future = Box::pin(future);
        let mut sender = Some(sender);
        std::future::poll_fn(|context| {
            let result = future.as_mut().poll(context);
            if let Some(sender) = sender.take() {
                let _ = sender.send(());
            }
            result
        })
        .await
    });
    receiver.await.unwrap();
    task
}

#[tokio::test]
async fn empty_status_and_close_do_not_start_a_process() {
    let (_root, config) = fixture();
    let computer = Computer::new(config);
    assert!(!computer.status().running);
    assert!(computer.status().backend.is_none());
    assert!(computer.status().atspi.is_none());
    computer.close_session("none").await.unwrap();
    assert!(!computer.status().running);
    computer.shutdown().await.unwrap();
}
#[tokio::test]
async fn native_diagnostics_are_cached_only_for_their_live_generation() {
    let (root, mut config) = fixture();
    config.idle_timeout = Duration::from_millis(40);
    let source = include_str!("../tests/fake-worker.mjs");
    let diagnosed = source.replace(
        "metadata:{driver_version:'fake-1'}",
        "metadata:{driver_version:'fake-1'},diagnostics:{backend:'fixture-native',atspi:false}",
    );
    assert_ne!(diagnosed, source);
    let script = root.path().join("status-worker.mjs");
    std::fs::write(&script, diagnosed).unwrap();
    config.prefix_args = vec![script.clone().into_os_string()];
    let computer = Arc::new(Computer::new(config));
    let first = call(
        computer.clone(),
        "a",
        json!({}),
        InvocationControl::default(),
    )
    .await
    .unwrap();
    assert_eq!(computer.status().backend.as_deref(), Some("fixture-native"));
    assert_eq!(computer.status().atspi, Some(false));
    computer.close_session("a").await.unwrap();
    until(|| !computer.status().running).await;
    assert!(computer.status().backend.is_none());
    assert!(computer.status().atspi.is_none());

    // The next current-protocol worker omits diagnostics. Unknown values must
    // stay unknown rather than inheriting the prior generation's result.
    std::fs::write(&script, source).unwrap();
    let next = call(
        computer.clone(),
        "a",
        json!({}),
        InvocationControl::default(),
    )
    .await
    .unwrap();
    assert_ne!(next.generation, first.generation);
    assert!(computer.status().backend.is_none());
    assert!(computer.status().atspi.is_none());
    bounded(computer.shutdown()).await.unwrap();
}
#[tokio::test]
async fn sessions_rotate_while_preserving_each_sessions_order() {
    let (root, config) = fixture();
    let computer = Arc::new(Computer::new(config));
    let gate = root.path().join("release");
    let first = tokio::spawn(call(
        computer.clone(),
        "a",
        json!({"label":"a1","wait_for_file":gate}),
        InvocationControl::default(),
    ));
    until(|| calls(root.path()).len() == 1).await;
    let a2 = admitted(call(
        computer.clone(),
        "a",
        json!({"label":"a2"}),
        InvocationControl::default(),
    ))
    .await;
    let a3 = admitted(call(
        computer.clone(),
        "a",
        json!({"label":"a3"}),
        InvocationControl::default(),
    ))
    .await;
    let b = admitted(call(
        computer.clone(),
        "b",
        json!({"label":"b1"}),
        InvocationControl::default(),
    ))
    .await;
    std::fs::write(gate, []).unwrap();
    first.await.unwrap().unwrap();
    a2.await.unwrap().unwrap();
    a3.await.unwrap().unwrap();
    b.await.unwrap().unwrap();
    let order: Vec<_> = calls(root.path())
        .iter()
        .map(|call| call["label"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(order, ["a1", "b1", "a2", "a3"]);
    bounded(computer.shutdown()).await.unwrap();
}
#[tokio::test]
async fn queued_stop_is_immediate_and_never_reaches_the_worker() {
    let (root, config) = fixture();
    let computer = Arc::new(Computer::new(config));
    let gate = root.path().join("release");
    let first = tokio::spawn(call(
        computer.clone(),
        "a",
        json!({"label":"active","wait_for_file":gate}),
        InvocationControl::default(),
    ));
    until(|| calls(root.path()).len() == 1).await;
    let cancellation = CancellationToken::new();
    let second = admitted(call(
        computer.clone(),
        "b",
        json!({"label":"withdrawn"}),
        InvocationControl::new(cancellation.clone()),
    ))
    .await;
    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_millis(150), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind, CallErrorKind::Cancelled);
    std::fs::write(gate, []).unwrap();
    first.await.unwrap().unwrap();
    assert_eq!(calls(root.path()).len(), 1);
    bounded(computer.shutdown()).await.unwrap();
}
#[tokio::test]
async fn stop_during_dispatch_keeps_the_real_completed_result() {
    let (root, config) = fixture();
    let computer = Arc::new(Computer::new(config));
    let cancellation = CancellationToken::new();
    let active = tokio::spawn(call(
        computer.clone(),
        "a",
        json!({"label":"active","delay_ms":80}),
        InvocationControl::new(cancellation.clone()),
    ));
    until(|| calls(root.path()).len() == 1).await;
    cancellation.cancel();
    let result = active.await.unwrap().unwrap();
    assert_eq!(result.completion, ActionCompletion::Completed);
    assert_eq!(calls(root.path()).len(), 1);
    bounded(computer.shutdown()).await.unwrap();
}
async fn failed_generation(mode: &str) {
    let (root, mut config) = fixture();
    config.call_timeout = Some(Duration::from_secs(1));
    let computer = Arc::new(Computer::new(config));
    let error = call(
        computer.clone(),
        "a",
        json!({"label":"once","fake_mode":mode}),
        InvocationControl::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, CallErrorKind::Unknown);
    let old = calls(root.path())[0]["generation"]
        .as_str()
        .unwrap()
        .to_owned();
    let next = bounded(call(
        computer.clone(),
        "a",
        json!({"label":"fresh"}),
        InvocationControl::default(),
    ))
    .await
    .unwrap();
    assert_ne!(next.generation, old);
    assert_eq!(calls(root.path()).len(), 2);
    bounded(computer.shutdown()).await.unwrap();
}
#[tokio::test]
async fn crash_is_unknown_and_a_later_call_uses_a_new_generation() {
    failed_generation("crash").await;
}
#[tokio::test]
async fn half_packet_times_out_without_replay() {
    failed_generation("halfpacket").await;
}
#[tokio::test]
async fn hung_call_is_killed_after_its_own_budget() {
    failed_generation("hang").await;
}

#[tokio::test]
async fn app_shutdown_preempts_an_inflight_operations_long_budget() {
    let (root, mut config) = fixture();
    config.call_timeout = Some(Duration::from_secs(30));
    let computer = Arc::new(Computer::new(config));
    let active = tokio::spawn(call(
        computer.clone(),
        "a",
        json!({"label":"hang","fake_mode":"hang"}),
        InvocationControl::default(),
    ));
    until(|| calls(root.path()).len() == 1).await;
    let pending = admitted(call(
        computer.clone(),
        "b",
        json!({"label":"never"}),
        InvocationControl::default(),
    ))
    .await;
    tokio::time::timeout(Duration::from_secs(2), computer.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        active.await.unwrap().unwrap_err().kind,
        CallErrorKind::Unknown
    );
    assert_eq!(
        pending.await.unwrap().unwrap_err().kind,
        CallErrorKind::Cancelled
    );
    assert_eq!(calls(root.path()).len(), 1);
    assert!(!computer.status().running);
}
#[tokio::test]
async fn closing_the_last_binding_allows_automatic_idle_retirement() {
    let (_root, mut config) = fixture();
    config.idle_timeout = Duration::from_millis(40);
    let computer = Arc::new(Computer::new(config));
    let first = call(
        computer.clone(),
        "a",
        json!({}),
        InvocationControl::default(),
    )
    .await
    .unwrap();
    computer.close_session("a").await.unwrap();
    until(|| !computer.status().running).await;
    let next = call(
        computer.clone(),
        "a",
        json!({}),
        InvocationControl::default(),
    )
    .await
    .unwrap();
    assert_ne!(next.generation, first.generation);
    bounded(computer.shutdown()).await.unwrap();
}
#[tokio::test]
async fn direct_child_exit_does_not_prove_the_native_stderr_lifetime_ended() {
    let (_root, config) = fixture();
    let (_tx, rx) = mpsc::unbounded_channel();
    let (status, _) = watch::channel(WorkerStatus::default());
    let mut actor = Actor::new(config, rx, status, CancellationToken::new());
    actor.start().await.unwrap();
    let actual_stderr = actor.live.as_mut().unwrap().stderr.take().unwrap();
    let barrier = Arc::new(tokio::sync::Notify::new());
    let waiting = barrier.clone();
    actor.live.as_mut().unwrap().stderr = Some(tokio::spawn(async move {
        waiting.notified().await;
        Ok(())
    }));
    let error = actor.retire().await.unwrap_err();
    assert!(error.message.contains("stderr"));
    assert!(actor.live.is_some());
    assert!(actor.cleanup_pending);
    barrier.notify_one();
    bounded(actor.retire()).await.unwrap();
    assert!(actor.live.is_none());
    actual_stderr.await.unwrap().unwrap();
}

#[tokio::test]
async fn expired_driver_binding_is_replaced_without_replaying_an_action() {
    let (root, config) = fixture();
    let (_tx, rx) = mpsc::unbounded_channel();
    let (status, _) = watch::channel(WorkerStatus::default());
    let mut actor = Actor::new(config, rx, status, CancellationToken::new());
    actor.start().await.unwrap();
    let first = actor.bind("session", "Agent").await.unwrap();
    actor.sessions.get_mut("session").unwrap().last_used =
        Instant::now() - Duration::from_secs(3601);
    let second = actor.bind("session", "Agent").await.unwrap();
    assert_ne!(first, second);
    assert!(calls(root.path()).is_empty());
    bounded(actor.retire()).await.unwrap();
}

fn screen_capture_requests(root: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("computer-driver/screen-capture.jsonl"))
        .unwrap_or_default().lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

fn enqueue_screen_capture(computer: &Computer) -> oneshot::Receiver<Result<(), CallError>> {
    let (reply, receiver) = oneshot::channel();
    computer.sender.send(Command::RequestScreenCapture(reply)).unwrap();
    receiver
}

fn change_screen_capture_fixture(root: &std::path::Path, config: &mut WorkerConfig, response: &str) {
    let source = include_str!("../tests/fake-worker.mjs");
    let changed = source.replace("respond(request,{status:'ready'});continue;", response);
    assert_ne!(changed, source);
    let script = root.join("screen-capture-worker.mjs");
    std::fs::write(&script, changed).unwrap();
    config.prefix_args = vec![script.into_os_string()];
}

#[tokio::test]
async fn screen_capture_denied_does_not_start_worker() {
    let (root, mut config) = fixture();
    config.screen_capture_granted = Some(false);
    let computer = Computer::new(config);
    let error = bounded(enqueue_screen_capture(&computer)).await.unwrap().unwrap_err();
    assert_eq!(error.code, "SYSTEM_PERMISSION_REQUIRED");
    assert_eq!(error.kind, CallErrorKind::BeforeDispatch);
    assert!(!root.path().join("computer-driver").exists());
    assert!(!computer.status().running);
    bounded(computer.shutdown()).await.unwrap();
}

#[tokio::test]
async fn closed_queued_screen_capture_observer_never_dispatches() {
    let (root, mut config) = fixture();
    config.screen_capture_granted = Some(true);
    let computer = Arc::new(Computer::new(config));
    let gate = root.path().join("release");
    let active = tokio::spawn(call(computer.clone(), "a", json!({"wait_for_file":gate}), InvocationControl::default()));
    until(|| calls(root.path()).len() == 1).await;
    drop(enqueue_screen_capture(&computer));
    std::fs::write(gate, []).unwrap();
    let before = active.await.unwrap().unwrap();
    let after = call(computer.clone(), "a", json!({}), InvocationControl::default()).await.unwrap();
    assert_eq!(before.generation, after.generation);
    assert!(screen_capture_requests(root.path()).is_empty());
    bounded(computer.shutdown()).await.unwrap();
}

#[tokio::test]
async fn explicit_screen_capture_retires_generation_and_old_session_binding() {
    let (root, mut config) = fixture();
    config.screen_capture_granted = Some(true);
    let computer = Arc::new(Computer::new(config));
    let before = call(computer.clone(), "a", json!({}), InvocationControl::default()).await.unwrap();
    bounded(enqueue_screen_capture(&computer)).await.unwrap().unwrap();
    assert!(!computer.status().running);
    let probes = screen_capture_requests(root.path());
    assert_eq!(probes.len(), 1);
    assert_eq!(probes[0]["generation"], before.generation);
    assert!(probes[0]["session"].is_null());
    let after = call(computer.clone(), "a", json!({}), InvocationControl::default()).await.unwrap();
    assert_ne!(before.generation, after.generation);
    assert_eq!(calls(root.path()).len(), 2);
    bounded(computer.shutdown()).await.unwrap();
}

#[tokio::test]
async fn failed_or_malformed_screen_capture_is_an_error_and_retires_exact_owner() {
    for response in [
        "respond(request,{status:'unavailable'});continue;",
        "respond(request,{status:'ready',unexpected:'not status metadata'});continue;",
        "respond(request,true);continue;",
    ] {
        let (root, mut config) = fixture();
        config.screen_capture_granted = Some(true);
        change_screen_capture_fixture(root.path(), &mut config, response);
        let computer = Computer::new(config);
        let error = bounded(enqueue_screen_capture(&computer)).await.unwrap().unwrap_err();
        assert_eq!(error.code, "COMPUTER_SCREEN_CAPTURE_REQUEST_FAILED");
        assert_eq!(screen_capture_requests(root.path()).len(), 1);
        assert!(!computer.status().running);
        bounded(computer.shutdown()).await.unwrap();
    }
}

#[tokio::test]
async fn dropped_inflight_screen_capture_observer_keeps_owner_until_result_and_retirement() {
    let (root, mut config) = fixture();
    config.screen_capture_granted = Some(true);
    let gate = root.path().join("release");
    change_screen_capture_fixture(root.path(), &mut config, &format!(
        "while(!existsSync({}))await sleep(5);respond(request,{{status:'ready'}});continue;", serde_json::to_string(&gate).unwrap()));
    let computer = Arc::new(Computer::new(config));
    let receiver = enqueue_screen_capture(&computer);
    until(|| screen_capture_requests(root.path()).len() == 1).await;
    let generation = screen_capture_requests(root.path())[0]["generation"].as_str().unwrap().to_owned();
    drop(receiver);
    assert!(computer.status().running);
    let next = tokio::spawn(call(computer.clone(), "a", json!({}), InvocationControl::default()));
    std::fs::write(gate, []).unwrap();
    assert_ne!(bounded(next).await.unwrap().unwrap().generation, generation);
    assert_eq!(screen_capture_requests(root.path()).len(), 1);
    bounded(computer.shutdown()).await.unwrap();
}

#[tokio::test]
async fn screen_capture_native_deadline_retires_without_retry() {
    let (root, mut config) = fixture();
    config.screen_capture_granted = Some(true);
    config.screen_capture_timeout = Some(Duration::from_millis(40));
    change_screen_capture_fixture(root.path(), &mut config, "await new Promise(()=>setInterval(()=>{},1000));");
    let computer = Computer::new(config);
    let error = bounded(enqueue_screen_capture(&computer)).await.unwrap().unwrap_err();
    assert!(error.message.contains("native probe deadline"));
    assert_eq!(screen_capture_requests(root.path()).len(), 1);
    assert!(!computer.status().running);
    bounded(computer.shutdown()).await.unwrap();
}

#[tokio::test]
async fn app_shutdown_preempts_screen_capture_native_deadline() {
    let (root, mut config) = fixture();
    config.screen_capture_granted = Some(true);
    change_screen_capture_fixture(root.path(), &mut config, "await new Promise(()=>setInterval(()=>{},1000));");
    let computer = Computer::new(config);
    let receiver = enqueue_screen_capture(&computer);
    until(|| screen_capture_requests(root.path()).len() == 1).await;
    tokio::time::timeout(Duration::from_secs(2), computer.shutdown()).await.unwrap().unwrap();
    assert!(receiver.await.unwrap().is_err());
    assert_eq!(screen_capture_requests(root.path()).len(), 1);
    assert!(!computer.status().running);
}

#[tokio::test]
async fn worker_timed_out_metadata_reaps_the_still_blocked_native_owner() {
    let (root, mut config) = fixture();
    config.screen_capture_granted = Some(true);
    change_screen_capture_fixture(root.path(), &mut config,
        "respond(request,{status:'timed_out'});await new Promise(()=>setInterval(()=>{},1000));");
    let computer = Computer::new(config);
    let error = tokio::time::timeout(Duration::from_secs(2), enqueue_screen_capture(&computer)).await.unwrap().unwrap().unwrap_err();
    assert!(error.message.contains("timed out"));
    assert_eq!(screen_capture_requests(root.path()).len(), 1);
    assert!(!computer.status().running);
    bounded(computer.shutdown()).await.unwrap();
}

#[tokio::test]
async fn shutdown_during_probe_never_reports_a_late_ready_as_success() {
    let (root, mut config) = fixture();
    config.screen_capture_granted = Some(true);
    let gate = root.path().join("release");
    change_screen_capture_fixture(root.path(), &mut config, &format!(
        "while(!existsSync({}))await sleep(5);respond(request,{{status:'ready'}});continue;", serde_json::to_string(&gate).unwrap()));
    let computer = Arc::new(Computer::new(config));
    let receiver = enqueue_screen_capture(&computer);
    until(|| screen_capture_requests(root.path()).len() == 1).await;
    let quitting = computer.clone();
    let shutdown = tokio::spawn(async move { quitting.shutdown().await });
    until(|| computer.shutdown_requested.is_cancelled()).await;
    std::fs::write(gate, []).unwrap();
    let error = bounded(receiver).await.unwrap().unwrap_err();
    assert_eq!(error.code, "COMPUTER_SCREEN_CAPTURE_REQUEST_CANCELLED");
    assert_eq!(error.kind, CallErrorKind::Cancelled);
    bounded(shutdown).await.unwrap().unwrap();
    assert!(!computer.status().running);
    assert_eq!(screen_capture_requests(root.path()).len(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn user_launch_context_preserves_only_original_user_paths() {
    use std::os::unix::ffi::OsStringExt;
    let home = OsString::from_vec(b"/tmp/user-\xff".to_vec());
    let cwd = PathBuf::from(OsString::from_vec(b"/tmp/work-\xfe".to_vec()));
    let context: Value = serde_json::from_str(&linux_user_launch_context(
        [
            (OsString::from("HOME"), home),
            (OsString::from("XDG_DATA_DIRS"), OsString::from("/opt/apps:/usr/share")),
            (OsString::from("PROVIDER_API_KEY"), OsString::from("not-forwarded")),
            (OsString::from("CUA_DRIVER_RS_HOME"), OsString::from("/private-driver")),
        ],
        cwd,
    )).unwrap();
    assert_eq!(context["cwd"], serde_json::json!(b"/tmp/work-\xfe".to_vec()));
    assert_eq!(context["environment"]["HOME"], serde_json::json!(b"/tmp/user-\xff".to_vec()));
    assert_eq!(context["environment"]["XDG_DATA_DIRS"], serde_json::json!(b"/opt/apps:/usr/share".to_vec()));
    assert!(context["environment"]["TMPDIR"].is_null());
    assert_eq!(context["environment"].as_object().unwrap().len(), 12);
    assert!(context["environment"].get("PROVIDER_API_KEY").is_none());
    assert!(context["environment"].get("CUA_DRIVER_RS_HOME").is_none());
}
