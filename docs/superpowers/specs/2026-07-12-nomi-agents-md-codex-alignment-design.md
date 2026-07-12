# Nomi Agent AGENTS.md Codex Alignment Design

> Date: 2026-07-12
> Status: Approved for implementation

## Problem

Nomi Agent already has a partial `AGENTS.md` reader, but its behavior does not
match Codex and is therefore unreliable as a compatibility contract. It reads
plain `AGENTS.md` files while walking upward from the session working directory,
uses the user home directory as a fallback boundary, has no override or fallback
filename precedence, and applies no combined byte budget. It also implements a
Nomi-only `@include` expansion that Codex does not currently support.

The production bootstrap does call the partial reader while constructing the
system prompt. The missing work is therefore not another prompt append; it is a
single, testable instruction-resolution contract that the real session startup
path consumes.

## Goals

- Load project instructions once when a Nomi Agent session is built or resumed.
- Always include the project-root instruction file when it exists and the
  session working directory is inside that project.
- Match Codex discovery precedence from the project root through the session
  working directory.
- Support `AGENTS.override.md`, `AGENTS.md`, configurable fallback filenames,
  configurable project-root markers, and a configurable combined project-doc
  byte limit.
- Keep Nomi user-level instructions independent from Codex user state while
  giving them the same override precedence.
- Preserve source paths and resolution diagnostics for tests and startup logs.
- Keep the resulting instruction snapshot stable for the lifetime of the
  session, including resumed sessions.

## Non-goals

- Do not read `~/.codex/AGENTS.md`; Nomi uses its own platform configuration
  directory for user-level guidance.
- Do not hot-reload instructions during a running session.
- Do not add a settings UI or an instruction-status screen in this change.
- Do not make project `.nomi.toml` loading hierarchical; only the instruction
  file discovery contract is in scope.
- Do not support Nomi's existing `@include` extension. Codex does not currently
  implement this syntax, and expanding it would make byte-budget and source
  precedence behavior diverge.

## Compatibility Contract

### User-level discovery

Under `nomi_config::config::app_config_dir()`, inspect candidates in this order:

1. `AGENTS.override.md`
2. `AGENTS.md`

Use the first non-empty readable file only. The user-level file is independent
from the project byte budget and is placed before every project instruction.

### Project root

Starting from the session working directory, walk upward and select the nearest
directory containing any configured project-root marker. The default marker
list is `[".git"]`; marker entries are filesystem names and only need to exist.

An explicitly empty `project_root_markers` list disables parent traversal and
treats the working directory as the project root. If no configured marker is
found, also treat only the working directory as the project scope. This avoids
the partial implementation's incorrect walk to the user's home directory.

### Project instruction chain

Walk from the detected project root down to the session working directory,
including both endpoints. At each directory, inspect candidates in this order:

1. `AGENTS.override.md`
2. `AGENTS.md`
3. each configured `project_doc_fallback_filenames` entry in list order

Select at most one non-empty readable file per directory. An override in a
directory replaces the regular or fallback file in that same directory; it
does not erase already loaded parent guidance. Concatenate selected files from
root to leaf with blank lines so later, more local instructions have the highest
model-visible precedence.

The project root file is therefore a mandatory discovery point rather than an
optional Nomi-global fallback.

### Byte budget

`project_doc_max_bytes` defaults to `32768` and limits the combined project
instruction content. User-level instructions do not consume this budget.
Selected project files are read root-to-leaf until the budget is exhausted. If
the final selected file is larger than the remaining budget, include its UTF-8
safe prefix and mark the snapshot truncated. Do not read later project files
after the combined limit is reached. A configured limit of zero disables
project instruction content while leaving user-level instructions available.

## Configuration

Add these Codex-compatible top-level keys to Nomi's global `config.toml` and
workspace `.nomi.toml` parsing:

```toml
project_doc_fallback_filenames = []
project_doc_max_bytes = 32768
project_root_markers = [".git"]
```

The file-layer representation uses `Option` fields so a workspace can
deliberately override a global value with `[]` or `0`. The resolved runtime
configuration uses concrete values with the defaults above. Workspace values
override global values field by field.

