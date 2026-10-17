//! Instruction reads use the same authorized Kernel tool as model reads.
//! There is no native filesystem fallback or implicit grant of fs.read.
use async_trait::async_trait;
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::ChatToolCall;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, Ordering};
use tokio_util::sync::CancellationToken;

#[path = "search_context.rs"]
mod search_context;

use crate::{
    AgentsMdPolicy, AgentEngineError, AgentEngineEvent, AgentEventSink, AgentToolInvoker,
    AgentTurnRequest, AgentWorkspaceReader,
};

/// Identity is part of an instruction observation, never an authorization mode.
/// Keep it attached to scopes and layers so a privileged read cannot satisfy a
/// later ordinary-user preflight for the same native path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum InstructionIdentity {
    #[default]
    CurrentUser,
    Administrator,
}

impl InstructionIdentity {
    fn from_arguments(arguments: &serde_json::Value) -> Result<Self, AgentEngineError> {
        arguments.get("execution_identity").map_or(Ok(Self::CurrentUser), |value| {
            serde_json::from_value(value.clone()).map_err(|_| AgentEngineError::WorkspaceContext(
                "invalid instruction execution identity".into(),
            ))
        })
    }

    fn label(self) -> &'static str {
        match self { Self::CurrentUser => "current_user", Self::Administrator => "administrator" }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct InstructionScope {
    identity: InstructionIdentity,
    path: String,
}

impl InstructionScope {
    fn new(identity: InstructionIdentity, path: impl Into<String>) -> Self {
        Self { identity, path: path.into() }
    }
}

struct InstructionReader<'a> {
    authority: &'a InstructionAuthority,
    identity: InstructionIdentity,
    tools: &'a dyn AgentToolInvoker,
    sink: &'a dyn AgentEventSink,
    sequence: &'a AtomicU32,
}

/// Shared ancestors are read only once per refresh. Never retain this cache
/// across effects or model rounds; absence is also just a per-refresh fact.
struct RefreshReader<'a> {
    inner: InstructionReader<'a>,
    cached: std::sync::Mutex<BTreeMap<String, Option<String>>>,
}

#[async_trait]
impl AgentWorkspaceReader for RefreshReader<'_> {
    async fn read_text(
        &self,
        path: &str,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, AgentEngineError> {
        if cancellation.is_cancelled() {
            return Err(AgentEngineError::Cancelled);
        }
        {
            let cached = self.cached.lock().map_err(|_| {
                AgentEngineError::WorkspaceContext("instruction cache unavailable".into())
            })?;
            if let Some(content) = cached.get(path) {
                return Ok(content.clone());
            }
        }
        let content = self.inner.read_text(path, cancellation).await?;
        // The caller uses a 16 KiB per-file limit. Oversized responses need no
        // cached copy: load_agents_md reports truncation and refresh rejects it.
        if content.as_ref().is_none_or(|text| text.len() <= 16 * 1024) {
            let mut cached = self.cached.lock().map_err(|_| {
                AgentEngineError::WorkspaceContext("instruction cache unavailable".into())
            })?;
            if cached.len() >= 4096
                || cached
                    .iter()
                    .map(|(path, text)| path.len() + text.as_ref().map_or(0, String::len))
                    .sum::<usize>()
                    + path.len()
                    + content.as_ref().map_or(0, String::len)
                    > 256 * 1024
            {
                return Err(AgentEngineError::WorkspaceContext(
                    "instruction refresh cache budget exhausted".into(),
                ));
            }
            cached.insert(path.to_owned(), content.clone());
        }
        Ok(content)
    }
}

#[async_trait]
impl AgentWorkspaceReader for InstructionReader<'_> {
    async fn read_text(
        &self,
        path: &str,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, AgentEngineError> {
        let mut assembled = String::new();
        let mut digest: Option<String> = None;
        let mut total_bytes = None;
        // At most 16 KiB of instructions. The host may shorten a page for
        // JSON escaping; continue by its exact byte cursor, never by length
        // guesses or line counts. Bound faulty/custom host implementations.
        for _ in 0..16 {
            let Some(value) = self
                .read_page(
                    path,
                    assembled.len(),
                    digest.as_deref(),
                    cancellation.clone(),
                )
                .await?
            else {
                if assembled.is_empty() && digest.is_none() {
                    return Ok(None);
                }
                return Err(AgentEngineError::WorkspaceContext(
                    "Instruction file disappeared during paged read".into(),
                ));
            };
            let invalid = || {
                AgentEngineError::WorkspaceContext(
                    "workspace.files/read returned an inconsistent instruction page".into(),
                )
            };
            let content = value
                .get("content")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(invalid)?;
            let sha = value
                .get("sha256")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(invalid)?;
            let size = value
                .get("total_bytes")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(invalid)?;
            let offset = value
                .get("offset")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(invalid)?;
            let eof = value
                .get("eof")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(invalid)?;
            if size > 16 * 1024 {
                return Err(AgentEngineError::WorkspaceContext(
                    "Instruction file exceeds 16 KiB; no partial rules were loaded".into(),
                ));
            }
            if value.get("path").and_then(serde_json::Value::as_str) != Some(path)
                || sha.len() != 64
                || !sha
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || digest.as_ref().is_some_and(|expected| expected != sha)
                || total_bytes.is_some_and(|expected| expected != size)
                || offset != assembled.len() as u64
                || assembled.len().saturating_add(content.len()) > size as usize
            {
                return Err(invalid());
            }
            digest = Some(sha.to_owned());
            total_bytes = Some(size);
            assembled.push_str(content);
            if eof {
                if assembled.len() as u64 != size
                    || value.get("next_offset") != Some(&serde_json::Value::Null)
                {
                    return Err(invalid());
                }
                return Ok(Some(assembled));
            }
            if content.is_empty()
                || value.get("next_offset").and_then(serde_json::Value::as_u64)
                    != Some(assembled.len() as u64)
            {
                return Err(invalid());
            }
        }
        Err(AgentEngineError::WorkspaceContext(
            "Instruction page count exceeded; no partial rules were loaded".into(),
        ))
    }
}

