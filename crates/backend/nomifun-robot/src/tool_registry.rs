//! Which robots are connected and what tools they offer.
//!
//! Sessions attach on handshake and detach on disconnect; the MCP proxy reads
//! from here. Tool descriptors are cached at attach time so `tools/list` never
//! has to round-trip a sleeping device.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::mcp_bridge::{RobotMcpClient, RobotToolDescriptor, ToolCallError};

/// Disjoint device-tool authority granted by one Agent capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotToolCapability {
    Display,
    Motion,
    DeviceTools,
}

impl RobotToolCapability {
    pub fn capability_id(self) -> &'static str {
        match self {
            Self::Display => "robot.display",
            Self::Motion => "robot.motion",
            Self::DeviceTools => "robot.device_tools",
        }
    }
}

/// Classify a firmware tool by its stable device-side namespace.
///
/// Unknown extensions stay in `robot.device_tools`; descriptions are not used
/// because they are device-supplied prose and therefore not an authority.
pub fn tool_capability(device_name: &str) -> RobotToolCapability {
    let namespace = device_name
        .strip_prefix("self.")
        .unwrap_or(device_name)
        .split('.')
        .next()
        .unwrap_or_default();
    match namespace {
        "display" | "screen" | "oled" | "emoji" | "face" | "led" => {
            RobotToolCapability::Display
        }
        "head" | "gimbal" | "motion" | "servo" => RobotToolCapability::Motion,
        _ => RobotToolCapability::DeviceTools,
    }
}

struct Attached {
    client: Arc<RobotMcpClient>,
    tools: Vec<RobotToolDescriptor>,
}

/// Live robot → (MCP client, cached toolset).
#[derive(Default)]
pub struct RobotToolRegistry {
    inner: RwLock<HashMap<String, Attached>>,
}

impl RobotToolRegistry {
    /// Register a connected robot and its discovered tools.
    pub async fn attach(
        &self,
        robot_id: &str,
        client: Arc<RobotMcpClient>,
        tools: Vec<RobotToolDescriptor>,
    ) {
        self.inner
            .write()
            .await
            .insert(robot_id.to_owned(), Attached { client, tools });
    }

    /// Forget a robot (link dropped).
    pub async fn detach(&self, robot_id: &str) {
        let removed = self.inner.write().await.remove(robot_id);
        if let Some(attached) = removed {
            attached.client.cancel_pending();
        }
    }

    /// Whether an authenticated device link currently owns a live MCP client.
    pub async fn is_attached(&self, robot_id: &str) -> bool {
        self.inner.read().await.contains_key(robot_id)
    }

    /// Cached toolset, empty when the robot is not connected.
    pub async fn tools(&self, robot_id: &str) -> Vec<RobotToolDescriptor> {
        self.inner
            .read()
            .await
            .get(robot_id)
            .map(|a| a.tools.clone())
            .unwrap_or_default()
    }

