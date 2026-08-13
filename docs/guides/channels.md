# Channels

A **channel** lets you operate a NomiFun agent from an external chat app —
Telegram, Feishu (Lark), DingTalk, WeChat, WeCom, QQ Bot, and others — instead
of sitting in front of the desktop window. You connect one or more bots to a
companion or customer-service agent, then decide who may use each bot in
private and group chats. Private chats keep the existing pairing flow; group
access is a simple per-bot policy.

Channels are useful when:

- you want to brief an agent from your phone or a group chat;
- you want a workspace-aware agent reachable from a team's existing IM;
- you want long-running tasks ([AutoWork](./autowork-requirements.md))
  to be kickable from outside the desktop without spinning up the WebUI.

> Built-in adapters are Cargo features on `nomifun-channel`: `telegram`,
> `lark`, `dingtalk`, `weixin`, `wecom`, `discord`, `matrix`, `mattermost`,
> `slack`, `twitch`, `nostr`, and `qqbot`. The default `nomifun-app` build
> enables all of them. A custom build may omit adapters, in which case those
> connectors are unavailable. The user-facing name is **Feishu (Lark)**;
> its protocol and configuration key remains `lark` for compatibility.

![Channels settings overview](../images/channels-01-overview.png)

## Where to find it

Open the Nomi page (`/nomi`), select a companion, and switch to the
**Remote** tab (`/nomi?companion=<id>&tab=remote`). That tab lists the
remote connectors for the selected companion — including Telegram, Feishu
(Lark), DingTalk, WeChat, WeCom, QQ Bot, Slack, Discord, Matrix, Mattermost,
Twitch, Nostr, and extensions. For each connected bot you'll see:

- a status pill (`stopped` / `connected`),
- the bot username once connected,
- the number of currently authorised users;
- transport credentials and the companion or customer-service binding;
- on adapters with reliable group metadata, a per-bot **Group access**
  selector.

WeCom and QQ Bot are runnable built-in adapters, not placeholder cards. WeCom
uses the Intelligent Bot long-connection protocol; QQ Bot uses the official
Gateway WebSocket and REST API.

## How a channel works

```
external IM ──▶ plugin (long-poll / WebSocket)
                    │
                    ▼
          chat scope + structured @ gate
                    │
                    ▼
            ChannelManager  ◀─▶  PairingService
                    │
                    ▼
              SessionManager  ──▶  agent / conversation
```

- **Plugin** owns the platform-specific connection (Telegram long-poll;
  Feishu, DingTalk, WeCom, and QQ Bot WebSocket; WeChat QR-code login).
- **Scope gate** classifies the event as direct or group, verifies a real
  platform `@` mention for group messages, and applies that bot's group access
  mode before pairing or session creation.
- **PairingService** turns a first contact into a request that you approve from
  the desktop UI. Direct messages keep the existing 6-digit-code flow; group
  allowlists reuse the same Pending and Authorised user records.
- **SessionManager** keeps follow-up messages stable while isolating each group
  from private chats, the owner's desktop transcript, and other groups.
- **Message loop** plumbs incoming messages into the agent stream and
  the agent's replies back out. It edits the in-flight message where the
  platform supports editing; adapters such as WeChat and WeCom fall back to
  follow-up replies.

## Private chat and group access

Group access is stored on each bot row as `group_access_mode`; it is not a
platform-wide switch. This matters when, for example, two Feishu apps on the
same NomiFun instance are bound to different companions.

| Setting | Wire value | Group-chat behaviour |
| ------- | ---------- | -------------------- |
| **All group members** | `all_members` | Any member may use a structured `@bot` mention without pairing. This is group-only admission and does **not** authorise that person for direct messages. |
| **Selected members** | `allowlist` | Only this bot's Authorised users may use it. A structured mention from somebody else creates or reuses a Pending request for the owner to approve. |
| **Disable group chat** | `disabled` | Ignore every group message, including messages from the owner and already-authorised users. Direct messages are unchanged. |

`allowlist` is the safe default for new companion bots and the migration value
for existing non-customer-service bots. Customer-service bots default to
`all_members`, and existing customer-service rows are backfilled to that value,
so an upgrade does not silently stop serving their groups. Owners can change
the mode afterwards.

