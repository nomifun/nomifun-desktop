# Connections & Devices

Open **Connections & Devices** (`/connections`) in the primary navigation and choose your task:

| Task | Entry | Access provided |
|---|---|---|
| Use Desktop from a phone | **Connect a phone** | A one-time NomiFun Mobile pairing code and an independent device credential |
| Use Desktop from another computer | **Other computer browsers** | WebUI address, administrator login, or a one-time browser login link |
| Call Desktop from an external Agent or tool | **External tools** | MCP / REST configuration and an installation access token |

Desktop owns the business data and execution. Mobile is a connected client; NomiRelay is optional transport.
Configure IM channels and physical robots under each companion's **Channels & Robots**. Both destinations link to each other.

The shared WebUI renderer supports desktop windows and browsers of at least 880×600.
Phones use the separate NomiFun Mobile app.

Legacy `/open-capabilities` and `/settings/webui` links redirect to the browser task;
`/settings/tools` redirects to external tools.

## Pair a phone on the same network

1. Start Desktop, open **Connections & Devices → Connect a phone**, and confirm the displayed Desktop identity.
2. Select **Same network** and **Enable network access**.
3. Choose a Desktop address your phone can reach. With several adapters or a VPN, choose a connected network.
4. Select **Generate phone pairing code**.
5. In NomiFun Mobile, select **Add Desktop**, then scan the code or paste the phone pairing link.
6. Wait for Desktop's pairing receipt and verify the phone name under **Paired phones**.
   The receipt means a device credential was issued. Mobile then completes the Desktop identity and protocol handshake for its business connection.

Codes expire shortly and can be redeemed once. The page shows remaining time and hides used or expired codes.
Generating a new code cancels any earlier code that has not been used. If Desktop cannot finish pairing (for example, when it already has the maximum number of paired phones), the page says so instead of showing a receipt.
Select **Generate a new pairing code** for another phone. Every phone receives its own credential;
model keys and the database stay on Desktop.

The phone and Desktop must share a reachable LAN or VPN. To use Desktop after leaving this network,
configure relay access and add its endpoint in Mobile.

## Pair across networks through a relay

Use your self-hosted NomiRelay, or obtain a Console address and enrollment code from your administrator.

1. Select **Across networks (relay)** in **Connect a phone**.
2. Enter the Relay Console address and select **Open console**. A deployment guide is available if no relay exists yet.
3. Create a v2 Desktop enrollment code in Console, paste it into Desktop, and select **Connect relay**.
4. Verify **Relay online**, Desktop API availability, and cross-network endpoint availability.
5. Generate a phone pairing code, scan or paste it in Mobile, confirm the Desktop identity, and finish pairing.

The relay enrollment code is for Desktop; the phone pairing code is for Mobile.
Mobile does not need relay administrator credentials or an `nfagent` enrollment secret.
Older v1 enrollments prompt an upgrade and reconnection before using the new device credential flow.
Desktop keeps one relay connection at a time. To use a different relay or a new enrollment code, disconnect the current one first.

Phone pairing codes are generated only when you select **Generate**; connecting or restarting the
relay never creates a pairing code.

- **Pause cross-network access** stops the relay connection and saves this choice across Desktop restarts.
  Phone credentials and any enabled same-network access remain.
- **Reconnect** resumes the saved enrollment. If enrollment was not confirmed, use this action to confirm it rather than repeatedly creating new enrollments.
- **Disconnect relay** revokes a v2 enrollment on the server before clearing local configuration.
  If the relay cannot confirm removal, the configuration is kept so the operation can be retried;
  **Forget on this Desktop only** then removes it locally without contacting the relay, and the old
  relay-side connection must be revoked in Relay Console. It does not delete Desktop business data or revoke individual phones.
- **Forget on this Desktop only** is also offered when Desktop cannot read its saved relay
  configuration, for example after the data folder moved to another Windows account.
  Phone credentials and same-network access remain.
- Older v1 enrollment can only be forgotten locally. Revoke its old Agent credential in Console as well.

## Manage paired phones

**Paired phones** lists authorized devices and their last use. It is not a real-time online list.

Select a phone's **Remove phone** action and confirm to revoke that phone's credential and current connections.
Other phones remain authorized, and Desktop conversations, tasks, and files remain.
The removed phone must pair again to restore access.

