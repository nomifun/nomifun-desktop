# Connect a XiaoZhi Robot

[简体中文](xiaozhi-robot.zh.md)

NomiFun can act as a local AI backend for a compatible XiaoZhi ESP32 robot. The
robot handles the microphone, speaker, display, buttons, and device-side MCP
tools; NomiFun provides the companion, chat model, ASR, TTS, memory, sessions,
and tool coordination.

This integration is optional. A firmware build configured for NomiFun connects
to your desktop over the LAN instead of using the default `xiaozhi.me` service.

## What You Need

Once paired, a physical device uses a separate **Robot** role and device session
under the same companion, sharing its persona, voice and long-term memory. The
desktop continues using the **Companion** role. Both roles remain available
together; you do not switch the desktop session to a Robot Agent. Reconnecting
reuses that device's input session, and viewing another conversation does not
redirect device input.

The Cohabit sidebar has separate **Companion** and **Robot** settings. Physical
control defaults to Robot. When viewing a Robot session,
**Connections → Conversation robot** shows its device-resource settings.
Pairing does not grant arbitrary tools: a call still requires the selected Agent's
Action, the exact device resource and device permission. Apply configuration
after the reply and operation settlement; active or paused turns block changes.

Integrations should use the Robot role's formal product-binding endpoint,
`PUT /api/product-agent-bindings/robot/{companion_id}`, with its `selection`,
exact `conversation_id` and complete `resource_selections`. Users can also explicitly
select Robot capabilities and real device resources for Companion through its own
formal configuration endpoint; this does not redirect physical input. Applying a configuration to an existing session
uses a formal binding transition and preserves its identity and history. Device
identity comes from that Robot session's affiliation; a display name, page
selection or message text cannot redirect it to another device.

Device settings show the permissions supported by the connected firmware. Photos, movement and extension tools require their respective permissions. Continuous observation requires an additional permission and remains scoped to the current conversation turn. Revoking permissions, unpairing or disconnecting invalidates earlier device authority. Photos and observations are attached to the originating conversation message.

This version uses a fresh robot data baseline and does not migrate old robot threads, Agent bindings or messages.

See the [companion architecture and acceptance status](../architecture/companion.zh.md)
(Chinese) for dual roles, multiple sessions, desktop delegation and shared
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

## 1. Configure the Companion and Robot Roles

1. Open **Desktop companions** in NomiFun and select or create a companion.
2. On **Overview**, configure its main chat model.
3. In the same **Model configuration** section, select an ASR model and a TTS
   model (including a voice when the provider requires one).
4. Test a normal text conversation with that companion first.

In **Robot** settings, select the official **Robot** (`companion.robot`) Agent
or a personal Agent that meets the role's requirements. The official setup keeps
companion identity, long-term memory, device operations and narrow desktop
delegation, reducing general tools and historical context. **Companion** settings
continue to configure desktop work; the roles are not mutually exclusive.
Upgrades do not silently replace saved companion or personal Agents or expand
their permissions. To adopt an edited revision, explicitly **Apply latest setup**
to the idle target session, then enable required device permissions separately.

Robot can have an independent default chat model. With no override
(`robot.model=null`), it inherits the Companion model. **Use Companion model
for future conversations** only changes future defaults, preserving existing sessions.
When viewing a Robot session, its role settings explicitly update that session
and save future defaults; editing the other role's defaults leaves existing
sessions unchanged.

The chat model remains your choice. Existing explicit reasoning settings or
the model default still apply; this preset does not replace the global model.
Natural-language commands still include recognition, model decisions, device
communication and speech synthesis. A lean configuration does not guarantee
millisecond response; latency requires measurements on a real device.

A provider existing in the catalog is not enough: Robot needs a usable independent
or inherited chat model, and voice interaction also needs the companion's ASR,
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

Before testing device tools, confirm all three requirements: the Robot role's
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

The official **Robot** Agent includes all four Actions. The official **Companion**
Agent handles desktop work and no longer includes physical control by default.
Available tools still depend on the exact device, firmware and permissions.
Upgrades do not automatically expand saved Agent configurations or existing
sessions. To add recommendations, choose **Edit Agent → Complete recommended
capabilities** in **Robot** settings, review the additions, then **Complete and
save**. Return to the corresponding Robot session and **Apply latest setup** after
its reply finishes. Use **Apply robot** when its device-resource revision needs
refreshing, and enable the required device permissions. Personal Agents also need
explicit grants for the Robot Actions you want to use.

Start with these checks:

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
  frequency in the **Robot** role's **Agent setup**. Adding general workspace or Office capabilities makes it an
  extended Agent and opts it out of automatic lean-context treatment. Tool
  descriptions explain capabilities and grant no authority.

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
Speech playback consumes the current turn's final valid public answer, excluding
reasoning, pre-tool narration and withdrawn steps. The firmware defines what an
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
For missing capabilities such as weather, current information, knowledge, computer work or elaborate motion planning,
Robot can direct even a short question to the same companion's desktop role.
The task runs in a separate linked session using Companion's own model, tools
and resources. It does not inject work into the chat you are viewing or transfer
computer authority to Robot. The reception Turn ends after acceptance; recorded results return to the original valid device connection;
completion and artifacts require the corresponding task receipt. Current-information queries still require working network
tools and credentials on Companion's side. A lean Robot with no explicit Session reasoning choice uses a lightweight
reception tier only for models with exact documented support; unknown models retain their defaults and desktop workers
use their own model settings. For an edited official Robot missing delegation, use **Edit Agent → Add recommended capabilities**;
for a personal Agent, select collaboration's `agent/delegate` manually in the workbench. Save, then **Apply latest settings**
to the idle device Session. See the
[companion architecture](../architecture/companion.zh.md) for the current status
of automatic follow-up orchestration and spoken result delivery.

The session selector lets you inspect separate histories. A new Companion session
retains the shared persona and long-term memory. **Continue in desktop window**
explicitly changes only the desktop quick-window target, preserving robot and
channel input destinations. Stopping playback and cancelling desktop work are
separate operations; cancel the exact task when you want work to stop. Questions
and approvals still use that task's formal interaction. Agreement in another
conversation, historical memory or stopping audio does not establish approval,
successful cancellation or completed work.

## How Data Flows

```text
Microphone -> XiaoZhi firmware -> NomiFun ASR -> Robot-role model
                                                     |
Speaker <- Opus audio <- XiaoZhi firmware <- NomiFun TTS
                                                     |
                         device MCP tools <----------+
                                                     |
                         linked desktop task <-- directed delegation --> Companion-role model
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
  companion's Robot default model or its inherited Companion model on **Overview**.
  Existing sessions also require explicitly applying a usable model. A global
  provider alone is not sufficient.
- **Speech is not recognized:** Configure an ASR model for the companion and
  verify the provider credentials and quota.
- **No audio reply is played:** Configure the companion's TTS model and voice,
  then verify speaker wiring and volume.
- **Head commands do nothing:** Confirm the Robot role's current Agent grants
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
