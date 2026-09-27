# Windows Case 处理进度

更新：2026-09-27。当前宿主 Windows。执行顺序见 [实施计划](IMPLEMENTATION-PLAN.zh.md)。
覆盖 675 个共享 + 82 个 Windows 专属 Case；按适用 Agent 展开为 2,374 槽。
Agent 槽数：GEN 600、COD 594、PAL 323、MM 567、CS 242、HOST 48。
共享 P0 已由 `85a079fc0` 提交/推送；W01 首批组件走查完成，不重复维护共享根因文本。

## Windows 专属集合

以下集合共 82 条，其余本平台任务取目录中的 Both Case；范围包含首尾。

| 家族 | Windows 专属 ID |
| --- | --- |
| CMD | 002、006～009、013～014、016、020、022、024、026、028、031～032、037、039、042、044、046、048、050、052、054、057、059、063～064、066、068、070、072、074、077、079、113、124～126、131、133 |
| PROC/TERM/FILE | PROC-005/013/021/045；TERM-011；FILE-021 |
| Browser/Computer | BROW-016；COMP-007 |
| UI/宿主 | REAL-001～015；WIN-001～018 |

## 全领域排程

每个领域的精确家族/ID 范围见实施计划；只取 Windows 适用且该 Agent 有产品目标的槽。

| 领域 | 槽数 | 批次 | 平台测试、排查和修复重点 | 状态 |
| --- | ---: | --- | --- | --- |
| D01 | 425 | W02 | 正式 Session/模型/工具面，中文参数、必填拒绝、版本冻结 | 待走查 |
| D02 | 302 | W02/W04 | 计划/完成、日志与 UI 终态；用户纠正/停止；零红色正向任务 | W02 GEN 完成报告仍有可见失败；当前证据 Schema 预检已补，问题未关闭 |
| D03 | 145 | W01/W03 | Win 路径、共享锁、原子 write/patch/delete、Artifact | W01-B1/B2 路径及原子写子断言已验；剩余待走查 |
| D04 | 476 | W01 | executable/args/cmd、PowerShell/cmd、编码、Job/ConPTY、退出与清理 | 新基线首批组件验证通过；完整 CMD/终端矩阵待走查 |
| D05 | 69 | W03 | 本地隔离 Git remote、SSH 夹具；取消/未知副作用 | local/file push owner 9 项通过；SSH 及完整产品路径待准备 |
| D06 | 247 | W04/W05 | WebView2 profile、Computer A11y、MCP/Plugin/Skill | Skill 首发已回归；其余待走查 |
| D07 | 124 | W02/W04 | 精确产品目标、Knowledge/Companion/Canvas/Customer owner | PAL/MM 新 Tauri 入口已走查；修复画布名称上下文遗漏 |
| D08 | 103 | W02/W04/W05 | GEN/COD/PAL/MM/CS 分角色的正式任务 | W02 四角色有新证据；GEN 仍 FAIL，完整集合与 CS 待走查 |
| D09 | 375 | W06 | Job 子孙进程、崩溃、恢复、撤权、并发与 LONG | checkpoint/执行 lease 组件 19 项已验；宿主故障、正式 UI 与 soak 仍待 |
| D10 | 33 | W01/W06 | WIN-001～018 与 PORT，盘符/路径/共享锁/宿主终态 | WIN-002/007/008/010/011 的部分组件断言通过；其余待走查 |
| D11 | 75 | W01/W03 | 越界路径/junction、旧授权、无资源与跨 owner 拒绝 | AUTH-009/010 的 cwd 子断言通过；其余 owner 与竞态待走查 |
| **合计** | **2374** | W01～W06 | 结果按 Case × Agent × Windows 独立判定 | 未跑不算通过 |

## 近期可领取任务

