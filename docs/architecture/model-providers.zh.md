# 模型供应商与任务路由

模型配置与调用以当前代码为准。供应商官网目录、账号权限和接口可用性会变化；旧核验矩阵保存在 Git 历史，不作为运行时能力表或永久白名单。

## 唯一配置源

| 数据 | 所有权 |
| --- | --- |
| `providers` | 供应商身份、启停、配置修订与默认连接 |
| `provider_connections` | 命名连接的地址、鉴权和加密凭据 |
| `provider_models` | 模型 ID 与展示、排序、启停 |
| `provider_model_capabilities` | 精确 `(provider_id, model, task)` 的协议、连接角色、transport、参数、限额和健康状态 |

模型 ID 可以手工填写；目录失败不能阻止合法配置。只有原生目录明确声明的任务或精确官方文档 profile 能自动建议接口，模型名推断和供应商已有 adapter 均不证明具体模型支持该任务。后台刷新不修改已保存接口。输入理解属于 Chat，不自动授予生成或语音路由。

当前任务集合来自 `ALL_MODEL_TASKS`：Chat、实时会话、图像生成、图像编辑、视频生成、音乐生成、语音合成、语音识别、Embedding、Rerank。一个模型可配置多个接口，每次调用只解析一个确切任务。缺失任务返回 `UnsupportedTask`；协议未知、任务不匹配或无 adapter 明确失败，不尝试供应商品牌默认值或 OpenAI 兜底。

## 协议与连接边界

`nomifun-model-invoke` 的协议 registry 是 serializer、支持任务、鉴权、endpoint 默认值和可编辑性的来源。模型编辑器通过 `/api/model-protocols` 读取 manifest，不维护第二套协议列表。配置期推荐只有显式保存 capability 后才参与执行；运行时不重新推荐或解析模型名。

同一供应商的不同任务可以使用不同域名、命名连接和凭据。`base_url_override` 只覆盖当前任务，`endpoint`、`poll_endpoint`、`content_endpoint` 和 `realtime_endpoint` 分别表达提交、轮询、内容和实时连接。绝对覆盖地址与凭据连接异源时，必须显式允许 `allow_cross_origin_credentials`。协议切换清除旧 transport 覆盖和跨域确认，再应用新 manifest；保留用户参数草稿和限额，不兼容的协议控制在界面及保存时明确报错，交由用户修正。

`provider_params` 使用协议自己的严格编码契约，不能包含凭据或本地 transport 字段；无法无损编码的值在保存时拒绝。响应只暴露 `has_credentials`，不回显秘密。调用图变化原子递增 `config_revision`；解析、异步任务和健康结果使用一致修订，旧探测结果不能覆盖新配置。

Chat 推荐由 [官方文档 profile](../../crates/backend/nomifun-model-invoke/src/model_profiles.rs)
统一生成，当前逐家核验日期为 2026-10-15。官方确认并推荐 Responses 的服务优先
`openai.responses`；仍主推或只确认支持 Chat Completions 的服务保持 `openai.chat_text`。
同品牌的按量、套餐、地区和具体型号分别匹配，不根据名称扩大支持范围。Custom/New API
探测优先 Responses；NomiFun 网关目录的已声明协议仍为确切来源。已保存协议不自动重写。Responses 的输入、
函数调用和流事件使用自己的 typed serializer；工具 schema 显式 `strict: false`，保留
平台可选参数的语义。多段推理摘要按 item 与 summary/content index 独立校验。

`max_tokens_field` 只指定 OpenAI 两种 Chat 协议的输出字段别名，数值仍由 typed 请求与
capability `output_limit` 的较小值决定；别名不能覆盖消息、工具或协议状态。
Chat Completions 的 `stream_options: null` 省略可选用量字段，对象中的 `include_usage`
可显式覆盖缺省值；Responses 的 `include` 数组合并加密推理续传字段，`include: null`
则省略不支持该字段的兼容服务选项。没有加密续传信息时不补造加密内容。
协议拥有的模型、输入、工具、流模式和云端状态字段不能由参数 JSON 注入。
推理强度只使用标准化的 `reasoning_effort` 配置；Responses 的原生 `reasoning.effort`
在保存时提示改用该字段，`summary`、`mode` 和 `context` 等其他原生设置保留。
Chat Completions 不发送本地 round、工具 metadata 或 `is_error` 字段；工具错误作为结果
文本表达，图片和音频在整批工具结果之后使用 user 附件消息传递，工具结果本身保持文本格式。

“OpenAI 兼容”不证明所有模态兼容。Ark 与火山语音、DashScope 原生协议、MiniMax 的地区连接等差异由各自 adapter 和 manifest 表达，不复制成另一份手写 endpoint 矩阵。新增或变更供应商时更新 registry、serializer 与定向合同测试，并针对实际账号验证；本地替身测试不代表云端权限或模型体验通过。

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

## 设置期实际协议验证

显式检测动作通过 `/api/providers/detect-chat-protocol` 或已保存供应商的同名子路径
读取当前模型与完整 capability 草稿，复用生产 OpenAI/Responses serializer 和流解码器。
先验证当前协议，只有接口或协议解析明确不兼容时尝试另一项；可达状态、空 JSON 探测、
认证、额度或瞬态故障不能产生已验证选择。返回的 capability 正是实际测试过的连接地址、
端点和参数，保持原模型限额，另声明检测请求的有界输出预算。

该动作不创建 Session、不写模型目录，也不授予运行时协议故障转移。已有供应商使用服务端
加密凭据和修订围栏；新供应商使用本次显式提供的连接。界面保存结果前复核草稿快照，
检测期间阻止重复提交，过期结果不覆盖模型、凭据或手工调整。检测明确提示用量并由用户点击，
普通键入及基础可达性探测不自动发起付费模型请求。
跨协议候选必须保留输出和思考语义；无法安全转移原生参数时停止选择，保留原草稿。

## 实现与验证

| 边界 | 源码 |
| --- | --- |
| 精确能力解析与修订一致性 | [resolve.rs](../../crates/backend/nomifun-model-invoke/src/resolve.rs) |
| 任务、协议与参数合同 | [manifest.rs](../../crates/backend/nomifun-model-invoke/src/manifest.rs) |
| 路由、adapter、URL 与鉴权 | [routes_table.rs](../../crates/backend/nomifun-model-invoke/src/routes_table.rs)、[adapters](../../crates/backend/nomifun-model-invoke/src/adapters/mod.rs)、[url_algebra.rs](../../crates/backend/nomifun-model-invoke/src/url_algebra.rs)、[auth.rs](../../crates/backend/nomifun-model-invoke/src/auth.rs) |
| 注册与地址合同测试 | [protocol_manifest.rs](../../crates/backend/nomifun-model-invoke/tests/protocol_manifest.rs)、[url_join_contract.rs](../../crates/backend/nomifun-model-invoke/tests/url_join_contract.rs) |

Session 模型切换和 native checkpoint 围栏见 [Agent Session](agent-session.zh.md)；实时语音的设备与工作所有权见 [语音架构](voice.zh.md)。
