use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use nomi_config::config::Config;
use nomi_mcp::manager::McpManager;
use nomi_providers::LlmProvider;

use crate::engine::AgentEngine;
use crate::output::OutputSink;
use crate::session::Session;

#[cfg(test)]
mod mcp_capability_policy_tests {
    use super::expand_mcp_tool_proxy_allowlist;

    #[test]
    fn proxy_marker_expands_only_exact_discovered_origins() {
        let expected = nomi_mcp::tool_proxy::canonical_mcp_display_name("bound", "lookup");
        let denied = nomi_mcp::tool_proxy::canonical_mcp_display_name("other", "lookup");

        let mut without_marker = vec!["mcp_resource_list".to_owned()];
        expand_mcp_tool_proxy_allowlist(&mut without_marker, [("bound", "lookup")]);
        assert!(!without_marker.contains(&expected));

        let mut allowed = vec!["mcp.tool_proxy".to_owned(), expected.clone()];
        expand_mcp_tool_proxy_allowlist(
            &mut allowed,
            [("bound", "lookup"), ("bound", "lookup")],
        );
        assert_eq!(
            allowed
                .iter()
                .filter(|name| name.as_str() == expected.as_str())
                .count(),
            1
        );
        assert!(!allowed.contains(&denied));
    }
}

/// Result of bootstrapping an agent engine with all features initialized.
pub struct BootstrapResult {
    pub engine: AgentEngine,
    pub provider: Arc<dyn LlmProvider>,
    pub mcp_managers: Vec<Arc<McpManager>>,
    pub has_mcp: bool,
}

fn expand_mcp_tool_proxy_allowlist<'a>(
    allowed_tools: &mut Vec<String>,
    discovered_tools: impl IntoIterator<Item = (&'a str, &'a str)>,
) {
    if !allowed_tools.iter().any(|name| name == "mcp.tool_proxy") {
        return;
    }
    for (server_name, tool_name) in discovered_tools {
        let provider_name =
            nomi_mcp::tool_proxy::canonical_mcp_display_name(server_name, tool_name);
        if !allowed_tools.contains(&provider_name) {
            allowed_tools.push(provider_name);
        }
    }
}

/// Builder for creating a fully-initialized `AgentEngine`.
///
/// Encapsulates the complete initialization pipeline so all consumers
/// (CLI, backend, delegated Agents) get consistent behavior:
///
/// - System prompt always includes model identity, working directory, date
/// - Tool usage guidance is always injected
/// - AGENTS.md is loaded from the workspace hierarchy
/// - Skills, MCP, and plan mode are enabled from `Config`
/// - Embedded AgentExecution is installed only when selected by the host
pub struct AgentBootstrap {
    config: Config,
    workspace: String,
    output: Arc<dyn OutputSink>,
    provider: Option<Arc<dyn LlmProvider>>,
    resume_session: Option<Session>,
    extra_skill_dirs: Vec<PathBuf>,
    host_skills: Vec<Arc<crate::host_skills::HostSkill>>,
    goal: Option<crate::goal::runtime::GoalSpec>,
    /// Host composition switch for embedded AgentExecution. CLI
    /// and standalone embeddings default to installing it; backend sessions
    /// explicitly disable it when Platform Gateway owns persistent execution
    /// or the caller is outside the trusted local-owner boundary.
    install_embedded_agent_execution: bool,
    /// Trusted main-process Browser Platform capability for this runtime.
    /// The embedded host resolves/signs caller identity before constructing
    /// this client; neither the model nor tool input can populate those fields.
    ///
    /// When present, the Browser tool uses this client as its only execution
    /// path and cannot lazily launch a private Chromium process.
    /// When present, the session is bound to a remote SSH host: the remote tool
    /// family takes over the `Read`/`Write`/`Edit`/`Bash`/`Grep`/`Glob` names
    /// instead of the local implementations. The connection lives behind this
    /// backend; the model never sees credentials or host identity.
    ssh_session: Option<Arc<dyn crate::ssh_backend::SshBackend>>,
}

impl AgentBootstrap {
    pub fn new(config: Config, workspace: impl Into<String>, output: Arc<dyn OutputSink>) -> Self {
        Self {
            config,
            workspace: workspace.into(),
            output,
            provider: None,
            resume_session: None,
            extra_skill_dirs: Vec::new(),
            host_skills: Vec::new(),
            goal: None,
            install_embedded_agent_execution: true,
            ssh_session: None,
        }
    }