| 任务 | 对应 Case / 断言 | 测试与修复安排 | 状态 |
| --- | --- | --- | --- |
| W01-A 命令形状与启动 | G0-003/004/010；PROC-001～004；WIN-008/009；CMD-131/133/147 | 验字面 argv、显式脚本、混合形式拒绝、PowerShell 初始化及退出状态；PATH 上 Bun 也通过 owner 实际启动 | 组件子断言已验证；持久/脱离命令 policy 和新构建 Tauri CMD 仍待验收 |
| W01-B 路径与权限边界 | AUTH-009/010；FILE-021；WIN-002～007/017 | cwd junction 已验；文件 owner 原始分段、原子 write/共享锁、删除链接及长路径回归 | B1/B2/B3 分别 80/57/37 项定向回归通过（重叠不累加）；跨盘/ACL/并发置换及正式 UI 待验 |
| W01-C Job 与终端清理 | PROC-027～036/045；WIN-010～012/014/016 | 实际 Job 子孙清理、leader 先退出、stdin/EOF、ConPTY resize/cancel/快速退出/UTF-8 分片 | 首批组件子断言已验证；代码页、模拟锁、应用强杀/重启仍待走查 |
| W02 正式核心入口 | AGEN-001/014/017；ACOD-001/008；APAL-001；AMUL-001 | 新 Session/冻结快照；文件、产物、删除、伙伴身份及画布名称由 UI/磁盘/DB 对账 | 首批 8 回合已验；MM 名称已修；GEN 完成报告仍 FAIL_VISIBLE_UX，不能计 PASS |
| W03 文件/Git/SSH | D03/D05/D11 剩余适用 Case | 文件准备期/错误归因、大小写、ACL/部分删除、隔离 local/file remote；其余逐簇推进 | 文件首批 32、大小写 50、ACL 60 项（重叠不累加）、Git push owner 9 项通过；并发/跨盘、SSH 与完整 UI 待验 |
| W04 UI/扩展与恢复 | REAL、OBS、D06；旧 MM retry | 旧失败历史丢失与无效重试入口已复现并修复；不重写旧 Snapshot | 原旧会话 Tauri 冷加载/刷新复验通过；其他恢复/停止/扩展 Case 待验 |
| W05 条件业务资源 | ACSR、媒体、Channel、Robot 及其 D07/D08 Case | 建最小正式入口/模型/测试租户；无前提不计 PASS，不擅自扩权 | 部分 BLOCKED_FIXTURE |
| W06 生命周期与长稳 | LIFE/CONC/LONG、WIN-015/016/018 | 逐状态故障注入、取消/重启/lease、宿主 UI/API 一致性，最后长稳统计 | Store checkpoint/lease 首批 19 项通过；真实宿主故障与长稳待验 |

## 已有结果与本批记录

- 2026-09-26，旧构建 `4d525c84d384`：APAL-001 名字正确（1 模型步）、AMUL-001 默认 Skill 首发成功（3 步）、
  GEN CMD-147 命令与 7 项属性准确（2 步）；各自零工具错误，原失败保留。
- 旧构建 `e67a8810d01d`：ACOD-001 相同只读请求 5 模型步、零工具错误、三文件哈希未变；此前回归的 1 次计划错误保留。
- 这些结果来自此前隔离 Tauri；新合并构建未执行的 Case 仍未验收，不能直接移植旧 PASS。
- 本轮 Windows 证据根：仓库外 `phase-2-3/2026-09-27/windows/`。本页只保留批次结果和残余断言。
- 2026-09-27 W01 首批：基于 `85a079fc0`，补测试后共 **43 个不同测试通过**；没有付费模型调用，
  不将测试数量折算为 43 个完整 Case。未发现新的生产缺陷，补齐了原先仅 Unix 覆盖的 cwd 链接边界。

| 验证命令（均重定向至外部目录） | 本批结果 | 证据文件 |
| --- | --- | --- |
| `cargo test -p nomi-process-runtime --test process_contract --test request_contract --test pty_contract` | 原有进程 10、PTY 8、请求 14 项通过 | `native-process-request-pty.log` |
| `cargo test -p nomi-process-runtime --test request_contract` | 新增 junction 后请求 16 项通过，替代上述 14 项计数 | `request-with-windows-junctions.log` |
| `cargo test -p nomifun-engine-core --lib process::tests` | 6 通过，1 项 Bun 检查原有 ignored | `engine-process-owner.log` |
| `cargo test -p nomifun-engine-core --lib process::tests::managed_owner_launches_bun_from_path -- --exact --ignored` | 本机有 Bun，显式运行该项并通过；本批不再留为 skipped | `engine-bun-path.log` |
| `cargo test -p nomifun-agent-domain-wave2 --lib process_schema::tests` | 2 通过，覆盖两种入口与互斥 Schema | `process-schema.log` |

### W01-B1 文件路径（基线 `bd9fea1fb`；提交 `a88932f02`）

