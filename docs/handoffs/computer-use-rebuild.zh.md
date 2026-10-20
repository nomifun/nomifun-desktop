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
| S1 | 本机内核交付完成，其余平台原生项待补 | Windows/Linux独立构建、真实协议13/13与12/12、自检60/63、22动作冻结及3个登记补丁一致性通过；Windows真实普通应用与输入已验，具体身份以本节最后的交付记录为准 | 发布范围内补其他平台与故障原生证据 |
| S2 | 本机完整替换已在main提交 | 完整替换d63d3833791a099931bd53dd2cf7fc39e11283d1、生命周期a35e4efc1及显式输入约束7374c0118；旧路径已删除，新五模块/ComputerHost/22动作/每请求分档/UI/API44/构建签名脚本完整接入；真实Full与NoImage链路、关闭、Rust/UI/全量/契约及开发包通过 | 按发布范围补S4 |
| S3 | 本机UI、i18n和文档已完成并补漏 | 默认全选、原生代际诊断、Stop等待提示、后台/平台说明及旧Linux/图片文档指导已按实际源码补齐；驱动状态API、四分组、typed前台标记、新建会话文案及实际拒绝链保持 | 随S4可得平台证据修正文档事实 |
| S4 | Windows实模、原生与unsigned独立安装验收通过，正式发布验收待补 | 当前桌面100%下两档实模任务各2/2及Full视觉通过；系统Calculator、Tauri、52轮会话/空闲退役、App内部worker正常退出通过。当前worker协议Windows13/13、Linux12/12；唯一当前用户安装/包内worker运行/卸载、宿主只读取消及正式HTTP/native只读Stop均1/1；新增真实WinForms后台拖拽中HTTP Stop 1/1。正式签名、公证和其余原生矩阵待补，未发布 | 按发布平台补其余验收，缺失项不阻塞本机实现 |
| S5 | 升级说明草稿已备，入口保留 | CHANGELOG Unreleased已写升级影响，架构文档已落；尚未发布，保留专项入口承接S4 | 完成发布范围内S4后定稿并退役入口 |

### S1收尾历史快照（2026-10-20；授权依然有效，最终身份见后文）

- 用户最新明确允许当前真实桌面开发测试，并要求修改AI方案中妨碍快速实施的不合理限制。已同步修订第1–11节、启动prompt与AGENTS专项入口：Windows先贯通，Mac/Linux/Wayland/其他DPI/凭据缺失只记待验；专用测试窗口与临时数据可在当前桌面使用，不读写无关真实文件、凭据或已登录浏览器。原生与实模事实仍不得由模拟替代，发布按平台验收。
- 本次进入S2前再次检查status/main/HEAD并执行`git pull --ff-only`，从c3e750542快进到`1d385e072571b2f71b90e7406aa3ab335aa8dd1c`。实际UI/API已为42，本项将升43，未覆盖他人的42及其会话展示代码。
- 该S1轮末版静态源：SOURCE SHA256 `f3d3427c14f64c3668aafd28746d230d77aeb8a8601206e2ba6f3846ce10ab68`；0002 SHA256 `da2c04b7d7d128d225d0ed695601448a136106dd20e2a7fe05581d89f8740b25`；owned_xtest源 `1e915494d350988dee25142b803e1640f96f103550385e84995907c621479b17`；457源记录，另SOURCE共458文件/13,163,242B。`--check`及固定来源`--verify-upstream`零差异。
- sync脚本SHA256 `40e8e93873fceb426aa508bcd45169fa4c0225072fb467e51933bd8635837835`：逐命令`git -c core.autocrlf=false -c core.eol=lf apply`，输入文本规范LF，登记raw patch SHA保持，SOURCE记录`git-apply-text-lf`；临时文本补丁实验证明默认Git设置会写CRLF，修后raw源=Git clean源，未修改任何全局Git配置。
- 该S1轮Windows末版：默认nested-target流水线build11.33s、真实协议 **10/10**（0 ignored/filtered，0.61s），独立self-check **60**工具，真实list冻结复验通过；worker SHA256 `e8470067f9a654bfceeea26662d85e6deedf6f7b0295b2a1d486182010d9f7c3`。证据`%TEMP%/nomifun-vendor-final-static-windows-94Z9yS/{report.json,pipeline.log,self-check.log}`。
- 该S1轮Linux末版：独立target/build-dir下`cargo build --locked`18.40s、`cargo test --locked`真实协议 **10/10**（0 ignored/filtered，0.61s），独立self-check **63**工具；Bun执行freeze `--check`通过。binary SHA256 `577020c9ab69fc6de4e52961704d303e20e85d6eda33b9173588089d53fd0f58`；日志`%TEMP%/nomifun-computer-worker-final-linux-{build,test}.log`。
- 该S1轮末版已按源码审查闭合清理责任：core row恢复与park独立尝试，observer/readback/ACK/Remove错误保留同一native owner；已恢复original的row可解责；popup不能遮蔽同client的PING窗口；创建成功但回读失败按保留name重识别四设备及attachments，None不允许绕过drain删除真实pair。Linux在guard前复用既有Xauthority准备；macOS初始化失败等待协议线程，避免主线程抢先以0退出。此段是实现/编译证据，尚非末版原生验证。
- 自动安全审查中止了后续原生故障注入子任务，提示“可能涉及网络安全风险”；该子任务未通过改写请求或换途径重试。末版原生故障项明确未执行。之后继续了安全的源码修正、只读审查、构建与无桌面协议；用户后来对当前桌面的新授权用于普通功能验证，不把未运行故障项改成通过。

以下是S1分轮证据与发现过程；其中“该轮待修”只描述当时源码，不代表末版仍未修正，也不把早轮原生成功归给末版。
### S1 源码、授权与环境

- **固定源码身份**：已实跑 `git clone --filter=blob:none --sparse`。当前远端 `cua-driver-rs-v0.37.0` 是 commit `2d2f99228d4d2508dc6fcc84c176b296e146714a`（非 annotated tag），与 D01 的 SHA 不同；按用户指定不可变 commit `4d959bbfa57f0c89f310330e849500b222084194` 锁定，不随 tag 漂移。仅按 D03 提取九个 crate 与编译资源；保留 5 份 Cargo 显式 target 声明所需源文件，未导入其测试夹具。
- **用户授权的 D04 例外**：2026-10-20，真实 worker 的 X11 强杀 POC 证明核心 XTEST 持键残留后，用户回复“允许为该缺陷追加最小登记补丁（推荐）”。只为输入崩溃清理追加 `0002-x11-owned-input-crash-cleanup.patch`；其他 C/D 合同保持。所有源码变化由登记补丁与 sync 应用，未直接改 vendor 源码；截至当轮，后续修正合并到同一 0002，尚无第三补丁；后续必要修复见下文 0003。
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

### S2/S3 本机完整替换与普通功能验证（历史轮次）

- **代码与契约**：删除旧 `nomi-computer`、`nomi-a11y`、旧RoleHost、Windows桥、Computer专测/夹具/脚本；混合验证保留其他域。新 `nomifun-computer` 仅包含 protocol/worker/result/surface/permissions，真实worker由宿主直接持有。22个Wave2动作与官方模板、标题、schema由冻结输入生成，UI/API为43；未修改canonical存储代际或基线SQL。
- **已执行的确定性检查**：`cargo check -p nomifun-app --features computer-use`通过（3m11s，日志 `%TEMP%/nomifun-computer-s2-app-check-2.log`）；此后仍有模型分档/前台投影/Unknown等改动，需最终重跑。新Computer五模块假worker测试当前 **21/21**，覆盖慢请求、崩溃、半包、超时、公平队列、排队/在途Stop、Quit抢占、stderr/native清理责任及空闲退休；真实worker的宿主protocol_smoke只执行guide/init/bind/close/shutdown，desktop_actions=0。Runtime T11新测 **2/2** 与既有普通并行回归 **1/1** 通过，证明Computer两观察仍串行、拒绝与Unknown均停止后续调用；foreground helper **3/3**、实时投影 **4/4** 通过。
- **UI与脚本**：聚焦UI **50/50**、其他受影响当前展示/聊天 **78/78**、preview **4/4** 通过；`bun run build:ui`通过（31.34s，仅既有大chunk提示）。打包脚本Node **20/20**，Windows纯模拟 **73通过/31跳过/0失败**，WSL脚本模拟 **16/16** 与 **20/20**；这些不作原生macOS或安装包证据。全量`bun run check`首轮通过docs/type/UI边界/i18n/theme/icons/dead-css/installer/retirement/external/Computer/process/Browser，UARC扫描器通用自测遗留期望不一致已修正且单项通过，第二轮进行中。Computer边界已检查6075条路径；主workspace不链接vendor。
- **真实Windows普通启动发现失败**：当前桌面仅使用临时专用WinForms测试App，通过新Computer→worker→`launch_app`启动，未输入、未截图、未故障强杀。精确保留App PID75796/worker PID79144及创建时间handle：close_session后App仍存活，正常250ms测试idle退休后worker与App一起退出，App退出码`0xC000013A`，没有自有graceful exit回执；host/observer均exit0，`survived_worker_retirement=false`。证据 `%TEMP%/nomifun-launch-lifetime-1735aba167c14bb1a8ecb8dfe7aff1a7/observer-report.json`；该轮worker SHA256为`e8470067f9a654bfceeea26662d85e6deedf6f7b0295b2a1d486182010d9f7c3`。此失败正以局部登记补丁修复，不能将编译或协议通过记为用户App生命周期通过。
- **本轮待完成**：按实际模型候选在每次wire请求和其对应工具执行中使用同一受信分档；保留历史图片资格要求。Windows修复保持用户启动应用独立于worker Job，不放宽通用进程树清理。完成后重新验证源码/worker身份、最终全量门槛和开发包；真实模型因当前任务尚无可用受控凭据未运行，不阻塞实施。

#### S2 增量证据（2026-10-20历史轮次，当时未提交）

