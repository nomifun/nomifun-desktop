use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::RuntimeError;
use crate::protocol::StableRpcMethod;

pub const UPSTREAM_INITIALIZE_METHOD: &str = "initialize";
pub const UPSTREAM_INITIALIZED_METHOD: &str = "initialized";
pub const UPSTREAM_COMPACT_METHOD: &str = "thread/compact/start";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamAppServerMethod {
    ThreadStart,
    ThreadResume,
    ThreadFork,
    TurnStart,
    TurnSteer,
    TurnInterrupt,
    RuntimeSessionDispose,
}

impl UpstreamAppServerMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThreadStart => "thread/start",
            Self::ThreadResume => "thread/resume",
            Self::ThreadFork => "thread/fork",
            Self::TurnStart => "turn/start",
            Self::TurnSteer => "turn/steer",
            Self::TurnInterrupt => "turn/interrupt",
            Self::RuntimeSessionDispose => "runtime/session/dispose",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinnedAppServerAdapter;

impl PinnedAppServerAdapter {
    pub const fn upstream_method(method: StableRpcMethod) -> UpstreamAppServerMethod {
        match method {
            StableRpcMethod::Create => UpstreamAppServerMethod::ThreadStart,
            StableRpcMethod::Resume => UpstreamAppServerMethod::ThreadResume,
            StableRpcMethod::Fork => UpstreamAppServerMethod::ThreadFork,
            StableRpcMethod::StartTurn | StableRpcMethod::FollowUp => {
                UpstreamAppServerMethod::TurnStart
            }
            StableRpcMethod::Steer => UpstreamAppServerMethod::TurnSteer,
            StableRpcMethod::Cancel => UpstreamAppServerMethod::TurnInterrupt,
            StableRpcMethod::SessionDispose => UpstreamAppServerMethod::RuntimeSessionDispose,
        }
    }

    pub fn initialize_params(client_version: &str) -> Value {
        json!({
            "clientInfo": {
                "name": "nomifun",
                "title": "NomiFun",
                "version": client_version
            },
            "capabilities": {
                "experimentalApi": false,
                "requestAttestation": false
            }
        })
    }

    pub fn initialized_notification() -> Value {
        json!({ "method": UPSTREAM_INITIALIZED_METHOD })
    }

    pub fn adapt(
        method: StableRpcMethod,
        params: Value,
    ) -> Result<UpstreamAppServerCall, RuntimeError> {
        let params = match method {
            StableRpcMethod::Create
            | StableRpcMethod::Resume
            | StableRpcMethod::Fork => Self::thread_open_params(params)?,
            StableRpcMethod::StartTurn | StableRpcMethod::FollowUp => {
                Self::turn_start_params(params)?
            }
            StableRpcMethod::Steer
            | StableRpcMethod::Cancel
            | StableRpcMethod::SessionDispose => Self::passthrough_params(params)?,
        };
        Ok(UpstreamAppServerCall {
            method: Self::upstream_method(method),
            params,
        })
    }

    pub fn thread_open_params(base: Value) -> Result<Value, RuntimeError> {
        let mut object = into_object(base)?;
        reject_policy_override(&object)?;
        object.insert("approvalPolicy".to_owned(), Value::String("never".to_owned()));
        object.insert(
            "sandbox".to_owned(),
            Value::String("danger-full-access".to_owned()),
        );
        Ok(Value::Object(object))
    }

    pub fn turn_start_params(base: Value) -> Result<Value, RuntimeError> {
        let mut object = into_object(base)?;
        reject_policy_override(&object)?;
        object.insert("approvalPolicy".to_owned(), Value::String("never".to_owned()));
        object.insert(
            "sandboxPolicy".to_owned(),
            json!({ "type": "dangerFullAccess" }),
        );
        Ok(Value::Object(object))
    }

    pub fn passthrough_params(base: Value) -> Result<Value, RuntimeError> {
        let object = into_object(base)?;
        reject_policy_override(&object)?;
        Ok(Value::Object(object))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamAppServerCall {
    pub method: UpstreamAppServerMethod,
    pub params: Value,
}

fn into_object(value: Value) -> Result<Map<String, Value>, RuntimeError> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| RuntimeError::Protocol("app-server params must be an object".to_owned()))
}

