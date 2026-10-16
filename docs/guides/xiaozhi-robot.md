# Connect a XiaoZhi Robot

[简体中文](xiaozhi-robot.zh.md)

NomiFun can act as a local AI backend for a compatible XiaoZhi ESP32 robot. The
robot handles the microphone, speaker, display, buttons, and device-side MCP
tools; NomiFun provides the companion, chat model, ASR, TTS, memory, sessions,
and tool coordination.

This integration is optional. A firmware build configured for NomiFun connects
to your desktop over the LAN instead of using the default `xiaozhi.me` service.

## What You Need

Once paired, a robot continues its companion's existing desktop conversation with the same persona, voice and long-term memory. Reconnecting does not create another conversation. The optional official **Robot companion** (`companion.robot`) preset can be explicitly selected for that same companion and session; it does not create another character.

Ordinary Agent conversations can explicitly select a robot in the resource picker before launch. In a companion's Cohabit view, use **Connections → Conversation robot** in the right sidebar, select a device paired with that companion, then click **Apply robot**. Select **No robot control** and apply to remove the conversation's device control resource. Wait until the current reply finishes; active and paused turns block changes. Pairing a single device does not automatically select it or grant control authority.

The picker calls `PUT /api/product-agent-bindings/companion/{companion_id}` with the current Agent `selection`, the existing `conversation_id`, and the complete `resource_selections`, including the `robot` and all other resources to retain. This uses the formal session binding transition and preserves session identity, the current model, other resources, and message history. Integrations should use this same endpoint; the companion's Agent dropdown does not select a robot resource.

Device settings show the permissions supported by the connected firmware. Photos, movement and extension tools require their respective permissions. Continuous observation requires an additional permission and remains scoped to the current conversation turn. Revoking permissions, unpairing or disconnecting invalidates earlier device authority. Photos and observations are attached to the originating conversation message.

This version uses a fresh robot data baseline and does not migrate old robot threads, Agent bindings or messages.

- The [NomiFun desktop app](https://github.com/nomifun/nomifun-desktop) running
  on a computer connected to the same LAN as the robot.
- A compatible XiaoZhi firmware build. The
  [nomifun-xiaozhi-yuntai](https://github.com/nomifun/nomifun-xiaozhi-yuntai)
  project includes the `esp32-s3n16r8-emoji` board and head-servo MCP tools.
- A configured NomiFun companion.
- Available chat, speech-recognition (ASR), and speech-synthesis (TTS) models.

The robot must be able to reach the computer directly. Guest Wi-Fi or AP
isolation can prevent two devices on the same Wi-Fi network from communicating.

## 1. Configure the Companion

1. Open **Desktop companions** in NomiFun and select or create a companion.
2. On **Overview**, configure its main chat model.
3. In the same **Model configuration** section, select an ASR model and a TTS
   model (including a voice when the provider requires one).
4. Test a normal text conversation with that companion first.

For device-focused interaction, select **Robot companion** under the Cohabit
sidebar's **Mind configuration → Agent**. Its default capabilities are limited
to the companion, long-term memory and robot operations. Switch only after the
current turn ends. The companion, session, model and complete stored history
remain intact; the lean model context selects only necessary recent content.
Then select and apply the device under **Connections → Conversation robot** and
enable its required device permissions separately. Upgrades never silently
replace existing companion or personal Agents or expand their permissions.

The chat model remains your choice. Existing explicit reasoning settings or
the model default still apply; this preset does not replace the global model.
Natural-language commands still include recognition, model decisions and device
communication, so changing the preset alone does not guarantee millisecond response.

The robot uses the models assigned to the companion it is bound to. A provider
existing in the model catalog is not enough by itself: the companion must have
a usable main chat model, and voice conversation also needs usable ASR and TTS
models.

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

Before testing device tools, confirm all three requirements: the companion's
current Agent explicitly grants the corresponding Robot Actions, the current
companion session explicitly selects this paired device as its `robot` resource,
and device settings allow the requested operation. Pairing alone does not grant
Agent tool authority. Ordinary voice conversations
do not require camera permission or a robot resource selected in advance.

| Robot Action | Device tools |
| --- | --- |
| `robot/vision` | Camera and vision tools; continuous observation needs an additional permission. |
| `robot/display` | Screen, expression and lighting tools. |
| `robot/motion` | Head, gimbal and servo tools, including `self.head.get_status` and the combined expression/motion tool `self.head.express`. |
| `robot/device` | Other device extension tools provided by the firmware. |

The latest recommended official Companion Agent includes all four Actions.
Available tools still depend on the selected robot, its firmware and device
permissions. Upgrading does not automatically expand saved official Agent
configurations or existing sessions. To add the recommended capabilities, choose
**Edit Agent** under the companion's **Agent setup**, select **Complete recommended
capabilities**, review the additions, and select **Complete and save**. Return to
the companion and, after the current reply finishes, select **Apply latest setup**
to explicitly apply the saved configuration to its current session. This keeps
the session identity, current model and message history. Then select the device
under **Connections → Conversation robot** in the Cohabit view, click **Apply robot**, and allow the required operations in
device settings. Personal Agents also need explicit grants for the Robot Actions
you want to use.

Start with these checks:

1. Speak to the robot and confirm that recognized text appears in the NomiFun
   companion session.
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
- Personality remains owned by the original companion. Select the lean **Robot
  companion** preset or edit timing and gesture frequency in the existing Agent
  instructions. Adding general workspace or Office capabilities makes it an
  extended Agent and opts it out of automatic lean-context treatment. Tool
  descriptions explain capabilities and grant no authority.

Saving creates a new device capability revision without changing Agent Actions
or device permissions. In the Cohabit view, **Apply robot** while the conversation
is idle to bind the new revision to the same session, preserving its identity,
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

## How Data Flows

```text
Microphone -> XiaoZhi firmware -> NomiFun ASR -> companion chat model
                                                     |
Speaker <- Opus audio <- XiaoZhi firmware <- NomiFun TTS
                                                     |
                         device MCP tools <----------+
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
  companion's main chat model on **Overview**. A global provider alone is not
  sufficient.
- **Speech is not recognized:** Configure an ASR model for the companion and
  verify the provider credentials and quota.
- **No audio reply is played:** Configure the companion's TTS model and voice,
  then verify speaker wiring and volume.
- **Head commands do nothing:** Confirm the companion's current Agent grants
  `robot/motion`, its current session selects the correct robot resource, device
  settings allow motion, and the firmware exposes `self.head.*`. For physical
  movement, also calibrate the device and enable motion as described by the board
  documentation. `self.head.get_status` belongs to `robot/motion` too and requires
  the corresponding Agent grant and device permission.

## Network Safety

Enabling LAN access makes NomiFun listen on the local network. Use it only on a
trusted LAN, keep the operating-system firewall enabled, and do not expose port
`25808` directly to the public Internet. Stop LAN access when you no longer need
robot or WebUI connections.
