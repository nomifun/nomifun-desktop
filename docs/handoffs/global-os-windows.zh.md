# Windows 接续任务：默认可信的全局操作系统能力

将本文件全文作为新会话的任务要求执行。先识别当前 Windows checkout 和实际环境，核实源码后直接实现并持续完成原生验收；不要停在方案报告，也不要再次等待产品方案确认。本交接的完成只表示要求和已知缺口已交付，不表示 Windows 功能已经完成。

## 产品合同

最高原则：**最大权限赋予，最可靠实现，不要搞骚操作。** NomiFun 是用户完全信任的个人 Agent。目标是文件和卷、程序与进程、应用安装、系统配置、服务和后台任务、桌面与跨应用操作、网络和已经提供的设备能力。npm 是一条依赖回归，不能代替操作系统能力。

普通操作使用真实登录用户；需要系统身份、交互桌面或独立许可时，由应用内部选择可靠的 Windows 原生执行路径。无需用户切换权限模式，不让整个 GUI 长期以 Administrator/LocalSystem 运行。不增加权限分级、可信开关、策略中心、命令白名单、授权数据库或第二套 Agent 后端。允许建立充分可靠的 helper/runner/service，生命周期依系统需要确定，复用现役任务、结果和取消链。

workspace 只承担默认目录、项目归属、watcher 和 VCS 语义；不能限制本机绝对路径、合法 `..`、链接、其他盘符、UNC 和进程 cwd。不得用把所有盘符加入允许列表模拟放开。项目外文件不能伪造为项目内产物。保留 OS ACL、UAC、UIPI、安全桌面和账号认证；不关闭 UAC 或其他系统保护，不向模型或日志暴露凭据。

用户明确的“只读”“不要执行”“停止”始终有效。Session/Turn/Event/Payload/Effect 是唯一事实链；保留登录身份、owner、冻结 binding、效果结算、取消、native checkpoint 和清理证明。未知执行结果不能自动重放，不能新增授权或重试账本。

## 先取得完整源码并确认环境

仓库名称为 `nomifun-desktop`，使用目标电脑的实际路径。优先通过 Git 取得包含本文件及本轮完整实现的 `main` 提交，先核对实际 HEAD、工作树与源码，不要重复应用快照。提交前验收及 `source-review` 的历史基线是 `5ffe448e2fe663f85a25ab09a44da6f280509626`；远端重写后的内容等价提交为 `0e54f6905d907951fd8c956de2a17946889e40d1`，两者 tree 均为 `6c82a96b37a2804888c6099f390226cdf86db84a`。仅 checkout 任一基线均不包含本轮改造。Git 交付以实际提交/推送回执和当前仓库为准，历史快照不随提交而改写。

开始前执行并保留结果：

```powershell
Get-Location
git status --short
git branch --show-current
git rev-parse HEAD
git diff --stat HEAD
git diff --name-status HEAD
git ls-files --others --exclude-standard
rustc -vV
rustup target list --installed
bun --version
```

本轮由根任务在仓库外导出完整 `source-review` bundle，源机位置为 `/tmp/nomifun-global-os-20261016/source-review`；将这个完整目录复制到 Windows，建议放在干净目标 checkout 的相邻目录。它包含六个文件：`BASE`（两行 `branch=main`、`head=<完整 SHA>`）、`tracked.patch`、`untracked.tar.gz`、`untracked-files.txt`（每行一个仓库相对文件路径）、`SOURCE-SHA256SUMS`（全部非删除 tracked 改动及必要新增源码的逐文件 SHA-256）和 `BUNDLE-SHA256SUMS`（另外五个 bundle 文件的 SHA-256）。归档只含本轮必要新增 regular files，不含 symlink、凭据、环境文件、数据库、聊天、下载缓存、target 或大体积验收产物。**普通 diff 不包含 untracked，新 checkout 只应用 patch 会缺源码。** 最终包与源码摘要由根任务在冻结导出后提供；不要猜测摘要，也不要混用不同导出批次的 patch、tar 或 manifest。

下面的六文件恢复仅供重建提交前的验收快照；其中 BASE 和摘要固定为原始历史基线。新 Git checkout 若没有该旧对象，应直接取得已交付的完整提交；不要修改 BASE、跳过摘要，或将旧 patch 强套到不同提交。已有完整 Git 交付的 checkout 跳过此恢复段，继续阅读实现与平台验收要求。

以下恢复命令针对一个**干净、无其他写入者、HEAD 精确等于 BASE 的 checkout**，在其根目录以 PowerShell 执行。需要已有 `git` 和 Windows 自带 `tar`；bundle 放在 `..\source-review`。若当前 checkout 有无关改动，保留它并另建干净 checkout，不能 reset/clean。任一步失败立即停止并保留现场。恢复会先核对包摘要、路径清单和 regular-file 类型，将 tar 解到新的临时 staging，再检查目标冲突后复制全部必要新增文件；不会覆盖既有 untracked 文件。

