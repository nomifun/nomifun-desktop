use std::collections::HashMap;

use nomifun_common::{CompanionId, DelegationPolicy, UserId};
use serde::{Deserialize, Serialize};

use crate::{GatewayMcpConfig, KnowledgeMountInfo, McpServerId};

macro_rules! optional_id_deserializer {
    ($name:ident, $id:ty) => {
        fn $name<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let value = Option::<String>::deserialize(deserializer)?;
            value
                .map(|value| {
                    <$id>::parse(value.clone())
                        .map(|_| value)
                        .map_err(serde::de::Error::custom)
                })
                .transpose()
        }
    };
}

optional_id_deserializer!(deserialize_companion_id, CompanionId);
optional_id_deserializer!(deserialize_user_id, UserId);

fn deserialize_required_companion_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    CompanionId::parse(value.clone())
        .map(|_| value)
        .map_err(serde::de::Error::custom)
}

/// In-session companion summon marker (spec §设计 B), stored at
/// `conversation.extra.summon` on ordinary work conversations.
///
/// The nomi factory reads it (via [`NomiBuildExtra::summon`]) to materialize
/// the companion's active skills, register the read-only
/// `recall_memories` / `propose_companion_memory` tools and inject the live
/// memory-snapshot context section. The persona is never taken over and
/// `save_memory` is never registered for a summoned work session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummonConfig {
    /// The summoned companion (canonical UUIDv7). Required.
    #[serde(deserialize_with = "deserialize_required_companion_id")]
    pub companion_id: String,
    /// Hand-picked memory ids, re-resolved live each turn under the
    /// snapshot budget (edits to a memory naturally propagate).
    #[serde(default)]
    pub memory_ids: Vec<String>,
    /// Companion skills excluded from materialization (subtractive; the
    /// default is every active skill).
    #[serde(default)]
    pub skill_exclusions: Vec<String>,
    /// Server-stamped epoch milliseconds. Required — clients never set it.
    pub summoned_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionMcpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMcpServer {
    pub mcp_server_id: McpServerId,
    pub name: String,
    pub transport: SessionMcpTransport,
}

/// Opt-in goal-driven continuation for a session. When present, the engine
/// keeps working toward `objective` across turns (with a completion audit)
/// until the model proves completion, hits `max_auto_continuations`, or
/// `max_turns`. Absent (the default) = normal one-shot turn behavior.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NomiGoalSpec {
    pub objective: String,
    /// Cap on automatic continuations (anti-runaway). Defaults to 8 when unset.
    #[serde(default)]
    pub max_auto_continuations: Option<usize>,
}

