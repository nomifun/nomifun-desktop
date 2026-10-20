# Computer Use 一步到位重构：实施方案

日期：2026-10-20（用户当日修订：Windows 本机先贯通，其他平台并行补验；允许当前用户桌面的专用测试窗口）。状态：实施中。原审计基线：`c3e75054203c907add82040fbb103579396534eb`（0.8.14）；本次规则修订对应 main `1d385e072571b2f71b90e7406aa3ab335aa8dd1c`，后续接续以实际 HEAD 为准。

本文件是本专项唯一现役施工与验收说明。目标是在**下一个发布版本**（建议 0.9.0，由发布负责人确定）中用 Cua Driver **全新实现** Computer Use：底层从零开始，旧实现整体删除；不迁移、不兼容旧数据与旧契约；依赖旧 Computer 的会话直接放弃并提示用户新建会话。

启动入口见[启动 prompt](computer-use-rebuild.start-prompt.zh.md)。实施前阅读根目录 [AGENTS.md](../../AGENTS.md)、[Agent Session](../architecture/agent-session.zh.md)、[全局 OS 能力](../architecture/global-os-capabilities.zh.md)、[系统许可](../architecture/system-permissions.zh.md)、[生命周期可靠性方案](agent-lifecycle-reliability.zh.md)（U01–U13 同样适用）与[开发验证](../contributing/development.md)。

**2026-10-20 用户修订依据与执行规则**：先在 Windows 本机贯通 S1、S2、S3；macOS、Linux、Wayland、其他 DPI/架构和真实模型验证并行补齐。缺少机器、桌面、签名或受控模型凭据只记为待验，不阻塞本机代码接入。允许使用当前用户桌面的专用测试窗口和临时数据；隔离账户、虚拟机或私有显示服务按测试需要选择，不是开发前置条件。质量与完整替换要求保持；平台支持和发布声明以对应证据为准。本修订替代此前要求三端门槛全部通过后才继续本机开发的执行限制。

调研与可行性证据保存在仓库外（只读参考，不是施工输入）：`C:\Users\rika0\code\nomifun\main\resources\computer-use-rebuild-design.zh.md`（选型调研）、`resources\_inventory.md`（21 个开源项目快照）、`resources\_probe\REPORT.md`（本机 Windows 构建与私有 worker 握手探针）。

---

## 1. 目标与交付边界

1. **一个内核，三端目标一致**：截图、无障碍树、输入、窗口与应用管理全部由 vendored Cua Driver 提供；NomiFun 不再维护任何自研原生桌面代码。Windows 先完成产品接入，其他平台保留同一架构并并行补验，未验能力不宣称已支持。
2. **全新开始**：旧 crate、旧动作契约、旧 Role Host、Windows 提权桌面桥、旧权限映射、旧 GUI 夹具、旧验证脚本、旧测试、旧文档全部删除。不写迁移代码，不读旧格式，不保留别名、开关或兼容分支。
3. **升级即用**：安装新版本后无需额外操作，新会话直接使用新 Computer；依赖旧 Computer 的会话变为只读并提示新建会话，历史记录保留（第 6.12 节）。
4. **任何支持工具调用的模型可用**：包括 OpenAI Chat 协议供应商；不支持图片输入的模型只用无障碍树工作。
5. **默认不打扰用户**：默认后台投递，不抢焦点、不移动真实鼠标；需要前台时可见、可停止。
6. **可靠可控**：Stop 立即撤回排队动作，在途动作按真实结果收束，卡死时强制回收；驱动崩溃自动恢复且不重放；任务完成后驱动进程自动退休；应用退出与崩溃都能回收。
7. **低成本**：动作、schema、工具标题与提示词都从一份冻结清单派生；测试只覆盖新实现；验证命令按改动范围最小化。
8. **开发验证与发布验收分开推进**：本机实现不等待其他平台或模型凭据。S4 按本次发布的平台、架构和功能范围完成安装包、原生及真实模型验收；脚本模拟只作确定性回归，未执行项不作可用性或平台支持结论。

**本版本不做**（见第 11 节，属于后续独立增量）：模型原生 computer tool（Anthropic/OpenAI/Gemini）、开源 GUI 模型专用档案（Qwen3-VL、UI-TARS）、`run_script`、Windows 提权窗口、沙箱/虚拟机、轨迹录制、经驱动的浏览器操作、Linux GNOME Shell helper 的自动安装。

## 2. 强制约束

| 编号 | 约束 | 执行要求 |
| --- | --- | --- |
| C01 | 一步到位 | 单个版本完成替换；不设功能开关、灰度、新旧并存或"旧实现兜底"。 |
| C02 | 全新开始，不兼容旧 | 删除旧动作 ID、旧 schema、旧 provider、旧配置键、旧 API 路由；不写迁移、映射、对账或旧数据清理代码；任何新代码都不解析旧格式。 |
| C03 | 旧会话直接放弃 | 依赖旧 Computer 的会话不尝试修复或转换，复用现有契约拒绝路径提示用户新建会话；历史只读保留。 |
| C04 | 只测新的 | 只为删除代码服务的旧测试随代码删除，不改写适配；不为旧数据、旧会话、升级路径编写测试或夹具。 |
| C05 | 不堆屎山 | 删除优先于保留；模块单一职责；不手抄 schema 与文案；不引入未使用的抽象。 |
| C06 | 成熟开源内化 | 驱动行为以上游为准，不重写、不分叉逻辑；本地补丁最少且全部登记。 |
| C07 | 能力不下降 | 覆盖截图、无障碍观察、点击/输入/按键/滚动/拖拽、窗口聚焦、启动应用；新增剪贴板、批量执行、后置条件校验、窗口排布、菜单调用。 |
| C08 | 遵守现役架构 | AgentSession/Turn/Event/Payload/Effect 是唯一事实链；不新增账本、broker、TTL 或工作门槛；不修改 canonical 存储；UI 只支持 880×600 及以上。 |

## 3. 已定技术决策

以下决策已确定，实施中不再重新讨论；只有必须改变产品合同或出现无法推断的关键事实时才提出问题。

| 编号 | 决策 |
| --- | --- |
| D01 | **驱动内核**：Cua Driver（trycua/cua `libs/cua-driver`，MIT）。起始快照固定 commit `4d959bbfa57f0c89f310330e849500b222084194`；`cua-driver-rs-v0.37.0` 是请求的版本标签，标签解析差异按 `SOURCE.json` 记录，固定 commit 为准。 |
| D02 | **拓扑**：NomiFun 自有私有 worker 进程 `nomifun-computer-worker`。宿主直接 spawn（macOS 保持 TCC 责任链），通过继承的 stdin/stdout 使用上游私有 worker 协议 v1（JSON 行）。不链接进主进程，不使用 daemon、MCP、HTTP 或 socket。 |
| D03 | **vendor 范围**：`vendor/cua-driver/` 只包含 `cua-driver-sdk`、`cua-driver-core`、`cua-driver-contract`、`platform-macos`、`platform-windows`、`platform-linux`、`cursor-overlay`、`cua-cursor-motion`、`pip-preview`，以及上游 `Cargo.lock`、LICENSE、`THIRD_PARTY_NOTICES.md`、`VERSION`、`contract/manifest.json` 和这些 crate 编译所需的资源。不引入上游 `cua-driver` 二进制 crate（遥测、更新、技能安装、扩展下载、HTTP）、`cua-perception`（AGPL 模型）、`cua-driver-uia`、Python/TypeScript 绑定与测试夹具。 |
| D04 | **最小登记补丁**：保留 platform-* 对 `cua-driver-core` 的 `default-features = false` 补丁，去掉 `rego`/`yaml`/`script`；同时承认用户已授权的输入崩溃清理等必要最小缺陷补丁。补丁以恢复已定行为为限，全部登记在 `vendor/CUA-DRIVER-PATCHES.md` 并经同步脚本应用，不再限定只能一个，不直接修改 vendored 文件。运行时写入位置通过环境变量收敛。 |
| D05 | **无遥测、无更新、无网络**：worker 只走 stdio；驱动的浏览器工具不暴露。 |
| D06 | **动作模型**：一个驱动工具 = 一个 NomiFun Action = 一个模型工具（第 6.4 节共 22 个）。保留能力 `computer`、角色 `system.computer_use`、资源种类 `computer`、`ResourceId` 为 `local-desktop`、资源操作 `observe/input/launch`、cargo feature `computer-use`；包与挂载 ID 改为 `nomifun.computer`/`domain-computer`。 |
| D07 | **冻结清单是唯一来源**：同步脚本从 worker 的 `list` 输出生成 `computer-tools.v1.json`，合并一份精简覆盖文件（标题、精简描述、删除的参数）。Wave2 动作定义、input schema、工具展示名与中英文标题都由它派生，不手写。只有一套精简档，供所有协议共用。 |
| D08 | **能力分档**：按当前路由 `ToolCalls`/`ImageInput` 在每次模型请求前计算：完整档 / 无图档（只给无障碍树，只允许 `element_token` 寻址）/ 不可用。只裁剪下发给模型的工具与参数，不改变冻结授权。 |
| D09 | **效果规则**：观察类只读、不建 Effect；动作类建 Effect，Computer 域 `resource_key` 为空（不进入跨会话唯一围栏）。`confirmed/partial/unverifiable/suspected_noop` 结算为成功并附驱动证据；派发前拒绝与排队撤回为零效果；只有 worker 崩溃、超时强杀或传输丢失记为 unknown，沿用现有 unknown 效果处理，不新增对账接口，不自动重放。 |
| D10 | **Stop**：排队请求立即撤回；在途动作等待真实结果（以驱动工具预算加余量为上限，界面显示"正在等待当前桌面操作结束"）；超过上限视为卡死，强杀 worker，该动作记 unknown，下一次调用自动启动新代际。 |
| D11 | **前台可见**：遵循默认可信 Agent 原则，Agent 可自行选择前台投递；驱动光标覆盖层常开并显示 Agent 名；会话中的工具调用行对前台投递与桌面坐标动作显示"前台"标记。不新增事件通道或弹窗。 |
| D12 | **平台目标与实测边界**：Windows 10/11 普通完整性优先接入（提权窗口返回结构化拒绝）；macOS 14+、Linux x86_64 的 X11/XWayland + AT-SPI 并行补验。原生 Wayland 按真实桌面 POC 得出的固定规则自动启用，不提供设置项；暂无结论时继续已证明的路由并记录待验，不阻塞 Windows 开发。GNOME Shell helper 只在文档说明手动安装。平台支持以对应编译、安装包和原生证据为准。 |
| D13 | **范围**：剪贴板读写纳入本版本；`run_script`、浏览器工具、录制/回放、`kill_app`、视觉解析不暴露。 |
| D14 | **UI/API 契约版本**：当前 main 已由其他工作升为 42，本专项完整替换时由 42 升为 43，保留他人的 42 改动，新旧前后端不混用。实施时复核实际 HEAD；若后续版本已被并发改动占用，继续递增，不覆盖或回退。 |

