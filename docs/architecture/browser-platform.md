# Browser Workspace

This document, the [Agent Session architecture](agent-session.md), and current source define the development contract.
The built-in side browser is a user work surface. Agent Browser Use is a separately authorized tool consumer.

## Product boundary

The embedded browser remains the primary browsing experience. Compatibility, navigation deduplication, Pause, diagnostics and recovery stay within its native panel. System-browser opening and authorized attached Chrome remain explicitly selected features for particular workflows; they are not a primary replacement or automatic failure fallback. External browsers may provide acceptance comparisons, never substitute for embedded-browser evidence.

The Browser domain owns one managed browser per authenticated owner and canonical AgentSession. The side-rail Browser
button permits direct user browsing without a Browser Module grant, Provider selection, or Agent resource binding.
Opening, retrying and user commands never create Agent authority or a second Session. Retired global management and
browser Settings surfaces remain removed.

The page is a native embedded WebView, not an iframe, screenshot stream, canvas viewer, or JPEG transport. The user and Agent operate the same page. After the Agent first uses this browser, native user input and page-changing chrome controls stay locked until that Turn ends and cleanup completes. A Turn that never uses it leaves it available to the user. There is no user takeover or hand-back mode.

Hiding the surface preserves its tabs. The Session has one managed runtime, profile and input coordinator.
`BrowserResource` wraps exact frozen Agent authority over the same `BrowserWorkspace`; a different authorization
definition never selects another page or profile. Agent operations still require the frozen action allowlist,
exact Provider, typed resource binding and current BrowserRunGuard. A user opening a page never grants Agent tools.
An Agent using attached Chrome leaves the user's managed browser independent. A different exact managed Provider
cannot silently reuse the existing entity.

The shared desktop renderer supports an inner viewport of at least 880×600. Chat and the Browser/Preview surface retain their horizontal split: their minimum widths are 360px and 340px, plus a 32px session tool rail. While Browser is open, Layout and ConversationShell derive temporary sidebar collapse from their complete container widths and the user's preferred sidebar widths, so hiding a sidebar cannot create a collapse/expand feedback loop. The main navigation and Session list yield only the space needed; a wider window, panel close, Session change or unmount releases the reservation and restores the preferred state. Border-box sizing keeps the measured sidebar and rail widths consistent with these limits. Visible split sizes are constrained without rewriting saved split or workspace-width preferences.

Only Browser registers the space reservation and its sidebar-expansion dismissal. Dismissal hides/detaches the Browser surface, retaining pages, runtime and input ownership; it does not destroy tabs or release an Agent's lock. Preview keeps its existing sidebar-collapse policy: layout callbacks never receive `closePreview`, which clears tabs and unsaved edits. Switching between Browser and Preview also transfers sidebar management. Layout, native input and active-Agent Stop require separate evidence; see [continuing validation](../validation/browser-platform.md).

Turn preparation freezes Browser authority without acquiring the managed browser input gate. An actual authorized
managed Browser invocation acquires its current Workspace and guard before native work. The first such invocation
retains ownership for the entire remaining Turn; any native child first created under that ownership starts with user
input disabled. A completed browser tool, ordinary tool failure, model thinking, unrelated tools and paused execution
do not release the gate. Ordinary chat, file work, attached Chrome and background rendering do not lock the managed
browser when the Turn never invokes it. Neither configured Browser permissions nor model prose predict ownership.

The retained owner settles native operations with input still locked. Only the exact durable Turn terminal followed by
successful finish releases ownership. Pending navigation or dialogs, unsettled or unknown effects, receipt persistence
failures and uncertain native cleanup remain protected by the original Turn's recovery. A failed acquisition that issued
no guard retains the same drain/terminal/failed-start-recovery boundary. Completion of an individual tool or native
operation never authorizes a mid-Turn unlock.

User APIs verify the authenticated owner and canonical Session, then admit commands through the Workspace coordinator.
Session `running` or `paused` does not itself lock the browser. The Browser snapshot reports actual input ownership and
gate failure; it is not overwritten from global Agent state. This same admission controls navigation, tab closure and downloads.