- `bun run check`第二轮完整exit0，含全部现役边界（日志 `%TEMP%/nomifun-computer-s2-full-check-2.log`）；UI/API43、现有桌面880×600合同和canonical基线保持。后续D08/patch3修订仍需最终复验，不用本轮替代最终包身份。
- **Windows普通应用存活缺陷的登记修复**：0003第一轮SHA256 `f16ee24ffcceb08c549558cb34f9a8c76b43236fc9eebe2808cfd4207b08caa5`，SOURCE `ca234f81d2dfe8fdd3abd0acd8bb777049c43f3c5b2d10b7f81021833399473c`，worker锁 `cf23a506c957d94c11a43c6fe1ac8619fdf761206045beba76278ed7a6602c30`，Windows worker `cc5c8180673eafa104723040b5edf14d6a3f0ac899238546b324b8cecb402c75`。只经登记补丁与sync改源码；用户应用通过交互shell精确parent与用户token环境启动，不继承worker Job、私有HOME/XDG/CUA或stdio句柄，不更改通用进程树策略。该轮Windows真实协议 **11/11**、self-check60、冻结22复验通过。其后为helper资源退休证明复用已有stderr EOF的修订仍进行中，以上不是修订后的最终身份。
- **两条真实普通启动路径通过**：direct证据 `%TEMP%/nomifun-launch-lifetime-direct-225d5d6c1595468282340abcbb18862e/observer-report.json`（App66428/worker74720），ShellExecute证据 `%TEMP%/nomifun-launch-lifetime-shell-5250da30a8544273893cf20b099c58df/observer-report.json`（App76072/worker59864）。App不在Job、worker在Job；close_session、正常idle、shutdown后同一App进程与窗口仍活，最后均由自有control文件优雅关闭。两轮私有环境泄漏false、profile匹配用户token true、cwd非私有，专用窗口DPI96（100%），未改变系统DPI，未输入、截图或强杀。
- **真实后台计算与文本任务通过**：`%TEMP%/nomifun-owned-desktop-845d164667e742a4a32b3ab0d8dce6e4/functional-report.json`，同根build-identity/exit回执；host示例SHA256 `8772347c2643c04304bcd8fb7a535ec04a8e7d428ee1eb3fbf5b9df023a5d4bb`，fixture `0af0628e97bcb470c901167489d7fe0ad24c9a94ea0d2d50a0503722d8bd2277`，worker为上述cc5c818。专用App PID78744、worker27396，**14/14**调用completed且归一化成功：每步新AX token后台点击2、+、3、=，AX与fixture回执均为5；后台type_text写60字符中英文多行文本，Save后临时文件逐字一致。截图/剪贴板/像素/前台输入调用均0；正常close/shutdown后App自有close.request优雅关闭。这是新Computer宿主直调真实worker证据，未经过产品HTTP/Kernel或真实模型，后二者不得据此记为通过。
- **Linux在0003第一轮源码上重新构建**：WSL普通用户独立目录，`cargo build --locked`10.24s、`cargo test --locked`2.05s/真实协议 **10/10**（0 ignored/filtered，0.50s），真实list冻结22 `--check`通过；binary SHA256 `3fd25a7ab61693d677d948082940c7a199221286a3f7757c3684d50bcf41c955`，日志 `%TEMP%/nomifun-computer-worker-patch3-linux.log`。无桌面动作，不作为原生Wayland或末版故障验收。
- **D08与文案返回链**：Broker每实际候选投影的普通/owned、故障切换、禁止改历史/新增工具、已验证事件先绑定再交付新测 **3/3**（14.57s build，日志 `%TEMP%/nomifun-computer-s2-attempt-projection-tests.log`）。已有control-plane契约演进 **6/6**（29.48s build/0.09s测试，`…/nomifun-computer-s2-contract-rejection-tests.log`）通过；源码核对失效冻结Action在Kernel编译时可先返回CAPABILITY_NOT_MATERIALIZED，现仅在现役Session契约演进入口复用既有AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED，App typed错误映射和中英“请新建会话”已命中。未读取真实旧会话、未加旧数据/升级夹具或测试。
- **Unknown与资源责任**：Computer的resource_key为空，因此原调用不重放，但不会占跨Session桌面唯一索引；canonical Session自身的Unknown准入/恢复围栏按原规则保留，不改Store。Computer业务未知不等于其精确native资源未关闭，退休仍经Computer.close_session/shutdown与原有stderr/进程owner证明。新增App测试首轮夹具虚构binding触发FK、0项通过，已改合法请求，待最终重跑；不将首轮失败计入通过数。新五模块最终假worker测试 **22/22**，含无图verify_state默认截图禁用；测试预算调度修正只作用fixture，产品动作预算未改。
- **署名**：真实locked Cargo metadata派生Windows305/macOS308/Linux369个依赖的许可清单（本体另列MIT），对应190/192/239份去重许可证文本；macOS只采源码metadata，不冒充编译/原生。9份NOTICE输入及worker lock哈希由边界守门，当前3patch/458源记录、22动作检查通过；最终源码身份继续随必要登记补丁修订记录。

#### 本机首批交付记录（2026-10-20，S2/S3；后续修正见下文）

- **源码提交**：完整S1/S2/S3源码与首次交付进展已提交main：`d63d3833791a099931bd53dd2cf7fc39e11283d1`。该提交包括原样许可/manifest的Git字节保护，不包含编译产物与私有证据；此后的交付身份更新为文档改动。源码提交`d63d3833791a099931bd53dd2cf7fc39e11283d1`与进展文档提交`5021ba0a718b9f431504cc6a3cc31e8bbaee5ad4`已一起正常推送至origin/main；2026-10-20核验本机与远端HEAD均为`5021ba0a718b9f431504cc6a3cc31e8bbaee5ad4`。当前用户代码签名证书只读计数为0，Windows签名thumbprint与受控模型测试凭据环境均未配置，已请求获授权配置的入口，不因此阻塞本机源码交付。
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

- **与定时任务并行集成**：正常合并 `add2284fc` 与定时任务工作台改造 `c36dd629c` 时，macOS 开启 `computer-use` 的 General 实际执行暴露初始工具表缺口：当前平台尚无 capture 时错误地阻断整个 Agent，再被 `Conflict` 文本分类误报为供应商 schema 错误。初始工具表现与 Kernel 安装使用同一冻结平台投影，仅已知但缺本平台定义的动作省略；未知动作、异常投影和其他 schema 仍拒绝。工具准备错误以宿主 typed `Internal` 保留来源。没有新增 schema 或改写 canonical 资源授权，Windows/Linux 的精确定义和严格重新编译合同保留；这是普通 Agent 平台隔离修复，不作为 macOS Computer 原生支持证据。

### Windows真实模型与生命周期修正（2026-10-20，S4本机增量）

- **来源与授权**：进入本轮已检查main/status/HEAD并`git pull --ff-only`；收尾`git fetch origin`仍为`add2284fc91c9bd0207dd89d4651c3c869a4a68c`，保护无关改动。用户已提供并授权StepFun Coding Plan测试凭据及`step-5-preview`，凭据经SecureString转测试进程stdin，不进argv、环境变量、仓库、报告或应用日志。固定`https://api.stepfun.com/step_plan/v1`的模型列表认证200、真实函数调用预检200/1次tool call/268 tokens；这是模型传输预检，不是桌面验收。
- **已修的生产缺陷**：NoImage投影误删`set_window_frame`的必填x/y，并在执行前误判为截图寻址；现在仅保留该动作顶层原生frame坐标，输入像素与批量范围限制保持。真实模型随后证实同名驱动会话跨Turn重绑失败：SDK的close只撤授权，未结束Core生命周期；同名并发也发生绑定冲突。自有private worker现以“Agent名+完整handle UUID”隔离原生身份，bind显式start_session，close/EOF/shutdown先成功end_session再撤权，失败保留精确清理责任；不重试业务动作。未改vendor、未追加补丁、未改canonical存储。
- **独立构建与协议**：`bun scripts/computer/build-worker.mjs --target x86_64-pc-windows-msvc --profile debug` **exit0/16.51s**；同target/build-dir下`cargo test --locked --manifest-path apps/computer-worker/Cargo.toml --target x86_64-pc-windows-msvc --test protocol` **13/13**（2.19s构建/1.41s测试）。WSL普通用户隔离target/build-dir对应命令 **12/12**（12.96s构建/0.83s测试）。新增协议用例直接验证bind后Core active、同名close/rebind与并存隔离，只管理会话/读取指南，不做桌面动作。两端自检60/63、实际worker冻结22复验均通过；Windows stage/artifact SHA256均`07d50d7a69635139af31fb7725731a727b1ca648b3db7e68795858e64b4f54d4`，Linux worker为`aa2006f5606ab2f5a92483e266888333f910ff772c688ac06ce839031ee05558`。WSL协议不冒充Linux原生桌面验收。
- **自动化回归**：`cargo test -p nomifun-computer --lib no_image_` **5/5**，完整`cargo test -p nomifun-computer` **24/24**（3.11s构建/2.36s测试）；`cargo check -p nomifun-app --features computer-use` **exit0/7.58s**；两feature的App `--lib computer_` **18/18**（19.51s构建/1.01s测试）。`bun run check`本轮末次完整exit0，包含Computer、desktop UI、Agent Session及全部现役边界。冻结与生成契约未变，未为本轮重复无关平台/模型排列组合。
- **真实Full最终验收**：先`cargo test -p nomifun-app --features browser-use,computer-use --test native_coding_reliability --no-run`，再仅运行实际测试二进制的`live_computer_chain::stepfun_full_route_ax_then_window_image --exact --ignored --nocapture --test-threads=1`，凭据单行stdin。**1/1通过，39.56s**；报告`%TEMP%/nomifun-computer-live-visual-8454b5b3e3e9482f9d2faa35afb0a476/full-5/live-computer-report.json`。使用新建chat.minimal测试配置，只授权冻结的get_window_state/click/type_text，无Shell/File/Browser/剪贴板/全桌面权限；真实生产HTTP→provider→Session/Runtime→Kernel→Computer→worker，非脚本模型。Session`01a159c6-fd93-7c92-bff4-d120c2fe29c7`，自有WinForms PID41084/HWND26479480、96 DPI：模型实际点击2、+、3、=得5，输入并保存固定中英文两行文本，文件与新AX值精确核验 **2/2任务通过**；AX阶段 **11/11调用、12模型步、32.953s**，6个Effect均Returned。随后同Session下一Turn **1/1调用、2模型步、5.429s**，请求同窗image/png **510×330**并在更晚步骤正确读出随机绘制8位码；码不在两Turn的prompt或AX label/value中。两Turn typed finish_reason均Completed，read-only调用不建Effect。截图元数据、call_id/Turn与公开视觉答案因果核验；现役历史按合同省略原图，`canonical_raw_images=0`，不将描述符冒充图片字节或编造图像哈希。
- **保留失败与修正事实**：full-1因测试提示JSON换行歧义输入了字面转义，自动用例0/1；full-2计算/文本已2/2但图片请求命中上述session_ended，整体0/1；修worker后的full-3实际读码成功，但测试错误要求历史保留原图，整体0/1；full-4实际9次调用、文件与效果均成功，测试又误把过滤AX快照未列出的有效索引当成越界，整体0/1。最终按驱动真实snapshot归属及成功accessibility回执核验，不猜隐藏label、不放松已执行越界或unknown；保留真实文件、计算按钮序列、typed完成与关闭门槛。上述早轮不改报自动通过。最终测试记录正常host shutdown、worker stopped、exact Child wait退出及同PID graceful回执均true；递归检查该临时目录未发现明文凭据，不提交任何截图或私有验收数据。
- **源码与实物身份**：SOURCE仍`da303c0b0d57eed26fd9abb55452ddef350e56a21ec585ae7186f99571aaf92e`，3个vendor补丁与冻结清单保持。private_worker源码`d989c994a7a1ff12014490b9eb6f6c1361969453588e58560f9c2281182717c7`，live测试源码`6f0c3529c53d9e72c939b9e20a3e305afb133a872c26ac611d0242a1333ca7a2`，测试二进制`be8ad7801a78d343fe010d9ac65a99abad6d1b6101be038101c5500aa5f5ac5f`；fixture源码`a97e59db3d52a57ad00dca582b3700c66fbba09baecc3385fbefeff2f222579f`、实物`b978d76623b2ecf96ff57cd7b8e6124c3677367fa15c8f29b946338aebc5e3b3`。7个源文件及构建摘要在同临时根`build-identity-full5.json`，SHA256`758ef24c1f85cc52bde888df7e3cfe3c2f89c994d6ff6d5fddce79f30081a2e6`。fixture随机码只在functional窗口Paint，不添加AX字段；csc需绝对源路径，复现说明已修正。
- **开发包复验**：使用上述未改UI的dev临时配置，再运行前述`bun x tauri build --debug --bundles nsis ...` **exit0**，Rust构建23.90s。`target/debug/nomifun-desktop.exe` **364,561,920B**、SHA256`93a4e0fb304a65dc2b71576001d75c603f26f95e4d6aa365e32dfeddca7d9cee`；NSIS开发包 **89,161,683B**、SHA256`f2eedb53eee9d44c4442e92acf66f7b741dc33a448ebce58c0207b43465acf0c`，使用同一07d50d7a worker。仍为NotSigned，未安装或发布。
- **用户新增产品决定与下一动作**：现役模型traits是建议元数据，缺vision_input不能禁图；六种生产Chat adapter均能编码图片，因此不能用删标签伪造真实NoImage路由。用户已明确选择“增加显式模型输入能力约束（推荐）”。下一步在通用、task-scoped模型接口增加`input_constraints.image_input`，缺省保持原行为、false仅收窄图片输入，冻结进每个实际候选并在转换后保持约束；不放入traits/机器health/transport/provider_params，不新增Computer开关或canonical事件。该配置与真实无图档验收尚未落地，继续推进。
- **同期主线合并与重验**：上述已验证修复先在本地main提交`a35e4efc1`；随后`git pull --ff-only`发现远端`e29e06da2`（含定时任务工作台`c36dd629c`）已更新145文件，并因双方进展文档重叠保护性中止。未stash/reset/覆盖用户改动；使用正常`--no-commit`合并，仅文档发生冲突，双方段落完整保留。基于合并结果，App `computer-use` check **exit0/4m08s**、两feature Computer宿主 **18/18**（10m48s冷构建/1.06s测试）、原生测试`--no-run` **exit0/7m25s**、完整`bun run check` exit0、UI构建 **exit0/36.63s**、7个受影响Computer UI文件 **104/104**（57.91s）通过。新鲜`full-merge-6/live-computer-report.json`（与full-5同临时父目录）实模再验 **1/1，39.00s**：Session`01a159e2-def8-78c1-b531-00a57af4933f`，PID80816/HWND145100718，计算与文本 **2/2**、AX **11/11调用/12模型步/32.262s**、图像 **1/1调用/2模型步/5.431s**、510×330 PNG随机码正确；正常关闭、exact Child退出及明文凭据检查仍全部通过。新测试实物SHA256`4dc0f8f76c388b8c1055c532358152b8bb39d6d398e500fb8e38d46e3aacca8d`。合并后的开发包与契约closure均已通过：Rust打包构建9m35s，NSIS产物89,100,343B/SHA256`4125f01cf5baeb5675d4624c823f30740416790e2f434e476f4571ac2bfa2880`，桌面exe364,325,888B/SHA256`ccf8446f8c0e1c2d2adadd138642ca9b6bd77ccc54daf8a71049033e9dc6e8ed`，worker仍为07d50d7a；包为NotSigned、未安装未发布。`bun run gate:agent-v2 contract-closure`通过41个JSON、官方check、contracts122/122及bin5/5；summary记录在`build.noindex/agent-capability-v2/a35e4efc1a76146185085823e9158f12447487c1/contract-closure/summary.json`（构建来源为上述两父提交的已合并工作树）。额外只读EnumDisplayMonitors/GetScaleFactorForMonitor实测唯一监视器100%，未改任何设置；150%仍未执行。012迁移已由并行主线占用，后续输入约束使用实际HEAD的新编号。