```powershell
$ErrorActionPreference = 'Stop'
$reviewBundle = (Resolve-Path -LiteralPath '..\source-review').Path
$checkoutRoot = (Get-Location).Path
function Assert-RelativeSourcePath([string]$path) {
  if (!$path -or $path -match '[\\:\x00-\x1f]' -or $path.StartsWith('/') -or
      @($path.Split('/') | Where-Object { $_ -in @('', '.', '..') }).Count) { throw "Unsafe relative path: $path" }
}
function Test-Sha256Manifest([string]$root, [string]$manifest) {
  $manifestRows = 0
  foreach ($line in Get-Content -LiteralPath $manifest) {
    if ($line -notmatch '^([0-9a-fA-F]{64}) [ *](.+)$') { throw 'Invalid SHA-256 manifest' }
    $expectedHash = $Matches[1]; $sourcePath = $Matches[2]; Assert-RelativeSourcePath $sourcePath
    $sourceFile = Join-Path $root $sourcePath
    $item = Get-Item -Force -LiteralPath $sourceFile
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Not a regular file: $sourcePath" }
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $sourceFile).Hash -ine $expectedHash) { throw "Hash mismatch: $sourcePath" }
    $manifestRows++
  }
  if (!$manifestRows) { throw 'Empty SHA-256 manifest' }
}
function Assert-NoExistingSourceOrLinkedParent([string]$sourcePath) {
  $targetFile = Join-Path $checkoutRoot $sourcePath
  for ($candidate = $targetFile; $candidate -ne $checkoutRoot; $candidate = Split-Path -Parent $candidate) {
    if (!$candidate) { throw 'Path escaped checkout' }
    try { $existing = Get-Item -Force -LiteralPath $candidate -ErrorAction Stop }
    catch { if ($_.CategoryInfo.Category -ne 'ObjectNotFound') { throw }; $existing = $null }
    if ($null -ne $existing) {
      if ($candidate -eq $targetFile) { throw "Existing target: $sourcePath" }
      if ($existing.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Linked/reparse parent: $candidate" }
    }
  }
}
if ((Get-Item -Force -LiteralPath $checkoutRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Use the actual checkout path rather than a junction' }
Test-Sha256Manifest $reviewBundle (Join-Path $reviewBundle 'BUNDLE-SHA256SUMS')
$baseline = ConvertFrom-StringData (Get-Content -Raw -LiteralPath (Join-Path $reviewBundle 'BASE'))
if ($baseline.branch -ne 'main' -or $baseline.head -ne '5ffe448e2fe663f85a25ab09a44da6f280509626') { throw 'Unexpected BASE' }
if ((git rev-parse HEAD).Trim() -ne $baseline.head -or @(git status --porcelain).Count) { throw 'Use a clean checkout at the exact BASE' }
$newFiles = @(Get-Content -LiteralPath (Join-Path $reviewBundle 'untracked-files.txt'))
foreach ($path in $newFiles) { Assert-RelativeSourcePath $path; Assert-NoExistingSourceOrLinkedParent $path }
if (@($newFiles | Sort-Object -Unique).Count -ne $newFiles.Count) { throw 'Duplicate manifest paths' }
$archive = Join-Path $reviewBundle 'untracked.tar.gz'
$tarFiles = @(& tar -tzf $archive); if ($LASTEXITCODE) { throw 'Cannot list archive' }
$tarDetails = @(& tar -tvzf $archive); if ($LASTEXITCODE) { throw 'Cannot inspect archive' }
if ($tarFiles.Count -ne $newFiles.Count -or @(Compare-Object ($tarFiles | Sort-Object) ($newFiles | Sort-Object)).Count) { throw 'Archive/list mismatch' }
if ($tarDetails.Count -ne $newFiles.Count -or @($tarDetails | Where-Object { !($_.StartsWith('-')) }).Count) { throw 'Archive must contain only regular files' }
$restoreStage = Join-Path ([IO.Path]::GetTempPath()) ('nomifun-source-restore-' + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $restoreStage | Out-Null
& tar -xzf $archive -C $restoreStage --no-same-owner; if ($LASTEXITCODE) { throw 'Archive extraction failed' }
git apply --check -- (Join-Path $reviewBundle 'tracked.patch'); if ($LASTEXITCODE) { throw 'Patch check failed' }
git apply -- (Join-Path $reviewBundle 'tracked.patch'); if ($LASTEXITCODE) { throw 'Patch application failed' }
foreach ($path in $newFiles) {
  Assert-NoExistingSourceOrLinkedParent $path
  $targetFile = Join-Path $checkoutRoot $path
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $targetFile) | Out-Null
  Copy-Item -LiteralPath (Join-Path $restoreStage $path) -Destination $targetFile
}
Test-Sha256Manifest $checkoutRoot (Join-Path $reviewBundle 'SOURCE-SHA256SUMS')
git diff --check; if ($LASTEXITCODE) { throw 'Restored source has whitespace errors' }
git status --short
```

成功核对前不要删除 staging 或源 bundle；成功后保留包、清单与恢复结果供审查。包摘要只证明传输/恢复完整性，最终构建与原生验收仍在本平台执行。

本轮必要新增源文件至少包括 `crates/backend/nomifun-runtime/src/{environment,node_distribution,node_acceptance}.rs`、`crates/backend/nomifun-runtime/node_distribution_contract.rs`、`crates/backend/nomifun-runtime/notices/npm-10.9.4-LICENSE.txt`、`crates/shared/nomi-system-execution/`、`crates/shared/nomi-native-files/`、`crates/backend/nomifun-engine-core/src/system_process.rs`、`crates/backend/nomifun-app/src/router/workspace_file_read.rs`、`apps/desktop/macos-system-execution/`、`scripts/validation/global-os-native.mjs`、`scripts/validation/global-os-system-cases.mjs`、`scripts/validation/readonly-foreign-exit.mjs` 和相应构建资产、测试、当前文档。清单以实际工作树为准，不能只复制这里列举的一部分；最终 readonly FFI 修复和文档加入后须重新冻结导出 bundle，并以新包实际摘要核对恢复，不沿用旧导出摘要。

完整阅读 `AGENTS.md`、`docs/architecture/agent-session.zh.md`、[当前全局能力架构](../architecture/global-os-capabilities.zh.md)和[本轮验证与三平台矩阵](../validation/global-os-2026-10-16.zh.md)，并核对 `git diff HEAD`。后续以当前源码和实际命令为准；本文件的审查快照不是恢复旧实现的依据。遵守 renderer 880×600 最小桌面合同，不添加手机布局或移动验收。

## 接续前已知代码事实与状态

共享文件、cwd、工具环境、完整 Node 和 macOS 管理员路径的最终变化，应从交付工作树的源码差异、当前架构和验证记录读取。macOS 上的编译、确定性 fixture、原生 App 与管理员验证分别记录，均不自动成为 Windows 证据。Windows 新适配必须复用交付后的共享入口，不复制 macOS 原生实现。

本轮共享进程改造删除 `CapabilityPolicy`、`SandboxPolicy`、`cwd_roots`、`MacSeatbelt` 和 `DenySpawn`；cwd 接受本机绝对路径、合法 `..` 与链接，保留目录存在性、参数、身份、效果和进程生命周期校验。用户环境不再删除正常显式 `NODE_OPTIONS`、`LD_*` 或 `DYLD_*`；应用内部递归变量仍按真实用途处理。

