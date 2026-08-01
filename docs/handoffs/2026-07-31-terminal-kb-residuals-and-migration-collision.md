# 交接：终端知识库残留隐患收口 + 迁移 022 版本冲突根治（2026-07-31）

- 分支：`worktree-fix-terminal-kb-residuals-20260731`（基于 origin/main `894ed6eb` = v0.3.5）
- 性质：上一轮根治（c6e42852，见 `2026-07-30-terminal-knowledge-live-binding.md`）的证据化复核 + 残留隐患收口 + 一个新发现的 P0 发布级故障
- 复核结论：事故三事实（README 停在 DISABLED / binding POST 后无效果 / 零 `Knowledge MCP: dispatching tool`）在 c6e42852 后的 main 上均已修复（v0.3.4 起含该修复）；本轮修的是复核中证实的残留缺陷。

## P0 · 迁移版本 22 重复 —— v0.3.5 无法启动（最高优先级）

`2a640df3`（codex ACP bridge swap）与 `6f29160d`（feishu connector 移除）各带一个 022 迁移，
`866f29d1` 合并后两者并存，**并已随 v0.3.5（`894ed6eb`）发布**。sqlx 0.8.6 不检测重复版本：
先嵌入的 022 正常 apply，第二个 022 的 `_sqlx_migrations` INSERT 触发 version 主键冲突回滚，
`run_migrations_with_retry` 重试后变成 `VersionMismatch(22)` 硬错误 →
`init_database` 失败 → **新装与 v0.3.4 升级用户全部无法启动**；工作区内 437 处
`init_database_memory` 测试同炸（本轮最初 11 个 nomifun-knowledge 失败即此因）。

修复：`022_drop_connector_credentials.sql` → `023_…`（先到先得，重命名后来者——与历史四次
冲突处理 52d4a8a0/72e8c8d7/f295663b/99166fa2 完全一致）。并**恢复** 52d4a8a0 引入、
被 v3 重构 eca7963d 误删的 `migration_file_versions_are_unique` 守卫测试（tests/db_lifecycle.rs）——
守卫消失正是本次冲突能悄然发生的原因。

已跑过 v0.3.5 的机器上 22 已提交为两个文件之一（read_dir 顺序不定）：
- 22=codex_acp（与重命名后一致）→ 自愈：023 作为新迁移正常补跑。
- 22=drop_connector → 仍会 `VersionMismatch(22)`。修复版需在发布说明中告知重置数据集
  （repo 的迁移重编号既定策略，见 Git 历史中 `2026-07-30-model-catalog-p3.md` 尾注）；未实现 checksum 改写自愈。

**发布动作建议：尽快出 v0.3.5.x/v0.3.6 热修，并考虑下架/替换 v0.3.5 安装包与 updater latest.json。**

## 残留缺陷修复（全部证据化确认后修）

1. **gateway `nomi_knowledge_set_binding kind=terminal` 死行**（从 agent 席位完整复刻原事故）：
   写 `('terminal', id)` 行但零读者（终端只读 `('workpath', key)`），响应还宣称"live 终端立即生效"。
   修复：`resolve_binding_row` 把 terminal 目标翻译到其 workpath 行（新增
   `TerminalService::knowledge_workpath_key`，与 `bind_knowledge` 同派生）；get/set 同步翻译；
   effect note 按 kind 说实话。
2. **README 工具声明忽略 declared backend**：`sync_knowledge_workspace` 硬编码
   `resolve_agent_family(..., None)`，而注入用 `row.backend`——README 会双向撒谎
   （`stepcode`+backend=codex → 显示无工具；args 有 claude+backend=gemini → 谎称有）。
   修复：签名加 `declared_backend`，五个调用点全部传 `row.backend` / `req.backend`。
3. **stale-resync 竞态**：binding 读在 workspace 锁外，旧 resync 可用旧 binding 覆盖新 unbind
   （落盘 symlink 继续暴露已卸载库内容）。修复：`knowledge_sync_locks`（按 cwd 的互斥）
   把"读 binding → mounts → README"整体串行；binding 读在锁内取的是当前持久化行，天然收敛到最新。
4. **`delete_binding` 不触发 hook**：删除行后 live 终端的 mounts/README 滞留到 relaunch。
   修复：与 `set_binding` 同一 hook 合同（canonical key，持久化后触发）。
5. **RC-4 余量（work_dir ≠ data_dir）**：终端实时解析先查 data_dir 再查 extra roots，
   自定义 cwd 落在 data_dir 下时两侧 key 分叉（终端绑字面 key，dispatch 解析 __default__——
   可能静默借用无关库的 scope）。修复：新增 `terminal_workpath_key_for_cwd`，注册过 work root
   时以其为准（与终端侧派生逐字节一致），data_dir 仅作未注册时的兜底。

