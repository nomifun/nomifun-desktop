# Linux 媒体许可与设备库存（2026-10-17）

目标为 WSL `Ubuntu-26.04` 的 `/home/muri/code/nomifun/nomifun-desktop`，普通登录用户 UID 1000。
本记录覆盖媒体调用链、独立许可、只读设备元数据、WebKit mock-device 契约验证，以及同一最终原生 Host 的正式 Settings 麦克风 request / Cancel / Allow / real-track / stop 验收。
后续验收只在独立私有 App 中取得并立即停止真实 audio track，没有读取、录制、保存或上传音视频内容/样本，没有修改日常 App 的 grant、配置或全局媒体服务。物理麦克风声音、真实音频图和摄像头仍未获证明。
网络/服务/Computer 与正式 App 构建及产品验收由本轮相应报告独立记证。

## 当前调用链与修复

- 语音输入：`ui/src/renderer/hooks/system/useSpeechInput.ts::startRecording` →
  `navigator.mediaDevices.getUserMedia({audio:true})` → 当前客户端 WebView/browser →
  `MediaRecorder` 与现役 ASR 服务。此次没有触发这条录音或 ASR 路径。
- Settings 麦克风测试：`CapabilityPermissionsContent.tsx::testMicrophone` → 同一
  `getUserMedia` → 立即停止所有轨道。后续同一最终原生 Host 已实际调用此正式入口，见下节原生验收。
- `/api/system/permissions` 的非 macOS `capture_state` 保持 `unknown`；
  非 macOS request 不能建立全局媒体 grant。Linux 没有可替代真实服务/设备调用的统一 TCC 布尔值。
  库存 `unknown` 不等于 denied、granted 或无设备。
- Desktop 原生宿主：Tauri → vendored Wry → WebKitGTK。
  修复前 Wry 与主 App 没有 `permission-request` 的 UserMedia 处理；WebKit 对未处理请求默认拒绝。
  本机 WebKitGTK 2.52.6 的 `enable-media-stream` 实测默认 `true`，`enable-webrtc` 默认 `false`；
  不能把本机拒绝归因于旧版本文档中的 media-stream 默认值。
- 新 `apps/desktop/src/linux_media.rs` 只安装到主 App WebView：显式启用 media-stream，
  只处理 `UserMediaPermissionRequest`，按真实 audio/video 属性显示原生 GTK 异步 Cancel/Allow。
  每次用户决定只结算当前 request，不建立 grant 数据库，也不把 WebKit allow 投影为系统/硬件 granted。
  `getUserMedia`/`MediaRecorder` 不需要启用 `RTCPeerConnection`，因此没有改 `enable-webrtc`。
- 允许出现原生确认的顶层上下文固定为配置的本地 dev origin（scheme/host/port 精确匹配）或
  正式 `tauri://localhost`；从远程顶层导航来的媒体 request 直接拒绝。
  页面开始新导航时取消旧 pending dialog，批准时再次核对 origin 与 document generation。
  退出/关闭和取消保留拒绝路径。
- WebKitGTK 4.1 的 UserMedia request 公开属性只有 audio/video 类型，没有 requesting-frame origin。
  原生提示只称真实的当前 NomiFun 顶层页面上下文，不声称知道请求 iframe 来源。
  `AppPreview`、`HTMLRenderer`、`OfficeArtifactViewer`、`PDFViewer` 四处已有预览 iframe
  现在均明确 `allow="camera 'none'; microphone 'none'"`；其中同源 `srcDoc` 原本不能靠默认 self
  阻止媒体请求。此次是这些具体预览的权限边界修复，没有建立通用 iframe 系统。
- 远程 WebUI 的 `mediaDevices` 属于浏览器所在客户端；服务端的 system-permission host 库存
  不能代表客户端浏览器或该客户端物理设备。Linux visible Browser 宿主的既有未接通边界
  不由本次主 renderer 媒体修复替代，也没有在本任务中改写其引擎结构。

## 只读实际库存

证据时间为 2026-10-17 11:21:53（Asia/Shanghai）。仓库外报告：
`/home/muri/nomifun-linux-evidence-20261017/linux-media-readonly-inventory.json`，
复现脚本为同目录 `linux-media-readonly-inventory.py`。
脚本只读取设备节点/版本与 libpulse source/sink metadata，不创建 PulseAudio stream。