## 4. 现状与退役原因（审计摘要）

| 编号 | 事实 | 位置 |
| --- | --- | --- |
| R01 | **跨会话桌面围栏**：唯一索引 `idx_agent_effects_resource_unsettled`；Computer 的 `resource_key` 按用户只有一个；输入出现未证明拒绝的错误即写入 unknown，此后该用户所有会话的 Computer 输入/启动准入失败（模型看到 `CAPABILITY_UNAVAILABLE`）。 | `nomifun-db/migrations/001_canonical_baseline.sql`；`router/agent_wave2_host.rs`；`router/agent_role_host.rs` |
| R02 | **非标准模型契约**：4 个自定义工具，输入必填 `expected_generation` 且需匹配同类型的最近观察。 | `nomifun-agent-domain-wave2/src/lib.rs`；`agent_role_host.rs` |
| R03 | **截图门槛**：路由未声明 `ImageInput` 即报错，没有无图降级。 | `router/engine_computer_media.rs` |
| R04 | **前台全局输入**：enigo 全局合成，抢焦点、移动真实鼠标。 | `nomi-computer/src/input.rs` 等 |
| R05 | **复杂度与验证脱节**：约 1.65 万行自研原生代码 + 3,591 行 Role Host + Windows 提权桥；验收使用脚本模型供应器，从未用真实模型证明可用性。 | `crates/agent/nomi-computer`、`crates/agent/nomi-a11y`、`windows_computer.rs` |

结论：整体设计问题，全部删除，不保留任何部分。

## 5. 目标架构

```text
NomiFun Desktop（Tauri，登录用户身份）
├─ UI：能力工作台 / 权限与驱动状态 / 工具调用展示（含"前台"标记）
├─ Agent Runtime + 模型路由：按 ToolCalls/ImageInput 分档下发 computer_* 工具
├─ Kernel + AgentSession 事实链（唯一事实来源，存储不改动）
├─ Wave2 Computer 模块（22 个 Action，由冻结清单派生）→ router/computer_host.rs
└─ crates/backend/nomifun-computer
   ├─ protocol   私有 worker 协议 v1 类型
   ├─ worker     spawn/父死回收/关停/代际 + 公平排队/单在途/超时/Stop + 驱动会话
   ├─ result     结果归一化（文本/图片/结构化/next_call）与效果结论
   ├─ surface    冻结清单、分档、可信字段、提示词
   └─ permissions macOS 许可探测与申请
        │ 直接 spawn + 继承 stdio（JSON 行）
        ▼
nomifun-computer-worker（apps/computer-worker，独立 Cargo 工作区）
└─ vendored Cua Driver：sdk / core / contract / platform-{macos,windows,linux} / cursor-overlay / cua-cursor-motion / pip-preview
```

调用链：模型工具调用 → Kernel 授权与 Effect 准入 → `computer_host` 校验资源绑定与动作 → `nomifun-computer` 剥离/注入可信字段、排队、发送 `call` → worker 返回 MCP 风格结果（`content` 文本/图片、`structuredContent`、`isError`）与 `completion` → 归一化为模型结果与效果结算。

## 6. 详细设计

### 6.1 vendor 快照与同步

- `vendor/cua-driver/`（D03 范围）、`vendor/CUA-DRIVER-PATCHES.md`、`vendor/patches/cua-driver/*.patch`。
- `scripts/vendor/sync-cua-driver.mjs`：按 tag 稀疏拉取上游（整库含大文件，使用 `--filter=blob:none --sparse`）→ 拷贝 D03 范围 → 应用补丁 → 记录来源 → 构建 worker → 用 `list` 重新生成冻结清单并输出差异供审阅 → 运行协议测试。
- `check:computer-boundary`：vendor 文件等于"上游快照 + 登记补丁"；仓库中不存在旧符号（见第 8 节 S2 门槛）。
- 根 `Cargo.toml` 的 workspace `exclude` 加入 `vendor/cua-driver` 与 `apps/computer-worker`；主工作区不依赖任何 vendored crate。

### 6.2 worker 二进制

- `apps/computer-worker/`：独立 `[workspace]`，以上游 `Cargo.lock` 为起点，使用与主工程相同的 stable 工具链。
- `src/main.rs` 改编自上游 `cua-driver/src/private_worker.rs` 与 `main.rs` 的 worker 启动段（保留 MIT 署名）：只接受 `__private-worker --generation <id>`；处理 `initialize/metadata/list/call/bind_session/close_session/shutdown`；worker 逻辑在后台线程，macOS 主线程运行光标覆盖层；不注册宿主扩展工具，不接 perception。
- Windows：复刻上游 DPI 感知 manifest、`.rc` 与 `embed-resource` 构建步骤。
- Linux：启用 `portal-input`；`portal-capture` 与原生 Wayland 启用规则在具备真实环境时补验。该结论不作为 Windows 接入的前置条件。
- `--self-check`：只执行 initialize/list/shutdown，供打包与安装后自检。

### 6.3 `crates/backend/nomifun-computer`

| 模块 | 职责 |
| --- | --- |
| `protocol` | 私有 worker 协议 v1 的 serde 类型（`ChannelRequest/Response`、`ActionCompletion`、`WorkerInitialization`、`ConfiguredDriverOptions`、`RuntimeAuthorizationOptions`、`TrustedSessionOptions`），与快照逐字段一致；初始化默认 standard 模式、`unrestricted_acknowledged:false`。 |
| `worker` | 复用 `nomi-process-runtime`（MCP stdio 同一 owner 模式）：直接 spawn、干净环境 + 白名单变量、Windows Job Object/Linux 父死信号/macOS watchdog、`shutdown` 超时后强杀、代际管理；按 Agent 会话公平排队、全局单在途、按工具预算超时、D10 Stop；Agent 会话首次调用 `bind_session`（`public_session` 为 Agent 名），会话退役时 `close_session`，驱动会话过期时透明重绑。 |
| `result` | `call` 结果转为文本 + 图片 parts；`structuredContent` 保留为紧凑 JSON；`next_call.tool` 改写为 `computer_` 前缀；无图档去除图片；输出效果结论（区分 worker 层错误 `worker_request_failed`/`session_not_bound`/`generation_mismatch` 与工具层 `isError`/`refused`，确定"已证明未派发"集合）。 |
| `surface` | 加载冻结清单并按平台过滤；派生 Action→驱动工具映射与 EffectClass；计算分档；剥离与注入可信字段；提供 Computer 能力的系统提示片段（由上游 MCP instructions 与 `SKILL.md` 核心规则精简而来，随清单版本化）。 |
| `permissions` | macOS `AXIsProcessTrusted(WithOptions)`、`CGPreflight/RequestScreenCaptureAccess` 在 NomiFun 进程内探测与申请；授权变化后重启 worker 代际。 |

worker 环境变量白名单以私有运行时目录收敛写入：`CUA_DRIVER_RS_HOME=<NomiFun 数据目录>/computer-driver`、`CUA_DRIVER_CURSOR_THEME_DIR=<同目录>/cursor-themes`；`HOME`/`USERPROFILE` 指向私有 driver home，Linux 的 `XDG_CONFIG_HOME`、`XDG_STATE_HOME`、`XDG_DATA_HOME`、`XDG_CACHE_HOME` 指向该运行时目录。固定快照的 portal 令牌实际位于 `XDG_CONFIG_HOME`，其他默认配置也可能读取 home，不能只设置 `CUA_DRIVER_RS_HOME` 后仍读取用户已有 Cua 配置。显示、授权总线和临时目录按当前测试或产品运行环境传入，按 D12 设置 `CUA_DRIVER_RS_ENABLE_WAYLAND`，保留上游允许的窗口变化等待/按键节奏变量。

### 6.4 动作与工具

全部条目由冻结清单生成，不手写。

| Action ID | 模型工具 | 驱动工具 | EffectClass | 资源操作 | 默认加载 | 分档 |
| --- | --- | --- | --- | --- | --- | --- |
| `computer/list_apps` | `computer_list_apps` | `list_apps` | ReadSensitive | observe | 常驻 | 全部 |
| `computer/list_windows` | `computer_list_windows` | `list_windows` | ReadSensitive | observe | 常驻 | 全部 |
| `computer/get_window_state` | `computer_get_window_state` | `get_window_state` | ReadSensitive | observe | 常驻 | 无图档强制 `include_screenshot:false` |
| `computer/get_desktop_state` | `computer_get_desktop_state` | `get_desktop_state` | ReadSensitive | observe | 常驻 | 仅完整档 |
| `computer/zoom` | `computer_zoom` | `zoom` | ReadSensitive | observe | 延迟 | 仅完整档 |
| `computer/verify_state` | `computer_verify_state` | `verify_state` | ReadSensitive | observe | 延迟 | 全部 |
| `computer/get_guide` | `computer_get_guide` | `get_guide` | Pure | observe | 延迟 | 全部 |
| `computer/click` | `computer_click` | `click`（含 button/count） | Physical | input | 常驻 | 无图档仅 `element_token` |
| `computer/type_text` | `computer_type_text` | `type_text` | Physical | input | 常驻 | 同上 |
| `computer/press_key` | `computer_press_key` | `press_key` | Physical | input | 常驻 | 全部 |
| `computer/hotkey` | `computer_hotkey` | `hotkey` | Physical | input | 常驻 | 全部 |
| `computer/scroll` | `computer_scroll` | `scroll` | Physical | input | 常驻 | 无图档仅 `element_token` |
| `computer/drag` | `computer_drag` | `drag` | Physical | input | 延迟 | 仅完整档 |
| `computer/move_cursor` | `computer_move_cursor` | `move_cursor` | Physical | input | 延迟 | 仅完整档 |
| `computer/set_value` | `computer_set_value` | `set_value` | Physical | input | 延迟 | 全部 |
| `computer/invoke_menu` | `computer_invoke_menu` | `invoke_menu` | Physical | input | 延迟 | 全部 |
| `computer/run_steps` | `computer_run_steps` | `run_steps` | Physical | input | 常驻 | 步骤受同样分档约束 |
| `computer/bring_to_front` | `computer_bring_to_front` | `bring_to_front` | Physical | input | 延迟 | 全部 |
| `computer/set_window_frame` | `computer_set_window_frame` | `set_window_frame` | Physical | input | 延迟 | 全部 |
| `computer/launch_app` | `computer_launch_app` | `launch_app` | ExecuteLocal | launch | 常驻 | 全部 |
| `computer/clipboard_read` | `computer_clipboard_read` | `clipboard_read` | ReadSensitive | observe | 延迟 | 全部 |
| `computer/clipboard_write` | `computer_clipboard_write` | `clipboard_write` | Physical | input | 延迟 | 全部 |