    /// Use a pre-created provider instead of creating one from config.
    pub fn provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Enable goal-driven continuation for this session (opt-in). Omit it (the
    /// default) and the engine behaves exactly as before.
    pub fn goal(mut self, goal: Option<crate::goal::runtime::GoalSpec>) -> Self {
        self.goal = goal;
        self
    }

    /// Select whether this host installs embedded AgentExecution.
    /// This is deliberately not part of [`Config`]: composition belongs to the
    /// embedding host, not to user TOML or model-writable runtime state.
    pub fn install_embedded_agent_execution(mut self, install: bool) -> Self {
        self.install_embedded_agent_execution = install;
        self
    }

    /// Resume from a previously saved session.
    pub fn resume(mut self, session: Session) -> Self {
        self.resume_session = Some(session);
        self
    }

    /// Bind this session to a remote SSH host. The remote tool family takes over
    /// the native `Read`/`Write`/`Edit`/`Bash`/`Grep`/`Glob` names, so the model
    /// operates the remote host through its ordinary vocabulary. Credentials and
    /// host identity live behind the backend and never reach model input.
    pub fn ssh_session(mut self, backend: Arc<dyn crate::ssh_backend::SshBackend>) -> Self {
        self.ssh_session = Some(backend);
        self
    }

    /// Add extra directories to scan for skills.
    pub fn extra_skill_dirs(mut self, dirs: Vec<PathBuf>) -> Self {
        self.extra_skill_dirs = dirs;
        self
    }

    /// Exact host-resolved Skills; never rediscovered by directory name.
    pub fn host_skills(mut self, skills: Vec<Arc<crate::host_skills::HostSkill>>) -> Self {
        self.host_skills = skills;
        self
    }

    /// Read-only access to the config (for session management before build).
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Build the fully-initialized engine.
    pub async fn build(mut self) -> anyhow::Result<BootstrapResult> {
        let cwd = &self.workspace;
        let cwd_path = std::path::Path::new(cwd);

        tracing::info!(target: "nomi_agent", workspace = %cwd, "agent bootstrap: workspace cwd resolved");

        let provider = self
            .provider
            .unwrap_or_else(|| nomi_providers::create_provider(&self.config));

        let tool_free = self.config.tools.enforce_builtin_allowlist
            && self.config.tools.builtin_allowlist.is_empty();
        let memory_dir = if tool_free {
            None
        } else {
            nomi_memory::paths::auto_memory_dir(cwd_path)
        };

        let file_cache = if self.config.file_cache.enabled {
            Some(Arc::new(std::sync::RwLock::new(
                nomi_tools::file_cache::FileStateCache::new(&self.config.file_cache),
            )))
        } else {
            None
        };

        let mut registry = nomi_tools::registry::ToolRegistry::new();
        registry.force_deferred_named(&self.config.tools.deferred_allowlist);
        // Opt-in write-root containment (§3.6): when `tools.write_root` is set,
        // resolve it to an absolute path the write tools enforce. Empty = off.
        let write_root: Option<std::path::PathBuf> = {
            let wr = self.config.tools.write_root.trim();
            if wr.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(wr))
            }
        };
        // SSH-bound session: the remote tool family takes over Read/Write/Edit/
        // Bash/Grep/Glob; the local filesystem/exec tools (incl. ApplyPatch, Lsp,
        // exec_command, write_stdin) are skipped so nothing operates the local
        // machine by mistake. Otherwise register the local family as usual.
        let ssh_backend = self.ssh_session.clone();
        if let Some(ssh) = ssh_backend.clone() {
            for tool in crate::ssh_tools::remote_tool_family(ssh) {
                registry.register(tool);
            }
        } else {
            registry.register(Box::new(nomi_tools::read::ReadTool::new(
                file_cache.clone(),
                Some(cwd_path.to_path_buf()),
            )));
            registry.register(Box::new(
                nomi_tools::write::WriteTool::new(file_cache.clone())
                    .with_write_root(write_root.clone())
                    .with_cwd(Some(cwd_path.to_path_buf())),
            ));
            registry.register(Box::new(
                nomi_tools::edit::EditTool::new(file_cache.clone())
                    .with_write_root(write_root.clone())
                    .with_cwd(Some(cwd_path.to_path_buf())),
            ));
            registry.register(Box::new(
                nomi_tools::apply_patch::ApplyPatchTool::new(file_cache)
                    .with_write_root(write_root.clone())
                    .with_cwd(Some(cwd_path.to_path_buf())),
            ));
            // VCS capabilities have their own typed native owner. Never map
            // a VCS grant to the general-purpose Bash tool: the Git family
            // enforces repository/workspace scope and serializes mutations.
            for tool in nomi_tools::vcs::local_vcs_tools(cwd_path.to_path_buf()) {
                registry.register(tool);
            }
            // Experimental `Lsp` code-navigation tool: registered only when at least
            // one language server is configured (default off → no behaviour change).
            {
                let mut lsp_map: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();
                for entry in &self.config.tools.lsp_servers {
                    if entry.command.is_empty() {
                        continue;
                    }
                    for ext in &entry.extensions {
                        lsp_map.insert(ext.trim_start_matches('.').to_ascii_lowercase(), entry.command.clone());
                    }
                }
                if !lsp_map.is_empty() {
                    registry.register(Box::new(nomi_tools::lsp::LspTool::new(
                        lsp_map,
                        cwd_path.to_path_buf(),
                    )));
                }
            }
        }
        // Native `remember` tool: persist durable project/user memories mid-session
        // to the file-based long-term memory (injected into future sessions).
        if let Some(mem_dir) = memory_dir.clone() {
            registry.register(Box::new(crate::memory_tools::RememberTool::new(mem_dir)));
        }
        let process_supervisor =
            nomi_process_runtime::ProcessSupervisor::new(nomi_process_runtime::SupervisorConfig::default());
        let process_capability = nomi_process_runtime::CapabilityPolicy {
            cwd_roots: vec![cwd_path.to_path_buf()],
            sandbox: if self.config.tools.bash_sandbox {
                nomi_process_runtime::SandboxPolicy::MacSeatbelt {
                    write_roots: vec![cwd_path.to_path_buf()],
                }
            } else {
                nomi_process_runtime::SandboxPolicy::UnrestrictedLocalOwner
            },
        };
        // Local shell/exec tools operate the local machine, so they are skipped
        // in an SSH-bound session (the remote Bash/Grep/Glob family already took
        // over those names above). The supervisor itself is still constructed —
        // later wiring (`with_process_supervisor`, `set_process_supervisor`)
        // depends on it regardless of session kind.
        if ssh_backend.is_none() {
            registry.register(Box::new(nomi_tools::bash::BashTool::new(
                Arc::clone(&process_supervisor),
                cwd_path.to_path_buf(),
                process_capability.clone(),
            )));
            registry.register(Box::new(nomi_tools::grep::GrepTool::new(
                cwd_path.to_path_buf(),
            )));
            registry.register(Box::new(nomi_tools::glob::GlobTool::new(
                cwd_path.to_path_buf(),
            )));

            // Numeric-session schemas share the same supervisor as Bash. The
            // ProcessStore is only a numeric-id adapter; it owns no OS process.
            let process_store = Arc::new(nomi_tools::process_store::ProcessStore::new());
            registry.register(Box::new(nomi_tools::exec_command::ExecCommandTool::new(
                Arc::clone(&process_supervisor),
                Arc::clone(&process_store),
                cwd_path.to_path_buf(),
                process_capability.clone(),
            )));
            registry.register(Box::new(nomi_tools::write_stdin::WriteStdinTool::new(
                Arc::clone(&process_supervisor),
                Arc::clone(&process_store),
            )));
        }