| 项目 | 实际结果 | 可得结论 |
| --- | --- | --- |
| `/dev/video*`、`/sys/class/video4linux/*` | 均为空 | 没有通过这些原生设备节点向该 distro 提供摄像头；不推断 Windows 宿主没有物理摄像头 |
| `/dev/snd` | 仅 `/dev/snd/timer` | 未提供 ALSA PCM capture/playback 节点 |
| `PULSE_SERVER` | `unix:/mnt/wslg/PulseServer` | WSLg 提供 PulseAudio endpoint |
| libpulse context | READY，枚举完整 | endpoint 可连接；未创建媒体 stream |
| source metadata | `RDPSource` / `RDP Source`；`RDPSink.monitor` / `Monitor of RDP Sink` | 有逻辑音频输入和输出 monitor；monitor 不是物理麦克风 |
| sink metadata | `RDPSink` / `RDP Sink` | 有逻辑音频输出 |
| 本用户 PipeWire | `pw-dump` exit 255，`can't connect: Host is down` | 当前用户默认 PipeWire endpoint 不可连接；不推断 WSLg PulseAudio 也不可用 |
| 物理麦克风、Windows/桌面独立许可 | `unknown` | 逻辑 RDPSource 存在不证明物理硬件、宿主许可或真实采集成功 |

版本为 `libwebkit2gtk-4.1-0 2.52.6-0ubuntu0.26.04.1`、
`libpulse0 1:17.0+dfsg1-2ubuntu4`、`pipewire 1.6.2-1ubuntu1.2`。
此次未用 root 身份替代交互桌面、portal 或媒体许可。

## GStreamer 音频后端 factory 库存

补充只读检查使用 Python GI `Gst.ElementFactory.find`、registry device-provider factory metadata 和
`dpkg-query -S`，没有实例化 element、启动 provider、创建 pipeline/stream 或读取内容。
证据为仓库外 `linux-media-gstreamer-factories.json`。

| Factory | 存在 | 实际 plugin / 发行包 |
| --- | --- | --- |
| `pulsesrc` | 是 | `pulseaudio` 1.28.2；`libgstpulseaudio.so` 属于 `gstreamer1.0-plugins-good:amd64` |
| `autoaudiosrc` | 是 | `autodetect` 1.28.2；属于 `gstreamer1.0-plugins-good:amd64` |
| `pipewiresrc` | 是 | `pipewire` 1.6.2；属于 `gstreamer1.0-pipewire:amd64` |
| `v4l2src` | 是 | `video4linux2` 1.28.2；属于 `gstreamer1.0-plugins-good:amd64` |
| `alsasrc` | 否 | 本次没有 ALSA PCM 设备，也不把此项缺失混同为 PulseAudio 不可用 |

同一 registry 有 `pulsedeviceprovider`（Audio Source/Sink，rank 256）、
`pipewiredeviceprovider`（Audio/Video Source/Sink，rank 257）、`v4l2deviceprovider`（rank 256）。
这里只查 factory metadata，没有启动这些 provider。

核对与本机版本对应的 WebKit `webkitgtk-2.52.6` 上游源码：音频 `getUserMedia` 沿
`GStreamerAudioCaptureSourceFactory` → `GStreamerAudioCaptureDeviceManager` 的 `GstDeviceMonitor`
Audio/Source 设备 → `GStreamerCapturer::createSource` 的 `gst_device_create_element`。
它没有硬编码通过 `autoaudiosrc` 选麦克风。GStreamer 1.28.2 的 Pulse device provider 将
Audio/Source 映射为 `pulsesrc`；实际选到哪个 provider/设备仍由本机运行结果决定。
这些源码事实与本机现存 pulse factory/只读 endpoint 库存相互对应，不等于真实 capture 成功。

本机 `pulsesrc` 确实在已声明的 `gstreamer1.0-plugins-good` 包内，当前 Debian runtime 的
base/good/pipewire 声明覆盖上述实际可用 factory；没有证据要求为本目标增加独立的
`gstreamer1.0-pulseaudio` 包依赖。其他发行版若拆包，应按其 plugin 文件归属提供对应 runtime 包，
不能只凭包名或 mock 测试推断。正式目标仍应核对真实设备、session、服务许可、codec 和释放结果。

## 已完成验证与证据边界

1. 使用目标 Linux 实际已编译的 gtk/tauri/webkit2gtk/url/anyhow 依赖 metadata，
   对 `linux_media.rs` 单独 `rustc --emit metadata` 通过；不占用正在构建 App 的 Cargo 锁。
   这是原生 API/typecheck，不是正式 App 链接、打包或设备验收。
2. 从当前源模块提取原样 `AppOrigin` 和该模块的 origin 测试，使用相同 url crate 原生编译执行：
   1 passed，0 failed。覆盖 scheme/host/port、用户信息、伪 localhost、about:blank 和正式 tauri origin。
   日志 `linux-media-origin-test.log`；完整 App 测试/构建以本轮总记录为准。
