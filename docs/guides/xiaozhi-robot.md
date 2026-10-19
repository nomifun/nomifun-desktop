# Connect a XiaoZhi Robot

[简体中文](xiaozhi-robot.zh.md)

NomiFun can act as a local AI backend for a compatible XiaoZhi ESP32 robot. The
robot handles the microphone, speaker, display, buttons, and device-side MCP
tools; NomiFun provides the companion, chat model, ASR, TTS, memory, sessions,
and tool coordination.

This integration is optional. A firmware build configured for NomiFun connects
to your desktop over the LAN instead of using the default `xiaozhi.me` service.

## What You Need

A paired device uses the companion's primary Agent, chat model, persona and Skill/MCP setup with an independent device Session and history. Entry points can run concurrently. Reconnection reuses that device Session; navigation never redirects its input.

Cohabit provides one Agent/model setup. **Connections → Conversation robot** configures the exact Session's device resources. Pairing alone grants no tools: calls still need the Agent Action, exact resource and device permission. Unbound or offline devices do not block unrelated chat or ordinary work.

Integrations use `PUT /api/product-agent-bindings/companion/{companion_id}` with `selection`, the exact `conversation_id` and complete `resource_selections`. Existing Sessions change through formal idle, effect-settlement and CAS transitions, preserving identity and history. Future defaults change only when requested. Device affiliation and the current connection determine the target; names, navigation and message text cannot redirect it.

Device settings show the permissions supported by the connected firmware. Photos, movement and extension tools require their respective permissions. Continuous observation requires an additional permission and remains scoped to the current conversation turn. Revoking permissions, unpairing or disconnecting invalidates earlier device authority. Photos and observations are attached to the originating conversation message.

This version uses a fresh robot data baseline and does not migrate old robot threads, Agent bindings or messages.
At startup, only the exact retired registry format without device permissions and
an authorization revision is isolated, byte for byte, with its old thread file in
`robot/retired-pre-authorization/`. The device list then starts empty; reconnect
and pair the robot again. Current device bindings and permissions remain intact.
Damaged or mixed formats and conflicting retirement files still prevent the
robot service from starting; startup never supplies missing permissions or
overwrites retired files.

See the [companion architecture and acceptance status](../architecture/companion.zh.md)
(Chinese) for one Agent, multiple sessions, on-demand background work and shared
context. This guide does not establish acceptance of real robot movement,
speech latency or cross-device approvals.

