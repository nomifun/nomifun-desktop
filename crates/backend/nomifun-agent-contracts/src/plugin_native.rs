//! Native Service identity. Execution remains behind the same Product host.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum NativePluginTarget {
    #[serde(rename = "x86_64-pc-windows-msvc")]
    WindowsX64,
    #[serde(rename = "aarch64-pc-windows-msvc")]
    WindowsArm64,
    #[serde(rename = "x86_64-unknown-linux-gnu")]
    LinuxX64,
    #[serde(rename = "aarch64-unknown-linux-gnu")]
    LinuxArm64,
    #[serde(rename = "x86_64-apple-darwin")]
    MacX64,
    #[serde(rename = "aarch64-apple-darwin")]
    MacArm64,
}

impl NativePluginTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowsX64 => "x86_64-pc-windows-msvc",
            Self::WindowsArm64 => "aarch64-pc-windows-msvc",
            Self::LinuxX64 => "x86_64-unknown-linux-gnu",
            Self::LinuxArm64 => "aarch64-unknown-linux-gnu",
            Self::MacX64 => "x86_64-apple-darwin",
            Self::MacArm64 => "aarch64-apple-darwin",
        }
    }

    pub fn current() -> Option<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") if cfg!(target_env = "msvc") => Some(Self::WindowsX64),
            ("windows", "aarch64") if cfg!(target_env = "msvc") => Some(Self::WindowsArm64),
            ("linux", "x86_64") if cfg!(target_env = "gnu") => Some(Self::LinuxX64),
            ("linux", "aarch64") if cfg!(target_env = "gnu") => Some(Self::LinuxArm64),
            ("macos", "x86_64") => Some(Self::MacX64),
            ("macos", "aarch64") => Some(Self::MacArm64),
            _ => None,
        }
    }

    pub fn entrypoint(self) -> &'static str {
        match self {
            Self::WindowsX64 | Self::WindowsArm64 => "service/plugin.exe",
            _ => "service/plugin",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginServiceExecution {
    #[default]
    Node,
    Native {
        target: NativePluginTarget,
    },
}

impl PluginServiceExecution {
    pub fn is_node(&self) -> bool {
        matches!(self, Self::Node)
    }
    pub fn entrypoint(&self) -> &'static str {
        match self {
            Self::Node => "service/main.mjs",
            Self::Native { target } => target.entrypoint(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginServiceRuntimeFingerprint;
    use serde_json::json;

    #[test]
    fn fingerprint_preserves_node_wire_format_and_rejects_mixed_backends() {
        let node = json!({"runtime_installation_id":"node-test", "runtime_target":"windows-x64",
            "runtime_executable_digest":"a".repeat(64), "node_version":"22.0.0"});
        let parsed: PluginServiceRuntimeFingerprint = serde_json::from_value(node.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), node);
        let native = json!({"native_target":"x86_64-pc-windows-msvc", "native_executable_digest":"b".repeat(64)});
        let parsed: PluginServiceRuntimeFingerprint =
            serde_json::from_value(native.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), native);
        let mut mixed = node;
        mixed["native_target"] = native["native_target"].clone();
        assert!(serde_json::from_value::<PluginServiceRuntimeFingerprint>(mixed).is_err());
        assert!(serde_json::from_value::<NativePluginTarget>(json!("unknown-platform")).is_err());
    }
}
