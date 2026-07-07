# Superpowers 内置集成 + 定期热更新 + 编码场景自动启用 — 设计

- 日期：2026-07-07
- 状态：已确认（用户批准推荐方案 A，授权实施至交付）
- 分支：`feat/superpowers-integration`

## 1. 目标 / 非目标

**目标**
1. **内置**：把 [obra/superpowers](https://github.com/obra/superpowers)（一套 14 个方法论 `SKILL.md` 的技能库，MIT）随 nomifun 出厂，开箱可用、离线可用。
2. **定期自动热更新**：后台周期性从 superpowers 的 GitHub Release 拉取新版技能，校验后原子替换本地副本，失败回退，不打断使用。
3. **编码场景自动启用**：当判定当前会话是"编码场景"时，自动把 superpowers 技能纳入 agent 上下文并注入 `using-superpowers` 引导，使 brainstorming / TDD / systematic-debugging 等技能按需自动触发（而非死重）。

**非目标（本期不做）**
- 不做 settings 页面的可视化开关（v1 用环境变量 + 默认开启；UI 开关列为后续项）。
- 不做引擎主循环级的按文件路径 glob 细粒度激活（`ConditionalSkillManager` 接入引擎循环）——降级为可选 Phase 3。
- 不向 superpowers 上游提交任何 PR。
- 不改动 superpowers 技能正文（保持上游语义与署名；仅按平台做必要的加载适配）。

## 2. 背景与关键事实（已代码核实，file:line 见附录）

**superpowers 的本质**：不是可执行插件，而是一组带 `name` + `description`（"Use when…" 触发语）的 markdown 技能。真正让它"活"起来的是会话开始时注入的 `using-superpowers` 引导——它确立"动手前先检查是否有匹配技能"的铁律，技能才会自动触发。仅把 md 拷进目录、或要求逐会话手动开启，按其自身定义**不算真集成**。

**nomifun 已有的两套技能系统（互不读取对方目录）**
- **nomi 引擎内核** `nomi-skills`：`SKILL.md` 同构格式；`load_all_skills(cwd, add_dirs, bare, mcp)` 读取 bundled + `~/nomi/skills` + `.nomi/skills` + **`add_dirs`（额外根）**；`AgentBootstrap` 已有 **`extra_skill_dirs` builder**；技能清单以 `<system-reminder>…skills…</system-reminder>` 注入 system prompt；`SkillTool` 供模型按需加载技能正文（**仅 inline；nomi 后端未挂 spawner，fork 技能不可用**）。还提供 `ConditionalSkillManager`（按 `paths:` glob 激活，已实现但**未接入引擎循环**）与 `SkillWatcher`（防抖热重载）。
- **后端技能 Hub** `nomifun-extension`：内置技能语料库 `include_dir!(assets/builtin-skills)`；`startup_materialize::materialize_if_needed` 以 **指纹 + 版本门控 + fs2 锁 + staging 原子 rename + Windows 重试** 物化到 `{data_dir}/builtin-skills`；`link_workspace_skills(workspace, [".claude/skills"], skills)` 把技能软链（Windows 用 NTFS junction，失败回退拷贝）进 ACP 工作区供外部 Claude Code/codex 用；Hub `installer.rs` 有安装/更新/热重载脚手架，但**远程下载是 stub**（`installer.rs:99`）。

**关键差异（决定架构）**：nomi 引擎**不读** `{data_dir}/builtin-skills`。故"单一来源统一分发"必须是：**一个物化目录 + 两条喂入通道**（nomi 用 `extra_skill_dirs`，ACP 用 `link_workspace_skills`）。

**编码场景检测**：仓库内**没有**运行时"自然语言意图分类器"。会话级已具备"按条件注入常驻行为提示"的先例（`factory/nomi.rs` 的 `compose_subagent_hint`）。superpowers 属通用编码方法论，会话级"是否编码场景"是恰当粒度；按 `*.rs` 等文件 glob 的细粒度激活对 superpowers 无额外收益且代价高。

## 3. 架构总览（方案 A：单一来源 + 双路径分发 + 场景引导）

```
                 ┌─────────────────────────────────────────────┐
   出厂内置        │  embedded 基线语料库（include_dir!，随版本指纹）  │
   (baseline)     └───────────────┬─────────────────────────────┘
                                  │ 启动物化(指纹门控/原子写/回退)
                                  ▼
   热更新覆盖      ┌─────────────────────────────────────────────┐
   (overlay)      │  {data_dir}/superpowers/  ← 有效技能目录        │◄──┐
                  └───────────────┬─────────────────────────────┘   │ 周期 janitor:
                                  │                                   │  GitHub Release
             ┌────────────────────┴───────────────────┐             │  → 校验 → 原子替换
             ▼                                          ▼             │  → 触发 reload
   nomi 引擎 (extra_skill_dirs)              ACP (link_workspace_skills)
   → 技能清单注入 system prompt               → 软链进 .claude/skills
             │                                          │
             └──────────────┬───────────────────────────┘
                            ▼
              编码场景判定 → 注入 `using-superpowers` 引导
              （nomi: factory/nomi.rs 追加 system_prompt；
                ACP: 新增 PreSendHook / first_message_injector）
```

有效技能目录取"overlay 优先，缺省 baseline"：出厂离线可用，联网自动新鲜，下载失败永不劣化。

## 4. 详细设计（分组件）

### 4.1 内置语料库与物化
- 在仓库内新增 superpowers 语料库（14 个技能的 `SKILL.md` 及其 `references/`；保留上游 `LICENSE`/署名）。放置于独立资产目录（**不**混入 `assets/builtin-skills/`，避免污染现有 ACP 内置技能语义与 `skill-tags.json`），例如 `crates/backend/nomifun-extension/assets/superpowers/`，以新的 `include_dir!` 常量嵌入。
- 复用 `startup_materialize` 同型逻辑（指纹门控 + 原子写 + 回退），物化 baseline 到 `{data_dir}/superpowers-baseline/`。
- "有效目录"解析：若存在 overlay（`{data_dir}/superpowers/`，来自热更新且校验通过）则用之，否则用 baseline。对外暴露 `effective_superpowers_dir(data_dir) -> PathBuf`。
- 平台适配：仅内联方法论所需的 md；上游的 `scripts/`（visual companion server、sdd 脚本等）对 nomi 引擎无意义且 fork 不可用，物化时可保留但不激活（不注册为可执行）。`using-superpowers` 的平台 `references/` 保留（ACP=Claude Code 时有用）。

### 4.2 nomi 引擎喂入
- `NomiAgentManager::new`（`manager/nomi/agent.rs`）构建 `AgentBootstrap` 时，若判定编码场景，调用 `.extra_skill_dirs(vec![effective_superpowers_dir])`，使 `load_all_skills` 纳入 superpowers，技能清单经 `build_system_prompt`（`context.rs:247`）自动进入 `<system-reminder>`。
- 引导注入：在 `factory/nomi.rs` 的 system_prompt 组装链（`compose_subagent_hint` 同款位置）追加 `using-superpowers` 引导文本（按平台改写为 nomi 语气：说明"编码前先检查并使用匹配技能；用 Skill 工具加载"）。仅编码场景注入。
- 约束：superpowers 技能一律 **inline**（nomi 后端无 spawner）。

### 4.3 ACP 喂入
- 对 ACP 会话，将有效 superpowers 目录下的技能经 `materialize_skills_for_agent` + `link_workspace_skills(workspace, [".claude/skills"], …)` 链入工作区，外部 Claude Code/codex 原生识别。
- 引导注入：新增一个 `PreSendHook`（`prompt_pipeline.rs` 的 `trait PreSendHook`）或扩展 `first_message_injector` 的 `InjectionConfig`，在编码场景下于首条消息前置 `using-superpowers` 引导。
- ACP 路径依赖外部 CLI 环境，端到端需真机联调（见 §8）。

### 4.4 编码场景判定（分层，可配置）
- **L1（默认，会话构建期）**：满足任一即判为编码场景——工作区是 VCS 根 / 含已识别工程清单（`Cargo.toml`/`package.json`/`pyproject.toml`/`go.mod` 等）；或会话启用了文件编辑类工具（Write/Edit/Bash）；或 agent/伙伴带 `coding` 场景标签（`skill-tags.json` 已有 `scenario_tags:["coding"]` 与 `audience_tags:["developer"]` 概念可复用）。
- **L3（本期不做）**：对首条消息做意图分类（YAGNI）。
- **L2（可选 Phase 3）**：接入 `ConditionalSkillManager::activate_for_paths` 到引擎循环，实现 mid-session 细粒度激活。
- 判定结果只决定"是否注入引导 + 是否喂入 superpowers 目录"。判定为否时对现有行为零影响。

### 4.5 定期热更新
- **来源**：superpowers GitHub Release。复用 `VersionCheckService`（`version.rs`，`new_dynamic` + GitHub releases API，代理感知 http client，无鉴权 60 req/hr）查询最新 tag；下载 release 的 source zip（`zipball_url` 或 `…/archive/refs/tags/<tag>.zip`）。仓库常量指向 `obra/superpowers`（独立于应用自更新的 `NOMIFUN_GITHUB_REPO`）。
- **调度**：仿 `state.rs:1481 spawn_idmm_record_janitor` 的 `tokio::spawn + interval` janitor，在 `create_router`（`routes.rs`，事件总线→WS 桥接后）启动 `spawn_superpowers_updater`。首 tick 立即触发（启动即检查）。默认周期 **每 6 小时**（远低于 GitHub 限流），带超时（`tokio::time::timeout` 包裹，因 `nomifun_net::http_client` 无超时）。
- **安全护栏**：仅认打 tag 的 Release；下载后 zip 安全解压（复用 `extract_zip_archive`+`safe_zip_entry_path`+`reject_zip_symlink`，需从 `skill_service.rs` 提升为共享模块）；host 允许清单（`github.com`/`codeload.github.com`/`objects.githubusercontent.com`，跟随重定向）；可选 sha256 校验（`sha2` 已在依赖）；staging → verify → 原子替换（复用 `startup_materialize` 的 rename+`retry_startup_file_op` 处理 Windows 共享冲突 5/32/33）；per-target 锁防并发。
- **落地与生效**：替换 `{data_dir}/superpowers/` overlay → 广播 `event_bus.broadcast(WebSocketMessage::new("superpowers.updated", …))`（经现有 routes.rs 桥自动到 UI）。**新会话**自然读到新目录；已存活会话本期不强制热切（Phase 3 用 `SkillWatcher` 补）。
- **开关**：默认开启；环境变量 `NOMIFUN_SUPERPOWERS_AUTOUPDATE=0` 关闭，`NOMIFUN_SUPERPOWERS_UPDATE_INTERVAL_SECS` 调周期。（settings-UI 开关列为后续。）

### 4.6 依赖与错误类型
- 把 superpowers 的"嵌入 + 物化 + 下载 + 校验 + 解压 + 原子替换 + 有效目录解析"做成一个内聚模块，**放在 `nomifun-extension`**——因为 zip 安全解压(`extract_zip_archive`)、物化(`startup_materialize`)、链接(`link_workspace_skills`)、指纹全在此 crate，复用面最大。
- `nomifun-extension/Cargo.toml` 当前无 `reqwest`/`nomifun-net`；**给它新增 `nomifun-net` 依赖**（shared crate，无循环风险），下载统一走 `nomifun_net::http_client()`（代理感知），并按 nomi-providers 模式补 connect/read timeout 或用 `tokio::time::timeout` 包裹。
- 周期 janitor 任务本身留在 `nomifun-app`（`create_router`），调用 extension 暴露的更新入口；`VersionCheckService`（`nomifun-system`）用于查询 GitHub Release（`nomifun-app` 已能构造它）。
- 现有 zip 安全函数 `extract_zip_archive`/`safe_zip_entry_path`/`reject_zip_symlink` 目前私有于 `skill_service.rs`——提升为 crate 内共享模块（如 `zip_safe.rs`）供 skill_service 与新模块共用。
- 新增错误变体：网络/下载失败、校验失败、解压失败（映射到 `AppError`）。全部按 warn-and-retry-next-tick 处理，不 panic、不阻断。

## 5. 分期交付

- **Phase 1（核心，先交付）**：内置语料库 + baseline 物化 + 有效目录解析 + nomi 喂入(`extra_skill_dirs`) + 场景判定(L1) + nomi 引导注入。→ 满足"内置 + 编码场景自动启用（nomi 路径）"。全 cargo 可测。
- **Phase 2（热更新）**：共享化 zip 安全解压 + superpowers 下载/校验/原子替换模块 + 周期 janitor + WS 广播 + 环境变量开关。→ 满足"定期自动热更新"。全 cargo 可测（mock GitHub + 坏包用例）。
- **Phase 2.5（ACP 路径）**：`link_workspace_skills` 链入 + ACP 引导 `PreSendHook`。→ 补齐"统一"的 ACP 半边；逻辑单测可覆盖，端到端需真机联调。
- **Phase 3（可选增强）**：`ConditionalSkillManager` 接入引擎循环 + `SkillWatcher` 活会话热切 + settings-UI 开关（含 DB 迁移与前端）。

## 6. 数据流（编码会话，nomi 路径）
1. 启动：embedded baseline 物化到 `{data_dir}/superpowers-baseline/`（指纹未变则跳过）。
2. janitor 首 tick：查 GitHub → 有新版则下载校验替换 overlay。
3. 新会话构建：判定编码场景 → `effective_superpowers_dir` 传入 `extra_skill_dirs` → `load_all_skills` 纳入 → 技能清单入 system prompt；`factory/nomi.rs` 追加 `using-superpowers` 引导。
4. 运行：模型见到引导与技能清单 → 命中场景时用 `Skill` 工具加载 brainstorming/TDD 等正文并遵循。

## 7. 错误处理与降级
- 下载/网络失败 → 保留上次 overlay 或 baseline，warn，下个 tick 重试。
- 校验/解压失败 → 不替换、告警、保留旧副本。
- 语料库单个技能解析失败 → 跳过该技能，不影响其余（沿用 loader 容错）。
- 场景误判为否 → 仅"未增强"，绝不打断正常对话。
- 全程不 panic；janitor 单次失败不影响循环。

## 8. 测试策略（TDD）
- **物化/指纹/有效目录**：baseline 首次物化、指纹未变跳过、overlay 优先、缺 overlay 回退。
- **下载模块**：mock GitHub（wiremock/本地）→ 正常安装；坏 zip（zip-slip/符号链接/`..`）被拒；host 不在白名单被拒；sha256 不符被拒；下载失败保留旧副本；原子替换在 Windows 路径成立（重试逻辑）。
- **场景判定**：各 L1 信号命中/不命中；非编码场景不注入。
- **nomi 喂入**：`extra_skill_dirs` 含 superpowers 时技能清单出现对应条目；引导文本仅在编码场景出现（可查 system_prompt 片段，参考 `context.rs` 现有排序测试）。
- **ACP**：`link_workspace_skills` 幂等、Windows junction/拷贝回退；PreSendHook 在编码场景前置引导。
- **验收测试（交付判定）**：编码场景会话收到"帮我做个 React todo list"应触发 brainstorming（先设计后写码）而非直接开写；nomi 路径以单测/脚本近似验证引导生效；ACP 路径真机联调。
- 前端若涉改：以 `bun run build` 验证（非仅 tsc）。

## 9. 决策记录
1. 注入强度：**门控绑定场景**——编码场景注入引导（技能自动触发，含 brainstorming），非编码不打扰。（用户默认 #1 + superpowers "真集成"定义调和）
2. 热更新：**默认自动、周期拉取**（遵用户原始"定期自动"需求），带安全护栏与开关。（覆盖我最初的"手动"默认）
3. 来源：**GitHub Release + 校验**，不追 main。
4. 范围：**全 14 技能内置**，编码场景启用。
5. 场景判定：**L1 会话级**为主，L3 不做，L2 归 Phase 3。

## 10. 后续项（交付后）
- settings-UI 的自动更新开关（DB 迁移 + api-types + 前端）。
- Phase 3：引擎循环级 `activate_for_paths` + `SkillWatcher` 活会话热切。
- ACP 真机端到端联调（外部 Claude Code/codex）。

## 附录：核实过的关键锚点（file:line）
- 引擎主循环/工具分发：`nomi-agent/src/engine.rs:652,693,1004-1052`；`orchestration.rs:24`。
- system prompt/缓存/技能段：`nomi-agent/src/context.rs:17,42,146,247-267`。
- 条件激活/watcher/发现：`nomi-skills/src/conditional.rs:39,79,107,155`；`watcher.rs:63,100`；`discovery.rs:56,130`。
- 加载/bundled/预算：`nomi-skills/src/loader.rs:42`；`bundled/mod.rs:50,62,78,108`；`prompt.rs:60`；`types.rs:107`。
- SkillTool/bootstrap：`nomi-agent/src/skill_tool.rs:24,38,101,185`；`bootstrap.rs:282,288,385,396,545,554,574`。
- nomi 工厂/ACP 管线：`nomifun-ai-agent/src/factory/nomi.rs:55-211,787`；`manager/nomi/agent.rs:~190,371-373,411-414`；`capability/prompt_pipeline.rs:24,46`；`first_message_injector.rs:34`；`skill_manager/mod.rs:12,264`。
- Hub/物化/zip/链接/路径：`nomifun-extension/src/hub/installer.rs:83,99,134,150,235,274`；`hub/index_manager.rs:33-52,86,92`；`startup_materialize.rs:47,148-198,227,268`；`registry.rs:153`；`skill_service.rs:22,41,47,115,735,789,982,1026,1433,1459,1485`；`error.rs:5`；`Cargo.toml`。
- 更新/调度/总线/配置/目录：`nomifun-system/src/version.rs:23,31,54,98`；`nomifun-net/src/lib.rs:3`，`proxy.rs:51`；`nomifun-realtime/src/broadcaster.rs:27,47`；`nomifun-app/src/router/routes.rs:67-73`，`state.rs:320,1481,1767`，`config.rs:11`，`cli.rs:29`；`nomifun-system/src/settings.rs:12`。
