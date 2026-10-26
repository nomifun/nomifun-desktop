# Windows 语音键盘升级与恢复验证

## 范围

2026-10-26 在 Windows x64 上验证安装器携带的独立维护入口、跨旧版命令边界的安装、维护期间的进程门禁、文件恢复和应用内更新暂停。没有改变键盘功能开关、登录偏好、Agent Session、数据库格式或已有用户数据。

旧版行为通过 `v0.8.14` 源码核对，以及不识别 `--keyboard-stop`、将未知参数当作普通启动的原生旧程序夹具验证。完整安装包使用真实 Tauri NSIS 模板、编译器、安装/卸载过程和唯一产品登记；其中的应用程序是无硬件原生夹具。这些证据不是对已发布历史 Tauri 安装包逐版运行的证明。

所有运行使用独立 LOCALAPPDATA、APPDATA、数据目录和测试进程。设备服务夹具使用真实文件锁和鉴权 IPC，但不扫描串口、不打开系统麦克风、不读取模型凭据。完整包使用唯一 product/id/publisher/protocol，并核对测试登记已清除、真实 `NomiFunKeyboard` 登录项指纹未改变。

## 实现边界

- 新包维护工具从私有临时目录运行，旧 exe 不接收新版本才支持的停服命令。
- 共享启动门禁和 GUI/设备生命锁贯穿停服、文件替换、提交或回滚。GUI 在存储和窗口初始化前检查门禁；设备服务在发布端点前检查门禁。
- payload 在停服前解压，含新卸载器；只备份、替换 payload 对应的应用文件，不复制或删除用户数据。升级不先运行旧卸载器。
- 文件替换失败、取消或安装器父进程退出时，维护进程核对回滚后恢复原运行意图。新版本登记、快捷方式和旧二进制清理只在提交后发布。
- 应用内更新的暂停可撤销，取消不会恢复旧按键或自动开麦；失败和命令取消后保留已验证安装包。手动安装的外部停采失败也使用相同的可恢复暂停。
- `/UPDATE` 保留已退出 GUI 的恢复意图；维护工具尚未启动时取消也会恢复原程序一次。成功的 GUI 重启继续交给 NSIS `/R`，保留原始启动参数。
- 正常退出保留 K1，更新临时停服，完整卸载只移除目标安装拥有的登录项。另一个安装实例持有设备时明确拒绝停错进程。

## 自动化与原生结果

| 验证 | 结果 | 实际覆盖 |
| --- | --- | --- |
| `cargo test -p nomifun-keyboard --all-targets --features maintenance-validation --locked` | 87/87 | 键盘 reducer、协议、GUI/设备停服、维护门禁、文件备份与回滚；Windows 不执行 Linux PTY 用例 |
| 维护程序命令领取竞态测试 | 1/1 | apply 被领取后发布 abort，不会误删新取消请求 |
| 实际桌面测试二进制定向测试 | 9/9 | 更新暂停/恢复与并发顺序 6 项、预检取消保留安装包/交接 2 项、流式队列满时控制恢复 1 项 |
| `bun test scripts/keyboard-packaging.test.mjs` | 3/3，另 8 项 POSIX 用例按平台跳过 | 真实 NSIS 编译、包配置、维护程序构建目标选择 |
| `bun test scripts/lib/macos-app-bundle.test.mjs` | 8/8，另 13 项按平台跳过 | 构建路由与静态资源约束；不是 macOS 原生验收 |
| 更新状态派生与更新窗口交互 | 20/20 | 已保留安装包恢复安装入口、过期事件隔离、安装拒绝和重试界面 |
| Windows 原生维护和真实 NSIS hooks | 18/18 | 下述维护/恢复矩阵 |
| 完整生产 Tauri NSIS 模板安装包 | 3/3 | 旧程序关闭时手动升级、旧程序运行中 `/UPDATE /R` 升级、实际部分写入失败恢复 |
| Windows 原生 Actor：流式/普通识别 | 2/2 模式 | 每种模式 4 类恢复场景、25 轮取消、27 段新录音；输入和 provider 为隔离 fixture |
| 文档、Windows 安装器合同、桌面 UI 边界 | 通过 | 本地链接、模板完整性和维护顺序、880×600 桌面边界 |

原生维护矩阵覆盖：没有服务的旧版、旧版运行后取消恢复、GUI/设备成功重启、语音关闭但后台服务运行、替换前后取消、意外维护命令、更新器重启交接、真实 Windows sharing-denied 中途失败、安装器父进程退出、失效端点、真实停采拒绝、另一个安装实例保护、实际 NSIS 成功/取消/失败，以及维护工具启动前取消和重复恢复回调。

完整包失败场景锁住新的卸载器目标，使主程序已经替换后再发生写入失败。验证原主程序、原卸载器逐字节恢复，旧 GUI 重新运行，设置摘要不变，`DisplayVersion` 仍为 `0.8.14`。成功场景继续启动新的 GUI/服务，再执行生成的卸载器，核对测试进程和专属产品登记均退出/移除。

