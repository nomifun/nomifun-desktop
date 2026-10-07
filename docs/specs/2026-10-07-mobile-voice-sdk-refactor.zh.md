# Mobile 语音设备 SDK 重构

2026-10-07用户最新明确授权本轮commit+push，覆盖此前Git等待限制；真实验收pending与可见GUI/安装运行/PR/发布/部署禁令保持。以下“未提交”描述是对应构建/验收执行时的历史状态。

后续共享UI已重新打包：Android实际Release/6checks、iOS当前JS/Hermes通过，新报告在Mobile/build.noindex/voice-sdk-ui-20261007092140及voice-ios-js-current-20261007-7f21c9，见status/acceptance。SDK69/patch不变，旧APK仍只证明旧源码，新目录才证明当前UI打包；native视觉/声学/Swift和两云体验仍待验。

后续RN Web批次已修复Web预算/clock/flush/close与仅voice动态theme，15项实际无头浏览器检查通过，见唯一status/acceptance。以下SDK原生/Android构建与APK仍是它们实际执行时的源码证据：SDK native patch未改，后来共享UI变更不能拿旧APK补证native包。用户已明确授权隔离无头RN Web验收，仍禁止可见GUI/Tauri/原生app和commit/push；真实设备/两云/native/Mac及用户验收仍待完成。

2026-10-07，按用户 SDK-first 决定补充当前 Mobile 全双工方案。使用现有 refactor/nomifun-mobile、RN0.86.2/Expo57.0.11，固定 `react-native-audio-api 0.13.6`（MIT）。设备 SDK 重构源码、定向行为验证及当前 SDK 的 Android Release 构建和静态产物核对已完成；Swift 原生编译、真实设备/云模型与用户体验验收仍未完成，整个 Goal 不标记 complete。

## 目标与边界

SDK 统一拥有 PCM 采集、播放、AudioContext 和设备权限，删除本任务自写的 Swift/Kotlin 设备与 DSP engines。Mobile 的 TypeScript facade 只适配既有 VoiceMediaEndpoint 与产品媒体合同；必要原生保证补在 SDK 的默认 false、voice-only 选项中，不改变普通 SDK caller 的默认行为。

这里的 SDK 是设备音频 SDK，不是模型厂商 SDK。供应商连接、长期模型密钥、Agent 与 canonical 工作仍在 Desktop；手机不增加 Agent/模型运行时，不持有厂商私钥。本次不改 UI、普通文字 hooks/API 或后端任务链。

旧八份 native bridge 源码及生产 Base64 路径已撤出，原八份 SHA 对应副本在 Desktop `build.noindex/voice-sdk-refactor-20261007/previous-voice-audio`。旧模块不再作为当前实现输入，不保留第二套设备 owner；历史构建证据继续保存。

## SDK 与 facade 的职责

| 范围 | 所有权与必需保证 |
| --- | --- |
| PCM/设备/DSP | SDK 负责实际采集、播放、context、平台语音处理与权限；facade 使用实际 rate/channel/格式，不假定设备匹配请求 |
| 身份与撤回 | facade 保留原 voice/session/activation epoch/output generation/segment revision；SDK 队列与事件也必须拒绝已经失效的采集或播放 generation |
| 输入额度 | 原生 capture→JS→服务器 disposition 的同一帧持有 duration permit。SDK sample ID 与产品 sequence 精确关联；只有对应 epoch/sequence/duration 的 InputAdmitted 或 InputReleased 归还该 permit，released 不证明模型准入 |
| 媒体预算 | 原生跨 JS 队列与 facade 均受真实音频时长和 wall-age 限制；不以 slot 数代替时长，不在 JS 阻塞时无界积压 |
| 语音处理 | 通过 SDK 的显式 voice request 启用实际平台处理；没有证据时不填 AEC 成功，音质仍需设备验证 |
| 停播与 played | flush 必须实际清除 SDK/sink 中旧 generation，反馈真实边界。Played 只源于实际 sink clock，保持 estimated/unknown 与 uncertain tail；不同 SDK context/sink 的 timeline 不合并成连续消费。scheduled、软件出队、JS 时间或普通完成 callback 都不证明已听 |
| 生命周期 | 后台/锁屏/来电/interrupt/route/权限变化原生停设备并撤活动 lease；前台或系统恢复不得自动开 mic，必须保人工恢复与原绑定规则 |
| close | bounded shutdown 必须等待真实 SDK 清理结果；不能确认时报告 unknown/失败，不能仅因 Promise 返回或删除 JS listener 声称已 join |

不得为了使用 SDK 缩减这些媒体合同，也不得另写一套设备/DSP engine。SDK 原生补丁只补合同所需的薄保证，保持默认关闭；网络与工作 authority 不进入 SDK。采集暂停、停播、结束 voice 和取消工作仍是独立动作。

## 当前进展与证据

