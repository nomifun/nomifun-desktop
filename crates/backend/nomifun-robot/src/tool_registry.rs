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
        self.inner.write().await.remove(robot_id);
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
}