- `computer_run_steps`：宿主逐步校验每个 step 的工具属于已授权 Action 且在本表内（剥离前缀后转发）；任一不满足则派发前拒绝。
- 宿主内部调用（不暴露给模型）：会话、光标、`check_permissions`、`health_report`。其余驱动工具一律不暴露。
- 官方模板 `assistant.general`、`companion.default` 在 seed manifest 中授予全部 22 个动作。
- 表中“常驻”表示冻结定义的默认 eager；完整 General 等大工具表仍沿现役全局工具窗口延迟披露并通过 ToolSearch 加载。Computer 不绕过或扩大该窗口，宿主重新校验工具表时保留已有的展示延迟；这不改变动作授权或平台 schema。

### 6.5 宿主接入

- 新建 `router/computer_host.rs` 实现 `RoleHostInvoker`（取代被删除的 `agent_role_host.rs`）：沿用通用的角色、provider、snapshot、registry generation 与唯一资源绑定校验，映射 Wave2 typed operation 到驱动工具，调用 `nomifun-computer`，按 6.6 节开始/结束 Effect。
- `nomi_core_builtins.rs` 安装新 invoker；`nomi_core_wave2.rs`、`nomi_core_role_defaults.rs`、`nomi_core_resource_bindings.rs`（`resolve_computer` 只判断平台与编译特性，许可在调用时检查）使用新包/挂载 ID。
- `engine_computer_media.rs` 只保留 Browser 截图投影；Computer 图片由 `result` 模块直接产出 typed image parts。
- Kernel 的 `InvocationControl` 继续传入，用于排队撤回与 Stop。

### 6.6 效果与 Stop

| 驱动结果 | 结算 | 给模型 |
| --- | --- | --- |
| 派发前拒绝（`refused`、`completion:not_started`、参数/目标/会话/授权错误） | failed（零效果） | 错误码 + `hint` + `next_call` |
| 缺少系统许可（宿主预检） | rejected（零效果），`SYSTEM_PERMISSION_REQUIRED` 载体 | 未执行，授权后重新观察 |
| `confirmed`/`partial`/`unverifiable`/`suspected_noop` | succeeded，附 `effect`、`evidence`、`delivered_count` | 结果与证据；未校验时提示重新观察 |
| Stop 时仍在队列 | 撤回，零效果；已建立的 Effect 沿现有 failed/INVOCATION_CANCELLED 结算，不新增存储终态 | — |
| worker 崩溃、超时强杀、传输丢失（`completion:unknown`） | uncertain（现有 unknown 处理） | 结果未知，先观察，勿重放 |

- `agent_wave2_host.rs` 中 Computer 域 `resource_key` 为空（`EffectEventRequest.resource_key` 本为 `Option`）；`launch_app` 用 ManagedEffect，其余动作用 ExternalUncertainEffect。不修改 `nomifun-agent-session`。
- `nomifun-engine-core/src/system_permission.rs` 改为新动作映射：屏幕录制对应带像素的观察（`get_desktop_state`、`zoom`、完整档 `get_window_state`），辅助功能对应无障碍观察与全部输入/窗口/剪贴板动作，`launch_app` 不依赖两者。
- Stop 见 D10。生命周期专项中旧 Computer 的条目（R08、T17、OS05 的 Computer 部分）由本实现承接，旧验收记录不再作为证据。

### 6.7 模型侧

- **分档**：在现有 `route_image_input` 同源位置计算 `ToolCalls`/`ImageInput`；无 `ToolCalls` 时不下发 Computer 工具并在能力工作台说明原因；无图档按 6.4 节裁剪，宿主拒绝像素坐标寻址并提示改用 `element_token`。
- **清单精简**：上游核心 10 个工具原始描述 + schema 约 4.86 万字符。精简档压缩描述，删除 `capture_mode`、`max_dimension`、与 `target` 重复的平铺 `scope/pid/window_id`（仅限接受 `target` 的工具）等参数，长指南改由系统提示片段或 `computer_get_guide` 提供；延迟加载的工具不进首轮请求。上游 schema 不含 oneOf/anyOf/$ref，兼容 OpenAI Chat 协议供应商。
- **OpenAI Chat 协议**：图片沿用现有 broker 逻辑，放在工具结果之后的 user 附件消息中；Computer 工具活跃时设置 `parallel_tool_calls=false`；多工具调用按序执行，遇到失败即停止其余调用。
- **可信字段**：驱动会话只通过宿主持有的 `session_handle` 绑定；宿主注入 `max_image_dimension`（默认 1568）；剥离模型提供的 `session`、`max_image_dimension`、`max_dimension`、`screenshot_out_file` 与所有下划线前缀参数（这些字段也不进入冻结清单）。
- **历史**：沿用 `max_recent_images` 图片历史裁剪。

### 6.8 生命周期

- worker 懒启动；有已绑定会话或调用时保持；无会话且空闲 5 分钟自动关停（U13）。
- 应用退出：纳入现有桌面关停协调，在数据库关闭前 `shutdown`，超时强杀；崩溃由 stdin EOF（worker 读到 EOF 即退出）与平台父死机制兜底。
- 不在缓存或恢复路径中启动、恢复或重试任何动作（U07）。
- worker 退出不等于原生清理已完成。对仍等待第三方应用 ACK 的必要 native guardian 保留精确清理责任，只继续释放同一资源，不重放输入；S2 关停与新代际按现有清理状态如实显示待定，不能将其作为普通 Runtime/缓存引用或仅凭 PID 退出宣布退休。

### 6.9 权限与平台

- **macOS**：worker 由 NomiFun.app 进程直接 spawn，不经 LaunchServices；与 App 同一 Developer ID 签名、开启 Hardened Runtime，先签 worker 再签 App。许可归属 NomiFun.app，worker 继承，不出现第二个授权条目。
- **Windows**：worker 继承交互会话与普通完整性；提权窗口返回 `background_uipi_blocked` 等结构化拒绝；锁屏/RDP 断开由驱动报告。
- **Linux**：需要 AT-SPI2；X11/XWayland 与 AT-SPI 在可用环境中验证，原生 Wayland 按 D12。portal 首次授权属于系统行为。当前 Windows 接入不等待这些环境；未经验证的后端不记录为已验支持。

### 6.10 UI

- 设置 → 能力与权限 → Computer Use：macOS 许可行（沿用现有探测与"去开启"）；驱动状态卡（worker 版本、平台后端、Linux 会话类型/Wayland/AT-SPI、最近错误），数据来自新增 `GET /api/computer/status`；删除 `/api/computer/permissions*`。
- 能力工作台：Computer 卡片按"观察 / 操作 / 应用 / 剪贴板"分组展示精确动作集，默认全选；文案说明后台优先、可能的前台操作与平台限制。
- 会话：新工具名与标题（由清单生成）；前台投递或桌面坐标动作的工具行显示"前台"标记（D11）；许可缺失提示沿用 `SystemPermissionNotice`，更新动作映射。
- 运行 `bun run check:desktop-ui-boundary`。

### 6.11 打包与构建

- 各平台构建与 release 脚本先构建 worker，输出 `apps/desktop/binaries/nomifun-computer-worker-<target-triple>[.exe]`，由 Tauri `bundle.externalBin` 打入安装包；运行时从主程序同目录解析；开发模式由 dev 脚本构建并通过 `NOMIFUN_COMPUTER_WORKER` 指定。
- macOS：worker 签名与公证随 App，`codesign --verify --deep --strict` 覆盖嵌套二进制。Windows：worker 参与 Authenticode 签名，更新 `check:windows-installer` 合同。Linux：删除 GStreamer 构建预检与运行依赖；按 `readelf`/`ldd` 结果写入 deb/rpm 依赖（预期 libxkbcommon、libpipewire（若启用）、libX11/libXtst/libXi）；AppImage 包含 worker。
- NOTICE 与 component notices 加入 Cua Driver（MIT）、其 `THIRD_PARTY_NOTICES.md` 署名与 worker 依赖许可。

### 6.12 旧数据与升级影响（不迁移）

新版本不读取、不转换旧 Computer 数据，以下行为全部来自现有通用机制，不新增代码：

