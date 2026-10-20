# Linux 接续任务：默认可信的全局操作系统能力

将本文件全文作为新会话的任务要求执行。先识别目标 Linux checkout、发行版、桌面和认证设施，核实源码后直接实施并完成原生产品验收；不要停在方案报告，也不要再次等待产品方案确认。本交接不是 Linux 已实现或已验证的证明。

2026-10-17 已在 WSL Ubuntu 26.04 接续实现。默认目录删除后同 Session 绝对文件/外部 cwd 闭环、原生管理员与 user/system 服务、Node/MCP/reset、debug 及正式 release-profile deb、release-lock、安装替换卸载的原始证据见[Linux 总验证记录](../validation/global-os-linux-2026-10-17.zh.md)。macOS 报告与 source-review 恢复步骤保留原始历史身份；接续使用当前 Git 源码，系统认证、实际登录桌面与完整产品验收按对应源码的分层证据判断。

本轮最后的 Node 补验已通过实际正式 release App 的 3 个真实 Turn 与正常重启：安装、后继 Turn、重启后同一 CLI 调用及卸载清理。自动准备与 cached-reuse 主体验证按不同原件计证，MCP/reset 仍保持组件层级；详见[Node 验证](../validation/global-os-linux-node-2026-10-17.zh.md)与[正式构建记录](../validation/global-os-linux-release-2026-10-17.zh.md)。源码审查包最终为 `source-review-linux-final-v3`；先前首版/v2 和正式源码快照保持原件。

