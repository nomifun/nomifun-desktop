# NomiFun 产品生态架构

本文说明 NomiFun Desktop、Mobile、小智云台、Agent 小程序和伙伴渠道如何组成
同一个产品系统。本文聚焦产品分工、通信方式与信任边界；具体操作和接口细节以文末
链接的使用指南为准。

English: [product-ecosystem.md](product-ecosystem.md)

## 四个相互关联的开源项目

| 产品 | 核心职责 | 文档入口 |
|---|---|---|
| [NomiFun Desktop](https://github.com/nomifun/nomifun-desktop) | 数据、模型、Agent、任务、工具、Skill、知识库、伙伴和小程序的本地事实源与执行中枢 | [架构总览](overview.zh.md) · [WebUI 远程访问](../guides/webui-remote-access.zh.md) · [小智接入](../guides/xiaozhi-robot.zh.md) |
| [NomiFun Mobile](https://github.com/nomifun/nomifun-mobile) | 直连已授权 Desktop 实例的 Android / iOS / H5 交互端 | [Mobile README](https://github.com/nomifun/nomifun-mobile#readme) |
| [NomiFun 小智云台](https://github.com/nomifun/nomifun-xiaozhi-yuntai) | 为 Desktop 伙伴提供语音、屏幕、运动和设备工具的 ESP32-S3 硬件端 | [固件 README](https://github.com/nomifun/nomifun-xiaozhi-yuntai#readme) · [Desktop 接入指南](../guides/xiaozhi-robot.zh.md) |
| [NomiFun Net Infra](https://github.com/nomifun/nomifun-net-infra) | 可选的自托管 NomiRelay 网络承载，把 NAT 后的 Desktop 或其他服务提供给跨网络客户端 | [产品页](https://www.nomifun.com/zh/products/net-infra/) · [门户接入文档](https://www.nomifun.com/zh/docs/guides/net-infra/) · [中继接入文档](https://github.com/nomifun/nomifun-net-infra/tree/main/docs/integration) |

## 「一个本地中枢，多个交互界面」

```text
                         可信局域网 / 已认证渠道

  NomiFun Mobile  ───────────────┐
  小智云台 ──────────────────────┤
  Agent 小程序 ──────────────────┼──▶ NomiFun Desktop
  伙伴 IM 渠道 ──────────────────┘       │
                                         ├─ 数据与会话事实源
                                         ├─ 模型与 Agent 运行时
                                         ├─ 需求、AutoWork 与 IDMM
                                         ├─ 工具、Skill、MCP 与 REST
                                         ├─ 知识库与工作上下文
                                         └─ 伙伴身份与记忆
```

Desktop 不只是又一个 UI，而是整个系统的权威中枢：它保存持久数据，解析模型和
Agent 配置，约束工具与知识库范围，并真正执行任务。其他界面不会各自再造一套配置，
也不需要复制一份主要数据集。

### Mobile 与 Desktop 局域网直连

在局域网场景中，Mobile 直接连接 Desktop 进程内的认证监听器。配对使用短时效、
一次性的二维码登录凭证：凭证 5 分钟后过期，而且成功消费后不能再次使用。认证后，
实时通信发生在手机与所选 Desktop 实例之间，**不经过 NomiFun 云中转服务器**。
因此，手机承担交互与控制职责；Desktop 仍然是数据、凭据、模型、Agent、任务和
工具的权威端。

这套设计带来三个直接收益：

- 模型密钥和持久工作区不需要复制到一个 Mobile 云账号，也不用在手机上各存一套；
- 伙伴、模型、Skill、知识库、需求或任务在 Desktop 中更新后，Mobile 看到的就是
  同一份状态；
- Desktop 所有者显式开启或关闭局域网边界，首次连接使用一次性凭证，而不是公开一个
  长期有效的 bearer URL。

局域网监听器本身不内置 TLS。请仅在可信局域网或可信专用 VPN 中使用，保持操作系统
防火墙开启，也不要把端口直接暴露到公网。完整威胁模型与操作步骤见
[WebUI 远程访问](../guides/webui-remote-access.zh.md)。

### Net Infra 是可选网络承载层，不是另一套业务后端

Mobile 无法通过可信局域网访问 Desktop 时，部署者可以自行运行 NomiRelay，并在能访问
Desktop WebUI 监听器的机器上运行 `nfagent`。Mobile 随后只连接 Relay 的业务入口，
不会持有中继管理员凭据或调用控制台 API。Relay 负责承载 HTTP/WebSocket 字节流和执行
网络策略，Desktop 仍然是应用数据与执行权威端。准确的部署和公网验收边界请以
[NomiRelay 接入文档](https://github.com/nomifun/nomifun-net-infra/tree/main/docs/integration)和
[Mobile Relay 指南](https://github.com/nomifun/nomifun-mobile/blob/main/docs/RELAY-INTEGRATION.md)
为准。

### 小智是伙伴的硬件交互界面

小智云台提供麦克风、扬声器、屏幕、舵机和设备侧工具；Desktop 提供伙伴身份、记忆、
知识库、模型、ASR、TTS、会话和工具编排。设备绑定到指定伙伴后，物理形态、桌面端和
手机端共享同一个受治理运行时，而不是另建一个割裂的机器人账号。

### 小程序是 Agent 创建的持久交互界面

Agent 小程序从普通会话中创建、预览并发布到 Desktop 管理的小程序库。Desktop 同时
管理已发布快照和受路径边界保护的工作副本；继续迭代时仍然新建可审计的普通会话，并
显式指向源码，而不是藏在小程序内部另造一套聊天系统。因此，生成式 UI 可以成为长期
可复用的工具，同时继续使用同一套本地存储边界、Agent 运行时和工具策略。

## 架构的先进性、独特性与创新性

### 一张能力图，多种入口

Desktop 伙伴、内置 Agent、受支持的 Agent CLI、Mobile、硬件、小程序、MCP/REST
调用方和 IM 渠道最终都汇入 Desktop 管理的能力。模型、Skill、知识库、需求、任务和
工具配置一次即可跨界面复用，不需要每个终端重复实现、重复配置和重复存储。

### 决策、实施与容灾可以分离

需求平台与 AutoWork 决定要认领和推进什么工作；Agent 协作把实施拆成可审计步骤；
IDMM 则作为独立的存活性与容灾层，处理决策停滞和可恢复的模型供应商故障。三类职责
可以协作，但不会被压缩成一次不可观察的模型调用。

### 硬件多模态进入统一 Agent 能力边界

语音、屏幕、运动和设备侧工具都被视为受治理的交互界面。伙伴可以在桌面 UI、Mobile、
IM 渠道和实体设备之间切换，同时保留同一份身份、记忆、知识库与能力策略。

### 知识库成为 Agent 的受治理工作上下文

知识库为 Agent 和长任务提供限定范围、可检索、可持久化的工作上下文。这里的
「COT 工作上下文」指显式的任务资料、来源、决策记录和经批准的回写产物，**不宣称记录
或暴露模型私有的隐藏思维链**。用户能检查和迁移的是明确产物，而不是不可验证的内部
推理过程。

### 伙伴的记忆、Skill 与设定可以进化和迁移

每个伙伴都有独立的人格、模型、记忆、知识库绑定、学习进度和 Skill 进化过程。伙伴
配置与记忆可以导出/导入，Skill 也可由用户明确选择加入迁移包。进化成果因此是用户
可控制、可带走的资产，而不是被困在某个托管账号中的画像。详见
[伙伴指南](../guides/companions.zh.md)。

### 渠道是网关，不是新的伙伴孤岛

超级桌面伙伴 Agent IM Channel 网关把外部聊天渠道接入同一个经过认证的伙伴与 Agent
运行时，不会为每个渠道悄悄创建新的身份、记忆孤岛或失控的能力集合。

## 产品创新里程碑

以下日期记录能力首次进入内部用户或正式产品的时间，是产品可用性里程碑，不是根据
源码提交时间倒推的日期。

### 2025 年内部已自研、实现并投入内部用户使用

1. 面向 xiaozhiAI 的原生 Computer Use 与 Browser Use。
2. 自动化需求管理平台，以及面向 Claude、Codex Agent 的持续工作 Loop。
3. 决策、实施与监督三方分离的 Agent 系统，以及智能决策容灾系统（IDMM）。
4. 硬件多模态接入伙伴。
5. 知识库作为 Agent 推理任务的持久工作上下文。
6. 伙伴记忆、Skill、设定的自进化与迁移架构。

### 2026 年初创新功能已上线

1. Agent Desktop 小程序。
2. 安全可控的客服集群系统。
3. NomiFun Mobile 直连 Desktop；局域网路径无 NomiFun 云端中转服务器。
4. NomiFun 独创的多 Agent 协作交互模式，由唯一、可审计的 `AgentExecution`
   聚合表达。
5. 超级桌面伙伴 Agent IM Channel 网关。
6. 另有多项尚未推出的保密能力。

## 相关文档

- [架构总览](overview.zh.md)
- [WebUI 远程访问](../guides/webui-remote-access.zh.md)
- [小智机器人接入](../guides/xiaozhi-robot.zh.md)
- [伙伴](../guides/companions.zh.md)
- [AutoWork 与需求](../guides/autowork-requirements.zh.md)
- [智能决策（IDMM）](../guides/intelligent-decision.zh.md)
- [Computer Use 与 Browser Use](../guides/computer-browser-use.zh.md)
