# 模型供应商目录核查（2026-10-06）

本次核查覆盖模型配置页的全部 **42 个供应商预设**。模型 ID 输入与供应商目录应同时可用：目录提供当前接口返回的候选或明确标记的官方文档建议；目录缺失、为空或请求失败均不构成模型 ID 白名单。

核查入口为 `ui/src/renderer/utils/model/modelPlatforms.ts`、`nomifun-model-invoke` 的 `PRESETS` 与 `nomifun-system/src/model_fetcher`。本报告记录当前接口、证据和修复结果，不新增另一份模型或能力定义。现行配置说明见 [模型配置指南](../guides/model-routing.zh.md)。

## 证据口径

- **公开实取**：本轮向官方模型目录发出不带用户 Key 的只读 GET，获得模型响应。该结果不证明具体账号具有模型调用权限。
- **官方接口**：已核对官方目录路径、鉴权和响应合同，运行时从供应商接口读取；本轮未进行真实账号鉴权验收。
- **官方文档建议**：供应商或订阅通道未公布可用的推理 Key 目录合同，展示本轮核对的文档模型 ID，返回 `catalog_source=official_documentation`，不能声称刷新了账号实时目录。
- **配置决定**：自定义网关或未明确公布目录合同的通道按配置地址尝试兼容发现；是否支持列表由该服务决定。

本轮未使用用户 API Key，未发送计费推理请求，也未修改供应商余额、订阅或配置。除下表明确标记的公开目录实取外，文档核对、源码修复与本地模拟响应测试不等同于真实供应商鉴权验收。

## 全部预设核查矩阵

路径均指目录请求或供应商文档建议来源。静态建议行保留当前预设的调用 Base URL，以区分按量、订阅和地域通道；不因目录失败切换这些通道。

### 通用与国际供应商（13 个）

