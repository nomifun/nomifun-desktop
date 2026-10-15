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
**Channels & Robots** tab (`/nomi?companion=<id>&tab=remote`). That tab lists the
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

## Private chat and group access

Group access is stored on each bot row as `group_access_mode`; it is not a
platform-wide switch. This matters when, for example, two Feishu apps on the
same NomiFun instance are bound to different companions.

| Setting | Wire value | Group-chat behaviour |
| ------- | ---------- | -------------------- |
| **All group members** | `all_members` | Any member may use a structured `@bot` mention without pairing. This is group-only admission and does **not** authorise that person for direct messages. |
| **Selected members** | `allowlist` | Only this bot's Authorised users may use it. A structured mention from somebody else creates or reuses a Pending request for the owner to approve. |
| **Disable group chat** | `disabled` | Ignore every group message, including messages from the owner and already-authorised users. Direct messages are unchanged. |

New companion bots default to `allowlist`; customer-service bots default to `all_members`. Owners can change each bot's mode.

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

Connect each bot from the intended companion's **Channels & Robots** tab, or bind it from **Customer Service** when it serves customers. One bot has one owner binding. Changing the binding resets the affected active channel sessions, so the next message resolves the new companion or service. If no saved binding resolves to a live companion, the channel remains unbound.

The connection form owns transport credentials and the binding. Configure the Agent and model on the companion or customer-service page. Available tools depend on that saved Agent, its resources and channel access policy; do not assume every desktop capability is available in IM.

Group chats use a separate Session for each bot and group. Automatically admitted guests receive model-only replies and do not gain the owner's private context or management tools. Private access, group admission and bot enable/disable remain separate controls.

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

## Related

- [Channel architecture](../architecture/channels.md) —
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
