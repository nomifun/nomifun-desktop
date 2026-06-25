# WebUI Remote Access

The desktop app already runs a backend on a localhost port for its own webview — why not just expose it? Because exposing an unauthenticated backend on a LAN would hand every device on that network full shell, file, and agent access.

**WebUI remote access** solves that. The desktop backend runs under a *trust-local-token* policy: the desktop's own webview is trusted via a per-boot secret it presents on every request (so you never log in locally), while any other client must authenticate. With one switch, an additional listener is bound on a stable LAN port that serves the app to remote browsers behind a login (password + QR), so you can use Nomi from your phone or another browser without giving up local-mode convenience.

This is **per-instance** — it lives inside your already-running desktop app — and is distinct from the dedicated [Web Server Deployment](./web-server-deployment.md). Use this when you have an existing desktop install and just want to reach it from another device on the same network. Use the dedicated server when you want a long-lived headless deployment.

![Open Capabilities WebUI panel](../images/webui-01-settings-overview.png)

## Where to find it

Open **Open Capabilities** (route `/open-capabilities`) and use the WebUI
remote-access panel. The legacy `/settings/webui` route redirects there.

- **WebUI remote access** controls the desktop LAN listener described in this guide.
- Other cards on the page manage public/remote capability exposure and should be
  reviewed separately before enabling them.

> The WebUI remote-access controls are meaningful inside the desktop shell. In a
> browser tab against `nomifun-web`, you are already using the dedicated Web host;
> use [Web Server Deployment](./web-server-deployment.md) settings instead.

## What enabling it does

Toggling **Enable WebUI** on starts an additional authenticated server inside the desktop process:

- **Default port `25808`** (`25809` in dev mode, `25810` when `NOMIFUN_MULTI_INSTANCE=1`).
- An admin user (default name `admin`) is provisioned with a freshly generated random password — shown in plaintext **once**, on this first start, so you can copy it.
- The server's lifetime is tracked by the desktop main process; the toggle reflects the *real* server state, not a remembered preference, so a silent failure (port conflict, etc.) leaves the switch off rather than misleading you into thinking it is up.

## Architecture: two listeners, one backend

The desktop process serves its backend on **two** sockets that share one in-process router (built once):

- A **permanent loopback listener** on an ephemeral port — the desktop's own webview, trusted via the per-boot secret. Always up; never disturbed by toggling remote access.
- An **on-demand LAN listener** on `0.0.0.0:25808` — bound only when you enable remote access, torn down when you disable it. Remote browsers reach this one and must log in. Trust is the secret (which only the desktop webview holds), *not* "arrived on loopback", so other OS accounts on a shared workstation and same-host reverse proxies are **not** auto-trusted. The LAN listener additionally enforces a Host/Origin allow-list (IP/localhost only, blocking DNS-rebinding) and rate-limits by real peer address.

Because of the exclusive data-dir lock, the desktop process is the only backend on its data directory — so the LAN listener lives *inside* the desktop app, it is not a co-running `nomifun-web`.

## Binding and the access URL

Enabling remote access binds **`0.0.0.0:25808`** (`25809` in dev; falls back to an ephemeral port if `25808` is taken) so other devices on your network can reach it. The displayed URL adapts:

- **The desktop's own machine**: `http://localhost:<port>`.
- **Remote (LAN/VPN)**: `http://<your-LAN-IP>:<port>` (e.g. `http://192.168.1.42:25808`). The candidate interface addresses are detected from the host's network interfaces; on a VPN host with multiple adapters, confirm the advertised address is the one your phone can actually reach.

A copy button copies the URL; clicking it opens it in your default external browser. The QR-code login is shown while the LAN listener is running.

## Login: username and password

The credentials panel shows:

- **Username** — defaults to `admin`. Editable via the pencil icon (server-side validation: 3–32 chars, `[a-zA-Z0-9_-]`, must not start or end with `-` / `_`).
- **Initial password** — shown in plaintext on the *very first* start, masked as `******` after that. The plaintext can be copied while it is visible. Once you copy it (or the first session ends), it switches to masked permanently.
  - The plaintext is only shown once because the backend stores a bcrypt hash, not the plaintext. After the first display, even the desktop UI cannot recover the original.

