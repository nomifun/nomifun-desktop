//! Bounded DOM semantics in WKWebView's isolated content world. These operations
//! do not supply trusted pointer/key input or website user activation.

use std::{collections::HashSet, sync::{Arc, atomic::{AtomicBool, Ordering}}};

use nomifun_browser_macos::engine::Page;
use nomifun_browser_platform::{run_guard::RunAdmissionError, runtime::*};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const SCRIPT: &str = include_str!("semantic.js");
const MAX_ELEMENTS: usize = 600;
const MAX_CONTENT_CHARS: usize = 32_768;
const MAX_NAME_CHARS: usize = 512;
const MAX_INPUT_BYTES: usize = 256 * 1024;

#[derive(Default)]
pub(crate) struct TabAutomation {
    observation: u64,
    observed_target: Option<BrowserTabTarget>,
    nonce: String,
    references: HashSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedElement {
    ref_id: String,
    role: String,
    name: String,
    focused: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedPage {
    content: String,
    elements: Vec<ObservedElement>,
    unobserved_frames: usize,
}

fn check_admission(cancel: &CancellationToken, locked: &AtomicBool) -> Result<(), WorkspaceError> {
    if cancel.is_cancelled() {
        return Err(RunAdmissionError::Cancelled.into());
    }
    if !locked.load(Ordering::Acquire) {
        return Err(RunAdmissionError::StaleRun.into());
    }
    Ok(())
}

/// The native layer distinguishes rejection before dispatch from uncertainty
/// after dispatch. Never retry a sent script, even if its callback is lost.
fn native_error(error: &str) -> WorkspaceError {
    match error {
        "BROWSER_ACTION_INTERRUPTED" => WorkspaceError::ActionInterrupted,
        "BROWSER_EXECUTION_UNCONFIRMED" => RunAdmissionError::WorkerFailed.into(),
        "BROWSER_STALE_OBSERVATION" => WorkspaceError::StaleObservation,
        "BROWSER_ELEMENT_NOT_ACTIONABLE" => WorkspaceError::NotActionable,
        "BROWSER_CANCELLED" => RunAdmissionError::Cancelled.into(),
        "BROWSER_ACTION_DENIED" => WorkspaceError::ActionDenied,
        "BROWSER_PAGE_FAILED" => WorkspaceError::PageFailed,
        "BROWSER_PAGE_STOPPED" => WorkspaceError::PageStopped,
        "BROWSER_PAGE_CRASHED" => WorkspaceError::PageCrashed,
        _ => WorkspaceError::NativeCommandFailed,
    }
}

fn action_native_error(error: &str) -> WorkspaceError {
    match native_error(error) {
        // A lost/failed WebKit completion does not prove that a DOM action was
        // never delivered. Preserve uncertainty instead of inviting a replay.
        WorkspaceError::NativeCommandFailed => WorkspaceError::ActionInterrupted,
        error => error,
    }
}

fn cancelled_action_error(result: Result<(), WorkspaceError>) -> WorkspaceError {
    match result {
        Err(WorkspaceError::Admission(RunAdmissionError::WorkerFailed)) => RunAdmissionError::WorkerFailed.into(),
        Err(WorkspaceError::Admission(RunAdmissionError::Cancelled)) => RunAdmissionError::Cancelled.into(),
        _ => WorkspaceError::ActionInterrupted,
    }
}

fn script(request: Value) -> String {
    // WK returns a JSON string across the Objective-C boundary. User text is a
    // JSON value, never interpolated as executable JavaScript or diagnostic text.
    format!("JSON.stringify(({SCRIPT})({request}))")
}

fn checked_result(value: Value) -> Result<Value, WorkspaceError> {
    match value.get("error").and_then(Value::as_str) {
        Some("stale") => Err(WorkspaceError::StaleObservation),
        Some("not_actionable") => Err(WorkspaceError::NotActionable),
        Some("unsupported") => Err(WorkspaceError::UnsupportedAction),
        Some("interrupted") => Err(WorkspaceError::ActionInterrupted),
        Some("limit") => Err(WorkspaceError::ObservationLimit),
        Some(_) => Err(WorkspaceError::NativeCommandFailed),
        None => Ok(value),
    }
}

fn action_request(action: &BrowserAction) -> Result<Value, WorkspaceError> {
    match action {
        BrowserAction::Click { button: BrowserMouseButton::Left, click_count: 1, .. } => Ok(json!({"operation":"click"})),
        BrowserAction::Type { text, .. } if text.len() <= MAX_INPUT_BYTES => Ok(json!({"operation":"type", "text":text})),
        BrowserAction::Select { labels, .. } if labels.len() <= MAX_ELEMENTS && labels.iter().all(|label|label.len() <= 2048) => {
            if labels.iter().collect::<HashSet<_>>().len() != labels.len() {
                return Err(WorkspaceError::NotActionable);
            }
            Ok(json!({"operation":"select", "labels":labels}))
        }
        BrowserAction::Scroll { delta_x, delta_y, .. } if delta_x.is_finite() && delta_y.is_finite()
            && delta_x.abs() <= 1_000_000.0 && delta_y.abs() <= 1_000_000.0 => {
            Ok(json!({"operation":"scroll", "delta_x":delta_x, "delta_y":delta_y}))
        }
        // Keyboard events and hover/drag are not approximated using dispatchEvent.
        _ => Err(WorkspaceError::UnsupportedAction),
    }
}

impl TabAutomation {
    pub(crate) fn invalidate_observation(&mut self) {
        self.observed_target = None;
        self.nonce.clear();
        self.references.clear();
    }

    fn validate(&self, reference: &BrowserElementRef) -> Result<(), WorkspaceError> {
        if self.observed_target.as_ref() != Some(&reference.target)
            || self.observation != reference.observation_generation
            || !self.references.contains(&reference.ref_id)
        {
            return Err(WorkspaceError::StaleObservation);
        }
        Ok(())
    }

    pub(crate) async fn observe(
        &mut self,
        page: Arc<Page>,
        target: BrowserTabTarget,
        cancel: &CancellationToken,
        input_locked: Arc<AtomicBool>,
    ) -> Result<BrowserObservation, WorkspaceError> {
        check_admission(cancel, &input_locked)?;
        self.invalidate_observation();
        if page.snapshot().document_generation != target.document_generation {
            return Err(WorkspaceError::StaleTarget);
        }
        self.observation = self.observation.checked_add(1).ok_or(WorkspaceError::ObservationLimit)?;
        let nonce = uuid::Uuid::now_v7().simple().to_string();
        let request = json!({"operation":"observe", "nonce":nonce,
            "max_elements":MAX_ELEMENTS, "max_content":MAX_CONTENT_CHARS, "max_name":MAX_NAME_CHARS});
        let cancellation = cancel.clone();
        let guard = Arc::new(move || input_locked.load(Ordering::Acquire) && !cancellation.is_cancelled());
        let value = page.evaluate(script(request), target.document_generation, guard, cancel.clone())
            .await.map_err(|error|native_error(&error))?;
        if cancel.is_cancelled() { return Err(RunAdmissionError::Cancelled.into()); }
        if page.snapshot().document_generation != target.document_generation {
            return Err(WorkspaceError::StaleObservation);
        }
        let result: ObservedPage = serde_json::from_value(checked_result(value)?)
            .map_err(|_|WorkspaceError::NativeCommandFailed)?;
        if result.elements.len() > MAX_ELEMENTS || result.content.chars().count() > MAX_CONTENT_CHARS
            || result.elements.iter().any(|element| element.name.chars().count() > MAX_NAME_CHARS
                || element.role.len() > 128 || element.ref_id.len() > 96
                || !element.ref_id.starts_with(&format!("wk.{nonce}.")))
        {
            return Err(WorkspaceError::ObservationLimit);
        }
        let elements: Vec<_> = result.elements.into_iter().map(|element| BrowserElement {
            reference: BrowserElementRef { target:target.clone(), observation_generation:self.observation, ref_id:element.ref_id },
            role:element.role, name:element.name, focused:element.focused,
        }).collect();
        self.references = elements.iter().map(|element|element.reference.ref_id.clone()).collect();
        if self.references.len() != elements.len() { return Err(WorkspaceError::NativeCommandFailed); }
        self.nonce = nonce;
        self.observed_target = Some(target.clone());
        let snapshot = page.snapshot();
        if snapshot.document_generation != target.document_generation { return Err(WorkspaceError::StaleObservation); }
        Ok(BrowserObservation { target, observation_generation:self.observation, content:result.content,
            elements, script_dialog:None, unobserved_frames:result.unobserved_frames,
            load: Some(snapshot.load.agent_projection()),
            content_url: snapshot.load.content_url.as_deref().map(nomifun_browser_platform::url_projection::project_metadata_url) })
    }

    pub(crate) async fn act(
        &mut self,
        page: Arc<Page>,
        action: BrowserAction,
        cancel: &CancellationToken,
        input_locked: Arc<AtomicBool>,
    ) -> Result<(), WorkspaceError> {
        check_admission(cancel, &input_locked)?;
        // Reject a declared unsupported operation before attempting any script.
        let mut request = action_request(&action)?;
        let reference = action.element();
        self.validate(reference)?;
        if page.snapshot().document_generation != reference.target.document_generation {
            self.invalidate_observation();
            return Err(WorkspaceError::StaleObservation);
        }
        request["ref_id"] = json!(reference.ref_id);
        request["nonce"] = json!(self.nonce);
        let cancellation = cancel.clone();
        let guard = Arc::new(move || input_locked.load(Ordering::Acquire) && !cancellation.is_cancelled());
        let value = page.evaluate(script(request), reference.target.document_generation, guard, cancel.clone())
            .await.map_err(|error|action_native_error(&error));
        let result = value.and_then(checked_result).and_then(|value| {
            if value.get("ok") == Some(&Value::Bool(true)) { Ok(()) }
            else { Err(WorkspaceError::NativeCommandFailed) }
        });
        // A callback may arrive after Stop even when the DOM mutation completed.
        if cancel.is_cancelled() {
            self.invalidate_observation();
            return Err(cancelled_action_error(result));
        }
        // Every sent action consumes its observation: page handlers may have
        // changed other controls without replacing the document or target node.
        self.invalidate_observation();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference() -> BrowserElementRef {
        BrowserElementRef { target: BrowserTabTarget { tab_id:"browser-1".into(), runtime_generation:3, document_generation:5 },
            observation_generation:7, ref_id:"wk.nonce.0".into() }
    }

    #[test]
    fn only_current_observation_owned_refs_are_admitted() {
        let reference = reference();
        let mut automation = TabAutomation { observation:7, observed_target:Some(reference.target.clone()),
            nonce:"nonce".into(), references:HashSet::from([reference.ref_id.clone()]) };
        assert!(automation.validate(&reference).is_ok());
        let mut forged = reference.clone();
        forged.ref_id = "wk.nonce.1".into();
        assert_eq!(automation.validate(&forged), Err(WorkspaceError::StaleObservation));
        let mut old = reference.clone();
        old.target.document_generation -= 1;
        assert_eq!(automation.validate(&old), Err(WorkspaceError::StaleObservation));
        old = reference.clone(); old.observation_generation -= 1;
        assert_eq!(automation.validate(&old), Err(WorkspaceError::StaleObservation));
        automation.invalidate_observation();
        assert_eq!(automation.validate(&reference), Err(WorkspaceError::StaleObservation));
    }

    #[test]
    fn unsupported_inputs_never_produce_a_script_request() {
        for action in [BrowserAction::Hover {element:reference()}, BrowserAction::Press {element:reference(), keys:"Enter".into()},
            BrowserAction::Click {element:reference(),button:BrowserMouseButton::Right,click_count:1},
            BrowserAction::Click {element:reference(),button:BrowserMouseButton::Left,click_count:2},
            BrowserAction::Drag {from:reference(),to:reference()},
            BrowserAction::Scroll {element:reference(),delta_x:f64::NAN,delta_y:0.0}]
        { assert_eq!(action_request(&action), Err(WorkspaceError::UnsupportedAction)); }
    }

    #[test]
    fn native_cancellation_does_not_reclassify_sent_input_as_retryable() {
        assert_eq!(native_error("BROWSER_ACTION_INTERRUPTED"),WorkspaceError::ActionInterrupted);
        assert_eq!(native_error("BROWSER_CANCELLED"),RunAdmissionError::Cancelled.into());
        assert_eq!(action_native_error("WK JavaScript evaluation failed"), WorkspaceError::ActionInterrupted);
        assert_eq!(checked_result(json!({"error":"interrupted"})),Err(WorkspaceError::ActionInterrupted));
        let unconfirmed = WorkspaceError::Admission(RunAdmissionError::WorkerFailed);
        assert_eq!(native_error("BROWSER_EXECUTION_UNCONFIRMED"), unconfirmed);
        assert_eq!(action_native_error("BROWSER_EXECUTION_UNCONFIRMED"), unconfirmed);
        assert_eq!(cancelled_action_error(Err(unconfirmed)), unconfirmed);
        assert_eq!(cancelled_action_error(Err(WorkspaceError::ActionInterrupted)), WorkspaceError::ActionInterrupted);
    }

    #[test]
    fn typed_text_is_json_data_and_no_trusted_event_is_claimed() {
        let input = "\"}); globalThis.injection = true; //\nsecret";
        let request = action_request(&BrowserAction::Type {element:reference(),text:input.into()}).unwrap();
        assert_eq!(request["text"],input);
        assert!(script(request.clone()).ends_with(&format!("({request}))")));
        assert!(!SCRIPT.contains("isTrusted:"));
        assert!(!SCRIPT.contains("KeyboardEvent("));
        assert!(!SCRIPT.contains("MouseEvent("));
    }

    #[test]
    fn duplicate_select_labels_and_unlocked_runs_are_rejected() {
        assert_eq!(action_request(&BrowserAction::Select {element:reference(), labels:vec!["same".into(),"same".into()]}),Err(WorkspaceError::NotActionable));
        assert_eq!(check_admission(&CancellationToken::new(),&AtomicBool::new(false)),Err(RunAdmissionError::StaleRun.into()));
    }
}