fn reject_policy_override(object: &Map<String, Value>) -> Result<(), RuntimeError> {
    for forbidden in [
        "approvalPolicy",
        "approvalsReviewer",
        "sandbox",
        "sandboxPolicy",
        "permissions",
        "permissionProfile",
    ] {
        if object.contains_key(forbidden) {
            return Err(RuntimeError::Protocol(format!(
                "caller cannot override fixed app-server field {forbidden}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_methods_map_only_to_the_pinned_app_server_surface() {
        assert_eq!(
            PinnedAppServerAdapter::upstream_method(StableRpcMethod::Create).as_str(),
            "thread/start"
        );
        assert_eq!(
            PinnedAppServerAdapter::upstream_method(StableRpcMethod::Resume).as_str(),
            "thread/resume"
        );
        assert_eq!(
            PinnedAppServerAdapter::upstream_method(StableRpcMethod::Fork).as_str(),
            "thread/fork"
        );
        assert_eq!(
            PinnedAppServerAdapter::upstream_method(StableRpcMethod::StartTurn).as_str(),
            "turn/start"
        );
        assert_eq!(
            PinnedAppServerAdapter::upstream_method(StableRpcMethod::FollowUp).as_str(),
            "turn/start"
        );
        assert_eq!(
            PinnedAppServerAdapter::upstream_method(StableRpcMethod::Steer).as_str(),
            "turn/steer"
        );
        assert_eq!(
            PinnedAppServerAdapter::upstream_method(StableRpcMethod::Cancel).as_str(),
            "turn/interrupt"
        );
        assert_eq!(
            PinnedAppServerAdapter::upstream_method(StableRpcMethod::SessionDispose).as_str(),
            "runtime/session/dispose"
        );
    }

    #[test]
    fn full_auto_overlay_matches_dc2_thread_and_turn_wire() {
        assert_eq!(
            PinnedAppServerAdapter::thread_open_params(json!({"cwd": "C:\\repo"})).unwrap(),
            json!({
                "cwd": "C:\\repo",
                "approvalPolicy": "never",
                "sandbox": "danger-full-access"
            })
        );
        assert_eq!(
            PinnedAppServerAdapter::turn_start_params(json!({"threadId": "thread-1"})).unwrap(),
            json!({
                "threadId": "thread-1",
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "dangerFullAccess"}
            })
        );
    }

    #[test]
    fn adapter_applies_policy_before_emitting_upstream_call() {
        let call = PinnedAppServerAdapter::adapt(
            StableRpcMethod::Create,
            json!({"cwd": "C:\\repo"}),
        )
        .unwrap();
        assert_eq!(call.method.as_str(), "thread/start");
        assert_eq!(call.params["approvalPolicy"], "never");
        assert_eq!(call.params["sandbox"], "danger-full-access");

        let call =
            PinnedAppServerAdapter::adapt(StableRpcMethod::Cancel, json!({"turnId": "t"}))
                .unwrap();
        assert_eq!(call.method.as_str(), "turn/interrupt");
        assert!(call.params.get("approvalPolicy").is_none());
    }

    #[test]
    fn caller_cannot_restore_an_upstream_policy_choice() {
        assert!(
            PinnedAppServerAdapter::thread_open_params(json!({
                "approvalPolicy": "on-request"
            }))
            .is_err()
        );
        assert!(
            PinnedAppServerAdapter::turn_start_params(json!({
                "permissions": ":read-only"
            }))
            .is_err()
        );
    }

    #[test]
    fn compact_is_internal_and_not_a_stable_rpc_method() {
        assert_eq!(UPSTREAM_COMPACT_METHOD, "thread/compact/start");
        assert!(
            StableRpcMethod::ALL
                .into_iter()
                .all(|method| method.as_str() != UPSTREAM_COMPACT_METHOD)
        );
    }
}