- **旧会话**：快照引用已删除的 Computer 包与动作，下一次运行时现有契约演进/编译拒绝路径返回 `AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED` 或 `CAPABILITY_ACTION_NOT_MATERIALIZED`，会话不能继续，历史只读保留。唯一改动是把这类拒绝的现有界面文案统一为"此会话依赖已更新的能力，请新建会话继续；已有记录会保留"，并确认实际返回的错误码命中该文案。伙伴默认模板的会话同样需要新建。
- **个人 Agent**：编辑器已有"缺失动作标为不可用、修正前不能保存"的通用处理；用户在 Agent 设置中重新勾选新的 Computer 动作即可。引用这类 Agent 的定时任务在用户修正前按现有错误提示失败。
- **官方模板**：seed manifest 更新后按现有流程生成新修订，新会话直接使用。
- **旧未结算 Computer 效果**：不处理。新实现的 Computer 效果不使用 `resource_key`，旧记录不会阻塞新会话；含旧记录的旧会话（每个用户至多一个）保持现有删除围栏。
- **配置**：删除 `ComputerConfig`；`ToolsConfig` 不拒绝未知字段，旧配置文件中的 `[tools.computer]` 被忽略。
- **macOS 许可**：许可属于 NomiFun.app，已授予的继续有效，无需重新授权。
- 以上内容写入发布说明，提示用户新建会话并在 Agent 设置中重新勾选 Computer。

### 6.13 文档

- 重写 `docs/guides/computer-browser-use.zh.md`/`.md` 的 Computer 部分。
- 新增 `docs/architecture/computer-use.zh.md`/`.md`（S5 定稿）：驱动与 worker、动作与分档、效果与 Stop、生命周期、平台边界。
- 更新 `docs/architecture/global-os-capabilities.zh.md`、`system-permissions.zh.md`、`overview`、`backend-crates`、`frontend`、`docs/contributing/building-and-packaging*`、生命周期方案中的 Computer 条目、`CHANGELOG.md`；删除旧 Computer 验证记录与交接文档中的 Computer 部分（以 Git 历史追溯）。

## 7. 删除清单

**整体删除**

- `crates/agent/nomi-computer/`、`crates/agent/nomi-a11y/`（含 examples、tests、`dev/` Docker 与 smoke 脚本）
- `crates/backend/nomifun-app/src/windows_computer.rs`
- `crates/backend/nomifun-app/src/router/agent_role_host.rs`
- `crates/backend/nomifun-app/examples/macos_computer_gui_fixture.rs`、`macos_computer_element_gui_fixture.rs`
- `scripts/validation/global-os-desktop-cases.mjs`、`global-os-linux-desktop-cases.mjs`、`global-os-windows-desktop-cases.mjs`、`global-os-windows-desktop-cleanup.mjs`、`linux-computer-native.sh`（其中非 Computer 用例先拆出保留）
- `ui/src/common/chat/computerPermissionContract.ts`（按新契约重写）
- `/api/computer/permissions*` 路由
- `nomi-config` 的 `ComputerConfig` 及其测试
- 工作区依赖中仅被旧 crate 使用的 `enigo`、`xcap`、`uiautomation`、`atspi`、`x11rb`、`gstreamer*` 等
- 所有只测试旧 Computer 行为的测试、fixture 与契约快照（直接删除，不改写）

**改写（替换 Computer 部分）**

- 工作区与打包：根 `Cargo.toml`、`apps/desktop/Cargo.toml`、`crates/backend/nomifun-app/Cargo.toml`（`computer-use` 指向 `nomifun-computer`）、`apps/desktop/tauri.conf.json`、各平台构建与 release 脚本
- 宿主：`nomifun-app/src/lib.rs`（移除 Windows Computer early entry）、`services.rs`（移除 `set_host_app_label`）、`router/{mod.rs, routes.rs, state.rs, nomi_core_builtins.rs, engine_kernel_session.rs, engine_computer_media.rs, computer_permissions.rs, history_process_display.rs, nomi_core_resource_bindings.rs, nomi_core_session.rs, nomi_core_wave2.rs, nomi_core_role_defaults.rs, agent_wave2_host.rs}`
- `crates/shared/nomi-system-execution`：移除 Windows Computer RPC、前台委托及 `build.rs` 中对 Computer 源码的构建身份哈希
- 领域与契约：`nomifun-agent-domain-wave2/src/lib.rs`；`nomifun-agent-contracts` 的 `first-party-agent-modules.v1.json`、`official-agent-seed-manifest.payload.json`、`tool-presentation.json`、平台矩阵、`generated/*`、`src/preset.rs`（用 `cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- write` 再生成）；`nomifun-engine-core/src/system_permission.rs`；`nomifun-agent-runtime/src/turn.rs`；`nomifun-ai-agent`；`nomifun-gateway/tests/production_bypass_audit.rs`；`nomifun-app/tests/*` 与 fixtures；`nomifun-agent-session` 中引用旧动作的测试改用新动作或删除
- UI：`CapabilityPermissionsContent`、`systemPermissionModel`、`useSystemPermissions`、`SystemPermissionNotice` 及其模型、`AgentCapabilityWorkspace`、`AgentResourcePicker`、`chatLib`、`MessageList`、`ipcBridge`、i18n（zh-CN/en-US，含 6.12 节文案）
- 检查脚本：新增 `scripts/check-computer-boundary.mjs` 并加入 `bun run check`；更新 `check-process-runtime-boundary.mjs`

## 8. 阶段交付计划

按 2026-10-20 用户修订，直接在 main 开发，Windows 本机先贯通 S1、S2、S3；其他平台的编译与原生验证并行进行。

- 每个实施阶段开始前以 `git pull --ff-only` 同步最新 main；保护无关改动，不 stash、reset、覆盖或清理用户工作。重叠文件按实际 HEAD 合并意图。
- 每次落到 main 的改动必须自洽：可编译、受影响测试通过、开发平台应用可构建打包；S2 不得出现“旧实现已删、新实现未接上”的状态。
- S1 的 Windows 基础通过后即可进入本机 S2；macOS、Linux、Wayland、其他 DPI/架构、签名或模型凭据缺失只记待验，不充当本机开发门槛，也不据此删掉平台实现或降低现有能力。
- S2 仍是一次完整替换：旧实现删除、新内核接入、保证界面可用的 UI/契约适配及 worker 构建打包同步落地。本地可以分多个提交，但获授权推送时作为完整替换一起交付。
- 检查按实际改动选择最小有效集合，已有证据不因接续或无关改动机械重跑。跨切面 S2 完成及正式发布前运行全量 `bun run check` 和受影响测试。
- 提交、推送、安装、系统权限操作与发布沿用会话已有授权；本说明不增加二次确认或额外审批步骤。

### S1：内核落地与本机 POC

- 交付：`vendor/cua-driver` 固定快照与最小补丁登记、同步脚本、独立 `apps/computer-worker`、协议测试、首版冻结清单与精简覆盖文件。
- 本机继续条件：Windows x64 worker 构建、`--self-check`、真实 worker 无桌面动作协议测试、vendor/冻结一致性检查通过；用专用测试窗口检查当前可执行的输入、关停与写入位置问题，结论记入第 12 节。发现阻断本机行为的缺陷先修，不等待无关平台的矩阵全部完成。
- 并行补验：macOS arm64/x64 构建与 TCC/覆盖层；Linux x64 构建、依赖、X11/AT-SPI、真实 GNOME Wayland 与 portal；Windows 其他 DPI/系统版本及受控故障用例。环境未获只记原因与下一动作，不阻塞本机 S2/S3。

### S2：完整替换接入（Windows 端到端可用）

- 交付：一次性删除第 7 节旧代码、测试、脚本、夹具与引用；生成 Wave2 22 个 Action 与契约；实现 `nomifun-computer` 五个模块和 `router/computer_host.rs`；完成 6.6–6.8 的效果、Stop、模型分档和生命周期；接入 macOS 许可预检；UI 旧动作/路由/许可引用改为新契约；UI/API 42→43；同步各平台 worker 构建、externalBin、签名脚本及 `check:computer-boundary`。
- 本机完成条件：Windows `cargo check -p nomifun-app --features computer-use`、受影响 Rust/UI 测试、`agent-v2-contract check`、`gate:agent-v2` 与 Computer 边界通过；涉及 UI 时跑桌面边界。现役源码、配置、测试与生成契约无旧 Computer 实现/动作残留；本机应用构建打包成功且包含 worker；跨切面完成时 `bun run check` 通过。
- 本机端到端先使用专用计算器/文本编辑窗口与临时数据跑通正式产品链路。有受控模型凭据即补真实模型；凭据不足时只将脚本结果记为确定性回归，继续实现，把真实模型结论留到 S4，不以缺凭据阻塞接入。
- 其他平台编译、T12 与原生验证在相应环境并行补齐，不把 Windows 结果外推到其他平台。

### S3：产品与文档

- 交付：6.10 的驱动状态卡与 `GET /api/computer/status`、能力工作台分组、“前台”标记；6.12 的新建会话文案；6.13 的文档重写与架构初稿。
- 完成条件：受影响 UI 用例和本次涉及的桌面边界、i18n、文档检查通过。可与 S2 的接入收尾并行，复用同一源码身份上的有效证据，不重复要求已通过的完整矩阵。

### S4：按发布平台验收

- 对本次拟发布的平台/架构完成适用的签名、公证、安装包、自检、系统依赖与原生验证；未获环境的平台继续待验，不能声称已验支持，也不阻止已达到要求的平台发布准备。
- 真实模型在可用、获授权的受控 provider 上完成发布范围内的计算器、文本编辑及相关能力分档/协议场景；以实际模型与结果记录覆盖，不设置固定厂商品牌数量作为日常开发门槛。
- 发布前 `bun run check`、受影响 Rust/UI 测试及对应安装包验证通过。第 9 节是按范围选择的验收清单，不要求每次开发重跑全部平台/模型排列组合；本次发布必需项未完成时继续修复或补验，未执行项如实记录。

### S5：发布与收尾

- 完成发布说明（含 6.12 升级影响）、CHANGELOG、版本号和架构定稿；在既有发布授权下从 main 发布，按实际验证说明支持范围，退役 AGENTS.md 本专项入口。

## 9. 测试与验收

只测试新实现；不编写旧数据、旧会话或升级路径的测试与夹具。以下 T/A 项是覆盖清单，按本次改动、可用环境与发布范围选择；不把完整矩阵变成每个阶段的重复门槛。Windows 本机集成先行，缺少环境或凭据的项记录待验并继续可独立推进的工作。

**自动化**

