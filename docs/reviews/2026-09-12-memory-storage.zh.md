# R35 长期记忆读写审计

全局审计未完成；memory 已阅读全部生产文件与现有单元/四个集成测试文件，路径归属和多文件事务仍有待办。

## 问题及局部修复

- R35-01：索引在 25000 字节直接切 UTF-8，中文可触发 panic；改为字符边界，保留优先完整行策略，并移除收集全部行的 Vec。
- R35-02：frontmatter 对非独立 opening 分隔符误判、CRLF/带空白 closing 分隔符偏移错误、无效 YAML 丢掉头部原文。以完整行切分的私有 helper 替代手工 offset 状态机；无效内容完整返回。保留原有最多 30 行和读取时跳过正文前空行的行为。
- R35-03：索引 read-modify-write 并发丢条目，非 UTF-8 被当成空索引覆盖。直接持文件锁追加，只读取最后一个字节判断换行；不读取并重写整份索引。8 线程追加 128 条旧实现本次只剩 10 条，修复后完整保留。
- R35-04：引用统计溢出、并发读改写丢计数、重写普通文本/非法 YAML、丢失未知 metadata 和正文空行。持锁读改写，仅更新 YAML mapping 的 usage_count/last_used，计数饱和，正文原样保留；无效文件不回写。读/写 store 使用同一文件的共享/排他锁协调，未新增锁管理器、后台任务或事务框架。
- R35-05：MemoryError 两个特殊分支只有自身测试、生产只转发 I/O 错误。全仓引用核对后删除 error.rs，公开函数直接返回 std::io::Result；删除无用 thiserror/rstest 直接依赖。删除的源文件可从 Git HEAD 恢复，未提交。
- R35-06：后端蒸馏声称全字段脱敏，但仅处理 content/description，name 同样会进入文件和索引。补一行复用现有 redact_secrets_owned；静态调用证据，不宣称外部模型红→绿。

## 验证证据

- 先运行新增六项回归：6/6 失败（UTF-8、无效 frontmatter、CRLF、溢出、非 UTF-8 索引、并发追加）；首次局部修复后全通过。
- 再运行三项引用回写回归：3/3 失败，128 次引用旧实现本次仅余 7 次；最终均通过。
- 最终 `cargo test -p nomi-memory -- --test-threads=4`：108 单元 + 42 集成 = 150/0。
- Agent 调用方：`cargo test -p nomi-agent --lib memory -- --test-threads=4` 10/0；`cargo test -p nomi-agent --test memory_context_integration -- --test-threads=4` 7/0。
- `cargo check -p nomifun-ai-agent -p nomifun-companion` 通过（含蒸馏 name 漏脱敏修复）。这是编译/调用兼容性检查，不是后端完整运行验证；未调用真实模型，未重复 UI 全量。

## 尚未完成（R35-07）

- paths::sanitize_path 的短路径替换不是单射，例如 /a/b 与 /a-b、不同等长中文路径会映射同一目录；改变命名涉及已有记忆归属/迁移，不能默默切换目录造成历史记忆消失。DefaultHasher 长路径稳定性、大小写/非 UTF-8 路径同列后续。
- write_memory 的同名覆盖是已有 remember/companion 的更新行为，本批没有强改为唯一文件；distill 同名候选与索引重复、多行字段/Markdown 转义、file+index 部分成功和跨调用去重仍需明确契约。
- 标准文件锁只协调遵守锁的调用方；非协作编辑器、替换文件、断电崩溃的原子发布不在已验证保证内。读入文件仍无大小上限。
- 引用回写拒绝已存在 symlink，但 metadata 检查与 open 间仍有 TOCTOU，hardlink/父目录 symlink 也不是完整文件系统隔离；不能把词法 basename 校验等同安全沙箱。新增 Unix symlink 用例在 Windows 未编译/运行。
- YAML mapping 保留字段值与正文，不保留 YAML 注释/排版；超过 30 行的手工 frontmatter 仍按既有上限处理。
- 路径测试的环境修改恢复模式和重复历史测试仍有精简空间；本批未为了删测试而改变平台路径契约。