Computer 已整体替换为 vendored Cua Driver 与 NomiFun 私有 worker。Linux 接续以[唯一实施方案的 Linux prompt](computer-use-rebuild.start-prompt.zh.md#linux-并行补验-prompt)、[Computer 架构](../architecture/computer-use.zh.md)及[方案第 12 节](computer-use-rebuild.zh.md#12-单份执行进展)为准；原自研 Computer 的源码路径、契约、测试和桌面验收不再作为施工输入。保留的历史报告只证明其原始源码，不证明新 worker 的 Linux 原生行为。

## 产品合同

最高原则：**最大权限赋予，最可靠实现，不要搞骚操作。** NomiFun 是用户完全信任的个人 Agent。目标覆盖文件和卷、程序与进程、应用/软件安装、系统配置、服务和后台任务、桌面与跨应用操作、网络与已经提供的设备；npm 只是依赖回归，不能缩减为 CLI/用户目录能力。

普通操作使用真实登录用户；需要系统身份、交互桌面或独立系统许可时，由内部选择稳定、受系统支持且可验证的原生路径。无需权限模式开关，不让整个 GUI 长期 root 运行。不增加分级、白名单、策略中心、自建授权数据库或第二套 Agent 后端。可采用充分可靠的 helper/runner/service，生命周期取决于目标平台需要，复用现役结果、取消和任务链。

workspace 只承担默认目录、项目归属、watcher/VCS 语义，不能充当本机访问上限。合法绝对路径、`..`、符号链接、挂载目录与工作区外 cwd 应直接使用，不能把 `/` 或 HOME 放入允许列表伪装放开。项目外文件不能伪造成项目内产物。Linux UID/GID、ACL、SELinux/AppArmor、系统认证与桌面许可仍遵守平台机制，不关闭保护，不保存管理员密码，不使用绕过 compositor 的临时技巧声称 Wayland 已支持。

用户明确的“只读”“不要执行”“停止”始终有效。Session/Turn/Event/Payload/Effect 保持唯一事实链；owner、冻结 binding、效果结算、native checkpoint、取消、unknown result 与清理证明继续可靠。不另建授权、重试或恢复账本，不重放未知效果，不用 root 身份替代桌面会话与独立许可。

## 先取得完整源码并确认环境

仓库名称为 `nomifun-desktop`，使用目标电脑的实际路径。优先通过 Git 取得包含本文件及本轮完整实现的 `main` 提交，先核对实际 HEAD、工作树与源码，不要重复应用快照。提交前验收及 `source-review` 的历史基线是 `5ffe448e2fe663f85a25ab09a44da6f280509626`；远端重写后的内容等价提交为 `0e54f6905d907951fd8c956de2a17946889e40d1`，两者 tree 均为 `6c82a96b37a2804888c6099f390226cdf86db84a`。仅 checkout 任一基线均不包含本轮改造。本轮源码提交为 `63b91d5af9851855c4c1c32d349a187a5840d430`（`feat(os): enable trusted global system capabilities`）；接续优先取得包含此提交及后续整合的最新 `main`。后续整合保留了远端伙伴、浏览器与 v0.8.12 更新，具体最终 HEAD 以 Git 推送回执和当前仓库为准，历史快照不随提交而改写。

开始前执行并保留必要结果（不公开个人环境/凭据）：

```sh
pwd
git status --short
git branch --show-current
git rev-parse HEAD
git diff --stat HEAD
git diff --name-status HEAD
git ls-files --others --exclude-standard
uname -srmo
cat /etc/os-release
rustc -vV
rustup target list --installed
bun --version
```

本轮根任务导出仓库外完整 `source-review` bundle，源机位置 `/tmp/nomifun-global-os-20261016/source-review`；复制完整目录到 Linux，建议与干净目标 checkout 相邻。目录有六个文件：`BASE`（两行 `branch=main`、`head=<完整 SHA>`）、`tracked.patch`、`untracked.tar.gz`、`untracked-files.txt`（每行仓库相对路径）、`SOURCE-SHA256SUMS`（全部非删除 tracked 改动与必要新增源码的逐文件 SHA-256）及 `BUNDLE-SHA256SUMS`（另外五个文件的 SHA-256）。tar 仅包含必要新增 regular files，不含 symlink、凭据、环境文件、数据库、聊天、缓存、target 或大体积产物。**普通 diff 不含 untracked；只应用 patch 会缺少必要源码。** 最终摘要由根任务冻结导出后提供，不猜测摘要，不混用不同批次载荷与清单。

下面的六文件恢复仅供重建提交前的验收快照；其中 BASE 和摘要固定为原始历史基线。新 Git checkout 若没有该旧对象，应直接取得已交付的完整提交；不要修改 BASE、跳过摘要，或将旧 patch 强套到不同提交。已有完整 Git 交付的 checkout 跳过此恢复段，继续阅读实现与平台验收要求。

以下在**干净、无其他写入者、HEAD 与 BASE 精确一致**的 checkout 根目录以 Bash 执行；需要 Git、tar、SHA-256 工具及常规 Linux shell 工具，bundle 放于 `../source-review`。已有无关改动的 checkout 原样保留，另备干净 checkout，不 reset/clean。任一步失败停止并保留现场，不边失败边覆盖文件。只接收与清单一致的相对 regular-file tar，将它先解到唯一 staging，拒绝已有目标或 symlink 父目录，再应用 patch 和复制全部新增源码。

```sh
set -euo pipefail
review_bundle="$(cd ../source-review && pwd -P)"
checkout_root="$(pwd -P)"
assert_relative_source_path() {
  case "$1" in ''|.|..|/*|*\\*|*:*|./*|*/./*|*/.|../*|*/../*|*/..|*//*|*/) return 1 ;; esac
  test "$(printf '%s' "$1" | LC_ALL=C tr -d '\000-\037')" = "$1"
}
assert_sha_manifest() {
  manifest_rows=0
  while IFS= read -r manifest_line; do
    manifest_hash="${manifest_line:0:64}"
    test "${#manifest_hash}" -eq 64
    case "$manifest_hash" in *[!0-9a-fA-F]*) return 1 ;; esac
    test "${manifest_line:64:2}" = '  '
    assert_relative_source_path "${manifest_line:66}"
    manifest_rows=$((manifest_rows + 1))
  done < "$1"
  test "$manifest_rows" -gt 0
}
assert_new_source_target() {
  test ! -e "$1"
  candidate="$1"
  while test "$candidate" != .; do
    test ! -L "$candidate"
    candidate="$(dirname -- "$candidate")"
  done
}
assert_sha_manifest "$review_bundle/BUNDLE-SHA256SUMS"
assert_sha_manifest "$review_bundle/SOURCE-SHA256SUMS"
(cd "$review_bundle" && sha256sum -c BUNDLE-SHA256SUMS)
test "$(sed -n 's/^branch=//p' "$review_bundle/BASE")" = main
review_base="$(sed -n 's/^head=//p' "$review_bundle/BASE")"
test "$review_base" = 5ffe448e2fe663f85a25ab09a44da6f280509626
test "$(git rev-parse HEAD)" = "$review_base"
test -z "$(git status --porcelain)"
restore_stage="$(mktemp -d "${TMPDIR:-/tmp}/nomifun-source-restore.XXXXXXXX")"
while IFS= read -r source_path; do
  assert_relative_source_path "$source_path"
  assert_new_source_target "$source_path"
done < "$review_bundle/untracked-files.txt"
LC_ALL=C sort "$review_bundle/untracked-files.txt" > "$restore_stage/expected.txt"
LC_ALL=C sort -u "$review_bundle/untracked-files.txt" > "$restore_stage/unique.txt"
cmp "$restore_stage/expected.txt" "$restore_stage/unique.txt"
tar -tzf "$review_bundle/untracked.tar.gz" > "$restore_stage/archive.txt"
LC_ALL=C sort "$restore_stage/archive.txt" > "$restore_stage/archive-sorted.txt"
cmp "$restore_stage/expected.txt" "$restore_stage/archive-sorted.txt"
tar -tvzf "$review_bundle/untracked.tar.gz" > "$restore_stage/archive-details.txt"
test "$(wc -l < "$restore_stage/archive-details.txt")" -eq "$(wc -l < "$review_bundle/untracked-files.txt")"
LC_ALL=C awk 'substr($0, 1, 1) != "-" { invalid = 1 } END { exit invalid }' "$restore_stage/archive-details.txt"
mkdir "$restore_stage/files"
tar -xzf "$review_bundle/untracked.tar.gz" -C "$restore_stage/files" --no-same-owner
git apply --check -- "$review_bundle/tracked.patch"
git apply -- "$review_bundle/tracked.patch"
while IFS= read -r source_path; do
  assert_new_source_target "$source_path"
  mkdir -p -- "$(dirname -- "$source_path")"
  cp -p -- "$restore_stage/files/$source_path" "$source_path"
done < "$review_bundle/untracked-files.txt"
sha256sum -c "$review_bundle/SOURCE-SHA256SUMS"
git diff --check
git status --short
```

SOURCE 清单必须只含规范相对路径和 `64位十六进制摘要 + 两个空格 + 路径`；取得包后仍需审查。不要执行来源包中的 shell 脚本跳过恢复检查。核对成功前保留 staging，成功后保留 bundle、清单与恢复结果。哈希不代替本平台编译、打包和原生验收。

必要新增源文件至少包含 `crates/backend/nomifun-runtime/src/{environment,node_distribution,node_acceptance}.rs`、`crates/backend/nomifun-runtime/node_distribution_contract.rs`、`crates/backend/nomifun-runtime/notices/npm-10.9.4-LICENSE.txt`、`crates/shared/nomi-system-execution/`、`crates/shared/nomi-native-files/`、`crates/backend/nomifun-engine-core/src/system_process.rs`、`crates/backend/nomifun-app/src/router/workspace_file_read.rs`、`apps/desktop/macos-system-execution/`、`scripts/validation/global-os-native.mjs`、`scripts/validation/global-os-system-cases.mjs`、`scripts/validation/readonly-foreign-exit.mjs` 及对应构建资产、测试与文档。实际工作树清单才是完整性依据，不能只复制部分新增文件；最终 readonly FFI 和文档修改后须重新冻结导出，以新包实际摘要验证，不沿用旧包摘要。

完整读 `AGENTS.md`、`docs/architecture/agent-session.zh.md`、[当前全局能力架构](../architecture/global-os-capabilities.zh.md)和[本轮验证与三平台矩阵](../validation/global-os-2026-10-16.zh.md)，核对实际 `git diff HEAD`。旧快照用于发现缺口，不作为恢复被退役代码的设计要求。遵守桌面 renderer 最小 880×600 合同，不添加手机布局或移动验收。

## 现役共享源码与平台差异

共享文件/cwd、命令准备、完整 Node 和 macOS 管理员执行的最终变化，以交付工作树中的源码差异、当前架构及验证记录为准。macOS 编译、fixture 和正式 App/helper 实测分别报告，均不能充当 Linux 证明。Linux 适配复用共享入口，不能复制 macOS 原生实现。

本轮共享进程改造已删除 `CapabilityPolicy`、`SandboxPolicy`、`cwd_roots`、`MacSeatbelt` 和 `DenySpawn`；cwd 接受本机绝对路径、合法 `..` 与链接，保留真实目录、参数、身份、效果和生命周期校验。普通用户的 `NODE_OPTIONS`、`LD_*`、`DYLD_*` 显式配置保留，应用内部递归变量按真实用途处理。

canonical 文件 read/write/patch/search/instruction/rename/delete 使用相同本机路径合同。新 `workspace.files/rename` 为 `{path,destination}`，两端 pinned parent 与 macOS `renameatx_np` / Linux `renameat2` no-replace，保留系统错误，不做复制后删除的跨卷模拟。当前 Linux ext4 元数据/ACL/xattrs 与外部路径、链接原生回归已有；v9fs 新 regular-file 发布通过，existing replace 的命名身份结果仍 unknown，不可重放，详见总验证记录。项目外收据使用绝对 `native_path`、`workspace_path=None`，不伪造项目事件/文件卡；默认目录被删后合法绝对文件操作仍可用。

canonical write/patch/delete/rename 共享宿主 `NATIVE_FILE_MUTATIONS` 活动写锁，直到 blocking worker 实际结束才释放；不同默认目录指向同一外部目标的活动修改不会交错。原 durable Effect key 仍是 owner + 真实 project root，同项目跨 Session 的 pending/unknown 由原 Store 唯一索引保护并跨重启保留。**不同项目指向同一外部 native 目标的未知 Effect 没有持久重叠围栏**，不能按当前磁盘 symlink 补造历史目标，也不能声称已具备跨项目 durable target fence。原任务/Session unknown 禁止重放仍有效；未来可靠扩展应在原 Effect 事实与 Store 事务表达，不另建授权或重试账本。

工具环境共用入口为 `nomifun_runtime::prepare_process_launch(command, args, cwd, env) -> Result<PreparedProcessLaunch { program, args, environment }, ToolError>`、`process_environment(env)`、`user_cli_root()`。每次启动组合环境，用户 PATH 优先；显式 PATH 不隐式混入宿主 PATH，保留真实 HOME、registry、代理和证书。持久用户 CLI 根为 `dirs::data_local_dir()/nomifun-user-tools/npm`，独立于 Agent dataset/runtime cache；只有受管 Node 且用户没有配置 npm prefix 才设置该 prefix。受管 Node 使用完整官方 tar/zip，发布在 `runtime/node-{version}-{exe_sha}/distribution-v1/`，校验完整树、npm/npx、合法链接和模式。该共享代码仍须 Linux 原生编译和发行/产品回归，不能以 macOS fixture 通过替代。

Agent `engine_process_host::resolve_managed_program`、Terminal `types::resolve_command_for_spawn` / `service::spawn_pty`、MCP `owner_stdio::StdioTransport::launch` 复用共用准备，显式 Agent shell 使用 `process_environment`。MCP 原私有继承环境允许列表与独立 PATH 查找已退役；Terminal/MCP UI 删除额外下载确认和二次下载重放，`withRuntimePreparation` 只读轮询进度。缺必要 Node/npm/npx/Bun 时在真正启动链自动准备，查询本身不下载。上述路径本轮没有 Linux 原生编译/产品证明。

MCP 私有 `normalize_transport`/shell split 已退役，结构化 command 不因 args 为空被拆成整条命令；显式 shell 使用 shell executable + args。本机文件名中的合法首尾空格、换行和 literal argv 不再被 trim/control 策略误拒绝，NUL/空命令/预算仍检查。AppHosting 通过已识别 package/lockfile/static/常规 Node 入口元数据自动准备 Node/Bun，customNode 优先，环境复用原取消/超时，不重写任意 shell 文本；Linux 原生回归仍需执行。

管理员能力沿现有 `workspace.process/exec`、`workspace.process/start` 使用 `execution_identity: current_user | administrator`（缺省 current_user），不增加用户模式。共享接缝在 `crates/backend/nomifun-engine-core/src/system_process.rs` 的 `SystemEngineOwner` / `SystemEngineSession`、`crates/shared/nomi-system-execution/src/lib.rs` 的 `SystemExecutionClient`，复用原 ProcessSupervisor 的 start/poll/terminal/write/close/resize/cancel/quiesce/shutdown 与 output/cleanup 回执。`op: terminal` 只读 `terminal_outcome_if_ready`，不 poll、不续 lease、不消费 running output；自然退出后晚 stdin/resize 要据此取得真实终态，Linux 适配必须保持。macOS 的 `apps/desktop/macos-system-execution/` 使用 SMAppService daemon + NSXPC；正式签名身份管理员产品验收状态须读本轮最终证据，不能由源码存在推断。Linux 现役分支使用固定 pkexec/root-owned runner、stdio RPC、原 ProcessSupervisor 与 typed readonly worker，当前真实 polkit 管理员产品已验收；Windows 尚未接通的管理员分支仍保留真实 unavailable。macOS daemon 不作为 Linux 实现。

macOS 使用同 Team/对应 App bundle identity 双向 XPC 签名校验，dev/prod daemon Label/Mach/plist 分离（`com.nomifun.desktop[.dev].system-execution`）且 helper 签名 ID 固定；每连接独立 supervisor，断联 shutdown/watchdog 清理，transport start/cleanup 围栏阻止迟到启动，RPC 丢失保持 unknown。build identity 由来源派生；升级先关闭准入并证明所有连接 shutdown，等待 SMAppService unregister 证明旧 service 退出再 register 新 bundle。root 身份保留 XPC 登录调用者 HOME/USER/LOGNAME。构建和签名在 `scripts/lib/macos-system-execution.mjs` 与现役 macOS app bundle dev/fast/release 路径集成，App 具有 `--unregister-system-execution`，说明为 `apps/desktop/macos-system-execution/README.md`。Linux 现役 pkexec/stdio adapter 与 deb/rpm/AppImage 安装资产保留同类任务/结果可靠性；当前 Ubuntu debug deb 已做原生安装、替换、普通用户启动、卸载与手动组件恢复，其他包/发行版仍按独立矩阵。Rust、Swift 双架构 typecheck、helper 链接、packaging/cache 和正式 App 管理员产品验收的具体结果以最终验证记录为准。

新增管理员只读接口必须一并承接，**不能只实现 system process runner**。正式 `workspace.files/read` / `read_file` 增加 `execution_identity: current_user | administrator`，默认 current_user，适用于 text、image、instruction_scope 和各自分页。仍是原 ReadLocal/Action/资源/冻结 ToolPlan，其他 file/VCS/search 不接受此 selector；系统身份写操作复用现役 process。当前代旧会话沿 `session_contract_evolution.rs → resolve_agent_session_contract_evolution → evolve_session_contract → validate_bundled_contract_evolution` 在原 idle/CAS/teardown/unknown 围栏内演进，不回填内容或新增授权账本。

共享 `crates/shared/nomi-native-files/src/{lib,source,scope,unix_directory}.rs` 的 `acquire(NativeReadRequest)` 仅做 I/O，无 Session/auth/db：Source 请求为绝对 UTF-8 path 与最多 8 MiB，InstructionScope 请求为绝对 path 与 recursive。typed acquisition 是 Source None 或 `NativeSource { canonical_path, bytes, sha256 }`（wire bytes 有界 base64），或含 canonical path/kind/recursive/绝对 directories 集合/complete/incomplete_reasons/entries_scanned 的 native scope。只有真实缺失祖先证明产生 Source None；EACCES/EPERM、悬空链接、NotADirectory 和读取变化保留 code/message/kind/raw_os_error，不能作为 missing_ok 的 absence。

系统接缝是 `SystemExecutionClient::acquire_read` / `op: acquire_read`，返回 acquisition 与 worker receipt并进入原 start admission/quiesce 围栏。macOS 使用同签名 helper `--readonly-acquire` 子模式的 typed JSON，原 ProcessSupervisor 管理 10 秒 deadline、16 MiB 输出、取消/join/断联/清理，未知结果为 `SYSTEM_READ_RESULT_UNKNOWN`，不猜成功或 absence。App `workspace_file_read.rs` 校验身份、输入、返回 kind/path/bytes/hash，再复用 FileService 的 `validate_agent_text_read_request` / `project_agent_text_page`、image decoder、`project_agent_instruction_scope`。严格 UTF-8、完整源摘要/hash pin、分页边界、BOM/行尾、image 4 MiB 和 envelope 限额保持同一语义，helper 不复制 decoder 或 shell cat。新 core、workspace/crate manifests 和 lock 已进入 helper build/cache identity。

本次 macOS 最终 readonly 组件身份为 `e4f8144299721aabbced92a43516dc981da8144ecb3da84d7f1575c89b7a2394`。真实 native 测试发现 foreign C/Swift host 经 libc exit 离开时 Rust 小JSON stdout可能未冲刷，即使worker退出0/reaped也为空；`nomi-system-execution/src/readonly.rs` 已显式 `write_all` 后 `flush`，失败退出74。Linux后续 helper/foreign-host 路径必须保持该要求：收到完整有界typed JSON与真实worker receipt才是 acquisition成功，不凭exit0/reaped推断。`scripts/validation/readonly-foreign-exit.mjs` 的5项真实C-exit回归、helper7项及对应完整检查通过，与正式App管理员7项分别计证。较早Intel/MSVC cfg日志不冒充最后flush修改后的重跑或Linux原生验证。

Linux 现役 typed acquire 已经通过 pkexec/stdio/root-owned runner 接回同一 Unix core/FileService 投影，真实系统认证、默认 Agent text 分页/hash/image/scope/ordinary-denial 产品已通过；不得退 ordinary 读取制造管理员结果。`nomifun-agent-runtime/src/workspace_context.rs` 按 `(execution_identity, native_path)` 保存 scope/layer/directory/recursive scope，每次 refresh 各身份 reader/cache 独立；省略/显式 current_user 不借 admin cache，管理员 process cwd 的 instruction_scope、每页 AGENTS/source read 都传同一身份。before-model、effect 后及同批次串行 effect→read 必须正确失效并发现新规则、延期、使用 fresh call ID；拒绝和不完整指令读取 fail closed，不能用启动后上下文倒推启动前已读规则。

内部 current_user instruction read 省略 selector，走正式缺省以保留未演进旧 strict schema 的普通读取；administrator 必须由冻结 read schema 声明枚举并完整校验参数，不支持则预检拒绝，无 root 派发/普通降级。同 batch 的可能 effect 在原 ToolCompleted 落盘后按 `affects_workspace && !not_applied` 及时 invalidate，再预检下一调用，failed/unknown 原 hold 保留；3 个实际 Session-loop 回归的先红后绿记录保留。Linux 构建生成自己的组件身份，不硬编码前述 macOS component ID。

本轮产品回归入口 `scripts/validation/global-os-native.mjs` 用本机 scripted provider 和公开默认 Agent API 驱动真实 native host、file/process owner 与 canonical 事实，不能声称真实模型推理已验证。macOS 原始证据在仓库外临时证据目录，提交的[验证记录](../validation/global-os-2026-10-16.zh.md)提供命令、结果和未覆盖项；临时目录不会随 Git 移交且可能被清理，必要日志应单独脱敏保留，不冒充本平台结果。Linux 接续须先按 Linux 路径/认证/桌面改造或补充原生验收入口，不能直接用 macOS shell/UID 夹具标 PASS。

runner 用隔离 App 正常 owner 登录 JWT，选择实际默认聊天 `chat.minimal` 或 `assistant.general`，不手工加能力/资源。管理员 CLI `--administrator` 自动分配 root:0700 唯一夹具并拒绝任意管理员目录。`global-os-system-cases.mjs` 包含 macOS launchd、唯一原生 CFPreferences 测试域、隔离配置、nc/curl loopback、失败 cleanup 和跨 App 退出存续阶段；Linux 应改为目标原生设施，不能复制 macOS 命令宣称已支持。CFPreferences 独立测试域不证明全部网络或安全配置。

以下为本交接更新时已经确认的 macOS 事实，**不是整轮最终完成声明**；后续结果和证据文件以[总验证记录](../validation/global-os-2026-10-16.zh.md)为准：

| 证据面 | 已确认事实与待验证部分 |
| --- | --- |
| 正式身份 | 最终production ID `com.nomifun.desktop` 的Developer ID签名debug App，notary `d3ed692e-1adf-44ff-8817-64ce62797706` Accepted；管理员/system时PID69753，用户解锁及允许麦克风后当前Desktop/媒体补验PID85132，均UID501、同一最终e4签名bundle。签名/staple/spctl见总记录，不冒充release或三平台认证 |
| 默认文件/进程入口 | 当前General、Minimal、Documents独立目录回归均通过；本机scripted provider，不声称联网模型推理。Documents不代表FDA、所有受保护目录或外接/网络卷验收 |
| 原生服务/网络/偏好 | 当前新唯一夹具system四阶段32业务步骤完整通过：安装/配置13、服务8、loopback网络6、恢复卸载5，各阶段answered、pending/unknown0与cleanup证明成立。历史App退出同PID存续、原owner重连和新Turn清理独立通过仍保留；旧completion step长748被正式512上限拒绝的失败及cleanup不被本次成功覆盖 |
| 管理员 | 当前最终App的完整7业务步骤通过：root0700/ordinary拒绝、root0600写读、root cwd AGENTS管理员只读预检及新规则延期后freshcall、UID0/cwd/登录HOME、独立running poll、XPC取消/terminal/确切PID退出与夹具清理；answered、pending/unknown0及cleanup证明成立。requiresApproval/rootcwd/空JSON旧失败保留，旧root夹具清理也另有成功回执，不改写failed历史 |
| 原生退出与系统组件卸载 | App69753产品SIGTERM observer协调退出/open-W0/7条runtime cleanup，签名App unregister0且原生回调完成，launchctl113服务absent，root helper70107/UID0与App退出。后续App85132补验后经CUA正常super+q退出/open-W0，媒体server85373 close0、两attempt保留，helper仍absent；日常App与旧未知状态不动 |
| 原e4桌面历史 | 原e4 App/PID85132的默认assistant.general桌面闭环通过：截图、AX点击、键盘12+34、独立结果46、退出本次Calculator、canonical answered/cleanup。原AppKit、背景focus成功和首AXRaise未知保持原身份；该Computer实现已退役，本行不证明私有worker的macOS或Linux原生行为 |
| 媒体 | 用户允许麦克风/解锁后，当前正式WK页1audio track/48000Hz真实采集通过，graph connected/running/time advance，all tracks ended/AudioContext closed，App探针microphone=granted。新video独立OverconstrainedError/43ms/0track，同时普通只读库存camera_suspended1/lidclosedtrue、App camera=not_determined；摄像头采集和拒绝/撤回/恢复矩阵仍未覆盖，旧拒绝/锁屏/合盖失败保留 |
| 交叉检查 | 较早readonly freeze的Intel Rust/Swift client、Windows共享core/process cfg通过，最后flush后未以旧日志冒充新重跑；无Linux原生编译、桌面或系统认证支持证明 |

管理员7项报告 `administrator-continuation-assistant.general-1792180035898-04f53e8d-4696-4bb4-9529-0bda448754ae.json` 的SHA-256为 `a1051dd77fe2979b1e4efe86eb921901f7504fecafa811f6ad6aed6a5ac3ae85`，在原Session `01a145f4-894d-73c3-ac2a-27d4d2918f87` 的新Turn完成。旧root夹具清理报告 `prior-administrator-fixture-cleanup-1792180318419-c09b7a47-7e68-4357-a08e-dc7484dcff89.json` 亦通过。当前完整system报告 `system-assistant.general-1792180741624-559a3c40-b6f3-4c11-aceb-d84034641ecc.json` 的SHA-256为 `6445b9a8d94d48f86a9227f87e6df6d51c99775435dc4daf01052e2bdbd0637e`，Session `01a1464b-eb69-7282-b763-8c1c2abf9227`、驱动任务37939退出0。driver修复20 tests/173 expectations通过，未放宽产品合同且无需App重建。以上是验收报告身份，不是source-review摘要；交付位置与最终矩阵以总验证记录为准。

上述原e4源码的Desktop报告 `desktop-assistant.general-1792199279748-e3fa2d37-8dae-423a-860f-2acbe7744fff.json` 的SHA-256为 `85899b7465379571aed89b598ca41fd9de8a631cf4836288f69fa743137f5a01`。媒体原生UI/身份确认 `media-native-v3-ui-confirmation.json` 的SHA-256为 `1aa2f13fb0230c1c1ee9b0de4c68f4fe26cd0d614bcdcf80af9afdb10e3f5c0e`，正式App许可探针为 `media-v3-after-capture-permissions.json`；attempt1音频成功、attempt2视频失败分开保留。只观察轨道/graph/生命周期/许可元数据，未读取、录制、保存或上传内容/样本，麦克风采集不冒充ASR/模型服务成功。

原生卸载报告 `native-helper-uninstall-verification.json` 的SHA-256为 `d13ac2d15ea10a933bd19197ff9273540f3516a3c5d69b4251104a2c75624deb`；补验页面关闭、App85132正常退出、server85373正常关闭、helper仍absent见 `media-v3-fixture-teardown.json`（SHA-256 `78665c752e2f43fe8835e538ea3bd4870da9eaedbcd0f8c8c73c7da3df96e13c`）。最终验收App已再次停止，仅保存仓库外 `accepted-final-stdio-app.app`，日常App未动、用户mic grant未重置；Linux接续分配新唯一fixture和本平台App，不假设旧Mac PID/API仍活跃。旧crash dataset pending=1原样，不因本次成功抹证据、新建dataset或放宽unknown；摄像头/完整隐私变更矩阵/外接卷/Linux原生缺口仍按真实矩阵承接。

以下文件、进程、系统执行等条目保留 2026-10-17 Linux 接续的源码范围；对应 owner 已接通的能力不按旧基线重复设计。Computer 行使用当前私有 worker 架构，其实际构建、协议和原生证据只在专项第 12 节维护：

| 能力 | 已有事实 | Linux 接续状态 |
| --- | --- | --- |
| 普通 Pipe/PTY、输出与进程组管理 | `nomi-process-runtime` 的 Unix 后端、Linux watchdog/recovery、exact identity | 完整 Linux process-runtime 281 passed / 1 ignored，Terminal 155 passed / 1 ignored；默认 General/Minimal 外部 cwd/Pipe 取消与精确 PID 回收通过，手工压力测试另计 |
| 本机系统认证/管理员执行 | 固定 `pkexec` runner、stdio RPC 与现役 ProcessSupervisor 结果/取消链已接通，deb/rpm 与 AppImage 安装资产已有 | 当前 WSL 以真实 per-process TTY agent 完成用户 polkit 认证，默认 General 管理员 7 正式步骤通过；修复 pkexec 创建线程寿命并保留原生保护；SSH sudo 仍是另一远程域 |
| 管理员只读文件/指令 | Linux typed `acquire_read` 已接原 native-files 与同 FileService 投影，不 ordinary fallback | runner/root 协议与默认 Agent 管理员 source/AGENTS identity、规则延期、UID/HOME、取消回收通过；认证拒绝/取消 fixture 与真实授予分别计证，完整系统许可矩阵按总验证记录 |
| Computer 产品入口 | `nomifun-app/src/router/computer_host.rs` → `nomifun-computer` → 独立 `apps/computer-worker`；22个Action与平台schema从冻结清单派生，宿主不链接桌面驱动 | Linux构建、真实协议及新worker原生证据按[专项进展](computer-use-rebuild.zh.md#12-单份执行进展)判断；旧默认General或自研驱动计数不沿用 |
| AT-SPI与X11 | `vendor/cua-driver/crates/platform-linux/src/{atspi,input,x11}`及对应工具实现提供原生观察与动作；本地修改只能走登记补丁 | 在目标登录桌面核对真实D-Bus/AT-SPI、窗口与输入结果；WSL、私有Xvfb和实体桌面分别计证，不能由源码或环境变量推断可用 |
| Wayland | worker编译`portal-input`，宿主当前固定`CUA_DRIVER_RS_ENABLE_WAYLAND=0`且未启用`portal-capture`；实现位于vendored `platform-linux/src/wayland` | 真实GNOME Wayland/portal及D12启用规则仍待专项验证；不恢复旧Notify/GStreamer实现或用XWayland单窗成功替代原生Wayland证据 |
| 服务与系统安装 | 系统任务沿现役 Linux runner；固定 runner/polkit 资产与安装/卸载/检查入口已有 | 默认 General/Minimal 各完成真实 systemd user service、配置与 loopback HTTP 闭环；真实 polkit 管理员通过。正式 release-profile deb、自动/补充 release-lock、App-exit 持久服务与本地安装回归结果按总验证记录，不把打包 fixture 当正式 Host |
| 媒体/网络/设备 | 语音 renderer 沿 `getUserMedia`/MediaRecorder，设置麦克风测试立即 stop tracks；Linux 后端不伪造 camera/microphone 全局 grant；Browser 宿主仍缺 | 正式 WebKit Settings 已通过 native Cancel/Allow、真实 audio track live→stop→ended；Pulse 后端 AUTHORIZING/图观察未证明真实声音链，物理来源/摄像头未覆盖，主机无 `/dev/video*`；Browser 宿主独立边界不变 |
| 既有 Browser 宿主 | `apps/desktop/src/main.rs` 在非 Windows/macOS 设置 `browser_resources=None`、`attached_chrome=None`；attached discovery 同样仅 Windows/macOS | 现有 Linux 产品实现缺口，不能归因为系统隐私拒绝；记录真实状态，不借本任务无关重写 Browser 引擎 |

第一方 Browser 文件来源还存在独立限制：`nomifun-app/src/router/engine_browser_tools.rs` 将 Session 默认 workspace 交给 `nomifun-browser-platform/src/uploads.rs::BrowserUploadScope`；其 `root: Dir`、`relative_components` 和 no-follow walk 仍拒绝绝对路径、`..` 与合法链接。这条现役 WebView2 上传链已纳入 [Windows 接续任务](global-os-windows.zh.md)，待原生设计/实现/验证；Linux 当前没有原生 Browser 宿主，不能把普通文件 owner 放开说成 Browser 全局上传已完成。macOS WKWebView Agent 上传仍为批准降级中的真实 `UnsupportedAction`，属于功能缺口，不能冒充权限不足。

## 入口与依赖顺序

1. **确认共享全局合同与默认 Agent。** 追踪 `crates/backend/nomifun-file/src/{resource,service}.rs`、`nomifun-engine-core/src/process.rs`、`nomifun-app/src/router/engine_process_host.rs`、`nomifun-agent-domain-wave2/src/{workspace_schema,process_schema}.rs`、`nomifun-agent-contracts/contracts/tool-model-guidance.json` 和默认 `assistant.general` 的实际 ToolPlan。验证新会话及可继续当前代旧会话真正可调用文件/进程，不要求用户重新建 Agent 或切权限。若冻结 binding 要演进，走现役空闲 canonical transition，不回填历史。
2. **确认统一启动和环境。** 检查 `crates/backend/nomifun-runtime/src/{resolver,managed,shell_env}.rs`、`nomifun-terminal/src/{types,pty}.rs`、`nomifun-mcp/src/owner_stdio.rs`、`crates/shared/nomi-process-runtime/src/{request,command_builder}.rs`、`platform/unix.rs`、`platform/linux_{watchdog,recovery}.rs`。Agent Pipe/PTY、Terminal、MCP、component 复用同一准备规则；native executable + literal argv 与明确 shell 分开。每次启动组合 PATH/代理/证书/工具根，保留 HOME、XDG 和用户显式配置，不反复改多线程宿主全局环境。
3. **验证现役系统认证链。** 复核 `nomi-system-execution/src/linux.rs`、固定 `nomifun-system-runner`、polkit action 与打包资产，沿既有 stdio RPC 和 ProcessSupervisor 完成本机认证验收，不新增第二套 runner 或 `sudo` shell 路径。认证仅使用系统原生交互，复用平台允许保留的认证上下文；拒绝、取消、无管理员、无 agent/DBus、headless 等分开返回，不循环弹窗。保留真实 UID/GID、登录用户、HOME/cwd/文件属主和用户工具配置，root 任务不能误写管理员 HOME。SSH sudo 不可当本机路径复用证明。
4. **验证受管进程、只读 acquire 与结果。** 使用现役 Session/Turn/Effect 和 process owner，提权侧负责 literal 命令、输出、exit、timeout、cancel、host disconnect、后代清理和部分效果，同时接通 typed `acquire_read` 的原生 transport/source/instruction-scope adapter 与同 FileService 投影。文件工具的真实系统拒绝沿同任务选择正确身份；read_file 管理员分支不能退 ordinary、shell cat、跳过 EACCES 或凭空补 absence。不要对所有 permission denied 自动提权/整段重放。保留 Linux watchdog/recovery 的进程身份和清理证据，不能给高权限进程返回假 PID/假清理。
5. **验收现役 Computer 的 X11 产品链。** 沿 `crates/backend/nomifun-app/src/router/computer_host.rs`、`crates/backend/nomifun-computer/src/{protocol,worker,result,surface,permissions}.rs`、`apps/computer-worker/src/private_worker.rs` 和固定vendored `platform-linux`核对实际调用。动作与schema只从 `crates/backend/nomifun-agent-contracts/contracts/computer-tools.v1.json`及精简覆盖派生，通过 `scripts/computer/generate-contracts.mjs`和正式契约生成器更新，不复制手写Action表。使用新测试会话、专用窗口和临时数据验证AT-SPI、截图、输入、窗口与用户App生命周期；后台不可用或unknown保持实际结果，不切前台假成功、不重放。worker写入按宿主私有HOME/XDG/CUA环境隔离，不能读取旧Computer配置或用户已有Cua配置。
6. **按专项D12补验原生Wayland。** 先核对 `apps/computer-worker/Cargo.toml`、`crates/backend/nomifun-computer/src/worker.rs`与vendored `platform-linux/src/wayland`，沿现役私有worker完成POC，不恢复自研portal/输入后端。记录真实compositor、portal/AT-SPI版本、授权/拒绝、后台动作、前台输入、session撤销与原生清理证据，再按[专项Linux接续](computer-use-rebuild.start-prompt.zh.md#linux-并行补验-prompt)得出固定启用规则。GNOME Shell helper仅手动安装说明；不新增设置项、自动安装或用Xvfb/WSLg/XWayland冒充原生Wayland。vendor变更仅通过 `vendor/CUA-DRIVER-PATCHES.md`登记和 `scripts/vendor/sync-cua-driver.mjs`应用。
7. **构建、发行与服务闭环。** 检查 `apps/desktop/{Cargo.toml,build.rs,tauri.conf.json}`、`scripts/desktop-build-linux.sh`、`scripts/release-linux.sh`、`scripts/run-linux-release.mjs`、release lock 和现役包配置。必要 polkit action/service/runner、D-Bus 或 systemd unit、桌面文件/portal app identity 随目标架构正确安装、升级、卸载。`.deb/.rpm/AppImage` 的支持与安装位置不能模糊；AppImage 未接系统组件不能伪称有同等管理员路径。system/user systemd、timer、发行版原生安装器/包管理器沿现役执行能力使用，不新建通用服务平台。
8. **审计现有网络/媒体和 Browser 宿主。** 跟踪现役麦克风 renderer、系统权限 router、桌面 WebView、Browser/媒体设备的真实 caller。Linux 桌面 Browser 产品边界与本任务分开说明，不借此重写已确定引擎结构。缺设备/会话、portal 未实现、独立许可拒绝和网络/证书失败分别记录；unknown 不等于 granted。系统服务不冒充交互桌面，无桌面时仍能完成的系统任务继续可用。
9. **退役与同步。** 删除被替代生产代码、DTO、状态/UI、生成项、旧 tests 和当前指导，更新正式 schema、模型说明与当前架构。调整旧 confinement/no-admin 断言为新行为，不能禁用仍有价值的 canonical、identity、atomicity、cleanup 测试制造 PASS。活动 Turn/冻结 Skill 快照不热替换；CLI 带 Skill 需导入 NomiFun 现有库/刷新路径，写入其他 Agent 产品目录不算可用。

## 不能恢复的历史限制

不得恢复 workspace 文件/cwd confinement、`cwd_roots` 白名单、MacSeatbelt/sandbox-exec 写沙箱、“受限默认+可信开关”、运行时下载额外审批和专用确认 UI。状态查询只读，真正执行缺基础运行时才自动准备，保留进度、真实失败、取消、校验和原子发布。

不得新增权限中心/白名单/授权数据库或第二 Agent 后端；不得恢复 retired gateway/transcript、双写、第二 binding/checkpoint ledger；不能绕过 owner、未知 Effect、native cleanup 和活动 Session 围栏。子任务工具裁剪和项目并发写锁按实际用途保留。不得以“Linux UnsupportedPlatform”把三平台目标删掉，也不得用一个空实现把检查变绿。

## 原生环境与当前真实阻塞

2026-10-17原始接续主机为 Windows 上的 WSL Ubuntu 26.04，有 systemd、D-Bus、GTK/WebKit、WSLg 和可连接的 AT-SPI。该轮WSLg的X server暴露XWAYLAND extension，portal frontend未导出RemoteDesktop/ScreenCast/Clipboard，因此不是完整Wayland全局桌面验收环境。这些是原环境身份；接续应重新核对目标主机。当前用户桌面的专用窗口和临时数据可用于Computer验证，隔离账户/私有显示服务/虚拟机按需要选用，不是开发前置条件。系统认证另需可完成polkit交互的实际环境，缺失项按领域记录，不阻塞独立的构建、协议或其他已具备条件的验证。

Computer的真实worker协议测试位于 `apps/computer-worker/tests/protocol.rs`，只管理协议/会话和读取指南，不做桌面动作；私有Xvfb/D-Bus可按需要验证专用窗口，但不证明原生Wayland、系统认证或实体设备。Linux `.deb/.AppImage/.rpm`链接/打包要求目标架构sysroot与系统依赖；仅添加target或交叉check不证明发行包可用。worker依赖及打包方式见[构建说明](../contributing/building-and-packaging.zh.md#computer-worker)，不恢复旧Computer的GStreamer依赖或已删除测试脚本。

真实系统配置、服务、网络、证书等变更只在隔离机器/测试用户或明确可回滚环境验证，不修改开发者日常配置凑验收。密码仅在系统原生 UI 输入，不新增密码字段或日志。不要修改 SELinux/AppArmor 策略为 permissive、全盘 chmod/chown 或弱化系统保护来制造通过。

官方依据（接续时核对目标版本）：[polkit 架构及每用户 session 的认证 agent](https://polkit.pages.freedesktop.org/polkit/polkit.8.html)、[pkexec 的返回值和被清理的执行环境](https://polkit.pages.freedesktop.org/polkit/pkexec.1.html)、[RemoteDesktop 的 EIS 与 session persistence](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html)、[portal Session 生命周期](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Session.html)。不能假设 pkexec 自动保留原用户 GUI 环境，也不能假设所有 portal backend 实现同一版本能力。

## 必须执行的检查

先核对 `package.json` 和 Cargo feature/测试名称，以最小直接检查覆盖修改，跨模块收尾做一次适当完整检查。至少运行：

```text
cargo test --locked -p nomi-process-runtime
cargo test --locked -p nomi-system-execution
cargo test --locked -p nomi-native-files
cargo test --locked -p nomifun-engine-core
cargo test --locked -p nomifun-file
cargo test --locked -p nomifun-runtime
cargo test --locked -p nomifun-computer
bun run check:process-runtime-boundary
bun run check:external-tools
bun run check:agent-session-boundary
bun run check:docs
```

另运行受影响 app/terminal/mcp、canonical history/reset/consumer、native recovery/取消/cleanup 测试；renderer改动运行定向UI、typecheck与桌面边界。Computer在独立工作区构建/测试，先用 `bun scripts/computer/build-worker.mjs --target x86_64-unknown-linux-gnu --profile release`取得实际worker，再运行其 `--self-check`和 `bun scripts/computer/freeze-tools.mjs --worker <该实物绝对路径> --check`；独立协议使用 `cargo test --locked --manifest-path apps/computer-worker/Cargo.toml --target x86_64-unknown-linux-gnu --release --test protocol`。按目标OS分离target/build目录，复用专项已通过的同源证据；这些命令不证明桌面操作。新增或改变Computer接线时运行 `bun run check:computer-boundary`及受影响App/契约检查。原生包用 `bun run build:linux`与现役release lock入口验证worker、依赖和安装/卸载，先核对实际脚本与架构；原生场景和真实模型按专项第9节、发布范围及可用环境选择，不再调用退役crate/dev脚本。

## 完整产品验收

隔离临时目录和数据根，从**默认 NomiFun Agent 产品入口**执行，保留首次失败与修复后新结果。每项分别记录源码/编译/确定性测试/原生产品层级、真实桌面组合和证据，不能用 CLI 依赖或单个文件用例代替整个系统能力矩阵。

| 项目 | 必须证明的结果 |
| --- | --- |
| 文件与卷 | 工作区外读/写/搜索/patch/rename/delete/cwd，合法 `..`、symlink、挂载、UID/GID/ACL/元数据；受保护用户目录、一种真实可用外接/网络卷；管理员写后核对实际内容/元数据 |
| 程序与进程 | bare native command、明确 shell、Pipe/PTY、GUI app 启动、支持的其他进程管理；普通/root 身份及真实 HOME/cwd；空参数、空格、中文、引号、`$ ; & | < > *` literal 不被误执行 |
| 软件安装 | 正常用户工具/应用安装卸载，以及真正系统权限的可回滚原生安装/系统操作；读取原生安装结果，不能仅退出 0 |
| 系统配置和服务 | 改并恢复隔离配置；systemd 或目标平台设施注册/启动/状态/停止/卸载测试 service/timer，区分 user/system；App 退出后存续符合用户意图 |
| X11 桌面 | 默认 Agent 截图、AT-SPI tree/动作、可靠 focus、click/type、剪贴板/已支持跨应用；正确登录 session、多屏/坐标，真实 native window 状态核对 |
| Wayland 桌面 | 对明确支持的 compositor/portal 组合分别验证 ScreenCast/PipeWire、RemoteDesktop/EIS 或适用原生路径、AT-SPI、聚焦实际能力；授权/撤回/session 失效和 restore token 更新；XWayland 单 app 成功不能代替 |
| 系统交互 | 原生认证首次授予、允许、拒绝、取消、无管理员/无 agent/设施缺失；可复用上下文；许可变化、必要重启后继续步骤，未知效果不重放 |
| 管理员只读与指令身份 | root 专属 AGENTS/source 夹具：普通读真实拒绝，管理员 text分页/hash pin、image、scope 复用 native acquire/同投影；默认/显式 ordinary 不借 admin cache；管理员 cwd 新规则先延期再 fresh call；同 batch effect→read刷新、missing_ok不吞拒绝、认证取消/断联/unavailable不普通fallback |
| 网络和设备 | 本机/局域网监听与访问；已提供麦克风/摄像头真实调用及 host/renderer/portal/device 权限链；缺硬件、缺显示环境、拒绝分别说明 |
| 生命周期 | timeout、cancel、host exit、后代清理，认证/portal 期间中断与实际结果未知；明确持久 service 按意图存续可停止，临时任务无泄漏 |
| 依赖 | 无系统 Node/npm/npx 的隔离环境自动准备完整官方发行；本地 npm fixture bin/lifecycle/literal 参数；新进程/MCP/后继 Turn/重启可用，升级和 Agent clean cut 不丢用户工具 |
| 环境 | 现有工具链、显式 PATH、registry、代理/证书、HOME/XDG、系统安装路径正确；并发准备、取消重试、完整性、原子发布；不粗暴改 profile/npm 设置或宿主全局环境 |

先跑不依赖联网/账号/安装工具的快速产品回归，原文提交：

> 请在当前工作区之外创建一个独立测试目录，用文件工具写入并修改一份文本，再在该目录启动一个本机进程，输出实际工作目录和文本内容；随后停止并确认该进程已退出，最后删除本次创建的测试目录，报告实际结果。

宿主分配唯一 workspace 外路径，仅操作本次内容；create/read/patch 必须走现役文件工具。使用系统 shell/既有程序，启动可取消等待进程，实际核对 cwd、修改后文本、output、退出与清理，不能只跑瞬间退出命令。

管理员子用例独立进行：夹具准备唯一且普通用户真实不可写目录，记录 owner/mode/ACL 并先证明 ordinary write 受 OS 拒绝；默认 Agent 沿系统路径写测试标记、读回、清理，仅原生认证。不能以普通可写目录冒充管理员成功，不改日常系统配置或既有用户文件。缺认证/隔离环境则交付可运行入口和明确未覆盖原因，不宣称通过。

## 交付标准与风险

交付可审查源码差异、当前架构、正式说明/生成项、定向检查、原生包与完整矩阵。说明用户现在直接能做什么、仍需系统认证/桌面许可什么、平台真实边界是什么、工程缺口是什么；逐项标待设计/待实现/待编译/待发行集成/待原生验证。只有对应 Linux 原生证据才可标该组合通过，不声称所有发行版、X11/Wayland 或三平台一并完成。

重点风险包括 pkexec 环境导致写错用户 HOME、提权丢失进程树管理、portal session/token 被当成永久自建授权、环境变量探测代替真实能力、X11 click-to-raise 副作用、Wayland fallback 假就绪、AppImage 遗漏系统组件、任务结束误杀持久服务，以及未知部分效果被重放。必须通过实现与真实结果消除，不能用提示文案或交接本身结案。保留无关工作和配置 Git identity；不 force-push/改写共享历史，不提交秘密、数据库、聊天或大体积产物。
