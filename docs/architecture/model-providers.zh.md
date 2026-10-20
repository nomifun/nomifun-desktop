# 模型供应商与任务路由

模型配置与调用以当前代码为准。供应商官网目录、账号权限和接口可用性会变化；旧核验矩阵保存在 Git 历史，不作为运行时能力表或永久白名单。

## 唯一配置源

| 数据 | 所有权 |
| --- | --- |
| `providers` | 供应商身份、启停、配置修订与默认连接 |
| `provider_connections` | 命名连接的地址、鉴权和加密凭据 |
| `provider_models` | 模型 ID 与展示、排序、启停 |
| `provider_model_capabilities` | 精确 `(provider_id, model, task)` 的协议、连接角色、transport、参数、输入约束、限额和健康状态 |

模型 ID 可以手工填写；目录失败不能阻止合法配置。只有原生目录明确声明的任务或精确官方文档 profile 能自动建议接口，模型名推断和供应商已有 adapter 均不证明具体模型支持该任务。后台刷新不修改已保存接口。输入理解属于 Chat，不自动授予生成或语音路由。

当前任务集合来自 `ALL_MODEL_TASKS`：Chat、实时会话、图像生成、图像编辑、视频生成、音乐生成、语音合成、语音识别、Embedding、Rerank。一个模型可配置多个接口，每次调用只解析一个确切任务。缺失任务返回 `UnsupportedTask`；协议未知、任务不匹配或无 adapter 明确失败，不尝试供应商品牌默认值或 OpenAI 兜底。

## 协议与连接边界

`nomifun-model-invoke` 的协议 registry 是 serializer、支持任务、鉴权、endpoint 默认值和可编辑性的来源。模型编辑器通过 `/api/model-protocols` 读取 manifest，不维护第二套协议列表。配置期推荐只有显式保存 capability 后才参与执行；运行时不重新推荐或解析模型名。

同一供应商的不同任务可以使用不同域名、命名连接和凭据。`base_url_override` 只覆盖当前任务，`endpoint`、`poll_endpoint`、`content_endpoint` 和 `realtime_endpoint` 分别表达提交、轮询、内容和实时连接。绝对覆盖地址与凭据连接异源时，必须显式允许 `allow_cross_origin_credentials`。协议切换清除旧 transport 覆盖和跨域确认，再应用新 manifest；保留用户参数草稿和限额，不兼容的协议控制在界面及保存时明确报错，交由用户修正。

`provider_params` 使用协议自己的严格编码契约，不能包含凭据或本地 transport 字段；无法无损编码的值在保存时拒绝。响应只暴露 `has_credentials`，不回显秘密。调用图变化原子递增 `config_revision`；解析、异步任务和健康结果使用一致修订，旧探测结果不能覆盖新配置。

Chat 推荐由 [官方文档 profile](../../crates/backend/nomifun-model-invoke/src/model_profiles.rs)
统一生成。系统预设及手动添加时，通用 OpenAI 兼容对话模板默认 `openai.chat_text`；
OpenAI 官方、MiniMax 和 xAI 保留文档明确的 Responses 默认。仅支持 Responses 不会自动提升模板优先级。
这里只调整默认模板及对应参数，既有自动检测、自动填写和用户手选流程保持；NomiFun 网关仍以目录声明为准。
已保存协议不自动重写。Responses 的输入、
函数调用和流事件使用自己的 typed serializer；工具 schema 显式 `strict: false`，保留
平台可选参数的语义。多段推理摘要按 item 与 summary/content index 独立校验。

`max_tokens_field` 只指定 OpenAI 两种 Chat 协议的输出字段别名，数值仍由 typed 请求与
capability `output_limit` 的较小值决定；别名不能覆盖消息、工具或协议状态。
Chat Completions 的 `stream_options: null` 省略可选用量字段，对象中的 `include_usage`
可显式覆盖缺省值；Responses 的 `include` 数组合并加密推理续传字段，`include: null`
则省略不支持该字段的兼容服务选项。没有加密续传信息时不补造加密内容。解码边界上，
Anthropic 事件 union 中为 null 的成员按缺失处理；Responses item 的 `encrypted_content`
不在 `output_item.done` 快照与终态响应之间比较，因为上游每次序列化都会重新加密。未知流
事件类型按供应商要求忽略：Responses 的内容权威仍是 item 快照加终态，未知 Anthropic 事件
不得携带内容成员。SSE 记录只在空行处提交；唯一例外是流结束时缺少最后分隔空行、但内容
完整的 `[DONE]` 结束标记仍按正常结束处理，其他未终止记录仍是分帧错误。同一次尝试中，
没有 `id` 的 OpenAI Chat 帧（如 Azure 风格的 `prompt_filter_results` 前导帧）属于该次
尝试唯一的响应，并由第一个带 id 的帧接管；相互矛盾的 id 仍是流违规。网关在任何输出之前于
流内转发的 `upstream_error` 与 `server_error` 一样在同一路由上重试；请求校验失败仍在流开始
前以 HTTP 错误返回，更具体的错误码仍优先。Anthropic 系列 Messages 编码器（Anthropic、Bedrock、
Vertex）发送工具 `input_schema` 时去掉这些服务拒绝的根级 `oneOf`/`anyOf`/`allOf`；根对象保留其
属性（根上没有属性时从各分支提升），运行时仍按完整的规范 schema 校验每次调用。
协议拥有的模型、输入、工具、流模式和云端状态字段不能由参数 JSON 注入。
推理强度只使用标准化的 `reasoning_effort` 配置；Responses 的原生 `reasoning.effort`
在保存时提示改用该字段，`summary`、`mode` 和 `context` 等其他原生设置保留。
Chat Completions 不发送本地 round、工具 metadata 或 `is_error` 字段；工具错误作为结果
文本表达，图片和音频在整批工具结果之后使用 user 附件消息传递，工具结果本身保持文本格式。