        let mut mcp_managers: Vec<Arc<McpManager>> = Vec::new();
        let mcp_manager = if !tool_free && !self.config.mcp.servers.is_empty() {
            match McpManager::connect_all(&self.config.mcp.servers).await {
                Ok(mgr) => {
                    let mgr = Arc::new(mgr);
                    mcp_managers.push(mgr.clone());
                    Some(mgr)
                }
                Err(e) => {
                    self.output
                        .emit_warning(&format!("MCP initialization error: {e}"));
                    None
                }
            }
        } else {
            None
        };
        let has_mcp = mcp_manager.is_some();

        if tool_free && !self.host_skills.is_empty() {
            anyhow::bail!("selected hosted Skills require the Skill tool in the Session policy");
        }
        let mut skills = if tool_free {
            Vec::new()
        } else {
            nomi_skills::loader::load_all_skills(
                cwd_path,
                &self.extra_skill_dirs,
                false,
                mcp_manager.as_deref(),
            )
            .await
        };

        crate::host_skills::merge_host_skills(&mut skills, &self.host_skills);
        let mut prompt_cache = crate::context::SystemPromptCache::new();
        if !tool_free {
            let agents_snapshot = crate::agents_md::resolve_agents_md(
                cwd_path,
                &self.config.project_instructions,
            );
            for file in &agents_snapshot.files {
                tracing::debug!(
                    target: "nomi_agent",
                    path = %file.path.display(),
                    scope = if file.is_global { "user" } else { "project" },
                    "agent bootstrap: loaded instruction file"
                );
            }
            for diagnostic in &agents_snapshot.diagnostics {
                tracing::warn!(
                    target: "nomi_agent",
                    message = %diagnostic.message(),
                    "agent bootstrap: instruction diagnostic"
                );
            }

            prompt_cache.set_agents_md(agents_snapshot.formatted);
        }
        // SSH-bound session: `cwd` here is a LOCAL scratch directory (design F2
        // keeps `extra.workspace` local for transcripts and attachments), while
        // every file/exec tool registered above is the remote family and no local
        // exec tool exists at all. Rendering that path as "Working directory"
        // would name somewhere none of the model's tools can reach, on a machine
        // it is not operating. Seed the section so `build_system_prompt`'s
        // `or_insert_with` default never runs; the date still comes along, since
        // that is the other half of what this section owes the model.
        if ssh_backend.is_some() {
            prompt_cache.set_environment(format!(
                "This session runs entirely on a remote host over SSH. Read, Write, Edit, Bash, \
                 Grep and Glob all act on that host, and no tool here can reach the local machine. \
                 There is no local working directory: the remote shell starts in the login user's \
                 default directory on the host (run `pwd` to see it), and paths from the local \
                 machine do not exist there.\nCurrent date: {}",
                chrono::Local::now().format("%Y-%m-%d")
            ));
        }
        let system_prompt = if tool_free {
            crate::context::build_chat_only_system_prompt(self.config.system_prompt.as_deref())
        } else {
            crate::context::build_system_prompt(
                &mut prompt_cache,
                self.config.system_prompt.as_deref(),
                cwd,
                &self.config.model,
                &skills,
                Some(self.config.compact.context_window),
                memory_dir.as_deref(),
                false,
                self.config.compact.toon,
            )
        };
        self.config.system_prompt = Some(system_prompt);

