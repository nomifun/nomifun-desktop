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
            "computer/click",
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
    computer.close_session("none").await.unwrap();
    assert!(!computer.status().running);
    computer.shutdown().await.unwrap();
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
