# Channels

A **channel** lets you operate a NomiFun agent from an external chat app —
Telegram, Lark / 飞书, DingTalk, WeChat — instead of sitting in front of
the desktop window. You enable a plugin, paste in its credentials,
authorize a chat user with a one-time code, and from then on messages
to your bot are dispatched to the agent and its replies come back into
the same thread.

Channels are useful when:

- you want to brief an agent from your phone or a group chat;
- you want a workspace-aware agent reachable from a team's existing IM;
- you want long-running tasks ([AutoWork](./autowork-requirements.md))
  to be kickable from outside the desktop without spinning up the WebUI.

> Each platform plugin is a Cargo feature on `nomifun-channel`
> (`telegram`, `lark`, `dingtalk`, `weixin`). The default NomiFun build
> ships with all of them on; if you build the backend yourself with a
> non-default feature set, the corresponding tab simply disappears.

![Channels settings overview](../images/channels-01-overview.png)

## Where to find it

Open the Nomi page (`/nomi`), select a companion, and switch to the
**Remote** tab (`/nomi?companion=<id>&tab=remote`). That tab lists the
remote connectors for the selected companion — built-in (Telegram,
Lark, DingTalk, WeChat, WeCom, Slack, Discord, extensions). For each
plugin you'll see:

- a status pill (`stopped` / `connected`),
- the bot username once connected,
- the number of currently authorised users,
- a per-channel **default agent** + **default model** selector.

Slack / Discord / WeCom appear as built-in placeholders today — the
backend wiring is feature-gated and still being built out for those
two; Telegram / Lark / DingTalk / WeChat are the ones you can run
today.

## How a channel works

```
external IM ──▶ plugin (long-poll / WebSocket)
                    │
                    ▼
            ChannelManager  ◀─▶  PairingService
                    │
                    ▼
              SessionManager  ──▶  agent / conversation
```

- **Plugin** owns the platform-specific connection (Telegram long-poll
  with exponential backoff, Lark / DingTalk WebSocket, WeChat QR-code
  login over SSE).
- **PairingService** turns "I'm John on Telegram, let me in" into a
  6-digit code that you approve from the desktop UI.
- **SessionManager** maps `(platform_user, chat_id)` to an agent
  conversation, so each external chat is a stable session and follow-up
  messages land in the same agent.
- **Message loop** plumbs incoming messages into the agent stream and
  the agent's replies back out as edits to the same IM message
  (everything except WeChat supports message editing — WeChat falls
  back to sending follow-up replies).

## Setting up each platform

### Telegram

1. Talk to [`@BotFather`](https://t.me/BotFather) and create a bot.
   Save the token (looks like `123456:ABC-DEF…`).
2. In **Nomi → Remote → Telegram**, paste the token.
3. Click **Test** — the backend calls `getMe` and shows the bot
   username on success.
4. Click **Enable**. The plugin starts long-polling
   (25 s timeout, exponential backoff up to 10 reconnects).

To pair a Telegram user with the desktop, the user messages your bot;
the bot replies with a 6-digit code (10-minute TTL). Paste / type the
code into **Nomi → Remote → Pending pairings** on the desktop
and click **Approve**. From then on that Telegram user can chat with
the agent.

### Lark / Feishu

1. Create a custom app in the Lark developer console with the events
   you need (text message, card action, bot menu).
2. Copy the **App ID**, **App Secret**, and (optional) **Encrypt key /
   Verification token**.
3. Paste them into the Lark form in the Channels tab and click
   **Enable**.

The Lark plugin connects via Lark's WebSocket long-connection (no
public webhook needed), with a 60-second event-dedup cleanup loop and
fragment reassembly. Replies are sent as **interactive cards** because
Lark's API only supports editing card messages.

### DingTalk

1. Create an internal app in DingTalk Developer Backstage with **Stream
   Mode** enabled.
2. Copy the **Client ID** and **Client Secret** into the DingTalk form
   and enable.

The DingTalk plugin opens a WebSocket using the standard DingTalk
stream-mode handshake; pairing flow is identical to Telegram.

### WeChat

1. WeChat is QR-code login. Click **Enable** on the WeChat plugin —
   the backend opens an SSE stream (`POST /api/channel/weixin/login/start`)
   that pushes QR-code refresh events.
2. Scan the QR with the WeChat app, confirm the login, and the plugin
   transitions to `connected`.

