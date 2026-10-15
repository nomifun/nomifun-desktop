//! Which robots are connected and what tools they offer.
//!
//! Sessions attach on handshake and detach on disconnect; the canonical Robot
//! Action owner reads from here. Tool descriptors are cached at attach time so
//! action materialization never has to round-trip a sleeping device.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::capability::RobotAction;
use crate::mcp_bridge::{RobotMcpClient, RobotToolDescriptor, ToolCallError};

/// Classify a firmware tool by its stable device-side namespace.
///
/// Unknown extensions stay in `robot/device`; descriptions are not used
/// because they are device-supplied prose and therefore not an authority.
pub fn tool_action(device_name: &str) -> RobotAction {
    let namespace = device_name
        .strip_prefix("self.")
        .unwrap_or(device_name)
        .split('.')
        .next()
        .unwrap_or_default();
    match namespace {
        "display" | "screen" | "oled" | "emoji" | "face" | "led" => RobotAction::Display,
        "head" | "gimbal" | "motion" | "servo" => RobotAction::Motion,
        "camera" | "vision" => RobotAction::Vision,
        _ => RobotAction::Device,
    }
}

pub fn requires_continuous_vision(device_name: &str) -> bool {
    requires_continuous_vision_for_action(device_name, tool_action(device_name))
}

pub fn requires_continuous_vision_for_action(device_name: &str, action: RobotAction) -> bool {
    action == RobotAction::Vision
        && device_name.split(['.', '_']).any(|part| matches!(part, "stream" | "watch" | "continuous" | "monitor"))
}

struct Attached {
    client: Arc<RobotMcpClient>,
    tools: Vec<RobotToolDescriptor>,
}

/// Complete catalog prepared from one exact authenticated connection. Its
/// fields are private so management callers cannot synthesize hardware tools.
pub struct PreparedRobotToolRefresh {
    robot_id: String,
    client: Arc<RobotMcpClient>,
    tools: Vec<RobotToolDescriptor>,
}

/// Retains the cache write lock while the Robot owner durably advances its
/// capability revision. Dropping it before publish leaves the old cache intact.
pub struct RobotToolRefreshCommit<'a> {
    map: tokio::sync::RwLockWriteGuard<'a, HashMap<String, Attached>>,
    prepared: PreparedRobotToolRefresh,
}

