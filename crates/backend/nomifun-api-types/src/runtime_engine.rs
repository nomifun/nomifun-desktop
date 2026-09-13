//! Open runtime identity contracts shared by HTTP and trusted host composition.
use nomifun_common::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const RUNTIME_HOST_CONTRACT_VERSION: u32 = 1;
pub const RUNTIME_ENGINE_BINDING_KEY: &str = "runtime_engine_binding";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEngineDescriptor {
    pub family_id: String,
    pub build_id: String,
    pub build_digest: String,
    pub display_name: String,
    pub host_contract_version: u32,
    /// Open identifiers, not an enum tied to the built-in engines.
    pub supported_profiles: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEngineBinding {
    pub family_id: String,
    pub build_id: String,
    pub build_digest: String,
    pub host_contract_version: u32,
    pub profile: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "selection", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeEngineSelector {
    Exact {
        family_id: String,
        build_id: String,
        build_digest: String,
    },
    Channel {
        family_id: String,
        channel: String,
    },
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/' | b':')
        })
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid(message: &str) -> AppError {
    AppError::BadRequest(format!("Runtime engine contract: {message}"))
}

impl RuntimeEngineDescriptor {
    pub fn validate(&self) -> Result<(), AppError> {
        if !identifier(&self.family_id)
            || !identifier(&self.build_id)
            || !digest(&self.build_digest)
        {
            return Err(invalid("invalid family/build identity or SHA-256 digest"));
        }
        if self.display_name.trim().is_empty() || self.display_name.len() > 256 {
            return Err(invalid(
                "display name is required and must be at most 256 bytes",
            ));
        }
        validate_contract_version(self.host_contract_version)?;
        let mut profiles = BTreeSet::new();
        if self.supported_profiles.is_empty()
            || self
                .supported_profiles
                .iter()
                .any(|profile| !identifier(profile) || !profiles.insert(profile))
        {
            return Err(invalid("profiles must be nonempty, valid and unique"));
        }
        Ok(())
    }
}

impl RuntimeEngineBinding {
    pub fn validate(&self) -> Result<(), AppError> {
        if !identifier(&self.family_id)
            || !identifier(&self.build_id)
            || !digest(&self.build_digest)
            || !identifier(&self.profile)
        {
            return Err(invalid("malformed exact binding"));
        }
        validate_contract_version(self.host_contract_version)
    }
}

fn validate_contract_version(version: u32) -> Result<(), AppError> {
    if version != RUNTIME_HOST_CONTRACT_VERSION {
        return Err(invalid("unsupported host contract version"));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEngineSelection {
    pub selector: RuntimeEngineSelector,
    pub profile: String,
}