Changing the owner password from a browser session (including the dedicated Web host) signs out every
browser session and all paired phones; pair the phones again afterwards. A password change made in the
Desktop app keeps paired phones.

A phone credential grants owner-level access to this Desktop's tasks, conversations, and tools.
Only pair trusted phones you own, and protect the QR code and pairing link.

## Access from another computer's browser

Open **Other computer browsers**, enable WebUI in the Desktop app, and copy its access address to a desktop browser on another computer.
Sign in with the administrator account, or copy the one-time **QR-code login** link in this task.

Browser login codes and Mobile pairing codes have different purposes.
Browser links enter through `/qr-login?token=…`; Mobile uses a `nomifun://pair` link wrapping `/device-pair?token=…`.

The default LAN WebUI port is `25808` (usually `25809` in a development channel, or `25810` in multi-instance mode).
Use the actual address shown in the UI. Initial administrator passwords may be shown only once;
the account panel can change the password or username afterward. If cross-network access was
configured before WebUI, the browser task shows the password as already set; use the password
action in the account panel to choose one before signing in from a browser.

Only the trusted in-app client can initialize Desktop credentials. A loopback connection,
browser or phone token, or forged page Origin cannot initialize the owner's password.
The LAN and relay listeners provision an access password before they open.

The WebUI switch controls the LAN listener. Relay access has an independent loopback endpoint and enabled state;
pausing it does not disable WebUI. Desktop cannot execute requests while it is closed or the machine is asleep.

## Already using a dedicated Web host

An installation owner signed into `nomifun-web` from a desktop browser can create phone pairing codes,
read paired devices, and revoke each device independently from **Connect a phone**.

The pairing link uses the current site's address. The phone must reach it. If the page was opened through
localhost or 127.0.0.1, reopen it through a reachable LAN IP or deployment domain before generating a code.

A browser cannot manage Tauri listeners or relay processes. Configure a dedicated host's ports, TLS,
and process lifecycle through [Web Server Deployment](./web-server-deployment.md).
Authenticated `nomifun-web` retains its first-run setup flow. Complete owner setup before
opening a dedicated deployment to untrusted visitors.
Device credentials cannot issue or revoke credentials for other devices; use an owner account or the Desktop app for management.

## External tools

Select the needed MCP / REST capability domains under **External tools** and copy the generated configuration.
Issue the installation access token in the Desktop app. Its plaintext appears once and it can be revoked independently.
This token belongs to the installation and is separate from phone credentials and browser sessions;
it does not select or impersonate a companion.

## Troubleshooting

- **Cannot find the entry:** global **Connections & Devices** manages phones, browsers, and external tools.
  Companion **Channels & Robots** manages IM and physical robots.
- **The phone cannot reach a LAN address:** check network connectivity, the selected adapter, VPN routing, and the host firewall.
  localhost and 127.0.0.1 are reachable only on the host itself.
- **Relay online but the phone does not work:** check endpoint health, then Mobile's Desktop identity, protocol version, and device authorization.
  Relay status alone does not mean a phone is paired.
- **A pairing code expired or was used:** generate a new phone pairing code rather than redeeming the old code again.
- **Protocol mismatch after upgrading Desktop:** upgrade Mobile to a compatible version.
  Desktop's canonical Agent Session remains the authority for conversation history.
- **A phone was removed:** pair it again using a new Desktop code. Restarting the relay or removing another phone does not restore its authorization.
- **Relay removal could not be confirmed, or the saved relay configuration cannot be read** (for
  example after the data folder moved to another Windows account): select **Forget on this Desktop
  only** to clear the local state, then connect again and revoke the old connection in Relay Console.
- **The UI differs from the installed version:** force-refresh the desktop browser to load the UI served by the current installation.

Authenticated endpoints expose file, tool, and Agent execution capabilities. Use LAN HTTP on a trusted network;
use a correctly configured TLS relay or dedicated Web deployment across public networks.
Anyone able to reach the address can read its installation identifier and the protocol version used for
pairing checks. On the same network it also shows the computer name; through a relay it shows
"NomiFun Desktop" unless `NOMIFUN_DESKTOP_NAME` sets a name for every address.
Do not publish screenshots containing active pairing codes, installation tokens, or passwords.
