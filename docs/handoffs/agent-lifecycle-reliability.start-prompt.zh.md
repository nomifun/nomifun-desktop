# 平台接续启动 prompt：多 Agent 生命周期可靠性

Windows 代码实施和当前可执行验收已经完成；唯一进展与证据保存在[方案第 10 节](agent-lifecycle-reliability.zh.md#10-单份执行进展)。以下两段分别复制到对应平台的仓库任务中。使用该平台实际 checkout，不套用原 Windows 绝对路径。基线提交由交付会话提供；若远端有后续改动，以实际 HEAD、原合同和新增差异复核。

这是平台实施与验收授权，不是另写方案或只做 review。当前会话对 commit/push 的授权仅用于本次 Windows 交付，后续任务仍不得自行提交、推送、安装、提权或发布。

## macOS 启动 prompt

```text
请在当前 macOS checkout 接续“多 Agent 生命周期可靠性与用户控制”专项，完成 macOS 编译、适用回归、原生验收和必要修复。

先检查 HEAD、分支和 git status，保护无关改动；可安全快进时 git pull --ff-only，并确认包含本次交付会话提供的基线提交。不得自动 stash、重置、覆盖或清理用户数据。

完整读取 AGENTS.md、docs/handoffs/agent-lifecycle-reliability.zh.md、本文、现役 Agent Session／Execution／Browser／全局 OS 架构和 docs/validation/browser-platform.zh.md。U01–U13、行为合同、T01–T37 和 OS01–OS10 均是约束。从唯一进展第10节未完成处接续；Windows已有证据，不重做已完成实施，不把它当作macOS通过。

先做最小P0复核，核对实际features、原owner、主线程退出链及测试实际数量，然后使用现役入口：

cargo build --locked --profile test -p nomifun-desktop --bin nomifun-desktop --features nomifun-app/browser-conformance
cargo test --locked -p nomi-process-runtime --test parent_death --test pty_contract --test process_contract --test darwin_registered --test darwin_application
cargo test --locked -p nomifun-app --features browser-conformance --test native_execution_recovery --test desktop_agent_shutdown

选择受影响的Session／Execution／App／Cron／Browser回归并修复具体失败。复用macos_appkit_main_loop_smoke、macos_appkit_termination_probe、macos_browser_compatibility_smoke、macos_application_controller及现役WK验收脚本；先读参数、固定descriptor和启动协议。package-macos-native-fixture.mjs只打包签名，不证明原生运行；使用原controller、LaunchServices和真实退出回执，不以普通cargo run、构建成功或零用例替代验收。

补适用OS01–OS10：pipe／PTY、leader先退出、watchdog／父死／reaping、WK导航／modal／popup／chooser中的Stop和退出、AppKit／Cmd-Q／信号、AX／TCC排队与取消、Attached Chrome跨target／断连及原页面保护、FD／进程稳态、Desktop／桌面WebUI控制。真实隔离Chrome用显式NOMIFUN_CHROME_BINARY和专用Profile。睡眠、锁屏、权限撤销及管理员设施仅在已授权独立受控环境执行；需人工操作／权限或缺环境时记录具体阻塞，同时继续独立验证。

保持能力和用户并发，不新增broker、第二账本、TTL或工作门槛。缓存不得启动／恢复／重试任务；单Agent Stop不聚合误杀，Stop持久围栏／原owner信号不被业务FIFO阻塞。原Turn继续、预算、回写和合法后台进程必须保留。使命完成与真正Quit分别取得资源退出证明，保护用户Chrome、共享WebContent、服务和持久数据，不按进程名或缓存PID／PGID扫杀。

只用隔离DB／Profile／专用进程，持续完成实现、测试、回归修复及受影响现役文档更新。只更新同一方案第10节，记录源码／构建身份、命令、非零用例数、原生回执、能力保持证据、未运行原因及下一动作。涉及UI运行bun run check:desktop-ui-boundary；仅880×600及以上，不加移动布局。定期简短报告，不反复询问是否继续，不自动commit/push、安装、提权或发布。实际适用验收完成后才报告macOS结果，未验项逐项保留。
```

## Linux 启动 prompt

```text
请在当前Linux checkout接续“多 Agent 生命周期可靠性与用户控制”专项，直接完成Linux编译、适用回归、原生验收及必要修复。

先检查git status、HEAD和分支，保护无关改动；可安全快进时git pull --ff-only，确认包含本次交付会话提供的基线提交。不得自动stash、重置或覆盖用户改动。完整读取AGENTS.md、docs/handoffs/agent-lifecycle-reliability.zh.md、本文、现役Session／Execution／Browser／全局OS架构及Linux验证文档。遵守U01–U13、行为合同、T01–T37和OS01–OS10，从第10节未完成处接续；Windows已验，不重做已完成实施或另写方案。

先做最小P0复核，记录OS／架构、Rust／Bun、普通身份、X11／Wayland会话、实际provider和权限。按现役features原生编译：

cargo build --locked -p nomifun-desktop --bin nomifun-desktop --features nomifun-app/browser-conformance
cargo test --locked -p nomi-process-runtime --test parent_death --test pty_contract --test process_contract

选择适用Session／Execution／App／Cron与资源回归，核对cfg、features和非零测试数量。Computer按docs/handoffs/computer-use-rebuild.zh.md第12节及独立apps/computer-worker工作区接续，使用固定vendor与当前协议，旧原生脚本和观察授权合同已退役。现役global-os-linux-native.mjs仅覆盖files-process／system／missing-default，先读当前参数、隔离数据与owner JWT认证协议。需要桌面的其他原生验收单独核验其现役入口与真实X11／Wayland环境，私有显示或XWayland不等于Wayland证明。不要默认调用会准备系统依赖或产出安装包的build:linux。

补验pipe／PTY、leader先退出、watchdog／父死／reaping、精确Stop、共享worker／client隔离、SIGINT／SIGTERM／Quit、FD／进程／PTY稳定性及Desktop／桌面WebUI控制。X11与真实Wayland portal授权、拒绝、撤销分别记录；WSLg／XWayland、私有D-Bus或mock不替代真实Wayland。涉及不同GNOME／KDE后端时分别取证。核对当前Desktop无managed／Attached Browser provider时列源码依据及N/A，不为本专项新增后端。

保持能力、用户并发、原owner和合法后台写者；不新增broker、第二账本、机器配额、TTL、强制worktree或声明门槛。缓存不启动／恢复／重试任务；原Turn继续，用户Stop不受业务FIFO阻塞，局部停止不聚合误杀。完成资源自动退休，真正Quit／父死关闭应用自有资源，保护用户浏览器、共享服务和持久数据。仅凭原owner／generation／退出证明清理，不按进程名或缓存PID／PGID扫杀。

只用隔离DB／Profile和受控进程。自主修具体失败，持续代码、测试、旧路径清理与现役文档更新；定期简报，只更新同一方案第10节，记录实际源码／构建、命令／测试数、原生证据、未运行原因及下一动作。模拟、真实native和未执行项分开。缺桌面、portal或依赖如实列明，同时继续独立验证；不得自行安装、提权、commit/push或发布。UI只支持880×600及以上；UI变化运行desktop边界检查。实际适用验收完成后才报告Linux结果。
```
