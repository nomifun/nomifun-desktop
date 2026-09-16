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
    AgentsMdPolicy, CodingEngineError, CodingEngineEvent, CodingEventSink, CodingToolInvoker,
    CodingTurnRequest, CodingWorkspaceReader,
};

struct InstructionReader<'a> {
    authority: &'a InstructionAuthority,
    tools: &'a dyn CodingToolInvoker,
    sink: &'a dyn CodingEventSink,
    sequence: &'a AtomicU32,
}

/// Shared ancestors are read only once per refresh. Never retain this cache
/// across effects or model rounds; absence is also just a per-refresh fact.
struct RefreshReader<'a> {
    inner: InstructionReader<'a>,
    cached: std::sync::Mutex<BTreeMap<String, Option<String>>>,
}

#[async_trait]
impl CodingWorkspaceReader for RefreshReader<'_> {
    async fn read_text(
        &self,
        path: &str,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, CodingEngineError> {
        if cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        {
            let cached = self.cached.lock().map_err(|_| {
                CodingEngineError::WorkspaceContext("instruction cache unavailable".into())
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
                CodingEngineError::WorkspaceContext("instruction cache unavailable".into())
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
                return Err(CodingEngineError::WorkspaceContext(
                    "instruction refresh cache budget exhausted".into(),
                ));
            }
            cached.insert(path.to_owned(), content.clone());
        }
        Ok(content)
    }
}

#[async_trait]
impl CodingWorkspaceReader for InstructionReader<'_> {
    async fn read_text(
        &self,
        path: &str,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, CodingEngineError> {
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
                return Err(CodingEngineError::WorkspaceContext(
                    "Instruction file disappeared during paged read".into(),
                ));
            };
            let invalid = || {
                CodingEngineError::WorkspaceContext(
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
                return Err(CodingEngineError::WorkspaceContext(
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
        Err(CodingEngineError::WorkspaceContext(
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
    ) -> Result<Option<serde_json::Value>, CodingEngineError> {
        let mut arguments =
            serde_json::json!({"path": path, "offset": offset, "limit": 16384, "missing_ok": true});
        if let Some(digest) = digest {
            arguments["expected_sha256"] = digest.into();
        }
        let value = self.invoke(arguments, cancellation).await?;
        if value.get("kind").and_then(|value| value.as_str()) == Some("workspace_file_absent") {
            if value.get("path").and_then(|value| value.as_str()) != Some(path) {
                return Err(CodingEngineError::WorkspaceContext(
                    "instruction absence belongs to another path".into(),
                ));
            }
            return Ok(None);
        }
        Ok(Some(value))
    }

    async fn invoke(
        &self,
        arguments: serde_json::Value,
        cancellation: CancellationToken,
    ) -> Result<serde_json::Value, CodingEngineError> {
        if cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        let binding = self
            .authority
            .tool_plan
            .binding("read_file")
            .ok_or_else(|| CodingEngineError::ToolNotExposed("read_file".into()))?;
        if binding.action_id.as_ref() != "workspace.files/read" {
            return Err(CodingEngineError::WorkspaceContext(
                "read_file is not bound to workspace.files/read".into(),
            ));
        }
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        if sequence >= 4096 {
            return Err(CodingEngineError::WorkspaceContext(
                "instruction read budget exhausted".into(),
            ));
        }
        let call = ChatToolCall {
            call_id: format!("coding-instructions:{sequence}").into(),
            name: "read_file".into(),
            arguments: StrictJsonValue(arguments),
            provider_metadata: None,
        };
        self.sink
            .emit(CodingEngineEvent::ToolCallCompleted {
                step: 0,
                call: call.clone(),
            })
            .await?;
        self.sink
            .emit(CodingEngineEvent::ToolStarted {
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
            Err(error @ (CodingEngineError::Cancelled | CodingEngineError::EventSink(_))) => {
                return Err(error);
            }
            Err(error) => {
                crate::CodingToolResult::text(call.call_id.clone(), error.to_string(), true)
            }
        };
        result.validate_for(&call.call_id)?;
        self.sink
            .emit(CodingEngineEvent::ToolCompleted {
                step: 0,
                result: result.clone(),
            })
            .await?;
        let [nomifun_chat_model_broker::ChatToolResultPart::Text { text }] =
            result.output.as_slice()
        else {
            return Err(CodingEngineError::WorkspaceContext(
                "instruction response must contain one JSON text part".into(),
            ));
        };
        if text.len() > 64 * 1024 {
            return Err(CodingEngineError::WorkspaceContext(
                "instruction response exceeds the bounded envelope".into(),
            ));
        }
        if result.is_error {
            return Err(CodingEngineError::WorkspaceContext(text.clone()));
        }
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| CodingEngineError::WorkspaceContext(error.to_string()))?;
        Ok(value)
    }
}

/// A turn-scoped, bounded instruction view. Re-read after effects: an edit or
/// command can itself change AGENTS.md. New rules are delivered before retry.
pub(crate) struct ScopedInstructions {
    authority: InstructionAuthority,
    sequence: AtomicU32,
    layers: BTreeMap<String, String>,
    // Remember inspected scopes even when no file existed. A later command
    // can create a new instruction file in one of these directories.
    directories: BTreeSet<String>,
    // Canonical roots of recursive inspections, re-scanned after effects so a
    // newly created descendant instruction file cannot remain invisible.
    recursive_scopes: BTreeSet<String>,
    dirty: bool,
}

struct InstructionAuthority {
    tool_plan: crate::CodingToolPlan,
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
        tools: &dyn CodingToolInvoker,
        sink: &dyn CodingEventSink,
        cancellation: CancellationToken,
    ) -> Result<DiscoveredScope, CodingEngineError> {
        let requested = if path.is_empty() { "." } else { path };
        let reader = InstructionReader {
            authority: &self.authority,
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
            return Err(CodingEngineError::Cancelled);
        }
        let scope: ScopeResponse = serde_json::from_value(value).map_err(|_| {
            CodingEngineError::WorkspaceContext("invalid instruction scope response".into())
        })?;
        if scope.path != requested
            || scope.recursive != recursive
            || !matches!(scope.kind.as_str(), "file" | "directory" | "missing")
            || crate::agents_md::normalize_workspace_directory(&scope.canonical_path)?
                != scope.canonical_path
        {
            return Err(CodingEngineError::WorkspaceContext(
                "instruction scope does not match the requested observation".into(),
            ));
        }
        if !scope.complete || !scope.incomplete_reasons.is_empty() {
            // Reasons originate at the owner, but are still data. Do not turn
            // arbitrary host response text into an instruction to the model.
            return Err(CodingEngineError::WorkspaceContext(format!(
                "instruction discovery is incomplete for {}; narrow the target or resolve unreadable/link/budget boundaries before retrying",
                serde_json::to_string(requested).unwrap_or_default(),
            )));
        }
        let base = if scope.kind == "directory" {
            scope.canonical_path.as_str()
        } else {
            scope
                .canonical_path
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("")
        };
        if scope.directories.is_empty() || scope.directories.len() > 64 {
            return Err(CodingEngineError::WorkspaceContext(
                "invalid instruction directory count".into(),
            ));
        }
        let mut directories = BTreeSet::new();
        for directory in scope.directories {
            if crate::agents_md::normalize_workspace_directory(&directory)? != directory
                || (directory != base
                    && !(recursive && scope.kind == "directory" && in_scope(&directory, base)))
                || !directories.insert(directory)
            {
                return Err(CodingEngineError::WorkspaceContext(
                    "inconsistent instruction directories".into(),
                ));
            }
        }
        if !directories.contains(base) {
            return Err(CodingEngineError::WorkspaceContext(
                "instruction scope omitted its base directory".into(),
            ));
        }
        Ok(DiscoveredScope {
            canonical_path: scope.canonical_path,
            kind: scope.kind,
            directories,
        })
    }

    pub(crate) fn new(request: &CodingTurnRequest) -> Self {
        Self {
            authority: InstructionAuthority {
                tool_plan: request.tool_plan.clone(),
                principal: request.principal.clone(),
                causality: request.model_request.causality.clone(),
                active_set_generation: request.active_set_generation,
            },
            sequence: AtomicU32::new(100),
            directories: BTreeSet::from([String::new()]),
            recursive_scopes: BTreeSet::new(),
            dirty: false,
            layers: BTreeMap::new(),
        }
    }

    pub(crate) fn has_context(&self) -> bool {
        !self.layers.is_empty()
    }

    pub(crate) fn context(&self) -> String {
        Self::render_layers(&self.layers)
    }

    fn render_layers(layers: &BTreeMap<String, String>) -> String {
        if layers.is_empty() {
            return "No repository instruction layers are currently available; no additional permissions are implied.".into();
        }
        let mut layers = layers.iter().collect::<Vec<_>>();
        layers.sort_by(|(left, _), (right, _)| {
            left.matches('/')
                .count()
                .cmp(&right.matches('/').count())
                .then_with(|| left.cmp(right))
        });
        layers.into_iter().map(|(path, content)| format!("Repository instructions scoped to the directory containing {path} and its descendants (deeper layers take precedence; no permission grant):\n{content}"))
            .collect::<Vec<_>>().join("\n\n")
    }

    pub(crate) fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Refresh before planning, compaction and completion, not just before the
    /// next filesystem call. Reads retain the current Kernel authority.
    pub(crate) async fn before_model(
        &mut self,
        tools: &dyn CodingToolInvoker,
        sink: &dyn CodingEventSink,
        cancellation: CancellationToken,
    ) -> Result<bool, CodingEngineError> {
        if !self.dirty || self.authority.tool_plan.binding("read_file").is_none() {
            return Ok(false);
        }
        let mut directories = self.directories.clone();
        for path in self.recursive_scopes.clone() {
            let scope = self
                .discover_scope(&path, true, tools, sink, cancellation.clone())
                .await?;
            if scope.canonical_path != path {
                return Err(CodingEngineError::WorkspaceContext(
                    "previously inspected recursive scope changed its canonical path".into(),
                ));
            }
            directories.extend(scope.directories);
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
        tools: &dyn CodingToolInvoker,
        sink: &dyn CodingEventSink,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, CodingEngineError> {
        let mut directories = BTreeSet::new();
        let mut scopes = BTreeMap::<String, bool>::new();
        for call in calls {
            let Some(binding) = self.authority.tool_plan.binding(&call.name) else {
                continue;
            };
            let value = &call.arguments.0;
            let mut paths = Vec::new();
            match binding.action_id.as_ref() {
                "workspace.files/read"
                | "workspace.files/write"
                | "workspace.files/delete"
                | "workspace.files/watch"
                | "workspace.artifacts/publish" => {
                    if let Some(path) = value.get("path").and_then(|v| v.as_str()) {
                        let recursive = binding.action_id.as_ref() == "workspace.files/delete"
                            || (value.get("format").and_then(|value| value.as_str())
                                == Some("instruction_scope")
                                && value.get("recursive").and_then(|value| value.as_bool())
                                    == Some(true));
                        scopes
                            .entry(path.to_owned())
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
                        .entry(path.to_owned())
                        .and_modify(|scan| *scan |= recursive)
                        .or_insert(recursive);
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
                            .entry(
                                value
                                    .get("cwd")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(".")
                                    .to_owned(),
                            )
                            .or_insert(false);
                    }
                }
                _ => {}
            }
            for path in paths {
                if let Err(error) = crate::agents_md::normalize_workspace_directory(path) {
                    return Ok(Some(format!("Operation not executed: {error}")));
                }
                scopes.entry(path.to_owned()).or_insert(false);
            }
        }
        if scopes.is_empty() {
            return Ok(None);
        }
        if self.authority.tool_plan.binding("read_file").is_none() {
            // No implicit fs.read grant. Process-only Agents may still execute
            // under their own instructions; explicitly disclose missing rules.
            return Ok(None);
        }
        if scopes.len() > 64 {
            return Ok(Some("Requested calls deferred: too many instruction targets in one batch; split or narrow the operation. Earlier calls may already have run.".into()));
        }
        let mut redirect = None;
        let mut recursive = self.recursive_scopes.clone();
        for (path, scan) in scopes {
            let normalized = match crate::agents_md::normalize_workspace_directory(&path) {
                Ok(path) => path,
                Err(error) => return Ok(Some(format!("Operations not executed: {error}"))),
            };
            let scope = match self
                .discover_scope(&normalized, scan, tools, sink, cancellation.clone())
                .await
            {
                Ok(scope) => scope,
                Err(CodingEngineError::WorkspaceContext(error)) => {
                    return Ok(Some(format!("Operations not executed: {error}")));
                }
                Err(error) => return Err(error),
            };
            if scope.canonical_path != normalized {
                redirect = Some(format!(
                    "Requested calls deferred; earlier calls may already have run. Path {} resolves to {}. Reconsider the operation using this canonical workspace path and its scoped instructions; arguments were not rewritten.",
                    serde_json::to_string(&normalized).unwrap_or_default(),
                    serde_json::to_string(&scope.canonical_path).unwrap_or_default()
                ));
            }
            if scan {
                recursive.insert(scope.canonical_path);
            }
            directories.extend(scope.directories);
        }
        if recursive.len() > 64 {
            return Ok(Some(
                "Requested calls deferred: recursive instruction scope budget exceeded; narrow the task. Earlier calls may already have run."
                    .into(),
            ));
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
            Err(CodingEngineError::WorkspaceContext(error)) => {
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
        result: crate::CodingToolResult,
        tools: &dyn CodingToolInvoker,
        sink: &dyn CodingEventSink,
        cancellation: CancellationToken,
    ) -> Result<(crate::CodingToolResult, bool), CodingEngineError> {
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
            return Err(CodingEngineError::Cancelled);
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
                return Err(CodingEngineError::WorkspaceContext(
                    "workspace.files/read is not active".into(),
                ));
            }
            let mut directories = BTreeSet::new();
            // Reject broad results before extra reads. A directory may contain
            // many hits, but each file still needs an exact canonical lookup.
            let parents = paths
                .iter()
                .map(|path| {
                    path.rsplit_once('/')
                        .map(|(parent, _)| parent)
                        .unwrap_or("")
                        .to_owned()
                })
                .collect::<BTreeSet<_>>();
            if self.directories.union(&parents).count() > 64 {
                return Err(CodingEngineError::WorkspaceContext(
                    "search instruction scope budget exceeded".into(),
                ));
            }
            for path in paths {
                let scope = self
                    .discover_scope(&path, false, tools, sink, cancellation.clone())
                    .await?;
                if scope.canonical_path != path || scope.kind != "file" {
                    return Err(CodingEngineError::WorkspaceContext(
                        "search hit path changed or is no longer a file".into(),
                    ));
                }
                directories.extend(scope.directories);
            }
            self.refresh(directories, tools, sink, cancellation).await
        }
        .await;
        match observed {
            Ok(changed) => Ok((result, changed)),
            Err(CodingEngineError::WorkspaceContext(_)) => Ok((crate::CodingToolResult::text(
                call.call_id.clone(),
                serde_json::json!({
                    "kind": "search_context_withheld",
                    "search_executed": true,
                    "snippets_withheld": true,
                    "notice": "The search returned, but its hit envelope or required repository instruction scopes could not be accepted completely. No snippets from this result are supplied to the model. Narrow path/limit and resolve unreadable, changed, aliased or oversized instruction scopes. Reading instructions requires fs.read in the frozen Agent selection; this turn cannot enable capabilities. Reconsider the search with a new call identity. This is not proof of absent matches or a rollback."
                }).to_string(), true), true)),
            Err(error) => Err(error),
        }
    }

    async fn refresh(
        &mut self,
        directories: BTreeSet<String>,
        tools: &dyn CodingToolInvoker,
        sink: &dyn CodingEventSink,
        cancellation: CancellationToken,
    ) -> Result<bool, CodingEngineError> {
        let directories = directories
            .into_iter()
            .map(|directory| crate::agents_md::normalize_workspace_directory(&directory))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let known = self
            .directories
            .union(&directories)
            .cloned()
            .collect::<BTreeSet<_>>();
        if known.len() > 64 || self.sequence.load(Ordering::Relaxed) >= 4096 {
            return Err(CodingEngineError::WorkspaceContext(
                "repository instruction discovery budget exhausted".into(),
            ));
        }
        let reader = RefreshReader {
            inner: InstructionReader {
                authority: &self.authority,
                tools,
                sink,
                sequence: &self.sequence,
            },
            cached: std::sync::Mutex::new(BTreeMap::new()),
        };
        let mut next = self.layers.clone();
        for directory in directories {
            let context = match crate::load_agents_md(
                &reader,
                &directory,
                AgentsMdPolicy {
                    max_file_bytes: 16 * 1024,
                    max_total_bytes: 24 * 1024,
                    ..Default::default()
                },
                cancellation.clone(),
            )
            .await
            {
                Ok(value) => value,
                Err(CodingEngineError::Cancelled) => return Err(CodingEngineError::Cancelled),
                Err(error) => return Err(error),
            };
            if !context.warnings.is_empty() {
                return Err(CodingEngineError::WorkspaceContext(format!(
                    "required repository instructions could not be read completely: {}",
                    context.warnings.join("; ")
                )));
            }
            // Remove stale/overridden layers only in the hierarchy just read.
            next.retain(|path, _| {
                let parent = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
                !(parent.is_empty()
                    || parent == directory
                    || directory.starts_with(&format!("{parent}/")))
            });
            for layer in context.layers {
                next.insert(layer.path, layer.content);
            }
        }
        if next
            .iter()
            .map(|(path, text)| path.len() + text.len() + 256)
            .sum::<usize>()
            > 48 * 1024
        {
            return Err(CodingEngineError::WorkspaceContext(
                "combined scoped instructions exceed the turn budget; narrow the task".into(),
            ));
        }
        if next != self.layers {
            sink.emit(CodingEngineEvent::InstructionsUpdated {
                context: Self::render_layers(&next),
            })
            .await?;
            self.layers = next;
            self.directories = known;
            return Ok(true);
        }
        self.directories = known;
        Ok(false)
    }
}

fn in_scope(directory: &str, root: &str) -> bool {
    root.is_empty()
        || directory == root
        || directory
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