Terminal publication and native final release are separate receipts.
If release fails after publication, an SDK retry must match the same root's canonical receipt, original Runtime terminal, delivery, Snapshot and route before retrying
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

The browser menu offers an explicitly confirmed Reopen browser action. It closes page tabs and loses unsaved page content, but does not clear conversation messages or project files. The backend holds the same Session operation fence, validates the exact runtime generation and closes through the coordinator's actual idle gate. A Turn that has never used this browser does not prevent this close. Once the Turn has acquired it, later unrelated work does not permit closing; the user cannot cancel or steal its guard through a browser command.
If the final unlock failed, closing still requires that exact Operation's canonical terminal, proven
native settlement and matching old Workspace instance/generation. `turn/paused` retains the same active Operation and
native checkpoint resume authority, so it cannot substitute for this final-terminal witness. If ownership remains held,
the user must finish the Operation through the formal Stop/cancel path and prove Runtime cleanup before this recovery
close. The old Workspace's confirmed destruction lets the SDK acknowledge its existing terminal and retry final release.
The new user Workspace has another generation and cannot be released by the old owner; the model is not replayed and
the original outcome is not rewritten.

macOS embeds a visible WKWebView child using Tauri's existing AppKit main thread and event loop. WK/AppKit
objects are created, used and released on that thread. There is no second NSApplication, downloaded browser
runtime, bundled system WebKit.framework, CEF helper/guardian, framework preload, message pump, allocator
exception or fallback engine. System WebKit owns WebContent processes.

## Browser identity and navigation recovery

Before the first request, ordinary macOS tabs and popups share one native desktop Safari compatibility identity: `macos-desktop-webkit-safari17-v1`. Its `Version/17.0` token is an advertised compatibility baseline, not a measured Safari/WebKit runtime version. Actual OS information is reported separately and unknown framework metadata remains unknown. The policy neither changes Session/Profile identity nor clears cookies, bundles Safari or another engine, patches JavaScript getters, or sets individual request headers. Windows retains its native identity.

One Page reducer owns request/start/redirect/commit/finish/cancellation/failure/process-termination state. The `load` summary reports a navigation control sequence, phase, displayable content, rounded progress, safe problems, and attempted/content addresses. Internal bootstrap has explicit source identity and is not user content; genuine about:blank popups can hold documents. Cancelled attempts map to `Stopped`, never successful navigation. Cancellation classification checks domain and code. Retained native navigation objects and dispatch receipts reject late callbacks, stale Stop commands, and old layout updates.

Panel/layout admission, native child displayability, and input ownership are independent. Empty failed/stopped/crashed pages hide the child on the native main thread and show host recovery content; retained/partial documents remain visible with their attempted/content address distinction. Every native show reapplies the current Page mask. Authorized Agent recovery navigation waits for layout without requiring an executable old document. Empty reads return `BROWSER_PAGE_FAILED`, `BROWSER_PAGE_STOPPED`, or `BROWSER_PAGE_CRASHED`; valid retained/partial content can be observed and captured with a safe load summary. Existing execution-uncertainty errors remain intact: no automatic DOM, POST, or Agent action replay.

User downloads use panel presentation independently of the document mask. A first direct attachment can continue destination confirmation while an empty document is hidden; actual panel hiding, input-owner changes and process termination still revoke pending choices. DOM interaction, media permissions and upload selection continue to require actual document visibility.

Windows keeps both the controller and child HWND hidden throughout every `content_state=None` phase, including requested/provisional recovery after a failure; a new request does not clear old error pixels retained by the controller. A real non-error document commit restores the same slot without replaying resize/show. `CurrentDocument` and `RetainedDocument` retain the existing panel and capture rules; actual HTTP 403 content is not hidden because of its status code. Pause controls the current tab; other open tabs keep running, and independent navigation of an existing named window is distinct from creating a new popup.

The Page-owned main-document trace is ephemeral and bounded to 32 attempts, 128 events, and a 128 KiB serialized-event budget. Capacity loss increments dropped independently of routing-proof loss. Response callbacks have no WKNavigation identity; unproven response association remains a Page-scoped sample and does not update the current response summary. Matching URLs or the latest sequence are insufficient proof. This does not claim console, subresource, or Fetch/XHR coverage; existing unavailable semantics remain.

