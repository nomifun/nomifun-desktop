//! MCP stdio bridge capability contracts shared across the backend.
//!
//! Requirement, knowledge, and Gateway bridges use a two-stage contract: an
//! opaque, non-serializable issuer config stays in the backend process, while
//! each child receives only a short-lived signed capability. Stateless bridge
//! configs (`OpenMcpConfig`, `ComputerMcpConfig`) and the process-private
//! browser issuer config live here too so downstream crates
//! (`nomifun-ai-agent` deserializing `AcpBuildExtra`, etc.) can reference the
//! same shape from a leaf crate.

use std::fmt;
use std::sync::Arc;

use nomifun_common::{
    CompanionId, ConversationId, KnowledgeBaseId, LoopbackCapabilityAccess,
    LoopbackCapabilityClaims, LoopbackCapabilityError, LoopbackCapabilityIssuer,
    LoopbackCapabilityLease, LoopbackCapabilityRenewalRequest, LoopbackSessionBinding,
    LoopbackSessionKind, TerminalId, generate_id, validate_uuidv7,
};
use serde::{Deserialize, Serialize};

pub const REQUIREMENT_CAPABILITY_DOMAIN: &str = "nomifun-requirement-mcp-v2";
pub const KNOWLEDGE_CAPABILITY_DOMAIN: &str = "nomifun-knowledge-mcp-v2";
/// Signed Requirement MCP authorization contract.
///
/// Version 2 deliberately binds a reusable child to its owner session, not to
/// one numeric claim generation: ACP runtimes and terminal PTYs can predate a
/// claim and survive across claims. Every mutating request under this contract
/// must instead carry a canonical requirement id, a positive
/// `claim_generation`, and that generation's unguessable 256-bit
/// `claim_token`. The server verifies the current owner/claim before resolving
/// it with a database compare-and-set over all three authority fields. Keeping
/// the contract version in the signed scope makes pre-token capabilities fail
/// closed.
pub const REQUIREMENT_EXACT_CLAIM_CONTRACT_VERSION: u8 = 2;
pub const BROWSER_CAPABILITY_DOMAIN: &str = "nomifun-browser-mcp-v1";
/// Structural safety fuse for concurrently retained Browser MCP runtimes in
/// one user-visible task family. This is not a process-global concurrency cap:
/// each `(user, conversation)` receives an independent allowance.
pub const MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY: usize = 16;

pub const REQUIREMENT_COMPLETE_TOOL: &str = "requirement_complete";
pub const REQUIREMENT_UPDATE_STATUS_TOOL: &str = "requirement_update_status";
pub const KNOWLEDGE_SEARCH_TOOL: &str = "knowledge_search";
pub const KNOWLEDGE_READ_TOOL: &str = "knowledge_read";
pub const KNOWLEDGE_WRITE_TOOL: &str = "knowledge_write";

/// Requirement ownership resolved by the backend before the child starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementCapabilityScope {
    pub owner_kind: LoopbackSessionKind,
    pub owner_session_id: String,
    pub verdict_contract_version: u8,
    pub requires_opaque_claim_token: bool,
}

impl RequirementCapabilityScope {
    pub fn validate(
        &self,
        session: &LoopbackSessionBinding,
    ) -> Result<(), LoopbackCapabilityError> {
        let typed_id_is_valid = match self.owner_kind {
            LoopbackSessionKind::Conversation => {
                ConversationId::try_from(self.owner_session_id.as_str()).is_ok()
            }
            LoopbackSessionKind::Terminal => {
                TerminalId::try_from(self.owner_session_id.as_str()).is_ok()
            }
            LoopbackSessionKind::ExternalProcess => false,
        };
        if !typed_id_is_valid
            || self.owner_kind != session.kind
            || self.owner_session_id != session.session_id
            || self.verdict_contract_version != REQUIREMENT_EXACT_CLAIM_CONTRACT_VERSION
            || !self.requires_opaque_claim_token
        {
            return Err(LoopbackCapabilityError::InvalidIdentity);
        }
        Ok(())
    }
}

pub type RequirementCapabilityClaims = LoopbackCapabilityClaims<RequirementCapabilityScope>;

/// Knowledge scope resolved from persisted mounts and the authoritative
/// workspace. The child cannot add ids, switch cwd, or enable writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeCapabilityScope {
    pub workspace_path: String,
    pub kb_ids: Vec<KnowledgeBaseId>,
}

impl KnowledgeCapabilityScope {
    pub fn validate(&self) -> Result<(), LoopbackCapabilityError> {
        if self.workspace_path.is_empty()
            || self.workspace_path.trim() != self.workspace_path
            || self.kb_ids.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(LoopbackCapabilityError::InvalidIdentity);
        }
        Ok(())
    }
}

pub type KnowledgeCapabilityClaims = LoopbackCapabilityClaims<KnowledgeCapabilityScope>;

/// The one JSON bootstrap passed to a bridge child. It contains short-lived
/// access plus a process-scoped renewal proof for exactly the same immutable
/// authorization; neither credential is the backend root issuer secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedMcpChildBootstrap<C> {
    pub port: u16,
    pub access: LoopbackCapabilityAccess<C>,
    pub renewal: LoopbackCapabilityRenewalRequest,
}

/// Main-process result of issuing one bridge capability. Only `bootstrap` is
/// serialized into the child env; `lease` stays in the runtime/PTY lifecycle
/// so teardown can revoke independently of child cleanup.
#[derive(Clone)]
pub struct ScopedMcpChildConfig<C> {
    pub bootstrap: ScopedMcpChildBootstrap<C>,
    pub binary_path: String,
    pub lease: LoopbackCapabilityLease,
}

impl<C: fmt::Debug> fmt::Debug for ScopedMcpChildConfig<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedMcpChildConfig")
            .field("bootstrap", &self.bootstrap)
            .field("binary_path", &self.binary_path)
            .field("lease", &self.lease)
            .finish()
    }
}

impl<C: Serialize> ScopedMcpChildConfig<C> {
    pub fn bootstrap_json(&self) -> Result<String, LoopbackCapabilityError> {
        serde_json::to_string(&self.bootstrap).map_err(|_| LoopbackCapabilityError::Malformed)
    }
}

pub type RequirementMcpChildConfig = ScopedMcpChildConfig<RequirementCapabilityClaims>;
pub type KnowledgeMcpChildConfig = ScopedMcpChildConfig<KnowledgeCapabilityClaims>;

/// Backend-private Requirement MCP issuer. This type intentionally does not
/// implement `Serialize`/`Deserialize`, and its secret is private + redacted.
#[derive(Clone)]
pub struct RequirementMcpConfig {
    port: u16,
    issuer: Arc<LoopbackCapabilityIssuer>,
    pub binary_path: String,
}

impl fmt::Debug for RequirementMcpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequirementMcpConfig")
            .field("port", &self.port)
            .field("issuer", &"[REDACTED]")
            .field("binary_path", &self.binary_path)
            .finish()
    }
}

