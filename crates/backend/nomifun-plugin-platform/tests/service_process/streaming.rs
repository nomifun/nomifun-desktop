//! Real Node incremental delivery on the original invocation lane. These are
//! process-contract tests, not evidence that Nomi model selection is connected.
use super::*;
use nomifun_plugin_platform::runtime::{
    InMemoryPluginRuntimeServiceHost, PluginRuntimePlatformError, PluginRuntimeServiceHostPort,
    PluginRuntimeServiceProcess,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const STREAM_SERVICE: &str = r#"
export async function start(context) {
  let started = 0, acknowledged = 0, aborted = 0;
  return {
    async invoke({ method, payload, signal, emit }) {
      if (method === 'stats') return { started, acknowledged, aborted, unary: emit === undefined };
      started++;
      if (method === 'corrupt') {
        const original = process.stdout.write;
        process.stdout.write = function(line, ...args) {
          const frame = JSON.parse(line);
          if (frame.kind === 'event') {
            process.stdout.write = original;
            if (payload.field === 'sequence') frame.sequence += 1;
            if (payload.field === 'call_id') frame.call_id = 'wrong-call';
            if (payload.field === 'request_id') frame.request_id = 'unknown-request';
            if (payload.field === 'host_generation') frame.host_generation += 1;
            return original.call(this, JSON.stringify(frame) + '\n', ...args);
          }
          return original.call(this, line, ...args);
        };
      }
      try {
        if (method === 'unserializable') {
          const value = {};
          value.self = value;
          await emit(value);
        }
        for (let i = 0; i < payload.count; i++) {
          await emit(method === 'oversized' ? 'x'.repeat(1024 * 1024 + 64) : { index: i, text: `chunk-${i}` });
          acknowledged++;
        }
        if (method === 'fail') throw new Error('failure after partial output');
        return payload.nullResult ? null : { completed: payload.count };
      } catch (error) {
        if (signal.aborted) aborted++;
        throw error;
      }
    },
    async dispose() {},
  };
}
"#;

struct Fixture {
    _directory: TempDir,
    process: Arc<dyn PluginRuntimeServiceProcess>,
    spec: ResolvedPluginServiceSpec,
}

impl Fixture {
    async fn new(timeout: Duration) -> Self {
        // This suite promises actual Node evidence; absence is an explicit
        // failure, not a passing no-op. Existing platform tests keep their policy.
        let node = node_executable().expect("Node is required for Service streaming tests");
        let directory = TempDir::new().unwrap();
        let module = write_module(&directory, "stream.mjs", STREAM_SERVICE);
        let spec = service_spec(
            &node, STREAM_SERVICE.as_bytes(), "stream-process", 1,
            PluginServiceLifecycle::OnDemand,
        );
        let process = factory(&node, &module, timeout)
            .start(PluginRuntimeServiceLaunch { spec: spec.clone(), host_generation: 1 })
            .await.unwrap();
        Self { _directory: directory, process, spec }
    }

    fn call(
        &self,
        method: &str,
        payload: serde_json::Value,
        cancellation: PluginRuntimeCallCancellation,
    ) -> (
        mpsc::Receiver<StrictJsonValue>,
        JoinHandle<Result<StrictJsonValue, PluginRuntimeServiceProcessError>>,
    ) {
        let (events, receiver) = mpsc::channel(1);
        let invocation = PluginRuntimeServiceInvocation {
            fence: fence(&self.spec, 1),
            call_id: Uuid::now_v7().to_string().into(),
            method: method.into(),
            payload: StrictJsonValue(payload),
            events: Some(events),
        };
        let process = self.process.clone();
        (receiver, tokio::spawn(async move { process.invoke(invocation, cancellation).await }))
    }

    async fn stats(&self) -> serde_json::Value {
        invoke(&self.process, &self.spec, 1, "stats", "stats", json!({}),
            PluginRuntimeCallCancellation::default()).await.unwrap().0
    }

    async fn wait_stats(&self, started: u64, acknowledged: u64, aborted: u64) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if self.stats().await == json!({"started": started, "acknowledged": acknowledged,
                    "aborted": aborted, "unary": true}) { break; }
                tokio::task::yield_now().await;
            }
        }).await.expect("stream counters did not settle");
    }
}

async fn next(events: &mut mpsc::Receiver<StrictJsonValue>) -> StrictJsonValue {
    tokio::time::timeout(Duration::from_secs(2), events.recv()).await
        .expect("event delivery timed out").expect("event stream ended early")
}