Every accepted group message must contain the platform's **structured mention
of this bot**. Text that merely looks like `@bot`, quoted content, and ordinary
unmentioned group traffic do not count. Such messages are ignored before a
pairing request or session can be created. Missing or unknown chat scope,
sender identity, or mention data also fails closed.

Group turns use a dedicated session scoped to that bot and group. They never
enter the owner's/private Conversation, and two groups never share a group
session. Selecting **All group members** grants no private-chat permission: if
the same person later sends the companion bot a direct message, normal pairing
still applies. For stable routing, NomiFun records such a member internally as
an `auto_group` identity, but hides it from Authorised users and fences its Nomi
runtime to a model-only reply. It receives no cron, AutoWork, requirement,
memory, cross-conversation, terminal, browser, computer-use, custom MCP, or
platform Gateway tools. An explicitly `approved` identity admitted by
`allowlist` retains the existing authorised channel capabilities.

The selector is shown only when an adapter can reliably report both group
scope and structured mentions. The initial supported set is Feishu, DingTalk,
WeCom, QQ Bot, Discord, Slack, and Mattermost. Matrix currently cannot classify
direct and group rooms consistently, while Telegram has not yet normalised
structured message-entity mentions, so neither shows the selector. Direct-only
adapters likewise omit it.

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

### Feishu (Lark)

1. Create a custom app in the Feishu/Lark developer console with the events
   you need (text message, card action, bot menu).
2. Copy the **App ID**, **App Secret**, and (optional) **Encrypt key /
   Verification token**.
3. Paste them into the **Feishu (Lark)** form in the Channels tab and click
   **Enable**.

The Feishu adapter connects via the platform's WebSocket long-connection (no
public webhook needed), with a 60-second event-dedup cleanup loop and
fragment reassembly. Replies are sent as **interactive cards** because
the API only supports editing card messages. The internal protocol key remains
`lark`; existing deployment settings do not need to be renamed.

### DingTalk

1. Create an internal app in DingTalk Developer Backstage with **Stream
   Mode** enabled.
2. Copy the **Client ID** and **Client Secret** into the DingTalk form
   and enable.

The DingTalk plugin opens a WebSocket using the standard DingTalk
stream-mode handshake; pairing flow is identical to Telegram.

### WeCom

1. In WeCom, create an **Intelligent Bot** and select **Long Connection
   (WebSocket)** mode.
2. Copy its **Bot ID** and **Secret** into the WeCom form.
3. Test the credentials, enable the bot, and keep NomiFun running to maintain
   the outbound WebSocket.

This mode does not require a callback URL, callback domain, or public IP.

### QQ Bot

