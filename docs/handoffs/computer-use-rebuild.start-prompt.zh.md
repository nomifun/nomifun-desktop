# 启动 prompt：Computer Use 一步到位重构

依据 2026-10-20 用户修订，主实现先在 Windows 本机贯通 S1/S2/S3；macOS、Linux 的编译和原生验证并行补齐。方案见 [computer-use-rebuild.zh.md](computer-use-rebuild.zh.md)，接续点始终是其第 12 节。以下平台 prompt 不要求主实现等待另一台机器、桌面或模型凭据。

这是实施授权，不是只做 review。开发、测试和例行修复直接推进；提交、推送、安装、系统权限操作和发布沿用会话已有授权，本说明不增加确认步骤。

## 主实施 prompt

```text
请在当前 nomifun-desktop checkout 持续实施“Computer Use 一步到位重构”：用 vendored Cua Driver + NomiFun 自有私有 worker 全新实现 Computer Use。旧实现整体删除；不迁移、不兼容、不读取旧数据与旧契约；依赖旧 Computer 的会话复用现有拒绝路径提示新建会话。

执行依据为 2026-10-20 用户修订：Windows 本机先贯通 S1/S2/S3，其他平台编译/原生验证并行补验。缺少 Mac、真实 Wayland、其他 DPI/架构、签名或模型凭据只记录待验，不阻塞本机接入，不因此删平台实现或降低能力。未验项不宣称已支持；S4 按本次发布平台与范围完成真实模型和安装包验收。

直接在 main 开发。先检查 git status、HEAD 和当前分支，确认 main，并 git pull --ff-only；每个阶段开始前再次同步。保护无关改动，不 stash、reset、覆盖或清理用户工作。重叠文件按实际 HEAD 合并意图。每次落到 main 的改动自洽、可编译、受影响测试通过、开发平台应用可构建打包，不出现“旧实现已删、新实现未接上”的状态。

完整阅读一次 AGENTS.md、docs/handoffs/computer-use-rebuild.zh.md（唯一施工说明）、docs/architecture/agent-session.zh.md、global-os-capabilities.zh.md、system-permissions.zh.md、docs/architecture/agent-execution.zh.md（生命周期约束 U01–U13 同样适用）、docs/contributing/development.md。接续时核对最新进展和实际源码，不机械重做已完成的阅读/测试。按修订后的 C01–C08、D01–D14 与本机阶段条件执行；只有必须改变产品合同或缺少无法推断的关键事实时才提出具体问题，同时继续不受阻的工作。

从方案第 12 节接续：
- S1：确认 AGENTS.md 现役入口；固定上游 commit 4d959bbfa57f0c89f310330e849500b222084194（版本标签 cua-driver-rs-v0.37.0，标签解析差异按 SOURCE.json 记录），用 --filter=blob:none --sparse 按 D03 导入 vendor/cua-driver。只读参考可用 C:/Users/rika0/code/nomifun/main/resources/cua。保留 D04 去默认 features 补丁及已授权的必要最小缺陷补丁，全部登记并经 sync 应用；不再限定只能一个。完成独立 apps/computer-worker、MIT 署名、DPI manifest、self-check、真实 worker 协议测试、冻结清单和精简覆盖。Windows 构建/self-check/协议与一致性检查通过后进入 S2，其他平台和额外原生 POC 并行补验。
- S2：一次完整替换落地：删除第 7 节旧实现/旧测试/夹具/配置/引用；由冻结清单生成 22 Action 和契约；实现 nomifun-computer 五模块、computer_host、效果/Stop/分档/生命周期；UI 旧动作/路由/许可接新契约；接入 worker 构建、externalBin 与各平台适用签名脚本；上线 check:computer-boundary。资源种类为 computer，ResourceId 为 local-desktop。当前交付的 UI/API 版本为 45；接续时以实际 HEAD 为准，保留并发递增，不重复执行历史的 42→43，也不回退版本。本机正式产品链路先用专用计算器/文本编辑窗口和临时数据跑通，有受控模型凭据即补真实模型；缺凭据不阻塞代码接入。
- S3：补齐驱动状态卡、GET /api/computer/status、能力工作台分组、“前台”标记、中英文文案与文档。核实旧会话实际拒绝错误码命中新建会话文案，不为旧会话/升级路径写测试。可与本机 S2 收尾并行。
- S4：按拟发布平台/架构补齐适用签名、公证、安装包、依赖、原生和真实模型验收；不把固定厂商数量或全平台排列组合变成日常开发门槛，未验平台只记待验，不冒充已支持。
- S5：发布说明（含升级影响）、CHANGELOG、版本与架构定稿，按既有发布授权交付并退役本专项入口。

保持质量与成本边界：
- 旧测试随旧实现直接删除，不改写适配；不写迁移、映射、对账、旧数据清理或旧数据/旧会话/升级夹具。
- 不新增功能开关、旧实现兜底、事件通道或 canonical 存储变更。AgentSession/Turn/Event/Payload/Effect 保持唯一事实链。
- 动作、schema、标题与提示词只从冻结清单/覆盖文件派生；vendored 变更只能通过登记补丁。模块单一职责，不堆未使用抽象。
- worker 退出不等于原生清理完成。保留必要 native guardian 的精确清理责任，待定状态沿现役清理机制表达；不把它作为普通 Runtime/缓存，也不重放输入。
- 假 worker 覆盖慢请求、崩溃、半包、超时等宿主逻辑；真实 worker 协议测试不做桌面动作；原生场景用受控测试窗口和精确测试实例。

按改动选择最小验证集合：cargo check -p nomifun-app --features computer-use；cargo test -p nomifun-computer；cargo test -p nomifun-agent-contracts；cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check；bun run gate:agent-v2；bun run check:computer-boundary；涉及 UI 时 bun run check:desktop-ui-boundary。worker 在 apps/computer-worker 内独立 build/test/self-check。跨切面 S2 完成与正式发布前跑 bun run check 全量和受影响 Rust/UI 测试；其他阶段不重复无关矩阵。

当前真实用户桌面允许用于本次专用测试窗口、测试会话和临时目录；输入只投递到这些窗口，不修改真实文件、生产数据、凭据或已登录浏览器内容，不提交隐私截图。故障用例优先精确自有 worker/测试进程；可用隔离账户、私有显示服务或虚拟机，但它们不是开发前置。不得擅自安装系统组件、提权或修改系统权限。真实模型用现有受控配置或用户提供的测试凭据，不在日志或文档暴露凭据。

持续实现、测试、删除旧路径与更新文档，不反复询问是否继续。只在方案第 12 节记录执行进展：源码/构建身份、实际命令、非零测试数、原生证据、待验原因与下一动作；自动化、真实原生、未执行项分别记录。规则文档按已确定的用户修订维护，不能把缺环境改写成已验通过。
```

