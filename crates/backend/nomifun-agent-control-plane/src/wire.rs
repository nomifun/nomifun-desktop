use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::ControlPlaneError;

pub(crate) fn wire_cast<T, U>(value: &T) -> Result<U, ControlPlaneError>
where
    T: Serialize,
    U: DeserializeOwned,
{
    Ok(serde_json::from_value(serde_json::to_value(value)?)?)
}

pub(crate) fn wire_name<T>(value: &T) -> Result<String, ControlPlaneError>
where
    T: Serialize,
{
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ControlPlaneError::Wire("enum did not serialize as a string".into()))
}

/// Authoring accepts catalog references, while a Session may also carry
/// host-captured library bytes. Those bytes never enter the public draft API.
pub(crate) fn document_payload(
    document: &nomifun_api_types::AgentPresetDocumentDto,
) -> Result<nomifun_agent_contracts::AgentPresetRevisionPayload, ControlPlaneError> {
    wire_cast(document)
}

pub(crate) fn payload_document(
    payload: &nomifun_agent_contracts::AgentPresetRevisionPayload,
) -> Result<nomifun_api_types::AgentPresetDocumentDto, ControlPlaneError> {
    let mut value = serde_json::to_value(payload)?;
    value["skill_bindings"] = serde_json::Value::Array(payload.skill_bindings.iter()
        .filter_map(|binding| binding.package_ref())
        .map(serde_json::to_value).collect::<Result<_, _>>()?);
    Ok(serde_json::from_value(value)?)
}
