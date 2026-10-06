# 模型管理、路由与故障转移

NomiFun 的**模型**页面是一套可扩展控制面，不是固定厂商清单。它把 provider
凭据、模型记录、任务能力与可靠性策略分开管理，让同一份目录可以被会话、伙伴、
计划任务、设定和创作复用。

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
私有 endpoint，或 Ollama、vLLM 等本地/自托管服务。不同任务可以配置各自的调用
接口；健康请求成功不等于所有媒体或技术操作都兼容。

每个模型的基本配置步骤：

1. 选择或创建 provider；
2. 填写该 provider 需要的 endpoint 与凭据；
3. 搜索 provider 返回的完整模型目录，或直接填写精确模型 ID；
4. 确认调用用途：ASR、TTS 等专用入口已带入对应用途；通用入口中的未知模型只需在输入 ID 后确认一次；
5. 对话模型可在配置首页的“上下文与输出”区域并排设置上下文窗口与最大输出上限，下方设置自动压缩阈值；未覆盖的值继续使用供应商/模型默认值；
6. 保存，并运行当前界面提供的健康/状态检查。

provider 凭据保存在本地配置中。任何云端 provider 仍会按自己的计费和数据政策
处理发送给它的内容。

对话、语音识别、语音合成及其他细分场景的模型行都提供**编辑模型**，可以在当前
页面修改并保存。编辑器只展示当前用途的调用接口，保存时保留其他用途的最新
配置。模型别名和描述由各用途共用；凭据和模型的完整调用接口集合仍可在
**供应商与密钥**中管理。

## 模型目录与调用接口

模型目录提供建议，不限制可录入的模型，也不要求先选择“支持的任务”。目录没有
匹配项或加载失败时，仍可手工填写模型 ID。ASR、TTS 等专用入口会带入调用用途；
通用入口无法核实任务的模型在填写 ID 后确认一次用途，不会被自动当作对话模型。
只有原生 API 明确声明的任务，或精确官方文档 profile，才会自动建议独立接口；
原生目录中的未来模型 ID 无需先进入内置名称清单。
仅根据模型名称推断的任务信息不会自动创建接口。需要不同协议、地址或凭据的任务，
可以在模型配置中通过“添加调用接口”分别配置。

手工修改模型 ID 会清除系统为原模型自动建议的接口，保留用户明确配置及已保存的
接口。目录声明的用途与当前用途冲突时，需要明确确认，当前用途在确认前保留；
后台刷新目录也不会替换已有配置。

托管模型目录可以表达这些任务族：

| 任务族 | 常见使用面 |
| --- | --- |
| Chat / Agent 回合 | 会话、伙伴、设定、计划任务、Canvas Assistant |
| Realtime | provider 支持的低延迟交互 |
| Vision | 带图片的聊天与分析 |
| 语音识别（ASR） | 语音输入、伙伴和设备语音 |
| 语音合成（TTS） | 伙伴、设备与 Canvas 音频节点 |
| 图片生成 / 编辑 | 创作 Canvas 与 Image Workbench |
| 视频生成 | 创作 Canvas 与 Video Workbench |
| 音乐生成 | 会话创作与 Creation |
| Embedding / Rerank | 检索与知识工作流 |

这些任务表示独立调用协议与 endpoint。运行时不会只凭模型名猜测图片或视频生成
接口，也不会静默使用另一个 provider 的同名模型。

保存时，后端检查所选协议是否支持该用途；实际调用只读取精确任务的已保存接口。
语音识别、语音合成或其他独立任务缺少接口时会明确报错，不会向 Chat HTTP 地址
回退发送请求。

Chat 不需要用户勾选识图、视频理解、音频输入、工具调用、推理、流式传输或模型
内置联网搜索。目录中的能力信息只作参考，旧配置缺少某个 trait 也不会因此关闭
模型能力。输入内容与请求选项由已注册协议能够表达的格式和实际 provider 响应
决定；某个协议没有对应 serializer 时，配置页面不能承诺可用。模型内置联网搜索
同样需要对应协议实现，不能因删除勾选项而自动获得所有供应商的搜索接口。

工具调用、推理和流式传输初始按支持处理，运行时只有在
400/422 的完整 provider 错误对象用机器字段明确指出不支持对应参数/能力时，才会记录
负向观察。鉴权失败、权限失败、限流、额度不足、超时、网络故障、5xx 与自然语言错误
文案都不会降级能力。

负向观察同时写入 capability 的持久化健康数据与进程内路由缓存。后续路由会移除已确认
不支持的工具调用/推理能力；流式被确认不支持后改走有界的单次 JSON 响应。修改模型的
调用配置或连接会清除旧观察，以便新配置重新从乐观状态验证。Realtime 始终是独立的
`realtime_conversation` 任务与协议，不再作为 Chat trait。

## 会话内能力路由

通用 Agent 预置了图像生成、图像编辑、视频生成、语音合成和音乐生成 Action。
普通会话遇到明确的即时创作请求时，优先调用对应任务的用户默认模型：

| 会话需求 | 默认键 | 必需能力 |
| --- | --- | --- |
| 看图/识图 | `models.default.vision` | 能表达图片输入的 Chat 协议，且没有已确认的“工具调用不支持”观察 |
| 生成图片 | `models.default.imageGeneration` | `image_generation` |
| 编辑图片 | `models.default.imageEdit` | `image_edit` |
| 生成视频 | `models.default.videoGeneration` | `video_generation` |
| 生成音乐 | `models.default.musicGeneration` | `music_generation` |
| 语音合成 | `models.default.speechSynthesis` | `speech_synthesis` |

没有设置默认值时，自动媒体 Action 从已启用且已配置对应任务调用接口的模型中选择，
优先使用健康检查通过的能力，再按模型管理中的提供方和模型顺序排序。没有兼容模型时
才要求用户配置；已设置但失效的默认模型不会被悄悄替换。专业创作界面仍可为一次显式
任务单独选择其它兼容模型。

视觉模型作为条件 Chat 候选冻结进新会话：只有当前请求实际包含图片时才参与路由，不会在普通文字
对话失败后冒充通用故障转移模型。主模型的 Chat 协议可以表达图片输入时，优先使用
主模型；目录或旧 trait 的缺失不会迫使它改走另一个模型，实际支持情况仍由 provider
响应决定。

模型目录与调用配置是两层：多模态 Chat 输入由协议格式和实际调用结果决定；创作任务
仍需要保存对应任务的调用接口。工具调用、推理与流式默认可用，但确定性负向观察会
收窄后续路由。Chat 的输入与技术能力不会自动创建生图、视频、音乐、TTS 或 ASR
接口；自动创作只会选择已配置对应任务接口的模型。

创作会把精确的 `{ providerId, model, task, capability }` 身份随每次已接纳
的媒体操作持久化。复用同一个幂等任务重试时，不能更换这些事实。

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
- 创作模型目录：
  `ui/src/renderer/pages/creativeStudio/models/catalog.ts`