3. focused UI：`AppPreview.interaction.test.tsx` 与 `OfficeArtifactViewer.test.tsx`：
   26 passed，0 failed，145 assertions；`bun run typecheck` exit 0；
   `bun run check:desktop-ui-boundary` 通过（2168 renderer sources，最小 880×600）。
4. 原生 WebKit API 契约 fixture 只设置 `enable_mock_capture_devices=true`，没有真实采集：
   `linux-media-fake-webkit.py` → `linux-media-fake-webkit-report.json`，3/3 passed。
   - 未处理 UserMedia：`NotAllowedError`，native request 1 次。
   - 明确 allow：1 条 mock audio track，随后 ended；`enable-webrtc=false`。
   - 同源 srcDoc iframe 加上述 media deny policy：`NotAllowedError`，native request 0 次。
   fixture 证明安装 WebKit 的 permission/policy 行为，不是生产 Rust adapter 的完整原生 UI交互，
   不是真实麦克风/摄像头、ASR、拒绝/撤回/恢复或发行包验收。

当前已有正式 App 的原生 GTK request、Cancel/Allow 和真实 audio track live→ended 证据；原生按钮由限定在私有 App 内的 AT-SPI 验收自动化操作，不声称用户人工点击。
仍未覆盖：物理麦克风声音、真实运行音频图与可归属 Pulse source-output 的创建/释放、摄像头、宿主隐私拒绝/撤回/恢复矩阵，以及发行包设备链。保留这些明确边界，不将未验证写成 OS 不支持或无麦克风。

## 同一最终原生 Host 的正式麦克风入口验收

补充验收在 2026-10-17 12:45–12:54（Asia/Shanghai）执行；仓库外主报告为
`/home/muri/nomifun-linux-evidence-20261017/linux-media-production-report.json`，
原始纯元数据与会话清理报告归档在同目录 `linux-media-production-evidence/`。

实际 Host 为 `native-host/final-native-owner-thread/nomifun-desktop`：

- Host SHA-256：`d2cb042593ec33ba4dd0b82e5ec31f026c1d49030f6a94b435220c2a2d793c04`。
- 配对 build identity：`cf41ea5eefe8741cbd44a86fbd5175a6d711cb6597332eafd893b4c11b3a8ada`。
- UID 1000，App PID 203704；这是本轮静态 UI debug Host，不是发行包或物理设备验收。
- 使用新建 0700 data/XDG、真正的私有 Xvfb :91、私有 session bus 与 AT-SPI bus。
  没有操作主验收 App、日常桌面、日常媒体许可、全局 Pulse/portal 服务或主用户配置。
  X11 隔离不替代 WSLg 的音频服务：App 继承实际 `PULSE_SERVER=unix:/mnt/wslg/PulseServer`。

通过正式 renderer 的 Settings → Capabilities & Permissions → Voice Input → Test microphone，
当前文档始终是 `tauri://localhost#/settings/permissions?tab=voice-input`。
使用已存在正式 DOM button 的 `.click()` 调用实际 React handler，没有新增测试路由或替换该 handler。
WebKit 自带 loopback inspector 仅安装临时元数据 observer：调用原始 `getUserMedia`，将同一 stream
交回正式回调，并观察正式 `track.stop()` 前后的 `readyState`；不返回假 stream、假 track 或假许可。
observer 强持有实际 MediaDevices wrapper，避免它在 inspector 连接结束后的 wrapper 回收导致观察失效。
没有打开 WebKit mock devices，也没有调用 MediaRecorder、ASR 或任何 PCM/频谱样本读取 API。

原生许可操作只匹配私有 AT-SPI bus 上唯一 App PID 203704、原生 alert `Question`、
`Allow access to the microphone?` 与精确的当前 `tauri://localhost` 上下文说明，然后点击其原生 Cancel/Allow。
没有通过 JS 或测试 permission handler 直接调用 WebKit `request.allow()`。

| 实际步骤 | 结果与证明范围 |
| --- | --- |
| 正式点击 → 原生 GTK Cancel | 唯一真实原生 microphone dialog 已拒绝；原生标签、PID、动作和时间保存在 `native-dialog-metadata.json` |
| 同文档再次正式点击 | 真实 `getUserMedia` 返回 `NotAllowedError`，没有新 GTK dialog；这是 WebKit 在该文档中的拒绝缓存，不伪称应用永久拒绝 |
| 正常 reload 同一正式页面，再次点击 → 新原生 Allow | 确有新的同 PID/精确顶层 origin 原生 dialog；实际返回 1 条 audio track、0 条 video track |
| 正式 Settings 回调立即 stop | 元数据证明该 audio track 在 Promise resolution 时为 `live`，正式 stop 前 `live`、之后 `ended`，全部轨道 ended；没有样本读取/录制/提交 |

