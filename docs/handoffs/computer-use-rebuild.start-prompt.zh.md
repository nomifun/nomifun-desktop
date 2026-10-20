# 启动 prompt：Computer Use 一步到位重构

依据 2026-10-20 用户修订，主实现先在 Windows 本机贯通 S1/S2/S3；macOS、Linux 的编译和原生验证并行补齐。方案见 [computer-use-rebuild.zh.md](computer-use-rebuild.zh.md)，接续点始终是其第 12 节。以下平台 prompt 不要求主实现等待另一台机器、桌面或模型凭据。

这是实施授权，不是只做 review。开发、测试和例行修复直接推进；提交、推送、安装、系统权限操作和发布沿用会话已有授权，本说明不增加确认步骤。

## 主实施 prompt

```text
请在当前 nomifun-desktop checkout 持续实施“Computer Use 一步到位重构”：用 vendored Cua Driver + NomiFun 自有私有 worker 全新实现 Computer Use。旧实现整体删除；不迁移、不兼容、不读取旧数据与旧契约；依赖旧 Computer 的会话复用现有拒绝路径提示新建会话。

执行依据为 2026-10-20 用户修订：Windows 本机先贯通 S1/S2/S3，其他平台编译/原生验证并行补验。缺少 Mac、真实 Wayland、其他 DPI/架构、签名或模型凭据只记录待验，不阻塞本机接入，不因此删平台实现或降低能力。未验项不宣称已支持；S4 按本次发布平台与范围完成真实模型和安装包验收。

直接在 main 开发。先检查 git status、HEAD 和当前分支，确认 main，并 git pull --ff-only；每个阶段开始前再次同步。保护无关改动，不 stash、reset、覆盖或清理用户工作。重叠文件按实际 HEAD 合并意图。每次落到 main 的改动自洽、可编译、受影响测试通过、开发平台应用可构建打包，不出现“旧实现已删、新实现未接上”的状态。

完整阅读一次 AGENTS.md、docs/handoffs/computer-use-rebuild.zh.md（唯一施工说明）、docs/architecture/agent-session.zh.md、global-os-capabilities.zh.md、system-permissions.zh.md、docs/handoffs/agent-lifecycle-reliability.zh.md（U01–U13 同样适用）、docs/contributing/development.md。接续时核对最新进展和实际源码，不机械重做已完成的阅读/测试。按修订后的 C01–C08、D01–D14 与本机阶段条件执行；只有必须改变产品合同或缺少无法推断的关键事实时才提出具体问题，同时继续不受阻的工作。

从方案第 12 节接续：
- S1：确认 AGENTS.md 现役入口；固定上游 commit 4d959bbfa57f0c89f310330e849500b222084194（版本标签 cua-driver-rs-v0.37.0，标签解析差异按 SOURCE.json 记录），用 --filter=blob:none --sparse 按 D03 导入 vendor/cua-driver。只读参考可用 C:/Users/rika0/code/nomifun/main/resources/cua。保留 D04 去默认 features 补丁及已授权的必要最小缺陷补丁，全部登记并经 sync 应用；不再限定只能一个。完成独立 apps/computer-worker、MIT 署名、DPI manifest、self-check、真实 worker 协议测试、冻结清单和精简覆盖。Windows 构建/self-check/协议与一致性检查通过后进入 S2，其他平台和额外原生 POC 并行补验。
- S2：一次完整替换落地：删除第 7 节旧实现/旧测试/夹具/配置/引用；由冻结清单生成 22 Action 和契约；实现 nomifun-computer 五模块、computer_host、效果/Stop/分档/生命周期；UI 旧动作/路由/许可接新契约；接入 worker 构建、externalBin 与各平台适用签名脚本；上线 check:computer-boundary。资源种类为 computer，ResourceId 为 local-desktop。实际 main 的 UI/API 版本已是 42，本专项升 43，保留他人的 42 改动，实施时再复核是否已有并发递增。本机正式产品链路先用专用计算器/文本编辑窗口和临时数据跑通，有受控模型凭据即补真实模型；缺凭据不阻塞代码接入。
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
请在 macOS main checkout 并行补齐 Computer Use 重构的 macOS 构建、产品接入和发布验收。先核对 git status、HEAD、main 并 git pull --ff-only，保护无关改动，不 stash/reset/覆盖。阅读 AGENTS.md、唯一实施方案与本 prompt，从方案第 12 节接续，消费最新完整替换，不重做 Windows 已交付内容，不改 canonical 存储或引入旧 Computer 兼容。

按可用环境构建 arm64（可得时 x86_64）worker并通过 self-check/协议；检查完整开发签名 App 直接 spawn worker 的 TCC 继承、主线程光标覆盖层、写入位置和输入清理。发布 macOS 前补齐适用 Developer ID/Hardened Runtime/公证、codesign --verify --deep --strict、安装包 worker，自有测试窗口中的计算器/TextEdit 保存、多窗口、Stop、崩溃恢复与 Cmd-Q 收束；真实模型凭据可得时执行，缺失只记待验。

当前用户桌面可用于专用测试窗口和临时数据，不改真实文件、既有会话、凭据或登录浏览器内容；故障只作用于精确自有测试进程。隔离账户/虚拟机按需要选择，不是前置条件。不擅自安装、提权或重置系统隐私设置；已有许可先复用，权限操作依会话授权。只更新第 12 节平台证据与下一动作，区分编译、脚本、原生与未执行项；未验不宣称 macOS 支持，不要求 Windows 主实现等待。
```

## Linux 并行补验 prompt

```text
请在 Linux main checkout 并行补齐 Computer Use 重构的 Linux 构建、产品接入和发布验收。先核对 git status、HEAD、main 并 git pull --ff-only，保护无关改动，不 stash/reset/覆盖。阅读 AGENTS.md、唯一实施方案与本 prompt，从第 12 节接续，消费最新完整替换，不写旧数据兼容或修改 canonical 存储。

记录发行版/内核、桌面、合成器、AT-SPI 和 portal 后端。构建 x86_64 worker（portal-input；portal-capture按真实环境结论），跑 self-check/协议与受影响检查，以 readelf/ldd核对依赖。用专用窗口验证 X11/XWayland、AT-SPI 和原生清理；有真实 GNOME Wayland 时验证 portal 授权/拒绝、后台动作/前台输入及 Shell helper边界，得出固定启用规则。Xvfb/WSLg/XWayland证据不冒充原生Wayland；缺少环境只记待验并继续其他实现，不阻塞Windows。

发布Linux前补齐适用 deb/rpm/AppImage 的worker、依赖与对应原生/真实模型任务；删除旧GStreamer依赖。保留精确native清理owner，worker退出不冒充清理成功，不重放输入。当前桌面可用专用测试窗口/临时数据，故障只作用于精确自有测试实例；不动真实文件、既有会话、凭据或登录浏览器。不擅自安装系统组件、提权或改系统权限；隔离显示服务/账户/虚拟机按方便选用，不是前置。只在第12节记录实际命令、非零测试数、源码/构建身份、原生证据和未验原因，不宣称未验支持。
```
