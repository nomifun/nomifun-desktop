# NomiFun Product Ecosystem Architecture

This document explains how NomiFun Desktop, Mobile, Xiaozhi Yuntai, hosted
applications, and companion channels form one product system. It describes product and
trust boundaries; endpoint details remain in the linked operator guides.

Simplified Chinese: [product-ecosystem.zh.md](product-ecosystem.zh.md)

## The four open-source projects

| Product | Primary responsibility | Documentation |
|---|---|---|
| [NomiFun Desktop](https://github.com/nomifun/nomifun-desktop) | Local source of truth and execution hub for data, models, Agents, tasks, tools, Skills, knowledge, companions, and hosted applications | [Architecture overview](overview.md) · [WebUI remote access](../guides/webui-remote-access.md) · [Xiaozhi integration](../guides/xiaozhi-robot.md) |
| [NomiFun Mobile](https://github.com/nomifun/nomifun-mobile) | Android / iOS / H5 interaction surface that directly uses an authorized Desktop instance | [Mobile README](https://github.com/nomifun/nomifun-mobile#readme) |
| [NomiFun Xiaozhi Yuntai](https://github.com/nomifun/nomifun-xiaozhi-yuntai) | ESP32-S3 voice, display, motion, and device-tool endpoint for a Desktop companion | [Firmware README](https://github.com/nomifun/nomifun-xiaozhi-yuntai#readme) · [Desktop integration](../guides/xiaozhi-robot.md) |
| [NomiFun Net Infra](https://github.com/nomifun/nomifun-net-infra) | Optional self-hosted NomiRelay transport for reaching Desktop and other services behind NAT across networks | [Product page](https://www.nomifun.com/products/net-infra/) · [Portal guide](https://www.nomifun.com/docs/guides/net-infra/) · [Relay integration](https://github.com/nomifun/nomifun-net-infra/tree/main/docs/integration) |

## Hub-and-surface model

```text
                       trusted LAN / authenticated channels

  NomiFun Mobile  ───────────────┐
  Xiaozhi Yuntai ────────────────┤
  hosted applications ───────────────┼──▶ NomiFun Desktop
  Companion IM channels ─────────┘       │
                                         ├─ data and conversation authority
                                         ├─ models and Agent runtimes
                                         ├─ requirements, AutoWork and IDMM
                                         ├─ tools, Skills, MCP and REST
                                         ├─ knowledge and working context
                                         └─ companion identity and memory
```

Desktop is the authority, not merely another UI. It owns the durable data set,
resolves model and Agent configuration, enforces tool and knowledge scope, and
executes work. The other surfaces do not each create a second configuration or
copy the primary data set.

### Mobile communicates directly with Desktop

Users begin at Desktop's global **Connections & Devices → Connect a phone**,
then scan or paste the pairing link through Mobile's **Add Desktop** entry.
Mobile's **Connections & Devices** page shows the selected Desktop identity,
connection status, and repair actions.

On a LAN, Mobile connects to the authenticated listener inside Desktop. Pairing
uses a short-lived, one-time phone pairing code that issues an independent
device credential for that phone. Expired or redeemed codes cannot be reused;
these codes differ from one-time login codes for other computer browsers.
After authentication, realtime interaction stays between
the phone and the selected Desktop instance; it does not pass through a NomiFun
cloud relay. The phone is therefore a control and presentation surface while
Desktop remains the data, credential, model, Agent, task, and tool authority.

This design has three practical advantages:

- model credentials and the durable workspace do not need to be copied to a
  mobile cloud account or stored independently on the phone;
- a change to a companion, model, Skill, knowledge base, requirement, or task is
  immediately the same state seen by Desktop and Mobile;
- the Desktop owner explicitly opens and closes the LAN boundary, and can rely
  on one-time pairing instead of publishing a long-lived bearer URL; each
  phone's authorization can be revoked independently without affecting others.

There is no built-in TLS on the LAN listener. Use it only on a trusted LAN or a
trusted private VPN, keep the operating-system firewall enabled, and never
publish the listener directly to the public Internet. See
[WebUI Remote Access](../guides/webui-remote-access.md) for the complete threat
model and operating steps.

### Net Infra is an optional transport layer, not another backend

When Mobile cannot reach Desktop over a trusted LAN, an operator can self-host
NomiRelay. The default user flow starts by creating a v2 enrollment code under
**Connect Desktop** (`/desktop-connect`) in Relay Console, then pasting it into Desktop's
**Connections & Devices → Connect a phone → Across networks (relay)**.
Native Desktop manages `nfagent` installation, startup, pause, reconnect, and
disconnection. Its independent loopback endpoint carries relay access without
requiring users to run an Agent manually or enable LAN WebUI.

After confirming relay and business-endpoint availability, Desktop generates
the phone pairing code. Mobile connects only to the relay business endpoint;
it never receives relay-admin
credentials or calls the console API. The relay carries HTTP/WebSocket bytes and
enforces network policy, while Desktop remains the application-data and
execution authority. Pausing or disconnecting a relay is separate from revoking
an individual phone. Running `nfagent` manually remains an advanced CLI deployment
option, rather than the default Desktop user flow.
Exact deployment and public-network validation belong to
the [NomiRelay integration docs](https://github.com/nomifun/nomifun-net-infra/tree/main/docs/integration)
and the [Mobile relay guide](https://github.com/nomifun/nomifun-mobile/blob/main/docs/RELAY-INTEGRATION.md).

### Xiaozhi is a hardware surface for a companion

Xiaozhi Yuntai adds microphones, speakers, a display, servos, and device-side
tools. Desktop supplies the companion identity, memory, knowledge, models, ASR,
TTS, sessions, and tool coordination. Binding the device to a companion keeps
the physical embodiment in the same governed runtime as desktop and mobile
interaction instead of creating a disconnected robot account.

### Application hosting runs ordinary local programs

Users write an application through ordinary conversations and coding tools,
then import source or compiled folders/ZIPs. Desktop runs commands in the
current managed directory and owns one foreground process tree. Applications
keep their own data and business architecture; optional platform access uses
automatic local trust, and MCP tools require explicit Agent/session selection.
See [application hosting architecture](app-hosting.md).

## Capabilities across entry points

### One capability graph, many entry points

Desktop companions, built-in Agents, supported Agent CLIs, Mobile, hardware,
hosted applications, MCP/REST callers, and IM channels all converge on Desktop-managed
capabilities. Models, Skills, knowledge, requirements, tasks, and tools are
configured once and reused rather than being reimplemented per surface.

### Decision, implementation, and recovery can be separated

The requirements platform and AutoWork decide what work to claim and advance;
Agent collaboration can divide implementation into auditable steps; IDMM adds a
separate liveness and recovery layer for stalls and recoverable provider faults.
These responsibilities can cooperate without collapsing planning, execution,
and resilience into one opaque model call.

### Hardware is part of the Agent capability boundary

Voice, display, motion, and device-side tools are treated as another governed
interaction surface. This allows a companion to retain the same identity and
memory while moving between desktop UI, mobile control, IM channels, and a
physical device.

### Knowledge is governed Agent working context

Knowledge bases provide scoped, retrievable, durable working context for Agents
and long-running tasks. They are not a claim to record or expose a model's
private hidden chain of thought. Explicit artifacts, sources, decisions, and
approved write-back remain inspectable and transferable.

### Companion assets can evolve and move together

Each companion has an independent persona, model, memory, knowledge bindings,
learning position, and Skill evolution process. Memory, companion configuration,
and optionally Skills can be exported and imported as migration packages. This
makes evolution a user-controlled asset rather than a profile trapped in one
hosted account. See [Companions](../guides/companions.md).

### Channels are gateways, not separate companion identities

The companion IM gateway connects external chat channels to the same
authenticated companion and Agent runtime. It does not silently create a new
identity, memory silo, or ungoverned capability set for each channel.

## Related guides

- [Architecture overview](overview.md)
- [WebUI remote access](../guides/webui-remote-access.md)
- [Xiaozhi robot integration](../guides/xiaozhi-robot.md)
- [Companions](../guides/companions.md)
- [AutoWork and Requirements](../guides/autowork-requirements.md)
- [Intelligent Decision (IDMM)](../guides/intelligent-decision.md)
- [Computer Use and Browser Use](../guides/computer-browser-use.md)