## macOS 并行补验 prompt

```text
请从包含本文件的最新 main 接续 Computer Use 的 macOS 剩余实现与 S4。先查 git status、HEAD、分支并 git pull --ff-only；每阶段同步，保护并发工作，不 stash/reset/覆盖/清理。当前 UI/API45，以实际 HEAD 为准，不回退版本；Linux 已验源码基线 9b8706f71 只作证据身份，不要求 checkout 回旧提交。

阅读 AGENTS.md、唯一方案 docs/handoffs/computer-use-rebuild.zh.md（尤其§8/9/12）、docs/architecture/computer-use.zh.md 及其会话/许可架构引用、docs/architecture/agent-execution.zh.md（生命周期约束 U01–U13）。从最新§12接续已定 C01–C08/D01–D14，不恢复旧 Computer 或旧数据兼容、不改 canonical 存储。vendor 仅通过登记补丁修改，动作/schema/标题仍由冻结输入生成。

Windows S1–S3及 Full/显式 NoImage 共享模型链路已有证据；Linux x64 release 三包、40项 Rust 测试、离线资源/ELF核验和3份 release lock 已完成。复用对应源码的有效证据，不机械重跑无关矩阵；这些不证明 macOS 原生、安装或正式身份通过。

复用现役 build:mac、worker 和原生验收脚本，先完成 arm64，Intel按可得环境补齐。实际构建 worker，运行 self-check、真实协议和冻结采集；检查 App 直接启动 worker、主线程光标覆盖层、私有写入位置及输入清理。正式发布范围须补 Developer ID/Hardened Runtime、嵌套 worker 签名、公证和包内实物校验；开发签名不冒充正式签名。验证 TCC 责任归属、只向 NomiFun.app 授权、缺权零执行和许可变化后的新代际。

当前用户桌面可用于专用测试窗口、新 Session 和临时文件，VM/测试账户均非前置。按方案验证 Calculator、TextEdit 保存、多窗口、实际设置功能、Stop、适用故障恢复及 Cmd-Q 回收；观察设置不算功能通过。保留精确目标/worker代际和真实清理证据，不重放未知输入；故障只触及精确自有实例，被拒动作不换路重试。可复用既有进程，仅操作能精确确认的本次测试窗口/标签，不以新PID为前置；不读取或改动其他用户内容、生产数据、凭据或已登录浏览器，不提交隐私截图，不擅自安装系统组件、提权或重置隐私许可；权限交互沿已有授权。

真实模型只用获授权的受控配置，记录实际模型、成功率、步数和耗时；脚本 provider 不算实模，共享模型证据不冒充 Mac 原生任务。缺机器、配置或凭据只记待验，继续其余工作。按改动范围运行最小验证，正式交付按拟发布范围补门槛，不自行发布。只在方案§12记录源码/产物/签名身份、命令、非零测试数、原生证据及未执行原因；自动化、实模、原生分别记证，不把缺失项写成通过，S5按实际完成状态处理。
```