/// Nomi-specific fields extracted from `extra` in build runtime options.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NomiBuildExtra {
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub preset_rules: Option<String>,
    #[serde(default)]
    pub max_turns: Option<usize>,
    /// Opt-in goal-driven continuation (see [`NomiGoalSpec`]).
    #[serde(default)]
    pub goal: Option<NomiGoalSpec>,
    /// Stable MCP server business IDs.
    pub mcp_server_ids: Option<Vec<McpServerId>>,
    #[serde(default)]
    pub session_mcp_servers: Vec<SessionMcpServer>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default, deserialize_with = "deserialize_user_id")]
    pub user_id: Option<String>,
    /// Marks a companion conversation: the factory registers its memory tools
    /// (recall/save memory, recent events) and skips unrelated Guide capabilities.
    #[serde(default, rename = "companion_session")]
    pub companion: bool,
    /// When set, this Nomi conversation is bound to a saved SSH host: the remote
    /// tool family operates that host instead of the local machine. References
    /// `ssh_hosts.ssh_host_id` (see the id-schema logical reference). Credentials
    /// are never here — only the host id.
    #[serde(default)]
    pub ssh_host_id: Option<String>,
    /// Remote working directory the agent's shell starts in for an SSH session.
    /// Defaults to the remote `$HOME` when absent. Distinct from `workspace`,
    /// which stays a LOCAL scratch path (skill/knowledge plumbing assumes local).
    #[serde(default)]
    pub ssh_remote_cwd: Option<String>,
    /// Opt-in to the Computer tool (screen/mouse/keyboard control) for this
    /// session. Falls back to host config / NOMIFUN_COMPUTER_USE when None.
    #[serde(default)]
    pub computer_use: Option<bool>,
    /// Opt-in to the Browser tool (CDP automation) for this session.
    /// Falls back to host config / NOMIFUN_BROWSER_USE when None.
    #[serde(default)]
    pub browser_use: Option<bool>,
    /// Platform Gateway MCP stdio bridge config, injected only from
    /// process-owned factory dependencies after authority resolution.
    #[serde(skip)]
    pub gateway_mcp_config: Option<GatewayMcpConfig>,
    /// Exact Platform Gateway tools omitted from this session's MCP tools/list.
    /// This is a subtractive runtime capability fence, never a grant.
    #[serde(default)]
    pub gateway_excluded_tools: Vec<String>,
    /// IM platform this conversation serves (e.g. "telegram", "lark"), set by
    /// the channel layer on Channel Agent sessions. Consumed by the companion
    /// prompt provider so the persona can acknowledge the remote context.
    #[serde(default)]
    pub channel_platform: Option<String>,
    /// Marks a dedicated external-channel conversation whose sender was
    /// admitted automatically by an `all_members` group policy rather than by
    /// explicit pairing approval. The Nomi factory treats this as a strictly
    /// subtractive authority marker and applies the model-only ceiling even
    /// though channel conversations are physically owned by the installation
    /// owner. It never grants a capability.
    #[serde(default)]
    pub channel_group_guest: bool,
    /// The companion this session is bound to (multi-companion upgrade). Set by the
    /// channel layer on Channel Agent sessions (platform binding > default
    /// companion) and consumed by the companion prompt provider to pick the
    /// persona; it is also bound into the signed Gateway child capability.
    /// `None` means there is no companion binding.
    #[serde(default, deserialize_with = "deserialize_companion_id")]
    pub companion_id: Option<String>,
    /// Knowledge bases mounted into this session's workspace, computed when
    /// the Agent runtime is created. The Nomi factory renders
    /// these into a system-prompt section so the agent knows what extended
    /// knowledge is available and where it lives.
    #[serde(default)]
    pub knowledge_mounts: Vec<KnowledgeMountInfo>,
    /// Write-back ("回血") switch: `true` invites the agent to persist new
    /// knowledge as markdown into the mounted directories; `false` declares
    /// them read-only. Prompt-level contract — the mounts themselves stay
    /// writable on disk.
    #[serde(default)]
    pub knowledge_writeback: bool,
    /// Write-back disposition ("回写意识") while `knowledge_writeback` is true:
    /// `manual` (the default) or `auto`. It is the only write-back knob —
    /// placement is always the base body.
    #[serde(default)]
    pub knowledge_writeback_eagerness: Option<String>,
    /// Opt-in for unattended IM-channel (bot) sessions to write back. Off by
    /// default. The nomi factory reconstructs
    /// the knowledge binding from this build-extra to resolve the per-surface
    /// write policy, so this MUST be threaded through — otherwise the
    /// reconstructed binding defaults it to `false` and `WriteSurface::ExternalChannel`
    /// is permanently `Disabled`.
    #[serde(default)]
    pub knowledge_channel_write_enabled: bool,
    /// Per-session 工具白名单（受限的持久执行 Agent 使用）。非空时引擎只保留
    /// 名单内的工具（bootstrap `retain_named`）。执行层在创建 Agent attempt
    /// conversation 时设置；普通会话恒空 = 不限制。
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Conversation-level delegation intent. This shapes when the Agent uses
    /// the unified persistent execution tools; it never grants tool authority.
    /// The factory always overwrites this from the typed runtime build option;
    /// a same-named value in open-ended JSON is never authoritative.
    #[serde(default = "default_delegation_policy")]
    pub delegation_policy: DelegationPolicy,
    /// In-session companion summon (spec §设计 B): skills + selected memories
    /// of one companion loaded read-only into an ordinary work conversation.
    /// `None` = not summoned (today's behavior, zero regression).
    #[serde(default)]
    pub summon: Option<SummonConfig>,
}

fn default_delegation_policy() -> DelegationPolicy {
    DelegationPolicy::Automatic
}

