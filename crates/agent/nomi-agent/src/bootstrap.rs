use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use nomi_config::config::Config;
use nomi_mcp::manager::McpManager;
use nomi_providers::LlmProvider;

use crate::engine::AgentEngine;
use crate::output::OutputSink;
use crate::session::Session;

/// **extract-llm: session-model adapter for `BrowserTool`'s extract seam.**
///
/// Wraps the session's [`LlmProvider`] + model params so `act(Extract)` can do real
/// LLM-driven structured extraction. The browser engine itself stays LLM-free — this
/// adapter lives at the bootstrap/facade layer (架构铁律). `complete(prompt)` issues a
/// minimal one-shot request — the (already spotlighted, untrusted-data-wrapped) extract
/// prompt as a single user message, no tools, no system prompt, no extended thinking
/// (extraction is mechanical) — and collects the streamed text deltas into the full
/// completion. Reuses the session's own model (user decision: "extract 用会话模型"), so
/// there is no separate model/cost surface. `None` provider → seam stays unwired (the
/// facade returns its deterministic `<data>` payload = zero-regression graceful degrade).
#[cfg(feature = "browser-use")]
struct SessionExtractModel {
    provider: Arc<dyn LlmProvider>,
    model: String,
    max_tokens: Option<u32>,
}

/// Drain a provider response for a tool-less, visible-text-only helper call.
/// `Done` is a commit candidate rather than an early return: the channel must
/// close cleanly before the candidate can be trusted, and any trailing event
/// invalidates it.
#[cfg(any(test, feature = "browser-use"))]
async fn collect_toolless_text_response(
    mut rx: tokio::sync::mpsc::Receiver<nomi_types::llm::LlmEvent>,
    context: &str,
) -> Result<String, String> {
    use nomi_types::llm::LlmEvent;
    use nomi_types::message::StopReason;

    let mut output = String::new();
    let mut terminal: Option<StopReason> = None;
    while let Some(event) = rx.recv().await {
        if terminal.is_some() {
            return Err(format!(
                "{context} provider emitted an event after terminal Done"
            ));
        }
        match event {
            LlmEvent::TextDelta(delta) => output.push_str(&delta),
            LlmEvent::Done { stop_reason, .. } => terminal = Some(stop_reason),
            LlmEvent::Error(error) => {
                return Err(format!("{context} provider stream failed: {error}"));
            }
            LlmEvent::ToolUse { .. }
            | LlmEvent::ToolUseDelta { .. }
            | LlmEvent::ToolUseTruncated { .. } => {
                return Err(format!(
                    "{context} provider emitted a tool call for a tool-less request"
                ));
            }
            LlmEvent::ProviderRoundId(_) => {
                return Err(format!(
                    "{context} provider emitted a retained round id for a non-retainable request"
                ));
            }
            LlmEvent::ThinkingDelta(_) | LlmEvent::ThinkingSignature(_) => {}
        }
    }

    let Some(stop_reason) = terminal else {
        return Err(format!(
            "{context} provider stream ended without terminal Done"
        ));
    };
    if stop_reason != StopReason::EndTurn {
        return Err(format!(
            "{context} provider stopped with {stop_reason:?}"
        ));
    }
    if output.trim().is_empty() {
        return Err(format!("{context} provider returned an empty response"));
    }
    Ok(output)
}

#[cfg(feature = "browser-use")]
#[async_trait::async_trait]
impl nomi_browser::extract::ExtractModel for SessionExtractModel {
    async fn complete(&self, prompt: &str) -> Result<String, String> {
        use nomi_types::llm::LlmRequest;
        use nomi_types::message::{ContentBlock, Message, Role};

        let request = LlmRequest {
            model: self.model.clone(),
            system: String::new(),
            messages: vec![Message::new(
                Role::User,
                vec![ContentBlock::Text { text: prompt.to_string() }],
            )],
            tools: Vec::new(),
            max_tokens: self.max_tokens,
            thinking: None,
            reasoning_effort: None,
            retain_provider_round: false,
        };
        let rx = self
            .provider
            .stream(&request)
            .await
            .map_err(|e| format!("extract model stream failed: {e}"))?;
        collect_toolless_text_response(rx, "extract model").await
    }
}