## RC-5 · 指导投递（事故事实"零工具调用"的另一半根因）

README 落盘但从未进模型上下文 → 模型不知道有库可查。实现三层（均为诚实、按需）：

- **MCP initialize `instructions`（主通道）**：KB MCP server 新增 `POST /context`
  （与 /tool 同鉴权；terminal 会话实时解析 binding；只回库名/描述 + write_enabled，
  不含路径/ID/密钥）。stdio bridge 启动时取一次，`render_instructions` 渲染 <2KB 文案
  （检索优先 + 库清单 + 回写状态 + "以 README.md 为实时真相"锚），手写 `get_info()`
  覆盖宏默认（宏默认 instructions=None，claude 会注入上下文、codex 作为 namespace 描述展示）。
  空挂载 → None（零 prompt 成本）。获取失败 → 无 instructions（等于今天），不阻启动。
- **AutoWork 每轮一行**：`TerminalDriver::knowledge_mounted`（实时解析，默认 false），
  `build_terminal_requirement_prompt` 仅在真挂载时附一行 `knowledge_search` 提示
  （prompt.rs 原"必须不含 knowledge"的断言翻转为按 flag 门控）。
- README 本身不变，仍由 resync 实时维护，作为 instructions 里的 live 锚点。

## gemini 一键接入（用户期望三 CLI 全通；本机 gemini-cli 0.53.1 实证后实现）

- 注入通道：`GEMINI_CLI_SYSTEM_DEFAULTS_PATH` 指向会话私有
  `{session_dir}/gemini-system-defaults.json`（最低优先级层，用户/工作区设置仍按 server key
  浅合并覆盖；若存在管理员真 system-defaults 先并入、只覆盖 `nomifun-*` 键）。
- 秘密不落盘：`mcpServers.<name>.env` 写 `"$NOMI_KB_MCP_CAPABILITY"` 占位符，
  gemini 从父进程环境展开——**本机实测**：占位符展开值送达 MCP 子进程、完整
  initialize/tools-list 握手成功（fake server 校验）、双 server（knowledge+requirement）并行注入 OK。
- 信任模型：**不设** `GEMINI_CLI_TRUST_WORKSPACE`——gemini 自己的 folder-trust 弹窗是用户可见
  的同意界面，未信任目录 stdio MCP 被 gemini 拒启（实测确认），平台不越权代答。
- README `has_search_tool` 门与注入器同步放宽为"任意已识别 agent 家族"。
- UI `applyLiveNote` 两语言更新（gemini 不再列为需重启注入的例外）。
- **未做**：gemini lifecycle hooks（≥0.49 有 AfterAgent/AfterTool，Google 官方迁移表
  Stop→AfterAgent）。调研已证实可行，但 hook 语义未本机实测，坚持"未验证不投递"，
  `supports_lifecycle_hooks` 仍返回 false（gemini 终端暂不 AutoWork）→ 独立跟进。

## 有意保留的设计取舍（已记录，不算缺陷）

- **常驻凭证扩权**：terminal capability 恒签三工具 + always-inject，PTY 环境里的 token 在用户
  后续挂库/开回写时权限随之增长（无重发事件）。这是对"冻结+relaunch 事故"的定向取舍；
  服务端 policy 是唯一执行点。已在 `issue_for_terminal` 文档中显式声明。
- 并发 binding POST 的"README 落后一版"窗口被串行锁消除；base 删除 → workspace 清理
  仍未接 hook（低危，卸载路径已覆盖大头）→ 跟进。

## 验证

- 新增测试：db_lifecycle 迁移唯一性守卫；knowledge service 终端 key 优先级 / delete hook 2 例；
  mcp_server `/context` 实时语义 1 例（未挂→挂载→伪造 token 三段）；knowledge_stdio
  instructions 渲染 4 例 + get_info 覆盖 1 例；terminal `knowledge_workpath_key` 派生一致性 1 例；
  enhance gemini 渲染断言重写（settings 文件形状 / $VAR 占位 / 秘密不落盘 / 不预信任）；
  requirement prompt 知识行门控翻转。
- 套件（本机）：nomifun-db 414+22 全过（重命名后）；nomifun-knowledge 273 全过；
  nomifun-terminal 143 过（`exit_during_relaunch_preflight…` 在 4 线程并发下偶发，
  隔离 3 连过——待与 main 基线对照确认为既有环境性抖动）；nomifun-requirement 102 全过；
  nomifun-gateway 138 全过；nomifun-app lib 647 全过；api-types 250 全过。
- 外部实证：gemini-cli 0.53.1 注入链全通（见上）；claude/codex 渲染回归由既有测试钉住。
