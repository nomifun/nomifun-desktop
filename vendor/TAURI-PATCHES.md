# NomiFun external WebView creation boundary

`tauri` and `tauri-runtime-wry` are exact copies of the already installed
crates.io 2.11.2 releases. Both report upstream commit
`499df79be65ef8c0670abc0207cd9e37b55d8491` in `.cargo_vcs_info.json`.
Their upstream Apache-2.0 and MIT licenses are retained. `wry` remains the
existing vendored 0.55.1 release, including its previous native-close patch.

The vendored iOS `Sources/Tauri/Logger.swift` also has trailing whitespace
removed from twelve lines for the staged whitespace check. This formatting
normalization does not change logging behavior.

The local patch adds explicit `WebviewBuilder::external_page()` creation mode.
Its default is disabled, so application windows retain upstream behavior.
External pages omit Tauri core/plugin initialization, application URI schemes,
IPC, plugin navigation/page-load/webview-created hooks and global application
script evaluation. Native view handles, layout, title/load callbacks, profile
environments, popup binding, input, downloads and native lifecycle stay intact.
An external page with an explicit startup script is rejected before creation.
This is host configuration, independent of URL, WebView label or website code.

The runtime only installs Wry IPC when Tauri supplied a handler. On Windows,
Wry likewise omits its `window.ipc` script and disables WebView2 web messages
when no handler exists. This removes application wrappers at creation; it does
not spoof a User-Agent, alter JavaScript browser getters or delete page scripts.

Upgrade these exact forks together, review the small local patch against the
new upstream release, and run the external-page native conformance fixtures.
Do not silently restore initialization to ordinary or popup browser children.
