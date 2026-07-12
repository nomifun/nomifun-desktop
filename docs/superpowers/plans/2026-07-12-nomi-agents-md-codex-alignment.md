# Nomi Agent AGENTS.md Codex Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every new or resumed Nomi Agent session load user and project `AGENTS.md` guidance with Codex-compatible root-to-working-directory precedence before its first model request.

**Architecture:** `nomi-config` resolves the three Codex-compatible discovery settings, `nomi-agent::agents_md` turns a workspace and those settings into an immutable diagnostic snapshot, and `AgentBootstrap` installs that snapshot into the system-prompt cache exactly once. The prompt builder only composes pre-resolved instruction text and performs no filesystem discovery.

**Tech Stack:** Rust 2024 workspace, serde/TOML configuration, `tempfile` filesystem tests, Tokio integration tests, Cargo test/clippy/fmt.

## Global Constraints

- The project-root `AGENTS.md` must be loaded when the session working directory is inside that project.
- Discovery order is user scope, then project root through working directory; later files have higher prompt precedence.
- Per-directory candidates are `AGENTS.override.md`, `AGENTS.md`, then configured fallback names; at most one non-empty file is selected per directory.
- `project_doc_max_bytes` defaults to exactly `32768` and caps combined project content, excluding user-level instructions.
- `project_root_markers` defaults to exactly `[".git"]`; an explicit empty list makes the working directory the root.
- Instruction files are resolved once at session build/resume and do not hot-reload.
- Nomi user guidance remains under Nomi's platform config directory; do not read `~/.codex`.
- Remove Nomi-only `@include` expansion so instruction behavior matches current Codex.

---

### Task 1: Codex-Compatible Configuration Contract

**Files:**
- Modify: `crates/agent/nomi-config/src/config.rs`
- Test: `crates/agent/nomi-config/src/config.rs`

**Interfaces:**
- Produces: `ProjectInstructionsConfig { project_doc_fallback_filenames: Vec<String>, project_doc_max_bytes: usize, project_root_markers: Vec<String> }`
- Produces: `Config.project_instructions: ProjectInstructionsConfig`
- Consumes: top-level TOML keys `project_doc_fallback_filenames`, `project_doc_max_bytes`, and `project_root_markers`

- [ ] **Step 1: Write failing default and merge tests**

Add tests that parse omitted, global, and project values and assert exact resolved semantics:

```rust
#[test]
fn project_instruction_defaults_match_codex() {
    let resolved = ProjectInstructionsConfigFile::default().resolve();
    assert!(resolved.project_doc_fallback_filenames.is_empty());
    assert_eq!(resolved.project_doc_max_bytes, 32 * 1024);
    assert_eq!(resolved.project_root_markers, vec![".git"]);
}

#[test]
fn project_instruction_project_layer_can_clear_global_values() {
    let global = ProjectInstructionsConfigFile {
        project_doc_fallback_filenames: Some(vec!["TEAM.md".into()]),
        project_doc_max_bytes: Some(65_536),
        project_root_markers: Some(vec![".git".into(), ".hg".into()]),
    };
    let project = ProjectInstructionsConfigFile {
        project_doc_fallback_filenames: Some(Vec::new()),
        project_doc_max_bytes: Some(0),
        project_root_markers: Some(Vec::new()),
    };
    let resolved = ProjectInstructionsConfigFile::merge(global, project).resolve();
    assert!(resolved.project_doc_fallback_filenames.is_empty());
    assert_eq!(resolved.project_doc_max_bytes, 0);
    assert!(resolved.project_root_markers.is_empty());
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test -p nomi-config project_instruction --lib
```

Expected: compilation fails because `ProjectInstructionsConfigFile` and `Config.project_instructions` do not exist.

- [ ] **Step 3: Implement file-layer and resolved configuration types**

Add the following types near the other top-level configuration structures:

```rust
const DEFAULT_PROJECT_DOC_MAX_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ProjectInstructionsConfigFile {
    pub project_doc_fallback_filenames: Option<Vec<String>>,
    pub project_doc_max_bytes: Option<usize>,
    pub project_root_markers: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInstructionsConfig {
    pub project_doc_fallback_filenames: Vec<String>,
    pub project_doc_max_bytes: usize,
    pub project_root_markers: Vec<String>,
}

impl Default for ProjectInstructionsConfig {
    fn default() -> Self {
        Self {
            project_doc_fallback_filenames: Vec::new(),
            project_doc_max_bytes: DEFAULT_PROJECT_DOC_MAX_BYTES,
            project_root_markers: vec![".git".to_owned()],
        }
    }
}
```

Flatten `ProjectInstructionsConfigFile` into `ConfigFile` so keys remain top-level. Merge every optional field with `project.or(global)`, resolve concrete defaults in `Config::resolve`, and add the resolved value to `Config`. Update direct `Config` literals with `project_instructions: Default::default()`.