| 预设值 | 目录请求、鉴权或通道 | 证据与结论 |
| --- | --- | --- |
| `custom` | 用户 Base URL + `/models`，使用所配鉴权 | 配置决定；不套用官方域名或要求返回固定模型集合 |
| `new-api` | 用户 Base URL 规范化到 `/v1/models`，使用所配鉴权 | 配置决定；[官方路由源码](https://github.com/QuantumNous/new-api/blob/main/router/relay-router.go)提供兼容 Models 路由 |
| `gemini` | `GET https://generativelanguage.googleapis.com/v1beta/models`，`x-goog-api-key` | [官方接口](https://ai.google.dev/api/models)；本轮修复 `nextPageToken` 分页 |
| `Agnes` | `GET https://apihub.agnes-ai.com/v1/models`，Bearer | 配置决定；[官方接入说明](https://wiki.agnes-ai.com/en/docs/overview)确认 Base URL、鉴权与 OpenAI 兼容性，未找到独立列表合同；无 Key 实取返回 401，未声明鉴权成功 |
| `OpenAI` | `GET https://api.openai.com/v1/models`，Bearer | [官方接口](https://platform.openai.com/docs/api-reference/models/object?lang=curl)；当前地址与 `data[].id` 解析匹配 |
| `Anthropic` | `GET https://api.anthropic.com/v1/models`，`x-api-key` + `anthropic-version` | [官方接口](https://platform.claude.com/docs/en/api/http/models/list)；本轮修复 `has_more` / `last_id` 分页 |
| `AWS-Bedrock` | 按所配 AWS 区域、凭据调用 SDK `ListFoundationModels` 与分页 `ListInferenceProfiles` | [官方接口](https://docs.aws.amazon.com/us_en/bedrock/latest/userguide/models-get-info.html)；保持 Anthropic Claude 协议族边界，不把其他模型族宣称为可调用的 Claude |
| `DeepSeek` | `GET https://api.deepseek.com/v1/models`，Bearer | [官方接口](https://api-docs.deepseek.com/api/list-models/)；直接消费目录 ID，不以旧别名替换新目录 |
| `Deepgram` | `GET https://api.deepgram.com/v1/models`；目录可公开读取，调用鉴权为 Token | [官方接口](https://developers.deepgram.com/guides/fundamentals/model-metadata)；公开实取 200，原始 `stt` 455 项、`tts` 102 项；按 `canonical_name` 去重并保留两个任务来源 |
| `Novita` | `GET https://api.novita.ai/openai/v1/models`，Bearer | [官方接口](https://docs.novita.ai/api-reference/model-apis-llm-list-models)；本轮补充 `title` / `context_size` 可选元数据解析 |
| `OpenRouter` | `GET https://openrouter.ai/api/v1/models`，目录可公开读取 | [官方接口](https://openrouter.ai/docs/api/api-reference/models/get-models)；公开实取 200，`data` 464 项 |
| `xAI` | `GET https://api.x.ai/v1/language-models`、`image-generation-models`、`video-generation-models`，Bearer | [官方接口](https://docs.x.ai/developers/rest-api-reference/inference/models)；按目录保留模态，STT/TTS 以明确服务配置项表示 |
| `Poe` | `GET https://api.poe.com/v1/models`，目录可公开读取 | [官方接口](https://creator.poe.com/api-reference/listModels)；公开实取 200，`data` 341 项 |

### 中国供应商与订阅通道（21 个）

| 预设值 | 目录请求、鉴权或通道 | 证据与结论 |
| --- | --- | --- |
| `MiMo` | `GET https://api.xiaomimimo.com/v1/models`，Bearer 或供应商 `api-key` | [官方接口](https://mimo.mi.com/docs/zh-CN/api/model/list-models)；本轮从固定建议改为实时目录 |
| `MiMo-Token-Plan-CN` | `https://token-plan-cn.xiaomimimo.com/v1`，计划 Key | [Token Plan 文档](https://mimo.mi.com/docs/zh-CN/tokenplan)建议 8 项，包含 2.6 Pro/Flash；与标准按量目录分开，三地区按同一计划文档核对 |
| `MiMo-Token-Plan-SGP` | `https://token-plan-sgp.xiaomimimo.com/v1`，计划 Key | 官方文档建议；保留新加坡通道 |
| `MiMo-Token-Plan-AMS` | `https://token-plan-ams.xiaomimimo.com/v1`，计划 Key | 官方文档建议；保留阿姆斯特丹通道 |
| `MiniMax` | `GET https://api.minimaxi.com/v1/models`，Bearer | [中国站官方目录接口](https://platform.minimax.cn/docs/api-reference/models/openai/list-models)；本轮标准中国站改为实时目录 |
| `MiniMax-Code` | `GET https://api.minimax.io/v1/models`，Bearer | [国际站官方目录接口](https://platform.minimax.io/docs/api-reference/models/openai/list-models)；本轮国际站改为实时目录 |
| `MiniMax-Coding-Plan` | `https://api.minimaxi.com/v1`，中国计划 Key | [官方 API 概览](https://platform.minimaxi.com/docs/api-reference/api-overview)；官方文档建议，未把标准目录合同推断为订阅授权目录 |
| `Dashscope` | 官方原生 `GET https://dashscope.aliyuncs.com/api/v1/models`，Bearer，分页 `output.models[].model` | [官方目录接口](https://help.aliyun.com/zh/model-studio/list-models)；本轮替换默认 `/compatible-mode/v1/models` 发现路径，自定义网关仍按配置发现 |
| `Dashscope-Coding` | `https://coding.dashscope.aliyuncs.com/v1`，计划 Key | [官方 Coding Plan](https://help.aliyun.com/zh/model-studio/coding-plan)；官方文档建议 10 项，不执行合成 Chat 请求来拉列表 |
| `Zhipu` | `https://open.bigmodel.cn/api/paas/v4`，Bearer | [官方模型概览](https://docs.bigmodel.cn/cn/guide/start/model-overview)建议 35 项，本轮补齐 GLM 5.3 系列，不声称存在 `GET /models` |
| `GLM-Coding-Plan` | `https://open.bigmodel.cn/api/coding/paas/v4`，计划 Key | [Coding Plan 概览](https://docs.bigmodel.cn/cn/coding-plan/overview)建议；本轮核对主线为 `glm-5.3`、`glm-5.3-flash`，不把标准付费 `FlashX` 混入计划建议 |
| `Moonshot` | `GET https://api.moonshot.cn/v1/models`，Bearer | [中国站官方目录接口](https://platform.kimi.com/docs/api/list-models)；官方接口，中国站目录独立 |
| `Moonshot-Global` | `GET https://api.moonshot.ai/v1/models`，Bearer | [国际站官方目录接口](https://platform.kimi.ai/docs/api/list-models)；官方接口，国际站目录独立 |
| `Ark` | 当前兼容尝试 `GET https://ark.cn-beijing.volces.com/api/v3/models`，Bearer | 配置决定；[官方 Ark 文档](https://www.volcengine.com/docs/82379)未证明该推理根下的 Bearer Models REST 合同；不能把控制面 HMAC 模型 API 混同为该接口 |
| `Ark-Coding-Plan` | `https://ark.cn-beijing.volces.com/api/coding/v3`，计划 Key | [官方 ZCode 模型说明](https://docs.volcengine.com/docs/ark/coding-plan-personal-ai-zcode?lang=zh)建议 15 项；[控制面目录 API](https://docs.volcengine.com/docs/ark/list-ark-coding-plan-model-api?lang=zh)使用不同鉴权，未混作推理 Key 的 Bearer Models 合同 |
| `Ark-Agent-Plan` | `https://ark.cn-beijing.volces.com/api/plan/v3`，计划 Key | [官方 ZCode 模型说明](https://docs.volcengine.com/docs/ark/agent-plan-personal-zcode?lang=zh)独立核对为 15 项建议；[控制面目录 API](https://docs.volcengine.com/docs/ark/list-ark-agent-plan-model-api?lang=zh)使用不同鉴权；精确官方根改用建议来源，自定义网关不继承该判断 |
| `Qianfan` | `GET https://qianfan.baidubce.com/v2/models`，Bearer | [官方 API 概览](https://cloud.baidu.com/doc/qianfan-api/s/Dmba8k71y)；官方接口 |
| `Qianfan-Coding-Plan` | `https://qianfan.baidubce.com/v2/coding`，旧 Coding Plan Key | [旧 Coding Plan 文档](https://cloud.baidu.com/doc/qianfan/s/imlg0beiu)建议 8 项；产品停止续订也不意味着可静默迁往不同 Token Plan 根或计费产品 |
| `Hunyuan` | `GET https://tokenhub.tencentmaas.com/v1/models`，Bearer | [官方目录接口](https://cloud.tencent.com/document/product/1823/130078)；官方接口，中国站独立 |
| `Hunyuan-Global` | `GET https://tokenhub-intl.tencentmaas.com/v1/models`，Bearer | [官方目录接口](https://cloud.tencent.com/document/product/1823/130078)；官方接口，国际站独立 |
| `Lingyi` | `GET https://api.lingyiwanwu.com/v1/models`，Bearer | [官方 API 文档](https://platform.lingyiwanwu.com/docs/api-reference)；已核对实时目录合同，未进行用户账号调用验收 |

### 聚合服务（6 个）

| 预设值 | 目录请求、鉴权或通道 | 证据与结论 |
| --- | --- | --- |
| `SiliconFlow-CN` | `GET https://api.siliconflow.cn/v1/models`，Bearer | [官方接口](https://api-docs.siliconflow.cn/docs/api/models-get)；不传 `type` / `sub_type` 以保留完整目录 |
| `SiliconFlow` | `GET https://api.siliconflow.com/v1/models`，Bearer | [官方接口](https://docs.siliconflow.com/en/api-reference/models/get-model-list)；与中国站使用不同配置地址 |
| `PPIO` | `GET https://api.ppio.com/openai/v1/models`，Bearer | [官方接口](https://ppio.com/docs/models/reference-llm-list-models)；默认路径正确；本轮补齐文档要求的 JSON 请求头与可选元数据 |
| `ModelScope` | `GET https://api-inference.modelscope.cn/v1/models`，目录可公开读取 | [官方实时目录](https://api-inference.modelscope.cn/v1/models)公开实取 200，返回 `object=list` / `data[].id`；输入推理 Key 前也可以预览目录 |
| `InfiniAI` | `GET https://cloud.infini-ai.com/maas/v1/models`，Bearer 通用 `sk-` Key | [官方集成说明](https://docs.infini-ai.com/shared/gen-studio/coding-tools/gs-use-kimi-code.html)；本轮支持可选 `max_output_length`；`0` 为未公开/不适用，不作为模型限额 |
| `Ctyun` | `GET https://ai.ctaigw.cn/v1/models`，Bearer AppKey | [官方接口](https://www.ctyun.cn/document/11061839/11062357)；默认路径与 `data[].id` 响应匹配 |

### StepFun（2 个）

| 预设值 | 目录请求、鉴权或通道 | 证据与结论 |
| --- | --- | --- |
| `StepFun` | `GET https://api.stepfun.com/v1/models`，Bearer | [官方目录接口](https://platform.stepfun.com/docs/zh/api-reference/models/list)；优先真实目录并保留新 ID；仅精确官方标准根的空目录或可用性失败允许标明文档建议来源，400/401/403 保持实际错误 |
| `StepFun-Plan` | `https://api.stepfun.com/step_plan/v1`，计划 Key | [官方 Step Plan 概览](https://platform.stepfun.com/docs/zh/step-plan/overview)建议 10 项，包含 `step-5-preview`；未确认订阅 Models 合同，不向标准按量根转发计划 Key |

## 已定位的问题与本轮修复

| 问题 | 对用户的影响 | 修复结果 |
| --- | --- | --- |
| 将所有通道当成 OpenAI `GET /models` | 部分订阅网关返回 400/404/405，用户看不到候选 | 按供应商与精确官方通道选择实时接口或文档建议，返回有类型的来源信息 |
| 旧 StepFun 配置仍保存 `platform=stepfun`，Base URL 已是 Step Plan | 按标准版 fetcher 请求计划根，或触发跨通道修正探测 | 精确识别官方 Step Plan 根并使用计划建议；保留用户实际调用 Base URL，不改变计费通道 |
| 标准供应商身份与已配置的订阅根不一致 | 标签仍是普通供应商，模型发现错误地套用按量目录合同 | MiMo、Dashscope、Zhipu、Qianfan、Ark 的精确官方订阅根按实际通道发现；自定义域名不套用该判断 |
| 固定目录被当成成功刷新结果 | 用户误以为目录实时、账号已获授权，最新模型缺席 | 文档建议明确显示来源；标准 MiMo、MiniMax 改用官方实时目录；未在列表的 ID 仍可输入 |
| Anthropic、Gemini 忽略分页 | 只显示首批模型，刷新也找不到后续项目 | 遍历官方游标；游标缺失或重复、后页失败均使整次发现失败，不返回伪完整结果 |
| Dashscope 标准版使用兼容调用根拉模型 | 模型推理可用但发现路径不正确 | 官方默认根使用原生分页目录与 `output.models[].model`；自定义网关保持兼容发现 |
| 公开目录也必须先填写调用 Key | OpenRouter、Poe、Deepgram、ModelScope 的目录预览无谓受阻 | 公开官方目录预览与保存、推理鉴权分开；自定义地址不继承官方无鉴权策略 |
| 通用响应 parser 只认识少量 Token 字段 | Novita/PPIO 名称和上下文、InfiniAI 最大输出丢失 | 消费供应商返回的可选 `title`、`context_size`、`max_output_length` 等声明；未知或无效值保持未知 |
| 刷新结果不展开列表，失败提示过于笼统 | 用户感到按钮没有反应，400 被误导为网络/DNS 问题 | 共享编辑器提供展开候选、加载、结果数量、空列表和失败反馈；手动 ID 输入始终可用 |
| 未知或手填模型自动被当作 Chat | ASR、TTS 等模型可能被发往错误任务接口 | 专用入口携带 `initialTask`，通用入口在 ID 输入后只确认一次调用用途；保存和调用均检查精确任务与协议，缺少匹配接口时不发出 Chat 兜底请求 |

“400 Bad Request”只能证明某次目录请求被供应商拒绝。用户确认是 StepFun Coding Plan，但未提供实际 Base URL；本轮不能将其供应商侧原因断言为 Key、套餐、模型权限或某个具体 URL。修复覆盖已证明的通道分派与交互反馈缺陷，仍保留实际错误供排查。

调用用途与模型目录分开：完整目录不按用途筛选，ASR、TTS 等专用入口带入用途；通用入口对未核实用途的 ID 提供一次确认。自动建议接口可消费原生 API 明确声明的任务（`ProviderDeclared`），例如 Deepgram 的 STT/TTS 来源分组，不要求未来 ID 出现在已知名称清单；另一来源是精确官方文档 profile（`OfficialDoc`），只有匹配已核验的精确 ID 与通道时才生成该来源。`Inferred` 或没有来源的信息不自动创建接口。手工改 ID 时清除自动接口，保留用户配置与已保存接口；可靠目录用途与当前用途冲突需要明确确认，确认前保留当前用途。后端保存检查协议 `supported_tasks`，运行时检查精确任务行与协议匹配，在错误任务配置触发任何 HTTP 调用前明确失败。

## 验证边界

本轮公开 GET 证据为 OpenRouter、Poe、Deepgram、ModelScope；数量是 2026-10-06 的响应快照，不能固化为测试白名单。其余预设的官方鉴权接口由文档合同与本地 HTTP 模拟响应核对，不宣称 42 个供应商均已通过真实账号验收。

本轮集成验证已完成：

| 验证范围 | 结果与覆盖 |
| --- | --- |
| `cargo test -p nomifun-system --lib model_fetcher` | 79 项通过，覆盖供应商目录、分页、地域与订阅分派、公开目录、元数据解析；Anthropic/Gemini 分页共享整次发现的 30 秒请求预算 |
| `cargo test -p nomifun-system --test model_fetch_routes` | 36 项通过；合计 115 项 Rust 目录与路由测试通过 |
| `nomifun-api-types` 的 `provider::tests` 与 `ts_export` | 分别 7 项、3 项通过；合计 10 项 API 合同与生成验证通过，生成绑定已核对 |
| UI 定向测试 | 5 个测试文件、54 项通过；包含 11 项目录 hook 测试，覆盖无 Key 手动发现、目录来源、避免自动鉴权报错及不同匿名账号的缓存隔离 |
| UI 类型、规则与差异检查 | 类型检查无错误，桌面 880px 边界、国际化与 `git diff --check` 通过 |
| 桌面浏览器验收 | 880×600 下操作实际共享编辑器：刷新自动展开完整的 4 项测试目录，当前 ID 保留，选择新模型、保存未列出的手填 ID 均通过；页面使用测试数据 |
| 报告预设覆盖 | 与 `MODEL_PLATFORMS` 逐项比对：42 项、42 个唯一值，无遗漏或额外项；无行尾空白 |

上述本地 HTTP 模拟响应与浏览器测试不代替真实供应商账号验收。本轮未使用用户 Key，未执行计费推理请求。

旧 `vertex-ai` / `gemini-vertex-ai` 别名不属于当前 42 个预设。它们已明确退休；旧配置将 Gemini 模型身份与 Anthropic Publisher 协议混用，因此返回明确不支持，而不是再造兼容列表。

## 调用用途防护验收

未知模型不再建立默认 Chat 接口；专用入口携带用途，通用入口确认用途后保存。模型目录另提供可选的 `tasks_source`，区分原生任务声明、精确官方文档映射与推测；模型 ID 的实时来源不会将名称推测升级为已确认任务。

本轮补充验收通过：164 项 UI 定向测试，覆盖用途传递、普通/专用入口、未知模型、可信目录自动采用、手填与延迟目录冲突确认、输出设置及完整目录交互；83 项目录单元测试和 38 项目录路由测试通过。模型调用服务的 29 项测试与模型配置路由的 11 项测试通过，其中验证 Chat-only 配置对 8 类媒体任务均在 HTTP 前拒绝、同一 ID 的 ASR/TTS 分别使用音频接口，以及 11 组任务/协议错配在新建或覆盖前拒绝。模型任务与 DTO 来源测试、TypeScript 绑定生成、类型检查、桌面 UI 边界和国际化检查通过。

880×600 浏览器使用共享编辑器与测试数据验证：未知 ID 保持无用途并禁止保存；原生 ASR 信息能够自动建立语音识别接口，保存载荷为 `speech_recognition` / `openai.audio_transcriptions`。本轮没有使用真实账号执行供应商推理，也没有迁移或重写已有模型配置。

细分场景编辑补充验收：60 项定向回归通过，覆盖全部 11 个场景在当前页面打开编辑器、保存和刷新，保留其他用途的最新配置，取消、失败重试、冲突拦截，以及清空共用别名和描述。共享页面与编辑器在桌面浏览器中使用测试数据验收，最低 880×600 下保存和取消按钮可用；类型检查、桌面 UI 边界和国际化检查通过。
