# crates/shared

Cross-layer Rust crates used by both the `agent` and `backend` groups.

Current crates:

| Crate | Role |
| --- | --- |
| `nomifun-net` | Shared outbound HTTP client/proxy behavior. |
| `nomi-redact` | Shared redaction helpers for sensitive text. |
| `nomi-process-runtime` | Backend-neutral child-process contracts and supervision. |

`crates/shared/*` is part of the workspace membership in the root
`Cargo.toml`. Add a crate here only when it genuinely belongs on both sides of
the backend/agent boundary; otherwise keep it in the owning group.

## Process runtime boundary

`nomi-process-runtime` is the single supervised runtime for command processes.
`Bash`, `exec_command`, and `write_stdin` are schema adapters over one shared
`ProcessSupervisor`. Lower-level backend adapters use `ChildProcessBuilder`
directly; `nomifun-runtime` only owns the bundled Bun toolchain and PATH setup.

OS ownership belongs only in `nomi-process-runtime/src/platform`: Windows Jobs and
ConPTY, Unix process groups and watchdogs, and Unix PTY descriptors must not be
reimplemented in command adapters. Explicit user hand-off launch code is
limited to exact reviewed call sites rather than directory-wide exemptions.

Run `bun run check:process-runtime-boundary` locally. The
`command-reliability.yml` workflow runs this gate and the process/tool
contract suites on Windows, macOS, and Linux.
