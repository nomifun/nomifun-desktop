//! Subtractive Session policy, independent of Agent identity and Engine tool names.
use nomifun_common::AgentToolPolicy;
use serde::{Deserialize, Serialize};

pub const EXECUTION_CONSTRAINTS_KEY: &str = "execution_constraints";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConstraints {
    pub version: u32,
    /// Reuse the existing Attempt policy. ReadShell is not an OS read-only sandbox.
    pub tool_scope: AgentToolPolicy,
    pub exclude_delegation: bool,
    /// Further narrows a Full attempt to the managed process lifecycle. This
    /// is projected from the persisted planner profile and cannot add tools.
    #[serde(default)]
    pub managed_process_only: bool,
}

impl Default for ExecutionConstraints {
    fn default() -> Self {
        Self {
            version: 1,
            tool_scope: AgentToolPolicy::Full,
            exclude_delegation: false,
            managed_process_only: false,
        }
    }
}

impl ExecutionConstraints {
    pub fn from_extra(extra: &serde_json::Value) -> Result<Self, nomifun_common::AppError> {
        let value = match extra.get(EXECUTION_CONSTRAINTS_KEY) {
            Some(value) => serde_json::from_value::<Self>(value.clone()).map_err(|error| {
                nomifun_common::AppError::Conflict(format!(
                    "Invalid execution constraints: {error}"
                ))
            })?,
            None => Self::default(),
        };
        if value.version != 1 {
            return Err(nomifun_common::AppError::Conflict(
                "Unsupported execution constraints version".into(),
            ));
        }
        Ok(value)
    }

    pub fn restricted(&self) -> bool {
        self.tool_scope != AgentToolPolicy::Full || self.managed_process_only
    }

    pub fn instruction(&self) -> Option<&'static str> {
        if self.managed_process_only {
            return Some(
                "This execution Attempt is restricted to one managed process lifecycle. Only start_process, poll_process, write_process_stdin, close_process_stdin, resize_process, and cancel_process are available from workspace tools. Do not inspect files, run exec_command/Bash, probe the directory, or substitute raw PIDs or shell backgrounding. Use the exact process_id returned by start_process and prove cleanup before completion.",
            );
        }
        match (self.tool_scope, self.exclude_delegation) {
            (AgentToolPolicy::Full, false) => None,
            (AgentToolPolicy::Full, true) => Some(
                "This execution Attempt cannot delegate to another Agent. Work with the remaining selected capabilities; do not route delegation through another tool.",
            ),
            (AgentToolPolicy::ReadOnly, _) => Some(
                "This execution Attempt is read-only: only the Agent's selected file read/search tools and fixed context resources are available. Do not modify files, start processes, delegate, or contact external tools. If the task needs an unavailable capability, report the limitation.",
            ),
            (AgentToolPolicy::ReadShell, _) => Some(
                "This execution Attempt permits selected file read/search and process execution, but no other external tools or delegation. Shell execution is not a read-only sandbox and can modify the workspace; follow the task's requested scope. Report unavailable capabilities rather than substituting an unrelated route.",
            ),
        }
    }

    /// Nomi's persistent native registration ceiling; never adds a grant.
    pub fn allows_nomi_tool(&self, name: &str) -> bool {
        if self.exclude_delegation
            && matches!(name, "nomi_delegate" | "subagent_send" | "subagent_wait")
        {
            return false;
        }
        if self.managed_process_only {
            return matches!(
                name,
                "start_process"
                    | "poll_process"
                    | "write_process_stdin"
                    | "close_process_stdin"
                    | "resize_process"
                    | "cancel_process"
            );
        }
        match self.tool_scope {
            AgentToolPolicy::Full => true,
            AgentToolPolicy::ReadOnly => matches!(
                name,
                "Read" | "Grep" | "Glob" | "ToolSearch" | "activate_vision_input"
            ),
            AgentToolPolicy::ReadShell => matches!(
                name,
                "Read"
                    | "Grep"
                    | "Glob"
                    | "ToolSearch"
                    | "activate_vision_input"
                    | "Bash"
                    | "exec_command"
                    | "write_stdin"
            ),
        }
    }

    pub fn allows_workspace_action(&self, capability_id: &str, action_id: &str) -> bool {
        if self.managed_process_only {
            return capability_id == "workspace.process"
                && matches!(
                    action_id,
                    "workspace.process/start"
                        | "workspace.process/poll"
                        | "workspace.process/input"
                        | "workspace.process/close_stdin"
                        | "workspace.process/resize"
                        | "workspace.process/cancel"
                );
        }
        match capability_id {
            "workspace.files" => match self.tool_scope {
                AgentToolPolicy::Full => true,
                AgentToolPolicy::ReadOnly | AgentToolPolicy::ReadShell => matches!(
                    action_id,
                    "workspace.files/read" | "workspace.files/search"
                ),
            },
            "workspace.process" => match self.tool_scope {
                AgentToolPolicy::Full => true,
                AgentToolPolicy::ReadShell => action_id == "workspace.process/exec",
                AgentToolPolicy::ReadOnly => false,
            },
            "workspace.vcs" | "workspace.artifacts" => {
                self.tool_scope == AgentToolPolicy::Full
            }
            _ => self.tool_scope == AgentToolPolicy::Full,
        }
    }
}
