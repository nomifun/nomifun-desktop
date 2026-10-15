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

OpenAI 官方新增 Chat 配置推荐 `openai.responses`；自定义供应商也可显式选择该协议，
普通兼容服务的缺省仍为 `openai.chat_text`。已保存协议不自动重写。Responses 的输入、
函数调用和流事件使用自己的 typed serializer；工具 schema 显式 `strict: false`，保留
平台可选参数的语义。多段推理摘要按 item 与 summary/content index 独立校验。

`max_tokens_field` 只指定 OpenAI 两种 Chat 协议的输出字段别名，数值仍由 typed 请求与
capability `output_limit` 的较小值决定；别名不能覆盖消息、工具或协议状态。
Chat Completions 的 `stream_options: null` 省略可选用量字段，对象中的 `include_usage`
可显式覆盖缺省值；Responses 的 `include` 扩展与必需的加密推理续传字段合并。
协议拥有的模型、输入、工具、流模式和云端状态字段不能由参数 JSON 注入。
推理强度只使用标准化的 `reasoning_effort` 配置；Responses 的原生 `reasoning.effort`
在保存时提示改用该字段，`summary`、`mode` 和 `context` 等其他原生设置保留。
Chat Completions 不发送本地 round、工具 metadata 或 `is_error` 字段；工具错误作为结果
文本表达，图片和音频在整批工具结果之后使用 user 附件消息传递，工具结果本身保持文本格式。

“OpenAI 兼容”不证明所有模态兼容。Ark 与火山语音、DashScope 原生协议、MiniMax 的地区连接等差异由各自 adapter 和 manifest 表达，不复制成另一份手写 endpoint 矩阵。新增或变更供应商时更新 registry、serializer 与定向合同测试，并针对实际账号验证；本地替身测试不代表云端权限或模型体验通过。

## 实现与验证

| 边界 | 源码 |
| --- | --- |
| 精确能力解析与修订一致性 | [resolve.rs](../../crates/backend/nomifun-model-invoke/src/resolve.rs) |
| 任务、协议与参数合同 | [manifest.rs](../../crates/backend/nomifun-model-invoke/src/manifest.rs) |
| 路由、adapter、URL 与鉴权 | [routes_table.rs](../../crates/backend/nomifun-model-invoke/src/routes_table.rs)、[adapters](../../crates/backend/nomifun-model-invoke/src/adapters/mod.rs)、[url_algebra.rs](../../crates/backend/nomifun-model-invoke/src/url_algebra.rs)、[auth.rs](../../crates/backend/nomifun-model-invoke/src/auth.rs) |
| 注册与地址合同测试 | [protocol_manifest.rs](../../crates/backend/nomifun-model-invoke/tests/protocol_manifest.rs)、[url_join_contract.rs](../../crates/backend/nomifun-model-invoke/tests/url_join_contract.rs) |

Session 模型切换和 native checkpoint 围栏见 [Agent Session](agent-session.zh.md)；实时语音的设备与工作所有权见 [语音架构](voice.zh.md)。
