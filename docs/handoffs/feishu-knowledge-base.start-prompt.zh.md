# 启动 prompt：飞书知识库（远端实时检索型）

依据 2026-10-27 用户决策：飞书知识库 = 本地文件夹（知识库）+ 策展的飞书文档名称索引；挂载即挂载选定文档集合；检索远端实时（正文不落库）；索引在挂载时自动刷新；一期仅 doc/docx + wiki 解析；`documents.read` 能力仅限知识链路。方案见 [feishu-knowledge-base.zh.md](feishu-knowledge-base.zh.md)，接续点始终是其第 12 节。

这是实施授权，不是只做 review。开发、测试和例行修复直接推进；提交、推送、安装、系统权限操作和发布沿用会话已有授权，本说明不增加确认步骤。

## 主实施 prompt

```text
请在当前 nomifun-desktop checkout 实施"飞书知识库（远端实时检索型）"：让用户能创建 N 个飞书知识库，每个库是一个本地文件夹，内含若干选定的飞书文档名称索引（stub）；挂载知识库即挂载该选定文档集合；knowledge_search 走飞书 Search v2 并与策展 token 集求交，knowledge_read 经官方 CLI 实时取正文；正文不落本地库。

执行依据为 2026-10-27 用户修订：远端实时检索（非快照同步）；一期仅 doc/docx + wiki 解析；批准新增 documents.read 能力但仅限知识链路，工作台 UI 不做正文预览/编辑；新鲜度策略为挂载时自动刷新索引（元数据级，非正文）。

直接在 main 开发。先检查 git status、HEAD 和当前分支，确认 main，并 git pull --ff-only；每个阶段开始前再次同步。保护无关改动，不 stash、reset、覆盖或清理用户工作。重叠文件按实际 HEAD 合并意图。每次落到 main 的改动自洽、可编译、受影响测试通过，不出现"UI 已放行、后端未接上"的状态。

完整阅读一次 AGENTS.md、docs/handoffs/feishu-knowledge-base.zh.md（唯一施工说明）、docs/handoffs/feishu-connector.zh.md（本专项登记其 §3 合同修订：documents.read 仅服务知识链路）、docs/architecture/agent-session.zh.md、docs/contributing/development.md。接续时核对方案 §12 最新进展和实际源码，不机械重做已完成的阅读/测试。按已定 F01–F10、D01–D12 执行；只有必须改变产品合同或缺少无法推断的关键事实时才提出具体问题，同时继续不受阻的工作。

从方案接续，分阶段推进：
- S1（飞书连接器读取能力）：先核定已安装 lark-cli 的 docs +fetch、wiki 解析与元数据命令的真实参数/scope（--help 与上游源码，登记回方案 §13）；adapter 增加 docs +fetch/wiki 解析/元数据读取的 typed 命令与解析器；permissions() 增加 documents.read capability；service 暴露内部 search_documents/fetch_document/resolve_link 方法（ready() + scope 检查 + output_error 分类）。覆盖 mock CliOutput 单测。
- S2（knowledge 域）：KnowledgeSourceKind::Feishu persisted enum；validate_source 放行 feishu（链接官方域校验、类型白名单 doc/docx/wiki、≤64 条、拒绝 live/rendered）；FeishuKnowledgePort trait + fail-closed 默认；stub 投影写入/标题改名/孤儿清理复用现有管线；索引刷新服务方法 + 挂载组装点挂钩（≤5s 等待、超时降级 + stale 标注进 KnowledgeMountInfo optional 字段）；BoundKnowledgeBase remote 标记随 typed binding 冻结；BoundKnowledgeReadService 与 /api/knowledge/search 的远端分发、.remote/ 伪路径 handle 与 read、远端正文写拒绝；resume_pending_source_fetches 覆盖 feishu。
- S3（接线与 UI）：nomifun-app 组合根注入 OfficeFeishuKnowledgePort；CreateStudio 恢复 feishu 类型与连接引导；详情页索引条目管理与"添加飞书文档"picker（复用 Documents 查询）；工作台"存入知识库"；derive_kind="feishu" 的卡片徽标与双语；退役/反转 feishuKnowledgeEntry.test.ts 的"非源类型"断言。
- S4（验收收尾）：受影响 Rust/UI 测试、check:desktop-ui-boundary、check:agent-session-boundary；真实飞书租户端到端验证并登记 docs/validation/feishu-knowledge-base.zh.md；feishu-connector.zh.md §3 追加合同修订行；CHANGELOG 如有既有惯例则补条目。

保持质量与成本边界：
- 正文不落库（唯一例外是显式 copy-as-editable）；live 模式 feishu 源不做；不做账号全域库、定时刷新、bitable/sheet 导出、写回飞书、工作台正文预览。
- 不新增第二套挂载/绑定/事实链；不修改 canonical AgentSession 存储；不引入未经使用的抽象。
- 远端库语料边界 = 策展 token 集交集；搜索返回 hits 必须有稳定 kdoc_ handle（.remote/ 伪路径，不随 stub 改名失效）。
- 索引刷新失败不阻塞会话启动；远端不可用返回可操作错误而非静默空结果；CLI 并发 ≤4、单次超时收敛。
- CLI 命令细节以实际安装版本的 --help 核定为准；核定结果回写方案 §13，不凭记忆猜测参数。

按改动选择最小验证集合：cargo test -p nomifun-feishu-connector；cargo test -p nomifun-knowledge；cargo check -p nomifun-app；UI 改动跑 bun run check:desktop-ui-boundary 与受影响前端测试；涉及 Session 边界跑 bun run check:agent-session-boundary。跨切面收尾与正式发布前跑 bun run check 全量；其他阶段不重复无关矩阵。

真实飞书租户验证需要已连接的飞书连接器与可用文档；缺少凭据或连接只记待验，不阻塞代码接入，不宣称未验支持。凭据与 token 只留官方 CLI store，不进入日志、文档或前端。

持续实现、测试与更新文档，不反复询问是否继续。只在方案 §12 记录执行进展：源码/构建身份、实际命令、非零测试数、真实飞书证据、待验原因与下一动作；自动化、真实 CLI、未执行项分别记录。
```
