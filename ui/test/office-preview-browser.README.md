# Real Office browser acceptance

This fixture mounts the actual `OfficeArtifactViewer` and calls the actual
Office artifact routes/service. It does not mock Office responses, readiness,
or renderer output. Documents are created in a disposable temporary directory
by the verified native OfficeCLI. The backend binds loopback only and installs
a test-only owner extension; it is not a deployment server.

Use an approved native executable, verifying its platform asset SHA-256 against
`crates/backend/nomifun-runtime/components.json`, then start:

```bash
NOMIFUN_OFFICECLI_PATH=/absolute/verified/officecli cargo run --locked -p nomifun-office --example preview_browser_fixture
NOMIFUN_WEB_PORT=17931 bun run --cwd ui dev:web -- --port 17932
```

Open `http://127.0.0.1:17932/test/office-preview-browser.html` in a desktop
browser at least 880×600. Check PPT, Word and Excel content, check policy,
switch back to an unchanged file, scroll PPT to slide2, update its source, then
stop and check the last capability. Expect real content and authenticated first
paint, three cache entries before changes, retained old frame during refresh,
the same scroll position after refresh, and HTTP403/CANCELLED after stopping.
The metrics observe the real component DOM and forward every fetch unchanged.

Close the owned test tab and stop both processes afterward; the backend drops
its temporary documents and cache. This exercises the shared viewer in WebUI
browser mode, not the complete desktop shell, authenticated deployment flow,
or the conversation watcher. Those retain separate targeted coverage.