### 显式模型输入约束（2026-10-20至21日，S4本机增量）

- **基线与范围**：在main `457b970a22a2106c1f21587311d3d3d6dabd6adf`、干净工作区再次`git pull --ff-only`确认最新后，实施用户已确认的通用Chat接口输入约束。`input_constraints.image_input`缺省/null/空对象保持原行为，false只收窄图片输入，true不扩展serializer；每个实际候选冻结约束并在Broker转换后保持，所有Chat发送入口拒绝不相容的真实用户/历史工具图片，不删历史。UI/API升44；013只新增nullable模型配置列，不改变Agent表、schema metadata或存储代际。
- **已通过的自动检查**：`cargo check -p nomifun-app --features computer-use` **exit0/4m05s**；`cargo test -p nomifun-api-types --test ts_export export_provider_domain_bindings -- --exact` **1/1**；DB `--test id_schema_contract --test provider_repository` **6/6+33/33**（44.34s构建、3.53s+104.10s测试）。UI两组11文件 **103/103+87/87**（106.81s+44.60s），其中8个新增用例覆盖三态、保存映射、发现竞态、selector和实际编辑器交互；typecheck、i18n **6/6**、desktop boundary2247源均通过。完整`bun run check` **exit0**，`bun run build:ui` **exit0/50.90s**，dist合同实测44。
- **发现与修正**：Rust测试编译发现两份现役测试构造器漏加默认字段，已仅补None。API新测试初轮 **0/1** 暴露Serde把空数组解为默认结构体，已收紧共享类型为map-only解码，未知/重复字段一并拒绝，断言保持。修后四crate定向lib测试 **2/2+1/1+2/2+1/1**，覆盖API、generic Agent发送前拒绝、独立调用的历史图片拒绝及配置pin、programmatic校验；System集成配置往返/管理 **1/1+1/1**；DB规范化/noop/health/clone及非法输入原子拒绝复验 **2/2**。独立route JSON schema原为手工静态文件且未接纳新typed字段，已由`agent-v2-contract write`（exit0/11.89s）从Rust类型生成并纳入check，保留原严格约束并补齐现役activation_features/streaming；canonical表与代际未变。后续App/实模/gate尚待本轮执行。
- **宿主与契约最终回归**：两feature的App `--lib --test native_coding_reliability --no-run` **exit0/9m46s**；实际App测试程序按`chat_broker_host:: nomi_core_chat_route:: model_management:: computer_`过滤 **62/62、11.00s**，包含每候选冻结、所有摘要scope、转换后禁图、真实请求投影及图片历史不能被裁掉的现役验证。`bun run gate:agent-v2 contract-closure` **exit0/41 JSON**，正式check0、contracts **123/123**、bin **5/5**、diffcheck0；summary在`build.noindex/agent-capability-v2/457b970a22a2106c1f21587311d3d3d6dabd6adf/contract-closure/summary.json`，代表该基线上的本轮工作树。完整`cargo test -p nomifun-computer -- --test-threads=1` **24/24、4.95s**。本轮源固定后再次完整`bun run check` **exit0**。
- **真实NoImage**：实际测试程序单独执行`live_computer_chain::stepfun_explicit_no_image_route_completes_ax_tasks --exact --ignored --nocapture --test-threads=1`，同一授权Step Plan/step-5-preview仍保留vision_input标签，仅显式接口`image_input:false`禁图。**1/1通过、24.05s**；临时报告`no-image-7/live-computer-report.json`（与full-5同父目录），Session`01a15a15-f189-72a2-919a-9ac33ffa67cf`，专用WinForms PID44076/HWND21042578。冻结唯一候选无ImageInput、保留false和ToolCalls，实际模型Turn身份与之相等；计算和文本保存 **2/2**，**11/11调用、9模型步、22.762s**，原生capture计数0、6个Effect均Returned，typed完成。不是删标签或仅用提示词伪造无图，也未将上游模型宣称为纯文本模型。
- **真实Full复验**：同一新测试实物单独执行`live_computer_chain::stepfun_full_route_ax_then_window_image --exact --ignored --nocapture --test-threads=1`，新鲜`full-input-8/live-computer-report.json`。**1/1通过、41.90s**，Session`01a15a17-2734-7221-862b-a5127e2ec168`，专用PID34988/HWND10360156；计算与文本 **2/2**，AX **12/12调用、13模型步、34.583s**，6个Effect Returned。后续同Session图片Turn **1/1调用、2模型步、6.131s**；原生PNG **510×330** 的随机8位绘制码在更晚模型步读对，仍不在AX/prompt中。三次Turn均核验唯一实际候选、正常typed完成；两case均host正常shutdown、worker stopped、exact Child退出及samePID graceful回执、明文凭据检查true，凭据只由SecureString经stdin提供，截图/私有验收数据不提交。`canonical_raw_images=0`仍遵守历史省略图片合同，视觉证据口径保持上轮因果核验。
- **源码与构建身份**：本轮live源码SHA256 `f89558d09f3be7340b97aafae6c25c86fe5946cdc79b1d8642562938aced5967`，测试实物`8bf64e7a010659f2aca70e28935f5310fda18c771abb053d516bc641cb680abb`；同临时父目录`build-identity-inputconstraints.json`记录71份Rust/脚本/协议/UI改动源与实物，SHA256 `aaec7222cbdef52a02b002772bcd5cc5ba909c967ebc56463e2b3bb6283ec353`。UI dist合同44/build ID `4bac2c9d-6dae-46a5-8dd5-9f78af49220b`。worker仍为07d50d7a、fixture仍b978d766，上游SOURCE、补丁、冻结22动作和worker源码均未改变，不重复独立worker协议构建。
- **暂存与行尾复验**：逐一核对上述71份改动源的构建身份后，仅暂存本专项代码、生成契约及架构文档。首次暂存发现新增013工作区为CRLF、Git规定LF，已规范为LF，原始Git blob与index完全相等，SHA256 `4c7f1423731d88ad21c3b8cc80863c05639175cfaf8726e00b70a28c22fbdd3d`；SQL语句不变，避免不同checkout得到不同迁移校验值。前述实模清单保留规范前的原字节身份；最终开发包使用规范后的源另列。LF源下DB定向 **2/2、2.53s**，现役`cargo test -p nomifun-shell --test stt_integration -- --test-threads=1` **8/8、4.93s**，验证非Chat语音配置与调用保持。
- **最终开发包**：仍使用上述dev配置和`bun x tauri build --debug --bundles nsis ...`，完整 **exit0**，LF源重建Rust **41.72s**。桌面EXE **364,457,984B/SHA256 `5920a6e6fa81e50152ddb54b5d9f171d864a031cea49dc783b1c8f12c09216ed`**；`target/debug/bundle/nsis/NomiFun Dev_0.8.14_x64-setup.exe` **89,131,488B/SHA256 `e8ab3994f6b7675ed7a376001e247c27cccb3d2fbb0b203169fc1b6ba4e52ad4`**。最终71源/产物清单`build-identity-inputconstraints-package.json` SHA256 `a77618c1ef6dabbbd0c78a85daf318ad91d9491bdc45cc1bf2c66ec7aa3ffd8e`；与实模源只差上述013行尾。NSIS实际645行纳入同一07d50d7a worker，619–629行纳入许可资源；EXE、安装包和worker均NotSigned，未安装、未发布。
- **主线源码提交**：`7374c011842947090fbb325bebb79070d1e7a8f9`（83文件）包含上述实现、生成契约、测试、UI/API44及架构。提交前再次fetch确认origin/main仍为457b970a2；83份暂存路径与71份最终源身份逐一相符，工作区与暂存diffcheck均0、文档检查0。原生/包/凭据和验收数据均在仓库外或忽略目录；保护同期主线，未stash/reset/覆盖用户改动。本条仅后续补记已验证源码身份。
- **下一动作**：本轮Windows实现、实模、测试与开发包已完成；按发布范围补正式签名、安装与其他平台原生项，保留专项入口承接S4。

### Windows安装包、Tauri与资源复验（2026-10-21，S4本机增量）

