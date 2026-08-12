# NomiFun Product Ecosystem Architecture

This document explains how NomiFun Desktop, Mobile, Xiaozhi Yuntai, Agent Mini
Apps, and companion channels form one product system. It describes product and
trust boundaries; endpoint details remain in the linked operator guides.

Simplified Chinese: [product-ecosystem.zh.md](product-ecosystem.zh.md)

## The three open-source products

| Product | Primary responsibility | Documentation |
|---|---|---|
| [NomiFun Desktop](https://github.com/nomifun/nomifun-desktop) | Local source of truth and execution hub for data, models, Agents, tasks, tools, Skills, knowledge, companions, and Mini Apps | [Architecture overview](overview.md) · [WebUI remote access](../guides/webui-remote-access.md) · [Xiaozhi integration](../guides/xiaozhi-robot.md) |
| [NomiFun Mobile](https://github.com/nomifun/nomifun-mobile) | Android / iOS / H5 interaction surface that directly uses an authorized Desktop instance | [Mobile README](https://github.com/nomifun/nomifun-mobile#readme) |
| [NomiFun Xiaozhi Yuntai](https://github.com/nomifun/nomifun-xiaozhi-yuntai) | ESP32-S3 voice, display, motion, and device-tool endpoint for a Desktop companion | [Firmware README](https://github.com/nomifun/nomifun-xiaozhi-yuntai#readme) · [Desktop integration](../guides/xiaozhi-robot.md) |

## Hub-and-surface model

```text
                       trusted LAN / authenticated channels

  NomiFun Mobile  ───────────────┐
  Xiaozhi Yuntai ────────────────┤
  Agent Mini Apps ───────────────┼──▶ NomiFun Desktop
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

On a LAN, Mobile connects to the authenticated listener inside Desktop. Pairing
uses a short-lived QR login token: it expires after five minutes and can be
consumed only once. After authentication, realtime interaction stays between
the phone and the selected Desktop instance; it does not pass through a NomiFun
cloud relay. The phone is therefore a control and presentation surface while
Desktop remains the data, credential, model, Agent, task, and tool authority.

This design has three practical advantages:

- model credentials and the durable workspace do not need to be copied to a
  mobile cloud account or stored independently on the phone;
- a change to a companion, model, Skill, knowledge base, requirement, or task is
  immediately the same state seen by Desktop and Mobile;
- the Desktop owner explicitly opens and closes the LAN boundary, and can rely
  on one-time pairing instead of publishing a long-lived bearer URL.

There is no built-in TLS on the LAN listener. Use it only on a trusted LAN or a
trusted private VPN, keep the operating-system firewall enabled, and never
publish the listener directly to the public Internet. See
[WebUI Remote Access](../guides/webui-remote-access.md) for the complete threat
model and operating steps.

### Xiaozhi is a hardware surface for a companion

Xiaozhi Yuntai adds microphones, speakers, a display, servos, and device-side
tools. Desktop supplies the companion identity, memory, knowledge, models, ASR,
TTS, sessions, and tool coordination. Binding the device to a companion keeps
the physical embodiment in the same governed runtime as desktop and mobile
interaction instead of creating a disconnected robot account.

### Mini Apps are durable Agent-made interfaces

An Agent Mini App is created from a normal conversation, previewed, and
published into a Desktop-managed library. Desktop maintains a published
snapshot and a guarded working copy. Continuing an app creates another ordinary
conversation with an explicit source path; it does not create a hidden second
conversation system. This makes a generated interface reusable while preserving
the same audit trail, local storage boundary, Agent runtime, and tool policy.

## What is distinctive about the architecture

### One capability graph, many entry points

Desktop companions, built-in Agents, supported Agent CLIs, Mobile, hardware,
Mini Apps, MCP/REST callers, and IM channels all converge on Desktop-managed
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

## Product innovation milestones

The dates below record when capabilities first reached internal users or the
released product. They are product-availability milestones, not inferred source
commit dates.

### Implemented in-house and used by internal users in 2025

1. Native computer use and browser use for the xiaozhiAI Agent runtime.
2. An automated requirements-management platform and continuous loops for
   Claude and Codex Agents.
3. A three-party Agent system separating decision, implementation, and
   supervision, with intelligent-decision recovery (IDMM).
4. Hardware multimodal access for companions.
5. Knowledge bases as durable working context for Agent reasoning workflows.
6. Self-evolution and migration of companion memory, Skills, and settings.

### Innovations launched in early 2026

1. Agent Desktop Mini Apps.
2. A security-controlled customer-service cluster system.
3. NomiFun Mobile directly connected to Desktop with no NomiFun cloud relay on
   the LAN path.
4. NomiFun's original multi-Agent collaboration interaction model, represented
   by the single auditable `AgentExecution` aggregate.
5. The super Desktop-companion Agent IM-channel gateway.
6. Additional unreleased capabilities remain confidential.

## Related guides

- [Architecture overview](overview.md)
- [WebUI remote access](../guides/webui-remote-access.md)
- [Xiaozhi robot integration](../guides/xiaozhi-robot.md)
- [Companions](../guides/companions.md)
- [AutoWork and Requirements](../guides/autowork-requirements.md)
- [Intelligent Decision (IDMM)](../guides/intelligent-decision.md)
- [Computer Use and Browser Use](../guides/computer-browser-use.md)