- [ ] **Step 4: Run configuration tests and verify GREEN**

Run:

```bash
cargo test -p nomi-config project_instruction --lib
cargo test -p nomi-config --lib
```

Expected: all `nomi-config` tests pass.

- [ ] **Step 5: Commit the configuration contract**

```bash
git add crates/agent/nomi-config/src/config.rs crates/agent/nomi-agent/tests/bootstrap_test.rs crates/agent/nomi-agent/src/spawner.rs
git commit -m "feat(agent): configure Codex-compatible project instructions"
```

### Task 2: Deterministic Instruction Resolver

**Files:**
- Modify: `crates/agent/nomi-agent/src/agents_md.rs`
- Test: `crates/agent/nomi-agent/src/agents_md.rs`

**Interfaces:**
- Consumes: `&Path` session working directory and `&ProjectInstructionsConfig`
- Produces: `resolve_agents_md(cwd, config) -> AgentsMdSnapshot`
- Produces: `AgentsMdSnapshot { project_root, files, formatted, project_bytes, truncated, diagnostics }`

- [ ] **Step 1: Replace legacy tests with failing Codex-contract tests**

Cover root inclusion, nested ordering, override selection, fallback selection, empty-file fallback, marker behavior, combined budget, UTF-8 truncation, unsafe fallback rejection, and literal `@include` text. The root regression must use a nested workspace:

```rust
#[test]
fn nested_workspace_loads_project_root_before_leaf() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join("AGENTS.md"), "ROOT_RULE").unwrap();
    let leaf = root.join("crates/agent");
    fs::create_dir_all(&leaf).unwrap();
    fs::write(leaf.join("AGENTS.md"), "LEAF_RULE").unwrap();

    let snapshot = resolve_agents_md_from(
        &leaf,
        &ProjectInstructionsConfig::default(),
        None,
    );

    assert_eq!(snapshot.project_root.as_deref(), Some(root));
    assert_eq!(snapshot.files.len(), 2);
    assert!(snapshot.formatted.find("ROOT_RULE").unwrap()
        < snapshot.formatted.find("LEAF_RULE").unwrap());
}
```

- [ ] **Step 2: Run resolver tests and verify RED**

Run:

```bash
cargo test -p nomi-agent agents_md::tests --lib
```

Expected: compilation fails because `AgentsMdSnapshot` and the new resolver do not exist.

- [ ] **Step 3: Implement candidate and root discovery**

Create focused helpers with these exact responsibilities:

```rust
fn detect_project_root(cwd: &Path, markers: &[String]) -> PathBuf;
fn directory_chain(root: &Path, cwd: &Path) -> Vec<PathBuf>;
fn candidate_names(config: &ProjectInstructionsConfig) -> Vec<&str>;
fn safe_fallback_name(name: &str) -> bool;
fn select_instruction_file(dir: &Path, candidates: &[&str])
    -> (Option<PathBuf>, Vec<AgentsMdDiagnostic>);
```

`detect_project_root` returns `cwd` when markers are empty or no marker exists. `directory_chain` includes root and cwd. Candidate selection skips missing, empty, and unreadable files, continuing within the same directory.

- [ ] **Step 4: Implement bounded root-to-leaf reading and formatting**

Read selected project files in order with one shared `remaining` byte budget. Snap a truncated prefix backward to a UTF-8 character boundary:

```rust
fn utf8_prefix(content: &str, max_bytes: usize) -> &str {
    let mut end = content.len().min(max_bytes);
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    &content[..end]
}
```

Resolve the user candidate separately using `AGENTS.override.md` then `AGENTS.md`, without consuming the project budget. Preserve source headers in `formatted`, and retain file metadata in broad-to-specific order.

- [ ] **Step 5: Run resolver tests and verify GREEN**

Run:

```bash
cargo test -p nomi-agent agents_md::tests --lib
```

Expected: all resolver tests pass, including the nested project-root regression.

- [ ] **Step 6: Commit the resolver**

```bash
git add crates/agent/nomi-agent/src/agents_md.rs
git commit -m "feat(agent): resolve hierarchical AGENTS instructions"
```

### Task 3: One-Time Bootstrap Injection

**Files:**
- Modify: `crates/agent/nomi-agent/src/context.rs`
- Modify: `crates/agent/nomi-agent/src/bootstrap.rs`
- Modify: `crates/agent/nomi-agent/tests/bootstrap_test.rs`
- Test: `crates/agent/nomi-agent/src/context.rs`
- Test: `crates/agent/nomi-agent/tests/bootstrap_test.rs`

**Interfaces:**
- Consumes: `AgentsMdSnapshot.formatted`
- Produces: `SystemPromptCache::set_agents_md(String)`
- Produces: a bootstrap-built `AgentEngine` whose first `LlmRequest.system` contains the startup snapshot