#[tokio::test]
async fn bounded_stream_delivers_before_completion_without_blocking_unary_calls() {
    let fixture = Fixture::new(Duration::from_secs(5)).await;
    let token = PluginRuntimeCallCancellation::default();
    let (mut events, task) = fixture.call("stream", json!({"count": 4}), token.clone());
    // Capacity one: after the first ACK the next event waits in the Actor;
    // a third cannot be emitted. A simultaneous unary call still completes.
    fixture.wait_stats(1, 1, 0).await;
    assert!(!task.is_finished(), "stream completed before its consumer drained it");
    for index in 0..4 {
        assert_eq!(next(&mut events).await.0, json!({"index": index, "text": format!("chunk-{index}")}));
    }
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), task).await.unwrap()
        .unwrap().unwrap().0, json!({"completed": 4}));
    assert!(events.recv().await.is_none());
    assert!(!token.is_canceled());
    fixture.wait_stats(1, 4, 0).await;
    fixture.process.stop().await;
}

#[tokio::test]
async fn stream_consumer_drop_explicit_cancel_and_future_drop_retire_only_their_request() {
    let fixture = Fixture::new(Duration::from_secs(5)).await;
    for (index, mode) in ["receiver", "token", "future"].into_iter().enumerate() {
        let token = PluginRuntimeCallCancellation::default();
        let (events, task) = fixture.call("stream", json!({"count": 100}), token.clone());
        fixture.wait_stats(index as u64 + 1, index as u64 + 1, index as u64).await;
        match mode {
            "receiver" => { drop(events); }
            "token" => { token.cancel(); }
            "future" => { task.abort(); }
            _ => unreachable!(),
        }
        let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap();
        if mode == "future" { assert!(result.unwrap_err().is_cancelled()); }
        else { assert!(matches!(result.unwrap(), Err(PluginRuntimeServiceProcessError::Rejected(_)))); }
        fixture.wait_stats(index as u64 + 1, index as u64 + 1, index as u64 + 1).await;
        assert!(token.is_canceled());
        assert!(fixture.process.terminal_result().is_none(), "cancellation killed the Service");
    }
    fixture.process.stop().await;
}

#[tokio::test]
async fn receiver_closed_before_admission_never_invokes_plugin() {
    let fixture = Fixture::new(Duration::from_secs(5)).await;
    let (events, receiver) = mpsc::channel(1);
    drop(receiver);
    let result = fixture.process.invoke(PluginRuntimeServiceInvocation {
        fence: fence(&fixture.spec, 1), call_id: "closed".into(), method: "stream".into(),
        payload: StrictJsonValue(json!({"count": 1})), events: Some(events),
    }, PluginRuntimeCallCancellation::default()).await;
    assert!(matches!(result, Err(PluginRuntimeServiceProcessError::Rejected(_))));
    fixture.wait_stats(0, 0, 0).await;
    fixture.process.stop().await;
}

#[tokio::test]
async fn partial_events_do_not_turn_a_final_failure_into_success() {
    let fixture = Fixture::new(Duration::from_secs(5)).await;
    let (mut events, task) = fixture.call("fail", json!({"count": 1}), PluginRuntimeCallCancellation::default());
    assert_eq!(next(&mut events).await.0["index"], 0);
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
    assert!(matches!(result, Err(PluginRuntimeServiceProcessError::Rejected(message))
        if message.contains("failure after partial output")));
    assert!(events.recv().await.is_none());
    assert!(fixture.process.terminal_result().is_none());
    let (mut events, task) = fixture.call(
        "stream", json!({"count": 1, "nullResult": true}),
        PluginRuntimeCallCancellation::default(),
    );
    assert_eq!(next(&mut events).await.0["index"], 0);
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert!(result.0.is_null(), "explicit null terminal result is not a missing response field");
    assert!(events.recv().await.is_none());
    fixture.process.stop().await;
}

#[tokio::test]
async fn stalled_consumer_does_not_extend_original_request_deadline() {
    let fixture = Fixture::new(Duration::from_millis(500)).await;
    let (_events, task) = fixture.call("stream", json!({"count": 100}), PluginRuntimeCallCancellation::default());
    let result = tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
    assert!(matches!(result, Err(PluginRuntimeServiceProcessError::Crashed(message))
        if message.contains("watchdog")));
    fixture.process.stop().await;
}