impl InstructionReader<'_> {
    async fn read_page(
        &self,
        path: &str,
        offset: usize,
        digest: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<Option<serde_json::Value>, AgentEngineError> {
        let mut arguments =
            serde_json::json!({"path": path, "offset": offset, "limit": 16384, "missing_ok": true});
        if let Some(digest) = digest {
            arguments["expected_sha256"] = digest.into();
        }
        let value = self.invoke(arguments, cancellation).await?;
        if value.get("kind").and_then(|value| value.as_str()) == Some("workspace_file_absent") {
            if value.get("path").and_then(|value| value.as_str()) != Some(path) {
                return Err(AgentEngineError::WorkspaceContext(
                    "instruction absence belongs to another path".into(),
                ));
            }
            return Ok(None);
        }
        Ok(Some(value))
    }

    async fn invoke(
        &self,
        mut arguments: serde_json::Value,
        cancellation: CancellationToken,
    ) -> Result<serde_json::Value, AgentEngineError> {
        // CurrentUser is the formal wire default. Omitting it also keeps
        // ordinary reads valid for a frozen strict schema that predates this
        // optional selector; administrator reads never downgrade that way.
        if self.identity == InstructionIdentity::Administrator {
            arguments["execution_identity"] = serde_json::json!(self.identity);
        }
        if cancellation.is_cancelled() {
            return Err(AgentEngineError::Cancelled);
        }
        let binding = self
            .authority
            .tool_plan
            .binding("read_file")
            .ok_or_else(|| AgentEngineError::ToolNotExposed("read_file".into()))?;
        if binding.action_id.as_ref() != "workspace.files/read" {
            return Err(AgentEngineError::WorkspaceContext(
                "read_file is not bound to workspace.files/read".into(),
            ));
        }
        if self.identity == InstructionIdentity::Administrator {
            let declares_administrator = binding.definition.input_schema.0
                .pointer("/properties/execution_identity/enum").and_then(serde_json::Value::as_array)
                .is_some_and(|values| values.iter().any(|value| value == "administrator"));
            let accepted = declares_administrator && jsonschema::options()
                .with_retriever(crate::tool_validation::NoExternalSchemaReads)
                .build(&binding.definition.input_schema.0)
                .is_ok_and(|validator| validator.is_valid(&arguments));
            if !accepted {
                return Err(AgentEngineError::WorkspaceContext(
                    "the frozen read_file schema does not admit administrator instruction acquisition; a current canonical binding transition is required".into(),
                ));
            }
        }
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        if sequence >= 4096 {
            return Err(AgentEngineError::WorkspaceContext(
                "instruction read budget exhausted".into(),
            ));
        }
        let call = ChatToolCall {
            call_id: format!("agent-instructions:{sequence}").into(),
            name: "read_file".into(),
            arguments: StrictJsonValue(arguments),
            provider_metadata: None,
        };
        self.sink
            .emit(AgentEngineEvent::ToolCallCompleted {
                step: 0,
                call: call.clone(),
            })
            .await?;
        self.sink
            .emit(AgentEngineEvent::ToolStarted {
                step: 0,
                call_id: call.call_id.clone(),
                capability_id: binding.capability_id.clone(),
                action_id: binding.action_id.clone(),
            })
            .await?;
        let invocation = crate::tool::invocation_for(
            self.authority.causality.agent_session_id.clone(),
            self.authority.principal.clone(),
            self.authority.causality.resolved_snapshot_ref.clone(),
            self.authority.active_set_generation,
            &self.authority.causality.turn_operation_id,
            call.clone(),
            binding,
        );
        let result = match self.tools.invoke(invocation, cancellation).await {
            Ok(result) => result,
            Err(error @ (AgentEngineError::Cancelled | AgentEngineError::EventSink(_))) => {
                return Err(error);
            }
            Err(error) => {
                crate::AgentToolResult::text(call.call_id.clone(), error.to_string(), true)
            }
        };
        result.validate_for(&call.call_id)?;
        self.sink
            .emit(AgentEngineEvent::ToolCompleted { runtime_hold: None,
                step: 0,
                result: result.clone(),
            })
            .await?;
        let [nomifun_chat_model_broker::ChatToolResultPart::Text { text }] =
            result.output.as_slice()
        else {
            return Err(AgentEngineError::WorkspaceContext(
                "instruction response must contain one JSON text part".into(),
            ));
        };
        if text.len() > 64 * 1024 {
            return Err(AgentEngineError::WorkspaceContext(
                "instruction response exceeds the bounded envelope".into(),
            ));
        }
        if result.is_error {
            return Err(AgentEngineError::WorkspaceContext(text.clone()));
        }
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| AgentEngineError::WorkspaceContext(error.to_string()))?;
        Ok(value)
    }
}