        let skills_arc = Arc::new(skills);
        let skill_checker = nomi_skills::permissions::SkillPermissionChecker::new(self.config.tools.skills.deny.clone());
        // No-gateway CLI/embedded engines share one Agent invocation runner
        // between fork-mode skills and embedded `nomi_delegate`. Platform
        // Gateway sessions disable the embedded deployment and expose the same
        // AgentExecution contract through the platform, so a model sees one tool.
        let local_invocation_runner = if self.install_embedded_agent_execution && !tool_free {
            Some(Arc::new(
                crate::local_agent_invocation::LocalAgentInvocationRunner::new(
                    provider.clone(),
                    self.config.clone(),
                    cwd_path.to_path_buf(),
                )
                .with_process_capability(
                    process_capability.clone(),
                    write_root.clone(),
                    self.config.tools.builtin_allowlist.clone(),
                )
                .with_token_budget(
                    self.config
                        .tools
                        .delegation_token_budget
                        .map(|limit| {
                            Arc::new(crate::local_agent_invocation::TokenBudget::new(limit))
                        }),
                ),
            ))
        } else {
            None
        };
        let skill_invocation_runner = local_invocation_runner.as_ref().map(|runner| {
            Arc::clone(runner) as Arc<dyn nomi_types::agent::AgentInvocationRunner>
        });
        let skill_tool = crate::skill_tool::SkillTool::with_invocation_runner(
            skills_arc,
            cwd.to_string(),
            skill_checker,
            None,
            skill_invocation_runner,
        )
        .with_host_skills(&self.host_skills)
        .with_process_supervisor(Arc::clone(&process_supervisor));
        let host_skill_commands = skill_tool.host_commands();
        registry.register(Box::new(skill_tool));
        if let Some(runner) = local_invocation_runner {
            // A saved Preset owns initial/on-demand placement. Standalone
            // sessions retain the usual deferred delegation tool.
            let deferred = !self.config.tools.enforce_builtin_allowlist
                || self.config.tools.deferred_allowlist.iter().any(|name| name == "nomi_delegate");
            registry.register(Box::new(
                crate::local_delegate_tool::LocalDelegateTool::new(runner).with_deferred(deferred),
            ));
        }

        let plan_active_flag = Arc::new(AtomicBool::new(false));
        if self.config.plan.enabled {
            registry.register(Box::new(crate::plan::tools::EnterPlanModeTool::new(
                Arc::clone(&plan_active_flag),
            )));
            registry.register(Box::new(crate::plan::tools::ExitPlanModeTool::new(
                Arc::clone(&plan_active_flag),
            )));
        }