impl RequirementMcpConfig {
    pub fn from_issuer(
        port: u16,
        issuer: Arc<LoopbackCapabilityIssuer>,
        binary_path: String,
    ) -> Self {
        Self {
            port,
            issuer,
            binary_path,
        }
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Wire-level MCP server name. Kept short so the longest wire-level tool
    /// name `mcp__nomifun-requirement__requirement_update_status` (51 chars)
    /// stays within Anthropic's 64-char tool-name limit (see ELECTRON-1JY).
    pub const SERVER_NAME: &'static str = "nomifun-requirement";
    /// Single child bootstrap env. There are no independently mutable
    /// port/token/identity variables and no legacy compatibility reader.
    pub const ENV_CAPABILITY: &'static str = "NOMI_REQ_MCP_CAPABILITY";

    pub fn issue_for_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<RequirementMcpChildConfig, LoopbackCapabilityError> {
        self.issue(
            user_id,
            LoopbackSessionBinding::conversation(conversation_id),
            conversation_id,
        )
    }

    pub fn issue_for_terminal(
        &self,
        user_id: &str,
        terminal_id: &str,
    ) -> Result<RequirementMcpChildConfig, LoopbackCapabilityError> {
        self.issue(
            user_id,
            LoopbackSessionBinding::terminal(terminal_id),
            terminal_id,
        )
    }

    fn issue(
        &self,
        user_id: &str,
        session: LoopbackSessionBinding,
        owner_session_id: &str,
    ) -> Result<RequirementMcpChildConfig, LoopbackCapabilityError> {
        let scope = RequirementCapabilityScope {
            owner_kind: session.kind,
            owner_session_id: owner_session_id.to_string(),
            verdict_contract_version: REQUIREMENT_EXACT_CLAIM_CONTRACT_VERSION,
            requires_opaque_claim_token: true,
        };
        scope.validate(&session)?;
        let claims = RequirementCapabilityClaims::issue(
            user_id,
            session,
            [REQUIREMENT_COMPLETE_TOOL, REQUIREMENT_UPDATE_STATUS_TOOL],
            scope,
        )?;
        let (token, renewal_proof) = self
            .issuer
            .activate(REQUIREMENT_CAPABILITY_DOMAIN, &claims)?;
        let lease = LoopbackCapabilityLease::new(
            self.issuer.clone(),
            REQUIREMENT_CAPABILITY_DOMAIN,
            claims.lease_id.clone(),
        );
        Ok(ScopedMcpChildConfig {
            bootstrap: ScopedMcpChildBootstrap {
                port: self.port,
                renewal: LoopbackCapabilityRenewalRequest {
                    lease_id: claims.lease_id.clone(),
                    renewal_proof,
                },
                access: LoopbackCapabilityAccess { token, claims },
            },
            binary_path: self.binary_path.clone(),
            lease,
        })
    }
}

/// Backend-private Knowledge MCP issuer. Like the requirement issuer, it can
/// only be used inside the main process and is skipped by build-extra serde.
#[derive(Clone)]
pub struct KnowledgeMcpConfig {
    port: u16,
    issuer: Arc<LoopbackCapabilityIssuer>,
    pub binary_path: String,
}

impl fmt::Debug for KnowledgeMcpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KnowledgeMcpConfig")
            .field("port", &self.port)
            .field("issuer", &"[REDACTED]")
            .field("binary_path", &self.binary_path)
            .finish()
    }
}

impl KnowledgeMcpConfig {
    pub fn from_issuer(
        port: u16,
        issuer: Arc<LoopbackCapabilityIssuer>,
        binary_path: String,
    ) -> Self {
        Self {
            port,
            issuer,
            binary_path,
        }
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    pub const SERVER_NAME: &'static str = "nomifun-knowledge";
    pub const ENV_CAPABILITY: &'static str = "NOMI_KB_MCP_CAPABILITY";

    pub fn issue_for_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
        workspace_path: &str,
        kb_ids: &[KnowledgeBaseId],
        allow_write: bool,
    ) -> Result<KnowledgeMcpChildConfig, LoopbackCapabilityError> {
        self.issue(
            user_id,
            LoopbackSessionBinding::conversation(conversation_id),
            workspace_path,
            kb_ids,
            allow_write,
        )
    }

    /// Issue a terminal-session capability. Unlike conversation/external
    /// issuance there is no `allow_write` switch: the signed tool list always
    /// carries all three knowledge tools, because a terminal's write authority
    /// is resolved LIVE from its workpath binding at every dispatch (the
    /// binding can change while the PTY runs, and the frozen claims of an
    /// already-launched CLI child cannot be re-issued without a relaunch).
    /// The dispatch layer fails writes closed when the live policy is
    /// disabled, in both directions — enabling AND revoking take effect
    /// immediately.
    ///
    /// Deliberate trade-off: this makes the capability in the PTY environment
    /// a STANDING credential whose effective authority grows when the user
    /// later binds bases or enables write-back — without a re-issuance event.
    /// Anything that can read the terminal's process environment could hold
    /// the token until PTY exit and use whatever the live binding then
    /// allows. Accepted because the alternative (freeze + relaunch) is
    /// exactly the incident this design fixed; the token never leaves the
    /// loopback + signed-claims trust boundary, and server-side policy is the
    /// single enforcement point.
    pub fn issue_for_terminal(
        &self,
        user_id: &str,
        terminal_id: &str,
        workspace_path: &str,
        kb_ids: &[KnowledgeBaseId],
    ) -> Result<KnowledgeMcpChildConfig, LoopbackCapabilityError> {
        self.issue(
            user_id,
            LoopbackSessionBinding::terminal(terminal_id),
            workspace_path,
            kb_ids,
            true,
        )
    }

    /// Issue a broker-owned capability for an authenticated external process.
    /// All identity and scope inputs must already have been resolved by the
    /// main process; the stdio client never supplies them.
    pub fn issue_for_external_process(
        &self,
        installation_owner_id: &str,
        process_session_id: &str,
        workspace_path: &str,
        kb_ids: &[KnowledgeBaseId],
        allow_write: bool,
    ) -> Result<KnowledgeMcpChildConfig, LoopbackCapabilityError> {
        self.issue(
            installation_owner_id,
            LoopbackSessionBinding::external_process(process_session_id),
            workspace_path,
            kb_ids,
            allow_write,
        )
    }

    fn issue(
        &self,
        user_id: &str,
        session: LoopbackSessionBinding,
        workspace_path: &str,
        kb_ids: &[KnowledgeBaseId],
        allow_write: bool,
    ) -> Result<KnowledgeMcpChildConfig, LoopbackCapabilityError> {
        let mut kb_ids = kb_ids.to_vec();
        kb_ids.sort();
        kb_ids.dedup();
        let scope = KnowledgeCapabilityScope {
            workspace_path: workspace_path.to_owned(),
            kb_ids,
        };
        scope.validate()?;
        let mut tools = vec![KNOWLEDGE_SEARCH_TOOL, KNOWLEDGE_READ_TOOL];
        if allow_write {
            tools.push(KNOWLEDGE_WRITE_TOOL);
        }
        let claims = KnowledgeCapabilityClaims::issue(user_id, session, tools, scope)?;
        let (token, renewal_proof) = self.issuer.activate(KNOWLEDGE_CAPABILITY_DOMAIN, &claims)?;
        let lease = LoopbackCapabilityLease::new(
            self.issuer.clone(),
            KNOWLEDGE_CAPABILITY_DOMAIN,
            claims.lease_id.clone(),
        );
        Ok(ScopedMcpChildConfig {
            bootstrap: ScopedMcpChildBootstrap {
                port: self.port,
                renewal: LoopbackCapabilityRenewalRequest {
                    lease_id: claims.lease_id.clone(),
                    renewal_proof,
                },
                access: LoopbackCapabilityAccess { token, claims },
            },
            binary_path: self.binary_path.clone(),
            lease,
        })
    }
}

pub const GATEWAY_CAPABILITY_DOMAIN: &str = "nomifun-gateway-mcp-v2";
pub const GATEWAY_LIST_TOOLS_OPERATION: &str = "tools/list";
pub const GATEWAY_CALL_TOOL_OPERATION: &str = "tools/call";
/// Top-level Conversation creation is a companion capability, not a capability
/// of an ordinary Conversation. User-driven creation enters through the
/// authenticated Conversation REST route; scheduled and Agent Execution
/// creation use their dedicated backend services.
pub const GATEWAY_CREATE_CONVERSATION_TOOL: &str = "nomi_create_conversation";