/// A turn-scoped, bounded instruction view. Re-read after effects: an edit or
/// command can itself change AGENTS.md. New rules are delivered before retry.
pub(crate) struct ScopedInstructions {
    authority: InstructionAuthority,
    sequence: AtomicU32,
    layers: BTreeMap<InstructionScope, String>,
    // Remember inspected scopes even when no file existed. A later command
    // can create a new instruction file in one of these directories.
    directories: BTreeSet<InstructionScope>,
    // Canonical roots of recursive inspections, re-scanned after effects so a
    // newly created descendant instruction file cannot remain invisible.
    recursive_scopes: BTreeSet<InstructionScope>,
    dirty: bool,
    loaded: bool,
}

struct InstructionAuthority {
    tool_plan: crate::AgentToolPlan,
    principal: nomifun_agent_contracts::PrincipalRef,
    causality: nomifun_chat_model_broker::ChatCausality,
    active_set_generation: u64,
}

struct DiscoveredScope {
    canonical_path: String,
    kind: String,
    directories: BTreeSet<String>,
}

#[derive(serde::Deserialize)]
struct ScopeResponse {
    path: String,
    canonical_path: String,
    kind: String,
    recursive: bool,
    directories: Vec<String>,
    complete: bool,
    incomplete_reasons: Vec<String>,
}

impl ScopedInstructions {
    async fn discover_scope(
        &self,
        path: &str,
        recursive: bool,
        identity: InstructionIdentity,
        tools: &dyn AgentToolInvoker,
        sink: &dyn AgentEventSink,
        cancellation: CancellationToken,
    ) -> Result<DiscoveredScope, AgentEngineError> {
        let requested = if path.is_empty() { "." } else { path };
        let reader = InstructionReader {
            authority: &self.authority,
            identity,
            tools,
            sink,
            sequence: &self.sequence,
        };
        let value = reader
            .invoke(
                serde_json::json!({
                    "format": "instruction_scope", "path": requested, "recursive": recursive,
                }),
                cancellation.clone(),
            )
            .await?;
        if cancellation.is_cancelled() {
            return Err(AgentEngineError::Cancelled);
        }
        let scope: ScopeResponse = serde_json::from_value(value).map_err(|_| {
            AgentEngineError::WorkspaceContext("invalid instruction scope response".into())
        })?;
        if scope.path != requested
            || scope.recursive != recursive
            || !matches!(scope.kind.as_str(), "file" | "directory" | "missing")
            || crate::agents_md::normalize_native_instruction_path(&scope.canonical_path)?
                != scope.canonical_path
        {
            return Err(AgentEngineError::WorkspaceContext(
                "instruction scope does not match the requested observation".into(),
            ));
        }
        if !scope.complete || !scope.incomplete_reasons.is_empty() {
            // Reasons originate at the owner, but are still data. Do not turn
            // arbitrary host response text into an instruction to the model.
            return Err(AgentEngineError::WorkspaceContext(format!(
                "instruction discovery is incomplete for {}; narrow the target or resolve unreadable/link/budget boundaries before retrying",
                serde_json::to_string(requested).unwrap_or_default(),
            )));
        }
        let base = if scope.kind == "directory" { scope.canonical_path.clone() }
            else { crate::agents_md::parent_directory(&scope.canonical_path) };
        if scope.directories.is_empty() || scope.directories.len() > 64 {
            return Err(AgentEngineError::WorkspaceContext(
                "invalid instruction directory count".into(),
            ));
        }
        let mut directories = BTreeSet::new();
        for directory in scope.directories {
            if crate::agents_md::normalize_native_instruction_path(&directory)? != directory
                || (directory != base
                    && !(recursive && scope.kind == "directory" && in_scope(&directory, &base)))
                || !directories.insert(directory)
            {
                return Err(AgentEngineError::WorkspaceContext(
                    "inconsistent instruction directories".into(),
                ));
            }
        }
        if !directories.contains(&base) {
            return Err(AgentEngineError::WorkspaceContext(
                "instruction scope omitted its base directory".into(),
            ));
        }
        Ok(DiscoveredScope {
            canonical_path: scope.canonical_path,
            kind: scope.kind,
            directories,
        })
    }

    pub(crate) fn new(request: &AgentTurnRequest) -> Self {
        Self {
            authority: InstructionAuthority {
                tool_plan: request.tool_plan.clone(),
                principal: request.principal.clone(),
                causality: request.model_request.causality.clone(),
                active_set_generation: request.active_set_generation,
            },
            sequence: AtomicU32::new(100),
            directories: BTreeSet::from([InstructionScope::new(InstructionIdentity::CurrentUser, "")]),
            recursive_scopes: BTreeSet::new(),
            // Materialize the bound workspace root before the first model
            // request. Otherwise the first legitimate source read discovers
            // AGENTS.md only after the model has already proposed its batch,
            // forcing a visible deferral before any work can begin.
            dirty: true,
            loaded: false,
            layers: BTreeMap::new(),
        }
    }

    pub(crate) fn has_context(&self) -> bool {
        !self.layers.is_empty()
    }

    pub(crate) fn restore_sequence(&self, next: u32) {
        self.sequence.store(next.max(100), Ordering::Relaxed);
    }

    pub(crate) fn context(&self) -> String {
        Self::render_layers(&self.layers)
    }