The user explicitly copies navigation diagnostics through an owner-checked read of the existing Workspace and exact target. It does not initialize a runtime, grant Agent authority, or upload data. Actual navigation/address bars retain original URLs; Agent/diagnostic metadata removes userinfo, query and fragment through the shared safe projection and enforces text limits. Cookies, credentials, bodies, form values and arbitrary NSError descriptions are excluded. Recovery remains explicit and tab-scoped; slow-load hints never trigger infinite reloads, bypass challenges, or clear the whole workspace.

Both adapters retain host dispatch, native attempt and public action facts. Dispatch and native attempt sequences are independent; unproven samples receive no document identity. A native Page owner maintains Windows NavigationId, success/error/cancellation/download state; the simplified Wry PageLoad callback no longer writes Browser state. Cumulative `counters` survive ring eviction and routing loss remains distinct from capacity loss. macOS policy samples expose public action type, method and source frame/origin without claiming association to the latest WKNavigation or proof of a physical gesture.

On Windows, Back/Forward advances the current control fence and records the host dispatch while preserving the actual document's phase, progress, problems and native binding. Same-document history therefore does not wait for a new full-document load. Only a real NativeStarting creates a new loading attempt; the empty `Page.navigateToHistoryEntry` reply does not prove completion, and SourceChanged(false) only updates metadata. The wire `load.navigation_sequence` can advance without a new native document attempt, so native cancellation/trace identity uses the exact NavigationId and its original attempt sequence. Late history origin remains unknown when a successor reply cannot establish its source. BeforeUnload retains its existing exact command/dialog scope and native decision; it does not restore snapshots or invent a cancelled attempt.

On macOS, Back/Forward also advances the control fence and records dispatch while retaining the current document and native attempt state. Only `didStartProvisionalNavigation` for the exact pending `WKNavigation` binds that dispatch to a new document loading attempt. A command reply, URL/title changes and metadata sampling prove neither a new document start nor completion. Address changes on a settled document still conservatively advance document generation to invalidate old DOM references, without inventing a main-document request or new attempt. Stop/cancellation receipts distinguish the exact history dispatch from the original native attempt instead of rebinding an old navigation to the latest control sequence. [Continuing validation scope](../validation/browser-platform.md) are recorded separately; passing units do not establish WK production GUI acceptance.

Optional `load.navigation_control` reports advisory repeats and a Page hold. Short-term repeats compare raw main-document addresses through Page-salted in-memory hashes; redirects, ordinary resources and unassociated actions do not count as reloads. Warnings never pause automatically. Explicit Pause blocks later page-originated main navigations and popups while retaining usable content. Resume only releases the hold; guarded address/reload/history commands also release it. Stop cancels the current load, not future website timers or all Fetch/XHR. Stop/Pause may cancel the exact pending human navigation, but join its native settlement before completion. They cannot cancel Agent operations, unlock early, or replay Stop against a successor document.

Windows NavigationStarting.Cancel may arrive after a GET has been sent, so Pause also intercepts the exact root Document at the CDP Fetch request stage. After the first explicit Pause, the interceptor remains attached until native controller closure. Resume changes logical state instead of using Fetch.disable to release old requests whose events have not arrived. Native root frame identity, Fetch.networkId and the earliest Network creation time establish request ownership without changing HTTP methods, headers or bodies. The shared clock proof currently covers x86/x64 CPUs reporting invariant TSC. Other CPUs omit navigation_control and return UnsupportedAction for Pause; ordinary navigation and Stop remain available. Supported capability and actual native acceptance are recorded separately in the [continuing validation](../validation/browser-platform.md).