WeChat does **not** support message editing — replies are delivered as
new messages in the same chat instead of in-place edits.

## Pairing and authorising users

A pairing request comes in two ways:

1. The platform user messages the bot for the first time (Telegram
   /Lark / DingTalk). The plugin auto-creates a pending request and
   replies to the user with the code.
2. You can approve / reject the pending request from
   **Nomi → Remote → Pending pairings** or programmatically
   via `POST /api/channel/pairings/approve` and
   `POST /api/channel/pairings/reject`.

Approved users are listed in **Authorised users**, with `last active`.
You can revoke at any time (`POST /api/channel/users/revoke`); the
service also cleans up that user's open sessions so the next message
re-pairs from scratch.

![Pairing approval](../images/channels-02-pairing.png)

## Channel Agent integration

Channel messaging uses the same Agent and Conversation runtime as the desktop;
it is not a separate Agent type or a switchable mode. When a bot is bound to a
desktop companion and uses Nomi, inbound turns are routed into that companion's
single persistent Conversation. The channel Agent context supplies the
companion persona and memory context and wires the **platform Gateway** tools,
so the desktop and IM surfaces share one transcript and one runtime identity.
That authority is never stored in Conversation metadata: after validating the
local instance owner, the Agent factory injects a scoped, expiring claim signed
by a process-private root.

The obsolete per-platform on/off preference has been removed; there is no
legacy off-state. A bot bound to a public agent follows the public-agent policy
instead: it uses an isolated per-chat Conversation and never receives the
platform Gateway claim; the factory's public-agent clamp fails closed.

What the gateway tools (all prefixed `nomi_*`, 32 of them today) let the
remote agent do on your behalf:

- **Conversations** — list every conversation with its runtime state,
  inspect one (status plus the latest messages, including an in-flight
  streaming reply), send a message or task prompt into any
  conversation, create new ones, update or delete old ones
  (`nomi_list_conversations`, `nomi_conversation_status`,
  `nomi_send_to_conversation`, `nomi_create_conversation`,
  `nomi_update_conversation`, `nomi_delete_conversation`).
- **Scheduled tasks** — list / create / update / delete cron jobs
  (`nomi_cron_list`, `nomi_cron_create`, `nomi_cron_update`,
  `nomi_cron_delete`).
- **Long-term memory** — read and write the companion's global memory bank
  (`nomi_memory_list`, `nomi_memory_save`, `nomi_memory_update`,
  `nomi_memory_delete`).
- **Requirements** — browse and manage the requirements platform
  (`nomi_requirement_list`, `nomi_requirement_create`,
  `nomi_requirement_update`, `nomi_requirement_delete`).
- **Terminals & supervision** — list terminal sessions, create new ones
  (optionally binding knowledge bases via `knowledge_base_ids`), and
  read / toggle a terminal's AutoWork binding and IDMM supervision
  (`nomi_list_terminals`, `nomi_create_terminal`, `nomi_get_autowork`,
  `nomi_set_autowork`, `nomi_get_idmm`, `nomi_set_idmm`).
- **Knowledge bases** — browse bases and bindings, rebind a
  conversation / terminal / companion, create a new base, write markdown
  files into one, trigger the AI digest, or fetch a URL as markdown —
  so the companion can deposit knowledge on its own
  (`nomi_knowledge_list_bases`, `nomi_knowledge_get_binding`,
  `nomi_knowledge_set_binding`, `nomi_knowledge_create_base`,
  `nomi_knowledge_write_file`, `nomi_knowledge_autogen`,
  `nomi_knowledge_fetch_url`). `nomi_knowledge_create_base` with
  `urls` fetches in the background — the call returns immediately, so
  don't create the base a second time while waiting; the base's
  description appearing means the fetch + digest pipeline is done.
- **Providers** — list the configured LLM providers
  (`nomi_list_providers`).

So *"move my daily-report cron to 9 am and tell me what's running
right now"* is a single Lark message.