/// A slash command item available in a conversation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommandItem {
    pub command: String,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MCP_SERVER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000123";
    const COMPANION_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";

    #[test]
    fn summon_config_roundtrips_through_serde() {
        let config = SummonConfig {
            companion_id: COMPANION_ID.into(),
            memory_ids: vec!["0190f5fe-7c00-7a00-8abc-000000000002".into()],
            skill_exclusions: vec!["heavy-refactor".into()],
            summoned_at: 1_722_000_000_000,
        };
        let json = serde_json::to_value(&config).unwrap();
        let parsed: SummonConfig = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn summon_config_lists_default_empty_but_identity_fields_are_required() {
        let parsed: SummonConfig = serde_json::from_value(serde_json::json!({
            "companion_id": COMPANION_ID,
            "summoned_at": 1,
        }))
        .unwrap();
        assert!(parsed.memory_ids.is_empty());
        assert!(parsed.skill_exclusions.is_empty());

        for invalid in [
            // companion_id missing entirely
            serde_json::json!({ "summoned_at": 1 }),
            // summoned_at missing (server must stamp it before persistence)
            serde_json::json!({ "companion_id": COMPANION_ID }),
            // companion_id not a canonical UUIDv7
            serde_json::json!({ "companion_id": "not-an-id", "summoned_at": 1 }),
            // unknown fields are rejected (extra.summon is a closed contract)
            serde_json::json!({
                "companion_id": COMPANION_ID,
                "summoned_at": 1,
                "persona_takeover": true,
            }),
        ] {
            assert!(
                serde_json::from_value::<SummonConfig>(invalid.clone()).is_err(),
                "must reject {invalid}"
            );
        }
    }

    #[test]
    fn nomi_build_extra_surfaces_summon_and_defaults_none() {
        let extra: NomiBuildExtra = serde_json::from_value(serde_json::json!({
            "summon": {
                "companion_id": COMPANION_ID,
                "memory_ids": [],
                "skill_exclusions": ["x"],
                "summoned_at": 42,
            }
        }))
        .unwrap();
        let summon = extra.summon.expect("summon must parse");
        assert_eq!(summon.companion_id, COMPANION_ID);
        assert_eq!(summon.skill_exclusions, vec!["x".to_owned()]);
        assert_eq!(summon.summoned_at, 42);

        let plain: NomiBuildExtra = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(plain.summon.is_none(), "absent summon must stay None");
    }

    #[test]
    fn nomi_build_extra_deserializes_delegation_policy() {
        let extra: NomiBuildExtra =
            serde_json::from_value(serde_json::json!({ "delegation_policy": "prefer_parallel" }))
                .unwrap();
        assert_eq!(extra.delegation_policy, DelegationPolicy::PreferParallel);
    }

    #[test]
    fn nomi_build_extra_delegation_defaults_automatic() {
        let extra: NomiBuildExtra = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(extra.delegation_policy, DelegationPolicy::Automatic);
        assert_eq!(
            NomiBuildExtra::default().delegation_policy,
            DelegationPolicy::Automatic
        );
    }

    #[test]
    fn nomi_build_extra_group_guest_marker_defaults_false_and_parses_true() {
        let plain: NomiBuildExtra = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!plain.channel_group_guest);
        assert!(!NomiBuildExtra::default().channel_group_guest);

        let guest: NomiBuildExtra = serde_json::from_value(serde_json::json!({
            "channel_group_guest": true,
            "channel_platform": "lark"
        }))
        .unwrap();
        assert!(guest.channel_group_guest);
        assert_eq!(guest.channel_platform.as_deref(), Some("lark"));
    }

    #[test]
    fn session_mcp_server_id_accepts_canonical_uuidv7() {
        let value = serde_json::json!({
            "mcp_server_id": MCP_SERVER_ID,
            "name": "temporary",
            "transport": { "type": "stdio", "command": "server" }
        });
        let parsed: SessionMcpServer = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.mcp_server_id.as_str(), MCP_SERVER_ID);
    }

    #[test]
    fn session_mcp_server_rejects_legacy_id() {
        let value = serde_json::json!({
            "id": 42,
            "name": "temporary",
            "transport": { "type": "stdio", "command": "server" }
        });
        assert!(serde_json::from_value::<SessionMcpServer>(value).is_err());
    }

    #[test]
    fn catalog_mcp_ids_require_canonical_uuidv7_strings() {
        let id = McpServerId::parse(MCP_SERVER_ID).unwrap();
        let parsed: NomiBuildExtra =
            serde_json::from_value(serde_json::json!({ "mcp_server_ids": [id.clone()] })).unwrap();
        assert_eq!(parsed.mcp_server_ids, Some(vec![id]));

        for invalid in [
            serde_json::json!([42]),
            serde_json::json!(["42"]),
            serde_json::json!(["550e8400-e29b-41d4-a716-446655440000"]),
            serde_json::json!([format!("mcp_{MCP_SERVER_ID}")]),
            serde_json::json!([true]),
        ] {
            assert!(
                serde_json::from_value::<NomiBuildExtra>(
                    serde_json::json!({ "mcp_server_ids": invalid })
                )
                .is_err()
            );
        }
    }
}
