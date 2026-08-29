use std::collections::BTreeSet;

use nomifun_agent_contracts::{
    AgentSessionId, NativeActionStart, RuntimeBindingContract, RuntimeBindingId, RuntimeCommand,
    RuntimeEventWireEnvelope, RuntimeRpcMethod,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::RuntimeError;
use crate::release::RUNTIME_HELLO_METHOD;

pub const RUNTIME_EVENT_METHOD: &str = "runtime/event";
pub const NATIVE_ACTION_START_METHOD: &str = "native_action/start";

pub const FORBIDDEN_INTERACTIVE_SERVER_METHODS: [&str; 9] = [
    "item/commandExecution/requestApproval",
    "item/fileChange/requestApproval",
    "item/permissions/requestApproval",
    "item/tool/requestUserInput",
    "mcpServer/elicitation/request",
    "applyPatchApproval",
    "execCommandApproval",
    "item/autoApprovalReview/started",
    "item/autoApprovalReview/completed",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeTransport {
    Stdio,
}

impl RuntimeTransport {
    pub fn parse(value: &str) -> Result<Self, RuntimeError> {
        match value {
            "stdio" | "stdio://" => Ok(Self::Stdio),
            "ws" | "wss" | "websocket" | "ws://" | "wss://" => Err(
                RuntimeError::Protocol("websocket runtime transport is unsupported".to_owned()),
            ),
            other => Err(RuntimeError::Protocol(format!(
                "unsupported runtime transport {other:?}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StableRpcMethod {
    Create,
    Resume,
    Fork,
    StartTurn,
    Steer,
    FollowUp,
    Cancel,
    SessionDispose,
}

impl StableRpcMethod {
    pub const ALL: [Self; 8] = [
        Self::Create,
        Self::Resume,
        Self::Fork,
        Self::StartTurn,
        Self::Steer,
        Self::FollowUp,
        Self::Cancel,
        Self::SessionDispose,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Resume => "resume",
            Self::Fork => "fork",
            Self::StartTurn => "start_turn",
            Self::Steer => "steer",
            Self::FollowUp => "follow_up",
            Self::Cancel => "cancel",
            Self::SessionDispose => "session_dispose",
        }
    }

    pub const fn contract_method(self) -> RuntimeRpcMethod {
        match self {
            Self::Create => RuntimeRpcMethod::Create,
            Self::Resume => RuntimeRpcMethod::Resume,
            Self::Fork => RuntimeRpcMethod::Fork,
            Self::StartTurn => RuntimeRpcMethod::StartTurn,
            Self::Steer => RuntimeRpcMethod::Steer,
            Self::FollowUp => RuntimeRpcMethod::FollowUp,
            Self::Cancel => RuntimeRpcMethod::Cancel,
            Self::SessionDispose => RuntimeRpcMethod::SessionDispose,
        }
    }

    pub fn exact_contract_set() -> BTreeSet<RuntimeRpcMethod> {
        Self::ALL
            .into_iter()
            .map(Self::contract_method)
            .collect()
    }
}

impl From<&RuntimeCommand> for StableRpcMethod {
    fn from(command: &RuntimeCommand) -> Self {
        match command {
            RuntimeCommand::Create(_) => Self::Create,
            RuntimeCommand::Resume(_) => Self::Resume,
            RuntimeCommand::Fork(_) => Self::Fork,
            RuntimeCommand::StartTurn(_) => Self::StartTurn,
            RuntimeCommand::Steer(_) => Self::Steer,
            RuntimeCommand::FollowUp(_) => Self::FollowUp,
            RuntimeCommand::Cancel(_) => Self::Cancel,
            RuntimeCommand::SessionDispose(_) => Self::SessionDispose,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Integer(i64),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientRequest {
    pub id: RequestId,
    pub method: String,
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientResponse {
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorObject>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpcErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSessionDisposeAck {
    pub agent_session_id: AgentSessionId,
    pub runtime_binding_id: RuntimeBindingId,
    pub disposed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServerFrame {
    Response {
        id: RequestId,
        result: Result<Value, RpcErrorObject>,
    },
    RuntimeEvent {
        id: RequestId,
        event: RuntimeEventWireEnvelope,
    },
    NativeActionStart {
        id: RequestId,
        start: NativeActionStart,
    },
}

pub fn command_params(command: &RuntimeCommand) -> Result<Value, RuntimeError> {
    let value = match command {
        RuntimeCommand::Create(params) => serde_json::to_value(params)?,
        RuntimeCommand::Resume(params) => serde_json::to_value(params)?,
        RuntimeCommand::Fork(params) => serde_json::to_value(params)?,
        RuntimeCommand::StartTurn(params) => serde_json::to_value(params)?,
        RuntimeCommand::Steer(params) => serde_json::to_value(params)?,
        RuntimeCommand::FollowUp(params) => serde_json::to_value(params)?,
        RuntimeCommand::Cancel(params) => serde_json::to_value(params)?,
        RuntimeCommand::SessionDispose(params) => serde_json::to_value(params)?,
    };
    Ok(value)
}

pub fn encode_request(
    id: RequestId,
    method: &str,
    params: Value,
) -> Result<Vec<u8>, RuntimeError> {
    if method != RUNTIME_HELLO_METHOD
        && !StableRpcMethod::ALL
            .into_iter()
            .any(|allowed| allowed.as_str() == method)
    {
        return Err(RuntimeError::RpcNotAllowed(method.to_owned()));
    }

    let mut encoded = serde_json::to_vec(&ClientRequest {
        id,
        method: method.to_owned(),
        params,
    })?;
    encoded.push(b'\n');
    Ok(encoded)
}

pub fn encode_response(
    id: RequestId,
    result: Result<Value, RpcErrorObject>,
) -> Result<Vec<u8>, RuntimeError> {
    let response = match result {
        Ok(result) => ClientResponse {
            id,
            result: Some(result),
            error: None,
        },
        Err(error) => ClientResponse {
            id,
            result: None,
            error: Some(error),
        },
    };
    let mut encoded = serde_json::to_vec(&response)?;
    encoded.push(b'\n');
    Ok(encoded)
}

pub fn decode_server_frame(bytes: &[u8]) -> Result<ServerFrame, RuntimeError> {
    let value = serde_json::from_slice::<Value>(bytes)?;
    let object = value
        .as_object()
        .ok_or_else(|| RuntimeError::Protocol("runtime frame must be a JSON object".to_owned()))?;

    if object.contains_key("jsonrpc") {
        return Err(RuntimeError::Protocol(
            "pinned app-server JSONL omits the jsonrpc header".to_owned(),
        ));
    }

    match (
        object.contains_key("id"),
        object.contains_key("method"),
        object.contains_key("result"),
        object.contains_key("error"),
    ) {
        (true, false, true, false) => decode_success_response(object),
        (true, false, false, true) => decode_error_response(object),
        (true, true, false, false) => decode_server_request(object),
        (false, true, false, false) => {
            let method = required_string(object, "method")?;
            Err(forbidden_or_unknown_server_method(method, true))
        }
        _ => Err(RuntimeError::Protocol(
            "runtime frame is not an exact response or server request envelope".to_owned(),
        )),
    }
}

pub fn decode_open_result(value: Value) -> Result<RuntimeBindingContract, RuntimeError> {
    serde_json::from_value(value).map_err(RuntimeError::Json)
}

fn decode_success_response(object: &Map<String, Value>) -> Result<ServerFrame, RuntimeError> {
    require_exact_keys(object, &["id", "result"])?;
    Ok(ServerFrame::Response {
        id: serde_json::from_value(object["id"].clone())?,
        result: Ok(object["result"].clone()),
    })
}

fn decode_error_response(object: &Map<String, Value>) -> Result<ServerFrame, RuntimeError> {
    require_exact_keys(object, &["error", "id"])?;
    Ok(ServerFrame::Response {
        id: serde_json::from_value(object["id"].clone())?,
        result: Err(serde_json::from_value(object["error"].clone())?),
    })
}

fn decode_server_request(object: &Map<String, Value>) -> Result<ServerFrame, RuntimeError> {
    require_exact_keys(object, &["id", "method", "params"])?;
    let id = serde_json::from_value(object["id"].clone())?;
    let method = required_string(object, "method")?;
    match method {
        RUNTIME_EVENT_METHOD => Ok(ServerFrame::RuntimeEvent {
            id,
            event: serde_json::from_value(object["params"].clone())?,
        }),
        NATIVE_ACTION_START_METHOD => Ok(ServerFrame::NativeActionStart {
            id,
            start: serde_json::from_value(object["params"].clone())?,
        }),
        _ => Err(forbidden_or_unknown_server_method(method, false)),
    }
}

fn forbidden_or_unknown_server_method(method: &str, notification: bool) -> RuntimeError {
    if FORBIDDEN_INTERACTIVE_SERVER_METHODS.contains(&method)
        || method.contains("requestApproval")
        || method.contains("permission")
        || method.contains("elicitation")
    {
        RuntimeError::Protocol(format!(
            "interactive app-server method {method:?} is forbidden in FullAuto"
        ))
    } else if notification {
        RuntimeError::Protocol(format!(
            "server notification {method:?} escaped the pinned fork adapter"
        ))
    } else {
        RuntimeError::Protocol(format!(
            "server request {method:?} is outside the exact inbound allowlist"
        ))
    }
}

fn require_exact_keys(object: &Map<String, Value>, expected: &[&str]) -> Result<(), RuntimeError> {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(RuntimeError::Protocol(format!(
            "runtime frame keys differ from the exact wire shape: expected {expected:?}, got {actual:?}"
        )))
    }
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, RuntimeError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| RuntimeError::Protocol(format!("{key} must be a string")))
}

#[cfg(test)]
mod tests {
    use nomifun_agent_contracts::{RuntimeHelloPayload, RuntimeRpcAllowlist};

    use super::*;

    #[test]
    fn stable_rpc_set_is_exactly_the_contract_set() {
        assert_eq!(
            StableRpcMethod::exact_contract_set(),
            RuntimeRpcAllowlist::frozen().methods
        );
        assert_eq!(
            StableRpcMethod::ALL.map(StableRpcMethod::as_str),
            [
                "create",
                "resume",
                "fork",
                "start_turn",
                "steer",
                "follow_up",
                "cancel",
                "session_dispose",
            ]
        );
    }

    #[test]
    fn websocket_is_explicitly_unsupported() {
        assert!(RuntimeTransport::parse("stdio://").is_ok());
        assert!(RuntimeTransport::parse("ws://").is_err());
        assert!(RuntimeTransport::parse("wss://").is_err());
    }

    #[test]
    fn approval_request_is_a_fatal_protocol_violation() {
        let frame = br#"{"id":7,"method":"item/commandExecution/requestApproval","params":{}}"#;
        let error = decode_server_frame(frame).unwrap_err();
        assert!(error.to_string().contains("forbidden in FullAuto"));
    }

    #[test]
    fn raw_upstream_notification_cannot_escape_the_adapter() {
        let frame = br#"{"method":"turn/started","params":{}}"#;
        let error = decode_server_frame(frame).unwrap_err();
        assert!(error.to_string().contains("escaped the pinned fork adapter"));
    }

    #[test]
    fn jsonrpc_header_is_rejected_for_the_pinned_jsonl_wire() {
        let frame = br#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        assert!(decode_server_frame(frame).is_err());
    }

    #[test]
    fn hello_fixture_has_no_unknown_fields() {
        use crate::credential::{CredentialHandleDescriptor, RuntimeHelloRequest};

        let fixture = include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/hello-rpc-allowlist.json"
        );
        let hello: RuntimeHelloPayload = serde_json::from_str(fixture).unwrap();
        assert_eq!(hello.rpc_allowlist, RuntimeRpcAllowlist::frozen());
        let encoded = encode_request(
            RequestId::Integer(1),
            RUNTIME_HELLO_METHOD,
            serde_json::to_value(RuntimeHelloRequest::new(
                CredentialHandleDescriptor::WindowsHandle { value: 42 },
            ))
            .unwrap(),
        )
        .unwrap();
        let encoded: serde_json::Value =
            serde_json::from_slice(encoded.strip_suffix(b"\n").unwrap()).unwrap();
        assert_eq!(encoded["id"], 1);
        assert_eq!(encoded["method"], RUNTIME_HELLO_METHOD);
        assert_eq!(
            encoded["params"]["credential_protocol"],
            "nomifun-inherited-handle-v1"
        );
        assert_eq!(
            encoded["params"]["credential_handle"],
            serde_json::json!({ "kind": "windows_handle", "value": 42 })
        );
        assert!(encoded.get("jsonrpc").is_none());
    }
}