canonical file read/write/patch/search/instruction/rename/delete 使用同一本机路径合同；新 `workspace.files/rename` 输入为 `{path,destination}`，重命名不覆盖目标。macOS/Linux 使用 pinned parent 与原生 rename，Windows 使用 `MoveFileExW` flags 0；Windows 原生编译、UNC/盘符/reparse/ACL、父目录并发仍待本平台验收。不要加 `MOVEFILE_COPY_ALLOWED` 或复制后删除来模拟跨卷 rename；实际 OS 错误必须保留。项目外收据记录绝对 `native_path`、`workspace_path=None`，不制造项目事件或文件卡；默认目录后来被删也不阻断合法绝对文件路径。

canonical write/patch/delete/rename 共享宿主 `NATIVE_FILE_MUTATIONS` 活动写锁，blocking worker 真实结束前不释放，跨默认目录对同一外部目标的活动修改不会并发交错。原 durable Effect key 仍是 owner + 真实 project root：同项目跨 Session 的 pending/unknown 继续由原 Store 唯一索引保护并跨重启保留；**不同项目指向同一外部 native 目标时，未知 Effect 尚无持久目标重叠围栏**。不能从现在磁盘上的 symlink 重建过去目标身份，也不能声称跨项目 durable target fence 已完成。原任务/Session 的 unknown 禁止自动重放保持有效；后续若扩展持久冲突证明，应进入原 Effect 事实和 Store 事务，不另建账本。

Windows 路径边界仍有待设计/原生核实项：`crates/backend/nomifun-file/tests/windows_workspace.rs` 的 `rejects_existing_win32_aliases_without_reading_or_mutating_the_target`、`rejects_existing_ads_without_exposing_stream_bytes` 仍有旧的全面拒绝预期；trailing dot/space aliases、ADS、原子 stream 写/删除和 Device/特殊路径必须按真实 Win32 语义、身份与元数据正确性重新设计并验证，不能在 macOS 猜测后改生产，也不能为保留旧断言恢复 workspace containment。junction 内容操作已改为全局语义，删除 junction 仅删除链接自身；对应新测试本轮未 Windows 原生运行。

本轮工具环境的现役共用入口为 `nomifun_runtime::prepare_process_launch(command, args, cwd, env) -> Result<PreparedProcessLaunch { program, args, environment }, ToolError>`、`process_environment(env)` 和 `user_cli_root()`。每次启动重新组合环境；用户 PATH 优先，显式 PATH 不被隐式混入宿主 PATH，保留真实 HOME/USERPROFILE 和 registry/代理/证书配置。用户 CLI 位于 `dirs::data_local_dir()/nomifun-user-tools/npm`，与 Agent dataset、runtime cache 分离；只有受管 Node 且用户没有配置 npm prefix 时才设置该持久 prefix。受管 Node 以完整官方 tar/zip 发布到 `runtime/node-{version}-{exe_sha}/distribution-v1/`，保留 npm/npx、合法链接和模式并校验完整树；受管 npm/npx 调用 `node.exe + npm/npx JavaScript 入口 + literal argv`，不借此声称用户安装的 `.cmd/.bat` 已解决。上述共享代码仍须 Windows 原生编译与回归。

Agent 的 `engine_process_host::resolve_managed_program`、Terminal 的 `types::resolve_command_for_spawn` / `service::spawn_pty`、MCP 的 `owner_stdio::StdioTransport::launch` 复用该共用准备，显式 Agent shell 使用 `process_environment`。MCP 私有继承环境允许列表和独立 PATH 查找已退役；Terminal/MCP UI 不再显示额外下载确认或二次重放，`withRuntimePreparation` 只读轮询进度。**Windows 尚有明确剩余差异：** Terminal 的 `pty.rs::terminal_command` 保留 `.cmd/.bat/.ps1` PowerShell wrapper；它没有收敛到 Agent/MCP 的 ProcessSupervisor，因此用户 shim 的共享 literal 语义、实现与原生验证仍待本交接完成。

MCP 已退役私有 `normalize_transport`/shell split，结构化 command 不因 args 为空而拆解整条命令；显式 shell 需 shell executable + args。正常含首尾空格、换行的本机文件名和 literal argv 不被 trim/control 策略误拒绝，NUL/空命令/预算检查保留。AppHosting 从已识别的 package/lockfile/static/常规 Node 入口元数据按需准备 Node/Bun，并复用环境与原取消/超时；customNode 优先，任意 shell 文本不被改写。保留的 Windows 专属桥仍待本平台编译和原生验证。

管理员能力沿现有 `workspace.process/exec`、`workspace.process/start` 使用 `execution_identity: current_user | administrator`（缺省 current_user），不是给用户增加模式开关。共享接缝在 `crates/backend/nomifun-engine-core/src/system_process.rs` 的 `SystemEngineOwner` / `SystemEngineSession` 与 `crates/shared/nomi-system-execution/src/lib.rs` 的 `SystemExecutionClient`，复用原 ProcessSupervisor 的 start/poll/terminal/write/close/resize/cancel/quiesce/shutdown 及 output/cleanup 回执。`op: terminal` 只读 `terminal_outcome_if_ready`，不 poll、不续 lease、不消费 running output；自然退出后的晚 stdin/resize 也要先取得真实终态，Windows 适配必须保留此合同。macOS 原生实现在 `apps/desktop/macos-system-execution/`，使用 SMAppService daemon + NSXPC；是否已通过正式签名身份管理员产品验收必须读取本轮最终证据，不能从源码存在推断。非 macOS 当前明确返回 `SYSTEM_EXECUTION_NOT_AVAILABLE`，因此 Windows 仍需完整平台实现；现 runner shell 编码使用 POSIX，Windows 必须正确接入原 PowerShell/原生命令语义，不能照搬。

