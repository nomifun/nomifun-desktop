# Browser Workspace

This document, the [Agent Session architecture](agent-session.md), and current source define the development contract.
The built-in side browser is a user work surface. Agent Browser Use is a separately authorized tool consumer.

## Product boundary

The Browser domain owns one managed browser per authenticated owner and canonical AgentSession. The side-rail Browser
button permits direct user browsing without a Browser Module grant, Provider selection, or Agent resource binding.
Opening, retrying and user commands never create Agent authority or a second Session. Retired global management and
browser Settings surfaces remain removed.

The page is a native embedded WebView, not an iframe, screenshot stream, canvas viewer, or JPEG transport. The user and Agent operate the same page. While the Agent runs, native user input and page-changing chrome controls are locked. A run must stop and its pending operations must settle before user input is restored. There is no takeover or hand-back state.

Hiding the surface preserves its tabs. The Session has one managed runtime, profile and input coordinator.
`BrowserResource` wraps exact frozen Agent authority over the same `BrowserWorkspace`; a different authorization
definition never selects another page or profile. Agent operations still require the frozen action allowlist,
exact Provider, typed resource binding and current BrowserRunGuard. A user opening a page never grants Agent tools.
An Agent using attached Chrome leaves the user's managed browser independent. A different exact managed Provider
cannot silently reuse the existing entity.

Every Agent Turn, including ordinary chat without Browser tools and attached Chrome Agents, acquires the managed
browser input gate during Runtime preparation. Creating the entity does not create a native page. A native child first
created during a run starts with user input disabled. When the canonical Turn is already running but its native
gate is not proven yet, user commands and first opening fail closed. Retained preparation owners drain cancellation
and failure while keeping hardware input locked. Only the exact durable Turn terminal permits final release; downstream
cleanup and terminal write failures cannot unlock input. A start that issued no guard uses the same drain/terminal/recovery boundary.

Terminal publication and native final release are separate receipts. If release fails after publication, an SDK retry
must match the same root's canonical receipt, original Runtime terminal, delivery, Snapshot and route before retrying
finish. It does not append another terminal or change failure into cancellation. Cancellation before execution keeps
its separate exact witness for generation zero. After complete Runtime teardown, the Kernel resource context retires
from its Weak cache by exact Arc identity. Old handles may remain alive, but replacements cannot reuse a closed
context and late old cleanup cannot evict the successor.

Production profiles belong to the authenticated owner and Session. Windows retains namespace
`nomifun.browser.session-managed-profile.v2` and directory `browser-v4/agent-sessions/<identity-hash>/`.
macOS 14+ uses a separate persistent `WKWebsiteDataStore.dataStoreForIdentifier`: a length-prefixed SHA-256
derivation combines `nomifun.browser.webkit.session-store.v1`, compiled channel, canonical data root, existing
trusted `storage-generation`, owner and canonical AgentSession into a stable UUID. Agent resource definition IDs,
project paths and Agent persistence parameters do not select profiles. Restart retains sign-in; channels, test roots
and distinct data roots do not share it. Moving/copying the data root selects a fresh sign-in namespace. WK data
is not part of portable backups. Restore creates a new destination generation and leaves the source installation's
system stores unchanged. Reset cleanup derives retired stores from the original root, archived trusted generation
and canonical Session owners, then awaits native deletion before the existing dataset coordinator finalizes reset.
There is no second Session/profile ledger. Missing/invalid archived identity evidence fails closed; without the native
cleanup port (including CLI/no-browser builds), reset stays pending and must finish in NomiFun Desktop. Session
deletion likewise requires confirmed native cleanup. Site clearing awaits public WK removal callbacks.
WK does not inherit Safari or CEF cookies; signing in once after switching is expected. Old CEF files are neither
read, scanned, migrated nor automatically deleted; removing them requires explicit user choice. Switching does not
clear chats, model configuration or credentials. Standalone native conformance fixtures may still use ephemeral data.
Old profile presence and contents do not affect startup, identity derivation, recovery or cleanup decisions. No old-format
reader, dual read, fallback or migration workflow is maintained. Leaving user files on disk creates no runtime compatibility
contract; the new Mac implementation owns only current WK stores. If the reset archive contains the canonical table,
its required current identity fields must be present; missing fields cannot be treated as an empty session set.
Canonical Session deletion closes the physical runtime and safely deletes its exact user profile even without an
Agent Browser binding, including after restart. No second profile or Agent authorization ledger is introduced;
historical Agent resource definitions remain available for settled effect receipts.