## Linux 并行补验 prompt

```text
请从包含本文件的最新 main 接续 Computer Use 的 Linux 原生产品与 S4。先查 git status、HEAD、分支并 git pull --ff-only；每阶段同步，保护并发工作，不 stash/reset/覆盖/清理。当前 UI/API45，以实际 HEAD 为准，不回退到历史源码或版本。

阅读 AGENTS.md、唯一方案 docs/handoffs/computer-use-rebuild.zh.md（尤其§8/9/12）、docs/architecture/computer-use.zh.md 及其会话/许可架构引用、docs/architecture/agent-execution.zh.md（生命周期约束 U01–U13）。遵循已定 C01–C08/D01–D14，不恢复旧 Computer/数据兼容、不改 canonical；vendor 只用登记补丁，动作/schema/标题由冻结输入派生。

§12已闭合源码 9b8706f71 的 Linux x64 release worker、协议12/12和宿主28/28共40项 Rust 测试、deb/rpm/AppImage 构建、真实离线提取、依赖/资源/ELF核验及3/3 release lock。优先取得实物并核对既有SHA后复用；无输入变化不重编三包或重跑40项。AppImage有已验证的ELF改写，按各包自身身份验收，不要求三包 Host/worker 原始SHA相等。离线包与锁不证明安装、分发签名或真实桌面通过。

记录实际发行版、内核、架构、登录桌面、合成器、AT-SPI 和 portal 后端。当前 CUA_DRIVER_RS_ENABLE_WAYLAND=0，portal-capture未启用；按D12在真实环境验证后确定固定启用规则，不新增开关，不因显示环境变量存在便宣称可用。用专用窗口、新 Session、临时文件验证真实X11/AT-SPI计算器、文本编辑保存、后台动作、适用前台输入、Stop及退出回收；有真实GNOME Wayland再补portal授予/拒绝和helper边界。Xvfb/WSLg/XWayland不冒充原生Wayland，观察设置不算设置功能验收。

当前用户桌面已允许专用测试，VM/测试账户/私有显示服务只是可选工具。可复用既有进程，仅操作能精确确认的本次测试窗口/标签，不以新PID为前置；不读取或改动其他用户内容、生产数据、凭据或已登录浏览器，不提交隐私截图；安装与权限操作沿已有授权，不擅自安装系统组件、提权或改系统许可。保留精确原生owner、worker代际和正常关闭证据，worker退出不等于全部清理完成，不重放未知输入。故障仅作用于精确自有实例，被拒动作不换路重试。

Windows Full/显式NoImage共享链路已有实模证据，可复用但不冒充Linux原生任务；脚本provider不算实模。平台实模仅用获授权配置，记录实际模型/成功率/步数/耗时，缺配置不阻塞其余工作。只为真实架构、源码或平台缺陷补构建和最小验证，不另造重复框架；安装包、原生、正式身份按拟发布范围分别验收，不自行发布。只在方案§12记录源码/产物身份、实际命令、非零测试数、原生证据及待验原因，自动化/实模/原生分开，不宣称未验支持，S5按实际完成状态处理。
```