macOS 组件使用同 Team 与对应 App bundle identity 的双向 XPC 签名校验，dev/prod 分别使用自己的 daemon Label/Mach/plist（`com.nomifun.desktop[.dev].system-execution`），helper 签名 ID 固定；每连接独立 supervisor，断联经 shutdown/watchdog 清理，start 与 cleanup 在 transport 上也有围栏，RPC 丢失保持 unknown。身份包含来源派生的 build identity；升级先关闭 helper 准入并证明所有连接 shutdown，等待 SMAppService unregister 证明旧 service 退出再 register 新 bundle。root 身份保留 XPC 登录调用者的 HOME/USER/LOGNAME。构建、资源与签名通过 `scripts/lib/macos-system-execution.mjs` 和现役 macOS app bundle 路径接入 dev/fast/release，App 提供 `--unregister-system-execution`；说明在 `apps/desktop/macos-system-execution/README.md`。此合同供 Windows 保持同等任务/结果可靠性参考，不要求复制 XPC、SMAppService、POSIX shell 或 macOS 授权状态。Rust、Swift 双架构 typecheck、helper 链接及 packaging/cache 检查的具体结果和正式 App 管理员验收状态以最终验证记录为准。

管理员文件读取已有独立共享只读接缝，**不能只移植旧 process adapter**。正式 `workspace.files/read` / `read_file` 新增 `execution_identity: current_user | administrator`，缺省 current_user，适用于 text、image、instruction_scope 及对应分页；Action、ReadLocal、原资源和冻结 ToolPlan 不变。其他文件动作、VCS 和 search 不接受这个 selector；需要系统身份的写入仍经现役 process。旧当前代会话通过 `session_contract_evolution.rs → resolve_agent_session_contract_evolution → evolve_session_contract → validate_bundled_contract_evolution` 在原 idle/CAS/teardown/未知效果围栏内演进，不另建工具、授权或内容账本。

只读 I/O 共用 `crates/shared/nomi-native-files/src/{lib,source,scope,unix_directory}.rs`：`acquire(NativeReadRequest)` 接收绝对 UTF-8 `Source { path, max_bytes<=8MiB }` 或 `InstructionScope { path, recursive }`，返回 typed `NativeReadAcquisition`。Source 含 `None` 或 `NativeSource { canonical_path, bytes, sha256 }`（wire bytes 为有界 base64）；scope 含 canonical path、kind、绝对 directories 集合、recursive、complete、incomplete_reasons、entries_scanned。**只有真正缺失的 Source None 是 absence**；EACCES/EPERM、悬空链接、非目录祖先和读取变化保留 typed error 的 code/message/kind/raw_os_error，不被 missing_ok 吞掉。

`SystemExecutionClient::acquire_read` 通过 `op: acquire_read` 返回 acquisition 与 worker receipt，纳入原 start admission/quiesce 围栏。macOS 采用同签名 helper `--readonly-acquire` 的 typed stdin/stdout 子模式，现役 ProcessSupervisor 管理 10 秒 deadline、16 MiB 输出、取消/join/断联/清理；未知结果保持 `SYSTEM_READ_RESULT_UNKNOWN` 与真实清理状态。App `workspace_file_read.rs` 校验身份、请求、返回 kind/path/bytes/hash，再复用 FileService 的 `validate_agent_text_read_request` / `project_agent_text_page`、图片 decoder 和 `project_agent_instruction_scope`。8 MiB 严格 UTF-8、完整源摘要/expected_sha256、UTF-8 分页、BOM/行尾、图片格式/4 MiB 和 envelope 预算保留同一投影语义，helper 不复制 decoder 或 shell cat；新 core、workspace/crate manifests 和 lock 进入 helper build/cache identity。

本次 macOS 最终 readonly 组件身份为 `e4f8144299721aabbced92a43516dc981da8144ecb3da84d7f1575c89b7a2394`。真实原生验收曾揭示 foreign C/Swift host 以 libc exit 离开时，Rust 小 JSON 留在 stdout buffer，导致 worker 退出0/已回收但输出为空；`nomi-system-execution/src/readonly.rs` 已采用显式 `write_all` 后 `flush`，失败返回74。后续 Windows native helper/FFI 必须保留这条协议要求：不能只凭 exit0/reaped 认定 acquisition 成功，必须拿到完整有界 typed JSON 和匹配 worker receipt。可重复的 `scripts/validation/readonly-foreign-exit.mjs` 有5项真实 C-exit 回归通过，helper7项及对应完整检查通过；它们与下面真实 App 管理员7项是不同证据面。较早 MSVC/Intel cfg 检查不能冒充本次 flush 修改后的重跑或 Windows 原生结果。

Windows 的**管理员只读 acquire transport 与原生 adapter 仍待设计、实现、编译及原生验证**，不能退回普通用户读取制造结果。普通 Windows owner 仍注入原 pinned `WindowsReadRoot`/metadata adapter；系统路径须保留 I/O 身份、ACL、reparse/race、regular-file、限额和错误语义。`nomifun-agent-runtime/src/workspace_context.rs` 的 scope/layer/directory/recursive-scope 键为 `(execution_identity, native_path)`，每次 refresh 各身份 reader/cache 独立；省略或显式 current_user 均不能借管理员缓存。管理员 process cwd 指令发现与每页 AGENTS/source read 都传同一身份。before-model、effect 后及同一串行 batch 的 effect→read 都须正确失效/刷新；新规则延期后使用新 call ID，不能忽略 EACCES、借另一身份的 absence，或从启动后上下文倒推启动前已读到规则。

内部 current_user instruction read 省略 selector、沿正式缺省，兼容尚未演进的旧 strict schema；administrator 则必须由冻结 read schema 明确声明枚举并通过完整参数校验后才派发，不支持时拒绝预检，无 root 派发或普通降级。同 batch 的可能 effect 在原 ToolCompleted 落盘后按 `affects_workspace && !not_applied` 及时 invalidate，下一调用再预检；保留 failed/unknown 的既有 hold。3 个实际 Session-loop 回归的红灯与修后结果均保留。目标平台应生成自己的组件身份，不硬编码上述 macOS component ID。

本轮产品回归入口是 `scripts/validation/global-os-native.mjs`，通过本机 scripted provider 和公开默认 Agent API 驱动真实 native host、file/process owner 与 canonical 事实，证据范围不包含真实模型推理。macOS 原始证据放在仓库外的临时证据目录，提交的[验证记录](../validation/global-os-2026-10-16.zh.md)列出具体命令、结果和未覆盖项。移交时单独保留必要脱敏日志，临时目录不会随 Git 复制且可能被清理；不要把旧 macOS 日志当成本平台执行结果。