The embedded browser does not expose F12, Inspect, or a user-visible DevTools window. Ordinary pages, popups, and host maintenance views explicitly disable that entry point, and the boundary scanner prevents it from returning. Low-level WebView2 protocol calls still implement Agent observation, native input, frames, and lifecycle; they are host-internal transport, not a DevTools product feature.

The conversation browser uses WebView2 on Windows and system WKWebView on macOS 14+, with the operating-system network stack. It does not install an application proxy, IP/port allowlist, or network settings UI, preserving system proxies and certificates plus localhost, LAN, WebSocket, and HMR workflows. The minimal boundary accepts only credential-free HTTP(S) top-level navigation and gives browser children no Tauri capability, local trust, backend credential, or arbitrary filesystem authority. Background `web.research` and rendering are separate consumers and retain strict public DNS/IP pinning.

Before compiling Agent configuration, a native host derives the Browser Role v2 default binding from its materialized bundled Provider. This is exact boot composition, not a user-facing Role setting or a migration of old installation bindings.

The browser menu offers an explicitly confirmed Reopen browser action. It closes page tabs and loses unsaved page content, but does not clear conversation messages or project files. The backend holds the same Session operation fence and validates the generation at a canonical idle boundary.
Without a native guard it uses ordinary idle close. If an issued guard's final unlock failed, closing requires that
exact Operation's canonical terminal, proven native settlement and matching old Workspace instance/generation.
Canonical running and unproven settlement still reject reopening; this user operation never cancels an active Agent
or manufactures a terminal. `turn/paused` retains the same active Operation and native checkpoint resume authority,
so it is not a final terminal for this close path. Reopening remains rejected while paused; the user must first finish
the Operation through the formal Stop/cancel path and prove Runtime cleanup before reopening. The old Workspace's confirmed destruction lets the SDK acknowledge its existing terminal
and retry final release. The new user Workspace has another generation and cannot be released by the old owner;
the model is not replayed and the original outcome is not rewritten.

macOS embeds a visible WKWebView child using Tauri's existing AppKit main thread and event loop. WK/AppKit
objects are created, used and released on that thread. There is no second NSApplication, downloaded browser
runtime, bundled system WebKit.framework, CEF helper/guardian, framework preload, message pump, allocator
exception or fallback engine. System WebKit owns WebContent processes.

## Browser identity and navigation recovery

Before the first request, ordinary macOS tabs and popups share one native desktop Safari compatibility identity: `macos-desktop-webkit-safari17-v1`. Its `Version/17.0` token is an advertised compatibility baseline, not a measured Safari/WebKit runtime version. Actual OS information is reported separately and unknown framework metadata remains unknown. The policy neither changes Session/Profile identity nor clears cookies, bundles Safari or another engine, patches JavaScript getters, or sets individual request headers. Windows retains its native identity.

One Page reducer owns request/start/redirect/commit/finish/cancellation/failure/process-termination state. The `load` summary reports an attempt sequence, phase, displayable content, rounded progress, safe problems, and attempted/content addresses. Internal bootstrap has explicit source identity and is not user content; genuine about:blank popups can hold documents. Cancelled attempts map to `Stopped`, never successful navigation. Cancellation classification checks domain and code. Retained native navigation objects and dispatch receipts reject late callbacks, stale Stop commands, and old layout updates.