    fn render_layers(layers: &BTreeMap<InstructionScope, String>) -> String {
        if layers.is_empty() {
            return "No repository instruction layers are currently available; no additional permissions are implied.".into();
        }
        let mut layers = layers.iter().collect::<Vec<_>>();
        layers.sort_by(|(left, _), (right, _)| {
            left.path.matches('/')
                .count()
                .cmp(&right.path.matches('/').count())
                .then_with(|| left.cmp(right))
        });
        layers.into_iter().map(|(path, content)| format!("Repository instructions observed as {} and scoped to the directory containing {} and its descendants (deeper layers take precedence for that identity; no permission grant):\n{content}", path.identity.label(), serde_json::to_string(&path.path).expect("path serializes")))
            .collect::<Vec<_>>().join("\n\n")
    }

    pub(crate) fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Refresh before planning, compaction and completion, not just before the
    /// next filesystem call. Reads retain the current Kernel authority.
    pub(crate) async fn before_model(
        &mut self,
        tools: &dyn AgentToolInvoker,
        sink: &dyn AgentEventSink,
        cancellation: CancellationToken,
    ) -> Result<bool, AgentEngineError> {
        if !self.dirty || self.authority.tool_plan.binding("read_file").is_none() {
            return Ok(false);
        }
        let mut directories = self.directories.clone();
        for path in self.recursive_scopes.clone() {
            let scope = self
                .discover_scope(&path.path, true, path.identity, tools, sink, cancellation.clone())
                .await?;
            if scope.canonical_path != path.path {
                return Err(AgentEngineError::WorkspaceContext(
                    "previously inspected recursive scope changed its canonical path".into(),
                ));
            }
            directories.extend(scope.directories.into_iter().map(|directory| InstructionScope::new(path.identity, directory)));
        }
        // Keep previously inspected directories in this refresh too: removed
        // descendant instructions must be read as absent and evicted, not left
        // in context just because they disappeared from the new metadata scan.
        let changed = self.refresh(directories, tools, sink, cancellation).await?;
        self.dirty = false;
        Ok(changed)
    }