/// Gateway-specific authorization surface inside the common loopback envelope.
/// User and Conversation identity live once in the common claims; this scope
/// contains only the Gateway projection and attribution policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayCapabilityScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion_id: Option<CompanionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_mode: Option<String>,
    pub profile: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_tools: Vec<String>,
    pub instance_owner: bool,
}

impl GatewayCapabilityScope {
    pub fn validate(&self) -> Result<(), LoopbackCapabilityError> {
        fn canonical(value: &str) -> bool {
            !value.is_empty() && value.trim() == value
        }
        fn canonical_optional(value: Option<&str>) -> bool {
            value.is_none_or(canonical)
        }

        if !canonical_optional(self.channel_platform.as_deref())
            || !canonical_optional(self.session_mode.as_deref())
            || !canonical(&self.profile)
            || !GatewayMcpConfig::is_known_profile(&self.profile)
            || self.excluded_tools.iter().any(|name| !canonical(name))
            || self
                .excluded_tools
                .windows(2)
                .any(|pair| pair[0].as_str() >= pair[1].as_str())
        {
            return Err(LoopbackCapabilityError::InvalidIdentity);
        }
        Ok(())
    }

    pub fn excludes(&self, tool_name: &str) -> bool {
        // A plain Conversation may delegate multiple Agents inside its own
        // Agent Execution, but it must never create peer top-level
        // Conversations. Only a companion-bound caller is a Conversation
        // creator on the Gateway surface. Keep this identity rule beside the
        // signed scope so tools/list and tools/call enforce the same boundary.
        (tool_name == GATEWAY_CREATE_CONVERSATION_TOOL && self.companion_id.is_none())
            || self
                .excluded_tools
                .binary_search_by(|name| name.as_str().cmp(tool_name))
                .is_ok()
    }
}

pub type GatewayCapabilityClaims = LoopbackCapabilityClaims<GatewayCapabilityScope>;
pub type GatewayMcpChildConfig = ScopedMcpChildConfig<GatewayCapabilityClaims>;

/// Backend-private Platform Gateway issuer. The root secret and installation
/// owner classification stay in the main process; one short-lived child
/// capability is issued per Conversation bridge.
#[derive(Clone)]
pub struct GatewayMcpConfig {
    port: u16,
    issuer: Arc<LoopbackCapabilityIssuer>,
    pub binary_path: String,
    authoritative_user_id: Arc<str>,
}

impl fmt::Debug for GatewayMcpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatewayMcpConfig")
            .field("port", &self.port)
            .field("issuer", &"[REDACTED]")
            .field("binary_path", &self.binary_path)
            .field("authoritative_user_id", &self.authoritative_user_id)
            .finish()
    }
}

impl GatewayMcpConfig {
    pub fn from_issuer(
        port: u16,
        issuer: Arc<LoopbackCapabilityIssuer>,
        binary_path: String,
        authoritative_user_id: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            port,
            issuer,
            binary_path,
            authoritative_user_id: authoritative_user_id.into(),
        }
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Wire-level MCP server name. Kept short so the longest wire-level tool
    /// name `mcp__nomifun-desktop__nomi_send_to_conversation` (47 chars) stays
    /// within Anthropic's 64-char tool-name limit (see ELECTRON-1JY).
    pub const SERVER_NAME: &'static str = "nomifun-desktop";
    pub const ENV_CAPABILITY: &'static str = "NOMI_GW_MCP_CAPABILITY";

    pub const PROFILE_LITE: &'static str = "lite";
    pub const PROFILE_WORK: &'static str = "work";
    pub const PROFILE_DESKTOP: &'static str = "desktop";
    pub const PROFILE_ADMIN: &'static str = "admin";
    pub const PROFILE_FULL: &'static str = "full";

    pub const LITE_DOMAINS: &'static [&'static str] = &[
        "conversation",
        "provider",
        "cron",
        "requirement",
        "autowork",
        "confirmation",
    ];
    pub const WORK_DOMAINS: &'static [&'static str] = &[
        "conversation",
        // Saved OpenClaw endpoints + local handshakes. Capability policy still
        // hard-denies credential/config mutation from Channel/Remote surfaces.
        "remote",
        "provider",
        "cron",
        "requirement",
        "autowork",
        "confirmation",
        "terminal",
        "files",
        "knowledge",
        "memory",
        "idmm",
        // The desktop default profile lets the lead Agent delegate persistent
        // work and inspect it (nomi_delegate/nomi_execution_get).
        // Remote projects the same domain through companion-scoped auth.
        "agent_execution",
    ];
    pub const DESKTOP_DOMAINS: &'static [&'static str] = &[
        "conversation",
        "remote",
        "provider",
        "confirmation",
        "terminal",
        "files",
        "browser",
        "computer",
        "agent_execution",
        "workshop",
    ];
    pub const ADMIN_DOMAINS: &'static [&'static str] = &[
        "system",
        "mcp",
        "extension",
        "skill",
        "hub",
        "agent",
        "remote",
        "channel",
        "companion",
        "memory",
        "provider",
        "confirmation",
        "workshop",
    ];
    const EMPTY_DOMAINS: &'static [&'static str] = &[];

    /// Map a profile to a capability-domain allow-list. `None` means full
    /// gateway exposure; unknown profiles intentionally resolve to an empty
    /// allow-list rather than widening access by typo.
    pub fn domains_for_profile(profile: &str) -> Option<&'static [&'static str]> {
        match profile.trim().to_ascii_lowercase().as_str() {
            "" | Self::PROFILE_FULL => None,
            Self::PROFILE_LITE => Some(Self::LITE_DOMAINS),
            Self::PROFILE_WORK => Some(Self::WORK_DOMAINS),
            Self::PROFILE_DESKTOP => Some(Self::DESKTOP_DOMAINS),
            Self::PROFILE_ADMIN => Some(Self::ADMIN_DOMAINS),
            _ => Some(Self::EMPTY_DOMAINS),
        }
    }

    pub fn is_known_profile(profile: &str) -> bool {
        matches!(
            profile,
            Self::PROFILE_LITE
                | Self::PROFILE_WORK
                | Self::PROFILE_DESKTOP
                | Self::PROFILE_ADMIN
                | Self::PROFILE_FULL
        )
    }

    pub fn default_profile_for_session(channel_platform: Option<&str>) -> &'static str {
        if channel_platform
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
        {
            Self::PROFILE_LITE
        } else {
            Self::PROFILE_WORK
        }
    }

    pub fn issue_for_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
        companion_id: Option<&str>,
        channel_platform: Option<&str>,
        session_mode: Option<&str>,
        excluded_tools: &[String],
    ) -> Result<GatewayMcpChildConfig, LoopbackCapabilityError> {
        let channel_platform = channel_platform.map(str::to_owned);
        let mut excluded_tools: Vec<String> = excluded_tools.to_vec();
        excluded_tools.sort();
        excluded_tools.dedup();

        let scope = GatewayCapabilityScope {
            companion_id: companion_id
                .map(CompanionId::parse)
                .transpose()
                .map_err(|_| LoopbackCapabilityError::InvalidIdentity)?,
            profile: Self::default_profile_for_session(channel_platform.as_deref()).to_owned(),
            channel_platform,
            session_mode: session_mode.map(str::to_owned),
            excluded_tools,
            instance_owner: user_id == self.authoritative_user_id.as_ref(),
        };
        scope.validate()?;
        let claims = GatewayCapabilityClaims::issue(
            user_id,
            LoopbackSessionBinding::conversation(conversation_id),
            [GATEWAY_LIST_TOOLS_OPERATION, GATEWAY_CALL_TOOL_OPERATION],
            scope,
        )?;
        let (token, renewal_proof) = self.issuer.activate(GATEWAY_CAPABILITY_DOMAIN, &claims)?;
        let lease = LoopbackCapabilityLease::new(
            self.issuer.clone(),
            GATEWAY_CAPABILITY_DOMAIN,
            claims.lease_id.clone(),
        );
        Ok(ScopedMcpChildConfig {
            bootstrap: ScopedMcpChildBootstrap {
                port: self.port,
                renewal: LoopbackCapabilityRenewalRequest {
                    lease_id: claims.lease_id.clone(),
                    renewal_proof,
                },
                access: LoopbackCapabilityAccess { token, claims },
            },
            binary_path: self.binary_path.clone(),
            lease,
        })
    }
}

