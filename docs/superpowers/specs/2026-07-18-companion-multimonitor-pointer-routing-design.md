# Companion Multi-Monitor Pointer Routing Design

## Goal

Make every visible desktop companion clickable and draggable on every monitor,
including mixed-DPI and negatively positioned displays, without regressing
transparent-area click-through where the operating system exposes enough pointer
information to implement it safely.

The repair covers macOS, Windows, Linux X11/XWayland, and native Linux Wayland.
It must also remove the current permanent-unclickable failure modes during
initialization, renderer/backend version skew, native sampling failure, display
changes, and effect teardown.

## Proven root cause

The companion window is created as a transparent, borderless, always-on-top
window and initially set to ignore all cursor events. The renderer polls the OS
cursor, converts it to DOM client coordinates, tests the existing
`[data-companion-hit]` targets, and restores event capture only while the cursor
is over an interactive target.

The current conversion assumes that `cursorPosition()`, `outerPosition()`, and
`window.devicePixelRatio` describe one coherent coordinate system:

```text
client = (cursor_physical - outer_origin_physical) / device_pixel_ratio
```

That assumption is false or fragile on every supported desktop family:

- On macOS, Tao 0.35.3 scales the global cursor with the primary display's
  scale factor, while it scales the window position with the display currently
  containing the window. With a 2x primary display and a 1x external display,
  the subtraction combines different units. The computed point is normally far
  outside the companion, so the window remains in ignore mode without throwing
  an exception.
- On Windows, `GetCursorPos` and `ClientToScreen` use a coherent signed virtual
  desktop coordinate space under Tao's Per-Monitor-V2 awareness. The current
  calculation therefore normally works, including negative monitor positions.
  It still depends on a cached outer-frame origin, a timely DOM DPR update, a
  borderless-window assumption, and matching snapshots while `WM_DPICHANGED` is
  moving a window across monitors.
- On Linux X11, Tao scales the cursor with the display default group's scale and
  the window origin with the current window's scale. Mixed-scale configurations
  can therefore reproduce the same category of mismatch as macOS.
- On native Wayland, Tao reports successful cursor position `(0, 0)` because the
  protocol does not expose a global pointer position. The current code treats
  that sentinel as real data, never enters its exception fallback, and can leave
  the initially ignored companion permanently unclickable.

The bug occurs before DOM dispatch. `mousedown`, click, hover, and
`startDragging()` all fail together because the native window never resumes
receiving pointer events. Existing hit-target and alpha-mask logic is not the
source of the bug.

## Success criteria

- A companion is clickable and draggable after moving wholly or partly onto any
  macOS, Windows, or X11/XWayland monitor, including displays left of or above
  the primary display.
- Mixed display scales and WebView page zoom values from 0.8 through 1.3 do not
  change the DOM point reported for the same location inside the companion.
- Changing the primary display, hot-plugging a display, crossing a DPI boundary,
  or reconnecting a Windows RDP session requires no cached-origin refresh and
  cannot permanently disable interaction.
- On macOS, Windows, and X11/XWayland, transparent areas continue to pass clicks
  to applications below the companion while declared companion hit targets
  remain interactive.
- On native Wayland, where global re-entry polling is unavailable, the companion
  remains fully interactive. The entire rectangular window captures input as an
  explicit compatibility fallback rather than silently becoming unclickable.
- Any unsupported backend, invalid native geometry, missing custom command,
  permission/version skew, initialization error, or teardown race resolves to
  capture mode (`ignore=false`).
- No renderer code subtracts global cursor and window coordinates or uses DOM
  DPR as the bridge between native and DOM coordinate spaces.

## Options considered

### 1. Correct the macOS scale factors in TypeScript

The renderer could divide cursor coordinates by the primary-display scale and
window coordinates by the current-window scale before subtracting them. This is
a small hotfix, but it encodes Tao 0.35.3's current implementation defect into
application code. A future Tao correction could double-correct the values, and
asynchronous origin/scale/size reads can still form an inconsistent snapshot
during a cross-display move. It also does not repair native Wayland.

### 2. Centralize the same global-coordinate compensation in Rust

Moving the platform branches into one Rust command reduces renderer complexity,
but it still relies on global monitor geometry, Tao's scale conventions, and
multiple time-sensitive getters. This changes the location of the assumption
without eliminating it.

### 3. Query a point relative to the actual native companion window

This is the selected design. macOS, Windows, and X11 each expose an API that
converts or queries the current pointer relative to a specific window. The
native point and the native content dimensions are normalized together, so DPI,
global origins, negative coordinates, frame offsets, and WebView zoom cancel
before the renderer sees the value.

Native Wayland intentionally returns an explicit unsupported result. Its
security model does not allow a client whose input region is empty to discover
that the pointer has re-entered the surface. Pretending that `(0, 0)` is a real
point is prohibited.

## Selected architecture

### Native pointer sampler

Add `apps/desktop/src/companion_pointer.rs` and register one app command named
`get_companion_local_pointer`. The command accepts the invoking
`tauri::WebviewWindow`, rejects every label except a non-empty
`companion-<id>`, and returns one of these JSON shapes:

```json
{ "kind": "point", "backend": "appkit", "xRatio": 0.25, "yRatio": 0.5 }
```

```json
{ "kind": "unsupported", "backend": "wayland" }
```

`xRatio` and `yRatio` are the pointer's native local coordinates divided by the
same native content width and height. Values outside `[0, 1]` are valid and must
not be clamped; they represent a cursor outside the window and prevent false
edge hits. Non-finite coordinates, missing native handles, unrealized windows,
missing pointer devices, and zero or negative dimensions return an error rather
than a fabricated point.

The command is synchronous and performs exactly one native pointer query. Tauri
runs synchronous commands on its main thread, satisfying AppKit and GTK thread
affinity and preserving the Windows UI thread's DPI-awareness context. The
existing polling interval remains 40 ms, so the native query count is no greater
than the current cursor-position IPC count.

Successful point backends are exactly `appkit`, `win32`, and `x11`. Explicitly
unsupported backends are `wayland` and `other`; unexpected native failures use
the command's error result rather than adding another response shape.

Platform implementations are isolated behind `cfg` modules:

- **macOS/AppKit:** obtain the companion content `NSView` from
  `WebviewWindow::ns_view()`, obtain its `NSWindow`, read
  `mouseLocationOutsideOfEventStream`, and convert the window point into the
  content view. Respect `NSView::isFlipped`, its bounds origin, and its bounds
  size before normalizing. All raw pointers remain inside the main-thread call;
  no Objective-C object or pointer crosses an async or thread boundary.
- **Windows/Win32:** obtain the current HWND on every call, run `GetCursorPos`,
  `ScreenToClient`, and `GetClientRect`, then normalize the resulting client
  point with that client rectangle. Point and size therefore share the same DPI
  virtualization context even when the monitor scale changes. Use
  `windows 0.61`, matching the locked runtime dependency.
- **Linux X11/XWayland:** detect the actual GTK display backend, not environment
  variables. Query the default pointer device with
  `GdkWindow::device_position_double` relative to the realized companion GDK
  window, and normalize with that same GDK window's width and height. This query
  remains valid while the X11 input shape is ignored and avoids Tao's global
  default-group scale.
- **Linux native Wayland or an unknown GTK backend:** return `unsupported`.
  Never call Tao's `(0, 0)` cursor implementation.

Target-only dependencies keep AppKit, Win32, and GTK code out of unrelated
builds. Their versions must match the versions already present in `Cargo.lock`:
`objc2 0.6`/`objc2-app-kit 0.3`, `windows 0.61`, and `gtk 0.18`.

### Renderer adapter and geometry boundary

Add a small renderer adapter that invokes `get_companion_local_pointer`,
validates the tagged response and finite ratios, and exposes a typed result. Add
a pure geometry function that maps a point sample to DOM client coordinates:

```text
client_x = x_ratio * window.innerWidth
client_y = y_ratio * window.innerHeight
```

`innerWidth` and `innerHeight` must be finite and positive. An invalid viewport
or response produces no point and activates capture fallback. Normalization is
the only native-to-DOM conversion, so WKWebView/WebView2/WebKitGTK page zoom and
display scale do not require separate renderer inputs.

The existing `isPointOverCompanionHitTarget` function remains responsible only
for DOM client-space rectangles, visibility, tolerance, and avatar alpha masks.
It receives the corrected client point but otherwise does not change.

### Click-through state machine

Refactor the effect so setup, one polling tick, failure recovery, and disposal
have explicit state transitions:

1. After building the native window, Rust queries the actual platform/backend
   before choosing its startup mode. AppKit, Win32, and X11 may initially use
   passthrough while the renderer is not ready, preventing a blank transparent
   startup window from blocking the desktop. Wayland, an unknown backend, or a
   backend-detection error starts in capture mode because no reliable recovery
   sampler has been established.
2. As soon as the companion effect owns the visible renderer, it first requests
   `ignore=false`, before importing or sampling any pointer geometry.
3. A valid point is converted to client coordinates and tested against the
   existing hit targets. Only then may the state become `ignore=!over`.
4. `dragging` and `captureAll` always force capture and skip sampling.
5. `unsupported` enters a stable capture-only mode and stops native polling.
   While in that mode, DOM `pointermove`/`pointerleave` updates hover state so
   hover-revealed companion controls still behave normally.
6. A transient command or geometry error forces capture and retries sampling on
   a one-second recovery cadence. A later valid sample resumes 40 ms polling.
   Repeated failures warn once per effect instance and do not flood IPC or logs.
7. Effect disposal marks the generation inactive before awaiting anything,
   removes timers/listeners, clears hover, and requests `ignore=false`
   unconditionally. Every asynchronous continuation checks that generation
   before mutating hover or native ignore state.

The cached last-ignore value is updated only after a successful native request.
Initialization has a top-level catch that performs the same capture fallback;
an origin-read failure can no longer be swallowed while leaving the initial
ignore state untouched. Origin caches, move/resize listeners, the one-second
origin heartbeat, `cursorPosition`, `outerPosition`, and `devicePixelRatio` are
removed from this feature.