Panel/layout admission, native child displayability, and input ownership are independent. Empty failed/stopped/crashed pages hide the child on the native main thread and show host recovery content; retained/partial documents remain visible with their attempted/content address distinction. Every native show reapplies the current Page mask. Authorized Agent recovery navigation waits for layout without requiring an executable old document. Empty reads return `BROWSER_PAGE_FAILED`, `BROWSER_PAGE_STOPPED`, or `BROWSER_PAGE_CRASHED`; valid retained/partial content can be observed and captured with a safe load summary. Existing execution-uncertainty errors remain intact: no automatic DOM, POST, or Agent action replay.

User downloads use panel presentation independently of the document mask. A first direct attachment can continue destination confirmation while an empty document is hidden; actual panel hiding, input-owner changes and process termination still revoke pending choices. DOM interaction, media permissions and upload selection continue to require actual document visibility.

The Page-owned main-document trace is ephemeral and bounded to 32 attempts, 128 events, and a 128 KiB serialized-event budget. Capacity loss increments dropped independently of routing-proof loss. Response callbacks have no WKNavigation identity; unproven response association remains a Page-scoped sample and does not update the current response summary. Matching URLs or the latest sequence are insufficient proof. This does not claim console, subresource, or Fetch/XHR coverage; existing unavailable semantics remain.

The user explicitly copies navigation diagnostics through an owner-checked read of the existing Workspace and exact target. It does not initialize a runtime, grant Agent authority, or upload data. Actual navigation/address bars retain original URLs; Agent/diagnostic metadata removes userinfo, query and fragment through the shared safe projection and enforces text limits. Cookies, credentials, bodies, form values and arbitrary NSError descriptions are excluded. Recovery remains explicit and tab-scoped; slow-load hints never trigger infinite reloads, bypass challenges, or clear the whole workspace.

### Navigation recovery validation 2026-10-10

A separate real Tauri window on macOS 26.3.0 passed ephemeral native checks for initial Stop without content, actual child masking, same-tab recovery, HTTP/JavaScript UA agreement across ordinary pages/iframes/fetch/images/popups, two-hop redirects within one attempt, page reload versus host-request separation, prompt empty-page errors, and retained HTTP 403 content after a subsequent failed navigation. A separate native Page without automatic download-directory configuration verified the real first-attachment destination picker, masking without picker cancellation, and cancellation through explicit cancel/panel hiding/input ownership change. No destination was selected or file written to user Downloads.

A 60-second live-site run in an ephemeral native Profile observed the Bilibili homepage without the browser-version advice page and readable Baidu search results; each retained one host navigation with no host reload loop. Login, actual video playback, minimum-macOS and Intel hardware runs remain uncovered, and site challenge avoidance is not promised. The identity uses existing dependencies and system frameworks with no new bundled engine resources; exact release-package byte deltas require an identical-parameter build comparison.

## Platform capabilities

| Capability | macOS WKWebView | Windows WebView2 |
| --- | --- | --- |
| Visible pages, tabs, navigation/history, reload/stop, ordinary user input | Native workspace retained | Existing implementation retained |
| Text/elements and visible screenshot | Main-document semantic observation; native snapshot | Existing protocol observation and screenshot |
| Ordinary click, text fill, option select, scroll, submit button | `semantic_dom`; single left click, replacement text | Existing `browser_input` |
| Drag, hover, key combinations, right/middle/double click, pointer capture, complex canvas/editors | Explicit `BROWSER_UNSUPPORTED_ACTION` | Existing support and frame restrictions retained |
| Iframes and closed shadow content | Uncovered content has no actionable refs; `unobserved_frames` is explicit | Existing frame routing |
| User file picker and download | Public WK/AppKit flow; no HTML `accept` prefilter | Existing native flow and filters |
| Agent upload/download, developer evaluate, full CDP diagnostics | Unsupported in this version | Existing actions and grants retained |
| Persistent sign-in, Session deletion/site-data clear | Separate macOS 14+ WKWebsiteDataStore | Directory Profile retained |