/// **P7B: session-model adapter for `BrowserTool`'s visual-fallback locator seam.**
///
/// Wraps the session's [`LlmProvider`] + model params so the facade can do vision-based
/// element location when DOM/aria anchoring fails (a `ref` went stale/detached). The
/// browser engine stays LLM-free — like [`SessionExtractModel`], this adapter lives at the
/// bootstrap/facade layer (架构铁律). `locate` sends one multimodal user message — a strict
/// "return JSON bounding box" instruction plus the (already engine-redacted) screenshot as a
/// [`ContentBlock::Image`] — and parses the model's pixel-box reply. No system prompt, no
/// tools, no extended thinking. Reuses the session's own model (same decision as Extract: no
/// separate model/cost surface). Gated behind `agent.browserUse.visualFallback` (host_default
/// OFF) because every fallback round-trips the vision model — a real token cost.
#[cfg(feature = "browser-use")]
struct SessionVisualLocator {
    provider: Arc<dyn LlmProvider>,
    model: String,
    max_tokens: Option<u32>,
}

#[cfg(feature = "browser-use")]
impl SessionVisualLocator {
    /// Shared multimodal round-trip: one user message (`prompt` text + a base64 PNG) → stream →
    /// collected text. No system prompt, no tools, no thinking (vision locating is mechanical).
    /// Both [`VisualLocator::locate`] (bbox) and [`VisualLocator::locate_labeled`] (SoM label)
    /// share this; only the prompt + reply parser differ.
    async fn run_vision(&self, prompt: String, png: &[u8]) -> Result<String, String> {
        use base64::Engine as _;
        use nomi_types::llm::LlmRequest;
        use nomi_types::message::{ContentBlock, Message, Role};

        let data = base64::engine::general_purpose::STANDARD.encode(png);
        let request = LlmRequest {
            model: self.model.clone(),
            system: String::new(),
            messages: vec![Message::new(
                Role::User,
                vec![
                    ContentBlock::Text { text: prompt },
                    ContentBlock::Image { media_type: "image/png".to_string(), data },
                ],
            )],
            tools: Vec::new(),
            max_tokens: self.max_tokens,
            thinking: None,
            reasoning_effort: None,
            retain_provider_round: false,
        };

        let rx = self
            .provider
            .stream(&request)
            .await
            .map_err(|e| format!("visual locator stream failed: {e}"))?;
        collect_toolless_text_response(rx, "visual locator").await
    }
}

#[cfg(test)]
mod toolless_response_tests {
    use super::collect_toolless_text_response;
    use nomi_types::llm::LlmEvent;
    use nomi_types::message::{StopReason, TokenUsage};

    async fn drain(events: Vec<LlmEvent>) -> Result<String, String> {
        let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));
        for event in events {
            tx.send(event).await.unwrap();
        }
        drop(tx);
        collect_toolless_text_response(rx, "test helper").await
    }

    fn done(stop_reason: StopReason) -> LlmEvent {
        LlmEvent::Done {
            stop_reason,
            usage: TokenUsage::default(),
        }
    }

    #[tokio::test]
    async fn toolless_helper_commits_only_a_clean_nonempty_end_turn() {
        let output = drain(vec![
            LlmEvent::TextDelta("complete".to_owned()),
            done(StopReason::EndTurn),
        ])
        .await
        .unwrap();
        assert_eq!(output, "complete");

        for events in [
            vec![
                LlmEvent::TextDelta("partial".to_owned()),
                done(StopReason::MaxTokens),
            ],
            vec![LlmEvent::TextDelta("partial".to_owned())],
            vec![
                LlmEvent::TextDelta("committed?".to_owned()),
                done(StopReason::EndTurn),
                LlmEvent::TextDelta("poison".to_owned()),
            ],
            vec![
                LlmEvent::TextDelta("payload".to_owned()),
                LlmEvent::ProviderRoundId("unexpected".to_owned()),
                done(StopReason::EndTurn),
            ],
        ] {
            assert!(drain(events).await.is_err());
        }
    }
}