已完成统一 TypeScript facade 和 SDK 接线，撤出旧 bridge/Base64 生产路径；没有新增设备/DSP engine。SDK 的显式 voice-only 补丁覆盖原生 capture→conversion→CallInvoker/JS→server disposition 的同一 duration permit、硬件 admission age、过期锁存与迟到 ACK 拒绝、旧 session/ID 过滤，以及独立原生 age monitor。后台/中断/路由撤销媒体 lease，前台返回不得自动启动。平台语音处理、sink timestamp、队列 flush 和清理结果通过 SDK 提供；默认 false/零预算的普通 SDK caller 保留原行为，产品工作 authority 不进入 SDK。

补丁通过 Mobile `package.json.patchedDependencies` 和 `patches/react-native-audio-api@0.13.6.patch` 持久化，SHA256 为 `d4f2119bead586f8371f2c329b473a4b9a87c7da82e5d43163bfae0af939b1ef`。当前包证据比对253份 Mobile src，以及相对官方发布包、排除仅 CRLF 差异后的69份 SDK 变更（57份非 iOS、12份 iOS）与隔离构建副本；补丁、锁文件和 source freeze 均匹配。它证明源码身份，不证明 iOS native 已编译。

| 当前 SDK 本地证据 | 实际结果与边界 |
| --- | --- |
| TypeScript/媒体行为 | audio-api endpoint 17项/89 assertions；Mobile 合计50项/259 assertions，typecheck 与 i18n 通过。测试覆盖原合同及失效分支，不替代真实音频设备 |
| SDK 原生 header | MSVC 实际编译运行13组/71 checks，覆盖额度、ACK、会话隔离、耗龄与原生监测；没有 RN 调度或硬件体验证明 |
| Android 当前 SDK | 隔离副本 `C:\nomi-v-dc8670c5` 的 `:app:assembleRelease`（59455）exit0，4m49s、576 tasks（510 executed、66 up-to-date） |
| 静态产物 | 6项检查通过：源码/补丁身份、当前构建终态、真实 DEX class_defs 中有 AudioAPIModule 且无旧 NomiFunVoiceAudioModule、生成的 RN PackageList 注册 AudioAPIPackage、实际 AArch64 SDK shared library、APK Hermes 与生成 bundle SHA 相同 |
| 最终包配置 | aapt2 确认 RECORD_AUDIO、MODIFY_AUDIO_SETTINGS 与既有 usesCleartextTraffic=true；无 foreground service 权限/服务或 debuggable=true。zipalign 16KiB 与29个 ELF 的 PT_LOAD 16KiB 静态检查通过；不证明真机页大小或音频体验 |
| H5/iOS JS | 当前 SDK H5 export 50 routes；iOS JS export 2135 modules、5792310字节 Hermes。分别保存在隔离副本 sdk-web-export、sdk-ios-export；iOS JS 不等于 Swift/Xcode 原生构建 |

当前 SDK APK 在 Mobile `build.noindex/voice-sdk-verified-20261007/nomifun-mobile-sdk-release-test-arm64.apk`，59974745字节，SHA256 `c75166268c7b287cc6a4bb0bab1dbf20a1fc980e6d29a4e7d8e06c4237bfd08d`。同目录保留 `sdk-artifact-evidence.json`、Gradle日志、最终 Manifest、zip/ELF alignment 和 `voice-policy-result.json`。Release variant 仍用 Expo 模板 TEST 签名，未发布或安装；正式签名/分发不属于本任务完成条件，未来需要时另获授权。

真实构建暴露的兼容问题已在同一持久补丁中修复：RN CXX flags 的语种适用范围与对象 ctor 次序、未启用 FFmpeg 的 guard、vendor release assertion 对应的 unused 值显式 `(void)`。仅构建进程 PATH 提供 Git Bash 给 SDK 脚本，未修改系统 PATH。没有关闭 `-Werror`、降级 RN/Expo/SDK 或读取签名密钥。重构前 Debug/Release 包及旧八份 source SHA 仍作为历史证据保留，不能认证新 SDK。

## 收口条件

源码与本地证据已经覆盖 SDK 接线、格式、跨 JS/server ACK duration-age 预算、旧 ID/epoch/generation 迟到过滤、flush/sink clock 合同和未知清理分支；真实设备仍必须证明输入/输出、前后台/中断/路由停设备、不自动录音、声学效果和清理结果。仅 interface、fake、类型检查、SDK 成功安装或静态 APK 都不能补证真实体验。

定向行为检查、Android 当前 SDK 源码构建与包内证据已完成。Swift/macOS 编译、Android/iOS/H5 真设备生命周期、两供应商权限、LAN/TLS、视觉/声学与并发性能仍须对应外验。GUI/app 安装运行禁令持续；无 commit/push/PR/发布，正式签名与分发也不自动授权。整个 Goal 与用户验收继续未完成。

当前 Prompt、Mobile 实施方案、status、acceptance 与 Mobile docs/VOICE.md 共同引用本补充；具体新结果由真实执行后更新，不把计划写成已通过。
