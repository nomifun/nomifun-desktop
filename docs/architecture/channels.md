# Messaging channels and group admission

Channels connect external messages to the current companion or customer-service owner without creating another Agent runtime or Session authority. See the [channel guide](../guides/channels.md) for user operations. This page records adapter-independent contracts; code declares platform capabilities.

## Per-bot policy

Each bot independently stores `channel_plugins.group_access_mode`:

| Value | Group admission |
| --- | --- |
| `all_members` | Members with stable sender IDs and a structured mention of this bot may enter the group path |
| `allowlist` | Use this bot's approved identities; other valid mentions create or reuse Pending requests |
| `disabled` | Ignore every group message, including the owner's and approved members' |

Companion bots default to `allowlist`; customer-service bots default to `all_members`. Writes reject unknown values. Bots on the same platform do not share policy, Pending requests, approved identities or Sessions. Group policy grants no direct-message authorization; `disabled` leaves direct messages and connectivity intact.

Processing filters self/replayed messages before classifying direct versus group. Group ingress requires reliable sender, bot, chat kind and structured mention before checking this bot's policy, identity and canonical Session admission. Plain-text `@bot`, quotations and mentions of another bot do not qualify. Missing metadata fails closed before creating Pending requests, Sessions or Agent calls. Unreliable adapters hide the group setting without relaxing backend checks.

## Identities and Sessions

`allowlist` reuses per-bot pairing and approved users. Pending group requests appear only to the owner; pairing codes are not posted into groups. Open groups use hidden `authorization_kind=auto_group` identities, excluded from approved users and unusable for ordinary direct messages or allowlists. An open-group event cannot downgrade an approved identity.

For companions, `auto_group` has a model-only ceiling at the common Agent factory: reply to the current group message without owner tools, private knowledge, memory, schedules, cross-session access, terminals, browsers, computer-use or custom MCP. Ingress admission grants no installation-owner authority.

Group Sessions are isolated by bot and group, separate from owner desktop conversations, member direct messages, other groups and other bots. Channels resolve Sessions through the current owner and typed binding. A Session without an authority binding is a conflict, never a reason to select an old record and rebind automatically. Creation, reset and cancellation call the canonical owner.

Customer-service messages remain in their own domain after group admission. Their current owner and frozen Agent contract determine Session and tool boundaries; historical one-shot implementation notes are no runtime requirement.

## Policy changes

`POST /api/channel/settings/group-access` accepts a bot ID and policy without credentials or reconnecting the bot. A per-bot write fence protects the transaction replacing policy and clearing group/unknown-scope Sessions and queued work while retaining direct Sessions. Ingress and delivery hold read fences through durable Turn handoff. Once an update returns, new admission cannot use the old policy; durably admitted work settles through its original canonical receipt.

## Sources and validation

| Boundary | Source |
| --- | --- |
| Adapters and types | [plugin.rs](../../crates/backend/nomifun-channel/src/plugin.rs), [types.rs](../../crates/backend/nomifun-channel/src/types.rs) |
| Ingress, identity and policy | [message_loop.rs](../../crates/backend/nomifun-channel/src/message_loop.rs), [pairing.rs](../../crates/backend/nomifun-channel/src/pairing.rs), [channel_settings.rs](../../crates/backend/nomifun-channel/src/channel_settings.rs) |
| Linearization fence | [group_policy.rs](../../crates/backend/nomifun-channel/src/group_policy.rs) |
| Session seam | [session_port.rs](../../crates/backend/nomifun-channel/src/session_port.rs), [app channel](../../crates/backend/nomifun-app/src/channel.rs) |

Changes cover all three policies, structured mentions, missing metadata, bot isolation, open-group capability ceilings, unaffected direct messages and update/delivery races. Adapter implementations and tests own platform support, without a second dated capability matrix.
