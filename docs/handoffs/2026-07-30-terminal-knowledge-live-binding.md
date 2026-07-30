# 交接：终端会话知识库「挂载显示正常、检索/回写失效」根治（2026-07-30）

- 分支：`dev/terminal-kb-live-binding-20260730`（基于本地 `main` f6bd86d1）
- 性质：bug 根治（证据化诊断 → 分层修复），非新功能
- 症状（用户现场）：终端会话（codex）知识库 UI 显示已挂载、binding 行 `enabled=true / writeback=true(staged/aggressive)`，但 `{cwd}/.nomi/knowledge/README.md` 停留在启动时的 "Write-back is DISABLED"，且整轮会话后端日志零 `Knowledge MCP: dispatching tool`。桌面普通对话 / 伙伴 / 渠道均正常，仅终端会话失效。

## 根因（5 层，全部证据化）

**RC-1 · codex 能力投递断裂（主根因，本机 codex-cli 0.144.6 实证）**
知识/需求 MCP 的能力令牌（`NOMI_KB_MCP_CAPABILITY` / `NOMI_REQ_MCP_CAPABILITY`）通过 PTY 父进程环境投递，但 codex 以 8 变量白名单（HOME/PATH/PWD/…）启动 MCP 子进程，令牌被剥离——`enhance.rs` 旧注释"MCP subprocesses inherit them naturally"对 codex 不成立。bridge 拿不到能力 → 退化 broker 模式（scope 启动时冻结）或失败退出 → 工具残废。claude 传全量 env 所以正常；桌面对话走 ACP 协议逐 server 传 env（`acp_assembler.rs:410`）所以正常。

**RC-2 · 活跃终端契约冻结（设计缺陷）**
README + 签名能力（kb_ids、allow_write→工具白名单）只在 4 个 PTY spawn 时刻刷新；`set_binding` 只发 UI WebSocket 事件、后端零订阅者。对照：conversation 每次 send 重读 binding，签名变化即回收 runtime（`service.rs:12384`）。终端无等价机制——binding 变更对运行中 PTY 完全惰性，且 capability renewal 刻意不可扩权（正确的安全设计，但意味着不 relaunch 就永远拿不到 `knowledge_write`）。

**RC-3 · UI 虚假承诺**
KnowledgeControl 乐观更新 + toast「下次发送消息时生效」（终端没有该时刻）；`applyAfterRelaunch` 脚注藏在弹层底部且与 toast 矛盾；无 relaunch 引导。gateway 工具 note "NEXT task start" 对终端同样失实。

**RC-4 · workpath key 根不一致**
终端侧 `session_workpath_key(cwd, work_dir)`，知识服务实时解析（`resolve_write_context_for_cwd`）用 `data_dir`——两个不同配置值。work_dir 下的默认工作路径终端，`knowledge_write` 实时解析落到错误的 binding 行。

**RC-5 · 指导投递缺失（记录，未在本次修复）**
README 写盘但从未送达模型：AutoWork prompt 刻意移除知识提示（`prompt.rs:500` 测试断言 `!contains("knowledge")`）；`driver.rs:22` 旧注释是从未实现的空头承诺；工具描述静态、不含库名/主题。修复 RC-1 后模型能真实看到三工具（描述含 "Call this FIRST"），但「让模型知道挂了什么库」仍是产品级跟进项。

## 修复方案（已实施）

**Fix A（RC-1）`crates/backend/nomifun-terminal/src/enhance.rs`**
`codex_mcp_argv` 为每个 MCP server 追加 `-c mcp_servers.<name>.env_vars=["<ENV名>"]`——codex 官方按名转发白名单（本机实测有效）。**只有变量名进 argv，秘密值仍走 PTY 环境**，不落盘不进 /proc cmdline。知识+需求两个 bridge 同时受益。