impl RobotToolRefreshCommit<'_> {
    pub fn publish(mut self) {
        self.map.get_mut(&self.prepared.robot_id).expect("prepared connection is locked").tools = self.prepared.tools;
    }
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

    pub async fn detach_connection(&self, robot_id: &str, connection_id: &str) {
        let mut map = self.inner.write().await;
        if map.get(robot_id).is_some_and(|attached| attached.client.connection_id() == connection_id)
            && let Some(attached) = map.remove(robot_id) { attached.client.cancel_pending(); }
    }

    pub async fn detach_if_disconnected(&self, registry: &crate::registry::RobotRegistry, robot_id: &str) {
        let Some(_lease) = registry.hold_connection(robot_id, None).await else { return; };
        self.detach(robot_id).await;
    }

    /// Whether an authenticated device link currently owns a live MCP client.
    pub async fn is_attached(&self, robot_id: &str) -> bool {
        self.inner.read().await.contains_key(robot_id)
    }

    /// Exact authenticated connection currently owning the cached toolset.
    pub async fn connection_id(&self, robot_id: &str) -> Option<String> {
        self.inner
            .read()
            .await
            .get(robot_id)
            .map(|attached| attached.client.connection_id().to_owned())
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

    pub async fn tools_for_connection(&self, robot_id: &str, connection_id: &str) -> Option<Vec<RobotToolDescriptor>> {
        self.inner.read().await.get(robot_id)
            .filter(|attached| attached.client.connection_id() == connection_id)
            .map(|attached| attached.tools.clone())
    }

    /// A failed refresh retains the complete prior catalog. A late reply from
    /// an old connection can never replace its successor's cache.
    pub async fn refresh(&self, robot_id: &str, expected_connection_id: &str) -> Result<(), ToolCallError> {
        let prepared = self.prepare_refresh(robot_id, expected_connection_id).await?;
        self.lock_refresh_commit(prepared).await?.publish();
        Ok(())
    }

    pub async fn prepare_refresh(&self, robot_id: &str, expected_connection_id: &str) -> Result<PreparedRobotToolRefresh, ToolCallError> {
        let client = self.inner.read().await.get(robot_id)
            .filter(|attached| attached.client.connection_id() == expected_connection_id)
            .map(|attached| Arc::clone(&attached.client)).ok_or(ToolCallError::Offline)?;
        let tools = client.list_tools().await.map_err(|error| ToolCallError::Rejected(error.to_string()))?;
        Ok(PreparedRobotToolRefresh {robot_id:robot_id.to_owned(),client,tools})
    }

    pub async fn lock_refresh_commit(&self, prepared: PreparedRobotToolRefresh) -> Result<RobotToolRefreshCommit<'_>, ToolCallError> {
        let map = self.inner.write().await;
        if !map.get(&prepared.robot_id).is_some_and(|attached| Arc::ptr_eq(&attached.client,&prepared.client)) {
            return Err(ToolCallError::Offline);
        }
        Ok(RobotToolRefreshCommit {map,prepared})
    }

    /// Cached tools inside one selected Agent Action ceiling.
    pub async fn tools_for_action(
        &self,
        robot_id: &str,
        action: RobotAction,
    ) -> Vec<RobotToolDescriptor> {
        self.inner
            .read()
            .await
            .get(robot_id)
            .map(|attached| {
                attached
                    .tools
                    .iter()
                    .filter(|tool| tool_action(&tool.device_name) == action)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Invoke a tool only inside one selected Agent Action ceiling.
    pub async fn call_for_action(
        &self,
        robot_id: &str,
        action: RobotAction,
        exposed_name: &str,
        args: Value,
    ) -> Result<String, ToolCallError> {
        self.call_exact_for_action(robot_id, action, exposed_name, None, args)
            .await
    }

    /// Invoke the exact device tool frozen into a host-owned AgentSession.
    ///
    /// A later firmware reconnect may publish the same provider-safe exposed
    /// name for a different device-side tool. Session projection therefore
    /// freezes both names and this method checks them together under the same
    /// registry read lock used to select the live client.
    pub async fn call_exact_for_action(
        &self,
        robot_id: &str,
        action: RobotAction,
        exposed_name: &str,
        expected_device_name: Option<&str>,
        args: Value,
    ) -> Result<String, ToolCallError> {
        self.call_frozen_for_action(
            robot_id,
            action,
            exposed_name,
            expected_device_name,
            None,
            None,
            args,
        )
        .await
    }

    /// Session adapters freeze raw device schemas before provider hardening.
    /// Check identity and schema under the same lock that selects the client;
    /// a firmware reconnect cannot silently reinterpret a frozen invocation.
    #[allow(clippy::too_many_arguments)]
    pub async fn call_frozen_for_action(
        &self,
        robot_id: &str,
        action: RobotAction,
        exposed_name: &str,
        expected_device_name: Option<&str>,
        expected_connection_id: Option<&str>,
        expected_input_schema: Option<&Value>,
        args: Value,
    ) -> Result<String, ToolCallError> {
        self.call_with_profile(robot_id, action, exposed_name, expected_device_name, expected_connection_id,
            expected_input_schema, None, args).await
    }

    /// The application holds the RobotRegistry action lease while passing the
    /// authoritative live record. Editable classification can narrow tool use;
    /// it cannot bypass hardware categories or the device's permission switch.
    #[allow(clippy::too_many_arguments)]
    pub async fn call_configured_for_action(
        &self, robot_id: &str, action: RobotAction, exposed_name: &str,
        expected_device_name: Option<&str>, expected_connection_id: Option<&str>,
        expected_input_schema: Option<&Value>, record: &crate::registry::RobotRecord, args: Value,
    ) -> Result<String, ToolCallError> {
        if record.robot_id != robot_id || expected_device_name.is_none() || expected_connection_id.is_none() || expected_input_schema.is_none() {
            return Err(ToolCallError::Rejected("Configured dispatch requires an exact live Robot record and frozen tool contract".into()));
        }
        crate::capability_profile::validate_profile_shape(&record.capability_profile)
            .map_err(|error| ToolCallError::Rejected(error.to_string()))?;
        if !record.permissions.allows_configured_tool(&record.capability_profile, expected_device_name.unwrap()) {
            return Err(ToolCallError::Rejected("Configured tool is disabled or lacks device permission".into()));
        }
        self.call_with_profile(robot_id, action, exposed_name, expected_device_name, expected_connection_id,
            expected_input_schema, Some(&record.capability_profile), args).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn call_with_profile(
        &self, robot_id: &str, action: RobotAction, exposed_name: &str,
        expected_device_name: Option<&str>, expected_connection_id: Option<&str>,
        expected_input_schema: Option<&Value>, profile: Option<&crate::capability_profile::RobotCapabilityProfile>, args: Value,
    ) -> Result<String, ToolCallError> {
        let (client, device_name) = {
            let map = self.inner.read().await;
            let attached = map.get(robot_id).ok_or(ToolCallError::Offline)?;
            if expected_connection_id
                .is_some_and(|expected| attached.client.connection_id() != expected)
            {
                return Err(ToolCallError::Offline);
            }
            let tool = attached
                .tools
                .iter()
                .find(|tool| tool.exposed_name == exposed_name)
                .ok_or_else(|| ToolCallError::Rejected(format!("unknown tool {exposed_name}")))?;
            let actual = profile.map(|profile| crate::capability_profile::resolve_tool_action(profile, &tool.device_name))
                .unwrap_or_else(|| tool_action(&tool.device_name));
            if actual != action {
                return Err(ToolCallError::Rejected(format!(
                    "tool {exposed_name} belongs to {}, not {}",
                    actual.id(),
                    action.id(),
                )));
            }
            if let Some(expected_device_name) = expected_device_name {
                if tool.device_name != expected_device_name {
                    return Err(ToolCallError::Rejected(format!(
                        "tool {exposed_name} no longer resolves to the device tool frozen into this AgentSession"
                    )));
                }
            }
            if expected_input_schema.is_some_and(|schema| schema != &tool.input_schema) {
                return Err(ToolCallError::Rejected(format!(
                    "tool {exposed_name} schema differs from the frozen AgentSession contract"
                )));
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

    async fn request_payload(rx: &mut mpsc::Receiver<Frame>) -> serde_json::Value {
        let frame=rx.recv().await.unwrap();
        let Frame::Text(text)=frame else { panic!("expected MCP text frame") };
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["payload"].clone()
    }

    #[tokio::test]
    async fn refresh_failure_retains_cache_and_old_connection_cannot_replace_successor() {
        let tools=Arc::new(RobotToolRegistry::default());
        let (tx,mut rx)=mpsc::channel(8);
        let client=Arc::new(RobotMcpClient::new(tx,"old".into()));
        tools.attach("device",client.clone(),vec![descriptor("vendor.original")]).await;
        let task={ let tools=tools.clone(); tokio::spawn(async move { tools.refresh("device","old").await }) };
        let request=request_payload(&mut rx).await;
        assert_eq!(request["method"],"tools/list");
        client.handle_incoming(json!({"id":request["id"],"result":{"tools":[{"name":"vendor.partial","inputSchema":{"type":"object"}}],"nextCursor":"more"}})).await;
        let request=request_payload(&mut rx).await;
        client.handle_incoming(json!({"id":request["id"],"error":{"message":"unavailable"}})).await;
        assert!(task.await.unwrap().is_err());
        assert_eq!(tools.tools("device").await[0].device_name,"vendor.original");
        let task={ let tools=tools.clone(); tokio::spawn(async move { tools.refresh("device","old").await }) };
        let request=request_payload(&mut rx).await;
        let (replacement_tx,_replacement_rx)=mpsc::channel(4);
        tools.attach("device",Arc::new(RobotMcpClient::new(replacement_tx,"new".into())),vec![descriptor("vendor.successor")]).await;
        client.handle_incoming(json!({"id":request["id"],"result":{"tools":[{"name":"vendor.old_refresh","inputSchema":{"type":"object"}}]}})).await;
        assert!(matches!(task.await.unwrap(),Err(ToolCallError::Offline)));
        assert_eq!(tools.tools("device").await[0].device_name,"vendor.successor");
    }

    #[tokio::test]
    async fn successful_refresh_changes_catalog_and_frozen_old_schema_is_rejected() {
        let tools=Arc::new(RobotToolRegistry::default());
        let (tx,mut rx)=mpsc::channel(4);
        let client=Arc::new(RobotMcpClient::new(tx,"socket".into()));
        let original=descriptor("self.head.turn");
        tools.attach("device",client.clone(),vec![original.clone()]).await;
        let task={ let tools=tools.clone(); tokio::spawn(async move { tools.refresh("device","socket").await }) };
        let request=request_payload(&mut rx).await;
        client.handle_incoming(json!({"id":request["id"],"result":{"tools":[{"name":"self.head.turn","inputSchema":{"type":"object","properties":{"angle":{"type":"integer"}}}}]}})).await;
        task.await.unwrap().unwrap();
        assert_ne!(tools.tools("device").await[0].input_schema,original.input_schema);
        assert!(matches!(tools.call_frozen_for_action("device",RobotAction::Motion,&original.exposed_name,Some(&original.device_name),Some("socket"),Some(&original.input_schema),json!({})).await,Err(ToolCallError::Rejected(_))));
    }

    #[tokio::test]
    async fn configured_dispatch_uses_owner_mapping_and_device_permissions() {
        let tools=Arc::new(RobotToolRegistry::default());
        let (tx,mut rx)=mpsc::channel(4);
        let client=Arc::new(RobotMcpClient::new(tx,"socket".into()));
        let tool=descriptor("vendor.neck.turn");
        tools.attach("device",client.clone(),vec![tool.clone()]).await;
        let directory=tempfile::tempdir().unwrap();
        let registry=crate::registry::RobotRegistry::load(directory.path()).await.unwrap();
        let (mut record,_)=registry.upsert_on_report(crate::registry::RobotReport {robot_id:"device".into(),client_id:"client".into(),board:"test".into(),firmware_version:"1".into()},1).await.unwrap();
        record.capability_profile.tool_overrides.insert(tool.device_name.clone(),crate::capability_profile::RobotToolOverride { action:Some(RobotAction::Motion),..Default::default() });
        let denied=tools.call_configured_for_action("device",RobotAction::Motion,&tool.exposed_name,Some(&tool.device_name),Some("socket"),Some(&tool.input_schema),&record,json!({})).await;
        assert!(matches!(denied,Err(ToolCallError::Rejected(_))));
        record.permissions.motion=true;
        let task={let tools=tools.clone(); let tool=tool.clone(); tokio::spawn(async move {tools.call_configured_for_action("device",RobotAction::Motion,&tool.exposed_name,Some(&tool.device_name),Some("socket"),Some(&tool.input_schema),&record,json!({})).await})};
        let request=request_payload(&mut rx).await;
        assert_eq!(request["params"]["name"],"vendor.neck.turn");
        client.handle_incoming(json!({"id":request["id"],"result":{"content":[{"type":"text","text":"accepted"}]}})).await;
        assert!(task.await.unwrap().is_ok());
    }

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
    fn device_names_map_to_disjoint_action_ceilings() {
        for name in [
            "self.display.draw",
            "self.screen.clear",
            "self.oled.write",
            "self.emoji.set_expression",
            "self.face.set",
            "self.led.set",
        ] {
            assert_eq!(
                tool_action(name),
                RobotAction::Display,
                "{name}"
            );
        }
        for name in [
            "self.head.look",
            "self.head.express",
            "self.gimbal.turn",
            "self.motion.stop",
            "self.servo.calibrate",
        ] {
            assert_eq!(tool_action(name), RobotAction::Motion, "{name}");
        }
        for name in ["self.camera.take_photo", "self.vision.describe"] {
            assert_eq!(tool_action(name), RobotAction::Vision, "{name}");
        }
        for name in ["self.audio_speaker.set_volume", "get_device_status"] {
            assert_eq!(
                tool_action(name),
                RobotAction::Device,
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn facial_expression_with_head_motion_cannot_use_display_authority() {
        let registry = registry().await;
        let (tx, _rx) = mpsc::channel::<Frame>(4);
        registry.attach("expression-robot", Arc::new(RobotMcpClient::new(tx, "expression-socket".into())),
            vec![descriptor("self.head.express")]).await;
        assert!(registry.tools_for_action("expression-robot", RobotAction::Display).await.is_empty());
        assert_eq!(registry.tools_for_action("expression-robot", RobotAction::Motion).await.len(), 1);
        let result = registry.call_for_action("expression-robot", RobotAction::Display,
            "robot_head_express", json!({"emotion":"happy","action":"nod"})).await;
        assert!(matches!(result, Err(ToolCallError::Rejected(_))));
    }

    #[tokio::test]
    async fn discovery_and_dispatch_respect_the_selected_action_ceiling() {
        let registry = registry().await;
        assert!(registry.is_attached("robot-1").await);
        let display = registry
            .tools_for_action("robot-1", RobotAction::Display)
            .await;
        assert_eq!(display.len(), 1);
        assert_eq!(display[0].device_name, "self.emoji.set_expression");

        let error = registry
            .call_for_action(
                "robot-1",
                RobotAction::Display,
                "robot_head_look",
                json!({}),
            )
            .await
            .expect_err("display authority must not dispatch a motion tool");
        assert!(matches!(error, ToolCallError::Rejected(_)));
        assert!(error.to_string().contains("robot/motion"));
        assert!(error.to_string().contains("robot/display"));
    }

    #[tokio::test]
    async fn detach_clears_the_live_link() {
        let registry = registry().await;
        registry.detach("robot-1").await;
        assert!(!registry.is_attached("robot-1").await);
        assert!(
            registry
                .call_for_action(
                    "robot-1",
                    RobotAction::Device,
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
            .call_exact_for_action(
                "robot-1",
                RobotAction::Display,
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