    pub(crate) async fn before_calls(
        &mut self,
        calls: &[ChatToolCall],
        tools: &dyn AgentToolInvoker,
        sink: &dyn AgentEventSink,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, AgentEngineError> {
        let mut directories = BTreeSet::new();
        let mut scopes = BTreeMap::<InstructionScope, bool>::new();
        for call in calls {
            let Some(binding) = self.authority.tool_plan.binding(&call.name) else {
                continue;
            };
            let value = &call.arguments.0;
            // The caller has already validated the complete batch against its
            // frozen schema. Only actions with the formal identity selector
            // can request privileged instruction acquisition.
            let identity = if matches!(binding.action_id.as_ref(),
                "workspace.files/read" | "workspace.process/exec" | "workspace.process/start") {
                InstructionIdentity::from_arguments(value)?
            } else { InstructionIdentity::CurrentUser };
            let mut paths = Vec::new();
            match binding.action_id.as_ref() {
                "workspace.files/read"
                | "workspace.files/write"
                | "workspace.files/delete"
                | "workspace.artifacts/publish" => {
                    if let Some(path) = value.get("path").and_then(|v| v.as_str()) {
                        let recursive = binding.action_id.as_ref() == "workspace.files/delete"
                            || (value.get("format").and_then(|value| value.as_str())
                                == Some("instruction_scope")
                                && value.get("recursive").and_then(|value| value.as_bool())
                                    == Some(true));
                        scopes
                            .entry(InstructionScope::new(identity, path))
                            .and_modify(|scan| *scan |= recursive)
                            .or_insert(recursive);
                    }
                }
                "workspace.files/search"
                | "workspace.vcs/status"
                | "workspace.vcs/diff"
                | "workspace.vcs/stage"
                | "workspace.vcs/commit"
                | "workspace.vcs/push" => {
                    let path = value
                        .get("path")
                        .and_then(|value| value.as_str())
                        .unwrap_or(".");
                    // Search applies hidden/ignore filters and may touch many
                    // files. Inspect its selected root here; after_call loads
                    // exact hit scopes before snippets enter the model view.
                    let recursive = binding.action_id.as_ref() == "workspace.vcs/stage";
                    scopes
                        .entry(InstructionScope::new(identity, path))
                        .and_modify(|scan| *scan |= recursive)
                        .or_insert(recursive);
                }
                "workspace.files/rename" => {
                    for field in ["path", "destination"] {
                        if let Some(path) = value.get(field).and_then(|value| value.as_str()) { paths.push(path); }
                    }
                }
                "workspace.files/patch" => {
                    if let Some(files) = value.get("files").and_then(|v| v.as_array()) {
                        for file in files {
                            if let Some(path) = file.get("path").and_then(|v| v.as_str()) {
                                paths.push(path);
                            }
                        }
                    }
                }
                "workspace.process/exec" | "workspace.process/start" => {
                    // A shell command is opaque. cwd instructions are known;
                    // arbitrary paths embedded in shell text are not inferred.
                    if value.get("process_id").is_none() {
                        scopes
                            .entry(InstructionScope::new(identity,
                                value
                                    .get("cwd")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(".")
                                    .to_owned(),
                            ))
                            .or_insert(false);
                    }
                }
                _ => {}
            }
            for path in paths {
                if let Err(error) = crate::agents_md::normalize_native_instruction_path(path) {
                    return Ok(Some(format!("Operation not executed: {error}")));
                }
                scopes.entry(InstructionScope::new(identity, path)).or_insert(false);
            }
        }
        if scopes.is_empty() {
            return Ok(None);
        }
        if self.authority.tool_plan.binding("read_file").is_none() {
            // No implicit workspace.files/read grant. Process-only Agents may still execute
            // under their own instructions; explicitly disclose missing rules.
            return Ok(None);
        }
        if scopes.len() > 64 {
            return Ok(Some("Requested calls deferred: too many instruction targets in one batch; split or narrow the operation. Earlier calls may already have run.".into()));
        }
        let mut redirect = None;
        let mut recursive = self.recursive_scopes.clone();
        for (path, scan) in scopes {
            let normalized = match crate::agents_md::normalize_native_instruction_path(&path.path) {
                Ok(path) => path,
                Err(error) => return Ok(Some(format!("Operations not executed: {error}"))),
            };
            let scope = match self
                .discover_scope(&normalized, scan, path.identity, tools, sink, cancellation.clone())
                .await
            {
                Ok(scope) => scope,
                Err(AgentEngineError::WorkspaceContext(error)) => {
                    return Ok(Some(format!("Operations not executed: {error}")));
                }
                Err(error) => return Err(error),
            };
            if scope.canonical_path != normalized {
                redirect = Some(format!(
                    "Requested calls deferred; earlier calls may already have run. Path {} resolves to {}. Reconsider the operation using this canonical native path and its scoped instructions; arguments were not rewritten.",
                    serde_json::to_string(&normalized).unwrap_or_default(),
                    serde_json::to_string(&scope.canonical_path).unwrap_or_default()
                ));
            }
            if scan {
                recursive.insert(InstructionScope::new(path.identity, scope.canonical_path));
            }
            directories.extend(scope.directories.into_iter().map(|directory| InstructionScope::new(path.identity, directory)));
        }
        if recursive.len() > 64 {
            return Ok(Some(
                "Requested calls deferred: recursive instruction scope budget exceeded; narrow the task. Earlier calls may already have run."
                    .into(),
            ));
        }
        let read_only_batch = calls.iter().all(|call| {
            self.authority.tool_plan.binding(&call.name).is_some_and(|binding|
                matches!(binding.effect_class, crate::AgentEffectClass::ReadOnly))
        });
        // The owner still resolves each explicit read target above. During a
        // read-only burst, already loaded instruction files cannot have been
        // changed by this turn; avoid re-reading the same ancestors on every
        // model step. Effects, commands, redirects and new directories retain
        // the normal fresh instruction check.
        if read_only_batch && self.loaded && !self.dirty && redirect.is_none()
            && recursive == self.recursive_scopes
            && directories.is_subset(&self.directories)
        {
            return Ok(None);
        }
        match self.refresh(directories, tools, sink, cancellation).await {
            Ok(changed) => {
                self.recursive_scopes = recursive;
                if changed {
                    Ok(Some(format!(
                        "Requested calls deferred; earlier calls may already have run. Repository instructions changed or a new directory scope was discovered. Read the current scoped instructions, reconsider the calls, and submit new call identities. {}",
                        redirect.unwrap_or_default()
                    )))
                } else {
                    Ok(redirect)
                }
            }
            Err(AgentEngineError::WorkspaceContext(error)) => {
                Ok(Some(format!("Operations not executed: {error}")))
            }
            Err(error) => Err(error),
        }
    }

    /// The host has already settled this read. This is a model-facing context
    /// projection, not a rollback or a claim that the search was not executed.
    /// A changed context or withheld result defers subsequent serial calls.
    pub(crate) async fn after_call(
        &mut self,
        call: &ChatToolCall,
        result: crate::AgentToolResult,
        tools: &dyn AgentToolInvoker,
        sink: &dyn AgentEventSink,
        cancellation: CancellationToken,
    ) -> Result<(crate::AgentToolResult, bool), AgentEngineError> {
        if !self
            .authority
            .tool_plan
            .binding(&call.name)
            .is_some_and(|binding| binding.action_id.as_ref() == "workspace.files/search")
        {
            return Ok((result, false));
        }
        result.validate_for(&call.call_id)?;
        if cancellation.is_cancelled() {
            return Err(AgentEngineError::Cancelled);
        }
        if result.is_error {
            return Ok((result, false));
        }
        let observed = async {
            let paths = search_context::hit_paths(call, &result)?;
            if paths.is_empty() {
                return Ok(false);
            }
            if !self
                .authority
                .tool_plan
                .binding("read_file")
                .is_some_and(|binding| binding.action_id.as_ref() == "workspace.files/read")
            {
                return Err(AgentEngineError::WorkspaceContext(
                    "workspace.files/read is not active".into(),
                ));
            }
            let mut directories = BTreeSet::new();
            // Reject broad results before extra reads. A directory may contain
            // many hits, but each file still needs an exact canonical lookup.
            let parents = paths
                .iter()
                .map(|path| {
                    InstructionScope::new(InstructionIdentity::CurrentUser, crate::agents_md::parent_directory(path))
                })
                .collect::<BTreeSet<_>>();
            if self.directories.union(&parents).count() > 64 {
                return Err(AgentEngineError::WorkspaceContext(
                    "search instruction scope budget exceeded".into(),
                ));
            }
            for path in paths {
                let scope = self
                    .discover_scope(&path, false, InstructionIdentity::CurrentUser, tools, sink, cancellation.clone())
                    .await?;
                if scope.canonical_path != path || scope.kind != "file" {
                    return Err(AgentEngineError::WorkspaceContext(
                        "search hit path changed or is no longer a file".into(),
                    ));
                }
                directories.extend(scope.directories.into_iter().map(|directory| InstructionScope::new(InstructionIdentity::CurrentUser, directory)));
            }
            self.refresh(directories, tools, sink, cancellation).await
        }
        .await;
        match observed {
            Ok(changed) => Ok((result, changed)),
            Err(AgentEngineError::WorkspaceContext(_)) => Ok((crate::AgentToolResult::text(
                call.call_id.clone(),
                serde_json::json!({
                    "kind": "search_context_withheld",
                    "search_executed": true,
                    "snippets_withheld": true,
                    "notice": "The search returned, but its hit envelope or required repository instruction scopes could not be accepted completely. No snippets from this result are supplied to the model. Narrow path/limit and resolve unreadable, changed, aliased or oversized instruction scopes. Reading instructions requires workspace.files/read in the frozen Agent selection; this turn cannot enable capabilities. Reconsider the search with a new call identity. This is not proof of absent matches or a rollback."
                }).to_string(), true), true)),
            Err(error) => Err(error),
        }
    }

    async fn refresh(
        &mut self,
        directories: BTreeSet<InstructionScope>,
        tools: &dyn AgentToolInvoker,
        sink: &dyn AgentEventSink,
        cancellation: CancellationToken,
    ) -> Result<bool, AgentEngineError> {
        let directories = directories
            .into_iter()
            .map(|directory| Ok(InstructionScope::new(directory.identity, crate::agents_md::normalize_native_instruction_path(&directory.path)?)))
            .collect::<Result<BTreeSet<_>, AgentEngineError>>()?;
        let known = self
            .directories
            .union(&directories)
            .cloned()
            .collect::<BTreeSet<_>>();
        if known.len() > 64 || self.sequence.load(Ordering::Relaxed) >= 4096 {
            return Err(AgentEngineError::WorkspaceContext(
                "repository instruction discovery budget exhausted".into(),
            ));
        }
        let mut next = self.layers.clone();
        // Each identity owns its reader/cache for this refresh. Ancestors are
        // shared only within the same identity, and no cache survives effects.
        for identity in [InstructionIdentity::CurrentUser, InstructionIdentity::Administrator] {
            let reader = RefreshReader {
                inner: InstructionReader {
                    authority: &self.authority,
                    identity,
                    tools,
                    sink,
                    sequence: &self.sequence,
                },
                cached: std::sync::Mutex::new(BTreeMap::new()),
            };
            for directory in directories.iter().filter(|directory| directory.identity == identity) {
                let context = crate::load_agents_md(
                    &reader,
                    &directory.path,
                    AgentsMdPolicy {
                        max_file_bytes: 16 * 1024,
                        max_total_bytes: 24 * 1024,
                        ..Default::default()
                    },
                    cancellation.clone(),
                ).await?;
                if !context.warnings.is_empty() {
                    return Err(AgentEngineError::WorkspaceContext(format!(
                        "required repository instructions could not be read completely: {}",
                        context.warnings.join("; ")
                    )));
                }
                // Updating one identity must never evict or replace another
                // identity's independent instruction observation.
                next.retain(|path, _| {
                    let parent = crate::agents_md::parent_directory(&path.path);
                    path.identity != identity || !in_scope(&directory.path, &parent)
                });
                for layer in context.layers {
                    next.insert(InstructionScope::new(identity, layer.path), layer.content);
                }
            }
        }
        if next
            .iter()
            .map(|(path, text)| path.path.len() + text.len() + 256)
            .sum::<usize>()
            > 48 * 1024
        {
            return Err(AgentEngineError::WorkspaceContext(
                "combined scoped instructions exceed the turn budget; narrow the task".into(),
            ));
        }
        if next != self.layers {
            sink.emit(AgentEngineEvent::InstructionsUpdated {
                context: Self::render_layers(&next),
            })
            .await?;
            self.layers = next;
            self.directories = known;
            self.loaded = true;
            return Ok(true);
        }
        self.directories = known;
        self.loaded = true;
        Ok(false)
    }
}

fn in_scope(directory: &str, root: &str) -> bool {
    crate::agents_md::path_in_scope(directory, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentEffectClass, AgentToolBinding, AgentToolInvocation, AgentToolPlan, AgentToolResult};
    use nomifun_agent_contracts::{ChatRouteIdentity, DigestHex, ResolvedSnapshotRef};
    use nomifun_chat_model_broker::{ChatCausality, ChatToolDefinition};
    use serde_json::{Value, json};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<AgentEngineEvent>>);
    #[async_trait]
    impl AgentEventSink for RecordingSink {
        async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    struct IdentityTool {
        calls: Mutex<Vec<Value>>,
        content: Mutex<String>,
        failure: Mutex<Option<&'static str>>,
    }
    impl Default for IdentityTool {
        fn default() -> Self {
            Self { calls: Mutex::default(), content: Mutex::new("ROOT_INSTRUCTION_A".into()), failure: Mutex::default() }
        }
    }
    #[async_trait]
    impl AgentToolInvoker for IdentityTool {
        async fn invoke(&self, invocation: AgentToolInvocation, _: CancellationToken) -> Result<AgentToolResult, AgentEngineError> {
            assert_eq!(invocation.binding.action_id.as_ref(), "workspace.files/read");
            let args = &invocation.call.arguments.0;
            self.calls.lock().unwrap().push(args.clone());
            let path = args["path"].as_str().unwrap();
            let identity = InstructionIdentity::from_arguments(args).unwrap();
            let protected = path.starts_with("/protected/");
            let value = if args["format"] == "instruction_scope" {
                let directory = if path == "/protected" { path.to_owned() } else { crate::agents_md::parent_directory(path) };
                json!({"path":path,"canonical_path":path,"kind":if path == "/protected" {"directory"} else {"file"},
                    "recursive":args["recursive"],"directories":[directory],"complete":true,"incomplete_reasons":[]})
            } else if protected && identity == InstructionIdentity::CurrentUser {
                return Ok(AgentToolResult::text(invocation.call.call_id, "Permission denied (os error 13)", true));
            } else if path == "/protected/AGENTS.md" {
                if let Some(error) = *self.failure.lock().unwrap() {
                    return Ok(AgentToolResult::text(invocation.call.call_id, error, true));
                }
                let content = self.content.lock().unwrap().clone();
                json!({"kind":"workspace_file_text_page","path":path,"content":content,"offset":0,
                    "total_bytes":content.len(),"eof":true,"next_offset":null,
                    "sha256":nomifun_agent_contracts::digest_bytes(content.as_bytes())})
            } else { json!({"kind":"workspace_file_absent","path":path}) };
            Ok(AgentToolResult::text(invocation.call.call_id, value.to_string(), false))
        }
    }