Windows ordinary tabs, popups and site-data maintenance views use explicit `WebviewBuilder::external_page()` creation mode. It omits Tauri core/plugin initialization, application URI schemes, IPC and global application scripts before the first document; default main/companion behavior remains. Wry omits `window.ipc` when no IPC handler exists. The same Tauri window, event thread and native WebView2 ports remain, without changing website getters or deleting scripts after loading. See the [exact dependency patches and licenses](../../vendor/TAURI-PATCHES.md). Native environment reads verify actual Runtime version, Profile and UA. Directory or persistence mismatches reject external navigation without clearing or repairing override-selected data. UA is optional diagnostic metadata: an unavailable property interface, failed read or invalid value leaves identity absent and logs a fixed reason without blocking ordinary pages or popups. No UA is guessed or truncated into a successful measurement. Reports contain a process-scoped opaque Profile token and override-presence flags, never user paths or arbitrary arguments.

Here external_page names the initialization boundary for external websites inside the native embedded WebView. The page stays in the product window; this API does not open or select the system browser.

Windows prepares an ordinary local-drive UDF path only when creating a native view. It verifies that this SDK input and the original ProfileStore directory identify the same directory object; the Session namespace and cleanup path remain unchanged. Before external navigation, the effective SDK UserDataFolder getter must also use a supported representation and identify that directory. An override using an unsupported verbatim path is rejected even when it names the same folder, because native conformance reproduced Cookie loss with that representation. Matching ordinary overrides remain supported. UNC, long and special-name paths that cannot be safely represented are rejected before creating a page; compatibility for those data roots is not claimed. The [continuing validation](../validation/browser-platform.md) records Cookie, LocalStorage and IndexedDB preservation across close and application restart, separate Session isolation and override rejection.

Windows iframe auto-attachment retains startup waiting so file-chooser policy is installed before the first iframe script. Renderer workers and worklets are admitted separately and promptly resumed through their exact native session, then detached after acknowledgement. These auxiliary sessions provide no DOM route or Agent authority. Failed auxiliary releases retain bounded ownership until native detach or controller closure; auxiliary admission failures do not invalidate unrelated iframe routes. The existing bounded event transport still reports routing loss when lifecycle events are dropped. Native Worker and nested Worker acceptance is recorded separately in the remediation evidence.

macOS conformance can select an immutable `NativeWebKit` comparison policy at Engine initialization. The entire Runtime and its popups inherit that policy; the production sidebar still defaults to Safari17. Public isolated-world reads establish the effective native UA; it remains unknown until measured and is never inferred from the OS or installed Safari. Identity A/B results and production-sidebar acceptance are separate evidence. The default policy does not change before native comparisons support it.

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
prove a site's business outcome; observe again. When a site requires manual interaction, let the Turn end or use Stop,
then wait for cleanup and final release. Completing a browser tool alone does not restore manual input.
Observation and user snapshots report `interaction_capabilities`;
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

## File selection and downloads

On macOS, Agent upload remains an explicit `UnsupportedAction` in the approved WKWebView feature downgrade. This is an implementation gap, not a system permission denial; ordinary user file selection has its own native path.

On Windows, optional `browser.upload` authorizes observed visible standard file inputs or inputs produced by a real native chooser event on the current page. `browser.act` alone grants no local file access. The global source-path refactor is still incomplete: `engine_browser_tools.rs` passes the Session's default workspace into `BrowserUploadScope`; its `root: Dir`, `relative_components` and no-follow walk still reject native absolute paths, parent traversal, links and junctions. These are remaining application restrictions, not the new global-OS contract or an OS limitation. The [Windows handoff](../handoffs/global-os-windows.zh.md) requires native design, implementation, compilation and WebView2 validation for Session-relative and absolute sources, legal links, UNC and cross-volume sources, directory-handle races and ACL behavior. Regular-file checks, source identity, before/after metadata checks, cancellation and retained-snapshot ownership must remain reliable. The host retains snapshots with original names, bytes and modification times: at most 16 files/64 MiB per call and 128 files/256 MiB per Runtime, released after real native closure. Tool completion or page reset does not invalidate File objects still retained by a page.

WebView2 delivers through `DOM.setFileInputFiles` without synthetic input/change events; empty lists are rejected. Frame, document, Session and current-request checks precede assignment. New iframe routes install chooser policy before initial scripts. Unknown routing, policy failure and stale observation grant no file authority. Directory upload, File System Access, Agent download/sandbox publication and complete abnormal-exit reclamation require separate delivery and acceptance.