        #[cfg(feature = "computer-use")]
        if self.config.tools.computer.enabled {
            tracing::info!(
                target: "nomi_agent",
                "computer-use ENABLED: registering the Computer tool (observe / click_element / \
                 launch / type / scroll). Desktop control is available to this session."
            );
            registry.register(Box::new(nomi_computer::ComputerTool::new(
                &self.config.tools.computer,
            )));
        }
        #[cfg(feature = "computer-use")]
        if !self.config.tools.computer.enabled {
            tracing::info!(
                target: "nomi_agent",
                "computer-use DISABLED for this session (config.tools.computer.enabled = false); \
                 the Computer tool is NOT registered — the agent falls back to the shell."
            );
        }
        #[cfg(not(feature = "computer-use"))]
        if self.config.tools.computer.enabled {
            tracing::warn!(
                target: "nomi_agent",
                "computer use enabled in config but this build lacks the computer-use feature"
            );
        }

        // codex-style stateless todo checklist tool. Always registered (not
        // deferred), surfaced to the frontend via the Plan event bridge.
        registry.register(Box::new(nomi_tools::update_plan::UpdatePlanTool::new()));

        // The MCP connection is established earlier for skill discovery, but
        // proxy registration remains after native bootstrap tools. Every MCP
        // proxy now owns an origin-stable reserved provider name, so neither
        // registration order nor a native ToolSearch can change its routing.
        let deferred_state = registry.deferred_state();
        registry.register(Box::new(nomi_tools::tool_search::ToolSearchTool::new(
            deferred_state,
        )));
        if let Some(manager) = &mcp_manager {
            let proxy_deferred = self
                .config
                .tools
                .deferred_allowlist
                .iter()
                .any(|name| name == "mcp.tool_proxy");
            let mut server_configs = self.config.mcp.servers.clone();
            if proxy_deferred {
                for config in server_configs.values_mut() {
                    config.deferred = Some(true);
                }
            }
            nomi_mcp::tool_proxy::register_mcp_tools(
                &mut registry,
                manager,
                &server_configs,
            );
        }

        // Per-node 工具白名单（受限角色的编排 worker）：非空时只保留白名单内的
        // 工具（含 MCP 代理）。放在全部注册之后、引擎构造之前；registry 会同步
        // 实时 deferred catalog，后续动态注册也能被搜索。ToolSearch 与旧顺序一致，
        // 始终保留；空 = 不限制（默认）。
        let mut allowed_tools = self.config.tools.builtin_allowlist.clone();
        // `mcp.tool_proxy` is a canonical capability marker, not a provider
        // tool name. Expand it only to the origin-stable names discovered from
        // MCP servers already resolved into this exact Session. A model cannot
        // use the marker to select or configure another server.
        expand_mcp_tool_proxy_allowlist(
            &mut allowed_tools,
            mcp_managers.iter().flat_map(|manager| {
                manager
                    .all_tools()
                    .into_iter()
                    .map(|(server, tool)| (server, tool.name.as_str()))
            }),
        );
        if !allowed_tools.is_empty() && !allowed_tools.iter().any(|name| name == "ToolSearch") {
            allowed_tools.push("ToolSearch".to_owned());
        }
        if self.config.tools.enforce_builtin_allowlist {
            registry.retain_only_named(&allowed_tools);
        } else {
            registry.retain_named(&allowed_tools);
        }

        anyhow::ensure!(host_skill_commands.is_empty() || registry.get("Skill").is_some(),
            "Hosted Skills require Skill in the Session tool policy");
        let mut engine = if let Some(session) = self.resume_session {
            AgentEngine::resume_with_provider(
                provider.clone(),
                self.config,
                registry,
                self.output,
                session,
                cwd_path.to_path_buf(),
            )
        } else {
            AgentEngine::new_with_provider(
                provider.clone(),
                self.config,
                registry,
                self.output,
                cwd_path.to_path_buf(),
            )
        };
        for command in host_skill_commands {
            engine.register_command(command)?;
        }
        if ssh_backend.is_some() {
            engine.set_remote_completion_evidence();
        }
        engine.set_plan_active_flag(plan_active_flag);
        engine.set_process_supervisor(Arc::clone(&process_supervisor));
        if let Some(spec) = self.goal {
            engine.set_goal(spec.objective, spec.max_auto_continuations);
        }

        Ok(BootstrapResult {
            engine,
            provider,
            mcp_managers,
            has_mcp,
        })
    }
}