- 子断言：AUTH-009、WIN-002～005；GEN/COD/MM 共享文件 owner，不折算为角色或完整 Case PASS。
- 首次结果：新增 5 项测试中 3 FAIL、2 PASS。真实 ADS 内容可读，`nested./report.txt` 读取了
  `nested/report.txt`；名称检查漏过 `COM¹`/`LPT³`、通配符及控制字符。首次日志保留。
- 修复：在 workspace/Artifact 共用的相对路径规范化中，先验证每个原始分段，再访问文件系统；
  补全 Windows 名称规则。UNC/device/绝对/盘符相对路径在根解析前拒绝；合法中文/空格/emoji 和
  既有文件大小写 alias 仍解析到同一 canonical 文件。
- 验证：`cargo test -p nomifun-file --test windows_workspace` 5/5；同 crate 的
  `--lib path_safety::tests` 19、`resource::tests` 3、`artifact_store::tests` 14，
  `--test file_read_write --test file_management` 39，共 80 项通过；`git diff --check` 通过。
- 证据：`windows/w01b-paths/01-before/windows-workspace.log` 与 `02-after/*.log`，日期根同上；
  未调用模型。仅验证普通 workspace 拒绝 UNC，未准备授权 UNC share；macOS/正式 UI 未验。

### W01-B2 原子写与共享锁（基线 `a88932f02`；提交 `24dc85eca`）

- 子断言：FILE-019～021/038/039、WIN-006/014，GEN/COD/MM 共用 owner；公共根因见 S-D03-01。
- 首次结果：9 项中的 2 FAIL（deny-delete 下 write 成功、原件被截断）、7 PASS。接入原子发布后，
  原有 `MoveFileExW` 又在允许全部共享的读句柄下拒绝；独立 Win32 探针确认并保留中间失败。
- 修复：Windows 原生替换先验证 write/delete 访问并保留 ACL/原件备份；绝不回退直接覆盖。
  补准确 created 回执、失败时不发成功事件、临时文件清理、明确解锁重试；发布/清理不确定时阻断换 key 重放。
- 验证：`nomifun-file --test windows_workspace` 10，`--lib service::tests::atomic_` 4、
  `service::tests::agent_` 10、`patch_temp_collision` 1；
  `nomifun-app --lib router::agent_wave2_host::tests` 32，共 57 项通过；`git diff --check` 通过。
  覆盖部分原生失败恢复、不覆盖并发目标、备份清理锁、只读目标以及重启后 durable pending 保持。
- 证据：`windows/w01b-atomic/{01-before,02-after,03-after,04-final,05-final,native-probe}/`。
  首次故障夹具未命中清理阶段、首次 App 测试误按旧 key 的 pending 文案判断新 key 拒绝，均保留；
  后者改为核对既有 Pending 记录、无新 effect 和磁盘原件，未放宽副作用断言。
- 未覆盖：原生 ACL 拒绝夹具、磁盘耗尽、应用崩溃窗口、macOS、正式 Tauri 全 Case；没有模型调用。

### W01-B3 删除链接与长路径（基线 `24dc85eca`；提交 `13da6ca3f`）

- 子断言：AUTH-009/010、FILE-024/032/036、WIN-005/007/017；共享根因见 S-D03-02。
- 首次 16 项中 3 FAIL、13 PASS：删除根内 junction 递归删除了真实目标；空路径删除了工作区根；
  合法 255 字符 basename 因临时名称拼接而失败。首次日志保留。
- 修复：删除前检查原始 entry 并明确拒绝末端链接与空路径；临时文件使用固定长度的独立名称。
  验证普通父目录中的根外 junction 只删除链接、既有文件大小写 alias 在 patch 准备期去重。
- 验证：`nomifun-file --test windows_workspace` 16；同 crate `--lib service::tests::atomic_` 4、
  `service::tests::agent_` 10、`patch_temp_collision` 1、`--test file_management remove_entry` 6，
  共 37 项通过；`git diff --check` 通过。长路径验证含超过 260 的嵌套路径、255 字符 basename 的
  write/overwrite/patch/read/delete，以及 256 字符超限稳定拒绝且不改近似 sibling。
- 证据：`windows/w01b-links-length/01-before/windows-workspace.log`、`02-after/`、`03-final/`。
  未覆盖新建文件大小写碰撞、并发链接置换、宿主总路径长度极限、跨盘/ACL、macOS 和正式 UI。

