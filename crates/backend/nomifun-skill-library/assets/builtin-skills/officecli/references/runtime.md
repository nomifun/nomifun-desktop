# Approved OfficeCLI execution

These skills require **iOfficeAI/OfficeCLI 1.0.156**, the component approved by
NomiFun's `crates/backend/nomifun-runtime/components.json`. Its official npm
identity is **`@officecli/officecli`**. Use the exact verified executable for all
examples, including help, create, render, edit, and close commands.

## Inside NomiFun

Use an already-authorized `workspace.process/exec` or `workspace.process/start`
tool with `component=officecli` and literal `args`. The current tool catalogue
provides its exact callable name. Choose exactly one of `component`, `command`
or `cmd`; an Office component call uses neither `command` nor `cmd`.

The Host resolves and verifies the approved executable at launch and sets the
fixed environment `OFFICECLI_NO_AUTO_RESIDENT=1` for every component command.
Do not request or copy physical executable paths or environment values from
Skill context resources, bind a shell function, or invoke an ambient PATH tool.
Component invocation does not require reading every Skill page first.

Office component commands are one-shot file operations: do not start `open` /
`watch` resident sessions. This Host execution contract takes precedence over
standalone shell and resident recipes elsewhere in these skills. Each command
must flush its edits before exiting. If the component is unavailable, report
the actual dependency failure and use application component diagnostics; do not
install a replacement package or substitute a same-named ambient program.
Existing process and file permissions still apply; selecting this Skill grants
no tools or permissions.

For operator/developer diagnostics where the **nomicore** binary is available,
run `nomicore --data-dir <actual-absolute-data-dir> tool-doctor --verify` and
inspect its `officecli` observation. `nomicore tool-doctor` alone is a read-only
inventory. `--verify` may extract approved tools and run private temporary
probes; neither mode opens the database. Source developers may run
`cargo run -p nomifun-app --bin nomicore -- tool-doctor --verify`.
The separate `nomifun-web` host CLI does not accept these subcommands.

Development-only `NOMIFUN_OFFICECLI_PATH` is an explicit absolute path for debug
builds. It still passes identity, version-range, and DOCX-to-HTML capability
checks. Release builds ignore this override.

## Run examples through the component target

Inside NomiFun, every bare `officecli` example represents an argument list for
`component=officecli`. Pass the tokens after `officecli` as separate literal
`args` to the authorized `workspace.process/exec` tool:

```json
{"component":"officecli","args":["help","docx","paragraph"]}
```

For a supervised command, use the authorized `workspace.process/start` tool:

```json
{"component":"officecli","args":["view","report.docx","html"],"wait_ms":1000,"timeout_ms":60000}
```

Use the actual task filename in place of `report.docx` or shell variables such
as `$FILE`. Do not include shell quoting characters around argument values:
`/body/p[1]`, `text=$50M` and spaces are already literal JSON strings. For batch
input, send the JSON text through the existing authorized process input and
close-stdin tools rather than turning a heredoc into a shell command. Poll the
returned process ID when needed and check the terminal exit status before
proceeding; an initial running observation is not completion. Preview failures
must retain their actual reason rather than becoming an empty result.

## Standalone use outside NomiFun

If no product-managed installation is in use, install the exact official scoped
package into a task-owned directory:

```bash
npm install --prefix /absolute/officecli-tools --no-audit --no-fund @officecli/officecli@1.0.156
```

Bind the resulting platform launcher by its resolved absolute path; verify
version `1.0.156`, then create a private temporary DOCX and generate its HTML
before using user documents. Native release artifacts are available at
<https://github.com/iOfficeAI/OfficeCLI/releases/tag/v1.0.156>; select the actual
platform/architecture and verify the SHA-256 recorded in the component manifest.
Do not run anonymous remote installer scripts, install the unscoped package,
or select a floating latest version.

NomiFun attachment previews use managed static HTML artifacts. `watch` and
interactive selection are separate, explicit standalone CLI workflows; they are
not required to deliver a read-only attachment preview.