当前 runner 使用隔离 App 正常 owner 登录取得的 JWT，支持默认 `chat.minimal`（实际默认聊天）或 `assistant.general`，不手工补 grant/resources；管理员用例为 `--administrator` 自动创建唯一 root:0700 夹具，禁止任意管理员目录。`global-os-system-cases.mjs` 包含 macOS launchd、唯一原生 CFPreferences 测试域、隔离配置和 nc/curl loopback，提供失败后的 canonical cleanup 与跨 App 退出存续阶段；不能把这些 macOS 命令照搬为 Windows 实现或证明。CFPreferences 的独立测试域不代表日常网络或安全配置全面验收。

以下是本交接更新时已经确定的 macOS 事实，**不是最终整轮完成声明**；后续进展、最终摘要与证据文件以[总验证记录](../validation/global-os-2026-10-16.zh.md)为准：

| 证据面 | 已确认事实与仍待结果 |
| --- | --- |
| 正式 App 身份 | 最终验收为 production ID `com.nomifun.desktop` 的 Developer ID 签名 debug App，notary `d3ed692e-1adf-44ff-8817-64ce62797706` Accepted；管理员/system验收PID69753，用户解锁并允许麦克风后的当前Desktop/媒体补验PID85132，均UID501、同一最终e4签名bundle。签名/公证、staple、spctl证据见总记录，不冒充release/三平台认证 |
| 默认文件/进程产品链 | 当前 General、Minimal 和 Documents 独立目录文件/进程回归均通过；供应器仍为本机 scripted fixture，不声称联网模型推理。Documents 结果不代替 FDA、所有受保护目录或外接/网络卷验收 |
| 原生服务/本机网络/偏好 | 当前新唯一夹具的完整system四阶段32业务步骤通过：安装/配置13、服务8、loopback网络6、恢复/卸载5，各阶段answered、pending/unknown=0、cleanup证明成立。此前App退出同PID存续/原owner重连/新Turn清理独立成功仍保留；旧completion step长748被正式512上限拒绝的失败及其cleanup保留，未被本次成功覆盖 |
| 管理员执行 | 当前最终 App 的完整7业务步骤已通过：root0700/ordinary拒绝、root0600标记写读、管理员 cwd 的 root AGENTS只读预检/新规则延期后freshcall、UID0+cwd+真实登录HOME、独立running poll、XPC cancel/terminal/确切PID退出、夹具清理；canonical answered、pending/unknown=0、cleanup证明成立。此前requiresApproval、rootcwd与空JSON失败均保留；旧root夹具另有成功清理回执，未覆盖原failed报告 |
| 原生退出与系统组件卸载 | App69753经既有SIGTERM observer协调退出，open-W0/本轮7条runtime cleanup；签名App `--unregister-system-execution` 退出0且原生回调完成，launchctl113证明服务不存在，root helper70107/UID0及App退出。后续App85132在补验后经CUA正常super+q退出/open-W0，独占媒体server85373 close0、两attempt元数据保留，helper仍absent；日常App和旧未知状态不动 |
| 桌面与跨应用 | 当前最终e4 App/PID85132的默认assistant.general直接完整桌面闭环已通过：截图、AX/fresh ref点击、键盘输入12+34、独立结果46、退出本次Calculator及canonical answered/cleanup。此前主线程自身AppKit激活、远端应用双向背景focus成功与首次AXRaise未知均保留；不再将current full Desktop写成锁屏待验 |
| 媒体设备 | 用户允许麦克风并解锁后，当前正式WK页真实采集通过：1条audio track/48000Hz，graph连接且running/time advance，随后all tracks ended和AudioContext closed；正式App探针microphone=granted。摄像头新独立请求43ms OverconstrainedError/0track，普通只读库存camera_suspended=1且lid_closed=true，正式App camera=not_determined；摄像头采集仍未成功，拒绝/撤回/恢复矩阵未覆盖，旧拒绝/锁屏/合盖失败保留 |
| 交叉检查 | 较早 readonly freeze 的 Intel Rust/Swift client及Windows共享core/process cfg离线检查通过；最后显式flush修改后未将这些旧记录冒充新重跑。没有Windows原生链接/App/UAC/桌面/installer证据，不替代目标平台工作 |

管理员7项报告为 `administrator-continuation-assistant.general-1792180035898-04f53e8d-4696-4bb4-9529-0bda448754ae.json`，SHA-256 `a1051dd77fe2979b1e4efe86eb921901f7504fecafa811f6ad6aed6a5ac3ae85`，沿原 Session `01a145f4-894d-73c3-ac2a-27d4d2918f87` 的新 Turn 完成。旧夹具清理报告为 `prior-administrator-fixture-cleanup-1792180318419-c09b7a47-7e68-4357-a08e-dc7484dcff89.json`。当前system报告为 `system-assistant.general-1792180741624-559a3c40-b6f3-4c11-aceb-d84034641ecc.json`，SHA-256 `6445b9a8d94d48f86a9227f87e6df6d51c99775435dc4daf01052e2bdbd0637e`，Session `01a1464b-eb69-7282-b763-8c1c2abf9227`，驱动任务37939退出0。driver修复20 tests/173 expectations通过，没有放宽产品完成合同，也无需重建App。这些是验收报告身份，不是 source-review 包摘要；交付位置和最终矩阵以总验证记录为准。

当前完整Desktop补验报告为 `desktop-assistant.general-1792199279748-e3fa2d37-8dae-423a-860f-2acbe7744fff.json`，SHA-256 `85899b7465379571aed89b598ca41fd9de8a631cf4836288f69fa743137f5a01`。媒体原生UI/身份确认见 `media-native-v3-ui-confirmation.json`（SHA-256 `1aa2f13fb0230c1c1ee9b0de4c68f4fe26cd0d614bcdcf80af9afdb10e3f5c0e`），实际App许可探针为 `media-v3-after-capture-permissions.json`；其attempt1音频成功、attempt2视频失败分别保留。只记录轨道/graph/生命周期和许可元数据，未读取、录制、保存或上传媒体内容/样本，也不从麦克风采集推断ASR或模型服务成功。