- **基线与范围**：在main `8dacffd0516eb7c0d08a5543fe1173591a0a83d7`、干净工作区检查分支/HEAD并`git pull --ff-only`确认最新，收尾再次同步仍最新。本轮沿用用户已授权的当前桌面，启动自有临时窗口与测试数据，不使用模型凭据或已登录浏览器，未请求提权或系统权限变更。后续发现Desktop fixture还会进入共享Keyboard注册，不能将dataset/UDF隔离外推为所有用户设置均未改变；确证影响、修复和清理见本节后文。模拟、真实原生及未执行项分别记录。
- **真实安装包解包**：对上一轮NSIS `e8ab3994f6b7675ed7a376001e247c27cccb3d2fbb0b203169fc1b6ba4e52ad4`，临时下载官方7-Zip 26.04 Linux便携CLI，经官方资产SHA256 `fc0327ba27e89bd086cf426dff17d77de582953cdbbc10a6576540a06853ffcd`校验，在WSL普通用户执行，未安装组件或修改PATH。实际列出38条、选取worker与许可12份解包并逐字节核对；对解包后的07d50d7a worker执行`--self-check` **exit0/60工具/协议1**，再执行`bun scripts/computer/freeze-tools.mjs --worker <解包worker> --check` **exit0/22动作**。证据根`%TEMP%/nomifun-nsis-unpack-j6o2qmqy`，`audit-summary.json` SHA256 `c7d70e80a32031439aacd88c7203c25a06ebc29681b7960ad86b6d85f2646827`；这是实际payload证据，不是安装或签名通过。
- **包内App身份**：随后仅解出30份应用/runtime文件，不执行NSIS或注册安装。包内App **364,457,984B/SHA256 `73c59986e153b97459a6e2f47c5c68f8328dddc26b51c8d5f091e1694253bdca`**；与stage `5920a6e6...`只有`.rdata`固定bundle marker的3字节UNK→NSS不同。官方Tauri CLI 2.11.2源码确认打包前写NSS、打包后恢复stage；仅内存替换这3字节后的stage SHA与包内App完全相等，另29份文件与输入一致。`payload-app-proof.json` SHA256 `0d1cdf70057650f78975345356e21a86b6cf90c3079c3f0274c86107b70db4dd`；不混同两个App哈希，worker仍为07d50d7a。
- **真实Desktop正常退出**：现役`run-windows-desktop-lifecycle.ps1 -DesktopExe <stage5920> -Mode quit -KeepArtifacts` **1/1、exit0/5.72s**。自有host PID79832正常ExitCoordinator退出，精确native句柄见证 **10/10 WebView2子进程退出**，独立测试sentinel存活后由其所有者正常清理。证据`%TEMP%/nomifun-lifecycle-01a15a2f-9115-7916-846e-5c84837fa7b8`，`native-exit-proof.json` SHA256 `376cfa8ec7947093852fa9402a67868c14ea62a88d36a63e373c590a4cfdd1ed`。该case未启动Computer，不能单独证明App内部Computer worker退出。
- **真实Tauri/WebView2控件**：新增opt-in `owned_tauri_smoke`，显式传入上述解包App与worker，使用现役debug lifecycle fixture的全新目录/标识/浏览器profile，保留exact Child并逐次核对AX回包PID/HWND，再使用新snapshot token。最终 **1/1、4.33s、6/6调用**：在自有Guid页面一次后台输入公开测试文本，新AX精确核验值；再用新token一次展开主题菜单，新AX确认浅色/深色同组radio且编辑值保留，不发送消息、不选择或保存设置。PID81468/HWND988806、96 DPI，external test worker PID50936；所有请求foreground=false、截图0、无输入重放、正常关闭全部true。报告`%TEMP%/nomifun-lifecycle-01a15a4f-89e0-7450-b6d0-c172d6d200b0/owned-tauri-report.json`；example编译期源码SHA256 `0dd3e6d9630a42d9f74e21e149cec2e63edb575c8e0c9acdd9f314490e458062`、实际EXE `7669c75d368601e2757960950cbe6fe5e162234e51c6a255c2fcf8b229a85c7c`。外部测试worker与App内部worker不同；foreground=false描述请求投递方式，不声称App启动从未改变系统焦点。
- **诊断失败保留**：Tauri前两轮分别因多owned HWND和误假设WebUI登录页而失败，0次输入；只读观察确认首轮仅5个窗口chrome元素，约500ms后出现真实web内容。上游`elements_complete=false`为固定保守标记，不能据此判超时。测试改用exact fixture title、真实Guid Document/唯一web Edit及有界只读等待；后来独立复核补上AX回包归属和编译期源身份后才取得上述最终通过。未改vendor或重放输入，早轮不计成功。
- **真实重复会话与空闲退役**：新增opt-in `owned_session_repeat_smoke <worker> <owned fixture> 50`，同一Computer宿主/worker代际/Agent名执行 **2轮暖机+50轮采样、52/52 AX读取与会话关闭**，无截图或输入。host PID72032句柄179/USER1/GDI0恒定；worker PID34380测量初值344/8/0，第16采样到348/9/0，之后 **35个点保持相同**（完整原始序列保留，含短暂GDI2）。读取耗时min/p50/p95/max **56/112/130/140ms**。末轮后约9725ms在显式shutdown前精确worker句柄退出0且owner stopped，再正常关闭宿主与fixture。证据`%TEMP%/nomifun-computer-repeat-0ac80175-c5fa-4156-839b-1a01b36d1cc3/repeat-report.json` SHA256 `9fed523b67e14d7d95829852d21d78ddb01a78f555fc8c0dc59611e8a0d6e21b`；源码 `1e96deaea1c8b97b63c8b4f337bf943aa50ec933ad530714d34892610ba0f50b`、EXE `2087e1788ccc96f0b6d3c19b9fda8fcacffb3bf5f982fabb59d1536c41a25acb`。这是一条有限Computer/native worker资源曲线，不替代整个Agent Runtime或长期泄漏结论，未加阈值、配额或机器策略。
- **当前worker的用户App存续**：现役`launch_lifetime_smoke` shell路径在07d50d7a worker上 **1/1、exit0/3.146s**，只启动自有GUI。native observer保留精确进程句柄，确认app PID76532不属于worker job、worker PID68808属于job；close_session、worker空闲退出与宿主shutdown后app/window始终存活，最后fixture正常关闭，无私有driver环境泄漏。证据`%TEMP%/nomifun-current-shell-b361daa2ca5248be81a399916ecca739/observer-report.json`。这证明当前end_session/退役保留用户App，不冒充被OS拒绝的start_minimized路径成功。
- **发现并修复的生产缺陷**：原生start_minimized请求在Windows前台锁检查处结构化拒绝，尚未启动目标进程；原宿主却记Unknown。仅在`isError=true`、effect/status键完全缺席且`code=background_unavailable`、`event_kind=app_launch`、`delivery_mode=background`精确组合时归Failed；固定vendor唯一该生产点位于所有启动分支之前。其他后台错误、显式unknown/partial及畸形marker保持原规则，不按文案猜测、不重试或切前台。`launch_lifetime_smoke`新增保存原始回包及归一化结论以保留失败事实。Computer纯自动化 **27/27**（新增3项），独立只读审查通过；未改vendor、worker协议、动作定义、API或canonical存储。
- **修后原生拒绝复核**：新鲜`%TEMP%/nomifun-current-launch-fixed-95dc3c7747b7465bb819732f3f51928c`，只发一次上述自有启动请求，约1.24s。回包completed/isError=true及精确三字段，现在effect=failed、target ready文件不存在；启动/observer例子均按原生拒绝退出1，**不计启动成功**，外层断言与收尾exit0。`launch-response.json` SHA256 `a92ca541f35fec57b5d973d81f3b39f0224b2574d1c0f760075f9587324ee5b0`、`controller-report.json` `7b57ccc08b48f56b0d7816b89dad1f6cac58ce2aa02b742100e7b0af1f65fe80`；当前例子EXE `fd8f02977b467a7b4bba8fe51cf1b51067c67cda2277f372b1ec43f83400e090`，worker仍07d50d7a。无输入、截图、强杀、权限或焦点设置修改，未重试这一业务请求。
- **修后自动检查**：`cargo test -p nomifun-app --features browser-use,computer-use --lib computer_ -- --test-threads=1` **18/18、1m04s构建/1.70s测试**；完整`bun run check` **exit0**（含Computer vendor/冻结22、桌面2247源与canonical/生命周期边界），文档检查 **145文件/1458本地引用、exit0**。三个opt-in例子统一`cargo build -p nomifun-computer --example launch_lifetime_smoke --example owned_tauri_smoke --example owned_session_repeat_smoke` **exit0/2.01s**；这次重建的EXE身份不能覆盖上列实际运行时哈希。
- **修后开发包**：同一dev配置和前述`bun x tauri build --debug --bundles nsis ...` **exit0、Rust25.75s**；stage App **364,458,496B/SHA256 `988d667aae2a7af52e5c0ab4456f6b290f90583d980bf9777c7b281194eb48c3`**，NSIS **89,138,546B/SHA256 `e4b1a65548738bf4467c0c26acbc4415e48c76b69bd5f8221fc401a4fcc9c7dc`**，07d50d7a worker保持。App/NSIS/worker仍NotSigned，未安装或发布。生产result源码SHA256 `8eafd86f9245a0c1d37d9326c92fa01e5f7b0399bc51449acfdf2874f558004f`，launch例子源码 `a2880feb4feccb4b5491c770f9459852cc1c0c4fa9b7dc51443ab3941396a8bd`，两新增例子源码保持上述身份。其余UI、API44、冻结与独立worker不变，复用上轮有效证据；源固定后fetch确认origin/main仍8dacffd05。
- **修后包内实物复验**：复用已校验的便携7zz，在全新`%TEMP%/nomifun-nsis-refresh-emwc2w2_`提取实际30份runtime；除App的同一3字节UNK→NSS变换外，其余29份含worker/许可资源逐字节一致。包内App **SHA256 `de0eea3bb98db8364c7729c0d89219ca896653bf28734bdcbe5f4e3a484c45ca`**，实际解包worker自检 **exit0/60工具/1.448s**；`payload-app-proof.json` SHA256 `ac509ba6112348971a2b3f4d2cbf35da279b10f2634bb293faa310c545b5c2de`。随后以此包内App与worker运行同一`owned_tauri_smoke` **1/1、exit0/5.62s、6/6调用**，PID33528/HWND15275420/96 DPI；4份AX回包归属均精确相等，编辑文本/主题弹层再次核验，图片0、正常关闭全true。最终报告`%TEMP%/nomifun-lifecycle-01a15a5d-a041-7812-8849-af196995e966/owned-tauri-report.json` SHA256 `961d18e1c334cafc87e39339af7309b2e674fc7e0a09b1b28090ee024907f907`，当前example实物 `e0bc964eae0b2ee42147a29adc05e20c52762c0ddd4ec2b20d221390642d5fb5`。无安装、注册、签名或发布；四份源码raw blob与暂存index逐一核对相同，未提交任何实物、截图或私有数据。

### App内部Computer退出与现役文档清理（2026-10-21，S4本机增量）