`semantic_dom` never fabricates `isTrusted` or user activation. Successful DOM calls or event dispatch do not
prove a site's business outcome; observe again. When a site requires manual interaction, use the existing
Stop, wait for cleanup, and unlock flow. Observation and user snapshots report `interaction_capabilities`;
the schema action union does not promise all actions on every runtime. Unsupported errors forbid repeating
the same action without making ordinary browsing unavailable. The browser menu explains common limits.

The public `WKOpenPanelParameters` API exposes multiple-selection and directory options, but no HTML `accept`
MIME/extension hints. The macOS chooser therefore does not prefilter file types; the user must choose a type
accepted by the website. This limitation does not grant extra file access or alter Windows picker filtering.

`browser/observe` returns semantic observation by default. Optional `screenshot:true` captures the same native
tab's viewport after exact model-route ImageInput admission. Pixels become a typed image part, never base64
model text. Screenshot and DOM observation must have the same target generation. A pending website dialog
returns `screenshot_status=awaiting_dialog`, not a fabricated image. This path needs no Computer Screen
Recording permission and does not use Headless rendering or supply the renderer's browser surface.

## Current implementation

### WK validation — 2026-10-09

The actual Tauri desktop App was exercised on Apple Silicon running macOS 26.6.2. Its canonical
AgentSession/Engine path used a local scripted model for 18 rounds and 220 events: visible navigation,
observation, form entry/selection/submission, scrolling and native snapshots reached typed model image parts.
Semantic actions reported `isTrusted=false`; a physical user action reported `true`. Stale references were
rejected, and unsupported drag was followed by successful ordinary interaction on the same page. This is
real App/tool integration with a deterministic model, not an external live-model acceptance result.

- Physical user input was blocked while the Agent ran and restored after GUI Stop. Pending-dialog Stop was
  observed in the GUI/DOM and the canonical cancelled terminal; its separate HTTP witness was absent, so
  that HTTP completion is not claimed.
- User interaction covered Chinese paste, confirm/prompt dialogs, popup, native file selection, and HTTP/blob
  downloads in the App. Back and Forward each restored the expected URL and native page content in the
  final App validation run. An in-flight page load stopped through the toolbar, with normal navigation still
  working afterward.
- Persistent profiles A/B remained isolated across restart. Clearing A remained effective after another
  restart without clearing B. Terminating the exact owned WebContent child and reloading preserved cookies.
- Session C displayed its distinct cookie and localStorage before deletion. Its first DELETE returned HTTP 200
  and canonical state `deleted`; production completion required absence from public store enumeration, and
  the exact system-store directory was gone. Six before/after SHA-256 comparisons for the A/configuration/chat/
  legacy-data controls remained identical. GUI inspection confirmed the removed native child and only A left
  in the session list. The earlier B DELETE HTTP 409 remains a separate failed attempt; the later App startup
  recovery is recorded separately and does not turn that first failure into a pass.
- An isolated temporary dataset accepted one request through the real factory-reset API. After normal exit
  and restart, the old WK store and canonical sessions were removed, a new generation and dataset receipt
  were established, and pending reset markers were consumed. The actual App displayed an empty session list
  and model selection. This was an explicitly exercised reset of disposable development data, not data clearing
  during the browser migration.
- Targeted checks passed: 105 backend tests, 7 native tests and 80 UI tests. The final distribution check found
  an arm64 App of 162,339,889 bytes across 6 regular files, with macOS 14 minimum, a valid ad-hoc signature,
  system WebKit linkage and no CEF, browser helpers or bundled WebKit. The ULMO DMG is 54,522,639 bytes;
  verification, read-only mounting and complete App parity passed. The gzip level 9 updater is 72,064,001 bytes;
  extraction parity and a disposable test-key signature passed, and the private test key was deleted. The dirty
  source tree correctly failed the release lock with exit 3. These are local test artifacts, not a published release.