### W02 核心入口首批（基线 `13da6ca3f`；提交 `ca37bb27f`）

正式 Tauri `cargo build -p nomifun-desktop --features tauri/custom-protocol`，使用 dev 配置及独立
data/work/profile；共 8 个新回合、40 个模型步骤，未超 8 回合/80 步预算，未调用媒体生成。
全部使用已有加密配置中的 StepFun Coding Plan / `step-3.7-flash`。初始钥匙与新目录不匹配的
准备失败日志保留，初始化完成后恢复配套钥匙；没有修改 Session/Snapshot 或扩大授权。

| Case / 子断言 | 首次发现与修复 | 验证结果与残余 |
| --- | --- | --- |
| AGEN-001/014/017；CTRL-007/OBS | GEN 入口、文件创建、Artifact 发布回读及删除成功；首次完成声明引用了失效文件证据。补过期路径提示及当前有效引用的 Schema 枚举，整批预检复用实际暴露的 Schema，原有效性/权限规则保留 | **仍 FAIL_VISIBLE_UX**：三次真实回合分别 9/10/9 步，均保留一次完成报告拒绝及后续恢复。最后一次在 Schema 预检拒绝，未进入控制执行；最终摘要还遗漏所要求的产物 ID/回读详情。磁盘字节、Artifact digest、临时文件删除和四次效果回执一致。后续需核对 provider 实际 payload 与引用/交付策略，不能靠重跑或隐藏错误关闭 |
| ACOD-001 只读源码与计划 | 正式编程入口，3 个源码文件 hash 不变，没有命令或写效果 | 5 步，0 owner 错误、0 可见工具失败。首次读 AGENTS 引起 2 个指令边界 deferred，另有 1 个 durable 正文省略占位；原记录保留，不将这些记录等同于 owner 执行失败 |
| ACOD-008 指定源码删除 | 新隔离 Git 项目，用户明确指定 `src/obsolete.rs`，保留 `src/keep.rs` | 4 步，0 工具错误；恰好一次 delete 回执，目标缺失、保留文件 hash 不变，无 shell 绕过 |
| APAL-001 伙伴身份与资源 | 从伙伴产品入口创建并配置 Windows验收伙伴 | 1 步、0 工具调用，名字正确，绑定本次 Companion，UI/Turn 均完成；仅本次样本，不外推所有入口 |
| AMUL-001 画布/资产库/Skill 首发 | 初次 Canvas context 缺名称，模型明确无法报告；补来自相同 Canvas ID 的有界标题，缺信息仍为 null | 首次与修复后各 1 步；修复后正确报告“无限画布 1”、0 节点/0 连线及默认技能。绑定准确，无模型写/媒体效果；文档中的新会话关联与面板状态变化单独保留，未宣称整份文档字节不变。旧 retry spinner 未复现、未关闭 |

- 验证：Runtime `cargo test -p nomifun-agent-runtime --lib` **138/138**，包含向实际模型请求暴露
  当前引用、失效/失败引用拒绝、空证据 Schema、计划/完成整批顺序；画布 context **6/6**，
  `bun run typecheck`、`bun run check:desktop-ui-boundary`、`git diff --check` 通过。
- 证据：`windows/w02-core/{01,02,03}/` 保存 binary hash、源补丁/基线、截图、DB、完整事件与模型轨迹；
  两组组件首次失败/修复后日志在 `windows/w02-completion/`、`windows/w02-canvas-title/`。
- 未覆盖：N=3/统计验收、完整角色矩阵、CS 条件资源、macOS、GEN 交付失败的最终关闭及旧 MM retry。
  上表只记录本批子断言和具体运行，不将组件数或 recovered 计入完整 Case PASS。

### W03 文件准备期与错误归因（基线 `ca37bb27f`；提交 `e15e8515e`）

- FILE-032/039、OBS-005/016 子断言，公共根因 S-D03-03。首次各保留一个 FAIL：包含结果未知
  标识文本的目录名使普通临时文件重名错误被误判为 unknown；同批文件目标 `new`/`new/child.txt`
  在第二项失败前已发布第一项并留下新文件。
- 修复：只识别 owner 生成的错误前缀；准备期拒绝重复或互为祖先的文件目标，两种顺序均零文件
  写入、零父目录创建。真正发布/清理不确定的处理规则保留。
