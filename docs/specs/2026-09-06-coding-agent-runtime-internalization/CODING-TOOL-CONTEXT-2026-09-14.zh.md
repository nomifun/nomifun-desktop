# Coding 长工具输出上下文策略（Slice67，未验证）

在 `rf/agent-capability-platform-v2` 本地继续实现。未执行构建、测试、服务、模型
调用或迁移，未 commit/push；历史验证记录不覆盖本轮，整体 CAR 未完成。

## 对照与缺口

阅读固定 Codex 基线 `6af345407d9c2a568da9d01b6c4b81a9e61495c0` 的
`codex-rs/core/src/context_manager/history.rs` 中工具输出的独立 truncation policy，
以及 `codex-rs/core/src/compact.rs` 的派生上下文替换边界。参考的是“工具输出
进入模型窗口时单独限制体积”，没有引入 Guardian、第二个 Session store、Codex
二进制、动态模块或改变源码基线。

当前 Coding 已有工具档案和摘要压缩，但进程长日志与 Git diff 仍完整进入活跃
模型窗口，可能挤占规划、用户要求和后续工具操作所需空间。仅依赖后续付费摘要
也可能在保留最新工具交换时超过上下文预算。

## 本轮实现

- 新增 Coding 私有 `tool_context.rs`。按已绑定 canonical capability 选择
  `process.exec` 的 `/output/text`、`vcs.diff` 的 `/patch`、`/staged_patch`、
  `/unstaged_patch`，不依赖模型自选的工具别名。
- 仅处理符合已知结构的单一文本 JSON 输出。原字段合计预算 16 KiB；按 UTF-8
  边界保留首尾，字段内标明省略字节及“不连续源码”。完整编码后的结果不超过
  24 KiB，而且必须小于原结果；JSON 转义膨胀时减小正文额度，而不是截断 JSON。
- 保留原 JSON 结构及非正文数据：进程 ID/state/exit code/cleanup、游标、输出
  原始 retained/dropped 字节、Git 路径和 owner truncated 标志均不改写。增加
  `_nomifun_context_excerpt` 标明每个字段原长度、保留/省略字节及历史回读方法。
  摘录不是可直接应用的 patch，也不把 owner 游标改成摘录字节位置。
- `turn.rs` 在原结果校验、完成证据/工作状态、Patch 恢复及档案记录之后才做
  模型投影。不改变 ToolCompleted、工具调用次数、成功/错误结果或外部效果。
- 正常闭合历史重建按 ToolStarted 的 capability 使用同一策略；无 admission 的
  延迟调用不被当作真实命令结果。仅投影现有持久化观察，不生成新工具事件或
  新执行证据。历史 source 本来可能被宿主截断，故不宣称与实时原文逐字相同。
- `search_tool_history` 新增可选精确 `call_id` 过滤，支持用空 query 定位摘录所
  指调用，再沿原 `read_tool_history` 分页。跨旧回合 ID 可能重复，仍需检查
  source_turn。查询过滤及分页不授予新访问范围，不重新执行工具。
- 档案说明补充 `source_may_be_bounded`：supplied/imported 历史可能早已被截断。
  `truncated=false` 仅说明本档案没有进一步裁剪该 source，不表示原 owner 输出
  完整，不能把历史摘录伪称为完整原始结果。

## 保留边界

文件/指令读取、搜索指令后处理、媒体、历史分页/其他 Engine 控制工具和未知社区
输出均不在本策略范围。缺失结构、原有 marker、不可解析 JSON 或元数据本身超限
时保留原结果，由既有上下文压缩/硬预算处理；不丢弃操作元数据来伪装可容纳。

实时原始结果仍先进入既有 sink/证据/档案路径，但这些路径各自已有预算：档案最多
128 条/4 MiB，每条最多 64 KiB 正文；宿主持久化观察也会截断或省略媒体。本轮没有
新增全量输出存储或扩大日志预算，档案读完不表示原始日志读完，已丢失的内容不会
恢复；不能仅为取回输出而重跑有副作用命令。

这是 Coding Engine 的上下文机制，不要求 Nomi 或社区 Engine 采用。公共 SDK、
注册机制、事件格式、平台授权及 Session exact binding 不变。Coding 构建更新为
`host2-coding-loop67` 并纳入新源码摘要；Nomi 仍为 `host43`。

仅运行了局部 rustfmt。尚无运行时收益或正确性的实测证据，不能宣称质量已超过
Codex。Git 网络凭据、未知效果人工核对、部分协议/生态生命周期和实际验证仍待完成。