    /// Cached tools inside one selected Agent capability ceiling.
    pub async fn tools_for_capability(
        &self,
        robot_id: &str,
        capability: RobotToolCapability,
    ) -> Vec<RobotToolDescriptor> {
        self.inner
            .read()
            .await
            .get(robot_id)
            .map(|attached| {
                attached
                    .tools
                    .iter()
                    .filter(|tool| tool_capability(&tool.device_name) == capability)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Invoke a tool by the name models see.
    pub async fn call(
        &self,
        robot_id: &str,
        exposed_name: &str,
        args: Value,
    ) -> Result<String, ToolCallError> {
        let (client, device_name) = {
            let map = self.inner.read().await;
            let attached = map.get(robot_id).ok_or(ToolCallError::Offline)?;
            let tool = attached
                .tools
                .iter()
                .find(|t| t.exposed_name == exposed_name)
                .ok_or_else(|| ToolCallError::Rejected(format!("unknown tool {exposed_name}")))?;
            (attached.client.clone(), tool.device_name.clone())
        };
        client.call_tool(&device_name, args).await
    }

    /// Invoke a tool only inside one selected Agent capability ceiling.
    pub async fn call_for_capability(
        &self,
        robot_id: &str,
        capability: RobotToolCapability,
        exposed_name: &str,
        args: Value,
    ) -> Result<String, ToolCallError> {
        self.call_exact_for_capability(robot_id, capability, exposed_name, None, args)
            .await
    }

    /// Invoke the exact device tool frozen into a host-owned AgentSession.
    ///
    /// A later firmware reconnect may publish the same provider-safe exposed
    /// name for a different device-side tool. Session projection therefore
    /// freezes both names and this method checks them together under the same
    /// registry read lock used to select the live client.
    pub async fn call_exact_for_capability(
        &self,
        robot_id: &str,
        capability: RobotToolCapability,
        exposed_name: &str,
        expected_device_name: Option<&str>,
        args: Value,
    ) -> Result<String, ToolCallError> {
        let (client, device_name) = {
            let map = self.inner.read().await;
            let attached = map.get(robot_id).ok_or(ToolCallError::Offline)?;
            let tool = attached
                .tools
                .iter()
                .find(|tool| tool.exposed_name == exposed_name)
                .ok_or_else(|| ToolCallError::Rejected(format!("unknown tool {exposed_name}")))?;
            let actual = tool_capability(&tool.device_name);
            if actual != capability {
                return Err(ToolCallError::Rejected(format!(
                    "tool {exposed_name} belongs to {}, not {}",
                    actual.capability_id(),
                    capability.capability_id(),
                )));
            }
            if let Some(expected_device_name) = expected_device_name {
                if tool.device_name != expected_device_name {
                    return Err(ToolCallError::Rejected(format!(
                        "tool {exposed_name} no longer resolves to the device tool frozen into this AgentSession"
                    )));
                }
            }
            (attached.client.clone(), tool.device_name.clone())
        };
        client.call_tool(&device_name, args).await
    }

    /// Cancel host waiters for the selected device without disconnecting it.
    /// Used when the owning Agent action is cancelled after dispatch.
    pub async fn cancel_pending(&self, robot_id: &str) -> bool {
        let client = self
            .inner
            .read()
            .await
            .get(robot_id)
            .map(|attached| Arc::clone(&attached.client));
        if let Some(client) = client {
            client.cancel_pending();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Frame;
    use serde_json::json;
    use tokio::sync::mpsc;

    fn descriptor(device_name: &str) -> RobotToolDescriptor {
        RobotToolDescriptor {
            device_name: device_name.to_owned(),
            exposed_name: crate::mcp_bridge::exposed_tool_name(device_name),
            description: device_name.to_owned(),
            input_schema: json!({ "type": "object" }),
        }
    }

    async fn registry() -> RobotToolRegistry {
        let (tx, _rx) = mpsc::channel::<Frame>(4);
        let client = Arc::new(RobotMcpClient::new(tx, "session-1".to_owned()));
        let registry = RobotToolRegistry::default();
        registry
            .attach(
                "robot-1",
                client,
                vec![
                    descriptor("self.emoji.set_expression"),
                    descriptor("self.head.look"),
                    descriptor("self.audio_speaker.set_volume"),
                    descriptor("get_device_status"),
                ],
            )
            .await;
        registry
    }

    #[test]
    fn device_names_map_to_disjoint_capability_ceilings() {
        for name in [
            "self.display.draw",
            "self.screen.clear",
            "self.oled.write",
            "self.emoji.set_expression",
            "self.face.set",
            "self.led.set",
        ] {
            assert_eq!(tool_capability(name), RobotToolCapability::Display, "{name}");
        }
        for name in [
            "self.head.look",
            "self.gimbal.turn",
            "self.motion.stop",
            "self.servo.calibrate",
        ] {
            assert_eq!(tool_capability(name), RobotToolCapability::Motion, "{name}");
        }
        for name in [
            "self.audio_speaker.set_volume",
            "self.camera.take_photo",
            "get_device_status",
        ] {
            assert_eq!(
                tool_capability(name),
                RobotToolCapability::DeviceTools,
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn discovery_and_dispatch_respect_the_selected_capability_ceiling() {
        let registry = registry().await;
        assert!(registry.is_attached("robot-1").await);
        let display = registry
            .tools_for_capability("robot-1", RobotToolCapability::Display)
            .await;
        assert_eq!(display.len(), 1);
        assert_eq!(display[0].device_name, "self.emoji.set_expression");

        let error = registry
            .call_for_capability(
                "robot-1",
                RobotToolCapability::Display,
                "robot_head_look",
                json!({}),
            )
            .await
            .expect_err("display authority must not dispatch a motion tool");
        assert!(matches!(error, ToolCallError::Rejected(_)));
        assert!(error.to_string().contains("robot.motion"));
        assert!(error.to_string().contains("robot.display"));
    }

    #[tokio::test]
    async fn detach_clears_the_live_link() {
        let registry = registry().await;
        registry.detach("robot-1").await;
        assert!(!registry.is_attached("robot-1").await);
        assert!(
            registry
                .call_for_capability(
                    "robot-1",
                    RobotToolCapability::DeviceTools,
                    "get_device_status",
                    json!({}),
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn exact_session_tool_rejects_a_reconnected_name_with_changed_device_identity() {
        let registry = registry().await;
        let (tx, _rx) = mpsc::channel::<Frame>(4);
        registry
            .attach(
                "robot-1",
                Arc::new(RobotMcpClient::new(tx, "session-2".to_owned())),
                vec![RobotToolDescriptor {
                    device_name: "self.emoji.set-expression".to_owned(),
                    exposed_name: "robot_emoji_set_expression".to_owned(),
                    description: "replacement".to_owned(),
                    input_schema: json!({ "type": "object" }),
                }],
            )
            .await;

        let error = registry
            .call_exact_for_capability(
                "robot-1",
                RobotToolCapability::Display,
                "robot_emoji_set_expression",
                Some("self.emoji.set_expression"),
                json!({}),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ToolCallError::Rejected(_)));
        assert!(error.to_string().contains("frozen into this AgentSession"));
    }
}
