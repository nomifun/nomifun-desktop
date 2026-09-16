# Coding Patch 文本语义与生产参数契约（2026-09-14，未验证）

本轮仅在 `rf/agent-capability-platform-v2` 修改源码；没有 commit/push。
没有执行构建、测试、真实工具调用、数据库迁移或外部服务验证。只进行了源码阅读、实现及
小范围格式化。当前 Coding `host2-coding-loop32`、Nomi `host17`，整体 CAR 尚未完成。

## 生产接线缺口

`coding_tool_surface::compile` 用平台 canonical schema 替换独立 Coding 工具定义中的 schema。
此前 `fs.write`、`fs.patch`、`vcs.status/diff/stage/commit` 在 Wave2 中回退到 open object，
导致生产模型拿不到真实参数字段和必需项。这是生产接口缺口，不只是文档不完整。

本轮新增 `nomifun-agent-domain-wave2/src/workspace_schema.rs`：

- 为上述现有 owner 提供严格参数契约；patch 约束文件、hunk、行数、坐标及逻辑行文本。
- commit 消息限制与实际 owner 一致为 512 字符，不再向模型宣称支持 65,536 字符。
- write 增加 owner 端 8 MiB 字节上限，防止字符数限制被多字节 UTF-8 放大。
- Kernel 在调用 workspace/process host 前执行 canonical schema 检查，不只依赖模型面验证。
- Coding 组装拒绝 standard builtin 回退为开放/不透明 schema；process 的严格 oneOf 变体仍可用。

这些契约属于平台，不把 File/VCS owner 或权限系统放入 Coding Engine。
源码集成的社区 Engine 可消费相同精确 schema，继续采用自身任务循环和规划策略。
不新增动态加载、打包后挂载或会话热切换。

## Patch 文本能力

参考本地 Codex 的 `codex-rs/apply-patch/src/text_file.rs` 和 `lib.rs` 中逐行保留换行符的
设计，独立实现 Nomifun 的 typed hunk 文本处理。没有引入 Codex 可执行文件、文件系统
owner、宽松匹配器或额外审批系统；也没有把此次源码阅读当作重新验证上游基线。

`agent_patch_lines.rs` 提供共享逻辑行读取：

- 识别 LF、CRLF 和单独 CR；原有/上下文行保留自身结束符，新增行采用第一个源结束符，
  没有源结束符时使用 LF。
- 保留源 UTF-8 BOM，模型提交的第一行正文无需重复 BOM。
- 保留原有 EOF 换行政策；空源新建沿用现有无末尾换行默认。移动到文件内部的原末行会补
  必要分隔符。没有在本轮增加“显式改变末尾换行”的 patch 字段，完整改写仍可使用 fs.write。
- 逻辑行参数不允许内嵌 CR/LF/NUL，避免模型把 CRLF 的 CR 错当正文。
- 读取页行号、literal search 行号和 patch 坐标共用结束符规则，byte offset 仍对应原始字节。
- 源行数量在构造期间限制；搜索使用无额外行集合分配的迭代器。

纯插入支持在文件中部/末尾定位：old_lines=0 时，old_start 是插入前的源行数，0 为开头。
纯删除的 new_lines=0 使用保留输出前缀的行数。仍兼容旧客户端可由 new_start 明确区分的
“下一行之前”写法；不通过内容模糊匹配猜位置。重复或乱序 hunk、上下文不符仍失败，
错误包含目标路径/准确源行号，不把实际源代码或秘密内容复制进持久错误日志。

## 读取与发布边界

patch 准备阶段改用现有有界 source reader，限制单文件/剩余总读取预算，检查读取期间变化；
不再在初始 metadata 大小检查之后调用最多可读取 256 MiB 的普通 UI 文本入口。
上下文文本借用源字节，避免额外完整副本。

创建或覆盖意图固定于准备阶段：

- 新建始终按不存在前提和 no-clobber 发布，不能因为发布前出现了文件而转成覆盖。
- 覆盖与回滚写入在发布阶段再次比较预期字节；目标已消失或改变时拒绝。
- 回滚比较也使用有界读取。既有并发用户修改仍不应被主动恢复为旧版本。
- 成功结果带 source_sha256 / written_sha256；新建或旧格式回执的 source 可缺省。
  written_sha256 是已提交给发布操作的字节版本，可用于下一次 read_file.expected_sha256；
  它不证明稍后的工作区仍未变化，也不证明任务或验证完成。

工具说明明确区分单文件原子发布与整个多文件事务。多文件 I/O 失败仍只有尽力回滚，
不是跨文件/进程事务；外部并发改写、检查后重命名、回滚删除竞态、发布后同步或清理失败
仍有后续 owner 工作，不能宣称完全隔离或自动安全恢复。

## 兼容及剩余项

canonical schema digest 和两个官方 Engine 构建指纹已变化；不迁移既有 immutable Session，
不按最新 channel 静默替换 exact binding。源码宿主需适配逻辑行参数和可选摘要回执字段。

没有放开 Git push：当前 owner 只支持已配置的 local/file remote，没有网络凭据授权链。
本轮没有实际创建 commit 或向任何 remote 写入。
跨启动进程证明、人工解隔离、checkpoint 延续、其他协议/生态生命周期、独立语义完成核对
与用户暂不要求执行的验证工作仍未完成。
