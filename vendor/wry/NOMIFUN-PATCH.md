# Native child-window ownership

Upstream: wry 0.55.1, from the published crates.io archive.
Archive SHA-256: `186f9871daa55fd9c016578b810d149de58367113db7fb72b462d2323ce19514`.
Published VCS metadata identifies `a5bf203a1c8dbb3583588382538d6521655222a8` with `dirty: true`;
the archive digest, not a claimed clean checkout, is the exact source provenance.
Original Apache-2.0/MIT license files are retained.

The only upstream source modification is in `src/webview2/mod.rs`: pass the
existing `is_child` flag to `attach_handlers` and do not install the automatic
`DestroyWindow` callback for child WebViews. Top-level windows and non-Windows
implementations are unchanged.

NomiFun already observes native WindowCloseRequested and explicitly destroys
owned children. Download-only popups request close while their browser-owned
downloads are still active; upstream's eager destruction invalidates the COM
operation before cancellation/completion can be acknowledged. The application
must wait for the authorized download before honoring this page-requested close.
Explicit user close and Stop still cancel and await downloads.

There is no event-token guessing, API detour, alternate browser, or JavaScript
event emulation. Keep this patch until an upstream host-owned close callback
provides equivalent ownership; never re-enable eager child destruction without
running the native popup/download/close conformance fixtures.

Validation: `browser_workspace_smoke --agent-only` drives the unified Runtime's
exact Browser download Action through a real target=_blank HTTP attachment and
verifies exact bytes, publication, popup retirement, and cleanup.
The other browser lifecycle fixtures continue to exercise explicit close.