- **基线与交付身份**：上一轮源码/进展已在main `5bb2787456f9b16a39b3d024b381b8297aab9dc5`推送并核对干净；本轮开始检查status/branch/HEAD并`git pull --ff-only`仍最新。继续使用已解包的实际App `de0eea3b...` 与worker `07d50d7a...`，生产Rust/UI/冻结/协议未改，不重新编译相同App或重复实模验收。
- **补齐退出证据**：旧正常Quit仅启动WebView2，不能证明App内部Computer回收。现役两验证脚本新增仅测试用`-WithComputer`，限定`-Mode quit -WithAgentResources`且不组合Stop/父死；只在全新隔离dataset的preset中按冻结清单派生并授予`get_guide`，由本地scripted provider通过正常HTTP/Agent/Kernel链调用一次，随后保持原MCP调用在途。没有产品开关、App hook、新事件或桌面输入/观察；固定guide实际经start/bind进入真实worker/Core。
- **纯自动检查与首次失败**：Node `--check`、PowerShell AST parse、其C# P/Invoke类型编译、diffcheck均exit0。首次native在`%TEMP%/nomifun-lifecycle-01a15a67-70f4-7471-9739-6b80f2d2e1ca` **0/1**：裸`node`在本机返回两份Application路径，旧fixture把数组交给ProcessStartInfo，sidecar启动失败；当轮没有Agent/Computer调用。已修为PATH优先首个可执行文件，显式绝对路径仍唯一。现役finally结束其exact自有Desktop/helper并移除临时owner JWT；不将该setup失败改报业务成功。
- **真实Desktop正常Quit最终通过**：`powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File scripts/validation/run-windows-desktop-lifecycle.ps1 -DesktopExe <解包App绝对路径> -Mode quit -WithAgentResources -WithComputer -KeepArtifacts -NodeExe <现有Node绝对路径>` **1/1、exit0**。本地脚本模型 **7次请求**、准备12,920ms；canonical唯一Computer proposal/start/completed分别seq122/123/129，guide实际6 topics、无error/hold/图片。host PID62400；内部worker PID15324/generation `32aa0d88-8707-4c3e-ba5e-955251b29ea4`，原native HANDLE核对image/创建时间/父子与命令行generation，Quit前再查同一status，bound_sessions=1。真实ExitCoordinator退出0/cleanup verified；**14/14原生句柄退出**：10个WebView2、pipe/ConPTY/在途stdio MCP各1及内部Computer worker。worker同一句柄退出码 **0**，不把job被动终止冒充协议正常关闭。独立测试sentinel在全部退出观察后仍存活，再由driver清理；模型sidecar/driver helper的现役结束与产品正常Quit分别记录，不称全程无TerminateProcess。
- **最终退出证据身份**：根`%TEMP%/nomifun-lifecycle-01a15a68-583f-7c47-b39d-cfa84799fdd4`，`native-exit-proof.json` SHA256 `f993136b13f4d1ebf80bd4dca18ea3927b924b3717e2f120a8568a667f33f2a0`、`agent-resources-ready.json` `ef85f2a81fc27ec0225461bb595210bedb044f8c96430fc9045b9f5bb7f410d5`。PS源码 `3fa7b8f8b305c8e3cdf6a5ec42d25a365062401bada0fd393411fa475e8d15fa`，sidecar源码 `be21ef83c8f06647afa41d635bbb84255e35f342b4737be1daf49b47cfa91cf2`；普通测试owner JWT已确认删除，既有Step Plan凭据未使用。仅模型被脚本化，原生worker/资源与退出是真实，不能据此增加真实模型成功数或故障/父死验收结论。
- **现役文档残留已修正**：`agent-session.zh.md`仍有两段旧ComputerTool/ref/Observe序号授权缓存合同，与新Host不存在的状态相矛盾；已替换为实际canonical因果/Snapshot检查、模型attempt表面回执与私有worker的独立代际/session/token责任，保留首次native fence为0合法等通用规则。系统许可按冻结`system_permissions`及准备后的参数计算，复合观察可同时需AX/屏幕录制；去掉旧观察授权撤销。英文架构同步当前合同；Windows接续中旧高完整性桥/remote focus改为当前普通用户worker边界；生命周期接续不再调用已删除的旧Computer原生脚本。未改canonical存储或测试历史数据，历史验收记录仍按原源码身份理解。文档检查 **145文件/1461本地引用、exit0**。
- **真实系统Calculator观察**：新增opt-in `owned_calculator_smoke`，显式worker/安装包CalculatorApp.exe/AUMID参数；Windows11 Pro x64 build26200，实际包11.2607.0.0，Calculator实物SHA256 `f26a20f8bd65fd23f9a7cf13145a52b01f37beb51962a7aa3b7892eb7894b242`。先查进程元数据确认没有已运行Calculator，再launch一次；typed PID实际指向既有共享ApplicationFrameHost，因此要求新frame、唯一CoreWindow子PID、精确Calculator路径/创建时间与native HANDLE全部成立才读AX。首次纯观察 **1/1、1.80s、2/2调用、39元素**；新Calculator PID11004/frame1119862，96 DPI，随后仅向重新核对的该frame发WM_CLOSE，Calculator退出且旧AFH保留。证据`%TEMP%/nomifun-calculator-observe-d83b5395-ec0e-44f1-90b6-5056e9aadf19/calculator-report.json` SHA256 `44048e9c2adbd015cdfae54160649300dbdf8a02e2baaf288da35bb70f06afee`，该轮源码 `cbe410f57d42423a0c49d8e2848168179f06fcc024ec3af4b65a01dff745630d`/EXE `cf3e2e1675295d0c798a08bb41e96c1ad35aa413ed974989bd129c32ec97366a`。不按共享AFH PID关闭整个应用，不读取其他窗口内容，已有用户记事本实例保持不动。
- **真实系统Calculator输入最终通过**：按第一次实际AX的中文标准模式和唯一invoke按钮扩展该例子，默认清除、二、加、三、等于，保留`--observe-only`且该模式不授权click；最后`cargo build -p nomifun-computer --example owned_calculator_smoke` **exit0/1.21s**。最终 **1/1、exit0/4.34s、13/13调用**：1次AUMID启动、7次定向AX、**5/5后台accessibility invoke**；每步重新校验native归属，使用不同snapshot2–6的新token且关联确切读取/点击序号，原生回执同时确认route与delivery。最终snapshot7在exact frame上唯一Text `显示为 5`，按该真实label核验结果，不虚构ValuePattern值。新Calculator PID77864/frame18486510，旧AFH31972；正常关闭仅此新frame，Calculator进程实际退出、共享AFH仍存活，session/worker正常关闭全部true。无截图、像素、前台请求、重放或强杀；AUMID正常激活可能改变系统焦点，后台描述仅用于上述五次invoke。这是实际原生驱动验收，**未调用Agent Runtime或真实模型**。
- **最终Calculator身份**：`%TEMP%/nomifun-calculator-native-d8b974e6-a20d-46e5-9c45-b4e535ebbb31/calculator-report.json` SHA256 `e2a554cbf73a54e7849ed1b2b8fa645f92924e3578bb83e905617e6fe177a704`，`owned-calculator-final-ax.json` `385f722eca3a65683c59f079a3f12d76f099419131a11ecbb49fe83ee48f3cbf`；最终例子源码 `8d8b223d5b6d76b589d3c4f05a5a73bb6ab4394e7f7ccf36a39a6b7cf1c51fa6`，EXE `5d014d6dc0e85a1f4023080bd4ceff19aa3043c5159a6d96a433e27412a60493`，worker仍07d50d7a。全部快照/报告在仓库外，未提交用户数据或截图；不将早轮纯观察身份覆盖为最终输入实物。
- **确证的fixture隔离缺陷**：准备unsigned安装验收时查明，旧Desktop lifecycle fixture仅隔离dataset/浏览器，main仍调用`keyboard::start`的production路径并写固定`NomiFunKeyboard`自启/共享endpoint。后续只读元数据与已知测试路径纯计算确证：当前Run值指向本轮解包的临时de0eea3b App；持续运行的Keyboard服务PID75728来自本轮临时73c59986 App，原生创建时间`2026-10-20T16:52:07.4350209Z`；共享desktop描述指向已退出的本轮host62400。故旧GUI轮次不能声称未触碰共享Keyboard；此前计算/AX/worker退出证据仍是真实发生，不据此掩盖隔离失败。原自启值在测试前未记录，不能推断或猜测恢复，已向用户具体说明并异步询问原设置。
- **隔离修复与新开发包**：仅在`main.rs`复用已有严格Windows debug lifecycle fixture guard，提前验证fixture后跳过该模式的Keyboard early/start/shutdown/update入口，阻断共享注册、硬件/音频/IPC与stop-owner；正常无fixture、非Windows及release的原入口/能力保持，未加产品开关或改Keyboard模块。独立源码复核通过；main源码SHA256 `7567be12e27c11ccedb85e440e113c404fe7270c5b407fa533210bce5466eba2`。同一dev配置Tauri NSIS构建 **exit0、Rust36.94s**：stage **364,459,008B/`9051a960e5d8d95491421000121b9e3e85991fecdbbfe8ee7fae23d01898c6e7`**，NSIS **89,112,476B/`97c38f2a938519d500ecbc6d9f56abd300b8335f41f34905785c6225e66a265a`**；实际解包App **`87d0edb91f668b758a3ec5fe3aac2048fb8d4324a34944d7ae357dedb6e590d1`**，worker仍07d50d7a。全新`%TEMP%/nomifun-nsis-keyboard-guard-k4zprkya/payload-app-proof.json` SHA256 `b57c35292a4ed0713b162f005073223c48d2d139c517c95822bb985fc8053102`，30份runtime实提取、仅预期3字节bundle marker差异，其余29份raw相同；实际worker自检60/协议1通过。均NotSigned，未安装/发布。
- **新包真实退出与共享状态核对**：PS新增只在`-WithComputer`执行的启动前、ready后（model sidecar前）、真实退出后三点元数据核对：仅特定Run值的存在/原生类型/raw摘要及两个精确endpoint文件的存在/大小/raw摘要，不输出值或解析正文，不读数据库。实际该值/两文件在三点均存在且摘要相同，防止启动写入、退出删除被两点比较漏过。新包重跑上述正常Quit **1/1、14/14句柄退出、内部worker82420退出0**；所有原有native身份/Actor/未知效果门槛保留，owner JWT删除。证据`%TEMP%/nomifun-lifecycle-01a15a83-5ba5-737c-9f71-2e60fa3bc1ec/native-exit-proof.json` SHA256 `4adf3ffcd17c694889b2a6a76dad60e8d4c3b504eff16d5169a68f56a80f8669`，`keyboard-shared.json` `5a46112db3c3c4aea4efabd18b0d38a051bb04c29d5241f2d8618fd5f9539701`；最终PS源码 `23f5188e9fdf06600cf62f8cca074b51538d67c5523d429df4fae00ee52b1667`。这证明新fixture在该轮未改已有元数据，不等同此前原用户设置已恢复。
- **新包Tauri复验与测试定位修正**：新包第一轮 **0/1、58次只读/0输入**，动态placeholder一度只有部分“发消息”字样，旧测试把展示文案误当固定locator；改为exact Guid Document中的唯一可操作web Edit。第二轮 **0/1**：输入与主题展开均成功，但popover另有“缩放”Edit，测试错误要求展开后仍只有一个Edit；现输入后按已知公开测试值精确区分，值后置条件未放松。最终 **1/1、exit0/4.27s、6/6调用**，同一新App87d与worker07d；所有值/菜单/正常关闭核验通过。最终源码 `42aaccfe645d8c75fa84699088dfe96bb9230e62aeed20374178597150e474fc`，EXE `1202e03d71e62ce81658f973be6db7a7957593493b1547199ed48e75645ebce9`；`%TEMP%/nomifun-lifecycle-01a15a90-d2f5-72e3-80f3-574ec0ec5a49/owned-tauri-report.json` SHA256 `024836364802da069e51e9fd705a71e7887b6d3aa57cabc44dec4d30325e25ac`。失败两轮均正常关闭、无重放，未改UI/Cua产品行为；不删除早轮失败记录。
- **精确清理已确证的测试副作用**：一次性Temp helper严格绑定上述PID75728的image SHA/路径/完整原生创建时间并保留无terminate权的进程句柄；非阻塞取得现役desktop/proof lifetime锁，核stale62400无live且文件摘要匹配，只按原file HANDLE删除该测试描述。Run值仅在REG_SZ/raw SHA精确等于已知临时App命令时移除（实际共享视图只删 **1** 次）；不猜恢复原值。所有合法创建/替换入口持desktop锁，NSIS另有未持锁但只删除的分支，already-absent可接受；不宣称全部producer互斥。随后创建自有proof，核loopback四元组确属75728，沿现役认证management协议只发送 **1** 次Shutdown，取得completed及原句柄退出码 **0**。服务自行撤下owner，只删除自己的proof并释放锁，不向GUI发送stop、不kill、不删用户owner/目录/数据；复核75728、两测试descriptor/proof均消失，lock文件保留。先尝试TxR时RegOpen返回6801、rollback成功，**零读取/删除/commit**，未启用系统组件；后续清理是单独审定的现有owner锁路径。
- **清理证据与待确认项**：一次性helper仅在`%TEMP%/nomifun-owned-keyboard-cleanup-90bb532052ab4564b0f30ff57c7f9e74`，源码 SHA256 `7e166c7e40c2cd4d8ee6b5044e22256a0adb5e8ec2ec6ae60c10826f6031c981`；执行 **1/1、exit0/1.41s**，`cleanup-report-e96c61f860dc48adbe6a9f15da5e8980.json` SHA256 `b4d95f86008c28cadad9633e973fb4524bc91cc30e8f50546c5c80a7e12cd7ec`。helper/私有IPC凭据/原始设置/报告不提交，未新增产品清理、迁移或兼容代码。当前错误测试自启已清除，**原用户登录启动偏好仍待用户回复后明确恢复**；清理成功不能冒称恢复了未知旧值。其余安装验证继续独立推进。
- **本轮最终检查与下一动作**：源固定后完整`bun run check` **exit0**，含文档145文件/1461本地引用、renderer2247源、i18n **6/6**、vendor458文件/3补丁/冻结22动作及现役canonical/生命周期边界；工作区diffcheck0。新增/改动原生例子分别由最小Cargo构建及上述非零原生用例验证；生产backend/worker/协议/动作/模型配置未改，复用上轮27/27 Computer、18/18 App及实模证据，不机械重跑。下一步使用现役UUID独占身份、关闭共享Keyboard hooks及WebView系统组件安装的临时packaging overlay，做无GUI的unsigned安装/同步卸载；此项不替代正式签名或默认Keyboard安装合同。

### Windows release安装、包内worker与只读取消（2026-10-21，S4本机增量）