“OpenAI 兼容”不证明所有模态兼容。Ark 与火山语音、DashScope 原生协议、MiniMax 的地区连接等差异由各自 adapter 和 manifest 表达，不复制成另一份手写 endpoint 矩阵。新增或变更供应商时更新 registry、serializer 与定向合同测试，并针对实际账号验证；本地替身测试不代表云端权限或模型体验通过。

## 模型输入约束与 Agent 路由

Chat capability 可显式保存 `input_constraints: { image_input: false }`，限制这个接口接收图片。共享的 `ModelInputConstraints` 同时派生 API、TypeScript 和模型管理工具 schema；约束保存在独立的 nullable JSON 配置列，不进入 `provider_params` 或上游请求。缺省、`null` 和空 `{}` 统一为 `None`，保持原有协议默认行为，也不为冻结记录增加空字段。非 Chat 用途拒绝非空约束。

`image_input: false` 从有效能力中移除 `ImageInput`；`true` 仅允许使用协议、候选路由与 adapter 本来支持的图片输入，不能增加任何一方缺少的能力。`traits` 是建议性模型元数据：缺少 `vision_input` 不等于禁图，存在该 trait 也不能覆盖显式禁图。目录刷新、健康检查和协议验证不从标签或探测结果自动填写输入约束。

配置保存、读取、克隆、连接修改、协议草稿与模型管理工具均保留约束。规范化后无变化的保存保持配置修订和健康观察；实际调用配置变化沿现有事务递增修订并使旧健康观察失效。数据库变更只添加模型配置列，不修改 canonical Agent 表、存储代际或 schema metadata。

Agent 冻结每个候选自己的约束与能力，Broker 在每次实际尝试中继续取路由与 adapter 的交集；主路由、重试和 failover 均按本次有效能力工作。[Computer Use](computer-use.zh.md) 据此选择 Full 或 NoImage，不能仅凭配置界面的标签或提示词宣称已经禁图。已有图片输入仍须通过实际路由的输入合同校验。

## 文档推荐与模型目录

现役建议数据分为 [国际原生平台](../../crates/backend/nomifun-model-invoke/src/model_profiles/global.json)、
[国内模型厂商](../../crates/backend/nomifun-model-invoke/src/model_profiles/china-foundation.json)、
[云平台与套餐](../../crates/backend/nomifun-model-invoke/src/model_profiles/cloud-platforms.json) 和
[聚合网关及其他服务](../../crates/backend/nomifun-model-invoke/src/model_profiles/gateways.json)。
每项记录官方来源、核验日期、协议依据及未知信息；这些是配置建议，不是运行时白名单。

`model_defaults` 只返回精确型号的已核实上下文、输出推荐、思考档位及协议参数。
原生目录已声明的数值优先；文档只补缺失值，输出推荐可采用厂商明确的同步/默认上限，
不把批处理、后台长输出或占满共享上下文的理论最大值作为普通同步请求配置。
Gemini 4 的共享窗口与旧代独立输入窗口分别记录；用户明确清除、修改及已保存的值不被建议覆盖。
未知 ID 仍可手工配置；未确认的输出数值、能力和档位保持未知，不套用同家族或上游原厂限制。

协议、型号和地址必须一起推荐。例如智谱部分型号的 Responses 使用 `/api/v1`，
阿里 Coding Plan 仍用 Chat Completions；Bedrock 新 Mantle 接口不冒充已实现的旧 InvokeModel 路由。
同样，聚合网关的实际目录、权限和部署限制不能从原厂同名模型推断。

任务与协议的逐供应商核验另见 [国际平台](../../crates/backend/nomifun-model-invoke/src/provider_protocol_audit/global-and-gateways.json)、
[国内平台、套餐与语音](../../crates/backend/nomifun-model-invoke/src/provider_protocol_audit/china-and-voice.json) 和
[聚合服务](../../crates/backend/nomifun-model-invoke/src/provider_protocol_audit/gateways.json)。每组覆盖全部十类用途，记录
已实现、需目录声明、尚未实现、证据不足及退役状态。型号限制按各平台/区域/套餐核对，不继承上游同名模型。
当前原生补齐包括 Gemini Interactions 图像与语音、Gemini batch embeddings、MiniMax/智谱/硅基流动 ASR
、智谱 TTS 及 InfiniAI 重排。火山语音的新鉴权/URL音频输入、部分厂商视频/音乐/实时能力仍需专门实现，不能用相近的
OpenAI serializer 作为可用默认值。原生目录缺少用途时，未知型号保持待配置，不再无条件归入对话。

