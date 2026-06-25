//! Replay runner: re-resolves recorded steps and dispatches through the normal
//! `act` path (all safety gates intact — redline, secret origin, firewall).
//!
//! **Security invariant**: replay does NOT bypass any gate. Each step is dispatched
//! through `BrowserTool::execute` exactly as if the LLM had issued it — the redline
//! gate, secret origin gate, and firewall all re-evaluate on each replayed step.
//! A step that would be blocked live is blocked on replay too.

use serde_json::{Value, json};

use crate::recording::{Recording, RecordedStep};
use crate::tool::BrowserTool;
use nomi_tools::Tool;
use nomi_types::tool::ToolResult;

/// Per-step outcome during replay.
#[derive(Debug, Clone)]
pub struct StepOutcome {
    /// The step index (0-based).
    pub index: usize,
    /// The action that was replayed.
    pub action: String,
    /// Whether this step succeeded.
    pub success: bool,
    /// The tool result (text or error).
    pub result: ToolResult,
}

/// Outcome of a full replay.
#[derive(Debug)]
pub struct ReplayResult {
    /// Per-step outcomes in order.
    pub outcomes: Vec<StepOutcome>,
    /// How many steps completed successfully.
    pub succeeded: usize,
    /// How many steps were blocked/failed.
    pub failed: usize,
}

/// The replay runner. Stateless — takes a recording and a tool reference.
pub struct ReplayRunner;

impl ReplayRunner {
    /// Replay a recording through the tool's normal `act` path.
    ///
    /// For each step:
    /// 1. Reconstruct the tool input from the recorded step's action + args.
    /// 2. Dispatch via `tool.execute(input)` — this re-enters all safety gates.
    /// 3. Collect the outcome.
    ///
    /// If a step is blocked (redline gate denies it), the outcome records
    /// `success: false` and replay continues (does NOT abort the entire run,
    /// so subsequent steps are still attempted if desired, but the caller can
    /// check `.failed > 0`).
    ///
    /// **Security**: because we dispatch through `execute`, every gate fires:
    /// - Redline gate (irreversible actions in bypass sessions → blocked)
    /// - Secret origin gate (secret:NAME resolved only for bound origins)
    /// - Firewall (egress restrictions)
    pub async fn replay(recording: &Recording, tool: &BrowserTool) -> ReplayResult {
        let mut outcomes = Vec::with_capacity(recording.steps.len());
        let mut succeeded = 0;
        let mut failed = 0;

        for (i, step) in recording.steps.iter().enumerate() {
            let input = Self::step_to_input(step);
            let result = tool.execute(input).await;
            let success = !result.is_error;
            if success {
                succeeded += 1;
            } else {
                failed += 1;
            }
            outcomes.push(StepOutcome {
                index: i,
                action: step.action.clone(),
                success,
                result,
            });
        }

        ReplayResult { outcomes, succeeded, failed }
    }