**Fix B（RC-2/RC-4）能力语义重构：冻结快照 → 实时真值**
- `mcp_bridge.rs`：`issue_for_terminal` 去掉 `allow_write` 参数，终端能力恒签三工具（search/read/write）。写权限从「签发时冻结的 allowed_tools」下沉到「dispatch 时实时 policy」——双向即时（后开可写、后关即禁）。conversation/external 签发语义不变（各自有正确的刷新机制）。
- `mcp_server.rs`：`handle_tool_request` 对 `session.kind == Terminal` 的会话，search/read/write 的 kb 范围从 `claims.scope.workspace_path` 的 workpath binding **每次调用实时解析**（`resolve_terminal_scope_for_cwd`，无 all-bases 兜底——未挂载即诚实报"没有挂载知识库"）；write 不再与 spawn 快照求交集（后挂的库立即可写）。信任边界不变：capability 仍钉死 user+terminal+workspace，binding 是同一 workspace 的服务端状态。
- `knowledge/service.rs`：`extra_managed_roots`（late-wire terminal work_dir，修 RC-4 的 key 推导分叉）+ `resolve_terminal_scope_for_cwd` + `set_binding` 进程内 hook（持久化后触发，带 canonical key）。
- `terminal/service.rs`：知识 MCP **始终注入**（对齐 requirement MCP 的 D2 always-inject 先例；空 kb 也注入，binding 后挂即用，从根上消灭"为了注入工具而 relaunch"）；新增 `resync_workpath_knowledge(changed_key)`——遍历 live 终端、workpath 匹配即重跑 mounts+README 同步。
- `app/router/state.rs`：接线（`add_managed_root(work_dir)` + binding hook → Weak<TerminalService> → `tokio::spawn(resync)`）。

**Fix C（RC-3）UI/文案诚实化**
- 终端 target 的挂载 toast 换新 key `knowledge.control.enabledOkTerminal`（"对当前终端会话即时生效"）。
- `terminal.knowledge.applyAfterRelaunch` → `applyLiveNote`：挂载/回写变更即时生效；仅"启动时未注入检索工具的会话"（gemini/自定义命令）需重启注入。
- gateway `nomi_knowledge_set_binding` note 与 companion 系统提示同步修正（终端即时、其余下次任务启动）。

## relaunch 之后还剩什么语义

- **工具注入**仍是 spawn 时刻行为（无法向运行中 CLI 进程注入 MCP server）。始终注入后，claude/codex 终端不再需要为知识库 relaunch；只有 gemini/自定义命令（本就无注入通道）和"MCP 服务未启动"两种情况例外。
- relaunch 时的 `sync_knowledge_workspace` 保留：刷新签名快照 + 给此前无 bridge 的会话注入。

## 测试

- 新增：`mcp_server.rs` 终端实时语义 3 例（事故形态复现：spawn 后挂库→同一 capability 检索命中；writeback 三段开/关/再关即时生效；conversation 只读门保持冻结 403）；`service.rs` hook canonical-key 触发、extra managed root key 一致性 2 例；`enhance.rs` codex env_vars 渲染 2 例；`mcp_bridge.rs` 终端恒三工具改写 1 例；`build_enhancement` 门更新（always-inject / 空 cwd 拒发）。
- 全量：nomifun-terminal 143 / nomifun-knowledge 304 / nomifun-api-types 19 / nomifun-gateway 138 / nomifun-companion 214 全过；`cargo check -p nomifun-app` 过；UI `tsc --noEmit` 过、`bun test` 1628 全过。
- 外部实证（本机）：codex 白名单环境剥离 `NOMI_KB_MCP_CAPABILITY`（复现 RC-1）；`-c mcp_servers.x.env_vars=[...]` 按名转发成功（验证 Fix A seam）。

## 已知残留 / 跟进

1. **gemini 注入**（用户期望三 CLI 全通）：gemini CLI 本机未装、`GEMINI_CLI_SYSTEM_SETTINGS_PATH` + settings `$VAR` 展开方案未实证，刻意不盲发（避免复刻 RC-1 的"未验证投递假设"）。README 的文件式读契约（has_search_tool=false 文案）仍对 gemini 有效。
2. **RC-5 指导投递**：AutoWork 提交是否恢复 README/主题图注入、bridge 工具描述是否动态带库名——产品级取舍，建议独立评审。
3. `resync_workpath_knowledge` 无独立单测（内部复用被 4 个 spawn 路径共测的 `sync_knowledge_workspace`；hook 触发与 key 规范化已各自有测试）。
4. 事故机器的 broker 模式 fallback 在 codex 下曾静默接管（scope 冻结、read-only），修复后管理式能力优先生效；broker 仅剩外置 CLI 注册使用。