    fn binding(name: &str, action: &str, effect_class: AgentEffectClass) -> AgentToolBinding {
        let capability = if action.starts_with("workspace.files/") { "workspace.files" } else { "workspace.process" };
        let definition = ChatToolDefinition { name: name.into(), description: "fixture".into(),
            input_schema: StrictJsonValue(json!({"type":"object","properties":{"execution_identity":{"enum":["current_user","administrator"]}}})), deferred: false };
        AgentToolBinding { model_name: name.into(), schema_digest: crate::tool::input_schema_digest(&definition.input_schema).unwrap(),
            canonical_input_schema_ref: format!("schema://{action}/input").into(), capability_contract_digest: DigestHex::from("c".repeat(64)),
            definition, capability_id: capability.into(), action_id: action.into(), resource_binding_ids: BTreeSet::new(),
            effect_class, parallel_safe: matches!(effect_class, AgentEffectClass::ReadOnly) }
    }

    fn instructions() -> ScopedInstructions {
        ScopedInstructions {
            authority: InstructionAuthority {
                tool_plan: AgentToolPlan::new([
                    binding("read_file", "workspace.files/read", AgentEffectClass::ReadOnly),
                    binding("start_process", "workspace.process/start", AgentEffectClass::ManagedEffect),
                ]).unwrap(),
                principal: nomifun_agent_contracts::PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() },
                causality: ChatCausality { agent_session_id: "session".into(), turn_operation_id: "turn".into(),
                    causation_event_id: "input".into(), resolved_snapshot_ref: ResolvedSnapshotRef {
                        snapshot_id: "snapshot".into(), snapshot_digest: DigestHex::from("b".repeat(64)) },
                    route_identity: ChatRouteIdentity::new("preset@1", "agent_chat", "route".into(), 1), operation_id: "model".into() },
                active_set_generation: 1,
            },
            sequence: AtomicU32::new(100), layers: BTreeMap::new(),
            directories: BTreeSet::from([InstructionScope::new(InstructionIdentity::CurrentUser, "")]),
            recursive_scopes: BTreeSet::new(), dirty: true, loaded: false,
        }
    }
    fn call(name: &str, identity: Option<&str>) -> ChatToolCall {
        let mut arguments = if name == "read_file" { json!({"path":"/protected/file"}) }
            else { json!({"command":"/bin/sh","cwd":"/protected"}) };
        if let Some(identity) = identity { arguments["execution_identity"] = identity.into(); }
        ChatToolCall { call_id: "proposed-call".into(), name: name.into(), arguments: StrictJsonValue(arguments), provider_metadata: None }
    }