- The Intel adapter cross-compilation check passed; this does not establish a complete Intel App or Intel
  hardware runtime. Those scopes, external live-model execution, a new Windows GUI regression, Developer ID
  signing and notarization remain unverified.

The historical CEF failures and Windows evidence below retain their original scope; they do not replace the
WK receipts above or expand their acceptance coverage.

The following is historical pre-migration CEF failure evidence; WK checks must not rewrite it as success. The bootstrap
document reached Ready in about 25 ms, but a newly ad-hoc-signed application's first persistent navigation still hit
the 30-second protocol deadline. Restarting the same artifact completed cold navigation in about 120 ms and passed
the native suite and exit. An independent ephemeral comparison navigated successfully, but a shutdown worker was
observed waiting in `SecItemCopyMatching` / Keychain decrypt for over 260 seconds. Forced runner cleanup was recorded
as failure, not successful native shutdown. Warm runs, same-artifact restarts and partial native tests do not prove
all first cold runs of freshly signed applications.

Those CEF/Keychain records no longer define implementation requirements and are not WK acceptance evidence.
WK receipts distinguish the actual App, direct Agent tool calls, deterministic model fixtures, external live models,
native fixtures, ARM/Intel compilation and target-machine execution. Unrun scopes remain unverified; warm runs,
mocks and independent native windows cannot substitute for the product journey.


The Windows conversation menu wires user-only site-data clearing with explicit confirmation. The runtime closes its pages, awaits native Profile clearing on an inert, hidden, same-profile controller, then destroys that controller. It is not a tab or an automation browser. Unknown work is not released by a timeout; recovery teardown still waits for native pending work. The two-persistent-profile regression, pre-dispatch cancellation, generation/Agent rejection and UI confirmation tests pass. Main-GUI clicking and the complete in-flight crash/shutdown matrix remain unfinished; the latest isolated preview includes the code but has not yet been launched manually.

The Windows main GUI now has live-model counter-repair evidence: capabilities selected in the workbench, a native click reproducing +2, Read/Edit changing app.js, a reload followed by trusted clicks yielding 1/2/3, and user continuation to 4 on the same page. An initial unadvertised Glob call was rejected; a corrective follow-up was needed. This is not an uninterrupted autonomous pass or full framework startup/build/GUI acceptance. See the implementation record for failures and receipts.

Application plugin scripts must not replace website APIs. The desktop uses adapters that keep explicit native dialog APIs without the upstream global `alert`/async `confirm` shim. The notification initializer runs only in first-party top-level app documents, where the official notification API needs it; browser pages keep native `Notification`. IPC ACLs remain unchanged. This boundary was added after real main-app inspection found `confirm()` returning a Promise despite the standalone native smoke passing; the smoke now includes these adapters.

- The desktop host creates native child views and owns their lifetime and input gate.
- BrowserWorkspace scopes user operations to the authenticated owner and canonical Session; Agent wrappers independently enforce exact grants.
- BrowserRunGuard is supplied by the authoritative Agent turn lifecycle, never by UI or model JSON.
- Element references include runtime, document, and observation generations. Native actions consume observations.
- The Nomi Browser tool is limited by the frozen capability set and exact provider.
- The renderer owns browser chrome and a native view slot; it does not render browser frames.

Windows WebView2 has real input and lifecycle smoke coverage. The main GUI has a live StepFun same-page repair loop: reproduce +2 by native click, Read/Edit the source, reload, verify 1/2/3, then continue as the user to 4. Full framework startup/build, macOS native execution, and the release packaging matrix remain unfinished.

Windows currently supports real HTML drag/drop within one native protocol session. WebView2/Chromium public virtual input cannot complete `dragend(move)` on the real source renderer across OOPIF sessions, so a cross-session drag fails explicitly before mouseDown. It does not synthesize DOM events, move the system-global pointer, or disable site isolation to imitate support.