**Choosing which companion greets the channel.** With [multiple companions](./companions.md),
bots are bound to companions **per channel row**: each row of
`channel_plugins` is one bot (the same platform can host several —
e.g. one Feishu in-house app per companion), its `companion_id` decides which companion
answers, and the `UNIQUE(type, bot_key)` constraint structurally
guarantees **one bot is never bound to two companions** (bot identity: Feishu
`app_id`, the Telegram bot id, DingTalk `client_id`, …). Binding or
unbinding calls `POST /api/channel/settings/companion` with a `plugin_id`,
which persists the row and resets **that channel's** active sessions in
one step — the next inbound message is greeted by the new companion's persona,
model, and knowledge mounts (the conversation carries `extra.companionId`).
Connecting a bot from a companion's **Remote** tab creates the channel row and
binds it to that companion in one go. A row without a companion binding falls back
to the per-platform preference `channels.<platform>.companionId`. There is no
implicit default-companion fallback: if neither binding resolves to a live
companion, the channel remains unbound and receives no companion persona. A
binding change resets the affected sessions so the next turn resolves the new
owner cleanly.

**Agent and model resolution.** Channel connection forms configure transport
credentials and owner bindings; they do not introduce another Agent or model
picker. A companion-bound Nomi bot uses the companion profile's model as the
authoritative value, with a provisioned `channels.<platform>.defaultModel` only
as fallback. A public-agent bot uses that public agent's resolved model under
the public capability clamp. An unbound channel defaults to Nomi; deployments
that explicitly provision `channels.<platform>.agent` can select another engine,
and ACP continues to consume its provisioned backend/model configuration.
After changing a platform-level provisioning preference, calling
`POST /api/channel/settings/sync` clears that platform's sessions so the next
turn resolves the new configuration.

## What works from the IM side

The platform-agnostic abstraction (`UnifiedIncomingMessage`,
`UnifiedOutgoingMessage`, `UnifiedAction`) covers:

- **Plain text** — both directions.
- **Edited streaming responses** — incremental updates from the agent
  are edited into the in-flight bot message (not on WeChat).
- **Action buttons** — confirmation prompts, retry actions, etc.,
  rendered as inline keyboards (Telegram), interactive-card buttons
  (Lark), or platform equivalents.
- **Bot mention / require-mention** — group chats can be configured
  to only respond when the bot is `@`-mentioned.

What you don't get from the IM side (yet):

- spawning teams (use the desktop / web UI for that);
- file uploads beyond what the platform plugin natively understands;
- per-user workspace selection — the agent's workspace is the one set
  on the conversation it routed to.

## Routes & API

| What                            | Where                                                   |
| ------------------------------- | ------------------------------------------------------- |
| Channels UI                     | `/nomi?companion=<id>&tab=remote`                       |
| List plugins / status           | `GET /api/channel/plugins`                              |
| Enable / disable                | `POST /api/channel/plugins/enable`, `…/disable`         |
| Test credentials                | `POST /api/channel/plugins/test`                        |
| Pending pairings                | `GET /api/channel/pairings`                             |
| Approve / reject pairing        | `POST /api/channel/pairings/approve`, `…/reject`        |
| Authorised users                | `GET /api/channel/users`, `POST .../users/revoke`       |
| Active sessions                 | `GET /api/channel/sessions`                             |
| Sync (clear sessions on change) | `POST /api/channel/settings/sync`                       |
| Bind channel companion          | `POST /api/channel/settings/companion`                  |
| WeChat QR login SSE             | `POST /api/channel/weixin/login/start`                  |

## Notes

- Plugin lifecycle is a state machine —
  `Created → Initializing → Ready → Starting → Running → Stopping → Stopped`,
  with any step able to transition to `Error`. The status pill in the
  UI is this enum.
- A revoked user's session is torn down before the user row is
  deleted. The next message from that platform user will trigger a new
  pairing code.
- Pairing codes are 6 digits, generated with `getrandom`, with a
  10-minute TTL. The pairing service runs a periodic sweep that
  expires pending codes whose TTL has passed.
- WeChat is feature-gated separately because its dependency tree is
  heavier (QR / login / auth flow). If you build with
  `--no-default-features`, you'll see the placeholder card but no
  enable button.

## Related

- [Companions](./companions.md) — multi-companion management, shared memory, and the
  per-companion knowledge bindings that ride on channel conversations.
- [AutoWork & Requirements](./autowork-requirements.md) — file a
  requirement from a chat, get notified when it lands via a webhook to
  Lark / HTTP / Slack (configured at **需求平台 → 扩展能力 → 通知**).
- [Web Server Deployment](./web-server-deployment.md) — exposes the
  same channels when you self-host the backend on a server.