## Architecture

### Configuration boundary

`nomi-config` owns parsing, merging, defaults, and the resolved
`ProjectInstructionsConfig`. It does not access instruction files.

### Resolver boundary

`nomi-agent::agents_md` owns a pure synchronous resolver:

```rust
pub fn resolve_agents_md(
    cwd: &Path,
    config: &ProjectInstructionsConfig,
) -> AgentsMdSnapshot;
```

`AgentsMdSnapshot` contains the detected project root, ordered loaded files,
formatted model-visible text, consumed project bytes, truncation state, and
non-fatal diagnostics. File metadata records its scope and original path.

The resolver separates candidate discovery from bounded content reading so the
same precedence contract can be tested without the system-prompt builder.

### Session startup boundary

`AgentBootstrap::build` resolves the snapshot once after the final workspace and
configuration are known, logs the ordered sources and diagnostics, and passes
the formatted content into `build_system_prompt`. The prompt builder no longer
reads the filesystem. `AgentEngine` receives the already assembled immutable
system prompt, so every provider turn and a resumed session use the same startup
snapshot.

This keeps filesystem discovery out of per-turn execution and preserves prompt
prefix stability.

## Prompt Composition

The stable system prompt order remains:

1. Nomi base identity and tool guidance
2. caller-provided custom system prompt
3. resolved user and project `AGENTS.md` instructions
4. memory and skills guidance
5. volatile environment metadata

The instruction section retains explicit source headers so precedence can be
audited and the model can distinguish user-level, root-level, and nested rules.
The resolver output is already ordered from broadest to most specific.

## Error Handling and Diagnostics

- Missing candidate files are normal and produce no warning.
- Empty files are skipped and discovery continues to the next candidate in the
  same directory.
- A metadata or read failure records a diagnostic and continues to the next
  candidate. Startup remains available when an optional instruction file is
  unreadable.
- Invalid fallback names that are absolute, empty, or escape their directory
  with parent components are ignored with a diagnostic. This prevents a config
  entry from becoming an unrelated arbitrary-file loader.
- Reaching the byte limit records a truncation diagnostic containing the active
  limit and final source path.
- Bootstrap emits source ordering at debug level and non-empty diagnostics at
  warning level. No file contents are logged.

## Testing

### Configuration tests

- defaults resolve to no fallbacks, `32768` bytes, and `[".git"]` markers;
- global values reach the runtime config;
- workspace values override global values, including explicit empty lists and
  a zero byte limit;
- serialization and minimal-file parsing remain backward compatible.

### Resolver tests

- project-root `AGENTS.md` loads when the working directory is nested;
- root and nested files are ordered root-to-leaf;
- `AGENTS.override.md` wins over `AGENTS.md` in the same directory;
- fallback names are considered only after both standard names;
- empty candidates fall through to the next candidate;
- no root marker limits discovery to the working directory;
- custom and empty root marker lists behave like Codex;
- the combined project budget truncates on a UTF-8 boundary and prevents later
  files from loading;
- user-level override precedence is independent of the project budget;
- unsafe fallback names are rejected;
- Nomi-only `@include` text remains literal and is not expanded.

### Integration tests

- the production bootstrap/system-prompt path includes a project-root marker
  rule for both new and resumed sessions;
- changing an instruction file after bootstrap does not mutate the engine's
  effective system prompt;
- custom prompt, project instructions, memory, skills, and environment sections
  preserve their documented order.

## Acceptance Criteria

- Starting a Nomi Agent session inside a repository loads the repository-root
  `AGENTS.md` before the first model request.
- Starting in a nested directory also loads applicable nested instructions,
  ordered after the root file.
- Overrides, fallback filenames, root markers, empty files, and the 32 KiB
  combined project limit match the Codex contract.
- Instructions are resolved once per session build/resume and remain stable for
  the session lifetime.
- The system prompt receives rules through the real `AgentBootstrap` production
  path rather than a test-only helper.
- Targeted configuration, resolver, prompt, and bootstrap tests pass; the
  affected crates pass formatting, lint-level compilation, and diff checks.