Windows tabs now install native script-dialog handling before their first navigation, including before popup binding. A single owned-work registry retains input, navigation, observation and screenshot operations when a dialog interrupts them. Agent replies use the current run guard; idle-user replies use the user gate and tab-local website dialog. A paused observation has no actionable element references, and a paused screenshot returns dialog metadata rather than a fabricated image. Initial-document prompts, beforeunload accept/cancel, asynchronous dialogs and popup initial dialogs have production-host smoke evidence. Exact-tab close now destroys the native page and settles only its scoped work, without cancelling the run or another page's dialog; concurrent closes serialize per tab. Full frame/creation/cancellation races and main-app visual acceptance still need work.

Windows iframe input uses native `DOM.getBoxModel` content quads and projective mapping instead of reimplementing CSS transforms. Same-process parents are converted from the protocol-session root back to their own viewport before hit testing; OOPIF session boundaries stay intact. Quads and viewports are rechecked before input, with degenerate, non-finite or changed geometry rejected. Real tests cover static perspective, ancestor perspective, individual 3D properties, motion paths, nested same-process clicking/typing and parent overlays. Dynamic-transform and cross-process drag completion coverage is still incomplete.

Native WebView2 zoom at 80%, 125% and 150% has verified far-positioned root buttons, nested projected clicks and trusted text input, with ZoomFactor readback and matching CSS viewport sizes. A scrolled root viewport at 150% also preserves correct input placement. This is native page-zoom acceptance, not device/phone emulation or a new product mode.

`cargo run -p nomifun-desktop --example browser_workspace_smoke -- --agent-only` runs the real isolated DesktopServer, configuration APIs, capability compiler and Nomi turn against a local scripted model-protocol endpoint. It drives a visible native WebView through observation, input, clicking and diagnostics, then checks input locking, the ordinary conversation reply and teardown. It is not autonomous coding acceptance with a live model or a new testing product UI.

The same fixture covers cancellation of a waiting model, discarded late actions, fresh observation of the same document on the next turn, and an in-flight native navigation. Application shutdown permanently seals runtime admission and awaits admitted builds and exact runtime exits before closing Browser and database resources. Failed exits retain the original instances for retry; a kill request or hidden page is not exit proof.

Website permissions now have a conversation-local human prompt backed by native deferrals. Only the visible, active UserReady tab can receive a one-request decision; the Agent cannot grant permissions. Timeout and hide/show denial are verified. Automatically cancelled requests stay denied within the current document; reloading permits a new request without persisting the denial in the Profile. Other device permissions and full main-application visual/interaction acceptance remain unfinished.

## File upload boundary (in progress)

The optional `browser.upload` capability enables `upload` for observed visible standard file inputs, or visible buttons opening an HTML chooser in the current page, on Windows. Custom buttons receive native clicks; dynamic/hidden inputs must come from that native chooser event. Paths are workspace-relative; an authorized directory handle and no-follow opens reject traversal, links/junctions, directories and special paths. `browser.act` alone does not grant local file upload. The host prepares snapshots preserving names, bytes and modification times, limited to 16 files / 64 MiB per call and 128 files / 256 MiB retained per Runtime. Native close releases those snapshots; Tool completion or a page resetting its input does not invalidate retained File objects.