原生卸载报告为 `native-helper-uninstall-verification.json`，SHA-256 `d13ac2d15ea10a933bd19197ff9273540f3516a3c5d69b4251104a2c75624deb`；后续媒体补验的页面关闭、App85132正常退出、server85373正常关闭和helper未重新注册，见 `media-v3-fixture-teardown.json`（SHA-256 `78665c752e2f43fe8835e538ea3bd4870da9eaedbcd0f8c8c73c7da3df96e13c`）。最终验收App已再次停止，仅保存仓库外 `accepted-final-stdio-app.app`，日常App未动、用户mic grant未重置；接续须用自己的唯一fixture/本平台App，不能假设旧PID/API仍运行。旧crash dataset pending=1原样保留，不因成功退出/卸载抹掉它、新建dataset或放宽unknown。摄像头、完整隐私变更矩阵、外接卷与Windows原生缺口仍按真实状态承接。

以下是本次在 macOS 只读审查得到的 Windows 基线；接续会话必须复核仍然成立的项：

| 能力 | 可确认的既有入口 | 本轮 Windows 状态 |
| --- | --- | --- |
| 普通 Pipe/PTY、输出、取消、后代清理 | `crates/shared/nomi-process-runtime/src/platform/windows.rs`、`platform/windows/conpty.rs`，Job/ConPTY 与 exact process identity | 既有实现；共享Rust cfg在macOS离线cross-check通过，Windows原生链接/完整App编译与生命周期回归仍待执行 |
| 本机管理员执行 | 审查未发现本机 UAC runner、受管 elevated 结果通道或系统执行组件 | 待设计、待实现、待打包、待原生验证 |
| 管理员只读文件/指令 | 正式 read selector、typed native-files core 与 App 投影接缝已共享；Windows privileged acquire/metadata adapter 未接通 | 待设计/实现/编译/原生验收，不能 ordinary fallback |
| 普通桌面 | `nomi-computer` 的 xcap/Enigo/SendInput、`nomi-a11y/src/windows/actor.rs` 的 UIA、Win32 focus | 既有实现；待完整产品原生验证 |
| 高完整性普通应用 | 当前 SendInput 失败文案提及高完整性/UIPI；未发现 uiAccess 或等效正式桌面执行桥接 | 待设计、待实现、待签名打包、待原生验证 |
| 登录桌面与系统服务会话桥接 | 未发现 WTS/CreateProcessAsUser 等本机桥接 | 按选定方案待设计、待实现、待验证；服务 session 0 不能算用户桌面 |
| `.cmd/.bat` 与 npm CLI | 低层 Windows resolver 只查 exe/com；Terminal 另用 PowerShell 包装脚本 shim | 共享改造须复核；Windows literal 参数与实际 shim 待原生验证，未收敛部分待实现 |
| 服务、计划任务、系统安装 | NSIS 安装器和普通执行已有；无通用受管本机管理员链可证明这些任务完成 | 复用原生设施的任务链待实现、待验证，不新建服务管理产品 |
| 媒体/网络/设备 | WebView2 `PermissionRequested` 与现役语音入口存在；非 macOS 系统权限状态有 unknown | 待 Windows 正式身份实测，不把 unknown 变 granted |
| Browser 上传的本机文件来源 | `nomifun-app/src/router/engine_browser_tools.rs` 把 Session 默认 workspace 交给 `BrowserUploadScope`；`nomifun-browser-platform/src/uploads.rs` 的 `root: Dir`、`relative_components` 与 no-follow walk 拒绝绝对路径、`..`、合法链接/junction | 尚存应用限制，待 Windows 设计、实现、编译与 WebView2 原生验证；普通文件工具放开不代表 Browser 上传已全局 |

## 入口与依赖顺序