#[cfg(feature = "browser-use")]
#[async_trait::async_trait]
impl nomi_browser::visual_fallback::VisualLocator for SessionVisualLocator {
    async fn locate(
        &self,
        screenshot: &[u8],
        instruction: &str,
    ) -> Result<nomi_browser::visual_fallback::VisualLocateResult, String> {
        // Strict-JSON instruction. The screenshot is at device-pixel resolution; the model
        // returns the box in that same image-pixel space (the facade divides by the live DPR
        // before dispatching the click). A not-found answer is `{"confidence": 0}`.
        let prompt = format!(
            "You are a precise UI element locator. The attached PNG is a screenshot of a web \
             page rendered at device-pixel resolution; the origin (0,0) is its top-left corner. \
             Find the single element described below and return its bounding box in IMAGE PIXEL \
             coordinates.\n\n\
             Target element: {instruction}\n\n\
             Respond with ONLY a JSON object — no markdown, no code fence, no prose:\n\
             {{\"x\": <left px>, \"y\": <top px>, \"width\": <px>, \"height\": <px>, \"confidence\": <0..1>}}\n\
             If you cannot confidently find the element, respond with {{\"confidence\": 0}}."
        );
        let out = self.run_vision(prompt, screenshot).await?;
        parse_visual_locate_result(&out)
    }

    async fn locate_labeled(
        &self,
        annotated_screenshot: &[u8],
        instruction: &str,
        n_labels: usize,
    ) -> Result<nomi_browser::visual_fallback::SomLabelResult, String> {
        // SoM mode: the screenshot has numbered labels (1..=n_labels) drawn on its clickable
        // elements. The model picks the label that matches the target — a finite choice, far
        // more reliable than free-form pixel regression. A not-found answer is `{"label": 0}`.
        let prompt = format!(
            "You are a precise UI element selector. The attached PNG is a screenshot of a web \
             page with numbered labels 1 to {n_labels} drawn on its interactive elements (each \
             label sits at the top-left of its element's box). Identify which single label \
             number marks the element described below.\n\n\
             Target element: {instruction}\n\n\
             Respond with ONLY a JSON object — no markdown, no code fence, no prose:\n\
             {{\"label\": <integer 1..{n_labels}>, \"confidence\": <0..1>}}\n\
             If none of the labeled elements matches, respond with {{\"label\": 0, \"confidence\": 0}}."
        );
        let out = self.run_vision(prompt, annotated_screenshot).await?;
        parse_som_locate_result(&out, n_labels)
    }
}

/// Parse the vision model's reply into a [`VisualLocateResult`](nomi_browser::visual_fallback::VisualLocateResult).
///
/// Tolerates models that wrap the JSON in prose or a ```` ```json ```` fence by extracting the
/// outermost `{...}` span. A reply with `confidence == 0`, a missing/zero box, or any
/// non-numeric field is treated as "element not found" (an `Err`) — the facade then surfaces
/// the original anchor error rather than clicking a bogus coordinate.
#[cfg(feature = "browser-use")]
fn parse_visual_locate_result(
    raw: &str,
) -> Result<nomi_browser::visual_fallback::VisualLocateResult, String> {
    use nomi_browser::visual_fallback::{PixelBox, VisualLocateResult};

    let trimmed = raw.trim();
    let start = trimmed
        .find('{')
        .ok_or_else(|| format!("visual locator returned no JSON object: {trimmed:.200}"))?;
    let end = trimmed
        .rfind('}')
        .filter(|e| *e >= start)
        .ok_or_else(|| format!("visual locator JSON span malformed: {trimmed:.200}"))?;
    let json_str = &trimmed[start..=end];
    let v: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("visual locator JSON parse failed ({e}): {json_str:.200}"))?;

    let confidence = v.get("confidence").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    let num = |k: &str| v.get(k).and_then(serde_json::Value::as_f64);
    match (num("x"), num("y"), num("width"), num("height")) {
        (Some(x), Some(y), Some(width), Some(height))
            if confidence > 0.0 && width > 0.0 && height > 0.0 =>
        {
            Ok(VisualLocateResult {
                pixel_box: PixelBox { x, y, width, height },
                confidence: confidence.clamp(0.0, 1.0),
            })
        }
        _ => Err(format!(
            "visual locator could not locate the element (confidence={confidence})"
        )),
    }
}