WebView2 file selection uses `DOM.setFileInputFiles` and reports `browser_protocol`; the driver does not synthesize input/change events. Empty lists are rejected because [Chromium's implementation](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/core/html/forms/file_input_type.cc) ignores them. The Agent can use the page's own reset/remove controls through native clicks. New HTML choosers are intercepted during Agent work, including configured iframe routes; the receiver carries no file authority, and Session/Frame/document/current-request checks precede file assignment. Real same-process/OOPIF standard and dynamic input cases, including parent-button delegation to an observed child, have file-data evidence; child navigation requires fresh observation. Directory uploads, user picker brokering (including dialogs already open when a run starts), File System Access APIs, remaining file cases and abnormal-exit snapshot reclamation are not yet delivered.

The Windows user file picker uses an isolated helper process. A private subcommand dispatches before application initialization and shows IFileOpenDialog; cancellation awaits its exact process-tree exit. Essential-only environment inheritance and bounded validated pipes remain in place. Helper-only native selection was verified in `--picker-selection-only`. UserReady HTML requests are now wired to this picker through native event targets and owned frame routes, with an isolated document object and a live UI-dispatch guard for file assignment. `--user-files-only` verifies native cancellation on hide, root/OOPIF navigation, close and Agent admission, while unrelated iframe navigation preserves selection. Temporary profile cleanup waits up to two seconds for Windows sharing violations after all controllers close, retaining ownership on failure. The wired successful delivery path still awaits user approval for the interactive `--user-file-selection-only` local temporary-file check; it is not claimed as verified. Directories, full cancel-event semantics and the remaining file matrix are unfinished. These fixtures add no product testing UI.

HTML `accept` now drives native Windows filtering using validated extensions and offline MIME mappings, including image/audio/video wildcards. Unknown or oversized hints remain unrestricted; raw webpage strings cannot inject Shell patterns. The native picker retains an all-files option because filtering is a hint, not file authority or content validation. Computer-use inspection of `--user-file-filter-only` verified TXT/CSV visible and PNG hidden by default, then PNG visible after switching to all files. The dialog was cancelled without selecting or delivering files.

New OOPIFs pause briefly at native attachment while the event worker recursively installs owned routing and file-chooser policy, then resume without waiting for an Agent observation. Failed policy does not resume an unprotected document and invalidates routing for native close/recovery. Run admission updates existing and future iframe policy, while idle users retain user policy. A real native-click fixture confirms interception in a newly created OOPIF before its first observation; unit tests cover configuration barriers and failure invalidation.

The isolated file picker distinguishes Open and Save modes with a v2 private protocol. UserReady downloads now use DownloadStarting deferrals and the same native WebView's download operation. Only visible, live, idle tabs can request a Save dialog; document/run authority is rechecked before native path submission. Each tab has at most four in-flight requests and one pending Save dialog. Hide, navigation and Agent entry cancel unconfirmed selection; authorized transfers can continue, while tab closure cancels and awaits native terminal state. Default download UI is suppressed so it cannot escape the input lock. Cleanup failures retain retry ownership. Real HTTP bytes, Unicode destinations, cancellation and an active transfer surviving Agent entry before closure have local evidence. Agent download/sandbox publication, download-status menu and the full failure/redirect/permission matrix remain incomplete; new downloads initiated during Agent work are still denied.

## Independent local search

The shared `headless_page` engine now owns both restricted search extraction and anonymous rendered-HTML snapshots. Search retains its exact engine-origin allowlist and fixed public DNS policy. RenderContent uses SSRF-validated system DNS for public GET resources, forwards browser-computed CORS/referrer headers, waits for pending broker requests and bounded DOM quiet, and returns at most 256 KiB of UTF-8 HTML with a truncation flag. Both retain exact process/profile cleanup ownership and check the pinned browser product. NomiCore now wires Knowledge's typed port through a Kernel non-Agent operation to an independent HeadlessRenderRuntime when a verified installed release is supplied. Missing runtime remains explicitly unavailable, without HTTP fallback. In this environment, the public render smoke is blocked because system DNS maps example.com to a reserved Fake-IP address; that range remains denied.

`web.research` is a separate optional capability module in the Agent workbench's Web category. It does not alias the provider-native `web.search` / `web_search` feature, require a model-native search feature, or grant Browser automation.

Its Headless owner uses an anonymous temporary context, an origin-restricted request broker, and exact process/profile cleanup. It never takes conversation tabs or login state. Runtime/adapter identity is frozen into the capability metadata and checked again for the Session and request.

Windows desktop startup now supplies an installed Chrome 120+ release using known installation paths, PE version metadata and a file fingerprint. Discovery does not execute Chrome, send searches or read old browser preferences. Missing or invalid installations remain unavailable. The live product is checked again before creating the search page, and changed releases reject the old binding. Public search and the capability catalog are verified; complete main-application Agent interaction acceptance remains unfinished.

Isolated search resolves only its fixed engine domains through Google Public DNS over HTTPS and sends search terms to Bing. It does not send arbitrary user hostnames to public DNS. Every resulting address must still be public and pinned to the actual connection; Fake-IP is not exempted. Other HTTP consumers keep their existing system-DNS policy.

Knowledge freezes the exact Browser Provider/source at composition and re-admits each request against the current registry generation/digest without silently selecting another Provider. It uses a Knowledge service principal, not a fabricated AgentSession, Snapshot or Workspace resource binding. Input is URL-only; output is final_url/html/html_truncated. Rendering allows two active jobs, sixteen total active/queued jobs, and a thirty-second queue deadline, supporting existing four-source batches. Cancellation retains ownership until join; cleanup failure or engine panic closes admission and cancels other jobs before releasing the slot, retaining failure evidence for shutdown. Startup resumes Knowledge work only after Kernel/Provider composition. A recording engine verifies actual Knowledge snapshot persistence, while real Chrome verifies private-target refusal without HTTP fallback; positive public rendering remains unverified due to Fake-IP DNS.

## Retirement boundary

The Nomi factory now receives an explicit BrowserRuntimeTarget rather than inferring Headless from a missing Workspace. Host classification reads persisted Conversation owner/source/cron/execution facts even without a native surface. Ordinary interactive chats can run without Browser, but selecting Browser on a host without native support fails explicitly instead of creating external Chromium. The old Headless Provider is removed: Browser automation for channel, scheduled and execution-step consumers is explicitly unavailable pending a new v2 owner, without Hub/private-engine fallback. Independently composed local search and Knowledge rendering remain available with their verified runtime.

Frontend v1 management/settings/DTO code and global management/login APIs are removed. The application's old Headless Hub construction, lease issuer, recovery entry point, telemetry/lifecycle loops, inventory forwarding and dedicated tests are physically removed. Boundary checks reject all production Hub construction instead of allowing one old root. Nomi manager Lane bindings, lease teardown, their tests, the factory binding module and the obsolete resolved-config flag are also removed. Native settle→terminal→finish and independent SSH/MCP/process cleanup remain. The nomi-browser crate, old bootstrap Browser registration/visual adapters/feature/prompt parameter, BrowserConfig/merge rules/storage API, and platform Hub/Lane/lease/scheduler/resource/identity modules are physically deleted. Platform now contains only revision, run_guard, runtime, uploads, url_projection and workspace. Remaining startup resource plumbing and old standalone engine entry points still require review.

Application startup no longer creates or recovers the old Hub's `browser-v2/headless/profiles/`, nor scans `browser-data` or `platform-profiles`. Existing old directories are untouched; new anonymous search and rendering retain their own process/profile cleanup. Vault hooks, key forwarding, BrowserTool's shared persistence and the engine vault module are removed. Engine in-memory snapshot utilities remain, but the new native and anonymous background paths do not import identities from an old shared vault.

The unused Hub-backed knowledge `BrowserFetcher` and its private lease/queue tests have been physically removed, including Agent-crate exports. Knowledge's existing `rendered_content_to_page` owns HTML conversion, with retained conversion/truncation coverage and fail-closed Provider tests. NomiCore now composes `BrowserRenderContentPort` through Kernel operations, not a Hub adapter. Boundary checks reject restoring either the old files or the exported type elsewhere in the Agent crate.

The boundary scanner rejects restoration of the removed UI paths, routes, and configuration declarations. Continue implementing and testing against v2 rather than adding aliases to those paths.
