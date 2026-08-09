# Connect a XiaoZhi Robot

[简体中文](xiaozhi-robot.zh.md)

NomiFun can act as a local AI backend for a compatible XiaoZhi ESP32 robot. The
robot handles the microphone, speaker, display, buttons, and device-side MCP
tools; NomiFun provides the companion, chat model, ASR, TTS, memory, sessions,
and tool orchestration.

This integration is optional. A firmware build configured for NomiFun connects
to your desktop over the LAN instead of using the default `xiaozhi.me` service.

## What You Need

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

The robot uses the models assigned to the companion it is bound to. A provider
existing in the model catalog is not enough by itself: the companion must have
a usable main chat model, and voice conversation also needs usable ASR and TTS
models.

## 2. Get the NomiFun OTA Address

1. Open the companion's **Remote control** tab.
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

Start with these checks:

1. Speak to the robot and confirm that recognized text appears in the NomiFun
   companion session.
2. Confirm that the reply is played by the robot and that speaking or pressing
   the device's interaction button can interrupt playback as supported by the
   firmware.
3. Ask the companion to read the head status or move its head. The
   `esp32-s3n16r8-emoji` firmware exposes device-side MCP tools under
   `self.head.*`, including `self.head.get_status`.

Servo motion is disabled by default on firmware builds that require calibration.
Read the board documentation and calibrate center/travel limits before enabling
automatic motion. Incorrect limits can stall or damage a servo.

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
orchestration; it does not make a cloud ASR, TTS, or chat provider local.

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
- **Head commands do nothing:** Confirm the firmware exposes `self.head.*`, then
  calibrate it and enable motion as described by the board documentation.

## Network Safety

Enabling LAN access makes NomiFun listen on the local network. Use it only on a
trusted LAN, keep the operating-system firewall enabled, and do not expose port
`25808` directly to the public Internet. Stop LAN access when you no longer need
robot or WebUI connections.
