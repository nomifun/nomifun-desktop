# 模型管理、路由与故障转移

NomiFun 的**模型**页面是一套可扩展控制面，不是固定厂商清单。它把 provider
凭据、模型记录、任务能力与可靠性策略分开管理，让同一份目录可以被会话、伙伴、
计划任务、设定和创意工坊复用。

> English: [model-routing.md](model-routing.md)

## 目录管理什么

打开**模型**（`/models`）可以管理：

- provider endpoint、协议、鉴权和 provider 级参数；
- 模型名、启用状态、上下文窗口、输出上限和任务能力；
- 当前版本支持的本地语音识别模型；
- 全局默认值、IDMM 设置和有序模型故障转移队列。

执行引擎是另一个维度。Nomi、Claude Code、Codex、OpenCode、OpenClaw 等回答
“谁来执行工作”；模型管理回答“这项工作使用哪个模型和能力”。

## 接入云端、兼容协议与自托管服务

原生后端包括 Anthropic、OpenAI-compatible、Amazon Bedrock 与 Google Vertex。
provider 目录还为多种服务提供预设和协议 profile。

OpenAI-compatible 或其它已登记协议可以使用自定义 base URL，连接云端网关、
私有 endpoint，或 Ollama、vLLM 等本地/自托管服务。只登记 endpoint 真实支持的
能力；健康请求成功不等于所有媒体、工具调用或流式操作都兼容。

每个模型的基本配置步骤：

1. 选择或创建 provider；
2. 填写该 provider 需要的 endpoint 与凭据；
3. 添加精确模型名；
4. 当上游默认值缺失或不准确时，覆写上下文窗口或输出上限；
5. 只启用 provider/协议合同真实支持的任务；
6. 保存，并运行当前界面提供的健康/状态检查。

provider 凭据保存在本地配置中。任何云端 provider 仍会按自己的计费和数据政策
处理发送给它的内容。

## 按任务声明能力

托管模型目录可以表达这些任务族：

| 任务族 | 常见使用面 |
| --- | --- |
| Chat / Agent 回合 | 会话、伙伴、设定、计划任务、Canvas Assistant |
| Realtime | provider 支持的低延迟交互 |
| Vision | 带图片的聊天与分析 |
| 语音识别（ASR） | 语音输入、伙伴和设备语音 |
| 语音合成（TTS） | 伙伴、设备与 Canvas 音频节点 |
| 图片生成 / 编辑 | 创意工坊 Canvas 与 Image Workbench |
| 视频生成 | 创意工坊 Canvas 与 Video Workbench |
| Embedding / Rerank | 检索与知识工作流 |

任务选择是显式的。运行时不会只凭模型名猜测图片或视频能力，也不会静默使用
另一个 provider 的同名模型。

创意工坊会把精确的 `{ providerId, model, task, capability }` 身份随每次已接纳
的媒体操作持久化。复用同一个幂等任务重试时，不能更换这些事实。

## NomiFun 免费模型

在当前版本提供时，**NomiFun 免费模型**使用内置托管 provider。无需先创建自定义
provider 或填写自己的 API Key，即可启用服务、刷新模型目录、执行健康检查并激活
可用模型。

这些仍属于在线第三方推理服务。可用性、额度、延迟与数据处理条款可能变化；
发送敏感内容前请阅读产品内提示。

## 模型故障转移队列

故障转移功能是一条有序的可靠性队列，不是多凭据轮询池。

它会：

- 把全局默认队列存储在 `agent.model_failover`；
- 允许单个会话通过 `extra.model_failover` 覆盖；
- 在该会话启用故障转移时被 IDMM 故障值守使用；
- 不会在 API Key 之间分摊负载。

常见队列：

```text
主模型 -> 便宜备用模型 -> 更强备用模型 -> 人工检查
```

当前运行时允许整条队列最多切换四次。如果所有 provider 都不可用、所需任务没有
被支持，或 prompt/tool 状态本身无效，故障转移也无法让这一轮成功。

## 与 IDMM、AutoWork 的关系

IDMM 有独立的故障值守与决策停滞值守。模型故障转移属于故障侧：当 provider
故障被判定为可恢复、且会话启用了故障转移时，IDMM 可以让会话运行时按配置队列
重试。

AutoWork 位于更上一层：它负责让带标签的需求队列继续认领和推进，而 IDMM 与
模型故障转移负责尽量让每个已认领回合活下来。

外部 ACP/CLI Agent 不参与 Nomi 引擎故障转移队列；它们的 provider 调用发生在
各自运行时内部。

## 真相来源

- provider 与模型设置 UI：
  `ui/src/renderer/pages/modelHub/`
- 共享模型存储类型：
  `ui/src/common/config/storage.ts`
- 模型故障转移：
  `crates/backend/nomifun-conversation/src/model_failover.rs`
- 故障转移 API：
  `crates/backend/nomifun-app/src/router/model_failover.rs`
- IDMM 策略：
  `crates/backend/nomifun-idmm/src/policy.rs`
- 创意工坊模型目录：
  `ui/src/renderer/pages/creativeStudio/models/catalog.ts`