- 验证：`nomifun-file --lib patch_temp_collision` 1、`service::tests::atomic_` 4、
  `service::tests::agent_` 11、`--test windows_workspace` 16，共 32 项通过；diff 检查通过。
- 证据：`windows/w03-file-errors/{01-before,02-after,03-ancestor-before,04-final}/`；无模型调用。
  未覆盖 Windows 新建大小写别名、跨盘/ACL、并发置换、Git/SSH、macOS 与正式 UI 完整 Case。

### W03 local/file Git owner（基线 `e15e8515e`）

- VCS-010/011/012、AUTH-004 子断言：实际临时 bare remote 的 ref 与回执一致；non-fast-forward、
  force、跨仓库绑定、缺失 remote、未挂载凭据和 secret 参数均保留拒绝边界。未发现新生产缺陷。
- `cargo test -p nomifun-app --lib router::agent_wave2_vcs_push::tests` **9/9**；证据
  `windows/w03-vcs/01/`，临时仓库均在该运行目录，无生产 remote/凭据、无模型调用。
- 未覆盖 receipt 丢失、取消/并发、hook、完整 VCS 矩阵与真实会话 UI。当前可用盘符只有 C:
  （Temp 同盘），无真实跨卷夹具；PATH 未发现 sshd，尚未配置隔离原生 SSH 测试主机。
  这些仅阻断相关跨卷/SSH Case，其他任务继续。

### W04 旧 MM 失败历史与重试（GUI 基线 `829db4dd9` + 本批源码补丁）

- AMUL-001、OBS-008/014、MGMT-013 的历史恢复子断言：从原 `WIN-MM-PROVISION-01` 失败现场，
  经正式 `nomicore backup/restore` 安装隔离 data/work/profile；保留 Session/消息/Turn ID、Snapshot
  与绑定。首次直接复制 DB 的启动被 root identity 门禁拒绝，保留 `01`，随后改用正式恢复入口。
- 旧现象在正式 Tauri 重现：错误卡片变空白，只剩静态 Refresh 重试图标。点击后 55 秒无新
  request/event/Turn，原 Turn 仍 failed；未观察到真正运行中的 spinner。根因是历史仅取文本并
  硬编码 complete，加上无 pendingTurn 时仍显示重试按钮、handler 直接返回。
- 修复：从 canonical Turn 补终态，不占用原消息分页；保留消息身份与公开错误。前端严格接受
  failed/stopped，恢复不得报告 completed，失败/停止不能作为可应用提案。仅未确认 pending
  的最后失败消息保留重试入口，仍复用原 key；已结算旧失败不再显示无效入口。
- 首次后端反例 1 FAIL、前端 28 PASS / 4 FAIL 保留；最终 UI 57、Store 1、App route 1，合计
  **59 项定向测试通过**，typecheck、desktop-ui-boundary、Tauri 构建通过。同步远端独立的伙伴
  UI 提交 `2d4dac3f6`，未覆盖他人改动；集成后的类型、边界及 Canvas 交互另验。
- 同一旧会话 Tauri 冷启动及刷新：失败卡片可见，无无效重试/运行状态；独立 SQLite 对账确认
  原 1 个 failed Turn、12 events、0 effects，原 Turn/event/message/effect 行完全不变，Snapshot
  content/envelope、绑定和画布 nodes/connections 哈希不变。实际点击重试 1 次、模型步 0、媒体 0。
- 证据：`windows/w04-mm-retry/{01,02,03-before,04-after,05-final,06-merged}/`；截图、原备份、
  构建 hash/patch、只读对账脚本均在仓库外。原 Skill 失败及旧错误分类仍按历史保留；本批关闭
  失真显示/无效按钮缺陷，不把旧任务改记成功，也不代表完整 AMUL/恢复矩阵或 macOS 通过。

### W03 新建大小写别名（基线 `875558993`）

- WIN-005、FILE-028/032 子断言：初次同批创建 `Report.txt` / `REPORT.TXT`，第二项失败但
  第一项已发布并保留（published/retained_created `[0]`）。原失败见 `01-before/new-alias.log`。