## 设置期实际协议验证

模型选择完成后，界面通过 `/api/providers/discover-model-protocols` 或
`/api/providers/{provider_id}/discover-model-protocols` 提交精确模型与 capability 草稿。
对话发现和已保存 capability 的 Chat 健康检查使用与 Agent 完全相同的 wire stack 运行同一
次无副作用工具往返：broker serializer 加共享 provider 参数合并、每轮一次单尝试 HTTP 请求
（SSE 或 AWS 事件流分帧）、每次尝试独立解码器并执行 broker 的事件序列规则。检查验证流式
工具调用、真实工具结果回传及最终可见回答；不创建 Session、不执行用户工具。每个候选按
Agent 相同的方式发送 capability 自身的输出上限（仅在未配置时使用 4096），因此超过模型最大值
的上限会在验证时失败，而不是在之后每轮对话失败；每个候选有时长预算，每轮只发一次请求，
不重试。所有候选均记录结果，已有可用选择优先，
否则选择已验证的文档推荐项，再回退到第一个可用项。全局鉴权、额度和限流失败停止后续请求。

多模态候选使用 [供应商协议审计](../../crates/backend/nomifun-model-invoke/src/provider_protocol_audit.rs)
的精确型号或原生目录任务声明，并绑定对应 codec、根地址、端点和专属参数。仅注册 serializer
不等于供应商支持；每型号协议限制不能被目录中的泛化任务标签绕过。证据区分实测、官方型号、
目录声明和未确认状态，自动流程不提交图片、视频、音乐等生成任务。

该动作不写模型目录，也不授予运行时协议故障转移。已有供应商使用服务端加密凭据和修订围栏，
新供应商使用当前表单连接；自定义中转地址不被官方默认地址替换。已核实的官方区域连接可切换原生根路径，
同时保留原区域域名，不跨区发送凭据。界面在目录选中、Enter 或失焦
后开始检查，300ms 去抖并仅在本次编辑缓存相同配置；改动参数或凭据会取消旧请求。应用结果前
复核草稿快照，所有用途检查结束前阻止保存，别名编辑不触发检查。正常状态仅显示连接结果；协议列表在“连接设置”中按需查看，备选失败的原始信息再收进“技术诊断”。
可选数值的 API 空值统一视为未设置，不得将“使用模型默认输出上限”误判为非法值；真实输入错误仅在对应字段提示。
已保存或手工修改的参数使用 `apply_recommended_defaults: false`，避免检查重新补回用户明确删除的字段；
推荐缺省只补新建且尚未手改的草稿。自动值仅归属当前模型，切换型号后释放，手改及持久化值保留。
OpenAI 协议参数转换使用前后端共享的测试向量：`response_format` 与 `text.format` 按
官方 schema 包装转换，回答详略和思考档位保留语义；推理强度在本地仍以顶层
`reasoning_effort` 为唯一配置入口。供应商未知扩展保持原值，不能凭字段名称推断不支持。
手动切换将旧协议专属控制暂存在界面草稿中，切回可恢复；这些界面元数据不会进入保存请求。
冲突或不可映射的语义约束需要用户通过明确的一键操作暂存，不能静默丢弃。实际检测复用同等
转换规则，有未解决约束时停止选择，保留原草稿，不发出丢失约束的候选请求。

## 实现与验证

| 边界 | 源码 |
| --- | --- |
| 精确能力解析与修订一致性 | [resolve.rs](../../crates/backend/nomifun-model-invoke/src/resolve.rs) |
| 任务、协议与参数合同 | [manifest.rs](../../crates/backend/nomifun-model-invoke/src/manifest.rs) |
| 路由、adapter、URL 与鉴权 | [routes_table.rs](../../crates/backend/nomifun-model-invoke/src/routes_table.rs)、[adapters](../../crates/backend/nomifun-model-invoke/src/adapters/mod.rs)、[url_algebra.rs](../../crates/backend/nomifun-model-invoke/src/url_algebra.rs)、[auth.rs](../../crates/backend/nomifun-model-invoke/src/auth.rs) |
| 注册与地址合同测试 | [protocol_manifest.rs](../../crates/backend/nomifun-model-invoke/tests/protocol_manifest.rs)、[url_join_contract.rs](../../crates/backend/nomifun-model-invoke/tests/url_join_contract.rs) |

Session 模型切换和 native checkpoint 围栏见 [Agent Session](agent-session.zh.md)；实时语音的设备与工作所有权见 [语音架构](voice.zh.md)。