原生 Actor 的两种模式分别覆盖：暂停确认后取消、原生停采仍在等待时取消调用、应用内停采超时、外部维护停采超时。每项都核对原生资源停止证明仍受约束、旧按住意图没有自动恢复、设置保持原值，随后一次新 K4 可以录音。另测语音关闭及启用但未配置模型时仍可进入维护，以及成功交接后旧 Actor 保持暂停。

两次 Actor 主进程都以 0 退出；各自 Job 记录 11 个所属进程。私有 WebView 子进程未全部自行退出，启动器使用该 Job 强制清理后剩余进程数为 0（`cleanupForced=true`）。这是测试清理证据，不能写成完整进程树优雅退出，也不等同于生产主程序的完整后端退出验收。

## 冻结产物与记录

生产 feature 集维护程序 SHA-256：

`22bef4eecb50e4e0de6bb239ad625e17367a1bd05226dd53e533d3af970aca3c`

NSIS hooks SHA-256：

`e10fc66df5331e2eb872fd36f5947cde7b7c2e0097d4301c252a3dbb125670c2`

安装器模板 SHA-256：

`b95b2224ca916c8301120217cb3105ce49c10eaa568df54647961eb1c17fb9a9`

最终运行期间源码和二进制身份保持一致。原始记录保留在本机临时目录，未作为产品资源提交：

- `C:\Users\muli\AppData\Local\Temp\nomifun-keyboard-upgrade-ZzD4j5\report.json`；SHA-256 `61a4e5631c70fa29ce875bee5af72baa6c31dac79a8b1726eddf993c3a6a2443`。
- `C:\Users\muli\AppData\Local\Temp\nomifun-keyboard-package-upgrade-tzBAak\report.json`；SHA-256 `724e655c347597ef173f6aac231219ec64f6d618ae8212c88ee7e51f64073441`。
- `C:\Users\muli\AppData\Local\Temp\nomifun-keyboard-actor-streaming-9bfc428fc7b94051b54396d31ebbe28e.json`；SHA-256 `0e0b8a9d800841896f8b0b1cf36f743cbd351a2526eb7fdb09428b28b7ce7d77`。
- `C:\Users\muli\AppData\Local\Temp\nomifun-keyboard-actor-platform-8a75276270d044fc9eda8da90f5d73df.json`；SHA-256 `859d59d37741f7fd6497bf4ba19cb0f72562db373435b6e8b157b8053e8b4943`。

两次 Actor 使用相同原生二进制 SHA-256 `5050b414e503237e4d19069cdce38646267a81465238924c9219287f6c61339d`，嵌入实际当前前端 `0b4fbd3f-e177-4639-a836-f1a8f2d7835b`（应用 `0.8.15`、实际 HEAD 合同 `46`）。桌面定向单元测试二进制 SHA-256 `a0e8e9b5d18753db198b1819dcbdd90afd635b16ba738aa3cc5dd99b35250d34`。

首次原生验证发现：设备已经返回停服回执、生命锁尚未释放时，维护循环重复发送请求会造成连接重置。已改为只请求一次、随后等待真实生命锁释放，并增加延迟退出回归。首次失败记录保留，未改写为通过。

桌面原生验证程序首次链接遇到既有 ORT 离线环境问题：继承的 `CARGO_NET_OFFLINE=true` 使 ort-sys 在查询已有 ONNX 分发缓存之前生成链接错误占位符。只在后续构建进程移除该变量，复用实际静态库；未改全局环境、依赖或生产代码，也未跳过真实链接。原生程序随后实际链接成功。该环境原因和处理与 [既有 Windows 构建记录](global-os-windows-2026-10-17.zh.md) 一致。

可重复运行入口：

```powershell
# 先构建无硬件 fixture，再以生产 feature 集构建维护程序。
cargo build -p nomifun-keyboard --example maintenance_fixture --features maintenance-validation --locked
bun scripts/build-keyboard-maintenance.mjs --profile debug
node scripts/validation/keyboard-upgrade-native-smoke.mjs --helper <维护程序绝对路径> --fixture <maintenance_fixture绝对路径>
node scripts/validation/keyboard-upgrade-package-smoke.mjs --helper <维护程序绝对路径> --fixture <maintenance_fixture绝对路径>
```

本次独立 Cargo 产物后来移至仓库忽略目录 `.tmp-keyboard-upgrade/`，没有移动原 `.tmp` 中的其他任务文件。

## 尚未覆盖

- 实体四键、实际 OS 麦克风、真实供应商 ASR 和每个历史发行安装包；合成输入、loopback provider 和原生设备服务证据需分别理解。
- 正式签名安装包发布；本次修改尚未发布。
- macOS/Linux 的编译和原生升级。共享暂停逻辑的 Windows 证据不证明其他平台；Linux 包脚本仍需单独处理“旧 exe 不支持新停服命令”的安装引导边界。
- 直接强杀维护进程或断电导致其无法执行回滚；需要重新运行安装包修复。普通取消、可捕获 I/O 失败和安装器父进程退出已有恢复验证。
- OS 拒绝启动安装器，以及原生停采成功后其他后端清理一直阻止 GUI 退出；不能确认资源释放时继续禁止覆盖，不声称已恢复一个尚未完整退出的应用。