- 准备期逐段比较目标，并读取实际父目录的 Windows case-sensitive 标记；新父目录取根内最近
  现存祖先的继承规则。不同名称仍按真实目录规则区分；查询失败不猜测，目录查询不越过绑定根。
  中间消失根反例及修正另存 `02-after/missing-root-before.log`，不覆盖首次结果。
- Windows 非管理员原生夹具：ASCII/重音 Latin 别名、父目录别名、祖先关系两种顺序全部零写入；
  case-sensitive 目录及其新建子目录的不同大小写文件保持不同文件身份，混合父目录标记也正确。
  对照 [Windows 目录规则](https://learn.microsoft.com/en-us/windows/wsl/case-sensitivity)，并保留本机
  `fsutil` 探针与 token 检查，未修改系统设置或提升权限。
- `windows_workspace` 19、`path_safety::tests` 20、`service::tests::agent_` 11，**50 项通过**；
  diff 检查通过。证据 `windows/w03-case-alias/{01-before,02-after,03-final,native-probe}/`。
  未覆盖大小写标记/目录并发置换、其他文件系统或全部 Unicode 等价组、正式 UI、macOS；无模型调用。

### W03 原生 ACL 与递归删除（基线 `39d8e741e`）

- FILE-020/028/037/038、LIFE-006/007 的 owner/回执子断言：先通过文件拒写、父目录拒新建、
  文件拒删、DACL/命名流保留；扩展到非空目录后，根 DELETE 与父 DELETE_CHILD 都拒绝时仍先
  删掉子文件再报错。随后 owner/host 反例确认递归中途失败被当普通失败结算，未保留未知效果 fence。
- 修复：Windows 在遍历前打开目标 DELETE handle 验证原生删除权限；递归中途失败或任务异常
  明确返回删除结果未知，清除旧文件列表缓存、允许诊断重读，保留 durable pending，禁止盲重试。
  未忽略 ACL 错误、未提升权限或补 grant；恢复测试 ACL 后，同 key/新 key/其他写仍被拦截。
- 最终 `windows_workspace` 24、`remove_entry_sync` 3、App `agent_wave2_host::tests` 33，
  **60 项通过**，diff 检查通过。已知根拒删零子项删除，中途拒删可枚举剩余内容；重启后的
  fence、只读诊断、成功 write/patch 的原 DACL 与命名流分别有独立断言。无模型调用。
- 证据 `windows/w03-acl/{01,02-final,03-partial-before,04-after,05-final}/`；名称为 `02-final`
  的首次目录失败原样保留，owner/host 首次分类失败另记。夹具目录清理后无残留，权限仅改临时对象。
  未覆盖权限/路径并发置换、进程强杀/磁盘 IO fault、完整 UI、跨卷及 macOS；不计完整 Case PASS。

### W06 checkpoint / 执行 lease（基线 `ea8e6a14f`）

- LIFE-009/010/011/013/014/020 的 Store 子断言：事务注入失败回滚、损坏拒绝、checkpoint 重开、
  取消/失败不复活、旧模型/工具/观察/checkpoint writer 拒绝、未决效果禁止自动恢复。
- 补上磁盘 SQLite 的两个独立 Store 连接竞争恢复夹具：只有一个获准；另一个收到明确 fence/lease
  拒绝。原 Store 的旧模型/工具写入不推进 cursor；关闭全部连接再重开，winner、旧 writer 拒绝、
  checkpoint digest 与 cursor 保持一致。本批未发现新生产缺陷。
- `native_execution_tests:: -- --skip native_pause_tests` **11/11**（含新磁盘项），
  `native_checkpoint_tests::` **8/8**，共 **19**；磁盘项另单跑通过，不重复计数，diff 检查通过。
  证据 `windows/w06-checkpoint-lease/{01-baseline,02-disk}/`；`01-baseline` 构建已包含新增测试。
- lease 过期是隔离 DB 的明确故障注入，两个 Store 在同一测试进程；未模拟真实经过时间。
  未覆盖两个 OS 进程竞争、正式 Tauri 强杀/恢复、完整 pause/resume、Job/浏览器清理与 N3/LONG；
  无真实模型调用，不计完整 Case 或 macOS PASS。

下一步：W03 跨盘/并发与剩余 ACL/VCS/SSH；W04 其余真实停止/恢复与扩展；
W05 条件资源和 W06 原生故障按夹具推进。GEN 完成报告保持开放，不重建 2,374 行日志/状态文件到 Git。
