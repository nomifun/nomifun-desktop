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