To change the password later, click the pencil icon next to the masked field. The form requires the new password and a confirmation; on success the new value is hashed and persisted, and the cached plaintext is cleared. The password validator rejects values shorter than 8 characters and a small list of common passwords (`password`, `12345678`, …).

The "reset password" path (when you forget it) generates a fresh 16-character random password server-side; a one-time displayed value, like the initial one.

![Login screen on the remote browser](../images/webui-03-login-screen.png)

## QR-code login

While WebUI is enabled (the LAN listener is running), a QR code appears in the credentials card.

- Scanning it from your phone opens `http://<host>:<port>/qr-login?token=<one-time>` in the phone's default browser.
- That URL hits a static page that calls `POST /api/auth/qr-login` with the token. The token is single-use and validated atomically; the server returns a session cookie + JWT and the page redirects to `/`.
- Tokens **expire after 5 minutes**; the UI auto-refreshes the QR every 4 minutes so a panel left open does not invalidate.
- A copy button next to the QR copies the full login URL (useful if your phone cannot scan), and a refresh button regenerates the token on demand.

QR login always logs you in as the configured WebUI admin (the primary admin user), regardless of how many users exist in the database — it is the per-instance "skip the password form" shortcut, not a multi-user feature.

![QR code login on phone](../images/webui-04-qr-login-phone.png)

## How this differs from `nomifun-web`

| | WebUI remote access | `nomifun-web` (Web Server Deployment) |
|---|---|---|
| Where it runs | Inside your already-running desktop app | A separate, headless binary |
| GUI required to start | Yes (the Settings toggle) | No |
| Admin provisioning | Auto-generated password on first enable | Interactive first-run setup, or `NOMIFUN_ADMIN_PASSWORD` |
| Default port | `25808` (prod), `25809` (dev) | `8787` |
| Survives reboot | Only if your desktop app is running | Yes, with systemd / Docker `restart: unless-stopped` |
| TLS | None built in (LAN-oriented) | Caddy / nginx in front; `NOMIFUN_HTTPS=true` |
| Use case | Quick remote access from a phone on the same network | A real always-on server |

If you find yourself leaving the desktop app running on a server-like box just so the WebUI server stays up, that is the cue to switch to a dedicated [Web Server Deployment](./web-server-deployment.md).

## Security notes

- The server listens on plain HTTP. Use it on a **trusted local network** (your home Wi-Fi, a VPN, Tailscale, etc.). For exposure beyond that, deploy `nomifun-web` behind a TLS reverse proxy instead.
- The admin user has the same capabilities as the local desktop user: shell access, file access, agent execution. Treat the admin password and QR tokens accordingly.
- Changing the password (in-app or via reset) invalidates all existing sessions because the JWT signing secret rotates atomically with the password update.
- The QR token is one-shot — once scanned and consumed it cannot be reused. A leaked token is therefore self-limiting, but a leaked URL **before** scanning still grants login. Don't post screenshots of the QR.

## Troubleshooting

**Toggle flips back to off immediately.** Another process is bound to the WebUI port. Pick a different port if you can configure it from the UI; otherwise stop whatever is holding `25808`.

**The QR code shows but my phone gets a connection error.** Check the LAN IP shown in the access URL — if your machine has multiple interfaces (Wi-Fi + Ethernet, VPN adapters), the auto-detected address may not be the one your phone can reach. Confirm your phone is on the same network/subnet, and that the firewall allowed Nomi when prompted.

**`./qr-login?token=…` says "Login failed: …".** The token expired (5-minute TTL) or has already been consumed. Click the refresh button next to the QR to mint a new one.

**I forgot the admin password.** Use the reset button (the pencil + reset icon next to the masked password), then sign in with the freshly generated value — it is shown once.

## See also

- [Running NomiFun as a Desktop App](./desktop-app.md)
- [Web Server Deployment](./web-server-deployment.md) — when you want a real always-on server, not a desktop side-channel.