- **基线与范围**：main `d93be95a81f65f75c35e30d4029d0fa64330675e`开始、再次`git pull --ff-only`仍最新；仅新增只读原生验收例子及修复验收脚本的模块加载，不改产品动作、协议、模型配置、UI/API44或canonical存储。全部测试使用普通用户token、当前桌面的自有WinForms窗口及临时数据；未使用模型凭据、生产数据或已登录浏览器，未提权、安装系统组件或发布。
- **独立优化worker**：`bun --no-env-file scripts/computer/build-worker.mjs --target x86_64-pc-windows-msvc --profile release` **exit0/3m55s**；实际artifact与staged sidecar相等，**20,780,544B/SHA256 `605b097d05ea526b0a7ee9c85167dc25a5665128e7996fd8eed6acc110178c06`**。`--self-check` **60工具/协议1**、`freeze-tools.mjs --worker <该实物> --check` **22动作**；同一独立target/build-dir执行`cargo test --locked --manifest-path apps/computer-worker/Cargo.toml --target x86_64-pc-windows-msvc --release --test protocol` **13/13、0 ignored/filtered、39.87s构建/0.67s测试**。不执行桌面动作；vendor仍为4d959bbf、458文件/3登记补丁。
- **release包与实际身份**：专属证据根`%TEMP%/nomifun-computer-install-config-193c23fb6e1c41c099a71aee6c33a609`内的`identity.json`（SHA256 `fa44bb2a0230c196e2dc28625182f53cda830980d1938812e039153114e10c31`）与`overlay.json`（`3f565ce697eb13f97a658d44a90855d242c4256bf9ce7312de17e3ebb38fbfba`）只用于本轮。`CI=true/NOMIFUN_EMBED_OFFICECLI=1/NOMI_CHANNEL=dev`，执行`bun --no-env-file x --no-install tauri build --config apps/desktop/tauri.conf.json --config apps/desktop/tauri.win.conf.json --target x86_64-pc-windows-msvc --config <identity.json> --config <overlay.json>` **exit0、Rust28m29s**；沿用已验证dist44，不执行prune/collector或target安装。唯一UUID身份、currentUser、关闭Keyboard hooks/文件关联/WebView安装与minimum要求/updater产物；GUI不启动。stage App **281,976,320B/`e59043d978ad8769a41b319e8d244c86ddb7321383bd5f0e708c16b11eb5e93f`**，NSIS **85,752,210B/`1b21144984223bfe90be48ec0f3d97c2e027a2a06cc96f89d427f90f8234e19e`**；两者与worker均NotSigned。
- **只读准备与真实安装**：现役`global-os-windows-installer.mjs --prepare`通过，报告`reports/windows-installer-prepare-b974ec53-c07b-4350-b21e-0c4e2276bda1.json` SHA256 `a812c5c1626eecaf86ca935f38b4d1b2ed210791352cb49ea5627738a6e429e3`。首次准备失败分别为证据目录继承ACL过宽、临时脚本误把Skip的JSON枚举当实际NSIS字符串，以及PS5无法解析默认注册值空键；均在安装器启动前。只收紧自有Temp目录ACL，按上游2.11.2源码确认Skip实际渲染为空模式且minimum为空，Node保留空键完整摘要后通过，不修改系统权限项或丢弃注册值。原始共享guard SHA256 `4d584dd79ded9d0efe1a7184b280411f65a9a067df9e95272b8e1657f6bcb848`。随后`--install` **1/1**：实际PID51352、ordinary token、`/S`、原生退出0、零process signal、GUI未启动；实际App **`bd87947965d421f08f06c9423238512a05d01d25d3dc8a606d451a57ac24cfa8`**完全等于stage的官方NSS marker预测，另29份worker/许可/resources逐字节相等。`windows-installer-install-b2e71a24-f52d-4445-ac5e-ca8e38c43b9f.json` SHA256 `37437113268e48a568145ba8baff75fb49e1cbb9bf5d88dd01bc2a1cd36cc8f3`。
- **包内worker真实只读取消**：安装目录内605b worker再次自检60/冻结22通过。新增`owned_cancel_smoke <installed-worker> <owned-WinForms-fixture>`，最小`cargo build -p nomifun-computer --example owned_cancel_smoke` **exit0/6.93s**；源码LF SHA256 `4b91c24b54b94978f1660a6a085a5901a5888e5dc11c8290f325bb95c5eeed24`，实际EXE `22a0442471b357f1c60bcc1ee2c7ea447dff92d0192d6b5294031ad66903289a`。**1/1**：4次宿主提交、3份真实Completed结果、1个排队B在派发前精确`COMPUTER_CALL_CANCELLED`撤回。A在宿主busy边界取消后保持waiting，等待真实`verify_state` **10,206ms/7样本/unsatisfied**，原控件值仍为0；后继C在同一session/worker代际新读成功。fixture81876、worker62088、96 DPI，原生句柄均正常退出0、两session关闭、owner stopped；输入0/图片0/重放0。报告`%TEMP%/nomifun-computer-cancel-6f845f56-f29c-416b-b77c-720e27c61a24/cancel-report.json` SHA256 `319ad25b7972f2e4f2d71cdc1dc4a97ff4273fc7276d132b1555215dd8eeddb5`。这是宿主/真实worker取消证据，不替代canonical Agent Stop、输入中Stop、崩溃或真实模型验收。
- **卸载前失败的精确归属**：首次观察器在`Get-FileHash`缺少autoload处exit1，尚无native回执；先保持未知，不重放。只读执行截至ProcessStartInfo之前的原前缀，得到与原失败完全相同的 **861B stderr/SHA256 `0c74d99573ffb320ff851592cfc2456d2abf96ba9dfee08d8669ec0d8ed6aea3`**，证明异常发生在Start之前、未派发卸载。显式导入现有系统Utility manifest后，同一前缀exit0/`COPY_HASH_VERIFIED`/stderr0。`reports/readonly-prelude-fixed-27e5f618-26ba-40ac-84b7-66f7911228c5/proof.json` SHA256 `6e0c884524aec09bb95bc709f9ad7548f590a7a3cbf7721753fbd26a04fbd209`；不安装模块、不修改系统配置。
- **首次实际卸载与保护结果**：Temp helper最终SHA256 `4629eb5eb958cd39e1ccf362b849c6fd46e4bd0bdd6f04d334127a32ad55739b`，仅前置上述导入、原canonical suffix保持相同。复制经SHA核验的唯一uninstall.exe，用原样最后参数`/S _?=<该唯一安装目录>`同步执行，**1/1、PID2836、ordinary token、原生退出0、零signal**；安装目录、12行唯一注册观察、两快捷方式及该App进程均消失，daily/Dev/Keyboard的注册、Run原生类型/raw摘要及endpoint/快捷方式文件摘要前后相同。实际使用的复制品随后由helper删除，未直接删除安装目录或修注册；`reports/no-gui-uninstall-5ec5e641-9b84-4fea-a998-ae4d4e9c7005.json` SHA256 `a075460ceaa2e1c0e290a051894eae521120e06fa28ba996cdd661213ebddd45`。此前未启动的79,385B复制品`canonical-uninstaller-754baafc-1f1f-487d-ab2b-6f35121226a7.exe`仍在私有证据根：自动审批拒绝其精确清理，仅给出blocked by policy，未重试或换路径删除。此项不影响已验证的产品卸载，亦不代表此前未知的用户Keyboard自启偏好已恢复；该偏好仍待用户回复。
- **维护修复与最终检查**：将上述模块加载缺陷以最小改动纳入现役`nativeBinaryGuard`：从PSHOME显式加载Utility manifest，限定`Microsoft.PowerShell.Utility\Get-FileHash`来源，其他执行合同保持。源码SHA256 `b8fddf09e4da1351c5f546207c91fd4c95bf21a5eebc4931881a0e4dec931f34`，定向测试源 `ff07bd7195af80349de154d54bd1e4be317c16e1658c89dbb7ce07f1d0e567a7`。`bun --no-env-file test scripts/validation/global-os-windows-installer.test.mjs` **15/15、161断言、6.11s**，含真实Windows只读guard回归 **679ms**：移除当前进程的Utility模块、禁autoload后，正确SHA通过、错误SHA拒绝、原文件不变，截断在ProcessStartInfo前，不运行任何安装器。最终`bun --no-env-file run check` **exit0**，含文档145/1461引用、renderer2247源、i18n6/6、vendor458/3补丁/22动作、安装器及现役canonical边界。产品backend/UI/契约未改，复用上轮Computer27/27、App18/18及真实Full/NoImage证据；无需机械重跑产品构建或桌面输入。实物身份与四份交付路径经独立只读复核，无过度原生/模型结论。
- **交付边界**：此轮没有模拟模型或输入，包内worker/原生窗口、安装器与卸载器均为真实普通用户执行；这是unsigned独立身份的无App GUI安装验收，不替代正式签名、默认Keyboard安装钩子、正式包或其他平台矩阵。全部实物、原始AX、注册观察及报告留在仓库外，未提交截图、凭据或用户数据。

### S3补漏与正式HTTP/native Stop（2026-10-21，S4本机增量）

- **基线与实际缺口**：从干净main `8070a619f51bc9634164afda00d727232885f0fd`接续，检查status/branch/HEAD并`git pull --ff-only`仍最新。复核发现S3虽已有入口，却有三处实际遗漏：首次启用Computer仍走通用只读默认（8/22），backend/AT-SPI字段从未赋值，现役Linux交接及图片指南仍含已删实现的施工指令/过时图片保留合同。本轮修复这些遗漏，保持当前API44、冻结动作与canonical存储，不新增迁移、兼容、产品开关或事件通道。
- **默认与文案**：Computer首次启用从catalog的完整actions派生默认选择，不手抄22定义；其他模块的默认及已有明确Computer选择（含空列表）保持，不加升级路径。现有capabilityChanges窄回归直接消费冻结JSON；相关三个UI文件测试 **12/12、45断言**，typecheck、desktop UI boundary **2247源**及i18n **6/6/8171keys/33模块**通过。中英文说明补后台优先、必要前台、系统权限与平台/应用限制。`bun --no-env-file run --filter=./ui build` **exit0/33.20s**，dist API44/build ID `4c8ad32b-0b82-4479-9f83-ed167c6d99bd`。
- **真实诊断接缝**：NomiFun私有initialize结果新增独立diagnostics，与原SDK metadata分开；只有实际embedded runtime初始化后才标当前native factory/dispatch家族。backend是实现标识，不能当权限、UIA或抓图就绪；Windows/macOS不为此额外枚举桌面、截图或提示。Linux走现有免Session的只读`check_permissions(prompt:false)`，仅保留真实bus探针布尔值，不保存文本/D-Bus地址，不猜portal/wlroots可用性、不启动额外Session；沿既有startup/process-owner预算收敛，不丢弃blocking探针。宿主每代覆盖两字段、退休清除，缺失/失败保持未知，GET status只读缓存。实际协议在隔离Linux bus上验证backend=x11/atspi=false，Windows验证win32/atspi不适用；新的假worker缓存/退休/新代际回归通过。worker新增源 `native_status.rs` SHA256 `e960a528a2c7947fceafe8db23f8ca0481139e8742965dd901ce1025663dd0fa`，private_worker `d5f6d4f5b5defb8f08ccb189d5bcae1e1f7655673bfece3d6cd23c803538daf0`；宿主worker `ecc724a08be192108c3da5842d471730bc50b36456481d0e3a491c9849a30724`。架构中英文同步缓存、未知与就绪边界。
- **worker与宿主验证**：Windows独立release构建 **exit0/43.15s**，artifact/stage **20,785,664B/SHA256 `52f476874ad6b2f4da211464b90cf198e2870bef16d64f004c9b7f8d6731fef9`**；self-check **60/协议1**、冻结 **22**通过，同target/build-dir真实协议 **13/13、0 ignored/filtered、39.59s构建/0.62s测试**。`cargo test -p nomifun-computer` **28/28、5.31s构建/2.59s测试**（含新诊断回归）；输入、半包、崩溃与超时宿主测试使用现有假worker，不作原生故障结论。WSL普通用户复用`/tmp/nomifun-computer-worker-s1-{target,build-linux}`，`cargo build --locked` **5.823s**、真实协议 **12/12、0.59s**、自检 **63**和实际Linux冻结 **22**通过；binary **564,611,632B/`c784d9e9362f4e487a1507fb4c1ebea8857f0fe46e5bf29c393088769ddea2cf`**。`%TEMP%/nomifun-linux-native-status-ak8xoxf1/report.json` SHA256 `5aa1c168fe74c4764de8e6a45f8fba65695deecb80f9f2892b855309d6bec0fc`；仅组件/隔离协议，无桌面/故障注入，不代表Linux原生桌面或Wayland。
- **正式HTTP/native只读Stop**：新增opt-in `native_computer_chain::runtime_stop::exact_http_stop_fences_runtime_while_the_real_read_settles_normally`，复用现有脚本Chat provider及owned WinForms，不加产品测试接缝。`cargo test -p nomifun-app --features browser-use,computer-use --test native_coding_reliability --no-run` **exit0/1m16s**；再显式worker52f/fixture b978/新artifact环境运行实际测试二进制 `--exact --ignored --nocapture --test-threads=1`，**1/1、0失败、12.20s**。同一Turn仅baseline AX及10s只读验证实际派发，模型另提议的后继读未派发。HTTP精确绑定canonical原输入，约 **23ms**建立唯一cancel fence（seq42）；同worker54960/代际`67fbb739-6655-4d9c-82f6-35971eb8091f`仍在waiting，真实 **10,376ms/7样本/unsatisfied/原值0**随后写入现有host_tool_settled和tool/result-recorded，未丢真实结果、未替换取消围栏。三次脚本模型请求、三个原生调用提议、两次实际读取、零后续派发、零Effect、零输入/图片/自动重放；不把脚本模型计为实模。正常AppServices关停后原worker HANDLE退出0、fixture86172 exact Child正常退出0。运行时源码 `381f4172d126e6b3b64fb1f5a672cbfcec95783eaa43af5b5fb0bdc7e303229e`、测试EXE `423e7c6d8061a5abf998b993886a074976a8923547e9e557d4ec0467461a8c6d`；暂存检查仅移除该测试末尾一个空行，提交源SHA256 `a37c3e6a21ddb989254baf6d3452e5ae14087177faeef352c96b34f7d43f1a50`，逻辑未变。`%TEMP%/nomifun-runtime-native-stop-5114dbceeb1c4e07ad127c7b8c0b2102/runtime-stop-report.json` SHA256 `ceb111d122b82bb4b510654f9d3a535934b0dab42475e7856225ddba0825a7df`。独立审阅修正了两次提交之间的测试竞态，现同时等齐唯一原结算和结果投影；无Runtime ToolCompleted或输入效果取消的虚构断言，也不外推整个长期Runtime实例退休。
- **旧指导与图片合同清理**：`global-os-linux.zh.md`不再指示进入已删crate/dev脚本，改指当前ComputerHost/五模块/私有worker/冻结及专项Linux接续；原e4/PID等保留为有身份的历史证据，其他域不改。Windows交接改为普通目标按冻结能力验收，高完整性只读按实际记录、受限操作保留结构化拒绝，不自动提权或前台重试。中英文communication移除Computer为MCP来源的旧描述，指向第一方Module→nomifun-computer→私有stdio worker；agent-engine移除旧Agent crate布局中的Computer。中英文guide删除max_recent_images/20张/出错剥图persist等旧描述，按当前typed图片、实际预算、未消费图与历史省略事实重写，需要时重新观察；Browser其他内容和nomi-config等其他专项字段不动。没有为旧数据/升级创建测试。收尾`bun --no-env-file run check:docs` **exit0、145文件/1468引用**，`git diff --check`通过。
- **当前开发包与总检查**：同既有dev overlay执行 `bun --no-env-file x --no-install tauri build --debug --bundles nsis --config apps/desktop/tauri.conf.json --config apps/desktop/tauri.win.conf.json --config %TEMP%/nomifun-computer-s2-dev-bundle.json` **exit0、Rust1m19s**，复用上述新dist与52f worker，未安装或启动GUI。stage App **364,459,008B/`814b73af8c5e72e84c659c9710ee2394cbdaab6b1e39621f374d513c7949c72f`**，NSIS **87,237,684B/`2135c473b3d04ef30b5cdf0d71137633fd503e6b3b92b685a18a76d5703d1448`**；`%TEMP%/nomifun-computer-s3-audit-package-c63fc3056c5c4104a76c23e2af43b196/build-identity.json` SHA256 `a191c405a4412ddc7822c6ff20e18e465c5601d15b660e94407c4a026c18797e`记录14份实际源输入。完整 `bun --no-env-file run check` **exit0**，含docs145/1466引用、i18n6/6、renderer2247、vendor458/3登记补丁/22动作与现役canonical边界；仅文档收尾再跑docscheck。正式签名、默认安装钩子、其他应用/DPI/平台、输入中Stop及故障矩阵不由该结果替代；原Keyboard自启偏好与被自动审批拒绝的旧临时复制品清理状态保持前述事实，未重试被拒动作。