| 编号 | 内容 |
| --- | --- |
| T01 | vendor 快照与补丁登记一致、旧符号零残留（`check:computer-boundary`） |
| T02 | 当前开发平台 worker 构建与 self-check；其他目标平台在对应环境并行补验 |
| T03 | 协议：真实 worker 的 initialize/metadata/list/bind_session/close_session/shutdown；generation 不匹配、未知操作、未绑定会话 |
| T04 | 冻结清单与 worker `list` 一致；Wave2 定义、schema、标题均由清单派生 |
| T05 | 结果归一化：文本/图片/`structuredContent`/`isError`/`next_call` 改写/无图档裁剪 |
| T06 | 效果结论表逐项覆盖，含 worker 层错误分类；Computer Effect 不带 `resource_key` |
| T07 | Stop：排队撤回零效果；在途等待真实结果；超时强杀记 unknown 且不重放 |
| T08 | 生命周期：懒启动、崩溃后新代际与会话重绑、空闲退役、应用退出关停、stdin EOF 退出 |
| T09 | `run_steps`：未授权或不在清单内的 step 派发前拒绝 |
| T10 | 分档：无 `ToolCalls` 不下发；无 `ImageInput` 裁剪工具与参数、拒绝像素寻址 |
| T11 | OpenAI Chat 协议：图片附件消息、`parallel_tool_calls=false`、多调用按序且失败即停；可信字段剥离与注入 |
| T12 | macOS 缺权返回 `SYSTEM_PERMISSION_REQUIRED` 且未执行；授权变化后重启代际 |
| T13 | UI：设置页状态卡、能力工作台分组、"前台"标记、权限提示 |

宿主测试用假 worker（脚本化的 stdio 进程）覆盖慢请求、崩溃、半包与超时；协议测试用真实 worker 二进制，不做桌面动作，可在任意机器运行。

**按发布平台选择的原生与真实模型验收**

| 编号 | 内容 |
| --- | --- |
| A01 | macOS Apple Silicon 签名包：只向 NomiFun.app 授权一次，worker 正常工作且无第二授权条目；后台完成计算器 6×7=42、TextEdit 输入并保存、系统设置开关、多窗口切换；Stop；worker 崩溃恢复；Cmd-Q 后 worker 退出 |
| A02 | macOS Intel：构建与冒烟（环境可得时） |
| A03 | Windows 10/11 普通用户：计算器、记事本保存、设置、Electron/Tauri 应用；100%/150% DPI；提权窗口结构化拒绝；退出回收 |
| A04 | Linux X11：计算器、文本编辑、AT-SPI 后台动作 |
| A05 | Linux GNOME Wayland：portal 授权与拒绝、AT-SPI 后台动作、D12 规则下的行为 |
| A06 | 按发布协议和能力分档选择可用受控真实模型：视觉工具调用、OpenAI Chat 协议图片附件、纯文本无图档等；记录实际 provider/模型、成功率、步数与时延，不要求凑齐固定厂商名单才继续开发 |
| A07 | 资源与安装包：重复任务后无废弃进程/句柄累积、空闲退役生效；本次发布平台安装包内 worker 存在，适用签名/公证有效且 self-check 通过；发布 Linux 时验证其依赖 |

当前用户桌面可以用于本次专用测试窗口、测试会话与临时数据。输入只投递到专用窗口，不修改真实文件、生产数据、凭据或已登录浏览器内容；截图与证据只保留测试内容，不提交隐私截图。故障用例优先使用自有测试 worker/进程和临时目录，只操作精确测试实例；隔离账户、私有 Xvfb、虚拟机等按方便与测试需要使用，不作为本机开发前置条件。不擅自安装系统组件、提权或修改系统权限；真实模型使用现有受控配置或用户提供的测试凭据，凭据不进入日志与文档。

## 10. 风险与对策

| 风险 | 对策 |
| --- | --- |
| 上游迭代快 | 固定快照、按需同步、最小登记补丁、清单冻结与协议测试守门 |
| 私有 worker 协议是上游内部接口 | 两端来自同一快照；协议测试覆盖全部操作 |
| macOS 背景输入依赖 SkyLight 私有框架 | 前台回退；跟随上游修复；新系统版本发布后回归 |
| Wayland 合成器差异 | 按已验证的 X11/XWayland 路由工作；原生 Wayland 在真实环境补验后按固定规则启用，待验不阻塞 Windows 接入 |
| 旧会话放弃带来的用户困惑 | 现有拒绝路径统一为"新建会话"文案；发布说明明确说明 |
| 截图/无障碍树中的提示注入 | 默认可信 Agent 原则下保持可见、可停；不自动授予新权限 |
| 构建体积与时间 | 独立工作区；关闭不需要的 feature |

## 11. 本版本之后的增量

1. 模型原生 computer tool：Anthropic `computer_toolset_20260801`（Messages 协议一次适配覆盖 Anthropic/Bedrock/Vertex）、OpenAI Responses `computer`、Gemini `computer_use`（需新增 Interactions API 通道）。
2. 开源 GUI 模型档案：Qwen3-VL `computer_use`（1000×1000 相对坐标）、UI-TARS 文本动作解析。
3. `run_script`、轨迹录制、Windows 提权窗口（需按机器安装与签名 UIAccess worker）、沙箱/虚拟机、GNOME Shell helper 自动安装。

优先级按用户实际模型分布决定；若以 OpenAI Chat 协议供应商为主，先做第 2 项。

## 12. 单份执行进展

只在本节记录进展：源码/构建身份、命令、非零测试数、原生证据、未运行原因与下一动作。模拟、真实原生与未执行项分开记录。

| 阶段 | 状态 | 证据与说明 | 下一动作 |
| --- | --- | --- | --- |
| S1 | 本机内核交付完成，其余平台原生项待补 | Windows/Linux独立构建、真实协议11/11与10/10、自检60/63、22动作冻结及3个登记补丁一致性通过；Windows真实普通应用与输入已验，具体身份以本节最后的交付记录为准 | 发布范围内补其他平台与故障原生证据 |
| S2 | 本机完整替换与门槛通过，待main提交身份核对 | 旧路径删除、新五模块/ComputerHost/22动作/每请求分档/UI/API43/构建签名脚本已完整接入；计算与文本保存、新HTTP/Runtime/Kernel产品链路及正常关闭通过；Rust/UI/全量check/契约门槛和89MB未签名开发安装包通过 | 提交与同步本机交付；真实模型留S4 |
| S3 | 本机UI、i18n和文档已完成 | 驱动状态API与卡片、四分组、typed前台标记、新建会话文案及实际错误返回链已接入，保留HEAD新展示结构 | 随S4可得平台证据修正文档事实 |
| S4 | 正式发布验收待补 | 当前桌面100%普通原生及未签名开发包已有证据；无正式签名、公证、安装或发布操作，当前任务未提供可用受控模型凭据/Windows签名配置 | 按发布平台补真实模型、签名/公证/安装与原生验收；缺失项不阻塞本机实现 |
| S5 | 升级说明草稿已备，入口保留 | CHANGELOG Unreleased已写升级影响，架构文档已落；尚未发布，保留专项入口承接S4 | 完成发布范围内S4后定稿并退役入口 |

### 最新授权与构建身份（2026-10-20）

- 用户最新明确允许当前真实桌面开发测试，并要求修改AI方案中妨碍快速实施的不合理限制。已同步修订第1–11节、启动prompt与AGENTS专项入口：Windows先贯通，Mac/Linux/Wayland/其他DPI/凭据缺失只记待验；专用测试窗口与临时数据可在当前桌面使用，不读写无关真实文件、凭据或已登录浏览器。原生与实模事实仍不得由模拟替代，发布按平台验收。
- 本次进入S2前再次检查status/main/HEAD并执行`git pull --ff-only`，从c3e750542快进到`1d385e072571b2f71b90e7406aa3ab335aa8dd1c`。实际UI/API已为42，本项将升43，未覆盖他人的42及其会话展示代码。
- 末版S1静态源：SOURCE SHA256 `f3d3427c14f64c3668aafd28746d230d77aeb8a8601206e2ba6f3846ce10ab68`；0002 SHA256 `da2c04b7d7d128d225d0ed695601448a136106dd20e2a7fe05581d89f8740b25`；owned_xtest源 `1e915494d350988dee25142b803e1640f96f103550385e84995907c621479b17`；457源记录，另SOURCE共458文件/13,163,242B。`--check`及固定来源`--verify-upstream`零差异。
- sync脚本SHA256 `40e8e93873fceb426aa508bcd45169fa4c0225072fb467e51933bd8635837835`：逐命令`git -c core.autocrlf=false -c core.eol=lf apply`，输入文本规范LF，登记raw patch SHA保持，SOURCE记录`git-apply-text-lf`；临时文本补丁实验证明默认Git设置会写CRLF，修后raw源=Git clean源，未修改任何全局Git配置。
- Windows末版：默认nested-target流水线build11.33s、真实协议 **10/10**（0 ignored/filtered，0.61s），独立self-check **60**工具，真实list冻结复验通过；worker SHA256 `e8470067f9a654bfceeea26662d85e6deedf6f7b0295b2a1d486182010d9f7c3`。证据`%TEMP%/nomifun-vendor-final-static-windows-94Z9yS/{report.json,pipeline.log,self-check.log}`。
- Linux末版：独立target/build-dir下`cargo build --locked`18.40s、`cargo test --locked`真实协议 **10/10**（0 ignored/filtered，0.61s），独立self-check **63**工具；Bun执行freeze `--check`通过。binary SHA256 `577020c9ab69fc6de4e52961704d303e20e85d6eda33b9173588089d53fd0f58`；日志`%TEMP%/nomifun-computer-worker-final-linux-{build,test}.log`。
- 末版已按源码审查闭合清理责任：core row恢复与park独立尝试，observer/readback/ACK/Remove错误保留同一native owner；已恢复original的row可解责；popup不能遮蔽同client的PING窗口；创建成功但回读失败按保留name重识别四设备及attachments，None不允许绕过drain删除真实pair。Linux在guard前复用既有Xauthority准备；macOS初始化失败等待协议线程，避免主线程抢先以0退出。此段是实现/编译证据，尚非末版原生验证。
- 自动安全审查中止了后续原生故障注入子任务，提示“可能涉及网络安全风险”；该子任务未通过改写请求或换途径重试。末版原生故障项明确未执行。之后继续了安全的源码修正、只读审查、构建与无桌面协议；用户后来对当前桌面的新授权用于普通功能验证，不把未运行故障项改成通过。