/// Connection config for the reliable "open" MCP stdio bridge.
///
/// Passed through `AcpBuildExtra::open_mcp_config` by the factory on Windows
/// (only — macOS/Linux already have reliable `open`/`xdg-open` and need no
/// nudging away from `cmd /c start`). The session assembler injects
/// `nomicore mcp-open-stdio` as a stdio MCP server exposing a single `open`
/// tool that ShellExecutes a URL / file / folder / application — giving the
/// agent a dependable launch path instead of the fragile `cmd /c start`
/// window-title quirk.
///
/// Unlike the requirement/gateway bridges this is STATELESS: opening is a pure
/// local OS call, so the bridge needs no HTTP callback — hence no `port`/`token`,
/// only the `nomicore` binary path to re-spawn the subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenMcpConfig {
    pub binary_path: String,
}

impl OpenMcpConfig {
    /// Wire-level MCP server name. Kept short so the wire-level tool name
    /// `mcp__nomifun-open__open` (23 chars) stays well within Anthropic's
    /// 64-char tool-name limit.
    pub const SERVER_NAME: &'static str = "nomifun-open";
}

/// Connection config for the computer-use discrete-tool MCP stdio bridge.
///
/// Passed through `AcpBuildExtra::computer_mcp_config` by the factory on every
/// desktop OS (macOS / Windows / Linux) when the host binary was built with the
/// `computer-use` feature. The session assembler injects `nomicore
/// mcp-computer-stdio` — an MCP server exposing the desktop computer-use
/// capability as discrete tools (snapshot / click / type / launch / …), a thin
/// facade over the in-tree `ComputerTool`, so codex/ACP get the same automation
/// the nomi engine has (macOS AX / Windows UIA / Linux AT-SPI via `nomi-a11y`).
///
/// Like the open bridge this is STATELESS at the protocol level (no HTTP
/// callback): it drives the local desktop directly, so it needs only the
/// `nomicore` binary path to re-spawn the subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerMcpConfig {
    pub binary_path: String,
}

impl ComputerMcpConfig {
    /// Wire-level MCP server name. Kept short so the longest wire-level tool name
    /// `mcp__nomifun-computer__cursor_position` (39 chars) stays within
    /// Anthropic's 64-char tool-name limit.
    pub const SERVER_NAME: &'static str = "nomifun-computer";
}

/// Audience of a browser child capability.
///
/// The first bridge is intentionally ACP-only. Keeping the audience in the
/// signed immutable scope prevents replay by a Gateway, renderer, or remote
/// adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCapabilitySurface {
    Acp,
}

/// Browser operation families authorized for one child runtime.
///
/// These values mirror the platform taxonomy without making this leaf contract
/// crate depend on the browser implementation crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCapabilityOperation {
    Manage,
    Navigate,
    Observe,
    Act,
    Screenshot,
    Tabs,
    Download,
    Debug,
    Crawl,
}

/// Server-authoritative ACP browser scope. The runtime id is generated at
/// issuance time, never accepted from model/tool arguments, and changes on
/// every ACP runtime rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserCapabilityScope {
    pub runtime_instance_id: String,
    pub agent_id: Option<String>,
    pub surface: BrowserCapabilitySurface,
    pub allowed_operations: Vec<BrowserCapabilityOperation>,
}

impl BrowserCapabilityScope {
    pub fn validate(
        &self,
        session: &LoopbackSessionBinding,
    ) -> Result<(), LoopbackCapabilityError> {
        if session.kind != LoopbackSessionKind::Conversation
            || session.conversation_id.as_deref() != Some(session.session_id.as_str())
            || validate_uuidv7(&self.runtime_instance_id).is_err()
            || self.agent_id.as_deref().is_some_and(|agent_id| {
                agent_id.is_empty() || agent_id.trim() != agent_id || agent_id.len() > 128
            })
            || self.allowed_operations.is_empty()
            || self
                .allowed_operations
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(LoopbackCapabilityError::InvalidIdentity);
        }
        Ok(())
    }

    pub fn allows(&self, operation: BrowserCapabilityOperation) -> bool {
        self.allowed_operations.binary_search(&operation).is_ok()
    }
}

pub type BrowserCapabilityClaims = LoopbackCapabilityClaims<BrowserCapabilityScope>;
pub type BrowserMcpChildConfig = ScopedMcpChildConfig<BrowserCapabilityClaims>;

/// Every discrete tool implemented by the stdio facade. `evaluate` remains in
/// the router for protocol compatibility but is not granted by the default ACP
/// capability because arbitrary page script execution is outside the
/// least-privilege surface.
pub const BROWSER_MCP_TOOL_NAMES: &[&str] = &[
    "back",
    "browser_close",
    "browser_close_all",
    "browser_crawl_many",
    "browser_fork",
    "browser_list",
    "browser_open",
    "browser_status",
    "capabilities",
    "click",
    "close_tab",
    "cursor",
    "download",
    "evaluate",
    "extract",
    "find_elements",
    "forward",
    "get_console_logs",
    "get_dropdown_options",
    "get_network_log",
    "get_page_errors",
    "get_page_text",
    "hover",
    "navigate",
    "observe",
    "open_link_new_tab",
    "press_key",
    "reload",
    "save_as_pdf",
    "screenshot",
    "scroll",
    "scroll_to_text",
    "search_page",
    "select_option",
    "set_value",
    "switch_frame",
    "switch_tab",
    "tabs",
    "type",
    "upload_file",
    "wait",
    "wait_for",
];

pub fn browser_tool_operation(tool: &str) -> Option<BrowserCapabilityOperation> {
    let operation = match tool {
        "browser_open" | "browser_fork" | "browser_list" | "browser_status" | "browser_close"
        | "browser_close_all" | "capabilities" => BrowserCapabilityOperation::Manage,
        "browser_crawl_many" => BrowserCapabilityOperation::Crawl,
        "navigate" | "back" | "forward" | "reload" => BrowserCapabilityOperation::Navigate,
        "observe"
        | "get_page_text"
        | "search_page"
        | "find_elements"
        | "get_dropdown_options"
        | "cursor" => BrowserCapabilityOperation::Observe,
        "screenshot" => BrowserCapabilityOperation::Screenshot,
        "tabs" | "switch_tab" | "close_tab" | "open_link_new_tab" => {
            BrowserCapabilityOperation::Tabs
        }
        "download" | "save_as_pdf" => BrowserCapabilityOperation::Download,
        "evaluate" | "get_console_logs" | "get_page_errors" | "get_network_log" => {
            BrowserCapabilityOperation::Debug
        }
        "click" | "extract" | "hover" | "press_key" | "scroll" | "scroll_to_text"
        | "select_option" | "set_value" | "switch_frame" | "type" | "upload_file" | "wait"
        | "wait_for" => BrowserCapabilityOperation::Act,
        _ => return None,
    };
    Some(operation)
}

