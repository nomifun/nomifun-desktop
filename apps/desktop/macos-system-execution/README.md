# macOS native administrator execution

The signed desktop application uses `SMAppService.daemon` (minimum macOS 14)
and an authenticated `NSXPCConnection` to run system operations. The ordinary
desktop, renderer, Computer and media callers keep their logged-in user and
desktop session. The daemon is a system execution transport, not an Agent
backend or a second authorization store.

`nomi-system-execution` compiles the Swift client bridge during Cargo builds.
The daemon links the same crate's Rust static library and creates one
`nomi-process-runtime::ProcessSupervisor` per authenticated XPC connection.
Program/argv, explicit shell, Pipe/PTY, stdin, resize, polling, deadlines,
cancellation and cleanup receipts retain that runtime's behavior. A client
transport start fence prevents a cancelled pending RPC from starting after a
successful quiesce. An invalidated client connection shuts down its supervisor;
the macOS runtime watchdog retains process cleanup authority if the daemon is
killed. A transport failure retains an unknown result, never fabricated exit
or cleanup proof.

The canonical `read_file` action also selects this owner for administrator
reads, including repository instruction discovery before an administrator
process uses a protected cwd. It sends a typed `acquire_read` request; the
same signed helper runs its internal `--readonly-acquire` child mode under the
existing process supervisor. No shell, interpreter, temporary script or second
Agent is involved. The worker has a 10-second deadline and 16 MiB output
retention, sufficient for an 8 MiB source encoded as base64 plus bounded JSON.
Timeout, cancellation, disconnection and unknown cleanup use the same process
outcomes. It never reports a complete acquisition before the worker is reaped.

The small `nomi-native-files` crate owns the shared native source acquisition
and instruction-scope traversal. Ordinary FileService reads and the privileged
worker reuse its fd identity, metadata, version and bounds checks. The App
uses the same existing text paging/SHA pin/UTF-8 and image decoder projections.
Only a proven missing path becomes an absent result; access refusal, dangling
links, changed content or incomplete discovery remain real errors. Scope and
instruction page reads retain `workspace.files/read` canonical events and the
chosen operation identity, so preflight does not silently discard unreadable
instructions or grant another capability.

Both XPC peers require Apple-issued code signing with the same Team ID. The
desktop permits only the `com.nomifun.desktop.system-execution` helper; the
helper permits its corresponding signed `com.nomifun.desktop` or
`com.nomifun.desktop.dev` application identity. The two App identities own
separate launchd labels and Mach services, so development upgrades do not stop
production tasks. Ad-hoc builds cannot claim
administrator readiness. The authenticated XPC peer's account supplies HOME,
USER and LOGNAME defaults. Explicit task environment is preserved, and an
administrator process retains its actual root execution identity. This service
does not acquire TCC grants or operate the interactive desktop.

The complete App contains:

- `Contents/Library/HelperTools/nomifun-system-execution`
- `Contents/Library/LaunchDaemons/com.nomifun.desktop.system-execution.plist`

Development Apps use `com.nomifun.desktop.dev.system-execution.plist` and the
matching Mach service; the signed executable implementation is shared.

`scripts/lib/macos-system-execution.mjs` builds the exact target/profile Rust
static library, links the native helper and copies both files. Development,
fast App and release builds invoke it after their host Cargo build returns.
`scripts/lib/macos-app-bundle.mjs` signs the nested helper with its fixed
identifier before sealing the App. The same final signed App is used for DMG
and updater archives. No helper is downloaded at runtime and no temporary
administrator shell is used.

On first administrator use the application registers its bundled daemon.
macOS may require approval in Login Items & Extensions; NomiFun opens that
native pane once per process and returns
`SYSTEM_EXECUTION_APPROVAL_REQUIRED` so the current task can recheck and
continue after approval. Registration, denial, unavailable signing or missing
bundle components return their real native error; repeated commands do not
create a NomiFun approval ledger. Ordinary operations never register this
service.

Updates replace the entire signed application bundle, including the daemon.
The client and helper compare a build identity derived from the shared runtime,
native reader, transport and Swift source, workspace lock/manifest and relevant
crate manifests, framed by repository-relative paths and byte lengths. The
packaging cache checks the same inputs plus the exact static library. When the
current registered daemon differs, the
client requests a service-wide temporary-task cleanup fence, requires exact
receipts, then unregisters and waits for the native completion callback before
re-registering the current bundled component. Unresolved cleanup remains
unknown and prevents an automatic upgrade from claiming success. Existing
work is owned by its existing XPC connection and must be quiesced before an
application upgrade exits. An uninstaller calls
`SystemExecutionClient::unregister()` while the signed App still exists and
waits for current process cleanup before removing the App. The actual signed
host provides `--unregister-system-execution` for this operation; it waits for
the system unregister completion instead of sleeping or treating an API call
as proof the old service exited. Removing the App
with Finder follows macOS background-item lifecycle; it is not represented as
a verified NomiFun uninstaller flow. The daemon is on demand (`RunAtLoad` is
false); explicit user-installed launchd services remain separate OS artifacts.

Useful checks:

```sh
cargo test -p nomi-system-execution --lib
node scripts/lib/macos-system-execution.mjs --target aarch64-apple-darwin --profile debug --app /absolute/path/NomiFun.app
node scripts/lib/macos-app-bundle.mjs sign --app /absolute/path/NomiFun.app --identity SIGNING_IDENTITY --target aarch64-apple-darwin
```

Deterministic Rust tests exercise the wire adapter and shared supervisor. A
signed App administrator acceptance must additionally verify system approval,
actual effective uid, an ordinary-user-unwritable isolated fixture, marker
content/metadata and cleanup through the default Agent product entry. It must
separately test denial, approval changes, cancellation and host disconnect;
building or authenticating the daemon alone does not establish those results.

Apple's [SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice)
and SDK contract require correct registration, user approval and re-registration
after changing daemon code or its plist. Distributed LaunchDaemon Apps also
need notarization; the release packaging submits the final signed App with the
nested helper. These are separate checks from the native operation itself.

Windows and Linux currently return `SYSTEM_EXECUTION_NOT_AVAILABLE`; their
native UAC/system authentication and session bridges require platform design,
implementation, packaging and native acceptance. The shared execution identity
does not imply that those platform adapters already exist.