- **当前包内实物复验**：固定上述2135c473 NSIS，复用已核验便携7zz（SHA256 `d1bc79f00e9815b24048ab167690bdd1dbc387de7dbc47e8d75d68689e49cf4f`），从38条安全相对entry只提取worker及11份许可，**12/12**与当前打包输入raw hash相等。解出的worker确为52f47687、**20,785,664B**；随后`--self-check` **exit0/0.656s/60工具/协议1**、实际worker冻结`--check` **exit0/0.219s/22动作**，manifest未改，Authenticode **NotSigned/无signer**。`%TEMP%/nomifun-computer-s3-audit-package-c63fc3056c5c4104a76c23e2af43b196/ownedpayload/payload-proof.json` SHA256 `f82475d392f1b781baf5aa4c0cfdca62c7792f37f5fd8e7201bcb7bc1246bab0`保存实际命令、log hash和12文件表。初次listing检查因未选uninstall.exe的Size为空而止步，保留该准备失败，只修未选元数据规则；不提取或运行该条目。未运行App/安装器/卸载器，无新增下载、系统安装、GUI或旧artifact清理。

### Windows真实后台输入中的HTTP Stop（2026-10-21，S4本机增量）

- **基线与最小改动**：从干净main `c02c4f835701d35d6ef9b0737f3413d76f2b45a6`接续，检查status/branch/HEAD并`git pull --ff-only`；提交前再次同步仍最新。仅改现役自有fixture、HTTP/native Stop测试及其parent注释：两个具体场景复用同一owner/canonical/关闭链，不加产品测试接缝或第二事件通道。fixture仅在显式`input-stop`模式预置公开ASCII文本，TextBox子类先正常处理自己的消息，再原子写DOWN、first held MOVE、UP一次性回执，不注入、不延迟、不拦截输入；原`functional`仍为空编辑框。删除已过时的静态fixture SHA注释，实际身份由每轮报告记录。
- **源码与编译身份**：fixture源SHA256 `0180ae21d2aca6ac15d704517cce984daa4c36a75c7701da7159530c1284f320`，Stop源 `8d76479913bcf23d1ffe55ccb784ad51b9ae998287b48222d359b14369fd3b32`，parent `f9258731d0072b464a1c0f050dd0585a84a4a9856d23ca7aa04ae8de11f95f83`。使用本机既有.NET Framework `csc.exe /nologo /target:winexe /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Web.Extensions.dll`编译，exit0，fixture **42,496B/`fe764abfe2bfb0b5fe4a0f4637fa3a3ee4d5067924d1ba200ea3c66801b8a694`**。最终 `cargo test -p nomifun-app --features browser-use,computer-use --test native_coding_reliability --no-run` **exit0/6.45s**（CI=true、NOMI_CHANNEL=dev、NOMIFUN_EMBED_OFFICECLI=1）；实际测试EXE SHA256 `2c74d87aee74b04fe5e48be594337041e36f301127c27d1bd0545dfd5425ed76`。两次原生运行均显式指定上述fixture、当前52f47687 worker和不同新artifact目录，以实际EXE `--exact <case> --ignored --nocapture --test-threads=1`串行执行。
- **真实输入Stop 1/1**：`native_computer_chain::runtime_stop::exact_http_stop_during_owned_background_drag_preserves_its_real_result` **1/1、0失败/ignored、8.41s**。只有模型是本地脚本Chat provider（三次请求），没有使用真实凭据。一次510×330自有窗口图片确已交付，从同snapshot `screenshot_frame`派生编辑框内坐标；独立审阅在运行前纠正屏幕`frame`误用和canonical raw-image断言，保留现役历史媒体descriptor合同。fixture84364/HWND16651556/子控件4396588、worker82316/代际`604c2133-592f-4552-894d-c3d623b530b3`、96 DPI；实际DOWN及带按钮MOVE已收到、UP尚未收到才Stop，canonical原输入身份与唯一派发均已核对。请求980ms、围栏观察1004ms，约 **24ms**建立唯一cancel fence（seq43），仍为原worker的waiting状态且未收到UP。
- **输入结果与关闭**：本次接收端实测 **1 DOWN/60 MOVE/1 UP、6060ms**，选中 **47**个公开文本字符，选区SHA与原文本切片一致、全文SHA未变、该控件自身capture已释放；测试只要求多个MOVE，不将请求steps冒充实收。驱动原结果仍如实保留`effect:unverifiable`及重新观察提示，独立接收端证明选区后置条件；不改写成驱动confirmed。唯一原`computer/drag` Effect从seq42在围栏前开始、seq44在围栏后Returned，`resource_key=None`；同call/operation的host_tool_settled和tool/result-recorded各一份，Turn保持Cancelled。三个模型工具提议仅两次实际派发（观察+原拖拽）、一个输入Effect、零后继派发/重放，canonical raw图0/历史descriptor1。正常AppServices关停后原worker保留HANDLE退出0，fixture exact Child由close.request正常退出0；无Desktop GUI/Keyboard bootstrap、强杀或系统设置改动。本结论仅覆盖普通WinForms后台PostMessage输入Stop，不外推前台SendInput、全局按键释放、其他应用或故障回收。
- **共享只读回归 1/1与证据**：同最终EXE在独立`native-read-1`运行原 `exact_http_stop_fences_runtime_while_the_real_read_settles_normally`，**1/1、0失败/ignored、12.10s**；fixture82600/worker82732/96 DPI，约 **34ms**观察唯一cancel fence（seq42），原验证 **10,338ms/7样本/unsatisfied/值0**正常结算、零输入Effect，原worker和fixture均正常退出0。证据根`%TEMP%/nomifun-computer-input-stop-897fdcc53d2e48bfacc7fbf5a2378e23`，`native-1/runtime-stop-report.json` SHA256 `0be35c36af4bfbb036da33cf09d8f502b679ea5e2877a0d07e198fbb61d57525`、`native-read-1/runtime-stop-report.json` `209c351ffe392f1fb6edd4280aa5c1071b8017ef33477b3b78255b73f419a978`；`identity-proof.json` `db529f368fea2b6fda78248722bf18d6bd2b5939335eba11d0d2649c7c8445a2`记录命令、源/EXE/log hash和两份关闭结果。产品Rust/UI/worker/契约未变，重新核对上一轮12份未变输入及App、NSIS、worker三个实物hash相同，复用已通过的构建打包证据，无需重包。截图、数据库及原始回执仅在自有临时根，不提交用户数据。
- **总检查与其他Windows矩阵事实**：完整`bun --no-env-file run check` **exit0**，含docs145/1468引用、renderer2247、i18n6/6、vendor458/3登记补丁/22动作及现役canonical边界；文档收尾再次docscheck、diffcheck通过。只读确认当前已有用户Notepad实例；安装清单只声明`%1`文件参数，未证明能强制独立新窗口并排除旧标签恢复，因此没有启动、关闭或读其UI/配置/内容，真实Notepad保存继续待验。Settings功能、150% DPI、High-IL及其他平台仍按可得环境补验；没有将新增后台输入证据扩成未执行的原生故障结论，未重试此前自动审批拒绝的动作。

### D10等待可见与Windows签名接线补漏（2026-10-21，S3/S4本机增量）