1. **核实共享路径与默认 Agent。** 从 `crates/backend/nomifun-file/src/{resource,service}.rs`、`crates/backend/nomifun-engine-core/src/process.rs`、`crates/backend/nomifun-app/src/router/engine_process_host.rs`、`crates/backend/nomifun-agent-domain-wave2/src/{workspace_schema,process_schema}.rs`、正式工具说明 `crates/backend/nomifun-agent-contracts/contracts/tool-model-guidance.json` 追踪到默认 `assistant.general` 实际 ToolPlan。验证新会话和可继续的当前代旧会话都可调用文件/进程；需要契约演进时沿现役空闲 canonical transition，不回填历史或强迫新建 Agent。
2. **收敛 Windows 命令准备。** 查看 `crates/shared/nomi-process-runtime/src/{request,command_builder}.rs`、`platform/windows.rs` 的 `PreparedCommand` / `resolve_program_on_path` / `command_argv`，以及 `crates/backend/nomifun-runtime/src/{resolver,managed,shell_env}.rs`、`nomifun-terminal/src/{types,pty}.rs`、`nomifun-mcp/src/owner_stdio.rs`。Agent Pipe/PTY、Terminal、MCP、component 使用同一启动准备规则；native executable 保留 literal argv，明确 shell 保留 shell 语义。移除失去用途的 Terminal 单独 shim 实现，保留 locale 等仍有真实用途的逻辑。
3. **实现可靠 UAC 和系统执行/只读 acquire 链。** 先按最低支持 Windows 版本与当前 Microsoft 官方文档确定正式机制，再实装 runner/service、IPC、认证复用与生命周期，同时完成 typed `acquire_read` transport 和 Windows source/instruction-scope 原生 adapter。不能只用 `Start-Process -Verb RunAs` 启动后丢失输出和进程管理；不能把 `ShellExecuteEx` 返回成功当成任务完成。复用现役 Session/Turn/Effect 与 process owner，保留 output、exit、start failure、partial effect、timeout、cancel、host disconnect 和后代清理证据。文件工具的真实 OS 拒绝沿同一任务选择正确身份；read_file 管理员分支不能降级 ordinary、shell cat 或跳过失败。不能遇到任何 permission denied 都无条件重放或提权。
4. **分别解决身份与桌面。** 明确 standard user、split-token 管理员、over-the-shoulder 认证到另一个账号的行为，保留原登录用户、目标 cwd、HOME/USERPROFILE、配置、ACL 和 profile 加载语义。系统执行与桌面执行是不同上下文；服务不能直接控制用户桌面。普通高完整性应用的观察、focus、输入需采用系统支持的完整性/桌面路径；安全桌面、锁屏和登出准确交给用户或报告环境事实，不自动输入认证，不降低系统保护。
5. **构建与安装同步闭环。** 审查 `apps/desktop/{Cargo.toml,build.rs,tauri.conf.json,tauri.win.conf.json}`、`apps/desktop/nsis/installer.nsi`、`scripts/desktop-build-win.ps1`、`scripts/run-win-build.mjs`、`scripts/check-windows-installer-contract.mjs`、`scripts/validation/windows-authenticode.mjs`。当前 NSIS 是 `currentUser` 安装；若选 uiAccess，必须解决可信签名与管理员才可写安装位置，不能给用户可写未签名二进制加 manifest 就宣称已通。必要组件随架构、签名、升级、卸载、资源定位和 release lock 完整交付。
6. **补齐原生全局任务。** 沿已有执行能力调用原生安装器、SCM、Task Scheduler、注册表/网络等适合目标的设施，不将系统安装强制改为 npm prefix。临时进程依任务清理，用户明确安装/保留的服务或计划任务按意图持续，并能停止、卸载。不要为了保留服务放弃所有临时后代管理。
7. **桌面与设备链。** 追踪 `crates/agent/nomi-computer/src/{capability,input,fallback_backend,screen,launch,permissions}.rs`、`crates/agent/nomi-a11y/src/windows/`、`crates/backend/nomifun-app/src/router/{engine_computer_media,computer_permissions}.rs`、`apps/desktop/src/browser_surface/permissions.rs`、现役麦克风 renderer 调用。正式 App/helper 身份、WebView2 permission、OS privacy、真实桌面和设备调用分别核对；Browser 保持 Windows WebView2，不借此更换引擎。
8. **完成 Browser 上传的全局来源。** 现役入口是 `crates/backend/nomifun-app/src/router/engine_browser_tools.rs` 的 `BrowserUploadScope::open(workspace)`，并非前述已放开的 file owner。继续追踪 `crates/backend/nomifun-browser-platform/src/uploads.rs` 的 `BrowserUploadScope { root: Dir }`、`relative_components`、pinned no-follow 读取与 `PreparedBrowserUpload`，以及 WebView2 实际文件赋值/chooser。设计并实现 Session 默认目录相对路径和本机绝对来源，接受合法 `..`、链接、盘符、UNC 与跨卷来源，不用所有盘符允许列表或先 canonicalize 再无保护 open 替代可靠句柄语义。保持 regular-file 类型限制、来源实际身份、读取前后 metadata、并发修改检测、取消/加入 worker、快照发布与保留生命周期。保留单次 16 文件/64 MiB、每 Runtime 128 文件/256 MiB 等预算；网页仍保留的 File 不能因工具返回、页面 reset 或源路径变化而提前失效。退休不再适用的相对路径解析/拒绝测试，同变更更新 schema/模型说明/错误和当前文档。macOS WKWebView Agent 上传仍真实 `UnsupportedAction`，是已有批准降级的功能缺口，不能当权限拒绝，也不能借本任务改引擎。
9. **退役与文档。** 同一变化删除被替代的代码、状态/UI、DTO、生成项和旧测试/指导。用新行为测试替代 workspace confinement、无管理员路径等失效要求，保留真实 cleanup、identity、atomicity 与 canonical 围栏。更新当前架构和能力矩阵，不另留可被生成器引用的旧方案。

## 不能恢复的历史限制

不得恢复 workspace 文件/cwd confinement、`cwd_roots` 允许列表、MacSeatbelt/sandbox-exec 写沙箱或“受限默认+可信模式”双轨。不得恢复每次准备运行时的额外下载确认 UI；状态查询应只读，真正启动缺依赖才自动准备并保留进度、取消、失败。不得把工具目录、权限库存、schema 声明或管理员认证完成当成能力就绪。

不得引入新权限等级/白名单/授权数据库；不得恢复 retired gateway、transcript、双写或第二 binding/checkpoint ledger。不能通过放宽 owner、Effect、native cleanup 或 Activity 安全边界“修复”全局能力。子任务工具裁剪和共享项目并发写锁仍按实际用途保留。

## 原生环境与当前真实阻塞

本轮主机是 macOS，没有 Windows 原生 API/桌面/UAC/MSVC/安装器及正式签名验收条件；这表示平台工作待交接，不表示 Windows OS 不支持。目标环境需提供原生 Windows 和 MSVC/Windows SDK、支持目标架构的 Rust/Bun、桌面 WebView2、可回滚测试用户或机器、普通用户 token 和可通过系统 UI 授权的管理员。高完整性桌面组件若选 uiAccess，还需正式签名链和受保护安装目录；没有这些条件就标待打包/待签名验证，不能跳过条件制造 PASS。

读取现有最低支持版本与发布合同，核实 x64/arm64 等实际支持架构。管理员凭据仅输入系统原生交互；不读取、保存或复制用户密码。真实服务、网络、证书、系统安装测试在隔离机器/测试用户或明确可回滚环境进行，不修改开发者日常配置凑验收。未知或缺设备应准确记录，不改成权限拒绝或 OS 不支持。

