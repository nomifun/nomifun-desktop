# Coding：交互命令观察链（未验证）

任务 CAR-04 / CAR-06。仅在 `rf/agent-capability-platform-v2` 本地实施，未 commit/push。
没有运行构建、测试、评测或命令 fixture；仅源码阅读和局部格式化。
Coding build：`host2-coding-loop23`，Nomi 保持 `host9`。

## 缺口与参考

原实现每次 stdin 都增加 workspace epoch，但命令永远与 launch epoch 比较。
于是即便该进程是唯一运行进程、没有其他修改、所有交互成功并已退出清理，结果也会因
自己的 stdin 被判成过期。另一方面 EOF/resize 没有增加 epoch，遗漏了可能触发程序工作的控制。

本轮读取本地 Codex `tools/handlers/unified_exec.rs` 和 `unified_exec/write_stdin.rs`。
它将 write_stdin 视为已有 exec session 的继续，终态观察关联原始 exec，而不是另一条启动命令。
Nomifun 保留自己的平台进程 owner 与效果准入，只完善 Coding 对这些既有操作的观察关联。

## 行为

- CommandProvenance 分别保存不可变 launch_epoch 与可延续的 current_epoch；没有把一次 stdin
  伪装成重新执行整个命令。状态事件字段分别是 launch_workspace_epoch/provenance_workspace_epoch。
- stdin、close_stdin、resize 都视为潜在交互效果，先使其他旧观察失效。
- 只有交互成功、请求/结果的 process_id 一致、该进程是唯一已知运行进程、且来源链在交互前仍
  对应前一个 epoch，才将这条命令链延续到新 epoch。调用 ID 保留，参数/env/stdin 内容不重复复制。
- 其他文件修改、新进程重叠、失败/未知启动或交互，均不能被之后的一次成功 stdin 恢复为有效。
  未形成合法结构化结果的交互仍增加 epoch，因此后续不可能跨过它无依据地续上旧链。
- 每个命令最多保留 8 个交互 ID；超出仍允许平台按原规则执行/清理，但记录 omitted_interactions，
  不再把不完整链用于 supported 判定。不是停止清理，更不会为获取证据自动重跑命令。
- 终态仍必须 exited、具备 exit code、真实 cleanup.reaped 且无运行进程；完成报告 supported
  还要求 exit zero、工具结果成功和当前 workspace epoch。取消/超时不是成功证据。
- 重复 poll 已观察终态不会给旧命令制造新时效，也不会替换第一次终态关联。

CompletionObservation 可保存有界 CodingCommandObservation，摘要后仍可看到启动和交互
调用关联。窗口同时限制 64 项/32KiB 序列化内容，旧项省略会计数，不允许报告引用已移出的 ID。
这是来源/时序提示，不包括原命令源码或输出，也不能让模型猜测被摘要省略的参数和结果。

## 边界

epoch 是引擎对已观察潜在效果的顺序，不是文件哈希、外部编辑监控或命令内部执行阶段证明。
同一命令可能先测试再写文件；新的交互链不会证明测试发生在它内部最后一次修改之后。
模型必须结合真实工具输出解释范围，不能仅凭 exit zero 或 current 标志宣称测试/任务完成。

平台 ProcessScope、turn 所有权、取消和 cleanup、不重放与跨启动隔离没有改变。
这不是人工隔离解除、安全 checkpoint continuation 或孤儿进程树退出证明。
新增事件字段缺省兼容历史解码，但历史不会变成当前回合证据；构建摘要已含修改的两个模块。

后续需要验证 start→stdin→EOF→poll、交互直接终态、失败/部分写入、外部工具修改、并发进程、
交互预算、重复终态 poll、观察字节预算和 compaction 的实际表现。当前没有这些运行证据，
不能拿历史进程测试当作本切片验收。其余协议/生态/复杂路径/语义/恢复能力仍按 STATUS 推进。
