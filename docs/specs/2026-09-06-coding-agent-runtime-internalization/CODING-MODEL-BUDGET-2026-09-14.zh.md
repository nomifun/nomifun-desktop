# Coding 模型能力预算（Slice68，未验证）

继续在 `rf/agent-capability-platform-v2` 本地实现；未运行构建、测试、模型调用、
服务或迁移，未 commit/push。整体 CAR 未完成，历史通过记录不覆盖本轮。

## 缺口

生产宿主已读取 exact route 的 primary/failover 能力并取交集，但 Coding 的
`from_limits` 无论模型大小都额外 `min(4096)`，大窗口模型也只能生成很短的完整
工具参数或回答。另一个问题是调用方若显式设置更小的输出额度，实际发送会降低，
而 ContextLifecycle 仍按较大的 model_budget 预留，造成不必要的上下文压力。

对照固定 Codex 基线 `6af345407d9c2a568da9d01b6c4b81a9e61495c0` 的
`codex-rs/protocol/src/openai_models.rs` 中 `resolved_context_window`、
`usable_context_window` 和 `auto_compact_token_limit`：借鉴的是模型能力上限与
Engine 可用预算/压缩阈值分离，不复制其具体百分比，也不声称 Codex 使用本轮的
16384 输出上限。没有改变源码基线或引入 Guardian。

## 实现

- `CodingModelBudget::from_limits` 输出额度改为平台提供的输出上限、上下文窗口
  的八分之一、16384 三者取最小。未知 context/output 仍使用 32768/4096；平台
  仍对每个候选路由分别补未知默认值再求交集，大 primary 不遮蔽小/未知 failover。
- 仅在模型已知能力允许时提高自动额度，不增加回合步数、流量字节预算或输出
  截断后的两次续接上限。额度是 ceiling，不要求模型消耗全部 token；允许的单次
  输出和潜在用量确实可能增加，尚未实测质量、费用或延迟收益。
- 在回合入口计算一次 `for_request`：显式更小额度继续生效，显式零值失败；
  发送参数、上下文预留和摘要输出限制使用同一个 effective budget。
- 每个主模型边界核对 max_output_tokens 与冻结的预留一致，不允许后续流程
  静默改大。摘要仍使用独立的更小输出限额，不对主请求或失败工具自动增额重试。
- 模型上下文加入本回合的固定 context/output/step 上限说明，要求完整参数、
  分拆大编辑并留出读结果/重新规划/完成说明的空间；预算耗尽不等于任务完成，
  不授予验证或额外效果权限。没有增加随意延长回合的模型控制工具。

按公式举例（不是运行结果）：已知 65536 context / 8192 output 可得到 8192；
131072 context / 32768 output 得到 16384；未知模型维持 4096。调用方若显式
要求 1024，则发送和预留都使用 1024。

## 边界和进度

上述自动政策用于官方宿主 `from_limits` 接线。源码接入者仍可显式构造合法
CodingModelBudget；该结构不是平台授权上限，真实能力与路由在 Broker 继续校验。
预算校验仍要求 context >= 2048、output > 0 且小于 context 一半。不改变推理
设置、模型选择、凭据或 Session exact binding；Nomi/社区其他 Engine 不被要求
采用 Coding 的额度政策。估算器仍不是实际 tokenizer。

Coding 更新为 `host2-coding-loop68`，既有源码摘要已覆盖 context_lifecycle/turn；
Nomi 保持 `host43`。仅对 context_lifecycle 运行局部 rustfmt，没有执行验证。
Git 网络凭据绑定、未知效果人工核对、部分协议/生态生命周期与实际验证仍待完成。