/// Parse the vision model's SoM reply into a [`SomLabelResult`](nomi_browser::visual_fallback::SomLabelResult).
///
/// Same JSON-extraction tolerance as [`parse_visual_locate_result`] (handles prose / ```` ```json ````
/// fences). The label MUST be an integer in `1..=n_labels` with `confidence > 0`; `label == 0`,
/// out-of-range, missing, or zero-confidence all map to `Err` ("no label matched") so the facade
/// surfaces the original anchor error rather than indexing a bogus / out-of-bounds label.
#[cfg(feature = "browser-use")]
fn parse_som_locate_result(
    raw: &str,
    n_labels: usize,
) -> Result<nomi_browser::visual_fallback::SomLabelResult, String> {
    use nomi_browser::visual_fallback::SomLabelResult;

    let trimmed = raw.trim();
    let start = trimmed
        .find('{')
        .ok_or_else(|| format!("SoM locator returned no JSON object: {trimmed:.200}"))?;
    let end = trimmed
        .rfind('}')
        .filter(|e| *e >= start)
        .ok_or_else(|| format!("SoM locator JSON span malformed: {trimmed:.200}"))?;
    let json_str = &trimmed[start..=end];
    let v: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("SoM locator JSON parse failed ({e}): {json_str:.200}"))?;

    let confidence = v.get("confidence").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    // Accept integer or float-encoded label; reject anything not in 1..=n_labels.
    let label = v.get("label").and_then(serde_json::Value::as_f64);
    match label {
        Some(l) if l.fract() == 0.0 && l >= 1.0 && (l as usize) <= n_labels && confidence > 0.0 => {
            Ok(SomLabelResult {
                label: l as usize,
                confidence: confidence.clamp(0.0, 1.0),
            })
        }
        _ => Err(format!(
            "SoM locator picked no valid label (label={label:?}, n_labels={n_labels}, confidence={confidence})"
        )),
    }
}

/// Result of bootstrapping an agent engine with all features initialized.
pub struct BootstrapResult {
    pub engine: AgentEngine,
    pub provider: Arc<dyn LlmProvider>,
    pub mcp_managers: Vec<Arc<McpManager>>,
    pub has_mcp: bool,
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
    goal: Option<crate::goal::runtime::GoalSpec>,
    /// Host composition switch for embedded AgentExecution. CLI
    /// and standalone embeddings default to installing it; backend sessions
    /// explicitly disable it when Platform Gateway owns persistent execution
    /// or the caller is outside the trusted local-owner boundary.
    install_embedded_agent_execution: bool,
    /// Machine-bound key for encrypted persistent-browser-login snapshots.
    #[cfg_attr(not(feature = "browser-use"), allow(dead_code))]
    persistent_login_key: Option<[u8; 32]>,
    /// Trusted main-process Browser Platform capability for this runtime.
    /// The embedded host resolves/signs caller identity before constructing
    /// this client; neither the model nor tool input can populate those fields.
    ///
    /// When present, the Browser tool uses this client as its only execution
    /// path and cannot lazily launch a private Chromium process.
    #[cfg(feature = "browser-use")]
    browser_lane_client: Option<nomi_browser::BrowserLaneClient>,
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
            goal: None,
            install_embedded_agent_execution: true,
            persistent_login_key: None,
            #[cfg(feature = "browser-use")]
            browser_lane_client: None,
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

    /// observe one mode cell with zero drift. Omit it (the default) to keep the
    /// Provide the application data key used to encrypt persistent-browser-login snapshots.
    pub fn persistent_login_key(mut self, key: [u8; 32]) -> Self {
        self.persistent_login_key = Some(key);
        self
    }