- **基线与真实缺口**：从干净main `3251c590ed043db5b931765723ff804f9e399e0a`接续，检查status/branch/HEAD并`git pull --ff-only`；本轮再次同步仍最新。逐条复核发现D10的真实等待状态虽已被原生HTTP验收证明，却没有UI消费者；另发现现役RC要求timestamped Host/package，但PS仅给worker传时间戳，且Tauri 2.11.2打包后恢复原Cargo Host，RC可能验证到仍未签名的stage。先修这些本机实现缺口，不以缺证书跳过代码接线，也不把自动化结果冒充正式签名。
- **D10现役UI**：前端只补声明服务端已有`phase`，保持API44。全局驱动卡在`running && phase=waiting_for_current_operation`且本次读取无错误时显示“正在等待当前桌面操作结束”；使用整行Alert，避免既有nowrap状态标签裁剪长英文。mounted/running时每秒通过既有GET只读刷新，沿SWR可见性规则停止后台刷新，离开Tab卸载后不再读；实际操作结束后等待提示自动消失，读取失败显示未知而非保留缓存等待结论。全局进程状态不归给某个Session，不新增事件、动作、端点或存储。卡片源SHA256 `d87bdf75263393c9e3de835fafad7975d2a9e276fc2827022b5f1c4030221f91`，中英文及生成类型同步。`bun --no-env-file test --cwd ui src/renderer/components/settings/SettingsModal/contents/ComputerDriverStatus.interaction.test.tsx` **5/5、42断言、5.11s**，覆盖自动消失、卸载停止读取、错误不沿用缓存；typecheck、desktop UI boundary **2247源**、i18n **6/6、8172keys/33模块**通过。
- **最小签名接线**：`desktop-build-win.ps1`只将既有`WINDOWS_TIMESTAMP_URL`同步为Tauri overlay的`timestampUrl`和`tsp:true`，保留worker相同`/tr`参数及unsigned/custom passthrough。RC在任何构建前要求既有证书指纹与TSA配置，在desktop cwd用`cargo metadata --no-deps --locked --offline --format-version 1`核对实际target_directory为其固定repo/target，避免用户Cargo配置使它误签旧默认路径；构建后对Tauri恢复的x64/release stage Host使用同身份/TSA单独sign、verify，再走原Host/package Authenticode admission及release lock。stage UNK与包内NSS是独立实物，不要求或宣称同hash，不加第二ledger。最终复核另修fresh RC暂存父目录缺失：先独占创建staging，再在try内建artifacts/copy/lock/rename；失败递归清理前复核本轮目录realpath、父路径、名称及dev/ino，碰撞不取得清理权限。PS按仓库CRLF规则保留实测输入身份，Git规范LF blob经属性过滤核对一致。PS源SHA256 `f6bfed4d317114bb2476f157873c202d8a58868d8dc4523a9718ca4cb728947b`，RC源 `09acbb88b8b08a359d5cde80ad3a31fed1462cfe47234dbe31d93209800a552e`。
- **受控工具、隔离文件与只读验证分列**：`bun --no-env-file test scripts/desktop-build-win.test.mjs scripts/release/run-windows-signed-rc.test.mjs scripts/validation/windows-authenticode.test.mjs` **12/12、48断言、1.57s**：实际PowerShell脚本的AST/私有副本与函数stubs验证worker失败即停、同TSA overlay及unsigned passthrough，RC纯计划/metadata拒绝和Auth纯函数保持；没有调用真正的rustup/Cargo build/signtool/Tauri或证书操作。另含真实隔离Temp普通字节暂存回归 **3/3**：四份复制产物及release-lock校验、重复不覆盖、缺包失败只清本轮暂存且保留邻接sentinel；这些文件不作为可执行程序或签名输入运行。真实只读Cargo metadata另行 **exit0**，当前实际target路径匹配。当前用户有效代码签名证书 **0**，指纹/TSA环境均未配置；实际RC命令在此环境按预期 **exit1/signed_rc_certificate_missing**，尚未进入metadata/构建/签名，回执SHA256 `ae81f33314004814d625086e81cba3b79cfb2237bf483c9504a9038a312700b2`。这些不是正式签名、公证、安装或真实模型验收。
- **最终开发包与身份**：最终`bun --no-env-file run --filter=./ui build` **exit0/24.95s**，dist API44/build ID `7cf81040-f9f5-4860-bb7e-63f1cd96d493`；同既有dev overlay执行 `bun --no-env-file x --no-install tauri build --debug --bundles nsis --config apps/desktop/tauri.conf.json --config apps/desktop/tauri.win.conf.json --config %TEMP%/nomifun-computer-s2-dev-bundle.json` **exit0、Rust15.92s/一个NSIS**。App **364,458,496B/`e301ad8b5f2e08eae857f4cc34000659d550185140de624ca69691f3796d1063`**，NSIS **87,225,788B/`936a18017bcc42b408de59289acc9a33857398f4922098c4f6b59f2ff170fe24`**，worker仍 **52f47687/20,785,664B**；三者实际Authenticode均NotSigned，未启动App GUI、安装器或卸载器。证据根`%TEMP%/nomifun-computer-d10-signing-cc4714f9b32140dd82bf3653e5203688`：`build-identity.json` SHA256 `7db5d20bac7a0999e05b3ec09e4e987594f358ebb5ab2f21b9994a900de9875e`含11份源/三个产物及实际命令，`validation-proof.json` `23d8622c0e62e9c6546371cd3daae5f228fd9defe27f83a326011375ebf80f21`含验证分类与日志身份。App Rust/worker/冻结未改，复用此前真实协议、Full/NoImage、HTTP只读和输入Stop证据；本轮没有机械重跑桌面操作或受影响范围之外Rust测试。
- **总检查与下一动作**：完整`bun --no-env-file run check` **exit0**，含docs145/1468引用、renderer2247、i18n6/6、vendor458/3登记补丁/22动作及现役canonical边界；日志SHA256 `908e5e06275e8e3242838d81f2152b9a524d8147037db26ec0b9c3d659986428`，收尾另跑docscheck/diffcheck。Windows正式签名仍需真实配置；当前Linux入口为WSL2 Ubuntu（UID1000、DISPLAY/Wayland变量存在，未见GNOME Session），不冒充原生GNOME/portal验收；macOS未提供原生入口。原Keyboard自启偏好和此前自动审查拒绝项保持已记录边界，未猜测恢复、重试或换路。继续按发布范围补S4，S5入口仍保留。

### Settings显示页入口POC与桌面阻断（2026-10-21，S4本机增量）

- **基线和范围**：从干净main `5f93cec92a9ea600db68039cb3885bd7af611ee5`检查status/branch/HEAD并`git pull --ff-only`，收尾同步仍最新；11份现役输入、三个开发包实物和已通过全量检查日志hash均与前轮一致。T01–T13/A01–A07逐项只读审计未发现新的确定产品实现缺口。为继续本机Settings/150% DPI矩阵，先检查入口；没有机械重跑既有测试、模型或包，也没有改动生产代码。
- **两版临时宿主与实际调用**：UI工具直接启动已核实存在的SystemSettings.exe后未返回可定位窗口。按[微软正式显示页URI](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-settings)，改用当前`Computer::call`和52f47687真实worker作单次`launch_app {launch_path:"ms-settings:display",start_minimized:false}`探测，不加urls/参数，不自动重放。唯一CreateNew临时Rust例子在本仓库`nomifun-computer/examples`定向编译，源码和EXE完整留在Temp，仓库入口在核对绝对路径、文件名及Temp副本hash后仅删除本轮文件，不提交新harness。两次`cargo build -p nomifun-computer --example settings_uri_probe_a048b64c136f45eaa786f5d5de5cbd5c`最终均 **exit0/1.17s及1.21s**。rev1源SHA256 `415573c6ac5f8b8e64f6c9fa2364a5fe8235d9feb364e193c4e3b424ed51e5c4`、EXE `5db6fbe3218b604824d446345327f74d57055510386eddf03ff84bab49393a8d`；rev2源 `dbba33465b0b0d1dcc8197b17c8537f430875b9e0d582e305bb64284e04aca41`、EXE `3b68c37d068bea4ca98ae0bb75d3caf80d242c322549d8b4f16813723966b76e`。
- **未将启动回执计为窗口验收**：第一版因已有Settings frame在预检中止，URI **0**；随后去掉“必须新窗”的额外门槛，允许按用户当前桌面授权复用精确既有frame，记录可见性/DWM/进程创建身份，并永不关闭或终止该既有窗口/共享进程。第二版实际只派发 **1**条URI，返回`completion:completed/is_error:false/pid:0/windows:[]`，但探针随后因native映射变化停止接管，**完整Settings/DPI验收0/1**，不能把ACK当作目标页已验证。原Settings20064与AFH31972保留HANDLE在收尾均alive；两版均close_session/shutdown成功、worker_running=false、无进程终止指令；没有操作鼠标键盘、没有请求改缩放。此为独立宿主POC，不是canonical Agent Stop或真实模型新结果。
- **真实桌面阻断与隐私**：URI后UI工具能返回Settings窗口对象，但辅助功能树为空；仅为定位查看一次窗口画面时实际出现Windows安全权限弹窗，立即停止GUI操作。未允许/取消该安全请求，未修改缩放、权限、真实文件或生产数据，也未将安全弹窗或背景画面存为验收artifact/提交；本轮临时窗口/截图变量已清除。现有computer-use工具规则明确禁止代操作安全/隐私许可，已请求用户手动处理此实际弹窗；无需额外账户或VM。后续收到处理完成信息后重新选择当前窗口，不使用旧句柄/坐标、不重发本轮URI。
- **证据和下一动作**：根`%TEMP%/nomifun-settings-uri-probe-a048b64c136f45eaa786f5d5de5cbd5c`保存两版源/EXE身份；`native-1/report.json` SHA256 `336b93a936f35ed9240b121455750ffa49ca05263542472a29d23ff1d14c579e`、`native-2/report.json` `9f81976832534f5054f5f8176f5915c41800d98e3540a37aedac4f99e5bce661`、`poc-summary.json` `0eec28c9b3954c86e3a0fcb4fe912d9fa803555fc5c6fac283a797289c7feda7`分别保留失败边界、一次URI真实回执与分类。两个探针均已终止，无本轮遗留的探针或worker等待项；临时仓库源已移除。收尾只运行docscheck/diffcheck，不重建未变产品。当前签名证书仍 **0**、指纹/TSA配置仍缺，原生macOS/Linux入口及原Keyboard自启偏好信息也未补齐；这些待验不改写为通过，S5入口保留。

### Linux构建预检XTest遗漏修正（2026-10-21，S4本机增量）

- **基线与缺陷**：干净main `e9ca9b30b58194f31c507e1edcc3a7bf4bf29a66`检查status/branch/HEAD并`git pull --ff-only`，已最新。只读核对当前锁定的`x11 2.21.0`：`xtest=["xtst"]`，其build.rs按已启用feature强制`pkg-config xtst`；Linux构建脚本却只预检x11/xi/xkbcommon，缺Xtst开发包的机器会通过Computer预检后才编译失败。仅补`scripts/desktop-build-linux.sh`的xtst预检、`libxtst-dev`建议与构建说明，同步两种语言构建文档；不改vendor、lock、运行依赖或产品实现。脚本保持UTF-8无BOM/LF，**11,384B/SHA256 `cdadd4772bb3a00d516bd70cdc4842422a675c59066fb52f5d1b62f78362f150`**。
- **只读实物依赖复核**：WSL Ubuntu/rika（UID1000）对既有`/tmp/nomifun-computer-worker-s1-target/debug/nomifun-computer-worker`运行`sha256sum`、`readelf -d`、`ldd`，均exit0，SHA256仍`c784d9e9362f4e487a1507fb4c1ebea8857f0fe46e5bf29c393088769ddea2cf`。当前7项NEEDED与此前实证相同：libX11.so.6、libXi.so.6、libxkbcommon.so.0、libgcc_s.so.1、libm.so.6、libc.so.6、ld-linux-x86-64.so.2；ldd解析 **13个库文件、缺库0**。XTest是本次补齐的构建要求，不据此增加当前实物没有的libXtst运行依赖；没有重建或执行桌面动作。
- **最小验证**：`wsl.exe -d Ubuntu -u rika -- bash -n /mnt/c/Users/rika0/code/nomifun/main/nomifun-desktop/scripts/desktop-build-linux.sh`及`wsl.exe -d Ubuntu -u rika -- pkg-config --exists x11 xi xtst xkbcommon`均exit0，**4个Computer构建模块可用**。`bun --no-env-file run check:computer-boundary` **exit0、458份vendor/3补丁/22动作/6020路径**；`bun --no-env-file run check:docs` **exit0、145文件/1468引用**，`git diff --check`通过。本轮未改Rust/UI，不新增或机械重跑测试，复用前轮非零测试与Windows开发包证据；未运行会安装target、构建或改系统的完整Linux入口。
- **待验保持**：这次是构建预检修复和既有worker库解析，不是完整LinuxApp包、AppImage最终库集合、GNOME/Wayland或跨架构原生验收。当前安全弹窗仍待用户手动处理，Windows正式签名/TSA与macOS/原生Linux入口仍未补齐；不对其自动操作，也不将待验记为通过，S5专项入口保留。

### 未执行项与下一动作

1. 本机S2完整替换与S3已通过上述门槛，源码与进展文档已同步origin/main并核对身份；后续按发布范围补S4。旧Computer专测直接删除，受影响混合验证保留非Computer部分，未保留旧实现兜底。
2. macOS arm64/x64构建、自检、冻结采集、签名App TCC；Windows实际Notepad保存、Settings功能、150% DPI和受控High-IL拒绝；真实Linux X11/GNOME Wayland、portal/helper与D12规则按可得环境补验。当前Windows实证100%自有WinForms、Tauri与系统Calculator，不外推其他应用/DPI；未为验收提权或改系统缩放。缺失项不阻塞本机实施，WSL/Xvfb不得冒充原生Wayland。
3. 用户提供的Step Plan凭据已用于上述真实Full与显式NoImage验收，未读取其他任务或生产配置；接口输入约束、设置页与通用调用拒绝已贯通。Windows unsigned独立身份安装/包内worker/同步卸载及正式HTTP/native只读、WinForms后台拖拽中Stop已通过；正式签名配置仍缺，正式签名/公证、正式包矩阵、系统许可修改和发布未执行。其余平台/架构/应用种类、其他输入方式及原生故障按可得环境补证，不将后台PostMessage结果外推前台SendInput、全局按键或长期Runtime退休。CHANGELOG升级草稿与架构已备，S4完成前保留专项入口。