官方约束依据（接续时重新核对版本适用性）：[SendInput/UIPI](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)、[服务与交互用户会话](https://learn.microsoft.com/en-us/windows/win32/services/interactive-services)、[UIAccess 的签名和安装位置条件](https://learn.microsoft.com/en-us/windows/security/threat-protection/security-policy-settings/user-account-control-allow-uiaccess-applications-to-prompt-for-elevation-without-using-the-secure-desktop)、[ShellExecuteEx 的结果与错误](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw)。

## 必须执行的检查

先核对 `package.json`、Cargo feature 和测试实际名称，按修改跑最小有效检查；跨模块收尾执行一次适当完整检查。至少覆盖：

```text
cargo test --locked -p nomi-process-runtime
cargo test --locked -p nomi-system-execution
cargo test --locked -p nomi-native-files
cargo test --locked -p nomifun-engine-core
cargo test --locked -p nomifun-file
cargo test --locked -p nomifun-runtime
cargo test --locked -p nomi-computer
cargo test --locked -p nomi-a11y
bun run check:process-runtime-boundary
bun run check:external-tools
bun run check:agent-session-boundary
bun run check:windows-installer
bun run check:docs
```

另跑受影响 app/terminal/mcp、canonical history/reset/consumer、取消和 native recovery 测试；根据新 runner 加入确定性协议与原生生命周期测试。renderer 变化运行定向 UI 测试、`bun run typecheck`、`bun run check:desktop-ui-boundary`。正式安装包从 `bun run build:win` 与已有签名/release lock 入口产出，验证实际安装后的资源路径、签名、升级与卸载。交叉 cargo check、mock UIA、PowerShell fixture 均不能代替原生产品验收。

## 完整产品验收

用隔离数据根，从**默认 NomiFun Agent 产品入口**执行；宿主分配唯一测试路径并保留首次失败、修复后结果、命令/事实引用与必要脱敏原生证据。以下各项单独记录实现/编译/确定性测试/原生产品结果，不能以一条 npm 或普通文件回归替代其余项。

| 项目 | 必须证明的实际结果 |
| --- | --- |
| 文件与卷 | 工作区外 read/write/search/patch/rename/delete/cwd；盘符、UNC、原生分隔符、链接、ACL/属主/元数据；受保护用户目录及一种实际可用外接/网络卷；管理员文件操作成功后核对内容和权限 |
| 程序与进程 | bare native command、明确 shell、Pipe/PTY、GUI app 启动、支持的其他进程管理；普通和系统身份、用户目录、环境正确；空参数、空格、中文、引号、`% ! & | < > ^ $` 作为 literal 参数不被误执行 |
| 应用安装 | 一条正常用户应用/工具安装与卸载，一条真实需系统权限的可回滚安装/系统操作；读取实际安装/卸载状态，不能仅看 exit 0 |
| 系统配置与持久任务 | 修改并恢复隔离测试配置；注册/启动/查询/停止/卸载 SCM 服务或 Task Scheduler 测试任务；原生状态和 App 退出后的存续符合用户意图 |
| 桌面 | 普通和高完整性测试应用分别截图、UIA、focus、click/type、剪贴板/已支持跨应用操作；多屏与正确登录桌面；安全桌面准确交接用户 |
| Browser 上传 | WebView2 真实 file input 和原生 chooser，工作区外绝对/相对 `..`、盘符/UNC/合法 reparse 与卷来源；原文件与目录并发替换/写入、ACL 拒绝的真实结果；核对网页获得的名称、字节、修改时间及 retained File 生命周期，撤销/异常关闭无泄漏；不得以工作区内一个上传成功代替 |
| 系统交互 | UAC 允许、拒绝、取消、无管理员、设施缺失；平台允许的认证复用；授权/桌面切换后继续受阻步骤；不得循环弹窗、重放部分安装或把认证完成当作任务完成 |
| 管理员只读与指令身份 | 管理员专属 AGENTS/source 夹具：普通读真实拒绝，管理员 text分页/hash pin、image、instruction_scope 复用 native acquire/同投影；默认/显式 ordinary 不借 admin cache；管理员 cwd 新规则先延期再 fresh call；覆盖同 batch effect→read刷新、missing_ok不吞拒绝、认证取消/断联/unavailable不普通fallback |
| 网络与设备 | 本机/局域网监听与访问；已提供麦克风、摄像头等真实调用及 WebView2/OS 许可链；无设备、无桌面、锁屏、权限拒绝分别记录 |
| 生命周期 | cancel、timeout、host exit、后代清理，认证期间中断与输出/结果未知；持久服务按意图保留且可停止，临时任务无泄漏 |
| 依赖 | 隔离 PATH 无系统 Node/npm/npx，自动准备完整官方包；本地 npm fixture 的 bin/lifecycle/参数/Windows cmd shim；新进程、MCP、后续 Turn 和重启可用；升级/缓存清理/Agent clean cut 不丢用户工具 |
| 环境 | 用户已有工具链、显式 PATH、registry、代理/证书、真实 USERPROFILE、系统安装位置；并发下载单飞、取消重试、完整性和原子发布；不修改全局进程环境或粗暴覆盖 profile/npm 配置 |

先跑无需联网/账号/新装工具的快速产品回归，原文提交：

> 请在当前工作区之外创建一个独立测试目录，用文件工具写入并修改一份文本，再在该目录启动一个本机进程，输出实际工作目录和文本内容；随后停止并确认该进程已退出，最后删除本次创建的测试目录，报告实际结果。

宿主核对目录确在 workspace 外，文件 create/read/patch 经过现役文件工具；使用系统自带 shell/测试程序，启动可取消等待进程，核对实际 cwd、修改后文本、输出和退出/清理，不能只跑瞬间退出命令。

管理员能力另用独立子用例：夹具创建唯一且普通用户确实不可写的测试目录，记录 ACL 并先证明 ordinary write 被系统拒绝；默认 Agent 经系统路径在其中写标记、读回、清理，仅系统原生 UI 认证。不得以普通用户可写目录冒充管理员 PASS，不修改日常配置、真实既有服务或用户文件。认证条件不足保留可运行入口和准确阻塞，不能宣称通过。

## 交付标准与风险

交付可审查源码差异、当前架构、生成合同、针对性检查、原生安装包/签名证据及上述矩阵。明确用户现在能做什么、仍需系统交互什么、哪些为系统真实边界、哪些工程未完成。所有缺口分别标待设计/待实现/待编译/待打包签名/待原生验证；没有平台证据不标 Windows 已完成。保持 macOS WKWebView 与 Windows WebView2 原产品结构，保留无关工作；使用配置 Git identity，不 force-push 或改写共享历史，未经要求不提交证据/凭据/缓存。

重点风险是 shim 二次解析导致参数被 cmd 解释、elevated runner 丢失 Job/取消、认证到另一个管理员后写错用户目录、服务 session 0 被误认为桌面、未签名/currentUser 安装却声称 uiAccess、未知效果被重放，以及任务清理误杀用户要求保留的系统服务。必须用真实结果消除这些风险，不能用提示文案或交接本身结案。