    /// Omit it (the default) → fail-closed (irreversible stays Blocked, gated egress fails).
    /// Bind the native Agent Browser tool to the process-wide Browser Session
    /// Hub. The supplied client already carries trusted caller identity,
    /// capability expiry, owner lease, and allowed operations; no corresponding
    /// fields are exposed to model input.
    #[cfg(feature = "browser-use")]
    pub fn browser_lane_client(mut self, client: nomi_browser::BrowserLaneClient) -> Self {
        self.browser_lane_client = Some(client);
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

        let memory_dir = nomi_memory::paths::auto_memory_dir(cwd_path);

        let file_cache = if self.config.file_cache.enabled {
            Some(Arc::new(std::sync::RwLock::new(
                nomi_tools::file_cache::FileStateCache::new(&self.config.file_cache),
            )))
        } else {
            None
        };

        let mut registry = nomi_tools::registry::ToolRegistry::new();
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
        let mcp_manager = if !self.config.mcp.servers.is_empty() {
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

        let skills = nomi_skills::loader::load_all_skills(
            cwd_path,
            &self.extra_skill_dirs,
            false,
            mcp_manager.as_deref(),
        )
        .await;

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

        let mut prompt_cache = crate::context::SystemPromptCache::new();
        prompt_cache.set_agents_md(agents_snapshot.formatted);
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
        let system_prompt = crate::context::build_system_prompt(
            &mut prompt_cache,
            self.config.system_prompt.as_deref(),
            cwd,
            &self.config.model,
            &skills,
            Some(self.config.compact.context_window),
            memory_dir.as_deref(),
            false,
            self.config.compact.toon,
            self.config.tools.browser.enabled,
        );
        self.config.system_prompt = Some(system_prompt);

        let skills_arc = Arc::new(skills);
        let skill_checker = nomi_skills::permissions::SkillPermissionChecker::new(self.config.tools.skills.deny.clone());
        // No-gateway CLI/embedded engines share one Agent invocation runner
        // between fork-mode skills and embedded `nomi_delegate`. Platform
        // Gateway sessions disable the embedded deployment and expose the same
        // AgentExecution contract through the platform, so a model sees one tool.
        let local_invocation_runner = if self.install_embedded_agent_execution {
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
        registry.register(Box::new(
            crate::skill_tool::SkillTool::with_invocation_runner(
                skills_arc,
                cwd.to_string(),
                skill_checker,
                None,
                skill_invocation_runner,
            )
            .with_process_supervisor(Arc::clone(&process_supervisor)),
        ));
        if let Some(runner) = local_invocation_runner {
            registry.register(Box::new(crate::local_delegate_tool::LocalDelegateTool::new(runner)));
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

        // Browser use. Production hosts inject a trusted BrowserLaneClient,
        // making the process-wide Browser Session Hub the sole owner/execution
        // path. AgentBootstrap never enables BrowserTool's standalone lazy
        // engine; tests and explicit low-level embeddings may still construct
        // BrowserTool directly outside this production bootstrap.
        #[cfg(feature = "browser-use")]
        if self.config.tools.browser.enabled {
            if self.browser_lane_client.is_some() {
                tracing::info!(
                    target: "nomi_agent",
                    "browser-use ENABLED: registering a Browser tool bound to the shared Browser \
                     Session Hub. Private Chromium fallback is disabled."
                );
            } else {
                tracing::error!(
                    target: "nomi_agent",
                    "browser-use ENABLED, but no BrowserLaneClient was issued. \
                     The Browser tool will fail closed; private Chromium fallback is disabled."
                );
            }
            // Thread host-resolved Browser security settings into the facade.
            // `browser.full_power` carries the current
            // `agent.browserUse.fullPower` preference.
            //
            // P3-G2: pass the session working directory `cwd` (= self.workspace) as the
            // per-session/per-pet workspace. It's the natural isolation point — for a
            // companion session it is `{companion_id}/workspace` (companion.rs sets
            // extra.workspace, which the manager resolves to this cwd); for a non-companion
            // session it's the conversation's own working dir. Downloads (E4) land in its
            // `downloads/` subdir instead of a temp dir. The non-companion
            // `{data_dir}/browser-profiles/{conversation_id}` subdivision (默认④) needs the
            // data_dir + conversation_id which the bootstrap does not hold (they live in the
            // upper manager/factory) — the cwd is already per-conversation isolated, so we
            // pass it directly (simplest correct) and leave the finer browser-profiles
            // layout to W4/deployment wiring (the field signature already supports it).
            // manager) → the facade falls back to the construction-time snapshot (unchanged).
            let mut browser_tool = nomi_browser::BrowserTool::new_managed(
                &self.config.tools.browser,
                self.config.tools.browser.full_power,
                self.config.tools.browser.persistent_login,
                Some(PathBuf::from(cwd)),
                self.persistent_login_key,
            );
            if let Some(client) = self.browser_lane_client.clone() {
                // Authoritative mode switch: once injected, BrowserTool::engine()
                // rejects facade-owned engine access before its cache/build path.
                browser_tool = browser_tool.with_lane_client(client);
            }
            // P7A: site-memory opt-in (LIVE pref `agent.browserUse.siteMemory`, default OFF). When
            // ON, inject a file-backed sink so the agent remembers site structure across sessions
            // (entries injected into observe as untrusted hints; secret-sourced entries dropped by
            // the store). Root is GLOBAL (browser identity is globally shared, NOT per-session) —
            // same `browser-data` root the gateway/engine use. OFF → no sink (zero behavior change).
            if self.config.tools.browser.site_memory {
                let sm_root = nomi_config::config::app_data_dir()
                    .join("browser-state")
                    .join("site-memory");
                let sink = nomi_browser::site_memory::FileSiteMemorySink::new(sm_root);
                let store = std::sync::Arc::new(nomi_browser::site_memory::SiteMemoryStore::new(
                    Box::new(sink),
                ));
                browser_tool = browser_tool.with_site_memory(store);
            }
            // extract-llm: reuse the session provider (the same `provider` driving this engine,
            // created above if none was injected) so act(Extract) does real LLM extraction. No
            // pref — extract is opt-in per-call by the agent; this only enables the capability.
            let extract_model = Arc::new(SessionExtractModel {
                provider: provider.clone(),
                model: self.config.model.clone(),
                max_tokens: self.config.output_max_tokens,
            });
            browser_tool = browser_tool.with_extract_model(extract_model);
            // P7B: visual fallback opt-in (LIVE pref `agent.browserUse.visualFallback`, default
            // OFF). When ON, inject a session-model `VisualLocator`: if DOM/aria anchoring fails
            // (a `ref` went stale/detached), the facade screenshots the page and asks the vision
            // model to locate the target by description, then clicks the DPR-mapped CSS point.
            // Reuses the session provider/model (no separate cost surface). OFF → no locator
            // injected, so the facade's fallback stays Unavailable (zero behavior change).
            if self.config.tools.browser.visual_fallback {
                let locator = Arc::new(SessionVisualLocator {
                    provider: provider.clone(),
                    model: self.config.model.clone(),
                    max_tokens: self.config.output_max_tokens,
                });
                browser_tool = browser_tool
                    .with_visual_locator(locator)
                    .with_visual_fallback_enabled(true);
            }
            registry.register(Box::new(browser_tool));
        }
        #[cfg(not(feature = "browser-use"))]
        if self.config.tools.browser.enabled {
            tracing::debug!(
                target: "nomi_agent",
                "browser-use enabled in config but this build lacks the browser-use feature; the \
                 native Browser tool is not registered."
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
            nomi_mcp::tool_proxy::register_mcp_tools(
                &mut registry,
                manager,
                &self.config.mcp.servers,
            );
        }

        // Per-node 工具白名单（受限角色的编排 worker）：非空时只保留白名单内的
        // 工具（含 MCP 代理）。放在全部注册之后、引擎构造之前；registry 会同步
        // 实时 deferred catalog，后续动态注册也能被搜索。ToolSearch 与旧顺序一致，
        // 始终保留；空 = 不限制（默认）。
        let mut allowed_tools = self.config.tools.builtin_allowlist.clone();
        if !allowed_tools.is_empty() && !allowed_tools.iter().any(|name| name == "ToolSearch") {
            allowed_tools.push("ToolSearch".to_owned());
        }
        registry.retain_named(&allowed_tools);

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

#[cfg(all(test, feature = "browser-use"))]
mod visual_locator_tests {
    use super::{parse_som_locate_result, parse_visual_locate_result};

    #[test]
    fn parses_clean_json_box() {
        let r = parse_visual_locate_result(
            r#"{"x": 100, "y": 200, "width": 40, "height": 20, "confidence": 0.9}"#,
        )
        .expect("clean JSON should parse");
        assert_eq!(r.pixel_box.x, 100.0);
        assert_eq!(r.pixel_box.y, 200.0);
        assert_eq!(r.pixel_box.width, 40.0);
        assert_eq!(r.pixel_box.height, 20.0);
        assert!((r.confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn extracts_json_from_code_fence() {
        // Vision models love wrapping JSON in a ```json fence — tolerate it.
        let raw = "```json\n{\"x\": 1, \"y\": 2, \"width\": 3, \"height\": 4, \"confidence\": 0.5}\n```";
        let r = parse_visual_locate_result(raw).expect("fenced JSON should parse");
        assert_eq!(r.pixel_box.width, 3.0);
    }

    #[test]
    fn extracts_json_from_surrounding_prose() {
        let raw = "Sure! Here is the box: {\"x\": 5, \"y\": 6, \"width\": 7, \"height\": 8, \"confidence\": 0.8} — hope that helps.";
        let r = parse_visual_locate_result(raw).expect("prose-wrapped JSON should parse");
        assert_eq!(r.pixel_box.x, 5.0);
    }

    #[test]
    fn confidence_clamped_to_unit_interval() {
        let r = parse_visual_locate_result(
            r#"{"x": 1, "y": 1, "width": 1, "height": 1, "confidence": 1.7}"#,
        )
        .expect("over-unit confidence should still parse");
        assert_eq!(r.confidence, 1.0);
    }

    #[test]
    fn not_found_confidence_zero_is_err() {
        assert!(parse_visual_locate_result(r#"{"confidence": 0}"#).is_err());
    }

    #[test]
    fn zero_sized_box_is_err() {
        // A box with no area can't be a click target → not-found.
        assert!(
            parse_visual_locate_result(
                r#"{"x": 1, "y": 1, "width": 0, "height": 10, "confidence": 0.9}"#
            )
            .is_err()
        );
    }

    #[test]
    fn missing_box_fields_is_err() {
        // Confidence present but no coordinates → can't act.
        assert!(parse_visual_locate_result(r#"{"confidence": 0.9}"#).is_err());
    }

    #[test]
    fn non_json_reply_is_err() {
        assert!(parse_visual_locate_result("I could not find that element.").is_err());
    }

    // ── SoM label parser (parse_som_locate_result) ──

    #[test]
    fn som_parses_valid_label() {
        let r = parse_som_locate_result(r#"{"label": 3, "confidence": 0.88}"#, 10)
            .expect("valid label should parse");
        assert_eq!(r.label, 3);
        assert!((r.confidence - 0.88).abs() < 1e-9);
    }

    #[test]
    fn som_extracts_label_from_code_fence() {
        let raw = "```json\n{\"label\": 7, \"confidence\": 0.6}\n```";
        let r = parse_som_locate_result(raw, 12).expect("fenced JSON should parse");
        assert_eq!(r.label, 7);
    }

    #[test]
    fn som_label_zero_is_err() {
        // {"label": 0} is the model's "none matched" sentinel.
        assert!(parse_som_locate_result(r#"{"label": 0, "confidence": 0}"#, 10).is_err());
    }

    #[test]
    fn som_label_out_of_range_is_err() {
        // Hallucinated label beyond n_labels must NOT index the label_map (would be OOB).
        assert!(parse_som_locate_result(r#"{"label": 99, "confidence": 0.9}"#, 12).is_err());
    }

    #[test]
    fn som_label_below_one_is_err() {
        assert!(parse_som_locate_result(r#"{"label": -1, "confidence": 0.9}"#, 12).is_err());
    }

    #[test]
    fn som_non_integer_label_is_err() {
        // A fractional label is not a valid 1-based index.
        assert!(parse_som_locate_result(r#"{"label": 2.5, "confidence": 0.9}"#, 12).is_err());
    }

    #[test]
    fn som_zero_confidence_is_err() {
        assert!(parse_som_locate_result(r#"{"label": 3, "confidence": 0}"#, 10).is_err());
    }

    #[test]
    fn som_missing_label_is_err() {
        assert!(parse_som_locate_result(r#"{"confidence": 0.9}"#, 10).is_err());
    }

    #[test]
    fn som_boundary_label_n_is_ok() {
        // label == n_labels is in range (inclusive upper bound).
        let r = parse_som_locate_result(r#"{"label": 5, "confidence": 0.7}"#, 5)
            .expect("label == n_labels is valid");
        assert_eq!(r.label, 5);
    }
}