以下是S1分轮证据与发现过程；其中“该轮待修”只描述当时源码，不代表末版仍未修正，也不把早轮原生成功归给末版。
### S1 源码、授权与环境

- **固定源码身份**：已实跑 `git clone --filter=blob:none --sparse`。当前远端 `cua-driver-rs-v0.37.0` 是 commit `2d2f99228d4d2508dc6fcc84c176b296e146714a`（非 annotated tag），与 D01 的 SHA 不同；按用户指定不可变 commit `4d959bbfa57f0c89f310330e849500b222084194` 锁定，不随 tag 漂移。仅按 D03 提取九个 crate 与编译资源；保留 5 份 Cargo 显式 target 声明所需源文件，未导入其测试夹具。
- **用户授权的 D04 例外**：2026-10-20，真实 worker 的 X11 强杀 POC 证明核心 XTEST 持键残留后，用户回复“允许为该缺陷追加最小登记补丁（推荐）”。只为输入崩溃清理追加 `0002-x11-owned-input-crash-cleanup.patch`；其他 C/D 合同保持。所有源码变化由登记补丁与 sync 应用，未直接改 vendor 源码；后续修正合并到同一 0002，没有第三补丁。
- **第二轮快照（历史身份，0002 v2）**：457 源记录，另 SOURCE 共 458 文件/13,157,510B；SOURCE SHA256 `8c9fd43d17b5f2cc37d1dbafd2bf70e86563bcf56e3ce5cc2b3642ff4d0d3d73`；0002 `057b8f97172d51d6ec5ab8683193f9bcf614feac0c7518273d8988c2bffbe9f9`；worker lock `009e0ced6224f52b88de8f85110fc0e61a6be85158aebe9269a1310264392035`。此身份已验证早期崩溃窗口，但迟答 App 问题仍在下述修订中，尚非最终交付。
- **环境与隔离**：Windows x64，rustc/cargo 1.97.1、Bun 1.3.14；Ubuntu WSL2 普通 uid 1000、同版 Rust、内核 6.18.33.2-microsoft-standard-WSL2。原生 POC 仅在已有 Xvfb/Openbox/GTK 3.24.41、私有 D-Bus、独立 HOME/XDG/Xauthority、禁 TCP 的测试桌面；无真实用户会话、已登录浏览器、生产根或凭据。未安装组件、提权或改系统许可。
- **运行位置审计**：上游还读取 `HOME`/`USERPROFILE/.cua-driver/config.json`；libei restore token 使用 `XDG_CONFIG_HOME/cua-driver/libei-persistent.token`，不能只设 `CUA_DRIVER_RS_HOME`/`XDG_STATE_HOME`。probe/freeze 已清空环境并隔离全部 HOME/XDG/CUA 目录，省略 DISPLAY，且显式不可达 `DBUS_SESSION_BUS_ADDRESS` 防止上游扫描真实 bus。X11 POC 观察到的运行文件仅在本次测试根；未执行 portal token 路径，不能据源码审计声称它已原生验证。

### S1 自动化与构建（真实协议不做桌面动作）

| 范围 | 实际命令、非零数量与源码身份 |
| --- | --- |
| vendor 一致性 | `node scripts/vendor/sync-cua-driver.mjs --check` 与 `--source <固定只读克隆> --verify-upstream` 通过，457 源零差异；更早的新 filtered sparse clone 验证也通过。脚本先核对已有 SOURCE 字节，不覆盖手改 vendor；新增补丁源明确 `upstream_path/upstream_blob:null`、`added_by_patch`。临时副本分别 **6** 项完整性、**12** 项新增源、**8** 项修订补丁断言通过；证据 `%TEMP%/nomifun-vendor-{added-source-test-fCdFG2,patch-edit-test-PFC98h}` |
| Windows 第二轮构建（历史） | 同步默认流水线实际执行 `cargo build --locked`、独立 `--self-check`、冻结生成及 `cargo test --locked --manifest-path apps/computer-worker/Cargo.toml`；build 11.46s，真实协议 **10/10**、0 ignored/filtered、0.62s，self-check protocol 1 / **60** 工具。设置 `CARGO_BUILD_TARGET=x86_64-pc-windows-msvc`、不显式设置 build-dir，成功从 Cargo artifact 找到嵌套输出。binary `target/x86_64-pc-windows-msvc/debug/nomifun-computer-worker.exe` SHA256 `12db14ba36a9565b33d29b468a30547b004669c6cb1be6738baa47490276d227`；证据 `%TEMP%/nomifun-vendor-final-revision-windows-P1gS8o/{report.json,pipeline.log,self-check.log}`。mt.exe 曾验证嵌入清单 asInvoker/uiAccess=false/PerMonitorV2，属静态证据，不冒充 100%/150% DPI 桌面验收 |
| Linux 第二轮构建（历史） | `CARGO_TARGET_DIR=/tmp/nomifun-computer-worker-s1-target CARGO_BUILD_BUILD_DIR=/tmp/nomifun-computer-worker-s1-build-linux /home/rika/.cargo/bin/cargo {build,test} --locked --manifest-path …/apps/computer-worker/Cargo.toml`；build 14.59s，真实协议 **10/10**、0 ignored/filtered、0.59s；独立 self-check protocol 1 / **63** 工具。binary SHA256 `8ee21f805f87ddbf5e4a055962d11a764c040d4150f54e12396a4b76e3bb3389`；日志 `%TEMP%/nomifun-computer-worker-owned-linux-v2-{build,test}.log`。readelf/ldd 已确认直接依赖 libX11.so.6、libXi.so.6、libxkbcommon.so.0 与基础 libc/libm/libgcc/loader，无缺库；portal-capture 尚未启用，不据此决定 D12 |
| 冻结与脚本 | `node --test scripts/computer/freeze-tools.test.mjs` **12/12**；真实 Windows list60、Linux list63 各派生规定 **22** 动作，并用当前正式 worker 分别 `--check` 通过；Linux 由现有 `/home/rika/.bun/bin/bun` 执行。冻结 SHA256 `2ed794b941d20136d544997c79692f42c935e692a54024de8bfbfa60a8b30d73`；仅 Linux/Windows capture，macOS 未伪造。typed resource kind 核对为 `computer`、默认 ResourceId 为 `local-desktop`，由覆盖文件重新完整派生，未手改生成 JSON |
| 构建隔离修正 | 首次仅设 target-dir 仍共用根 build.noindex，导致跨 OS E0461；未删缓存，改各 OS 独立 build-dir 后通过。同步脚本现自动使用 `<实际 target_directory>/build-<platform>-<arch>.noindex`，保留显式环境值，并以 Cargo compiler-artifact 的 executable 为准。脚本 SHA256 `1ef1ba124d73eb67fc7551992daafcc3d1759deb42f81912116faf58d09655cf`；Windows 默认无 build-dir/嵌套 target 的两次真实流水线通过。WSL 无 Node/cargo 默认 PATH 的首条命令未执行测试，改用已有绝对路径后通过 |
| 最小仓库检查 | `cargo fmt --manifest-path apps/computer-worker/Cargo.toml -- --check`、`git diff --check` 通过；`bun run check:docs` 已在此前进展版通过，最终文案还需重跑。主 workspace 仍 68 包，未链接 vendor；S1 未改产品 Rust/UI，不将这些检查冒充 S2 全量、打包或真实模型验收 |

### S1 隔离原生：第二轮正式 worker 的历史证据

- **三项实际 worker 矩阵通过（上述 Linux binary）**：真实私有协议、专用 Xlib 应用；完成 list_windows、窗口/桌面截图、后台 click/press_key、前台 `NomiFun世界`，再在 10 秒 Shift+左键拖拽或 Unicode 临时映射在途时强杀。精确 worker 强杀 `/tmp/nomifun-computer-s1-owned-x11-9sdtti4y/report.json`；原 worker 进程组强杀 `…-t19zicvu/report.json`；Unicode 在途强杀 `…-ae8tqiw1/report.json`。三者均以 exact guardian PID 的实际 waitpid exit0、owned pair 消失作清理证据，在启动新代际前核验；用户 core 与独立第二 master 同 Shift/Button3 哨兵保持，原 owner 最后各自释放，完整两套 keymap 摘要相等。新代际只读不创建设备，后续输入及正常 shutdown 可用。截图仅留尺寸/字节数/摘要，不保存像素。
- **最早崩溃窗口正式 worker 35/35 通过**：`/tmp/nomifun-worker-early-ready-a8_sunh4/report.json`；同一 GTK PID，**30** 次 worker SIGKILL + **5** 次原进程组 SIGKILL。仅对测试 worker 设临时 LD_PRELOAD 读取屏障：已读真实私有 Ready、尚未返回 Rust 完成输入绑定时停止并立即强杀；没有产品测试 hook/安装。各代 guardian 实际退出0、零 pair 残留、core/第二 master 输入与 map 保持，GTK 原文本/Combo/Menu 与空 keyevent 列表原样、真实 PING 确认；fresh list/shutdown 及 sentinel EOF 最后恢复通过。首样本 `…-gpscxyqz` 因暂停所有 Tokio 线程后仍在 kill 前要求 X roundtrip 而测试超时，夹具改为 Ready 证据后立即 kill 再观察；保留首失败，不当作产品通过。
- **后台 Unicode 原生限制**：前一正式构建 `/tmp/nomifun-computer-s1-owned-x11-4933v3ml/report.json` 的 type_text 返回结构化 `background_unavailable`：私有 Xvfb 无可用 focus-free backend，不能热加 uinput；因此未派发后台 Unicode/未执行其强杀，不能计通过。未 chmod、安装或改用前台假成功；库级 background borrow 证据单列在下文。
- **该轮源码缺口及后续修正**：迟答App、core row恢复失败、popup协议识别及创建后回读失败均曾有责任断点，已由同一0002的末版控制流修正；源码审查确认、构建和协议通过。末版原生复验未执行，原因见最新状态。

### S1 发现过程与独立原语/consumer（不冒充正式 worker）