`observer-denied.json` 和 `observer-allowed.json` 分别保留拒绝与成功，不用后一次覆盖前一次。
这一结果证明该实际 Host 的原生许可与真实 audio-track 生命周期可用；它没有读取 deviceId/label，
因此不声称已确定实际选择的设备，更不证明 Windows 宿主的物理麦克风、声音有效或 ASR 成功。
不能再把本机麦克风全称为“没有设备”；此前只读库存确实有可连接的 WSLg RDPSource。

随后只尝试一次补充 silent AudioContext 图观察：临时 observer 计划将同一 stream 短暂连接到
source→analyser，仅读 state/currentTime/sampleRate/fftSize，关闭图后交回正式 stop。
这属于延长 stream 寿命的验收插桩，不能当成纯 Settings 的原始执行时序。
该补充没有获得可接受的 graph 成功证据：

- 只读 libpulse observer 最初可 READY；后续查询超时，新的 context 停在 AUTHORIZING（2）并超时。
  observer 未创建 capture stream；它已断开，失败原样保留。没有重启或改写全局服务来制造成功。
- 补图期间 inspector 一次 10 秒观察超时；正常退出前的一次恢复查询有回复，但 fixture collector
  没有保留该回复体。因此 `AudioContext` state、clock 推进和 API `close` 不能核验，均记为 unproved。
  不把暂时不响应直接定性为产品缺陷，也不把它扩大为永久 Pulse 不可用。
- 可归属本 App 的 Pulse source-output 创建/释放没有获证明；无重复采集请求、无绕过许可或补造 graph pass。
  此项失败与前面正式 real-track live→ended 的独立证据分开保留。

结束时向唯一私有 App PID 203704 发正常 SIGTERM，实际 App exit 0；私有 launcher 的
`cleanup.json` 为 `complete`，App、Xvfb、Openbox、session bus 与 8 个私有 bus service 均已 reaped。
App renderer PID 204061 和 metadata observer PID 214784 也已不存在。
对本 UID 的 `/proc` 环境按这次唯一私有 data/session 路径做精确匹配，残留进程为 0；
此次 loopback inspector endpoint 已关闭，最终 Host 文件 SHA-256 也已再次实读核对。
图尝试的 API close 未观察到，最终资源边界由全部 owning native processes 正常退出和私有会话回收证明；
不将进程退出替代为 API close 成功。私有数据与报告保留为证据，不纳入源码、凭据或提交。

## 官方 API 依据

- [WebKitGTK UserMediaPermissionRequest](https://webkitgtk.org/reference/webkit2gtk/2.41.3/class.UserMediaPermissionRequest.html)：未处理请求默认拒绝，公开设备类型属性。
- [WebKitGTK permission-request](https://www.webkitgtk.org/reference/webkit2gtk/stable/signal.WebView.permission-request.html)：宿主决定，支持异步持有 request，原生 GTK dialog 示例。
- [WebKitGTK enable-webrtc](https://webkitgtk.org/reference/webkit2gtk/2.40.5/property.Settings.enable-webrtc.html)：PeerConnection 设置与 media-stream 的关系。
- 目标系统 `/usr/share/gir-1.0/WebKit2-4.1.gir` 与实际 GI 查询用于核对当前安装版本；
  旧在线文档的默认值不替代本机实测。

补充源码依据：

- [WebKit 2.52.6 音频 capture source](https://github.com/WebKit/WebKit/blob/webkitgtk-2.52.6/Source/WebCore/platform/mediastream/gstreamer/GStreamerAudioCaptureSource.cpp)
- [WebKit 2.52.6 capture device manager](https://github.com/WebKit/WebKit/blob/webkitgtk-2.52.6/Source/WebCore/platform/mediastream/gstreamer/GStreamerCaptureDeviceManager.cpp)
- [WebKit 2.52.6 capturer](https://github.com/WebKit/WebKit/blob/webkitgtk-2.52.6/Source/WebCore/platform/mediastream/gstreamer/GStreamerCapturer.cpp)
- [GStreamer 1.28.2 Pulse device provider](https://github.com/GStreamer/gstreamer/blob/1.28.2/subprojects/gst-plugins-good/ext/pulse/pulsedeviceprovider.c)
