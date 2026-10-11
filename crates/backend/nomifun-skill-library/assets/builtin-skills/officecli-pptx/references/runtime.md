# Approved OfficeCLI execution

These skills require **iOfficeAI/OfficeCLI 1.0.156**, the component approved by
NomiFun's `crates/backend/nomifun-runtime/components.json`. Its official npm
identity is **`@officecli/officecli`**. Use the exact verified executable for all
examples, including help, create, render, edit, and close commands.

## Inside NomiFun

First read the **Host-owned Office execution component** context resource.
The compiled Skill body gives its exact `read_context_resource` ID. This is a
separate immutable runtime resource derived from
`nomifun_runtime::resolve_tool(nomifun_runtime::ToolId::OfficeCli)`, not a field
invented by the Skill or permission to access arbitrary files.

Require `component=officecli` and `status=ready`. Read its `executable_path`,
`version`, `digest`, `source=https://github.com/iOfficeAI/OfficeCLI`, and
`package=@officecli/officecli`. Bind every command to that exact verified
absolute executable. Require `execution_mode=one_shot` and apply the declared
`environment` (`OFFICECLI_NO_AUTO_RESIDENT=1`) to every child command. Inside
NomiFun do not start `open` / `watch` resident sessions; this execution contract
takes precedence over standalone resident recipes elsewhere in these skills.
Each supervised command must flush its edits before exiting. The Host admits the component even when the managed cache
has not been extracted yet. Do not guess cache paths or look up a same-named
PATH command. If status is `unavailable`, report the real dependency failure
and use application component diagnostics; do not install a replacement global
package. Existing process and file permissions still apply.

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

## Bind examples to that exact executable

Every bare `officecli` in these skills is shorthand for the verified path.
Before executing example commands, bind a task-local function, replacing the
placeholder with the actual ready Host resource:

```bash
OFFICECLI_EXE='/actual/verified/absolute/path/officecli'
officecli() { OFFICECLI_NO_AUTO_RESIDENT=1 "$OFFICECLI_EXE" "$@"; }
officecli --version
```

```powershell
$OfficeCliExe = 'C:\actual\verified\absolute\path\officecli.exe'
function officecli {
  $previous = $env:OFFICECLI_NO_AUTO_RESIDENT
  try {
    $env:OFFICECLI_NO_AUTO_RESIDENT = '1'
    & $OfficeCliExe @args
  } finally {
    if ($null -eq $previous) {
      Remove-Item Env:OFFICECLI_NO_AUTO_RESIDENT -ErrorAction SilentlyContinue
    } else {
      $env:OFFICECLI_NO_AUTO_RESIDENT = $previous
    }
  }
}
officecli --version
```

Preserve the same executable for the whole task. Do not update it midway.
Version output alone is not proof of capability; product admission also creates
a private DOCX and validates generated HTML. Run each editing operation and
check its exit status before proceeding. Preview failures must retain their
actual reason rather than becoming an empty result.

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