- **初始已证实失败**：未加 0002 的正式 worker 在拖拽强杀后 core button_mask=256、Shift key50 持续3.2秒；新代 list 不解除输入。报告 `/tmp/nomifun-computer-s1-x11-oqs_xcw3/report.json`（399e58e3…）、`…-exkl1ma3/report.json`（407e75b4…）；首夹具 `…-h5f6vfkr` 的 WM 尚未就绪失败另保留。所有样本只作用私有 X server，最终关闭其测试环境。
- **最小修复原语**：`/tmp/nomifun-xi-owned-input-poc/native-{5foxvdul,kt3lyvyw}/report.json` 证明 XIAddMaster → XISetClientPointer/Focus → owned XTEST slave 能投递 legacy core events；精确 Remove XIFloating 只释放 own，core/第二 master 哨兵不动。直接 own Unicode mapping 在 GTK 失败（`gtk3-{9ineraef,07_ozyg8}`）；core-mirror 修后 `gtk3-mirror-ooqudt_j` **5/5**，含 Hello NomiFun世界、按钮/Combo/Menu与双 map 恢复；首镜像 `…-cpj6zu2n` 的 XKB 规范化差异通过实际写后 readback CAS 修复，失败保留。
- **退休时序失败与修复原语**：直接 Remove、只 park/focusNone/PING 仍导致 GTK 迟到 XIChangeCursor / XI_BadDevice（`gtk-retire-remove-yx9tzn7r`、`gtk-retire-park-uqr_vlno`）；Device Enabled=false 使私有 Xvfb SIGSEGV（`gtk-retire-disable-2aboq9xw`），未采用。release own keys/buttons → only-own XIGrab 到自有最低 InputOnly 停车窗 → own focusNone → 真实 App PING → Remove：同 GTK **30** 代 `gtk-retire-release-park-grab-flym7j9u`、全屏 **10** 代 `gtk-retire-fullscreen-release-park-grab-s2amsddy`、两独立 App 交替 **10** 代 `gtk-multi-retire-7hk4_pya` 通过。均在 `/tmp/nomifun-xi-owned-input-poc/`，同 PID/原字段保留，含正常与 SIGKILL；每代另有原生 PING/GLib idle nonce，不能用第一轮动作成功替代跨代存活。
- **Rust consumer**：最新 `/tmp/nomifun-rust-xtest-guardian-6y3g8tfe/report.json` **12** 项通过，包括同 GTK30代正常/worker强杀/PG强杀、30代无效按键零输入收尾、30代最早 Ready-read 强杀、前/后台临时 Unicode、de layout 刷新、后来作者覆盖保留、无 DISPLAY。consumer SHA256 `83939ccbbeea8037794bcf73c285bc1c5f22f7371b598e87faacc13da7f573f2`。原 GTK 崩溃样本 `…-m_mi8wbf` 与早期窗口在第11代失败 `…-wdo61oai` 均保留；修正增加原 core hover/focus client 的只读采集进入同一 drain。早期的7/7只说明该轮输入，不是 App 跨代证据。
- **清理边界**：fork 仅在 fresh worker 初始单线程、SDK/Tokio 前；guardian 先 setsid/清父死信号再建资源，关无关 FD（不持 protocol stdin/stdout），仅私有 lifetime pipe EOF 触发清理；精确 owner+nonce+guardian 原生身份，不建 DB/文件账本。正常 worker finish 等真实 guardian 状态。worker 与 guardian 同时意外死亡的恢复未证明，不以误伤用户输入的 release-all 或猜旧 map 所有权兜底；S2 宿主仍须对真实清理状态与 unknown 效果负责。

### S2/S3 本机完整替换与普通功能验证（实施中）

- **代码与契约**：删除旧 `nomi-computer`、`nomi-a11y`、旧RoleHost、Windows桥、Computer专测/夹具/脚本；混合验证保留其他域。新 `nomifun-computer` 仅包含 protocol/worker/result/surface/permissions，真实worker由宿主直接持有。22个Wave2动作与官方模板、标题、schema由冻结输入生成，UI/API为43；未修改canonical存储代际或基线SQL。
- **已执行的确定性检查**：`cargo check -p nomifun-app --features computer-use`通过（3m11s，日志 `%TEMP%/nomifun-computer-s2-app-check-2.log`）；此后仍有模型分档/前台投影/Unknown等改动，需最终重跑。新Computer五模块假worker测试当前 **21/21**，覆盖慢请求、崩溃、半包、超时、公平队列、排队/在途Stop、Quit抢占、stderr/native清理责任及空闲退休；真实worker的宿主protocol_smoke只执行guide/init/bind/close/shutdown，desktop_actions=0。Runtime T11新测 **2/2** 与既有普通并行回归 **1/1** 通过，证明Computer两观察仍串行、拒绝与Unknown均停止后续调用；foreground helper **3/3**、实时投影 **4/4** 通过。
- **UI与脚本**：聚焦UI **50/50**、其他受影响当前展示/聊天 **78/78**、preview **4/4** 通过；`bun run build:ui`通过（31.34s，仅既有大chunk提示）。打包脚本Node **20/20**，Windows纯模拟 **73通过/31跳过/0失败**，WSL脚本模拟 **16/16** 与 **20/20**；这些不作原生macOS或安装包证据。全量`bun run check`首轮通过docs/type/UI边界/i18n/theme/icons/dead-css/installer/retirement/external/Computer/process/Browser，UARC扫描器通用自测遗留期望不一致已修正且单项通过，第二轮进行中。Computer边界已检查6075条路径；主workspace不链接vendor。
- **真实Windows普通启动发现失败**：当前桌面仅使用临时专用WinForms测试App，通过新Computer→worker→`launch_app`启动，未输入、未截图、未故障强杀。精确保留App PID75796/worker PID79144及创建时间handle：close_session后App仍存活，正常250ms测试idle退休后worker与App一起退出，App退出码`0xC000013A`，没有自有graceful exit回执；host/observer均exit0，`survived_worker_retirement=false`。证据 `%TEMP%/nomifun-launch-lifetime-1735aba167c14bb1a8ecb8dfe7aff1a7/observer-report.json`；该轮worker SHA256为`e8470067f9a654bfceeea26662d85e6deedf6f7b0295b2a1d486182010d9f7c3`。此失败正以局部登记补丁修复，不能将编译或协议通过记为用户App生命周期通过。
- **本轮待完成**：按实际模型候选在每次wire请求和其对应工具执行中使用同一受信分档；保留历史图片资格要求。Windows修复保持用户启动应用独立于worker Job，不放宽通用进程树清理。完成后重新验证源码/worker身份、最终全量门槛和开发包；真实模型因当前任务尚无可用受控凭据未运行，不阻塞实施。

#### S2 本轮增量证据（2026-10-20，未提交）

- `bun run check`第二轮完整exit0，含全部现役边界（日志 `%TEMP%/nomifun-computer-s2-full-check-2.log`）；UI/API43、现有桌面880×600合同和canonical基线保持。后续D08/patch3修订仍需最终复验，不用本轮替代最终包身份。
- **Windows普通应用存活缺陷的登记修复**：0003第一轮SHA256 `f16ee24ffcceb08c549558cb34f9a8c76b43236fc9eebe2808cfd4207b08caa5`，SOURCE `ca234f81d2dfe8fdd3abd0acd8bb777049c43f3c5b2d10b7f81021833399473c`，worker锁 `cf23a506c957d94c11a43c6fe1ac8619fdf761206045beba76278ed7a6602c30`，Windows worker `cc5c8180673eafa104723040b5edf14d6a3f0ac899238546b324b8cecb402c75`。只经登记补丁与sync改源码；用户应用通过交互shell精确parent与用户token环境启动，不继承worker Job、私有HOME/XDG/CUA或stdio句柄，不更改通用进程树策略。该轮Windows真实协议 **11/11**、self-check60、冻结22复验通过。其后为helper资源退休证明复用已有stderr EOF的修订仍进行中，以上不是修订后的最终身份。
- **两条真实普通启动路径通过**：direct证据 `%TEMP%/nomifun-launch-lifetime-direct-225d5d6c1595468282340abcbb18862e/observer-report.json`（App66428/worker74720），ShellExecute证据 `%TEMP%/nomifun-launch-lifetime-shell-5250da30a8544273893cf20b099c58df/observer-report.json`（App76072/worker59864）。App不在Job、worker在Job；close_session、正常idle、shutdown后同一App进程与窗口仍活，最后均由自有control文件优雅关闭。两轮私有环境泄漏false、profile匹配用户token true、cwd非私有，专用窗口DPI96（100%），未改变系统DPI，未输入、截图或强杀。
- **真实后台计算与文本任务通过**：`%TEMP%/nomifun-owned-desktop-845d164667e742a4a32b3ab0d8dce6e4/functional-report.json`，同根build-identity/exit回执；host示例SHA256 `8772347c2643c04304bcd8fb7a535ec04a8e7d428ee1eb3fbf5b9df023a5d4bb`，fixture `0af0628e97bcb470c901167489d7fe0ad24c9a94ea0d2d50a0503722d8bd2277`，worker为上述cc5c818。专用App PID78744、worker27396，**14/14**调用completed且归一化成功：每步新AX token后台点击2、+、3、=，AX与fixture回执均为5；后台type_text写60字符中英文多行文本，Save后临时文件逐字一致。截图/剪贴板/像素/前台输入调用均0；正常close/shutdown后App自有close.request优雅关闭。这是新Computer宿主直调真实worker证据，未经过产品HTTP/Kernel或真实模型，后二者不得据此记为通过。
- **Linux在0003第一轮源码上重新构建**：WSL普通用户独立目录，`cargo build --locked`10.24s、`cargo test --locked`2.05s/真实协议 **10/10**（0 ignored/filtered，0.50s），真实list冻结22 `--check`通过；binary SHA256 `3fd25a7ab61693d677d948082940c7a199221286a3f7757c3684d50bcf41c955`，日志 `%TEMP%/nomifun-computer-worker-patch3-linux.log`。无桌面动作，不作为原生Wayland或末版故障验收。
- **D08与文案返回链**：Broker每实际候选投影的普通/owned、故障切换、禁止改历史/新增工具、已验证事件先绑定再交付新测 **3/3**（14.57s build，日志 `%TEMP%/nomifun-computer-s2-attempt-projection-tests.log`）。已有control-plane契约演进 **6/6**（29.48s build/0.09s测试，`…/nomifun-computer-s2-contract-rejection-tests.log`）通过；源码核对失效冻结Action在Kernel编译时可先返回CAPABILITY_NOT_MATERIALIZED，现仅在现役Session契约演进入口复用既有AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED，App typed错误映射和中英“请新建会话”已命中。未读取真实旧会话、未加旧数据/升级夹具或测试。
- **Unknown与资源责任**：Computer的resource_key为空，因此原调用不重放，但不会占跨Session桌面唯一索引；canonical Session自身的Unknown准入/恢复围栏按原规则保留，不改Store。Computer业务未知不等于其精确native资源未关闭，退休仍经Computer.close_session/shutdown与原有stderr/进程owner证明。新增App测试首轮夹具虚构binding触发FK、0项通过，已改合法请求，待最终重跑；不将首轮失败计入通过数。新五模块最终假worker测试 **22/22**，含无图verify_state默认截图禁用；测试预算调度修正只作用fixture，产品动作预算未改。
- **署名**：真实locked Cargo metadata派生Windows305/macOS308/Linux369个依赖的许可清单（本体另列MIT），对应190/192/239份去重许可证文本；macOS只采源码metadata，不冒充编译/原生。9份NOTICE输入及worker lock哈希由边界守门，当前3patch/458源记录、22动作检查通过；最终源码身份继续随必要登记补丁修订记录。