    #[tokio::test]
    async fn administrator_preflight_reads_and_refreshes_through_its_canonical_identity() {
        let mut instructions = instructions();
        let tool = IdentityTool::default(); let sink = RecordingSink::default();
        instructions.before_model(&tool, &sink, CancellationToken::new()).await.unwrap();
        assert!(tool.calls.lock().unwrap().iter().all(|args| args.get("execution_identity").is_none()));
        let admin = call("start_process", Some("administrator"));
        let deferred = instructions.before_calls(&[admin.clone()], &tool, &sink, CancellationToken::new()).await.unwrap();
        assert!(deferred.is_some_and(|reason| reason.contains("submit new call identities")));
        assert!(instructions.context().contains("ROOT_INSTRUCTION_A"));
        assert!(instructions.context().contains("observed as administrator"));
        let admin_reads = tool.calls.lock().unwrap().iter().filter(|args| args["execution_identity"] == "administrator").cloned().collect::<Vec<_>>();
        assert!(admin_reads.iter().any(|args| args["format"] == "instruction_scope" && args["path"] == "/protected"));
        assert!(admin_reads.iter().any(|args| args["path"] == "/protected/AGENTS.md" && args["missing_ok"] == true));
        // A fresh call can execute only after the new context is published.
        assert!(instructions.before_calls(&[admin], &tool, &sink, CancellationToken::new()).await.unwrap().is_none());
        *tool.content.lock().unwrap() = "ROOT_INSTRUCTION_B".into();
        instructions.invalidate();
        assert!(instructions.before_model(&tool, &sink, CancellationToken::new()).await.unwrap());
        assert!(instructions.context().contains("ROOT_INSTRUCTION_B"));
        assert!(!instructions.context().contains("ROOT_INSTRUCTION_A"));
        let events = sink.0.lock().unwrap();
        assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::ToolCallCompleted { step: 0, call }
            if call.arguments.0["execution_identity"] == "administrator" && call.arguments.0["path"] == "/protected/AGENTS.md")));
        assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::InstructionsUpdated { context } if context.contains("ROOT_INSTRUCTION_B"))));
    }

    #[tokio::test]
    async fn current_user_cannot_borrow_privileged_scope_or_read_only_cache() {
        for explicit in [None, Some("current_user")] {
            let mut instructions = instructions(); let tool = IdentityTool::default(); let sink = RecordingSink::default();
            let admin = call("read_file", Some("administrator"));
            assert!(instructions.before_calls(&[admin.clone()], &tool, &sink, CancellationToken::new()).await.unwrap().is_some());
            assert!(instructions.before_calls(&[admin], &tool, &sink, CancellationToken::new()).await.unwrap().is_none());
            let previous = instructions.context();
            let ordinary = call("read_file", explicit);
            let refusal = instructions.before_calls(&[ordinary], &tool, &sink, CancellationToken::new()).await.unwrap().unwrap();
            assert!(refusal.contains("Permission denied"));
            assert_eq!(instructions.context(), previous, "a failed refresh cannot partially replace accepted layers");
            assert!(tool.calls.lock().unwrap().iter().any(|args| args["path"] == "/protected/AGENTS.md" && args.get("execution_identity").is_none()));
        }
    }

    #[tokio::test]
    async fn mixed_identity_batch_keeps_required_ordinary_denial() {
        let mut instructions = instructions(); let tool = IdentityTool::default(); let sink = RecordingSink::default();
        let refusal = instructions.before_calls(&[call("read_file", Some("administrator")), call("read_file", Some("current_user"))],
            &tool, &sink, CancellationToken::new()).await.unwrap().unwrap();
        assert!(refusal.contains("Permission denied"));
        assert!(!instructions.has_context(), "failed mixed preflight must not publish a privileged prefix");
    }

    #[tokio::test]
    async fn privileged_instruction_errors_and_oversize_remain_fail_closed() {
        for failure in [Some("file is not valid UTF-8"), Some("Permission denied (os error 13)"), None] {
            let mut instructions = instructions(); let tool = IdentityTool::default(); let sink = RecordingSink::default();
            *tool.failure.lock().unwrap() = failure;
            if failure.is_none() { *tool.content.lock().unwrap() = "x".repeat(16 * 1024 + 1); }
            let refusal = instructions.before_calls(&[call("start_process", Some("administrator"))], &tool, &sink, CancellationToken::new()).await.unwrap().unwrap();
            assert!(refusal.contains(failure.unwrap_or("exceeds 16 KiB")));
            assert!(!instructions.has_context());
        }
    }

    #[tokio::test]
    async fn invalid_identity_is_rejected_before_instruction_acquisition() {
        let mut instructions = instructions(); let tool = IdentityTool::default(); let sink = RecordingSink::default();
        assert!(instructions.before_calls(&[call("start_process", Some("root"))], &tool, &sink, CancellationToken::new()).await.is_err());
        assert!(tool.calls.lock().unwrap().is_empty());
        let mut legacy = self::instructions();
        let mut bindings = ["read_file", "start_process"].iter().map(|name| legacy.authority.tool_plan.binding(name).unwrap().clone()).collect::<Vec<_>>();
        let read = bindings.iter_mut().find(|binding| binding.model_name == "read_file").unwrap();
        read.definition.input_schema = StrictJsonValue(json!({"type":"object","additionalProperties":false,
            "properties":{"path":{"type":"string"},"format":{"type":"string"},"recursive":{"type":"boolean"},
                "missing_ok":{"type":"boolean"},"offset":{"type":"integer"},"limit":{"type":"integer"},"expected_sha256":{"type":"string"}}}));
        read.schema_digest = crate::tool::input_schema_digest(&read.definition.input_schema).unwrap();
        let old_schema = read.definition.input_schema.0.clone();
        legacy.authority.tool_plan = AgentToolPlan::new(bindings).unwrap();
        legacy.before_model(&tool, &sink, CancellationToken::new()).await.unwrap();
        let ordinary = ChatToolCall { arguments: StrictJsonValue(json!({"path":"plain.txt"})), ..call("read_file", None) };
        assert!(legacy.before_calls(&[ordinary], &tool, &sink, CancellationToken::new()).await.unwrap().is_none());
        let validator = jsonschema::validator_for(&old_schema).unwrap();
        assert!(tool.calls.lock().unwrap().iter().all(|args| validator.is_valid(args) && args.get("execution_identity").is_none()));
        let count = tool.calls.lock().unwrap().len();
        let refusal = legacy.before_calls(&[call("start_process", Some("administrator"))], &tool, &sink, CancellationToken::new()).await.unwrap().unwrap();
        assert!(refusal.contains("frozen read_file schema"));
        assert_eq!(tool.calls.lock().unwrap().len(), count, "unsupported administrator reads must not dispatch or retry as current_user");
    }
}