#[tokio::test]
async fn event_identity_and_sequence_corruption_fail_the_original_generation() {
    for field in ["sequence", "call_id", "request_id", "host_generation"] {
        let fixture = Fixture::new(Duration::from_secs(5)).await;
        let (mut events, task) = fixture.call("corrupt", json!({"count": 1, "field": field}),
            PluginRuntimeCallCancellation::default());
        let result = tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
        assert!(matches!(result, Err(PluginRuntimeServiceProcessError::Crashed(_))), "{field}: {result:?}");
        assert!(events.recv().await.is_none(), "corrupt {field} event was delivered");
        fixture.process.stop().await;
    }
}

#[tokio::test]
async fn incremental_event_uses_existing_frame_byte_limit() {
    let fixture = Fixture::new(Duration::from_secs(5)).await;
    let (mut events, task) = fixture.call("oversized", json!({"count": 1}), PluginRuntimeCallCancellation::default());
    let result = tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
    assert!(matches!(result, Err(PluginRuntimeServiceProcessError::Crashed(_))));
    assert!(events.recv().await.is_none());
    fixture.process.stop().await;
}

#[tokio::test]
async fn unserializable_event_rejects_call_without_waiting_for_watchdog() {
    let fixture = Fixture::new(Duration::from_secs(5)).await;
    let (mut events, task) = fixture.call("unserializable", json!({"count": 0}),
        PluginRuntimeCallCancellation::default());
    let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
    assert!(matches!(result, Err(PluginRuntimeServiceProcessError::Rejected(_))));
    assert!(events.recv().await.is_none());
    assert!(fixture.process.terminal_result().is_none());
    fixture.process.stop().await;
}

#[tokio::test]
async fn host_stream_uses_same_registration_and_release_fence_as_unary_calls() {
    let node = node_executable().expect("Node is required for Service streaming tests");
    let directory = TempDir::new().unwrap();
    let module = write_module(&directory, "stream.mjs", STREAM_SERVICE);
    let spec = service_spec(&node, STREAM_SERVICE.as_bytes(), "host-stream", 1,
        PluginServiceLifecycle::OnDemand);
    let host = Arc::new(InMemoryPluginRuntimeServiceHost::new(Arc::new(factory(
        &node, &module, Duration::from_secs(5),
    ))));
    host.bind_active(spec.clone(), true).await.unwrap();
    // This case measures stream registration and release fencing, not cold
    // startup (including hashing the Node binary). Finish startup before the
    // two-second event assertion, keeping both protocol deadlines unchanged.
    let initial = host.invoke(&spec, "warmup".into(), "stats".into(),
        StrictJsonValue(json!({})), PluginRuntimeCallCancellation::default(), 1).await.unwrap();
    assert_eq!(initial.0["started"], 0);
    let (events, mut receiver) = mpsc::channel(1);
    let task = {
        let (host, spec) = (host.clone(), spec.clone());
        tokio::spawn(async move {
            host.invoke_with_events(&spec, "stream".into(), "stream".into(),
                StrictJsonValue(json!({"count": 100})), PluginRuntimeCallCancellation::default(),
                1, Some(events)).await
        })
    };
    assert_eq!(next(&mut receiver).await.0["index"], 0);
    assert!(matches!(host.invoke(&spec, "stream".into(), "stats".into(),
        StrictJsonValue(json!({})), PluginRuntimeCallCancellation::default(), 1).await,
        Err(PluginRuntimePlatformError::DuplicateBridgeCall(_))));
    // A changed release retires the same generation and all its stream calls.
    let replacement = service_spec(&node, STREAM_SERVICE.as_bytes(), "host-stream", 2,
        PluginServiceLifecycle::OnDemand);
    host.bind_active(replacement.clone(), true).await.unwrap();
    assert!(matches!(tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap(),
        Err(PluginRuntimePlatformError::StaleServiceGeneration)));
    while receiver.recv().await.is_some() {}
    assert!(matches!(host.invoke(&spec, "old".into(), "stats".into(),
        StrictJsonValue(json!({})), PluginRuntimeCallCancellation::default(), 2).await,
        Err(PluginRuntimePlatformError::StaleServiceGeneration)));
    let result = host.invoke(&replacement, "new".into(), "stats".into(),
        StrictJsonValue(json!({})), PluginRuntimeCallCancellation::default(), 2).await.unwrap();
    assert_eq!(result.0["started"], 0, "new generation replayed the old stream");
    host.stop(&replacement.plugin_product_id).await.unwrap();
}