#### 本机交付最终记录（2026-10-20，S2/S3）

- **同步与保护**：交付前再次核对main/status/HEAD并执行`git pull --ff-only`，仍为`1d385e072571b2f71b90e7406aa3ab335aa8dd1c`且Already up to date。未stash/reset/clean或覆盖无关改动。保留该HEAD的ToolDetails、FileSummary、MessageTimeline与既有生命周期合同。源码与migration manifest复核为零diff：`src/schema.rs`、`001_canonical_baseline.sql`、`agent-store-migration-manifest.envelope.json`；未改canonical存储。
- **冻结的最终源与worker**：SOURCE SHA256 `da303c0b0d57eed26fd9abb55452ddef350e56a21ec585ae7186f99571aaf92e`；0003 SHA256 `6700e4d1a822ca63b61eca5d716e3770470943398c5bd4c6b78656365b35e5fa`；worker锁仍`cf23a506c957d94c11a43c6fe1ac8619fdf761206045beba76278ed7a6602c30`。最后登记修订使one-shot helper在启动前持有精确worker stderr的非继承duplicate、二次核活，沿宿主已有EOF保留清理责任；原生Wait父死/13s只自退，不把句柄带给用户App；短命ShellExecute设置SEE_MASK_NOASYNC。该修订经独立源码复核与上游重建零差异，不冒充故障注入通过。
- **打包流水线的实物worker**：`bun scripts/computer/build-worker.mjs --target x86_64-pc-windows-msvc --profile debug`通过（1m51s）；实际stage与artifact均SHA256 `3a51f1dbc139355c86808cd7a1cfa0622d04a54ba92d609c2794b6fa46da1929`。同构建目录真实协议 **11/11**（2.58s build/1.22s测试），该stage实物`--self-check`为protocol1/tool_count60/固定4d959bbf commit。最终冻结清单SHA256 `feb910d741bfb2dc1f142ae0b9fe492c42159878f9c8cb2e76b06758e3c004cb`；`drag.steps`仅按真实两端SDK恢复integer计数文案，批量array描述保留，脚本 **16/16** 通过。
- **真实产品链路通过**：`cargo test -p nomifun-app --features browser-use,computer-use --test native_coding_reliability native_computer_chain::dedicated_window_token_action_crosses_http_runtime_kernel_and_canonical_history -- --ignored --exact --nocapture`，**1/1**（20.20s增量构建/4.89s测试）。证据 `%TEMP%/nomifun-computer-product-chain-480d596dcf6a45c8a4ed226615fcd259/{product-chain-report.json,canonical-effects.json,history.json,cleanup-evidence.json,exit.json}`。新会话`01a15974-d268-7160-b487-1e328cf6a450`、专用App PID68176/HWND33297562：真实生产HTTP路由→Runtime→Kernel→Computer→实物worker，按实际ToolSearch披露后启动/读取AX/token后台点击/再读AX确认2，记录launch与click **2**条Returned Effect且resource_key为空、观察不建Effect，保留历史。脚本供应器共 **9** 次HTTP请求（**4**次ToolSearch、**0**辅助请求、5个native及最终阶段）；不是受控真实模型验收。正常`shutdown_nomi_core_host`成功，缓存状态端点实测worker running=false，自有control文件关闭后核对同PID的graceful FormClosed回执；未将回执冒充Windows进程reap。截图/剪贴板/像素寻址调用均0。
- **该产品用例发现并关闭的接线缺口**：首轮夹具错误trait在provider API拒绝、0 native；第二轮canonical平台anyOf被初始模型表strict-object检查拒绝；第三轮General全局工具窗口将Computer延迟后，安装复核错误反转deferred位。现初始模型表使用同源平台冻结定义，保留canonical授权union和既有展示延迟；严格Plan相等校验不放宽。真实Computer正式surface纯测 **1/1**（22定义），General压力→展示→再编译严格相等纯测 **1/1**。用例同时遵循现役ToolSearch，没有放大工具窗口或强推可见性。
- **最终验证**：`cargo check -p nomifun-app --features computer-use` **exit0/2m43s**；两feature `--all-targets` **exit0/6m36s**（仅已有Browser测试源dead_code提示）；桌面包两feature实际编译通过。`cargo test -p nomifun-computer` **22/22**；App `--lib computer_` **18/18**、现役presentation模块 **13/13**；canonical causation **16/16**（24.83s）、Broker conformance **72/72**（1.74s）、control-plane现役演进 **6/6**。UI本轮沿前述 **50+78+4** 定向回归，最终`bun run build:ui` **exit0/35.11s**。`bun run check`末轮完整exit0（含UI/AgentSession/Computer等全部边界）；`bun run gate:agent-v2 contract-closure` **exit0/41 JSON**，其中官方check0、contracts **122/122**、bin **5/5**、diffcheck0。命令日志为`%TEMP%/nomifun-computer-s2-{app-check-final,app-computer-final-tests,presentation-final-tests,crate-final-tests,causation-tests,broker-final-tests,contract-gate-final,full-check-post-native,ui-build-final}.log`。
- **现役生成检查的最小修正**：此前gate的契约check与测试127+5均通过，自动TS导出却把3份HEAD Timeline绑定写回6行尾空格。将5个Timeline展示类型纳入已有官方规范化write/check、去除测试自动写标志；所有字段与合同语义保持，3份绑定回HEAD零diff。未手改generated绑定，也未掩盖gate失败；最终gate已通过。
- **开发安装包**：未安装、未签名、未发布。`bun x tauri build --debug --bundles nsis --config apps/desktop/tauri.conf.json --config apps/desktop/tauri.win.conf.json --config %TEMP%/nomifun-computer-s2-dev-bundle.json`通过；临时覆盖仅dev身份/协议与updater以及跳过已完成的UI重建/缓存裁剪，完整resources保留。Tauri编译 **9m21s**。`target/debug/bundle/nsis/NomiFun Dev_0.8.14_x64-setup.exe` **89,126,968B**，SHA256 `d48937d666bb99ea98f8b08e253adc6056e487f1b9d5a56c3b6e1d191b5f511a`；`target/debug/nomifun-desktop.exe` **364,561,408B**，SHA256 `23ecfee5a5bad9280646ab950f208fe25425d3635ded9b03ef3f34f06faee175`。生成NSIS输入清单645行确认worker实物及619–629行许可资源全部入包，stage哈希与上述原生验收一致；Authenticode状态NotSigned。无已有7z可解包，未下载安装解包组件，不把输入清单/编译成功冒充安装与解包后验签。
- **可复现测试窗口**：新建`tests/fixtures/owned-computer-window.cs`是本轮实际编译源码原字节，10,151B/SHA256 `fa1087933afe899e07f02f612b732d2afb236286c7d4f19334f4fde5bb7b2144`；opt-in测试头部记录已用csc及3个系统程序集引用、3个显式环境路径。未携带用户绝对路径、凭据或本轮实例ID。测试EXE SHA256 `6d21efc9fb9bf4cc046014facfda687252f922f0f6d8a6dacf67fc8e51ae0baf`。新fixture不读旧Computer格式或真实用户资料。
- **暂存核对**：新文件首次进入index时，原样许可证的尾空格与unified diff空上下文的必需空格被检查命中；仅对登记patch、原样许可/Windows manifest设置精确Git属性，代码继续检查。另发现Git LF转换会改变4份许可文本的SHA，已对原样许可及资源manifest设置`-text`并重新入index，保留原字节，未修改许可证或补丁内容。对720份vendor/冻结/许可/原生fixture等受保护输入逐一比较工作区原字节Git blob SHA与index，**零差异**；最终cached/worktree diffcheck均0。开发包、PDB、日志和本机验收数据均未暂存。

### 未执行项与下一动作

1. 本机S2完整替换与S3已通过上述门槛；保持main同步并核对提交/推送身份。旧Computer专测直接删除，受影响混合验证保留非Computer部分，未保留旧实现兜底。
2. macOS arm64/x64构建、自检、冻结采集、签名App TCC；Windows当前桌面专用测试应用与可得DPI；真实Linux X11/GNOME Wayland、portal/helper与D12规则均按可得环境补验。缺失项不阻塞本机实施，WSL/Xvfb不得冒充原生Wayland。
3. 真实模型任务使用明确提供的测试凭据，缺凭据不阻塞代码实现；当前专项的测试凭据及Windows签名环境配置为空，未读取其他任务或生产配置。正式签名/公证、安装验证、系统许可修改和发布未执行；CHANGELOG升级草稿与架构已备，S4完成前保留专项入口。