- The [NomiFun desktop app](https://github.com/nomifun/nomifun-desktop) running
  on a computer connected to the same LAN as the robot.
- A compatible XiaoZhi firmware build. The
  [nomifun-xiaozhi-yuntai](https://github.com/nomifun/nomifun-xiaozhi-yuntai)
  project includes the `esp32-s3n16r8-emoji` board and head-servo MCP tools.
- A configured NomiFun companion.
- Available chat, speech-recognition (ASR), and speech-synthesis (TTS) models.

The robot must be able to reach the computer directly. Guest Wi-Fi or AP
isolation can prevent two devices on the same Wi-Fi network from communicating.

## 1. Configure the Companion and Voice

1. Open **Desktop companions** in NomiFun and select or create a companion.
2. On **Overview**, configure its main chat model.
3. In the same **Model configuration** section, select an ASR model and a TTS
   model (including a voice when the provider requires one).
4. Test a normal text conversation with that companion first.

The default primary Agent is `companion.default`. Weather, research, knowledge, files and computer operations use authorized tools directly in the receiving Session. Device actions need explicit Robot capabilities, exact device resources and permissions. Complex work can use background or parallel execution when useful.

After editing an Agent, explicitly **Apply latest setup** to an idle Session with settled operations. Updating defaults never replaces other Sessions. ASR, TTS and vision retain their individual media settings.

The chat model remains your choice. Existing explicit reasoning settings or
the model default still apply; this preset does not replace the global model.
Natural-language commands still include recognition, model decisions, device
communication and speech synthesis. A lean configuration does not guarantee
millisecond response; latency requires measurements on a real device.

A provider existing in the catalog is not enough: Companion needs a usable primary chat model, and voice interaction also needs the companion's ASR,
TTS and any required voice selection.

## 2. Get the NomiFun OTA Address

1. Open the companion's **Channels & Robots** tab.
2. In **Robot connection**, select **Add a robot**.
3. If NomiFun reports that LAN access is off, select **Turn it on**.
4. Keep the dialog open and copy one of the displayed OTA addresses. It ends in
   `/robot/ota` and normally uses NomiFun's LAN port `25808`.

Choose the address whose IP belongs to the network shared with the robot. Do not
use `127.0.0.1`: on the ESP32 it refers to the robot itself.

## 3. Point the Firmware at NomiFun

Flash a compatible firmware build and open the robot's Wi-Fi setup page. Under
**Advanced settings**, paste the complete OTA address from NomiFun into the
**OTA address** field, then save the Wi-Fi settings and restart the robot.

The device requests that address at startup. NomiFun responds with the WebSocket
configuration required for `/robot/v1`; you do not need to construct or enter a
WebSocket URL manually.

For board selection, building, flashing, wiring, and servo precautions, follow
the firmware repository's README and the board-specific
`main/boards/esp32-s3n16r8-emoji/README.md`.

## 4. Bind the Robot

1. After restart, the robot displays and reads out a six-digit activation code.
2. Return to NomiFun's **Add a robot** dialog.
3. Enter the code and select **Bind to this companion**.
4. Wait for the robot to appear in the companion's **Robot connection** list.

Activation codes expire. If a code is rejected, restart the connection flow and
use the newest code shown on the device. A robot already bound to another
companion must be unbound there before it can be claimed again.

## 5. Verify the Connection

Before testing device tools, confirm all three requirements: the device Session's
current Agent explicitly grants the corresponding Robot Actions, that device's
Robot session binds the correct `robot` resource, and device settings allow the
operation. Pairing alone does not grant Agent tool authority. Ordinary voice
conversation does not require camera or movement permission.

| Robot Action | Device tools |
| --- | --- |
| `robot/vision` | Camera and vision tools; continuous observation needs an additional permission. |
| `robot/display` | Screen, expression and lighting tools. |
| `robot/motion` | Head, gimbal and servo tools, including `self.head.get_status` and the combined expression/motion tool `self.head.express`. |
| `robot/device` | Other device extension tools provided by the firmware. |

The unified official **Companion** Agent includes ordinary work and the four Robot Actions. New seeds do not override saved custom grants. Use **Edit Agent → Add recommended capabilities**, review and save, then **Apply latest setup** to the idle device Session. When its device resource version changes, **Apply robot** separately and enable the required device permissions.

1. Speak to the robot, open its **Robot** session in the session selector, and
   confirm that recognized text appears in the correct device's history.
2. Confirm that the reply is played by the robot and that speaking or pressing
   the device's interaction button can interrupt playback as supported by the
   firmware.
3. Ask the companion to read the head status or move its head. The
   `esp32-s3n16r8-emoji` firmware exposes device-side MCP tools under
   `self.head.*`, including `self.head.get_status`.

Conversational expressions and gestures use the same session's authorized MCP
tools. Updated gimbal firmware offers `self.head.express` for a combined face and
head gesture. Because its face animations can also move the servos, this combined
tool requires `robot/motion` and device motion permission. The model chooses
appropriate moments instead of moving on every sentence. Emotion tags in spoken
text are not control instructions. An `accepted` or queued response confirms
submission, not completed physical movement.

Servo motion is disabled by default on firmware builds that require calibration.
Read the board documentation and calibrate center/travel limits before enabling
automatic motion. Incorrect limits can stall or damage a servo.

## 6. Configure and extend robot capabilities

Open **Channels & robots → Robot connections → Capability configuration** for a
device. Its MCP tools and JSON Schema generate the editor; it does not depend on
board names, tool names, or a fixed number of servos.

- Enable or disable tools, edit descriptions and default arguments, and classify
  tools with unfamiliar names. Existing camera and physical-motion categories
  cannot be downgraded to weaker permissions. Original device schemas still
  constrain every argument; configuration cannot widen physical travel limits.
- Create named actions such as “Greeting” from an existing tool and fixed
  arguments. Multiple named actions may reuse one device tool and retain its
  permission and effect-receipt path. Saving or refreshing does not execute them.
- Personality remains owned by the original companion. Edit timing and gesture
  frequency in the companion's unified **Agent setup**. Adding ordinary work capabilities does not create a separate device Agent or force a heavier interaction projection. Tool descriptions grant no authority.

Saving creates a new device capability revision without changing Agent Actions
or device permissions. In Cohabit, open that device's Robot session and
**Apply robot** under **Conversation robot** while idle. This binds the new
revision to that session, preserving its identity,
model, history and other resources. Old tool invokers cannot dispatch after the
configuration changes. When firmware publishes new tools, **Refresh device
capabilities**, then reapply the robot to the conversation.

This editor configures and extends capabilities; it is not a manual hardware
control panel. New hardware primitives require firmware support, while defaults,
usage descriptions and named actions over existing primitives are editable.

Speech recognition and device frame processing are scheduled separately. MCP
replies, cancellation and disconnects remain responsive while ASR is pending;
cancelled or replaced connections cannot submit late transcripts as new commands.
Speech consumes formally committed public prefixes with exact source identity before the whole work Turn ends. Raw tokens, reasoning, tool bodies and withdrawn steps are excluded. The firmware defines what an
`accepted` receipt means; a spoken reply does not prove physical completion.

In `auto` and `realtime` modes, one `listen.start` opens a continuous listening
window. Server-side VAD separates utterances without closing that window. After
a motion finishes silently, the next utterance can still be recognized without
a TTS start/stop transition. Continuous silence retains only bounded pre-roll
audio and does not repeatedly submit empty transcriptions. An explicit
`listen.stop`, `abort`, or connection closure ends the window; manual mode still
follows the device's explicit start/stop lifecycle. Local regressions must send
multiple Opus utterances after a single start, rather than sending another start
before every command. This contract does not establish that the current change
has passed physical microphone, playback, or interaction-latency acceptance.

## 7. Desktop Tasks and Multiple Sessions

Clear movement or expression commands execute directly and finish without speech after all requested actions are accepted.
Answer questions and explicit requests for a spoken receipt; explain failures, blockers, uncertainty or necessary clarification.
Ordinary companionship follows your requested depth.
Physical replies use natural spoken wording without decorative emoji, kaomoji, ASCII emoticons or stage directions.
Requested quotations, formulas and code retain their symbols; existing historical replies remain unchanged.
Weather, current information, knowledge, files and computer operations use authorized tools directly in the device Session. Durable work, context isolation or useful parallelism can use generic background execution. Results deliver along the original request relation without mandatory model summarization.

Current information still needs working network tools and credentials; physical actions retain exact device-resource and permission checks. Apply changed authoring explicitly to an idle Session with settled operations. See the [companion architecture](../architecture/companion.zh.md) for status and validation boundaries.

The session selector lets you inspect separate histories. A new Companion session
retains the shared persona and long-term memory. **Continue in desktop window**
explicitly changes only the desktop quick-window target, preserving robot and
channel input destinations. Stopping playback and cancelling background work are
separate operations; cancel the exact task when you want work to stop. Questions
and approvals still use that task's formal interaction. Agreement in another
conversation, historical memory or stopping audio does not establish approval,
successful cancellation or completed work.

## How Data Flows

```text
Microphone -> XiaoZhi firmware -> ASR -> one Companion Agent / device Session
                                                    |          |
                                                    |          +-> authorized work / device MCP tools
                                                    +-> optional background / parallel work
                                                    |
Speaker <- Opus <- paced, backpressured TTS <- committed public prefixes / source results
```

Voice data and conversation content are processed according to the providers
selected in NomiFun. "Local backend" describes the robot gateway and session
coordination; it does not make a cloud ASR, TTS, or chat provider local.

## Troubleshooting

- **No OTA address is shown:** Turn on LAN access in the desktop app and confirm
  the computer has a non-loopback network address.
- **The robot cannot reach the OTA address:** Put both devices on the same LAN,
  allow NomiFun through the OS firewall, and disable guest/AP isolation.
- **The activation code is missing or rejected:** Confirm that the OTA address
  ends in `/robot/ota`, restart the robot, and use the newest six-digit code.
- **`Nomi conversation has no provider/model configured`:** Configure the bound
  companion's primary chat model on **Overview**.
  Existing sessions also require explicitly applying a usable model. A global
  provider alone is not sufficient.
- **Speech is not recognized:** Configure an ASR model for the companion and
  verify the provider credentials and quota.
- **No audio reply is played:** Configure the companion's TTS model and voice,
  then verify speaker wiring and volume.
- **Head commands do nothing:** Confirm the device Session's current Agent grants
  `robot/motion`, the corresponding Robot session binds the correct resource, device
  settings allow motion, and the firmware exposes `self.head.*`. For physical
  movement, also calibrate the device and enable motion as described by the board
  documentation. `self.head.get_status` belongs to `robot/motion` too and requires
  the corresponding Agent grant and device permission.

## Network Safety

Enabling LAN access makes NomiFun listen on the local network. Use it only on a
trusted LAN, keep the operating-system firewall enabled, and do not expose port
`25808` directly to the public Internet. Stop LAN access when you no longer need
robot or WebUI connections.