1. Create a bot on the [QQ Open Platform](https://q.qq.com/) and copy its
   **AppID** and **ClientSecret**.
2. In the platform console, apply for the `GROUP_AND_C2C` intent; without it,
   QQ cannot deliver group or C2C messages to the bot.
3. Paste the credentials into the **QQ Bot** form, test, and enable it.

The adapter receives events over the official Gateway WebSocket and sends
replies through the official REST API.

### WeChat

1. WeChat is QR-code login. Click **Enable** on the WeChat plugin —
   `POST /api/channel/weixin/login/start` starts the login, and the backend
   publishes QR-code refresh events to the desktop over the existing app
   WebSocket.
2. Scan the QR with the WeChat app, confirm the login, and the plugin
   transitions to `connected`.

WeChat does **not** support message editing — replies are delivered as
new messages in the same chat instead of in-place edits.

## Pairing and authorising users

For a companion-bound bot, a first **direct message** from an unknown user
creates a Pending request and returns a 6-digit code with a 10-minute TTL. The
owner approves or rejects it in **Nomi → Remote → Pending pairings**, or through
`POST /api/channel/pairings/approve` and
`POST /api/channel/pairings/reject`.

In a group using **Selected members** (`allowlist`), an unknown member's valid
structured mention creates or reuses the same per-bot Pending record. NomiFun
does not post the pairing code publicly into the group; the owner reviews the
request in the existing Pending list. Approved identities appear in that bot's
**Authorised users** list, so no separate member directory or group ACL needs
to be maintained.

**All group members** does not create pairing requests. It creates or reuses a
hidden, per-bot `auto_group` identity only to keep group routing stable; that
identity is not added to Authorised users and cannot authorise a direct message.
Customer-service-bound bots keep their existing direct-message service
semantics instead of being forced into the companion pairing flow.

You can revoke an approved user at any time
(`POST /api/channel/users/revoke`). The service cleans up that user's open
sessions. Their next companion-bot direct message must pair again; their group
access then follows the bot's current `group_access_mode`.

![Pairing approval](../images/channels-02-pairing.png)

## Channel Agent integration

Channel messaging uses the same Agent and Conversation runtime as the desktop;
it is not a separate Agent type or a switchable mode. For a companion-bound
Nomi bot, an authorised direct turn keeps the existing private Conversation
behaviour. An admitted group turn instead uses a dedicated group-scoped
session. An explicitly approved `allowlist` member keeps the existing
authorised channel capabilities. An automatically admitted but unapproved
`all_members` guest gets model-only replies without loading the owner's private
companion context, and no group turn is appended to the owner's or any member's
private transcript.

Conversation metadata never stores owner authority. After validating the local
instance owner, the Agent factory injects any platform Gateway capability as a
scoped, expiring claim signed by a process-private root. Group admission — even
under `all_members` — is not owner authorisation and never promotes the sender
to private-message access. An `auto_group` Nomi runtime is forcibly model-only;
an explicitly `approved` allowlist user keeps the capabilities already provided
to authorised channel users.

The bot lifecycle enable/disable switch and `group_access_mode` are separate:
disabling the bot stops its connection, while `disabled` as a group mode only
turns off group processing. A bot bound to a customer-service agent still runs
the group gate first. Admitted messages remain entirely in the
customer-service domain (no companion Conversation) and are answered by a
disposable one-shot engine session whose tool table is fixed at construction
to three read-only tools (knowledge search, knowledge read, and customer-service
note search). It never receives the platform Gateway claim. Direct messages
retain the existing customer-service auto-service semantics.

When an owner-authorised Nomi channel context receives the gateway tools (all
prefixed `nomi_*`, 32 of them today), they let the remote agent do the
following on your behalf:

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
right now"* can be a single Feishu message from an appropriately authorised
context.

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
as fallback. A customer-service-bound bot uses that agent's own configured
provider/model from the customer-service console. An unbound channel defaults to Nomi; deployments
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
  (Feishu), or platform equivalents.
- **Safe group access** — supported group-aware adapters always require a
  structured mention of the bot, then apply `all_members`, `allowlist`, or
  `disabled`. This mention requirement is not an optional toggle.
- **Group isolation** — admitted group turns continue in that bot-and-group's
  session without sharing a private or different group's transcript.

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
| Group access                    | `POST /api/channel/settings/group-access`               |
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
- A revoked user's session is torn down before the user row is deleted. Their
  next companion-bot direct message triggers pairing again; group messages
  follow the bot's current group mode.
- Changing group access atomically updates the mode and retires group/unknown
  sessions plus their pending queue while preserving direct sessions. New
  messages use the new mode after the API returns; a turn that already reached
  durable admission may finish its one reply.
- Pairing codes are 6 digits, generated with `getrandom`, with a
  10-minute TTL. The pairing service runs a periodic sweep that
  expires pending codes whose TTL has passed.
- Adapters are feature-gated. A custom `--no-default-features` build must
  explicitly enable every connector it intends to expose.

## Related

- [Channel group access contract](../specs/2026-08-13-channel-group-access.zh.md) —
  the normative per-bot policy, identity ceiling, migration, and adapter
  capability matrix (Chinese).
- [Companions](./companions.md) — multi-companion management, per-companion
  memory, and the
  per-companion knowledge bindings that ride on channel conversations.
- [AutoWork & Requirements](./autowork-requirements.md) — file a
  requirement from a chat, get notified when it lands via a webhook to
  Feishu / HTTP / Slack (configured at **需求平台 → 扩展能力 → 通知**).
- [Web Server Deployment](./web-server-deployment.md) — exposes the
  same channels when you self-host the backend on a server.