- [ ] **Step 1: Write failing prompt-cache and bootstrap integration tests**

Add a prompt-cache test proving pre-resolved instructions appear between custom prompt and memory/skills. Replace the existing bootstrap smoke test with a recording provider that captures `LlmRequest.system`, build from a nested workspace with root and leaf rules, run one turn, and assert both rules are present root-first.

Add a stability assertion: after bootstrap but before the model turn, rewrite both files; the captured request must still contain the original startup values and exclude the rewritten values.

- [ ] **Step 2: Run integration tests and verify RED**

Run:

```bash
cargo test -p nomi-agent bootstrap_with_agents_md --test bootstrap_test
cargo test -p nomi-agent pre_resolved_agents --lib
```

Expected: tests fail because bootstrap does not expose a pre-resolved cache input and the legacy prompt builder still reads files itself.

- [ ] **Step 3: Make the prompt builder composition-only**

Add this cache method:

```rust
pub fn set_agents_md(&mut self, instructions: String) {
    self.sections.insert("agents_md", instructions);
    self.joined = None;
}
```

Change the AGENTS section of `build_system_prompt` to read only the cached
`agents_md` section. Remove its `crate::agents_md` import and all direct
filesystem calls. Update existing context tests to call `set_agents_md` when
they need project rules.

- [ ] **Step 4: Resolve and install the snapshot once in bootstrap**

Before `build_system_prompt`, execute:

```rust
let agents_snapshot = crate::agents_md::resolve_agents_md(
    cwd_path,
    &self.config.project_instructions,
);
for file in &agents_snapshot.files {
    tracing::debug!(target: "nomi_agent", path = %file.path.display(), "loaded agent instructions");
}
for diagnostic in &agents_snapshot.diagnostics {
    tracing::warn!(target: "nomi_agent", message = %diagnostic.message(), "agent instruction diagnostic");
}
let mut prompt_cache = crate::context::SystemPromptCache::new();
prompt_cache.set_agents_md(agents_snapshot.formatted);
```

Do this for both new and resumed sessions through the common `build` path. Do
not re-resolve in `AgentEngine::run`.

- [ ] **Step 5: Run prompt and bootstrap tests and verify GREEN**

Run:

```bash
cargo test -p nomi-agent pre_resolved_agents --lib
cargo test -p nomi-agent bootstrap_with_agents_md --test bootstrap_test
cargo test -p nomi-agent --test bootstrap_test
```

Expected: the root and leaf markers appear in the first request in order, and
post-bootstrap file edits do not alter the captured system prompt.

- [ ] **Step 6: Commit bootstrap integration**

```bash
git add crates/agent/nomi-agent/src/context.rs crates/agent/nomi-agent/src/bootstrap.rs crates/agent/nomi-agent/tests/bootstrap_test.rs
git commit -m "feat(agent): load AGENTS rules at session startup"
```

### Task 4: Regression and Delivery Verification

**Files:**
- Modify if required by compiler: direct `Config` literals under `crates/agent/nomi-agent/` and `crates/backend/nomifun-ai-agent/`
- Verify: all files changed by Tasks 1-3

**Interfaces:**
- Consumes: completed configuration, resolver, prompt, and bootstrap contracts
- Produces: passing affected-crate verification with no formatting or diff errors

- [ ] **Step 1: Run formatting and compile affected packages**

```bash
cargo fmt --all -- --check
cargo check -p nomi-config -p nomi-agent -p nomifun-ai-agent
```

Expected: all packages compile. If direct `Config` literals are reported missing
the new field, add `project_instructions: Default::default()` and rerun.

- [ ] **Step 2: Run complete affected test suites**

```bash
cargo test -p nomi-config --lib
cargo test -p nomi-agent --lib
cargo test -p nomi-agent --test bootstrap_test
```

Expected: all tests pass.

- [ ] **Step 3: Run lint and repository integrity checks**

```bash
cargo clippy -p nomi-config -p nomi-agent -p nomifun-ai-agent --all-targets -- -D warnings
git diff --check
git status --short
```

Expected: clippy and diff checks pass; status contains only the intended plan or
implementation changes.

- [ ] **Step 4: Commit any final compatibility fixes**

If Task 4 required code edits, stage only those exact files and commit:

```bash
git add crates/agent/nomi-config/src/config.rs crates/agent/nomi-agent crates/backend/nomifun-ai-agent
git commit -m "test(agent): verify AGENTS startup compatibility"
```

If no code edits were required, do not create an empty commit.

- [ ] **Step 5: Record final evidence**

```bash
git log -5 --oneline
git status --short --branch
```

Expected: the design, configuration, resolver, and bootstrap commits are present
and the working tree is clean.