    /// Convert a recorded step back into the tool input JSON that `execute` expects.
    ///
    /// The input is `{ "action": step.action, ...step.args }`. The `action` key is
    /// always added (it was stripped during recording to avoid redundancy with
    /// `RecordedStep::action`). The args already contain `secret:NAME` tokens (never
    /// plaintext), so replay correctly triggers the secret resolution path.
    fn step_to_input(step: &RecordedStep) -> Value {
        let mut input = match &step.args {
            Value::Object(map) => Value::Object(map.clone()),
            _ => json!({}),
        };
        // Always inject the action key.
        if let Value::Object(ref mut map) = input {
            map.insert("action".to_string(), Value::String(step.action.clone()));
        }
        input
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::RecordedStep;
    use async_trait::async_trait;
    use nomi_browser_engine::{
        ActResult, ActSpec, BrowserEngine, BrowserError, Capabilities, Effect,
        LoadState, NavResult, Observation, ObserveOpts,
    };
    use nomi_browser_engine::progress::Progress;
    use nomi_config::config::BrowserConfig;
    use serde_json::json;
    use std::sync::Arc;

    /// A fake engine that succeeds on the first act call and fails on the second
    /// (simulating the redline gate blocking step 2). Actually, for this test we
    /// use the REAL redline gate by making step 2 an irreversible action in a
    /// bypass session — the gate blocks it before it reaches the engine.
    struct AlwaysSucceedEngine;

    #[async_trait]
    impl BrowserEngine for AlwaysSucceedEngine {
        fn capabilities(&self) -> Capabilities {
            Capabilities { browser_ready: true, headful: false, display_available: false, engine: "fake".into() }
        }
        async fn navigate(&self, _url: &str, _new_tab: bool) -> Result<NavResult, BrowserError> {
            Ok(NavResult {
                final_url: "https://shop.example.com".into(),
                http_status: Some(200),
                redirected: false,
                load_state: LoadState::Load,
            })
        }
        async fn screenshot(&self) -> Result<Vec<u8>, BrowserError> {
            Err(BrowserError::Unsupported { capability: "screenshot".into(), hint: "fake".into() })
        }
        async fn rendered_html(&self) -> Result<String, BrowserError> {
            Err(BrowserError::Unsupported { capability: "html".into(), hint: "fake".into() })
        }
        async fn observe(&self, _opts: &ObserveOpts) -> Result<Observation, BrowserError> {
            Err(BrowserError::Unsupported { capability: "observe".into(), hint: "fake".into() })
        }
        async fn act(
            &self,
            _spec: &ActSpec,
            _progress: &Progress,
        ) -> Result<ActResult, BrowserError> {
            Ok(ActResult {
                success: true,
                message: "done".into(),
                effect: Effect { changed: true, before_anchor: None, after_anchor: None },
            })
        }
        async fn debug_snapshot(&self) -> Result<nomi_browser_engine::DebugSnapshot, BrowserError> {
            Err(BrowserError::Unsupported { capability: "debug".into(), hint: "fake".into() })
        }
    }

    /// **Security test**: replay re-dispatches each step through the act path and
    /// respects the redline gate. A 2-step recording where step 2 is irreversible
    /// in a bypass session → step 2 is blocked, step 1 passes.
    #[tokio::test]
    async fn replay_redispatches_each_step_and_respects_gate() {
        use nomi_browser_engine::{ElementEntry, SnapshotGen};

        // Build a BrowserTool that BYPASSES approval (yolo) so the redline gate
        // hard-denies irreversible actions.
        let t = BrowserTool::with_policy(
            &BrowserConfig::default(),
            true,  // session_bypasses_approval (yolo)
            false, // evaluate_full_power
            false, // evaluate_persistent_login
            None,  // workspace_dir
            None,  // runtime_mode
            None,  // secret_source
        );
        // Inject the fake engine.
        *t.engine.lock().expect("engine") = Some(Ok(Arc::new(AlwaysSucceedEngine)));

        // Seed a snapshot with a safe button (step 1) and a dangerous button (step 2).
        *t.last_snapshot.lock().expect("snap") = Some(Observation {
            generation: SnapshotGen(1),
            yaml: "<data></data>".into(),
            entries: vec![
                ElementEntry { r#ref: "f0e1".into(), role: "button".into(), name: "Next".into(), frame_seq: 0 },
                ElementEntry { r#ref: "f0e2".into(), role: "button".into(), name: "Pay now".into(), frame_seq: 0 },
            ],
            url: Some("https://shop.example.com/checkout".into()),
            truncated: false,
            current_page_is_post: false,
            boxes: Default::default(),
        });

        // Build a recording: step 1 = click safe button, step 2 = click dangerous button.
        let recording = Recording {
            steps: vec![
                RecordedStep {
                    intent: "click Next".into(),
                    action: "click".into(),
                    args: json!({"ref": "f0e1"}),
                    selector: Some("button.next".into()),
                    url: "https://shop.example.com/checkout".into(),
                },
                RecordedStep {
                    intent: "click Pay now".into(),
                    action: "click".into(),
                    args: json!({"ref": "f0e2"}),
                    selector: Some("button.pay".into()),
                    url: "https://shop.example.com/checkout".into(),
                },
            ],
            created_url: "https://shop.example.com/checkout".into(),
        };

        // Replay.
        let result = ReplayRunner::replay(&recording, &t).await;

        // Step 1 should succeed (safe button, engine returns Ok).
        assert_eq!(result.outcomes.len(), 2);
        assert!(
            result.outcomes[0].success,
            "step 1 (safe click) should succeed: {:?}",
            result.outcomes[0].result.content
        );

        // Step 2 should be BLOCKED by the redline gate (irreversible in bypass session).
        assert!(
            !result.outcomes[1].success,
            "step 2 (irreversible click) should be blocked by the redline gate"
        );
        let content = &result.outcomes[1].result.content;
        assert!(
            content.to_lowercase().contains("blocked")
                || content.to_lowercase().contains("irreversible"),
            "step 2 error should mention blocked/irreversible: {content}"
        );

        // Summary counts.
        assert_eq!(result.succeeded, 1);
        assert_eq!(result.failed, 1);
    }
}