An isolated helper owns user Open/Save dialogs through a bounded private protocol. Only visible, live, idle tabs may request selection. Hiding, navigation and Agent acquisition of this browser revoke unconfirmed choices; cancellation awaits real process-tree exit. Windows safely parses HTML `accept` as a filter hint while retaining all files; public WK APIs do not expose equivalent filtering metadata on macOS. Filters prove neither file authority nor content validity.

User downloads retain native DownloadStarting/download ownership, with at most four active requests and one pending Save dialog per tab. Authorized transfers may continue when the Agent acquires this browser, but new user downloads are denied while that ownership is held. Tab closure cancels and awaits native terminal state; failed cleanup retains retry ownership. Default download UI cannot bypass the input lock. See [continuing validation](../validation/browser-platform.md) for incomplete network, redirect, permission and platform coverage.

## Independent search and rendering

`web.research` is optional and separate from Browser automation and provider-native search. Its anonymous `headless_page` owner uses temporary contexts, restricted-origin forwarding and exact process/Profile cleanup without visible tabs or login state, and never acquires the managed browser input gate. Search resolves only fixed engine domains through fixed public DNS. Content rendering uses system DNS with strict public-address validation, connection pinning and GET-only forwarding, retaining browser CORS/Referrer. Fake-IP and private networks receive no exemption.

Windows/macOS discover Chrome 120+ in known installation locations and freeze its release/file fingerprint, rechecked before actual page creation. Discovery neither scans arbitrary PATH nor runs or installs the browser. Missing qualified dependencies remain unavailable without blocking startup. macOS fingerprints cover the real framework, helpers, Libraries and Resources, not just the launcher.

Knowledge freezes the exact Provider/source and re-admits through Kernel non-Agent operations and an independent HeadlessRenderRuntime without inventing Session, Snapshot or workspace identity. URL input yields `final_url/html/html_truncated`; HTML is limited to 256 KiB, with conversion owned by Knowledge. Limits are two active jobs, sixteen active/queued jobs and a thirty-second queue deadline. Cancellation retains ownership until join; cleanup failure or panic closes admission and preserves shutdown failure. Missing runtime and unsafe sources never fall back to ordinary HTTP.

Visible WK/WebView2, Attached Chrome and background Chrome engines require separate evidence. Passing one path does not validate another. Public rendering, real-model and device gaps are tracked in [continuing validation](../validation/browser-platform.md).

## System browser and retirement boundary

Attached Chrome uses an explicitly authorized connection without acquiring the managed browser input gate. Temporary debugger rendering overrides remain from the first non-observation action until run settlement, rather than being released immediately after one wheel acknowledgement. Uncertain setup/release closes only NomiFun's connection, without closing user tabs or changing Profile, launch arguments or OS windows.

Host classification uses explicit `BrowserRuntimeTarget` and persisted owner/source/cron/execution facts, never a missing Workspace. Ordinary chat may omit Browser; selecting it without a native host fails explicitly. Background automation for channels, schedules and execution steps still lacks a current v2 owner. Do not restore Hub/Lane, leases, private engines, shared vaults or old Profile import. Independent search and Knowledge rendering keep their own owners.

## Sources and acceptance

- [User workspace owner](../../crates/backend/nomifun-app/src/router/browser_workspace.rs) and [Agent tool seam](../../crates/backend/nomifun-app/src/router/engine_browser_tools.rs).
- [Knowledge composition](../../crates/backend/nomifun-app/src/router/knowledge_browser.rs).
- [Platform interface](../../crates/backend/nomifun-browser-platform/src/lib.rs) and [engine](../../crates/agent/nomi-browser-engine/src/lib.rs).
- [Continuing validation and incomplete coverage](../validation/browser-platform.md).

Run affected native, platform, Runtime or UI checks and current Browser boundary checks. Renderer/UI-rule changes also run `bun run check:desktop-ui-boundary`. Historical logs, screenshots and assertion counts remain in Git rather than becoming compatibility contracts.