/// Process-private issuer configuration for the browser stdio proxy.
///
/// The child receives only one renewable, audience-bound bootstrap. It never
/// receives a Chromium debugging port, CDP endpoint, profile path, cookie, or
/// storage value.
#[derive(Clone)]
pub struct BrowserMcpConfig {
    port: u16,
    issuer: Arc<LoopbackCapabilityIssuer>,
    pub binary_path: String,
}

impl fmt::Debug for BrowserMcpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserMcpConfig")
            .field("port", &self.port)
            .field("issuer", &"[REDACTED]")
            .field("binary_path", &self.binary_path)
            .finish()
    }
}

impl BrowserMcpConfig {
    pub fn from_issuer(
        port: u16,
        issuer: Arc<LoopbackCapabilityIssuer>,
        binary_path: String,
    ) -> Self {
        Self {
            port,
            issuer,
            binary_path,
        }
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Wire-level MCP server name. Kept short so the longest wire-level tool name
    /// `mcp__nomifun-browser__get_dropdown_options` (42 chars) stays within
    /// Anthropic's 64-char tool-name limit.
    pub const SERVER_NAME: &'static str = "nomifun-browser";
    pub const ENV_CAPABILITY: &'static str = "NOMI_BROWSER_MCP_CAPABILITY";

    pub fn issue_for_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
        agent_id: Option<&str>,
    ) -> Result<BrowserMcpChildConfig, LoopbackCapabilityError> {
        let scope = BrowserCapabilityScope {
            runtime_instance_id: generate_id(),
            agent_id: agent_id.map(str::to_owned),
            surface: BrowserCapabilitySurface::Acp,
            allowed_operations: vec![
                BrowserCapabilityOperation::Manage,
                BrowserCapabilityOperation::Navigate,
                BrowserCapabilityOperation::Observe,
                BrowserCapabilityOperation::Act,
                BrowserCapabilityOperation::Screenshot,
                BrowserCapabilityOperation::Tabs,
                BrowserCapabilityOperation::Download,
                BrowserCapabilityOperation::Debug,
                BrowserCapabilityOperation::Crawl,
            ],
        };
        let session = LoopbackSessionBinding::conversation(conversation_id);
        scope.validate(&session)?;
        let claims = BrowserCapabilityClaims::issue(
            user_id,
            session,
            BROWSER_MCP_TOOL_NAMES
                .iter()
                .copied()
                .filter(|tool| *tool != "evaluate"),
            scope,
        )?;
        // Multiple ACP runtimes (including cluster attempts) may legitimately
        // share a conversation. Their fresh runtime ids keep Lane ownership
        // distinct, so issuing one must not revoke its siblings.
        let (token, renewal_proof) = self.issuer.activate_concurrent_bounded(
            BROWSER_CAPABILITY_DOMAIN,
            &claims,
            MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY,
        )?;
        let lease = LoopbackCapabilityLease::new(
            self.issuer.clone(),
            BROWSER_CAPABILITY_DOMAIN,
            claims.lease_id.clone(),
        );
        Ok(ScopedMcpChildConfig {
            bootstrap: ScopedMcpChildBootstrap {
                port: self.port,
                renewal: LoopbackCapabilityRenewalRequest {
                    lease_id: claims.lease_id.clone(),
                    renewal_proof,
                },
                access: LoopbackCapabilityAccess { token, claims },
            },
            binary_path: self.binary_path.clone(),
            lease,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const OTHER_USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    const KB_A: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const KB_B: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    const TEST_COMPANION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    fn kb_id(value: &str) -> KnowledgeBaseId {
        KnowledgeBaseId::parse(value).expect("canonical knowledge-base test ID")
    }

    fn test_issuer() -> Arc<LoopbackCapabilityIssuer> {
        Arc::new(LoopbackCapabilityIssuer::random().unwrap())
    }

    fn requirement_config(port: u16, binary_path: &str) -> RequirementMcpConfig {
        RequirementMcpConfig::from_issuer(port, test_issuer(), binary_path.into())
    }

    fn knowledge_config(port: u16, binary_path: &str) -> KnowledgeMcpConfig {
        KnowledgeMcpConfig::from_issuer(port, test_issuer(), binary_path.into())
    }

    fn gateway_config(port: u16, binary_path: &str, owner: &str) -> GatewayMcpConfig {
        GatewayMcpConfig::from_issuer(
            port,
            test_issuer(),
            binary_path.into(),
            Arc::<str>::from(owner),
        )
    }

    #[test]
    fn requirement_issuer_is_redacted_and_build_extra_cannot_serialize_it() {
        let cfg = requirement_config(41234, "/usr/bin/nomicore");
        let debug = format!("{cfg:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("root-secret"));

        let extra = crate::AcpBuildExtra {
            requirement_mcp_config: Some(cfg),
            ..Default::default()
        };
        let json = serde_json::to_string(&extra).unwrap();
        assert!(!json.contains("requirement_mcp_config"));
        assert!(!json.contains("root-secret"));
    }

    #[test]
    fn requirement_child_is_short_lived_domain_and_session_bound() {
        let cfg = requirement_config(41234, "/bin/nomicore");
        let child = cfg
            .issue_for_conversation(TEST_USER_ID, "0190f5fe-7c00-7a00-8abc-012345678901")
            .unwrap();
        let access = &child.bootstrap.access;
        assert_eq!(child.bootstrap.port, 41234);
        assert_eq!(
            access.claims.session.conversation_id.as_deref(),
            Some("0190f5fe-7c00-7a00-8abc-012345678901")
        );
        assert_eq!(
            access.claims.scope.verdict_contract_version,
            REQUIREMENT_EXACT_CLAIM_CONTRACT_VERSION
        );
        assert!(access.claims.scope.requires_opaque_claim_token);
        assert!(access.claims.scope.validate(&access.claims.session).is_ok());
        assert!(access.claims.allows(REQUIREMENT_COMPLETE_TOOL));
        assert!(
            cfg.issuer
                .verify_access(REQUIREMENT_CAPABILITY_DOMAIN, &access.claims, &access.token,)
                .is_ok()
        );
        assert!(
            cfg.issuer
                .verify_access(KNOWLEDGE_CAPABILITY_DOMAIN, &access.claims, &access.token)
                .is_err()
        );

        let bootstrap_json = child.bootstrap_json().unwrap();
        assert!(!bootstrap_json.contains("/bin/nomicore"));
        assert!(!bootstrap_json.contains("root-secret"));
        assert!(!format!("{:?}", child.bootstrap.renewal).contains("root-secret"));
    }

    #[test]
    fn requirement_capability_rejects_pre_exact_claim_contract() {
        let cfg = requirement_config(41234, "/bin/nomicore");
        let child = cfg
            .issue_for_conversation(TEST_USER_ID, "0190f5fe-7c00-7a00-8abc-012345678901")
            .unwrap();
        let mut stale_scope = child.bootstrap.access.claims.scope.clone();
        stale_scope.verdict_contract_version = 1;

        assert_eq!(
            stale_scope.validate(&child.bootstrap.access.claims.session),
            Err(LoopbackCapabilityError::InvalidIdentity)
        );
        let mut tokenless_scope = child.bootstrap.access.claims.scope.clone();
        tokenless_scope.requires_opaque_claim_token = false;
        assert_eq!(
            tokenless_scope.validate(&child.bootstrap.access.claims.session),
            Err(LoopbackCapabilityError::InvalidIdentity)
        );
        assert!(
            serde_json::from_value::<RequirementCapabilityScope>(serde_json::json!({
                "owner_kind": "conversation",
                "owner_session_id": "0190f5fe-7c00-7a00-8abc-012345678901"
            }))
            .is_err(),
            "pre-fence scope without a signed verdict contract must fail closed"
        );
    }

    #[test]
    fn knowledge_child_binds_workspace_bases_and_write_scope() {
        let cfg = knowledge_config(41235, "/bin/nomicore");
        let terminal = cfg
            .issue_for_terminal(
                TEST_USER_ID,
                "0190f5fe-7c00-7a00-8abc-012345678901",
                "/workspace",
                &[kb_id(KB_B), kb_id(KB_A)],
            )
            .unwrap();
        assert_eq!(
            terminal.bootstrap.access.claims.scope.kb_ids,
            vec![kb_id(KB_A), kb_id(KB_B)]
        );
        assert_eq!(
            terminal.bootstrap.access.claims.scope.workspace_path,
            "/workspace"
        );
        // Terminal capabilities always sign all three tools; write authority
        // is enforced live per dispatch from the workpath binding.
        assert!(
            terminal
                .bootstrap
                .access
                .claims
                .allows(KNOWLEDGE_WRITE_TOOL)
        );

        // Conversation issuance keeps the allow_write switch (its runtime is
        // recycled on binding changes, so frozen claims stay accurate).
        let readonly = cfg
            .issue_for_conversation(
                TEST_USER_ID,
                "0190f5fe-7c00-7a00-8abc-012345678901",
                "/workspace",
                &[kb_id(KB_A)],
                false,
            )
            .unwrap();
        assert!(
            !readonly
                .bootstrap
                .access
                .claims
                .allows(KNOWLEDGE_WRITE_TOOL)
        );

        let writable = cfg
            .issue_for_conversation(
                TEST_USER_ID,
                "0190f5fe-7c00-7a00-8abc-012345678901",
                "/workspace",
                &[kb_id(KB_A)],
                true,
            )
            .unwrap();
        assert!(
            writable
                .bootstrap
                .access
                .claims
                .allows(KNOWLEDGE_WRITE_TOOL)
        );
        assert_ne!(
            readonly.bootstrap.access.token,
            writable.bootstrap.access.token
        );
    }

    #[test]
    fn external_knowledge_child_uses_broker_owned_identity_and_scope() {
        let cfg = knowledge_config(41235, "/bin/nomicore");
        let child = cfg
            .issue_for_external_process(
                TEST_USER_ID,
                "external-random",
                "/canonical/workspace",
                &[kb_id(KB_A)],
                false,
            )
            .unwrap();
        let claims = &child.bootstrap.access.claims;
        assert_eq!(claims.user_id.as_str(), TEST_USER_ID);
        assert_eq!(claims.session.kind, LoopbackSessionKind::ExternalProcess);
        assert_eq!(claims.session.session_id, "external-random");
        assert_eq!(claims.session.conversation_id, None);
        assert_eq!(claims.scope.workspace_path, "/canonical/workspace");
        assert_eq!(claims.scope.kb_ids, vec![kb_id(KB_A)]);
        assert!(!claims.allows(KNOWLEDGE_WRITE_TOOL));

        let empty = cfg
            .issue_for_external_process(
                TEST_USER_ID,
                "external-empty",
                "/canonical/empty",
                &[],
                false,
            )
            .unwrap();
        assert!(empty.bootstrap.access.claims.scope.kb_ids.is_empty());
    }

    /// Same Anthropic 64-char tool-name bound as every MCP bridge (ELECTRON-1JY).
    /// The longest requirement tool is `requirement_update_status`.
    #[test]
    fn requirement_mcp_tool_names_stay_within_anthropic_64_char_limit() {
        let longest_tool = "requirement_update_status";
        let wire_name = format!(
            "mcp__{}__{}",
            RequirementMcpConfig::SERVER_NAME,
            longest_tool
        );
        assert!(
            wire_name.len() <= 64,
            "Anthropic 64-char tool-name limit exceeded: {} ({} chars)",
            wire_name,
            wire_name.len()
        );
    }

    #[test]
    fn gateway_issuer_is_redacted_and_build_extra_cannot_serialize_it() {
        let cfg = gateway_config(
            41235,
            "/usr/bin/nomicore",
            "0190f5fe-7c00-7a00-8000-000000000001",
        );
        let debug = format!("{cfg:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("root-secret"));

        let extra = crate::AcpBuildExtra {
            gateway_mcp_config: Some(cfg),
            ..Default::default()
        };
        let json = serde_json::to_string(&extra).unwrap();
        assert!(!json.contains("gateway_mcp_config"));
        assert!(!json.contains("root-secret"));
    }

    #[test]
    fn gateway_child_binds_operations_identity_surface_profile_and_exclusions() {
        let cfg = gateway_config(41235, "/usr/bin/nomicore", TEST_USER_ID);
        let child = cfg
            .issue_for_conversation(
                OTHER_USER_ID,
                "0190f5fe-7c00-7a00-8abc-012345678901",
                Some(TEST_COMPANION_ID),
                Some("lark"),
                Some("yolo"),
                &["nomi_delegate".into(), "nomi_delegate".into()],
            )
            .unwrap();
        let access = &child.bootstrap.access;
        assert_eq!(child.bootstrap.port, 41235);
        assert_eq!(access.claims.user_id.as_str(), OTHER_USER_ID);
        assert_eq!(
            access.claims.session.conversation_id.as_deref(),
            Some("0190f5fe-7c00-7a00-8abc-012345678901")
        );
        assert!(access.claims.allows(GATEWAY_LIST_TOOLS_OPERATION));
        assert!(access.claims.allows(GATEWAY_CALL_TOOL_OPERATION));
        assert_eq!(access.claims.scope.profile, GatewayMcpConfig::PROFILE_LITE);
        assert_eq!(access.claims.scope.excluded_tools, vec!["nomi_delegate"]);
        assert!(!access.claims.scope.instance_owner);
        assert!(
            cfg.issuer
                .verify_access(GATEWAY_CAPABILITY_DOMAIN, &access.claims, &access.token)
                .is_ok()
        );

        let mut forged_user = access.claims.clone();
        forged_user.user_id = nomifun_common::UserId::parse(TEST_USER_ID).unwrap();
        forged_user.scope.instance_owner = true;
        assert!(
            cfg.issuer
                .verify_access(GATEWAY_CAPABILITY_DOMAIN, &forged_user, &access.token)
                .is_err()
        );

        let mut forged_conversation = access.claims.clone();
        forged_conversation.session =
            LoopbackSessionBinding::conversation("0190f5fe-7c00-7a00-8abc-012345678902");
        assert!(
            cfg.issuer
                .verify_access(
                    GATEWAY_CAPABILITY_DOMAIN,
                    &forged_conversation,
                    &access.token,
                )
                .is_err()
        );

        let mut forged_scope = access.claims.clone();
        forged_scope.scope.channel_platform = None;
        forged_scope.scope.profile = GatewayMcpConfig::PROFILE_WORK.into();
        assert!(
            cfg.issuer
                .verify_access(GATEWAY_CAPABILITY_DOMAIN, &forged_scope, &access.token)
                .is_err()
        );
    }

    #[test]
    fn gateway_scope_reserves_top_level_creation_for_companions() {
        let cfg = gateway_config(41235, "/usr/bin/nomicore", TEST_USER_ID);
        let plain = cfg
            .issue_for_conversation(
                TEST_USER_ID,
                "0190f5fe-7c00-7a00-8abc-012345678901",
                None,
                None,
                None,
                &[],
            )
            .unwrap();
        assert!(
            plain
                .bootstrap
                .access
                .claims
                .scope
                .excludes(GATEWAY_CREATE_CONVERSATION_TOOL)
        );

        let companion = cfg
            .issue_for_conversation(
                TEST_USER_ID,
                "0190f5fe-7c00-7a00-8abc-012345678902",
                Some(TEST_COMPANION_ID),
                None,
                None,
                &[],
            )
            .unwrap();
        assert!(
            !companion
                .bootstrap
                .access
                .claims
                .scope
                .excludes(GATEWAY_CREATE_CONVERSATION_TOOL)
        );
    }

    #[test]
    fn gateway_correctly_signed_expired_claims_fail_closed() {
        let cfg = gateway_config(41235, "/usr/bin/nomicore", TEST_USER_ID);
        let child = cfg
            .issue_for_conversation(
                TEST_USER_ID,
                "0190f5fe-7c00-7a00-8abc-012345678901",
                None,
                None,
                None,
                &[],
            )
            .unwrap();
        let now = nomifun_common::unix_time_secs();
        let expired = cfg
            .issuer
            .renew_at::<GatewayCapabilityScope>(
                GATEWAY_CAPABILITY_DOMAIN,
                &child.bootstrap.renewal,
                now.saturating_sub(nomifun_common::LOOPBACK_CAPABILITY_TTL_SECS + 1),
            )
            .unwrap();
        assert_eq!(
            cfg.issuer
                .verify_access(GATEWAY_CAPABILITY_DOMAIN, &expired.claims, &expired.token,),
            Err(LoopbackCapabilityError::Expired)
        );
    }

    #[test]
    fn dropping_unaccepted_child_config_revokes_its_renewable_lease() {
        let cfg = requirement_config(41234, "/bin/nomicore");
        let child = cfg
            .issue_for_conversation(TEST_USER_ID, "0190f5fe-7c00-7a00-8abc-012345678901")
            .unwrap();
        let renewal = child.bootstrap.renewal.clone();

        assert!(
            cfg.issuer
                .renew::<RequirementCapabilityScope>(REQUIREMENT_CAPABILITY_DOMAIN, &renewal)
                .is_ok()
        );
        drop(child);
        assert_eq!(
            cfg.issuer
                .renew::<RequirementCapabilityScope>(REQUIREMENT_CAPABILITY_DOMAIN, &renewal,),
            Err(LoopbackCapabilityError::InvalidToken)
        );
    }

    #[test]
    fn gateway_profile_domains_are_curated_and_unknown_is_empty() {
        assert_eq!(
            GatewayMcpConfig::domains_for_profile(GatewayMcpConfig::PROFILE_FULL),
            None
        );
        assert!(
            GatewayMcpConfig::domains_for_profile(GatewayMcpConfig::PROFILE_WORK)
                .unwrap()
                .contains(&"requirement")
        );
        // Ordinary conversations and companion sessions both use the work
        // profile, so neither may call 创意工坊 tools. The dedicated desktop and
        // admin profiles retain the domain for explicitly privileged surfaces.
        assert!(!GatewayMcpConfig::WORK_DOMAINS.contains(&"workshop"));
        assert!(GatewayMcpConfig::DESKTOP_DOMAINS.contains(&"workshop"));
        assert!(GatewayMcpConfig::ADMIN_DOMAINS.contains(&"workshop"));
        assert!(!GatewayMcpConfig::LITE_DOMAINS.contains(&"workshop"));
        assert!(GatewayMcpConfig::WORK_DOMAINS.contains(&"agent_execution"));
        assert!(GatewayMcpConfig::DESKTOP_DOMAINS.contains(&"agent_execution"));
        assert!(!GatewayMcpConfig::LITE_DOMAINS.contains(&"agent_execution"));
        assert!(GatewayMcpConfig::WORK_DOMAINS.contains(&"remote"));
        assert!(GatewayMcpConfig::DESKTOP_DOMAINS.contains(&"remote"));
        assert!(GatewayMcpConfig::ADMIN_DOMAINS.contains(&"remote"));
        assert!(!GatewayMcpConfig::LITE_DOMAINS.contains(&"remote"));
        assert_eq!(
            GatewayMcpConfig::domains_for_profile("typo-profile"),
            Some(&[][..])
        );
        assert_eq!(
            GatewayMcpConfig::default_profile_for_session(Some("lark")),
            GatewayMcpConfig::PROFILE_LITE
        );
        assert_eq!(
            GatewayMcpConfig::default_profile_for_session(None),
            GatewayMcpConfig::PROFILE_WORK
        );
    }

    #[test]
    fn open_mcp_config_json_roundtrip() {
        let cfg = OpenMcpConfig {
            binary_path: "/usr/bin/nomicore".into(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: OpenMcpConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, parsed);
    }

    /// The open server's single tool `open` stays well within Anthropic's
    /// 64-char wire-level tool-name limit.
    #[test]
    fn open_mcp_tool_name_stays_within_anthropic_64_char_limit() {
        let wire_name = format!("mcp__{}__{}", OpenMcpConfig::SERVER_NAME, "open");
        assert!(
            wire_name.len() <= 64,
            "{wire_name} ({} chars)",
            wire_name.len()
        );
    }

    #[test]
    fn computer_mcp_config_json_roundtrip() {
        let cfg = ComputerMcpConfig {
            binary_path: "/usr/bin/nomicore".into(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: ComputerMcpConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, parsed);
    }

    /// The computer bridge's longest discrete tool name must stay within
    /// Anthropic's 64-char wire-level tool-name limit.
    #[test]
    fn computer_mcp_tool_name_stays_within_anthropic_64_char_limit() {
        let wire_name = format!(
            "mcp__{}__{}",
            ComputerMcpConfig::SERVER_NAME,
            "cursor_position"
        );
        assert!(
            wire_name.len() <= 64,
            "{wire_name} ({} chars)",
            wire_name.len()
        );
    }

    #[test]
    fn browser_mcp_config_issues_scoped_acp_capability() {
        let cfg = BrowserMcpConfig::from_issuer(41_000, test_issuer(), "/usr/bin/nomicore".into());
        let child = cfg
            .issue_for_conversation(TEST_USER_ID, OTHER_USER_ID, Some("agent-1"))
            .unwrap();
        assert_eq!(child.bootstrap.port, 41_000);
        assert_eq!(
            child.bootstrap.access.claims.scope.surface,
            BrowserCapabilitySurface::Acp
        );
        assert!(
            child
                .bootstrap
                .access
                .claims
                .scope
                .allows(BrowserCapabilityOperation::Navigate)
        );
        assert!(
            !child.bootstrap.access.claims.allows("evaluate"),
            "arbitrary page script execution is not in the default ACP scope"
        );
        for tool in ["get_console_logs", "get_page_errors", "get_network_log"] {
            assert!(
                child.bootstrap.access.claims.allows(tool),
                "read-only ACP debug capability must include {tool}"
            );
        }
        for tool in [
            "browser_open",
            "browser_fork",
            "browser_list",
            "browser_status",
            "browser_close",
            "browser_close_all",
            "browser_crawl_many",
        ] {
            assert!(
                child.bootstrap.access.claims.allows(tool),
                "default ACP browser capability must include {tool}"
            );
        }
        assert!(
            child
                .bootstrap
                .access
                .claims
                .scope
                .allows(BrowserCapabilityOperation::Crawl)
        );
        assert!(
            child
                .bootstrap
                .access
                .claims
                .scope
                .allows(BrowserCapabilityOperation::Debug)
        );
        assert!(validate_uuidv7(&child.bootstrap.access.claims.scope.runtime_instance_id).is_ok());
    }

    #[test]
    fn browser_mcp_config_keeps_sibling_runtimes_in_one_conversation_active() {
        let cfg = BrowserMcpConfig::from_issuer(41_000, test_issuer(), "/usr/bin/nomicore".into());
        let first = cfg
            .issue_for_conversation(TEST_USER_ID, OTHER_USER_ID, Some("agent-1"))
            .unwrap();
        let second = cfg
            .issue_for_conversation(TEST_USER_ID, OTHER_USER_ID, Some("agent-2"))
            .unwrap();
        assert_ne!(
            first.bootstrap.access.claims.scope.runtime_instance_id,
            second.bootstrap.access.claims.scope.runtime_instance_id
        );
        assert!(
            cfg.issuer
                .verify_access(
                    BROWSER_CAPABILITY_DOMAIN,
                    &first.bootstrap.access.claims,
                    &first.bootstrap.access.token,
                )
                .is_ok(),
            "a sibling ACP runtime must not revoke an existing runtime"
        );
        assert!(
            cfg.issuer
                .verify_access(
                    BROWSER_CAPABILITY_DOMAIN,
                    &second.bootstrap.access.claims,
                    &second.bootstrap.access.token,
                )
                .is_ok()
        );
    }

    #[test]
    fn browser_mcp_config_caps_one_task_family_and_drop_restores_capacity() {
        let cfg = BrowserMcpConfig::from_issuer(41_000, test_issuer(), "/usr/bin/nomicore".into());
        let mut children = Vec::with_capacity(MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY);
        for index in 0..MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY {
            children.push(
                cfg.issue_for_conversation(
                    TEST_USER_ID,
                    OTHER_USER_ID,
                    Some(&format!("agent-{index}")),
                )
                .unwrap(),
            );
        }

        assert_eq!(
            cfg.issue_for_conversation(TEST_USER_ID, OTHER_USER_ID, Some("overflow"))
                .unwrap_err(),
            LoopbackCapabilityError::CapacityExceeded
        );

        drop(children.pop());
        children.push(
            cfg.issue_for_conversation(TEST_USER_ID, OTHER_USER_ID, Some("replacement"))
                .expect("dropping the final lease guard must restore exact task capacity"),
        );
        assert_eq!(children.len(), MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY);
    }

    #[test]
    fn browser_mcp_task_capacity_is_isolated_by_user_and_conversation() {
        let cfg = BrowserMcpConfig::from_issuer(41_000, test_issuer(), "/usr/bin/nomicore".into());
        let saturated = (0..MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY)
            .map(|index| {
                cfg.issue_for_conversation(
                    TEST_USER_ID,
                    OTHER_USER_ID,
                    Some(&format!("saturated-{index}")),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();

        let other_conversation = cfg
            .issue_for_conversation(TEST_USER_ID, TEST_USER_ID, Some("other-conversation"))
            .expect("one conversation must not consume another conversation's capacity");
        let other_user = cfg
            .issue_for_conversation(OTHER_USER_ID, OTHER_USER_ID, Some("other-user"))
            .expect("one user must not consume another user's capacity");

        assert_eq!(
            saturated.len(),
            MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY
        );
        drop(other_conversation);
        drop(other_user);
    }

    #[test]
    fn browser_mcp_task_capacity_is_atomic_under_concurrent_issuance() {
        let cfg = Arc::new(BrowserMcpConfig::from_issuer(
            41_000,
            test_issuer(),
            "/usr/bin/nomicore".into(),
        ));
        let attempt_count = MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY + 8;
        let barrier = Arc::new(std::sync::Barrier::new(attempt_count));
        let mut handles = Vec::with_capacity(attempt_count);
        for index in 0..attempt_count {
            let cfg = Arc::clone(&cfg);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                cfg.issue_for_conversation(
                    TEST_USER_ID,
                    OTHER_USER_ID,
                    Some(&format!("racing-{index}")),
                )
            }));
        }

        let mut accepted = Vec::new();
        let mut capacity_rejections = 0;
        for handle in handles {
            match handle.join().expect("issuance thread panicked") {
                Ok(child) => accepted.push(child),
                Err(LoopbackCapabilityError::CapacityExceeded) => capacity_rejections += 1,
                Err(error) => panic!("unexpected concurrent issuance error: {error}"),
            }
        }

        assert_eq!(
            accepted.len(),
            MAX_BROWSER_MCP_CAPABILITIES_PER_TASK_FAMILY,
            "the atomic issuer lock must never exceed the per-task runtime family bound"
        );
        assert_eq!(capacity_rejections, attempt_count - accepted.len());
    }

    #[test]
    fn browser_capability_scope_rejects_duplicate_operations_and_wrong_session_kind() {
        let mut scope = BrowserCapabilityScope {
            runtime_instance_id: generate_id(),
            agent_id: None,
            surface: BrowserCapabilitySurface::Acp,
            allowed_operations: vec![
                BrowserCapabilityOperation::Manage,
                BrowserCapabilityOperation::Navigate,
            ],
        };
        let conversation = LoopbackSessionBinding::conversation(OTHER_USER_ID);
        assert!(scope.validate(&conversation).is_ok());

        scope
            .allowed_operations
            .push(BrowserCapabilityOperation::Navigate);
        assert_eq!(
            scope.validate(&conversation),
            Err(LoopbackCapabilityError::InvalidIdentity)
        );
        scope.allowed_operations.pop();
        assert_eq!(
            scope.validate(&LoopbackSessionBinding::terminal(OTHER_USER_ID)),
            Err(LoopbackCapabilityError::InvalidIdentity)
        );
    }

    #[test]
    fn browser_management_tools_have_one_shared_operation_contract() {
        for tool in [
            "browser_open",
            "browser_fork",
            "browser_list",
            "browser_status",
            "browser_close",
            "browser_close_all",
        ] {
            assert_eq!(
                browser_tool_operation(tool),
                Some(BrowserCapabilityOperation::Manage),
                "{tool}"
            );
            assert!(BROWSER_MCP_TOOL_NAMES.contains(&tool));
        }
        assert_eq!(
            browser_tool_operation("browser_crawl_many"),
            Some(BrowserCapabilityOperation::Crawl)
        );
        assert!(BROWSER_MCP_TOOL_NAMES.contains(&"browser_crawl_many"));
        for tool in ["get_console_logs", "get_page_errors", "get_network_log"] {
            assert_eq!(
                browser_tool_operation(tool),
                Some(BrowserCapabilityOperation::Debug),
                "{tool}"
            );
            assert!(BROWSER_MCP_TOOL_NAMES.contains(&tool));
        }
    }

    /// The browser bridge's longest discrete tool name (`get_dropdown_options`)
    /// must stay within Anthropic's 64-char wire-level tool-name limit.
    #[test]
    fn browser_mcp_tool_name_stays_within_anthropic_64_char_limit() {
        let wire_name = format!(
            "mcp__{}__{}",
            BrowserMcpConfig::SERVER_NAME,
            "get_dropdown_options"
        );
        assert!(
            wire_name.len() <= 64,
            "{wire_name} ({} chars)",
            wire_name.len()
        );
    }

    /// The Anthropic 64-char wire-name bound (ELECTRON-1JY). The gateway
    /// advertises as `SERVER_NAME`, so a wire name is `mcp__{SERVER_NAME}__{tool}`.
    /// This asserts the server-name prefix leaves a workable budget for tool
    /// names (>= 42 chars). PER-TOOL enforcement — iterating every registered
    /// name against the real limit — lives in `nomifun-gateway`'s registry
    /// self-test (`registry_builds_and_names_fit_mcp_limit`); this avoids a
    /// stale hand-picked exemplar here.
    #[test]
    fn gateway_server_name_leaves_workable_tool_name_budget() {
        let prefix = format!("mcp__{}__", GatewayMcpConfig::SERVER_NAME).len();
        let budget = 64usize.saturating_sub(prefix);
        assert!(
            budget >= 42,
            "server name '{}' leaves only {budget} chars for tool names (need >= 42); pick a shorter SERVER_NAME",
            GatewayMcpConfig::SERVER_NAME,
        );
    }
}
