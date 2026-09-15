use super::*;
use crate::tool_middleware::BeforeToolInput;
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use serde_json::{Value, json};
use std::sync::Mutex;

type Trace = Arc<Mutex<Vec<String>>>;
struct Probe {
    name: &'static str,
    trace: Trace,
    admission: bool,
    timeout: Duration,
    concurrent: bool,
}
#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "hook admission probe"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object", "properties":{"quantity":{"type":"integer"},"password":{"type":"string"}},"required":["quantity"],"additionalProperties":false})
    }
    fn is_concurrency_safe(&self, _: &Value) -> bool {
        self.concurrent
    }
    fn execution_timeout(&self, _: &Value) -> Duration {
        self.timeout
    }
    async fn preflight_hook(&self, _: &Value, _: &ToolExecutionContext) -> Result<(), String> {
        self.trace
            .lock()
            .unwrap()
            .push(format!("preflight:{}", self.name));
        if self.admission {
            Ok(())
        } else {
            Err("private rejection detail".into())
        }
    }
    async fn execute(&self, input: Value) -> ToolResult {
        self.trace.lock().unwrap().push(format!(
            "execute:{}:{}",
            self.name,
            input["password"].as_str().unwrap_or("")
        ));
        ToolResult::text(format!("{} completed", self.name))
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}
struct Gate {
    trace: Trace,
    mode: &'static str,
}
#[async_trait::async_trait]
impl ToolCallMiddleware for Gate {
    async fn before_tool(&self, input: BeforeToolInput) -> Result<BeforeToolDecision, String> {
        self.trace
            .lock()
            .unwrap()
            .push(format!("gate:{}", input.tool_name));
        assert!(input.invocation_id.starts_with("tool-call-v1-"));
        assert_eq!(input.arguments["quantity"], 3);
        if input.arguments.get("password").is_some() {
            assert!(input.redacted);
            assert_eq!(input.arguments["password"], "[REDACTED]");
        }
        match self.mode {
            "deny" => Ok(BeforeToolDecision::Deny {
                reason: "quantity rejected".into(),
            }),
            "error" => Err("private service body".into()),
            "fail_b" if input.tool_name == "B" => Err("failed B".into()),
            "panic" => panic!("private panic body"),
            "timeout" => {
                std::future::pending::<()>().await;
                unreachable!()
            }
            _ => Ok(BeforeToolDecision::Allow {}),
        }
    }
    fn label(&self) -> &str {
        "probe"
    }
}
#[derive(Default)]
struct Emitter;
impl ProtocolEmitter for Emitter {
    fn emit(&self, _: &ProtocolEvent) -> std::io::Result<()> {
        Ok(())
    }
}
fn call(name: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: format!("call-{name}"),
        name: name.into(),
        input: json!({"quantity":3,"password":"real-secret"}),
        extra: None,
    }
}
async fn batch(
    registry: &ToolRegistry,
    calls: &[ContentBlock],
    middleware: &[Arc<dyn ToolCallMiddleware>],
    protocol: bool,
) -> ToolCallOutcome {
    let authority = ProviderToolAuthority::from_request_tools(&registry.to_tool_defs());
    if protocol {
        let writer: Arc<dyn ProtocolEmitter> = Arc::new(Emitter);
        execute_tool_calls_with_protocol(
            registry,
            calls,
            &authority,
            &writer,
            "turn",
            None,
            nomi_compact::CompactionLevel::Off,
            false,
            middleware,
        )
        .await
        .unwrap()
    } else {
        execute_tool_calls_scoped(
            registry,
            calls,
            &authority,
            "turn",
            None,
            nomi_compact::CompactionLevel::Off,
            false,
            middleware,
        )
        .await
        .unwrap()
    }
}
fn registry(
    trace: &Trace,
    names: &[&'static str],
    admission: bool,
    timeout: Duration,
    concurrent: bool,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    for &name in names {
        registry.register(Box::new(Probe {
            name,
            trace: trace.clone(),
            admission,
            timeout,
            concurrent,
        }));
    }
    registry
}
#[tokio::test]
async fn before_tool_both_paths_gate_canonical_arguments_and_keep_original_dispatch_input() {
    for protocol in [false, true] {
        let trace = Trace::default();
        let registry = registry(&trace, &["A"], true, Duration::from_secs(1), false);
        let gates: Vec<Arc<dyn ToolCallMiddleware>> = vec![Arc::new(Gate {
            trace: trace.clone(),
            mode: "allow",
        })];
        let result = batch(&registry, &[call("A")], &gates, protocol).await;
        assert!(!block_is_error(&result.results[0]));
        assert!(result.fatal_hook_error.is_none());
        assert_eq!(
            *trace.lock().unwrap(),
            ["preflight:A", "gate:A", "execute:A:real-secret"]
        );
    }
}
#[tokio::test]
async fn before_tool_deny_and_technical_failure_never_dispatch_target() {
    for protocol in [false, true] {
        for mode in ["deny", "error", "timeout", "panic"] {
            let trace = Trace::default();
            let registry = registry(&trace, &["A", "B"], true, Duration::from_millis(20), false);
            let gates: Vec<Arc<dyn ToolCallMiddleware>> = vec![Arc::new(Gate {
                trace: trace.clone(),
                mode,
            })];
            let result = batch(&registry, &[call("A"), call("B")], &gates, protocol).await;
            assert!(result.results.iter().all(block_is_error));
            assert_eq!(result.fatal_hook_error.is_some(), mode != "deny");
            assert!(
                !trace
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|entry| entry.starts_with("execute:"))
            );
            assert!(!format!("{:?}", result.results).contains("private"));
        }
    }
}
#[tokio::test]
async fn before_tool_preflight_rejection_precedes_hooks_and_absent_hooks_preserve_behavior() {
    let trace = Trace::default();
    let registry = registry(&trace, &["A"], false, Duration::from_secs(1), false);
    let gates: Vec<Arc<dyn ToolCallMiddleware>> = vec![Arc::new(Gate {
        trace: trace.clone(),
        mode: "allow",
    })];
    let result = batch(&registry, &[call("A")], &gates, false).await;
    assert!(result.fatal_hook_error.is_some());
    assert_eq!(*trace.lock().unwrap(), ["preflight:A"]);
    trace.lock().unwrap().clear();
    let result = batch(&registry, &[call("A")], &[], false).await;
    assert!(!block_is_error(&result.results[0]));
    assert_eq!(*trace.lock().unwrap(), ["execute:A:real-secret"]);
}
#[tokio::test]
async fn before_tool_schema_failure_precedes_preflight_and_hook() {
    let trace = Trace::default();
    let registry = registry(&trace, &["A"], true, Duration::from_secs(1), false);
    let gates: Vec<Arc<dyn ToolCallMiddleware>> = vec![Arc::new(Gate {
        trace: trace.clone(),
        mode: "allow",
    })];
    let invalid = ContentBlock::ToolUse {
        id: "bad".into(),
        name: "A".into(),
        input: json!({"quantity":false}),
        extra: None,
    };
    let result = batch(&registry, &[invalid], &gates, false).await;
    assert!(block_is_error(&result.results[0]));
    assert!(trace.lock().unwrap().is_empty());
}
#[tokio::test]
async fn before_tool_fatal_preserves_prior_completed_result_and_skips_later_tools() {
    for protocol in [false, true] {
        let trace = Trace::default();
        let registry = registry(
            &trace,
            &["A", "B", "C"],
            true,
            Duration::from_secs(1),
            false,
        );
        let gates: Vec<Arc<dyn ToolCallMiddleware>> = vec![Arc::new(Gate {
            trace: trace.clone(),
            mode: "fail_b",
        })];
        let result = batch(
            &registry,
            &[call("A"), call("B"), call("C")],
            &gates,
            protocol,
        )
        .await;
        assert!(result.fatal_hook_error.is_some());
        assert!(!block_is_error(&result.results[0]));
        assert_eq!(
            result.hook_not_dispatched_call_ids,
            BTreeSet::from(["call-B".to_owned(), "call-C".to_owned()])
        );
        assert_eq!(
            *trace.lock().unwrap(),
            [
                "preflight:A",
                "gate:A",
                "execute:A:real-secret",
                "preflight:B",
                "gate:B"
            ]
        );
    }
}
#[tokio::test]
async fn before_tool_concurrent_fatal_closes_remaining_dispatch() {
    let trace = Trace::default();
    let registry = registry(&trace, &["A", "B"], true, Duration::from_secs(1), true);
    let gates: Vec<Arc<dyn ToolCallMiddleware>> = vec![Arc::new(Gate {
        trace: trace.clone(),
        mode: "error",
    })];
    let result = batch(&registry, &[call("A"), call("B")], &gates, true).await;
    assert!(result.fatal_hook_error.is_some());
    assert_eq!(*trace.lock().unwrap(), ["preflight:A", "gate:A"]);
}

#[tokio::test]
async fn before_tool_waiter_cancellation_never_dispatches_target() {
    let trace = Trace::default();
    let registry = registry(&trace, &["A"], true, Duration::from_secs(1), false);
    let gates: Vec<Arc<dyn ToolCallMiddleware>> = vec![Arc::new(Gate {
        trace: trace.clone(),
        mode: "timeout",
    })];
    let calls = [call("A")];
    assert!(
        tokio::time::timeout(
            Duration::from_millis(10),
            batch(&registry, &calls, &gates, false)
        )
        .await
        .is_err()
    );
    assert_eq!(*trace.lock().unwrap(), ["preflight:A", "gate:A"]);
}