### Linux compatibility boundary

X11 and XWayland retain rectangular/alpha-aware click-through through the
normal polling path. Native Wayland guarantees interaction by capturing the
whole companion window. This is a deliberate fail-operable policy: Wayland does
not expose the global pointer information required to switch a fully ignored
surface back on.

Per-target Wayland click-through would require a separate input-region design in
which DOM hit rectangles or alpha runs are synchronized to the compositor. It
is not emulated with stale pointer positions in this repair because that would
reintroduce permanent and false-edge failures.

Native Wayland also does not guarantee programmatic placement of a top-level
window on a selected monitor. User/compositor dragging remains supported, but
exact saved-position restoration across displays is a compositor protocol
limitation and must not be represented as a pointer-routing success or failure.

## Error and safety policy

- Interaction wins over transparent-area click-through on every uncertainty.
  The safe value is always `ignore=false`.
- The custom command is window-label scoped and returns no global cursor or
  display topology, minimizing exposed native information.
- Native pointers are reacquired on every call and never cached.
- Native ratios are never clamped, rounded, or converted through unsigned
  coordinates.
- A new renderer talking to a stale Rust shell without the new command remains
  clickable through the existing `setIgnoreCursorEvents(false)` API and retries
  only on the recovery cadence.
- Cleanup does not trust the cached ignore state. It always requests capture so
  a hidden and later re-shown companion cannot inherit permanent passthrough.
- No platform-specific compensation is applied to another platform. In
  particular, macOS primary-display scale logic is not introduced on Windows.

## Test strategy

### Rust unit tests

- Normalize ordinary, negative, and outside-window local points without
  clamping.
- Reject NaN, infinity, zero dimensions, and negative dimensions.
- Verify macOS flipped and unflipped Y conversion, non-zero view-bounds origins,
  and all four boundaries.
- Verify the window-label guard accepts only non-empty `companion-<id>` labels.
- Keep platform API wrappers thin enough that the shared normalization and
  validation functions carry the deterministic behavior tests.

### Renderer Bun tests

- Convert normalized points at viewport sizes representing zoom 0.8, 1.0, and
  1.3, including ratios below zero and above one.
- Reject malformed tagged responses, non-finite ratios, and invalid viewports.
- Prove the current mixed-DPI regression reaches the existing hit target after
  normalization.
- Exercise the controller with mocked native dependencies: initialization,
  inside/outside transitions, dragging, `captureAll`, unsupported Wayland,
  transient failure and recovery, old-shell command absence, disposal during an
  in-flight tick, and unconditional cleanup capture.
- Add a wiring contract that fails if the hook reintroduces `cursorPosition`,
  `outerPosition`, or `window.devicePixelRatio`.

### Build and regression checks

- Run the focused pointer/hit-target Bun tests, then the full companion-page Bun
  test directory.
- Run the UI TypeScript type check.
- Run desktop Rust tests, Rust formatting checks, and a current-host desktop
  compile check.
- Use platform CI to compile the target-only macOS, Windows, and Linux modules.
  Windows compilation must include `x86_64-pc-windows-msvc`; Linux must compile
  against GTK 3.24.

### Hardware/runtime matrix

- macOS: Retina 2x plus external 1x in both primary-display directions; external
  display left, right, above, and below; change primary display; cross the scale
  boundary; hot-plug; zoom 0.8/1.0/1.3.
- Windows: display scale pairs 100/100, 100/125, 125/150, 150/100, and 200/100;
  negative X/Y layouts; cross-display dragging; primary-display change; hot
  plug; RDP reconnect; zoom 0.8/1.0/1.3.
- Linux X11/XWayland: scale 1 and 2, ignored-input state, inside/outside sampling,
  negative layouts where supported, and XWayland detection inside a Wayland
  session.
- Linux native Wayland: explicit unsupported response, final `ignore=false`,
  click and drag reception, working hover fallback, and no `(0,0)` false hit.
- On supported polling backends, verify both sides of the contract: the
  companion receives click/drag inside its hit targets, and a sentinel window
  below receives clicks through transparent areas.

## Known protocol boundary

Polling has an intentional maximum transition latency of one 40 ms interval. A
mouse user who moves from outside the window and clicks faster than that interval
can still send that first click through. Eliminating this bound, including strict
touch-first-click support, requires native synchronous hit testing or compositor
input regions driven by DOM geometry; it is a separate interaction architecture,
not a coordinate bug fix. This repair does not conceal that limitation, but it
does guarantee that the companion cannot remain permanently unclickable.

## Out of scope

- Forking or patching Tao/Tauri.
- Adding compositor-specific Wayland layer-shell placement protocols.
- Building per-target Wayland compositor input regions in this change.
- Changing companion hit-target rectangles, avatar alpha-mask semantics,
  drag behavior, visual layout, or persisted window geometry.
- Replacing polling with platform-specific native event-hook/subclass systems.
