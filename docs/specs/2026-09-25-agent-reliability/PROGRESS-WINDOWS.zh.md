# Windows Case 处理进度

更新：2026-09-29。当前宿主 Windows。执行顺序见 [实施计划](IMPLEMENTATION-PLAN.zh.md)。
覆盖 675 个共享 + 82 个 Windows 专属 Case；按适用 Agent 展开为 2,374 槽。
Agent 槽数：GEN 600、COD 594、PAL 323、MM 567、CS 242、HOST 48。
共享 P0 已由 `85a079fc0` 提交/推送；W01 首批组件走查完成，不重复维护共享根因文本。
当前先处理共享 Case 与关联 Windows 问题，Windows 专属余项排在共享阶段之后。

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
| D01 | 425 | W02 | 正式 Session/模型/工具面，中文参数、必填拒绝、版本冻结 | REG-005/008 发现混批已修；完整模型/注册矩阵待走查 |
| D02 | 302 | W02/W04 | 计划/完成、日志与 UI 终态；用户纠正/停止；零红色正向任务 | GEN 文件/产物/删除场景 N3 通过，历史失败保留；完整控制/观测矩阵待验 |
| D03 | 145 | W01/W03 | Win 路径、共享锁、原子 write/patch/delete、Artifact | 路径、原子写、读取及写入父目录置换子断言已验；最终文件/临时文件、枚举/删除竞态待验 |
| D04 | 476 | W01 | executable/args/cmd、PowerShell/cmd、编码、Job/ConPTY、退出与清理 | 新基线首批组件验证通过；完整 CMD/终端矩阵待走查 |
| D05 | 69 | W03 | 本地隔离 Git remote、SSH 夹具；取消/未知副作用 | local/file push owner 9 项通过；SSH 及完整产品路径待准备 |
| D06 | 247 | W04/W05 | WebView2 profile、Computer A11y、MCP/Plugin/Skill | Skill 首发已回归；其余待走查 |
| D07 | 124 | W02/W04 | 精确产品目标、Knowledge/Companion/Canvas/Customer owner | PAL/MM 新 Tauri 入口已走查；修复画布名称上下文遗漏 |
| D08 | 103 | W02/W04/W05 | GEN/COD/PAL/MM/CS 分角色的正式任务 | GEN 单场景 N3 通过，四角色历史证据保留；完整集合与 CS 待走查 |
| D09 | 375 | W06 | Job 子孙进程、崩溃、恢复、撤权、并发与 LONG | checkpoint/lease 19、取消计数 14 项组件已验；其他宿主故障与 soak 仍待 |
| D10 | 33 | W01/W06 | WIN-001～018 与 PORT，盘符/路径/共享锁/宿主终态 | WIN-002/007/008/010/011 的部分组件断言通过；其余待走查 |
| D11 | 75 | W01/W03 | 越界路径/junction、旧授权、无资源与跨 owner 拒绝 | AUTH-009/010 的 cwd、读取与写入父目录子断言通过；其余 owner 与竞态待走查 |
| **合计** | **2374** | W01～W06 | 结果按 Case × Agent × Windows 独立判定 | 未跑不算通过 |

## 近期可领取任务

| 任务 | 对应 Case / 断言 | 测试与修复安排 | 状态 |
| --- | --- | --- | --- |
| W01-A 命令形状与启动 | G0-003/004/010；PROC-001～004；WIN-008/009；CMD-131/133/147 | 验字面 argv、显式脚本、混合形式拒绝、PowerShell 初始化及退出状态；PATH 上 Bun 也通过 owner 实际启动 | 组件子断言已验证；持久/脱离命令 policy 和新构建 Tauri CMD 仍待验收 |
| W01-B 路径与权限边界 | AUTH-009/010；FILE-021；WIN-002～007/017 | cwd junction 已验；文件 owner 原始分段、原子 write/共享锁、删除链接及长路径回归 | B1/B2/B3 分别 80/57/37 项定向回归通过（重叠不累加）；跨盘/ACL/并发置换及正式 UI 待验 |
| W01-C Job 与终端清理 | PROC-027～036/045；WIN-010～012/014/016 | 实际 Job 子孙清理、leader 先退出、stdin/EOF、ConPTY resize/cancel/快速退出/UTF-8 分片 | 首批组件子断言已验证；代码页、模拟锁、应用强杀/重启仍待走查 |
| W02 正式核心入口 | AGEN-001/014/017；ACOD-001/008；APAL-001；AMUL-001 | 新 Session/冻结快照；文件、产物、删除、伙伴身份及画布名称由 UI/磁盘/DB 对账 | GEN 文件/产物/删除场景 N3 通过（8/9/8 步）；MM 名称已修，其余组合待验 |
| W03 文件/Git/SSH | D03/D05/D11 剩余适用 Case | 文件准备期/错误归因、大小写、ACL/部分删除、隔离 local/file remote；其余逐簇推进 | 文件首批 32、大小写 50、ACL 60 项（重叠不累加）、Git push owner 9 项通过；并发/跨盘、SSH 与完整 UI 待验 |
| W04 UI/扩展与恢复 | REAL、OBS、D06；旧 MM retry | 旧 MM 历史/重试、命令取消及原生暂停显示已修复；不重写旧 Snapshot | 旧会话冷读、COD 停止、GEN 暂停/结束回合已复验；其他恢复/扩展待验 |
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

### W04 真实进程停止与取消投影（基线 `bfb7b515b`）

- REAL-010/COD、OBS-008/014、LIFE-018/019 子断言：两个独立 data/work/profile 均经正式恢复，
  从 Tauri 新建 Coding Session，使用既有加密 StepFun Plan / step-3.7-flash，同提示执行只写
  started PID 标记的 120 秒 Bun 脚本；运行中点击正式停止。首次 native cleanup 正确，但 UI
  只显示“已运行 bun”，详情遗漏取消状态与已产生的输出，记 `FAIL_VISIBLE_UX`，原证据保留。
- 根因/修复：取消可先于 Runtime ToolCompleted，历史读取漏掉已提交的 host settlement。
  现在按相同 Turn/call 读取有界宿主结算；仅原生 process 且 cleanup reaped 的取消标记为
  canceled，其他错误与清理未确认仍为 error。前端保留该终态、丢弃成功 artifact 资格，迟到
  completed 不覆盖取消；canonical Turn 元数据控制历史标题，停止确认后刷新已提交历史。
- 首次后端 1 FAIL、UI 模型 2 FAIL、视图 1 FAIL 保留；修复后 UI 106 + 视图 5、后端历史
  owner 4 + wire 3，共 **118 项通过**；typecheck、desktop-ui-boundary、Tauri 构建通过。
- 原取消 Session 冷加载：显示“已取消执行”“已取消 bun”与 STARTED 输出；独立比较数据库
  确认原 events/turns/messages/effects、绑定及全部 Snapshot content/envelope 均未变。
- 新 Session 同提示实时回归：无手动刷新也显示取消和部分输出；Turn cancelled，process
  receipt cancelled/reaped，实际 PID 消失、finished.txt 不存在、源脚本/兄弟文件哈希不变；
  取消后无新增模型/工具执行。两次分别 **2 模型步、1 个进程**，清理 1165/1175ms；不重复计冷读。
- 证据：`windows/w04-stop/{01,02-before,03-after,04-final,05-ui-after,06-live-after}/`。
  未覆盖复杂子孙树、强杀/重启、其他角色、全体恢复矩阵与 macOS。统计残余：两次 Runtime
  `turn_cancelled.model_steps` 都是 0，而事件链各有 2 个 model step；本页按事件链计数，W06 继续核对。

### W06 取消模型步数（基线 `cdd02dea3`）

- LIFE-018/019、取消统计子断言：W04 两个真实样本各启动 2 个模型步，SDK 外层取消分支却
  固定返回 0，驱动 Future 被丢弃后已记录进度也丢失。补模型 open 后取消反例，首次 `[0] != [1]`。
- SDK 按 Turn 保留驱动经 host 记录的模型进度，在关闭输出时冻结，并用于清理后的真实终态；
  迟到记录不能改已关闭 Turn，也不能计入下一 Turn。驱动内部取消 fallback 同样保留已知步数。
- `nomifun-ai-agent --lib unified_runtime::tests` **13/13**、`engine_sdk::tests` **1/1**：
  准备阶段 0、模型已开始 1、连续两步后取消 2；原 cleanup-before-terminal、异常/会话释放与
  迟到写入边界保留。diff 检查通过，证据 `windows/w06-cancel-metrics/{01-before,02-after,03-final}/`。
- 本批使用确定性模型端口，无新增真实模型调用。历史两个 0 不回写；修复后的真实 StepFun
  计数、强杀时私有 terminal 缺失、完整跨重启统计与 N3/LONG 仍未验证。

### GEN 完成证据：owner 路径（基线 `a709e3a94`）

- CTRL-007、WIN-005/007 的证据身份子断言：先复现不相关中文文件变动使观察失效；同时补
  模型参数别名与真实目标相同的反例。read/write/patch/delete 回执新增有界 owner 路径观察，
  Runtime 使用实际路径与根摘要判重；Windows 使用原生解析后的大小写，其他宿主仍保守比较。
- 不把模型参数、缺失/畸形/跨根元数据当作身份；失败、不透明效果和未决进程仍使证据失效。
  历史 epoch 不重标，旧回执不补造身份，元数据也计入观察窗口和双层 JSON 文本预算。
- Runtime **140**、Windows owner **26**、文本分页 **1**、App host **33**，共 **200 项通过**；
  含真实 junction/大小写/中文别名、不同文件、分页 wire 预算及 owner 到回执的字段对账。
  首次失败与旧模拟回执缺少身份字段的中间失败保留在 `windows/w07-gen-completion/01-investigation/`
  和 `02-owner-paths/`；最终证据 `04-owner-paths-verified/`。本批无模型调用。
- 未覆盖：并发原生重命名/替换、全部文件系统与 macOS；Artifact 观察有效期、最终答复遗漏及
  provider 实际传输仍继续核对，原三个 GEN FAIL 不改记 PASS。

### GEN Artifact 证据与发送边界（基线 `fb96eac26`）

- CTRL-007、AGEN-014/017 子断言：首次反例确认 Artifact 发布也会误使源文件证据失效。
  产物 owner 现在返回绑定根摘要；Runtime 只延续同根受保护产物及确定未修改的文件观察，
  跨根/畸形/失败/不透明效果仍失效。最终答复字段明确要求包含用户所需细节，仍原样交付 summary。
- Runtime **142**、Artifact **14**、App host **33**、Broker Schema **1**、诊断脱敏 **1**，
  共 **191 项通过**；前端及 Tauri 构建通过。首次证据失败、测试夹具缺 tool choice 和诊断依赖
  缺失的中间失败均保留在 `windows/w07-gen-completion/05-artifact-before/` 至 `10-tauri-build/`。
- 正式 Tauri 新 GEN 使用原提示、加密 StepFun Plan / step-3.7-flash，**6 个模型步骤**；
  4 项效果 returned，独立字节/产物 hash/临时文件不存在均正确，0 owner/工具预检错误。
  仅显式开启的 debug 发送记录确认有效路径及最终答复说明进入实际请求；不记录消息或凭据。
- **本次仍 FAIL**：第 6 步发生 `EXECUTION_MODEL_PROVIDER_UNAVAILABLE`，native pause 已持久化且
  cleanup 已确认，UI 却持续显示运行；刷新后也没有暂停原因。未产生最终答复，不能计正向 PASS。
  旧 canonical 行及 Snapshot content/envelope 未变；后台 updater HTTP 错误另有记录，与 Agent
  暂停原因分开。证据 `11-live-01/`；`12-live-02/` 只准备了夹具，尚无新模型调用。
- 原三个 GEN FAIL 保留；最终答复完整性、N=3、其他角色和 macOS 尚未通过。新暂停显示问题
  转 W04 优先复现/修复，不把这次暂停改成完成或自动重放已有副作用。

### W04 原生暂停显示与结束回合（基线 `96de43d68`）

- OBS-008/014、LIFE-015/019 子断言：W07 首次真实暂停后，前端未订阅 `turn.paused`，状态复核
  将暂停一直视为未知；热态转圈不停止，冷读没有原因。新增复核反例首次 1 FAIL，原现场保留。
- 暂停通知只触发 canonical 重读；精确回合的暂停停止活动显示并保留队列阻断，不发完成事件、
  不触发最终文本后处理。主面板/侧栏明确暂停，只有已知公开原因码进入投影；补结束回合入口，
  复用既有 cancel 与释放确认，不自动 resume 或重复效果。
- 定向 UI/状态 **97**、后端投影 **2**，共 **99 项通过**；typecheck、desktop-ui-boundary、
  i18n 与 Tauri 构建通过。首次未使用 DOM preload 的 4 个夹具失败及未使用 import 的类型错误
  保留，修正命令/源码后另记结果，未降低断言。
- 原暂停 Session 经正式备份/恢复后 Tauri 冷读：原因明确、编辑禁用、无运行状态；0 新模型步，
  原 308 events、Turn、effects、Snapshot content/envelope 不变。点击结束后为 cancelled，新增
  1 条取消事件；原事件和 4 项 returned 效果不变，文件/产物字节正确、临时文件不存在，其他会话未变。
- 证据 `windows/w08-pause/{01-before,02-after,03-final,04-cold,05-hover-final,06-final-cold}/`。
  热态由实际 React hook 的暂停通知/重读回归覆盖；新模型故障的正式 UI 热态、恢复预算/授权入口、
  复杂 owner 清理、其他角色与 N3/LONG 仍待验。本批不把原 GEN 失败改记成功，无新增真实模型调用。

### GEN 本批真实回归结算

- `w07-gen-completion/11-live-01`：6 步，供应商不可用后暂停，未交付最终答复；暂停显示另由
  `8c451e155` 修复，原运行仍 FAIL。`12-live-02`（`8c451e155`）：8 步、0 工具错误，4 项效果
  returned；完整答复、原生文本打开（UTF-8/LF）、文件侧栏及独立字节/产物/删除对账均通过。
- `13-live-03`（同构建）：10 步；第 2/5 步分别将 ToolSearch 与 write/delete 混批，产生
  4 项预检错误、0 错误 owner dispatch；随后完成 4 项效果，最终答复细节也完整。记
  **FAIL_RECOVERED**，不能以最终产物正确覆盖两次混批。实际请求已经含明确的 ToolSearch 批次约束。
- 本批上限 3 个新回合/36 步，实际 **3 回合/24 步，1 次场景通过、2 次失败**。历史 3 次 GEN
  失败、首轮暂停以及本次混批均保留；原有 Session/快照未改写。N=3 仍未通过，不计完整角色集合。
  S-D02 的完成引用/最终答复修复已有真实正向证据；当前新根因为 S-D01-01 工具发现混批。

### W02 ToolSearch 请求控制与 GEN N3（基线 `50586dcc7`）

- REG-005/008、CTRL-007、AGEN-001/014/017 子断言：互斥说明已经送达，但请求缺少调用数量
  控制。首个确定性反例为请求字段 null，期望 false。原批次零 dispatch 拒绝断言保持通过。
- 原生模型输入增加可选并行偏好；仅 ToolSearch 与其他工具同在当前工具面时请求单调用。
  OpenAI/Responses 与 Anthropic 系协议携带相应字段；普通只读并行、调用方原偏好和旧输入
  默认行为保留。Gemini 适配器无对应字段，所有协议仍执行现有整批预检，不依赖供应商守约授权。
- Runtime **142**、Broker **35**、HTTP executor **9**、SDK **13**，共 **199 项通过**，Tauri
  构建通过。首个反例及回归测试缺命名空间的中间编译失败分别保留在 `01-before/`、`02-after/`。
- 同构建 `2c9c4197d030…`、原提示、加密 StepFun Plan / step-3.7-flash，三个新隔离 Tauri
  Session 为 **8/9/8 步，共 25 步**（上限 3 回合/36 步）。25 个实际请求均携带 false，均 HTTP
  200，每模型步一个调用；0 预检/owner 错误，12 项效果 returned，三个 Turn 均 completed。
- 独立核对产物工具回读的完整字节、源文件/产物 hash、临时文件不存在及最终答复四项细节；
  UI 过程、文件侧栏和原生文本打开（UTF-8/LF）一致。原 Session/events/effects/Snapshot 未改写。
  该 GEN 场景 **N3 通过**，W02/W07 的所有历史失败保留。
- 证据 `windows/w09-discovery-batch/`。模型计数按 model_step_started 对账，宿主 step 0
  指令读取另记；初次统计误含 step 0 的派生结果保留。其他角色/供应商、完整 Case 组合、macOS
  与 LONG/99% 统计尚未覆盖，不能把该场景外推为 2,374 槽全部通过。

### W03 Artifact 未知结果归因（基线 `620045889`）

- FILE-032/039、OBS-016、LIFE-006/007 的 owner/结算子断言：错误中的普通路径只要含
  Artifact unknown 标识就被误判，补丁在零发布时也返回未确认结算。分类、原生临时名冲突、
  host 补丁目录反例均先失败；测试误用状态枚举的编译失败另保留，未改生产代码掩盖它。
- 只识别 owner 的 Conflict 类型和完整标识前缀。目录/原文件/不属于本次操作的临时文件字节
  不变；失败正确结算 rejected，重开 host 后新的合法写入成功。真实发布后身份置换导致回滚
  无法确认的错误仍为 unknown，已有 pending/restart/新 key 拒绝回归保持通过。
- Artifact owner **15**、临时文件冲突 **1**、App host **34**，共 **50 项通过**；diff 检查通过。
  证据 `windows/w10-artifact-outcome/{01-before,02-after}/`。无新增模型调用；只计组件子断言，
  完整 UI、更多原生 IO 故障/并发置换、其他平台仍未覆盖。

### W03 内容读取的命名空间置换（基线 `d1abc7cce`）

- AUTH-009/010、WIN-007、FILE-018 子断言：用确定性 hook 在校验后替换父目录 junction，
  读取后恢复原目录；首次返回根外夹具的 6 字节并成功。另一反例在读取中替换同名文件，旧实现
  也返回成功。两个首次 FAIL 的日志、原目录及 observation 均保留在 `w11-read-races/01-before/`。
- Windows reader 持有受限根目录句柄，逐段打开已解析目标且不跟随新链接；核对实际句柄路径，
  并在返回前比较当前命名目标与已读文件身份。根/父目录换链、转入 `.nomifun`、父目录移到根外
  均不能提供根外内容；同名文件替换返回冲突，保留新文件且允许随后显式重读。
- 目录仅申请元数据访问，不要求列举权限、不改变文件 sharing。合法根/根内 junction、中文、
  大小写目录和长路径保持通过。旧 Agent 全文入口复用受限读取及 8 MiB 上限；普通宿主读取入口
  保持原合同，非 Windows 路径保留特殊文件的打开前检查。
- reader **6**、原生句柄 **2**、Windows workspace **28**、Agent 文件 owner **11**、App host
  **34**，共 **81 项通过**，diff 检查通过；最终日志见 `w11-read-races/05-final/`。无模型调用。
- 只计内容读取的组件/原生子断言；目录枚举、写/patch/delete 的置换窗口、跨 host/重启的根身份、
  网络卷及其他 reparse 类型、正式 UI/macOS 仍待验。不将读取修复表述为全部文件竞态已关闭。

### W03 写入父目录的命名空间置换（基线 `a35ab3ae7`）

- AUTH-009/010、WIN-007、FILE-019/020/024/025/028 子断言：准备目录后、发布前将父目录换成
  根外 junction，write、source-guarded patch、新建三项首次均修改了根外文件。首次及元数据
  句柄初版仍失败的现场分别保留在 `w12-write-races/01-before/`、`02-after/`。
- 准备结果现在持有目录保护直至阻塞 owner 发布/清理结束；创建前核对目录身份，只申请本次
  创建已需要的权限。独占的空临时文件按关闭删除，使父目录不能在发布中被原地改成 junction；
  在保护文件创建前已经改链则拒绝，根外无新增文件。原生 ReplaceFile 的 ACL/ADS 语义保留。
- 新 ACL 反例首次失败：中间目录的拒写 ACE 一并拒绝原打开方式申请的 SYNCHRONIZE。
  路径解析改用 NtOpenFile 的元数据访问，不申请列举/遍历或更改 ACL；读取复用同一方式。
- 文件 owner/读取/patch **33**、Windows 原生 **30**、App host **34**，共 **97 项通过**；
  含共享锁、ACL/命名流、长路径和失败结算。祖先 POSIX rename/原地改链/句柄释放另重复 20 次
  通过，不重复计数。原 POSIX 夹具漏 NUL 的失败及修正记录保留，生产断言未放宽。
- 补充目标父目录拒绝 S 的夹具：原地写入对照不能证明原子发布应成功；直接 ReplaceFileW/硬链接
  同样被拒绝。初始错误正向前提及试验保留在 `06-parent-access/`，按原子合同验证拒绝、原件/事件
  不变与释放后的原生成功；不记正向 PASS，也不降级为截断写入。未采用该试验中的路径解析 fallback。
- 最终日志见 `w12-write-races/05-regression/`、`06-parent-access/`，diff 检查通过；无模型调用或新增 Tauri UI 验收。
  最终文件名/发布临时文件置换、枚举/删除、跨重启根身份、网络卷及其他平台仍待验；仅关闭本批
  父目录子断言，完整 Case、100 seed、全矩阵 N3/LONG/99% 不计通过。

### 共享 FILE 清理与部分失败恢复（W13，基线 `98ba50795`）

- S-D03-09：临时名、备份名被替换后清理误删两项首次 FAIL；原件备份被换名后错误恢复另首次
  FAIL。原文件/并发文件及 observation 分别保留在 `w13-publication-entries/02-before/`、
  `05-restore-before/`，不覆盖历史失败。
- 记录创建/替换时的文件对象；Windows 清理和恢复按完整文件 ID 核对，用句柄执行，恢复目标
  不覆盖；身份变化保持 unknown。共享逻辑记录临时文件已被发布消耗，后续错误不再按旧名清理。
- owner/发布/原生句柄 **31**、Windows workspace **30**、App host **34**，共 **95 项通过**；
  覆盖并发目标、旧读者、只读、ACL/ADS、共享锁、失败结算。最终日志 `07-verified/`，diff 通过。
  删除等待旧句柄的中间失败已修；POSIX rename 夹具原先误预期能越过 deny-delete，原生证明被
  sharing error 32 拒绝，夹具增加释放后的成功对照。相关中间结果均在 `04-regression/` 保留。
- 无模型调用或新增 Tauri UI 验收。ReplaceFile 的最终目标/暂存源发布窗口尚未关闭；文件 symlink
  夹具在当前普通权限下返回 1314，未提权。其他 reparse/文件系统、Unix 清理与恢复仍需原生验证。

### 共享 FILE 新建发布的暂存源（W14，基线 `6c63f3038`）

- S-D03-10 / FILE-019/028/038/039：暂存文件普通改名、POSIX 替换、原地改写三项首次均发布错误
  字节；日志、原文件和 observation 保留于 `phase-2-3/2026-09-28/windows/w14-create-publication/01-before/`。
- Windows 新建分支持有原创建句柄直至原子不覆盖重命名/同步结束，拒绝外部写入和名称置换；
  失败清理使用同一句柄。并发目标不覆盖，成功后释放句柄，后续原生编辑正常；公共根因见 S-D03-10。
- 发布/owner/清理 **29**、Windows workspace **30**、App host **34**，共 **93 项通过**；新增
  4 项以同一二进制重复 **20/20**，不重复计数。并发目标含文件、目录、junction；现有 ACL/ADS、
  共享锁、长路径、部分失败/恢复与 unknown fence 保持通过，diff 检查通过。
- 证据 `02-after/`、`03-regression/`、`04-repeat/`；首次 Cargo 多过滤参数位置错误另保留，修正
  命令后才运行回归。无模型调用或正式 UI 验收；既有 ReplaceFile 目标/暂存源窗口、Unix 清理、
  100 seed、完整 UI/角色、N3/LONG/99% 门槛仍待验，不计完整 Case 或共享阶段完成。

### 共享 FILE 补丁来源检查（W15，基线 `dc2c6510f`）

- S-D03-11 / AUTH-009/010、FILE-018/025/026/031：预检和内容匹配各复现临时父目录换链、读取中
  同名替换两项 FAIL；根内实际内容不匹配时，旧预检仍能借根外文件通过。四个首次现场、hook 补丁
  和日志保留于 `phase-2-3/2026-09-28/windows/w15-patch-source-reads/01-before/`。
- 预检及回滚使用当前授权的受限 reader；发布前匹配在已保护的父目录下读取，核对身份和有界字节。
  读取失败或来源变化保留冲突/跳过恢复，不移除 source guard，不自动重试。
- 来源/reader/patch **31**、Windows workspace **30**、App host **34**，共 **95 项通过**；新增
  4 项同构建重复 **20/20**，diff 检查通过。现有整批预检、恢复、ACL/ADS、长路径和重启 fence 保持通过。
- 最终证据 `02-after/`、`03-regression/`、`04-repeat/`。无模型调用或正式 UI；源检查结束到
  ReplaceFile 的窗口、Unix 原生行为、枚举/删除、100 seed、完整角色及 N3/LONG/99% 仍待验。

### 共享 FILE 普通文件删除身份（W16，基线 `53b0ebe92`）

- S-D03-12 / FILE-034/035/036/037/038：删除访问检查后普通改名和 POSIX 改名均导致并发文件被
  误删。两项首次 FAIL、原件、observation 和 hook 补丁保留于
  `phase-2-3/2026-09-28/windows/w16-file-delete/01-before/`。
- Windows 普通文件改为检查并持有 DELETE 句柄，拒绝名称置换，以句柄删除且保留只读属性约束；
  只申请元数据权限，不新增内容读取或父目录列举要求。递归删除仍保持原有 unknown 分类。
- 删除 owner **5**、Windows workspace **32**、文件管理 **6**、App host **34**，共 **77 项通过**；
  新增 2 项竞态及 2 项原生 ACL/旧读者检查各重复 **20/20**，diff 检查通过。拒删无事件、成功一次
  删除事件、旧读者原字节、同名新文件保留、原生拒 RD 删除对照与重启 fence 均通过。
- 最终证据 `02-after/`、`03-regression/`、`04-repeat/`。无模型或正式 UI；打开句柄前的父目录/
  目标置换、目录递归、既有文件发布、Unix、100 seed、完整角色及 N3/LONG/99% 仍待验。

### 共享 FILE 受限递归删除（W17，基线 `385084d85`）

- S-D03-13 / AUTH-009/010、FILE-034～039：父目录换链分别导致根外文件/目录被删，普通/POSIX
  改名分别使递归删除误删并发目录，四项首次 FAIL 原样保留于
  `phase-2-3/2026-09-28/windows/w17-delete-traversal/01-before/`。
- 目标沿授权根句柄逐段打开；递归枚举和删除均保留目录/子项句柄，嵌套 junction 只移除链接。
  空目录直接按句柄删除，不额外要求列举；部分失败继续 unknown，保留缓存失效及重启 fence。
- 中间使用 ReOpenFile 获取目录游标返回访问拒绝，4 项正常删除失败保留于 `02-after/`。
  `03-native-probe/` 核对后改用原生空相对名重开当前目录对象，未放开 ACL 或按路径回退。
- 删除/路径 owner **25**、Windows workspace **34**、文件管理 **6**、App host **34**，共
  **99 项通过**；新增 7 项同构建各重复 **20/20**。覆盖祖先移出根的普通/POSIX 尝试、300 个长
  Unicode 文件、多页枚举、40 层目录、junction 根外哨兵、空目录拒列举、共享锁与 ACL 部分失败。
- 最终证据 `04-native-cursor/`、`05-adjacent/`、`06-regression/`、`07-repeat/`，diff 检查通过。
  无模型/UI；其他文件系统与 reparse 类型、Unix、进程强杀/IO 故障、100 seed、完整角色及
  N3/LONG/99% 仍待验。普通枚举和既有文件发布竞态单列，未声明共享阶段完成。

### 共享 FILE 指令目录枚举（W18，基线 `4aac7540d`）

- S-D03-14 / AUTH-009/010、FILE-011/012 递归子断言：工作区根和子目录短暂换成根外 junction 后
  恢复，旧 owner 均返回 `entries_scanned=13`、`complete=true`，根内原目录实际为空。两项首次
  owner FAIL、完整 observation 和磁盘现场保留于
  `phase-2-3/2026-09-28/windows/w18-directory-scope/02-owner-before/`；`01-before/` 的夹具 UUID
  不合法错误另保留，修正夹具后才命中产品反例，独立对账为 `first-failure-oracle.json`。
- 递归指令扫描接入受限目录 reader；Windows 在返回条目前核对目录身份，持有拒绝删除共享的
  列举句柄，以同一原生游标获取名称/类型。复用 W17 游标，未改授权、Schema 或扩大读取范围。
- 目录/路径 owner **31**、Windows workspace **34**、App host **34**，共 **99 项通过**；新增
  5 项同构建各重复 **20/20**。覆盖提前停止后的句柄释放、祖先普通/POSIX 改名拒绝、长 Unicode
  多页、隐藏/忽略目录指令与 junction incomplete；原递归删除/ACL/重启 fence 保持通过。
- 最终证据 `03-after/`、`04-adjacent/`、`05-regression/`、`06-repeat/`，diff 检查通过。无模型/UI；
  文件清单/搜索 walker、非递归元数据窗口、Unix/其他文件系统、100 seed、完整角色及长期门槛仍待验。

### 共享 FILE 搜索遍历与规则读取（W19，基线 `5f7403e83`）

- S-D03-15 / AUTH-009/010、FILE-013～018 子断言：校准后的库打开观察器确认正式 owner 仍读取
  绑定根上方的 `.ignore`；首次证据 `phase-2-3/2026-09-28/windows/w19-search-walk/02-owner-before/`。
  `09-child-before/` 用旧 walker 配置的组件探针确认子目录规则加载前换链会打开根外 `.ignore`；
  新 owner 同点拒用该目录，规则读取为零，明确 incomplete，原件及恢复后的正常搜索保持正确。
- 首次 manifest 误用不存在的 workspace log 依赖记在 `01-before/`；改用已有版本的测试依赖。
  根直接换链原返回 `symlink_entry`/incomplete，旧夹具要求必须抛错过严，原 FAIL 留存且不计新增
  生产缺陷；`04-compatibility/` 的大小写规则别名误拒绝另保留，按原生目录敏感性修复。
- 搜索改为受限目录帧遍历；不读取根外规则，仓库标记不探测根外或读取 gitdir 正文。规则和文本
  共用 64 MiB 读取预算，规则解析累计至多 4,096 行、每行 4,096 字节；忽略项也受扫描预算约束。
  规则读取/解析/预算失败明确 incomplete，并停止依赖它的子树；权限、Schema 与实际 match 回读规则保留。
- 文件/搜索 owner **41**、Windows workspace **34**、App host **34**，共 **109 项通过**；新增
  9 项最终同构建各重复 **20/20**。覆盖优先级/反选/嵌套仓库、无仓库和指定子目录、显式隐藏文件、
  Windows hidden 属性、大小写敏感/不敏感、BOM/CRLF/Unicode offset、全文 SHA、match limit 和规则预算。
- 最终证据 `07-regression/windows-workspace.log`、`10-child-after/`、`11-final/`、`12-repeat-final/`；
  较早回归另保留，diff 通过。无模型/UI；普通文件清单/侧栏、更多 ACL/文件系统/Unix、文件 symlink
  夹具、原生 IO 故障、100 seed、完整角色及 N3/LONG/99% 仍待验，未声明共享阶段完成。

### 共享 FILE 元数据读取（W20，基线 `ad468c2c2`）

- S-D03-16 / AUTH-009/010、FILE-011/018 元数据子断言：父目录临时换链后，旧 metadata owner
  把根内 6 字节文件报告为根外的 24 字节；非递归 instruction_scope 则把根内文件报告为目录且
  complete。两项首次 FAIL 与原磁盘/observation 保留于
  `phase-2-3/2026-09-28/windows/w20-entry-metadata/02-owner-before/`。
- 两入口接入受限元数据 reader；只读取实际句柄属性并拒绝新链接。元数据模式下的真实缺失祖先
  保留 missing，权限拒绝不转 absence；普通 text/image MIME、大小、时间、名称和合法 alias 保留。
- `01-before/` 保留测试误序列化内部 DTO 的编译失败。`04-access/`、`05-native-access/` 保留
  初版仅拒 RA 的夹具失败：独立原生调用同样成功，未形成拒绝条件。补父目录拒列举后原生错误 5、
  owner 与 scope 均拒绝；恢复顺序失败另留痕并恢复所有临时 ACL，未更改产品权限或放宽原生对照。
- 元数据/读取 owner **32**、Windows workspace **36**、元数据 API **6**、App host **34**，共
  **108 项通过**；新增 5 项各重复 **20/20**。覆盖根/文件/目录/深层 missing、非法文件祖先、
  私有目录拒绝、父目录拒 RD/S 与文件拒数据读、大小写 alias、嵌套搜索及零变更事件。
- 最终证据 `03-after/`、`06-effective-denial/`、`07-regression/`、`08-repeat/`，diff 通过。无模型/UI；
  普通清单/侧栏、既有发布、跨重启根身份、其他文件系统/Unix、100 seed、完整角色及长期门槛仍待验。

### 共享 FILE 两级目录树（W21，基线 `8982a8a11`）

- S-D03-17 / AUTH-009/010、FILE-016/018 目录读子断言：浏览目录在校验后换链、子目录在预取前
  换链均返回根外文件名，两个首次 FAIL 见 `phase-2-3/2026-09-28/windows/w21-directory-tree/02-owner-before/`。
  原生访问拒绝的非空子目录被返回空 children，第三项首次 FAIL 及现场见 `03-access-before/`。
  `01-before/` 另保留测试借用/移动 root 冲突的编译错误，未改生产逻辑掩盖夹具错误。
- 树与子目录预取改为传递原 authority 的受限游标；分类取同一目录对象的条目类型，不预取链接
  目标。子目录无法读取/分类时返回错误，避免未读到的内容伪装为空；未扩权或改两级返回结构。
- 目录/游标 owner **8**、Windows 原生/授权 **3**、目录 API **6**，共 **17 项通过**；新增
  5 项同构建各重复 **20/20**。多个授权根与显示根分离、严格 workspace 拒绝、隐藏项、私有目录
  隐藏、合法根内链接入口、拒绝根外链接和目录优先排序保持通过，无变更事件。
- 最终证据 `04-after/`、`05-regression/`、`06-repeat/`，diff 通过；仅跑直接相关检查。无模型/UI；
  扁平清单及缓存、独立侧栏列表入口、既有发布、其他文件系统/Unix、100 seed、完整角色和长期门槛仍待验。

### 共享 FILE 文件清单缓存顺序（W22，基线 `59c419857`）

- S-D03-18 / FILE-040 失效与重读子断言：失效后的旧扫描重建旧缓存、旧扫描覆盖较新结果两项
  首次 FAIL 保留于 `phase-2-3/2026-09-28/windows/w22-inventory-cache/01-before/`；真实事件订阅者
  经公开 API 在写/删事件期间读到旧列表，第三项首次 FAIL 见 `02-event-before/`。
  第一版事件夹具缺 delete 授权的拒绝另保留；仅为该临时夹具声明实际测试操作，产品授权未变。
- 缓存条目身份作为发布资格；移除条目同时撤销在途旧扫描，较新结果不会被覆盖。只在观察到
  失效时重读一次，再失效则明确冲突；取消/失败释放未发布条目，不新增永久根代数表。
  写/删事件现在先失效再发送，订阅者可读取新列表；不同根保持各自的缓存语义。
- 缓存/清单 owner **12**、清单 API **6**、写入/事件 **9**、删除/事件 **6**，共 **33 项通过**；
  新增 6 项同构建各重复 **20/20**，包括持续失效、取消清理、其他根隔离和补丁事件后的恢复。
- 最终证据 `03-after/`、`04-adjacent/`、`05-regression/`、`06-repeat/`，diff 通过；无模型/UI。
  底层扁平遍历/规则、原生 watcher 失效接线、超过 256 事件/落后 UI、根身份/容量长期治理、其他
  平台和完整 N3/LONG/99% 仍待验，不计完整 FILE-040 或共享阶段完成。

### 共享 FILE watcher 停止身份（W23，基线 `2d499042d`）

- S-D03-19 / FILE-040 watcher 生命周期支撑、AUTH-005/A13：通过原始非 canonical 路径停止已
  删除文件、已删除 Office 工作区时，旧实现均返回成功但保留监听登记。两项首次 FAIL 与登记
  快照保留于 `phase-2-3/2026-09-28/windows/w23-watch-lifecycle/01-before/`。
- 保存 owner 与请求别名到原 canonical 目标的关系；停止优先采用登记身份，不跟随新目标。
  同一活动别名改指向时返回冲突，先停止再重订；一个目标停止不影响同 owner 的另一个目标。
  原生撤销失败不再吞掉，未知失败保留登记以便核对/重试；最后一个 owner 清理所有对应别名。
- watcher owner **24**、原生接口 **14**，共 **38 项通过**；新增 4 项同构建各重复 **20/20**，
  含删除重建后的真实文件/Office 事件、新 owner 接收、共享 owner 保留、stop-all 和登记/别名清空。
- `03-adjacent/` 保留 junction 删除后残留空目录导致重建失败的夹具记录；按库语义移除空目录后
  另验。最终证据 `02-after/`、`04-retarget/`、`05-regression/`、`06-repeat/`，diff 通过。
  无模型/UI；未知 native unwatch 故障注入、迟到回调、多窗口引用语义、debounce 隔离/容量、缓存
  接线、事件丢批/落后 UI、其他平台和全量统计仍待验，未声明共享阶段完成。

### 共享 FILE Office watcher 事件隔离（W24，基线 `9288686b8`）

- S-D03-20 / FILE-040 watcher 支撑、AUTH-005：父/子工作区监听同一个 Office 文件时，全局
  debounce 抑制其中一个工作区的通知；停止重订后新 owner 也继承旧抑制。两项首次 FAIL 及
  回调观察保留于 `phase-2-3/2026-09-28/windows/w24-watch-debounce/01-before/`。
- 保留 200ms 规则，将 Office debounce 放进各 workspace 注册生命周期；共享该注册的 owners
  仍一起接收，最后 owner 停止时清理，不再积存在单文件 watcher 的全局表。
- watcher owner **27**、原生接口 **14**，共 **41 项通过**；新增 3 项各重复 **20/20**。
  固定时间的确定性回调反例及真实父/子 watcher 都通过；双方收到正确 owner/workspace，停止父
  监听后子监听继续收到新文件事件，最后登记与全局 Office 痕迹为空。
- 最终证据 `02-after/`、`03-regression/`、`04-repeat/`，diff 通过，无模型/UI。活动注册内大量
  路径的容量/TTL、尾事件补发、迟到回调、缓存接线、workspace.files/changed 队列/落后 UI、其他
  平台及完整 N3/LONG/99% 仍待验，不计完整 FILE-040 或共享阶段完成。

### 共享 FILE changed 批次队列（W25，基线 `e43923041`）

- S-D03-21 / FILE-040、PORT-012：超长相对路径让整批校验失败，正常事件与 dropped 一并
  清空；批次交付后相同路径新修改被旧 debounce 压掉。两项首次 FAIL/观察保留于
  `phase-2-3/2026-09-28/windows/w25-workspace-event-queue/01-before/`。
- 入队共用原路径合同，异常通知计入 dropped 而不污染正常批次；交付时清 debounce，只合并
  未交付批次内的重复事实。未改 schema、路径边界或扩展可接受输入。
- 队列/原生监听 **7**、事件合同 **1**，共 **8 项通过**；7 项同构建各重复 **20/20**，包含
  真实递归 FS 事件。混合反例验证 7 个无效路径 + 7 个溢出 = dropped 14，保留 256 项正确顺序，
  批内重复仍合并，交付后新事件可见；原 debounce 身份上限保持通过。
- `03-regression/` 保留 rustc `STATUS_ACCESS_VIOLATION` 崩溃，未执行测试；确认进程结束、
  保存内存/磁盘状态后，同源码以单 Cargo job 重试通过，未修改断言。最终日志
  `02-after/event-contract.log`、`04-compiler-retry/`、`05-repeat/`，diff 通过。
  无模型/UI；native rescan 标记、更多不可表示名称、整体传输预算、取消/恢复中批次消费、客户端
  全量对账、其他平台及 N3/LONG/99% 仍待验，不计完整 FILE-040/PORT-012 或共享阶段完成。

### 共享 FILE 原生事件缺口（W26，基线 `69c234a92`）

- S-D03-22 / FILE-040、PORT-012：native rescan 无路径通知完全消失；监听错误被误记为丢失
  一个事件。两项首次 FAIL/观察保留于
  `phase-2-3/2026-09-28/windows/w26-native-rescan/01-before/`。
- 可选 rescan_required 表示原生流存在未知缺口；dropped 只统计确知的本地丢弃。空批次也交付
  缺口，附重新读取工作区的提示，交付后清零；Access 的 rescan 标记及根变更同样保留。
  按分量严格编码相对路径，不可表示名称计 dropped，不再有损转换成其他名称；私有/根外路径仍过滤。
- 队列/原生监听 **12**、事件 Schema/发布合同 **4**，共 **16 项通过**；新增 6 项同构建各
  重复 **20/20**。验证 Unicode、Windows 非法 UTF-16 回调名称、已知/未知损失共存和旧四字段
  编码兼容；不可表示名称为回调注入，未声称原生文件系统创建该名称或 Unix 原生验证通过。
- `02-after/generated-contract-check.log` 保留 Cargo.lock 摘要漂移；实际锁文件最后更新于
  `ad468c2c2`，manifest 尚为 `85a079fc0`。正式 generator write 仅刷新三个文件中的六处摘要，
  随后 check 通过；未改验收记录或手填 digest。最终证据 `03-adjacent/`、`04-contract-refresh/`、
  `05-repeat/`，diff 通过；无模型/UI。native 重订、批次传输/取消恢复、完整 UI 全量对账、其他平台
  及 N3/LONG/99% 仍待验，不计完整 Case 或共享阶段完成。

### 共享 FILE 原生监听到清单缓存（W27，基线 `b7d8fdc25`）

- S-D03-23 / FILE-040、A13/A17：真实 Office 创建通知期间，清单缺少新文件；真实删除通知
  期间，清单仍含已删除文件。两项首次 FAIL/磁盘与 API 对照保留于
  `phase-2-3/2026-09-28/windows/w27-watch-inventory/01-before/`。
  `03-activation-before/` 另保留首次监听/重订复用订阅空窗前旧清单的第三项首次 FAIL。
- 监听服务显式接入文件路由使用的 FileService 弱引用；原生回调先失效再过滤/合并/发送，
  Office 注册也处理非 Office 变化和错误/rescan；共享文件监听在未知缺口时失效全部登记涉及的根。
  订阅成功撤销旧快照；使用已保存路径覆盖相交的祖先/子目录清单，不重解析已删除目标。
- watcher/缓存 owner **39**、原生监听接口 **14**、文件路由 **5**、App 监听上下文 **12**，
  共 **70 项通过**；新增
  6 项同构建各重复 **20/20**。事件接收方在发送尚未返回时通过公开 API 读取，并与真实磁盘
  核对；覆盖首次/重订空窗、原生错误/rescan 注入、非 Office 变化、目录相交与前缀同名隔离、
  文件删除后的失效和弱引用释放；App 重新编译确认正式接线。
- 修复后证据 `02-after/`、`04-regression/`、`05-repeat/`；无模型/UI。未监听时的主动刷新、
  watcher 尾事件/迟到回调、完整 UI 丢批全量对账、扁平遍历/根身份、其他平台及 N3/LONG/99%
  仍待验，不计完整 FILE-040 或共享阶段完成。

### 共享 FILE 真实 UI 文件树对账（W28，基线 `b62671c7e`）

- S-D03-24 / FILE-040、PORT-012、A17：正式 Tauri 中移出最后一个文件，根目录请求 200 后
  仍显示 last.txt；已展开 nested 清空后，根刷新仍保留 ghost.txt。两项首次 UI FAIL 的截图、
  独立磁盘清单、请求日志、DB/事件与构建保留于
  `phase-2-3/2026-09-28/windows/w28-ui-tree-reconcile/02-before-ui/`；三项组件首次 FAIL 见 `03-hook-before/`。
- 删除成功空响应的忽略逻辑；旧树只确定哪些目录需要重新读取，不再提供当前文件名。未读
  children 与确知为空的数组分开；完整刷新后再发布树，旧 root/child 请求、跨 source 与卸载
  后响应不回填，清理 loading 定时器。失败重读保持原快照并记录错误，不冒充新的空结果。
- UI hook/刷新入口/路径 mapper 共 **13 项通过**，新增 9 项同源码各重复 **20/20**；
  typecheck、desktop-ui-boundary、前端与正式 Tauri 构建通过。基线二进制 `24d056e7881a…`，
  修复版 `d8cc6ce26df2…`；源码差异及完整 hash 在 `01-baseline-build/`、`07-fixed-build/`。
- `08-after-ui/` 使用另一套隔离 data/work/profile；2560×1392 桌面窗口的根清空、已加载
  子目录清空、空目录再次新增文件共 **3 项 UI 子断言通过**，均独立比对磁盘。前后各从正式
  入口新建一个 GEN 准备回合，使用既有加密 StepFun Plan / step-3.7-flash，共 **2 模型步、
  0 effects**；没有以模型答复代替文件断言。两个测试进程已结束，正式 backup 保存终态数据。
- 自建文件删除被自动审查拒绝后，改为可逆移入证据目录并核对 hash；未执行被拒的删除。
  自动化定位/夹具路径守卫错误与首份根断言组装时间错误均留痕，补充独立核对记录，未覆盖原件。
  未覆盖 dropped/rescan/重连自动触发 UI 对账、读取失败的可见状态、100 seed、完整角色及
  N3/LONG/99% 和其他平台；不计完整 FILE-040/PORT-012 或共享阶段完成。

### 共享 FILE WebSocket 重连对账（W29，基线 `b4e5a0958`）

- S-D03-25 / FILE-040、PORT-012：恢复信号未触发任何重读，首次组件 FAIL 见
  `phase-2-3/2026-09-28/windows/w29-reconnect-reconcile/01-before/`。正式 Tauri 中断该隔离
  profile 唯一的网络子进程，期间移出 before.txt 并创建 after.txt；WebSocket 已从 conn-1
  恢复到 conn-2，主应用/浏览器/renderer 的 PID 与启动身份保持，文件树仍显示 before.txt、
  且无新的 workspace 请求。首次 UI FAIL、完整进程归属/时间线/磁盘/截图见 `02-before-ui/`。
- 会话工作区接入既有 conversation.reconnected；断线及传输层 resync 信号复用同一重读入口，
  保持 2 秒合并、末次补读和卸载隔离。未新增传输权限或改写会话资源。
- 工作区 **15**、WebSocket 恢复 **7**，共 **22 项通过**，新增 3 项各 **20/20**；包含
  服务端 resync 帧的组件校验。typecheck、desktop-ui-boundary 与正式 Tauri 构建通过。
- 新隔离 `06-after-ui/` 重复相同故障：仅网络 PID 更换，约 1 秒后 conn-2 建立，随后自动
  workspace 200 响应，文件树更新为 after.txt，磁盘/hash 独立核对通过；没有手动刷新或重进。
  修复版二进制 `b5bfd64306a5…`，构建及完整 hash 见 `05-fixed-build/`。
  前后各一个正式 GEN 准备回合，既有加密 StepFun Plan / step-3.7-flash，共 **2 模型步、
  0 effects**；两个主应用均已结束并正式备份终态数据。
- 未覆盖真实服务端队列积压/丢弃计数、连接未断时的 native rescan 通知、重复/乱序完整矩阵、
  终端及其他角色入口、100 seed、其他平台和 N3/LONG/99%；不计完整 Case 或共享阶段完成。

### 共享 FILE 读取失败可见状态（W30，基线 `90da8a8e0`）

- S-D03-26 / FILE-018/040、A08/A17：首读失败返回 []、刷新失败没有旧快照失效标记，两项
  组件首次 FAIL 见 `phase-2-3/2026-09-28/windows/w30-workspace-read-errors/01-before/`。
  正式 Tauri 将自建工作区可逆移入证据目录后，workspace 请求 400，仍显示 known.txt 且无
  失败/过期提示；首次 UI FAIL、磁盘/hash/DB/截图见 `02-before-ui/`，原内容保留。
- 失败/失效请求返回 null，搜索入口不再当作空结果隐藏输入。按来源和请求记录失败，显示
  中英告警及重试；未取得快照时不展示“项目为空”。重试保留搜索条件并重读失败目录，只有
  已验证的读取清除对应错误，其他目录成功与旧请求不得覆盖较新的失败。
- `04-retry-before/` 保留初版即时重试成功但错误标记残留的失败，已同步更新请求所见失败状态；
  `06-regression/typecheck.log` 保留 Alert 不接受 role 属性的类型错误，改用组件自带告警角色，
  可访问性断言保持。最终 **24 项定向通过**，新增 9 项同源码各 **20/20**；typecheck、i18n、
  desktop-ui-boundary、前端和正式 Tauri 构建通过。证据 `07-final-checks/`、`08-repeat/`、`09-fixed-build/`。
- `10-after-ui/` 为另一套隔离 data/work/profile：旧快照告警、重新进入后的无快照失败、恢复
  原目录后通过“重试”成功三项正式 UI 子断言通过；独立核对 400→200 与相同文件 hash。
  修复版二进制 `4fa07d3c1d74…`；前后各一个正式 GEN 准备回合，既有加密 StepFun Plan /
  step-3.7-flash，共 **2 模型步、0 effects**。两个测试应用已结束并正式备份终态数据。
- 未覆盖原生 ACL/网络/混合错误的完整 UI 矩阵、全部角色/终端入口、100 seed、其他平台和
  N3/LONG/99%；不计完整 FILE-018/040 或共享阶段完成。

### 共享 FILE 侧栏目录边界（W31，基线 `8fb877e4e`）

- S-D03-27 / AUTH-010、FILE-018：单层目录入口直接展开根外 junction，目标/祖先/根在
  canonical 校验后置换也返回根外文件名；四项首次 FAIL 与保留夹具见
  `phase-2-3/2026-09-28/windows/w31-workspace-listing/01-before/`。正式 Tauri 首次 UI
  FAIL 见 `02-before-ui/`：根外 outside-only.txt 出现在项目侧栏，HTTP 200、磁盘、DB 和截图已核对。
- 修复见共享条目。Windows 单层目录 13、句柄枚举 2，共 **15 项定向通过**；侧栏 13 项
  同二进制各 **20/20**，含根内链接、断链、私有别名及路径置换。格式/diff 检查和正式 Tauri
  构建通过，日志见 `04-regression/`～`06-fixed-build/`，二进制 `b86fddcca1b7…`。
- 新隔离 `07-after-ui/`：根外链接不可展开、根内链接正常显示 inside-only.txt、缓存目录
  转向根外后返回 403 并显示读取告警三项通过；根外名称未出现，内外 sentinel hash 未变。
  前后各一个正式 GEN 准备回合，既有加密 StepFun Plan / step-3.7-flash，共 **2 模型步、
  0 effects**；应用已结束并正式备份数据。夹具链接目标检查误用字符串索引、即时 UIA 快照
  尚未含告警的两项 harness 失败另留痕；修正检查并在 HTTP 响应后重观测，未放宽断言。
- 未覆盖 Unix 原生竞态、接纳前/跨重启根身份、权限错误完整分类、扁平清单、全部角色/终端、
  100 seed 与 N3/LONG/99%；不计完整 AUTH-010/FILE-018 或共享阶段完成。

### 共享 FILE 扁平清单与忽略规则（W32，基线 `79f559ee8`）

- S-D03-28 / FILE-016/018、AUTH-010：父目录 .ignore 误滤、根外 Git 排除规则被读取、损坏
  UTF-8 规则仍返回成功、根目录置换泄漏名称四项首次 FAIL，见
  `phase-2-3/2026-09-28/windows/w32-workspace-inventory/01-before/`。正式 Tauri 的
  `@visible` 显示空候选，但根内 visible.txt 存在且未被根内规则排除；首次 UI FAIL 与
  HTTP 200、磁盘/hash/DB/截图保留在 `02-before-ui/`。
- 修复见共享条目。清单/缓存/监听 21、搜索 9、既有同步清单 5、目录 API 6，共 **41 项
  定向通过**；20 项清单检查各 **20/20**，另一项用同一夹具验证 **20,000 / 20,001** 文件
  成功边界与显式超限各 **20/20**，无部分结果入缓存。覆盖规则优先级、根内 gitdir/commondir、
  根外拒绝、子目录置换和坏规则修正后重试；格式/diff 与正式 Tauri 构建通过。
- `07-after-ui/` 使用新隔离 data/work/profile：visible.txt 恢复可选，ignored.txt 仍被根内
  .gitignore 排除，普通根外 junction 内容不入候选三项通过；独立核对菜单范围、成功 HTTP、
  内外文件 hash 和 canonical 状态。修复版二进制 `26460c5b8969…`，完整构建见 `06-fixed-build/`。
  前后各一个正式 GEN 准备回合，既有加密 StepFun Plan / step-3.7-flash，共 **2 模型步、
  0 effects**；应用均已结束、profile 子进程为零并正式备份数据。
- 未覆盖文件候选读取失败的可见提示、根外 Git 元数据的显式资源授权入口、Unix 原生竞态、
  跨重启根身份、完整 ACL/角色、100 seed 与 N3/LONG/99%；不计完整 Case 或共享阶段完成。

### 共享 FILE 文件候选失败与重试（W33，基线 `fff100d2d`）

- S-D03-29 / FILE-018、A08/A09/A17：错误无告警/重试、加载时旧候选仍可选择，两项组件
  首次 FAIL 见 `phase-2-3/2026-09-28/windows/w33-file-mention-errors/01-before/`。
  正式 Tauri 中损坏的 .ignore 使 `/api/fs/list` 返回 409，`@visible` 却显示“搜索结果为空”；
  首次 UI FAIL、原始坏规则、文件 hash、HTTP/DB/事件/截图见 `02-before-ui/`。
- 文件读取状态独立为可回归的 hook，按会话/工作区/尝试匹配当前结果，关闭与卸载取消回填；
  同工作区不同会话也重新读取。菜单区分加载、失败与正常零匹配，复用现有中英错误文案；
  重试保持草稿焦点，不提交表单，键盘选择只使用当前成功候选。
- **12 项定向通过，各 20/20**，覆盖恢复、零结果、稳定来源不重复请求、切换、迟到成功/
  失败及卸载；typecheck、i18n、desktop-ui-boundary、前端和正式 Tauri 构建通过。
  证据 `04-regression/`～`06-fixed-build/`，二进制 `1e4a1ac6ab48…`。
- 新隔离 `07-after-ui/` 验证明确错误/重试；保留并替换坏规则后点击“重试”，HTTP 409→200，
  visible.txt 恢复为候选，原 `@visible` 未变；无需重新聚焦即可继续输入 `.txt`。原坏规则与
  文件 hash、canonical 终态独立核对通过。前后各一个正式 GEN 准备回合，既有加密 StepFun
  Plan / step-3.7-flash，共 **2 模型步、0 effects**；应用结束、profile 子进程为零并正式备份。
- 未覆盖完整 ACL/网络错误分类、实际多会话竞态 UI、其他角色/平台、100 seed 和 N3/LONG/99%；
  不计完整 FILE-018 或共享阶段完成。

### 共享 FILE 既有目标的普通写入竞态（W34，基线 `c2c843b6f`）

- S-D03-30 / FILE-020/025/026：校验后改写目标、原生替换调用前改写目标两项首次 FAIL，
  操作均成功并覆盖并发内容。日志与基线源码见
  `phase-2-3/2026-09-28/windows/w34-publication-target-lock/01-before/`。首轮 TempDir
  helper 在失败退出时清理了夹具；原日志保留，修正显式外部夹具保留方式后，相同生产反例
  再次失败并留下磁盘/观察文件，见 `02-retained-before/`，没有覆盖第一次结果。
- 修复见共享条目。最终核对通过同一目标句柄读取有界完整字节，普通写入在替换窗口被
  原生共享检查拒绝；操作结束后仍允许后续编辑。原生 ReplaceFileW 的 ACL/命名流合并、
  部分失败恢复和按身份清理继续保留，没有直接 truncate 回退或放宽错误断言。
- 发布/替换/清理、Agent patch 和 Windows 写入接口合计 **52 项去重定向通过**，包含
  deny-write/deny-delete、只读、旧读句柄、父目录权限、ACL/命名流及多文件恢复；两个首次
  反例及补充的同长度字节变更共 **3 项新增回归**，最终同二进制各 **20/20**。格式与 diff
  检查通过，见 `04-regression/`、`07-equal-length-final/`。
  空格式日志导致的统计脚本异常单独留痕，修正统计后去重，测试结果未改写。
- 本批为原生故障注入，**0 模型步、无 UI 验收**。暂存源、目标名称/重解析点置换、映射写入、
  Unix、正式 UI/全部角色、100 seed 与 N3/LONG/99% 仍未覆盖，不计完整 Case 或共享阶段完成。

### 共享 FILE 暂存来源与发布证据（W35，基线 `68a6e68ab`）

- S-D03-31 / FILE-020/038/039、A05/A07/A17：原生间隙内改写暂存字节、替换暂存身份
  （字节相同）均返回成功并删掉原件备份；未确认失败仍发送预期内容。三项首次 FAIL、完整
  夹具/字节/事件见 `phase-2-3/2026-09-28/windows/w35-staged-publication/01-before/`。
- 创建/写入期间限制暂存名与字节变化；替换前按创建时身份、期望字节核对并按句柄设置属性。
  Native 返回后重新核对已发布对象，持有读保护直到备份清理结束；不匹配或无法核实时保留
  备份并返回 unknown，分别记录已发布、暂存消耗和字节确认状态，不误清理重新占用的暂存名。
- 多文件错误新增有界未确认索引，未确认的当前文件不自动回滚，已确认前项仍按既有规则恢复。
  未确认内容不发内容更新，可能发布或清理未确认时失效缓存。进一步复现普通错误掩盖恢复/
  清理不确定性、真实暂存置换后被误分类，以及仅清理不确定时未失效缓存；首次结果分别保留
  在 `04-fence-before/`、`07-cache-before/`，已补统一 unknown 分类与缓存失效。
- 文件发布/恢复/权限及宿主协议共 **65 项去重定向通过**；**11 项新增回归各 20/20**。
  包含原件保留、同字节异身份、源归属、清理期间保护、未确认项不回滚、确认内容仍可通知、
  64 个索引完整落入 2 KiB 错误预算、重启/换 key 拒绝及已知失败不误留 fence。格式/diff
  通过，见 `05-final-checks/`、`08-final-repeat/`；宿主报告二进制来自前者，末次仅清理缓存
  条件的修改由重新编译的文件回归覆盖。
- **0 模型步、无 UI 验收**。原生间隙内仍可能发生需核对的发布，尚未证明零副作用；
  未确认发布的消费者重读通知、目标名称/映射写入竞态、Unix、全部角色及 N3/100 seed/
  LONG/99% 仍待验，不计完整 Case 或共享阶段完成。

### 共享 FILE 预览重读与事件接线（W36，基线 `a738bbc7b`）

- S-D03-32 / FILE-018/038/039/040 子断言：原生 unknown 缺通知、前端无正文事件变为 undefined、
  保存回包误清后续草稿，首次 FAIL 见
  `phase-2-3/2026-09-28/windows/w36-preview-reconciliation/03-native-before/`。
  两次 ACL 夹具未形成预期不确定性的原始结果保留在 `01-before/`、`02-fixture-correction/`。
- 基线正式 Tauri `05-before-ui/` 的单次写入进入 unknown/pending 与暂停，目标字节/mtime
  保持，但预览仍显示旧内容。首版修复 `14-after-ui/` 同样失败；追到生产宿主使用空事件
  接收器。两次 UI FAIL、独立磁盘/数据库/事件断言、模型轨迹均保留，不改成组件 PASS。
- 修复见共享条目。扩展回归还发现关闭重开未激活页签，以及取消重读后保存失败留下加载
  标记，首次失败分别保留在 `08-regression/`、`10-save-before/`；均已修复。
- 文件发布/权限、React 预览组件、宿主投递/组合共 **72 项去重定向通过**；18 项前端、2 项
  原生和 1 项宿主关键回归各 **20/20**。覆盖空正文、图片重读、读失败/恢复、逆序响应、
  编辑撤回、关闭重开/卸载、脏草稿删除保护、慢读和失败 mtime、超长代码裁剪及用户事件隔离。
  类型、i18n、桌面边界、UI 构建通过；测试使用可恢复的逐项 IPC spy，避免污染其他测试。
- 最终正式 Tauri `17-final-ui/`（二进制 `a333d51e1899…`）三项通过：mtime 不变仍按通知重读
  真实磁盘、读拒绝保留正文并告警、恢复原 DACL 后点击重试读取新正文。磁盘 hash、canonical
  unknown/pending/暂停、截图和 UIA 独立核对；前两次 FAIL 不关闭。三次均使用已有加密
  StepFun Plan / step-3.7-flash，共 **9 模型步、3 pending effects**，未重放或宣称写入成功。
  夹具 ACL 已恢复，应用已结束、profile 子进程为零并正式备份。编译栈溢出/默认构造错误，
  以及重试夹具的 ACL、UTC 解析、断言脚本失败均另存；只纠正夹具，未扩权或放宽产品断言。
- 其他预览类型/角色、丢批乱序、完整发布窗口、Unix/macOS、N3/100 seed/LONG/99% 仍待验，
  不计完整 Case 或共享阶段完成。

### 共享 FILE 跨实例与相交根缓存（W37，基线 `d7ae3dad0`）

- S-D03-33 / FILE-040 清单对账子断言：不同实例在通知时读到旧清单，嵌套写入/删除遗留
  祖先缓存，重命名遗留旧名，四项首次 FAIL 与夹具见
  `phase-2-3/2026-09-28/windows/w37-shared-inventory/02-native-before/`。
  首轮测试宏限定名编译错误保留在 `01-before/`。
- `03-before-ui/` 通过普通设置关闭 Office 自动预览，先查询 `@old`，再由真实模型创建
  new.txt；磁盘与回执成功，但重新查询 `@new` 显示无匹配。原 UI FAIL、截图、DB、事件、
  模型轨迹和两次 HTTP 200 清单读取保留；没有启动 Office watcher。
- 修复见共享条目。**31 项去重定向通过，7 项新增各 20/20**，涵盖跨实例通知时序、相交
  根写/删/重命名、读取权限、扫描中途失效、原有缓存/watcher 生命周期及正式路由接线。
  未修改 renderer；复用已验证 UI 产物，正式 Tauri 构建与 diff 检查通过。
- 新隔离 `07-after-ui/`（二进制 `cc233e17bb65…`）在同样配置下，新文件候选恢复、原文件
  候选保留，UIA 的实际选项与独立磁盘字节、canonical 终态/回执一致。前后两个 GEN 会话
  使用已有加密 StepFun Plan / step-3.7-flash，共 **6 模型步、2 returned effects**。
  应用均已结束、profile 子进程为零并正式备份；首次传输计数漏读 backend log 的记录与
  修正另存，没有覆盖原结果。
- 未覆盖无 watcher 的外部变更、原生名称/映射发布竞态、完整丢批/乱序、Unix/macOS、
  全部角色、N3/100 seed/LONG/99%；不计完整 Case 或共享阶段完成。

### 共享 FILE 替换目标确认与回滚资格（W38，基线 `e3fc5e75b`）

- S-D03-34 / FILE-020/025/038：原生调用前换走已校验目标，再放入外来文件，两项首次 FAIL
  覆盖不同内容与相同内容异身份；失败仍标记正文已确认，进入当前项回滚候选。备份身份
  清理已能保住并发文件，本批补确认状态。证据及完整夹具见
  `phase-2-3/2026-09-28/windows/w38-target-publication/01-before/`。
- 原目标句柄持续保护字节；原生成功后同时验证已发布来源/正文及备份目标身份，确认后
  才开放正文通知、回滚和备份清理。无法确认时保留 unknown、所有恢复文件及未确认索引，
  只发重读通知。私有标记改为 publication_verified，公共错误/观察格式沿用原合同。
- **56 项去重定向通过，4 项新增各 20/20**。覆盖同字节异身份、真实置换后的当前项不回滚、
  元数据通知/unknown 分类，以及既有替换、清理、ACL/命名流、write/patch 负向。
  可写映射初版夹具错误假设能进入原生窗口，两个失败保留于 `03-mapped-source/`、
  `04-mapping-observation/`；实际为共享冲突 32、零发布，关闭映射后发布成功。
- **0 模型步、无 UI 验收**。普通权限下文件 symlink 的 PowerShell 与原生 Rust 探针均失败，
  原生码 1314，见 `08-link-fixture/`；只阻断相关夹具。尚未证明原生间隙零副作用，换走后
  换回、其他映射组合、Unix/macOS、完整 UI/角色、N3/100 seed/LONG/99% 仍待验。

### 共享 FILE 补偿身份与部分失败反馈（W39，基线 `ebca6dabc`）

- S-D03-35 / FILE-031/033/039 子断言：同内容外来文件被当成己方发布回滚，首次 FAIL 及磁盘
  夹具保留于 `phase-2-3/2026-09-28/windows/w39-rollback-identity/01-before/`。修复见共享条目；
  既有对象正常恢复、同字节异身份保留、早期检查后置换和迟发错误均有定向回归。
- 首轮正式 Tauri `07-ui/` 磁盘保护通过，但普通共享冲突被报成平台不支持，模型声称零修改；
  UI FAIL 与错误分类/数量反馈两项组件首次 FAIL 分别留在 `07-ui/`、`10-feedback-before/`。
  原回执已有部分索引，没有把问题误记成所有信息丢失；测试编译/导出夹具错误也单独留存。
- **79 项去重定向通过，10 项新增各 20/20**；涵盖发布/替换/补偿、Windows 写入负向、
  错误分类、私有诊断剥离及 64 索引完整保留在 2 KiB 预算内。正式 Tauri 构建、diff 通过。
  未修改 renderer，未运行全仓测试；Unix 实现尚无本机原生验收。
- 最终 `11-final-ui/`（二进制 `73e7f90570be…`）使用新工作目录、Session 和相同提示。
  独立进程确实置换首项并占用后项，磁盘 hash 与 canonical 的 published=[0]、restored=[]、
  skipped=[0] 一致；执行失败分类和明确数量已送达模型，UI 曾正确展示部分结果。
  但模型随后违反“不重试”要求，再次调用补丁，在准备期被拒绝；人工通过停止按钮结束，
  终态 cancelled。**整体保留 FAIL_UI_CONTINUATION**，不把中间正确说明当最终 UI PASS。
- 两次均沿用加密 StepFun Plan / step-3.7-flash，共 **23 模型步、3 rejected effects**。
  最终运行 13 步，未超预先限定的 16 步；应用/夹具进程及 profile 子进程已清零并正式备份。
  完成控制与用户不重试约束转下一共享批；原生间隙、Unix/macOS、完整角色及
  N3/100 seed/LONG/99% 仍未验，不计完整 Case 或共享阶段完成。

### 共享失败回合的受阻收尾（W40，基线 `554f557ba`）

- S-D02-07 / CTRL-007、FILE-031/039 子断言：导入 W39 的真实失败并补确定性首次 FAIL，
  见 `phase-2-3/2026-09-28/windows/w40-control-after-failure/01-before/`；已派发失败补丁后
  提交 blocked 报告仍被要求重读，不能交付失败摘要。修复见共享条目。
- **35 项去重定向通过，5 项新增各 20/20**。覆盖实际失败派发、恢复后的 control batch、
  待核对状态保留、新增输入义务、成功/仅 unverified 拒绝、陈旧证据和存活进程拒绝。
  正式 Tauri 两次构建及 diff 通过；未修改 renderer，没有重复全仓测试。
- `05-ui/`（二进制 `ff6b703a9991…`）仍 FAIL：模型在 blocked 报告中引用失效成功读取及
  失败补丁，Schema 如实拒绝后转向重读和第二次补丁。补待核对状态下可直接使用的受阻
  报告格式后，`08-final-ui/`（二进制 `205b047ac4ef…`）仍重读/重试，没有有效 blocked
  报告，两次均通过正式停止按钮结束为 cancelled，首次与修复后 FAIL 分别保留。
- 同样提示、独立工作目录和并发 helper 下，外来首文件、另存发布对象与第二文件均完整；
  最终运行置换落在较晚补偿窗口，记录 rollback_failed=[0]，与预定 skipped=[0] 不同，
  该精确索引断言也保留失败，未扩大断言制造通过。完整截图、磁盘/DB/事件及模型轨迹在外部。
- 两次使用加密 StepFun Plan / step-3.7-flash，分别 **15/11 模型步、2/2 rejected effects**；
  均未超单次 16 步上限。应用、helper 和 profile 子进程清零，正式备份保存。模型错误收尾
  **仍未通过**，继续核对实际模型请求、历史/压缩及控制选择；其他角色/平台、N3/100 seed/
  LONG/99% 仍未验，不计完整 Case 或共享阶段完成。

### 共享失败交付与历史结果展示（W41，基线 `454914d8a`）

- S-D02-08 / CTRL-007、FILE-031/039、A17 子断言：确定性首次 FAIL 见
  `phase-2-3/2026-09-28/windows/w41-control-transport/02-before/`；待核对补丁的最终回答
  被再加一次模型审查。前置原有失败检查后直接失败，原文本/目标保留；正常重读后仍可
  通过证据结算。既有 wire Schema 检查通过，未证明 W40 的工具定义在传输中丢失。
- **123 项去重定向通过（Rust 41、UI 82），5 项新增各 20/20**；涵盖结束/恢复、完成证据、
  取消/steering、错误隔离、迟到文本/跨 Turn、历史游标与显示时间。类型、桌面边界、UI 与
  正式 Tauri 构建、diff 通过；未跑全仓测试。rustc 一次异常退出及相同命令复编结果另留存。
- `06-ui/`（二进制 `e02ed2a6d958…`）原提示、新 Session/隔离目录/并发 helper 下，4 模型步、
  1 rejected patch；模型选择有效 blocked 报告，回合自动 failed，未重试，待核对目标保留。
  此样本验证 blocked 路径，普通文本结束路径仍只计组件结果。磁盘完整，但 UI 只留错误
  卡片，已交付正文被折叠；初次 UI FAIL 及三项展示反例见 `06-ui/`、`08-ui-before/`、
  `12-cursor-before/`。首版 `10-cold-ui/` 仍 FAIL，历史 created_at 是游标，不能当结束时刻。
- 修复后 `14-final-ui/`（二进制 `476f797c0ea5…`）正式冷加载同一失败会话，默认折叠的
  过程区外可直接看到部分结果和失败提示。独立比较 events/messages/effects/turns 原样一致，
  0 新模型步/效果；外来文件、另存发布对象与第二文件 hash 均保持。初版断言漏计
  CompletionDelivered、把报告控制误作文件操作的夹具错误另存，原 UI FAIL 没有覆盖。
- 仅一次真实 StepFun Plan / step-3.7-flash 运行；后两次正式恢复均只读。三个应用/profile
  与 helper 已结束，正式备份保留。完整 N3/角色、plain-final 分支 UI、其他失败/恢复矩阵、
  Unix/macOS、100 seed/LONG/99% 仍未验，不计完整 Case 或共享阶段完成。

### 共享失败停止的固定三样本验收（W42，基线 `6b8d82fe2`）

- S-D02-09 / CTRL-007、FILE-031/039、A17/A18：冻结构建 `476f797c0ea5…`、GEN Revision、
  StepFun Plan / step-3.7-flash 配置及原提示，使用三个新 Session，各自独立 data/work/profile。
  预定三次均保留，未因中途失败换提示或重新取样；证据位于
  `phase-2-3/2026-09-28/windows/w42-failure-stop-n3/01～03/`。
- `01` 首轮子断言通过：4 模型步，普通文本结束，failed 且部分正文可见，无完成审查/重试；
  W41 尚缺的该分支正式 UI 证据已补到一份。`02` 首次 FAIL：5 模型步，blocked 报告使用
  `call_...` 占位 ID 被正确拒绝，修正后 failed，保留模型参数错误及 recovered 记录。
- `03` 首次 FAIL：7 模型步，错误后重读两文件，completion 被要求先更新计划，随后以三项
  supported 描述“一项已改、另一项未改”，canonical 却结为 completed。磁盘证实 second.txt
  仍为原文，UI 也只显示部分成功；这是完成判定问题，不能因事实描述正确而计 PASS。
- 三次均恰好一次带全文摘要保护的多文件补丁，原生并发置换成立，外来首文件/保留对象
  完整，第二文件 hash 未变，3 rejected effects 均可对账。应用、profile、helper 已清零并
  正式备份。总计 **6 回合、16 模型步；首轮 1/3，N3 未通过**，所有首次失败保留。
- 本批只验收及排查，未修改产品或重跑全仓测试；下一批优先处理读回事实被当成原任务
  完成证明的问题，未知/失效引用继续拒绝。完整 Case/角色、Unix/macOS、100 seed/LONG/
  99% 仍未验，不计共享阶段完成。

### 共享部分任务完成判定（W43，基线 `f201a3651`）

- S-D02-10 / CTRL-006/007、FILE-031、A17/A18：沿用 W42 `03` 的真实首次 FAIL；读回
  `first=after/second=before` 后的全 supported 报告曾把未完成任务结为 completed。修复见
  共享条目；旧 v1 状态保守迁移，恢复后未完成目标重新要求当前观察，Agent 切换继续阻断。
- **161 项去重定向通过（Runtime 160、App 1），11 项新增各 20/20**。覆盖部分发布、精确
  write 修复、已全发布的迟发失败、零发布负向、未知/未确认结果、旧状态恢复、后续范围变更、
  stale/未知引用、存活进程及预期失败命令；另将既有持久化回归重复 20/20。未跑全仓测试。
- 首次正式构建触发 rustc 1.97.1 ICE，panic 后五分钟仍占用约 2GB/持续 CPU，终止确切构建
  会话并保留日志。相同源码/feature/config 关闭增量后 6m43s 构建通过，未删除缓存或改断言。
- 最终正式 Tauri `06-ui/`（二进制 `92da26949216…`，Session
  `01a0e703-6b05-7220-87b3-aa851e88176d`）两回合共 4 模型步、1 rejected patch；并发置换
  成立，外来首文件与保留发布对象为 after，second 全文 hash 不变。错误后无任何工具操作，
  v2 状态保留两项待重读及 unresolved second，canonical failed；默认折叠 UI 同时显示失败和
  实际部分结果。应用/helper/profile 清零并正式备份。
- 真实模型本次走普通文本停止分支；W42 的误 supported-report 路径只由确定性回归证明。
  W42 N3 仍为 1/3，不重写旧失败。完整角色/平台、N3、100 seed、LONG/99% 仍未验，
  不计完整 Case 或共享阶段完成。

### 共享重启后精确修复（W44，基线 `8036f4363`）

- S-D02-11 / CTRL-006/007、FILE-031、A05/A07/A17/A18/A19：从 W43 failed Session 的原始
  正式备份恢复，固定提示要求重读两文件、只修 unresolved `second.txt`，不得再写 `first.txt`。
  确定性 completed-only 来源 FAIL、`03-ui` 的旧构建 recovery 拒绝、`05-ui` 的 history 拒绝
  （0 模型步）均保留；两层版本化只读状态现跨构建，Session/Turn/Snapshot 与记录格式仍精确校验，
  运行中 checkpoint 不放宽。`09-ui` 又保留 recovered 后首个补丁提案被空计划门禁拒绝的 FAIL：
  最终虽修复，但有 2 次 patch 调用、1 工具错误，独立判定不通过。恢复上下文刷新现只使已有
  计划要求重规划，不再把空计划变成 effect gate。
- **18 项去重定向通过，4 项新增各 20/20**；两次关闭增量的正式 Tauri 构建通过。最终
  `12-ui/`（二进制 `3ecc583caf5e…`，同一 Session）用未污染 W43 备份及独立 data/profile：
  5 模型步，effect 前重读 first/second，恰好一次只含 second 且绑定新摘要的 patch、一次成功
  effect、零工具错误，成功后再读两文件。first SHA-256 和 mtime 均不变，second 精确变为 after，
  v2 recovery 清空、canonical completed；UI 展开显示“已读取 4 个文件，已编辑 1 个文件”。
  应用/profile 清零并生成官方备份，独立 verdict 为 `PASS_RESTART_REPAIR`。
- 本批只记 recovered PASS，不把 W42 的首轮 1/3 改写为 N3，也不外推其他角色、范围变更、
  Unix/macOS、100 seed、LONG/99% 或完整共享 Case。完整证据在仓库外 Windows W44。

### 共享已闭合任务显式继续（W45，基线 `187d3ddec`）

- S-D02-12 / CTRL-001/006/007、REAL-013、A05/A07/A17/A18/A19 子断言：从 W44 `12-ui`
  completed Turn 的未污染正式备份恢复，同一 Session/Snapshot 下跨新构建显式调用
  `resume_task`；固定提示要求先更新计划，只重读 first/second 前 3 行及取得整文件摘要，
  不得重放修改。确定性跨构建拒绝的首次 FAIL 保留；现在只忽略历史 engine build ID/digest，
  Session、runtime binding、Snapshot、来源 Turn、当前精确引用和记录格式仍严格校验。
- `05-ui`～`21-ui` 依次保留重复 resume、计划参数错误、错误声称未导入、无证据 supported
  completion，以及 AGENTS/目标重复读取。`25-ui` 曾以 4 模型步通过原 oracle，但代码复核发现
  导入后指定的 `update_plan` 会被通用 tool-choice 归一化覆盖，因此只保留为中间样本。
  修正顺序后的 `29-ui` 又证明 provider 可忽略 Specific 选择：6 次目标读取后才更新计划，严格
  oracle FAIL。现导入待重规划时先构造全部 schema/context，再把模型工具面收窄为仅
  `update_plan`；首版过早收窄导致 completion schema 构造失败的单测也保留并修复。
- `33-ui` 已先完成 resume/plan，但模型误把分页 `read_file.sha256` 当页摘要，尝试超 schema 的
  8 MiB 读取并进入重复读取/压缩；3 个可见错误，超过预设 12 步后由正式停止按钮结束为
  cancelled。工具说明和 `expected_sha256` schema 现明确任意成功文本页都返回整份源文件摘要与
  总大小，不需要为取摘要读取全文。所有真实 FAIL、截图、数据库、事件和模型轨迹均保留。
- **Runtime 167/167 通过，8 项关键回归各 20/20**（其中新增 6 项）；格式、diff 与相关正式
  Tauri 构建通过。最终 `37-ui`（二进制 `8f43261a2137…`）用新的 data/profile 再从 W44 备份
  恢复：5 模型步，调用顺序为 resume_task、update_plan、恰好两次 read_file、update_plan、
  report_completion；零工具错误、零 effect。first/second 的 SHA-256 与 mtime 均不变，UI 展开
  显示两次读取及 4 类工具，canonical completed，严格 verdict 为 `PASS_TASK_CONTINUATION`。
  应用/profile 子进程清零并生成官方备份。
- 本批验证的是 closed-turn task continuation，不是 CTRL-009/010 的同 Turn pause checkpoint、
  cancelled/completed checkpoint 恢复；不关闭这些 Case。W42 N3 仍为 1/3；其他范围变更、角色、
  Unix/macOS、100 seed、LONG/99% 与完整共享阶段仍待验。

### 共享 pause/resume 终态与竞争矩阵（W46，基线 `085ce59e3`）

- S-D09-01 / CTRL-009/010、LIFE-015/017/020/021/022、CONC-007/008 子断言：生产 owner API
  的合法暂停/恢复基线通过，保持同一 Turn、递增 generation/fence，恢复前已完成 write 只执行
  一次；错误 digest/revision/owner/Snapshot/build/active set、超预算、未证 cleanup 均原子拒绝。
- 新增三命令竞争回归：paused 状态下 pause request、resume authorization 与 cancel 并发，无论
  前两者的串行次序，最终恰好一个 cancelled terminal；active Turn、pause 与 checkpoint 清空，
  后续新 key 不能恢复。首次整组运行把 terminal head 误期望为 idle，第二次又在 terminal 后重新
  构造 preparation 而先被正确 fence；两个夹具 FAIL 分别保留，改用既有 ready 合同及预先准备的
  stale 数据后通过，未改产品状态机。
- App 路由补 completed 负向：完成后的新 resume key 返回客户端错误；原 key 仅重放旧授权回执，
  150 ms 后事件数、模型请求数、写次数与 completed 状态均不变。新增 Store/App 两项各
  **20/20**；完整定向为 Session pause **10**、checkpoint **8**、App recovery **3**、pause 投影
  **2**、UI 暂停组件 **9** 项通过。仅测试/进度变更，无生产或 renderer 改动，未重复构建 Tauri。
- 本批证明的是 owner `/execution/resume` checkpoint 路径；W45 的 model-facing `resume_task` 是
  closed-turn continuation。会话区现只验证暂停原因、发送阻断与停止入口，owner resume 尚无正式
  会话 UI；LIFE-016 的进程/Browser/MCP in-flight 清理、完整 OBS-007、其他平台/角色及长期门槛
  仍待验，不计完整 Case 或共享阶段完成。

### 共享 in-flight 进程暂停清理（W47，基线 `68f47efed`）

- S-D09-02 / LIFE-016、PROC-033/039、CONC-004、A12/A13/A17/A19 子断言：App 集成夹具用
  `start_process` 启动实际 PowerShell helper，helper 将自身 PID 写入隔离工作区并保持运行；下一次
  provider 请求等待期间发起 owner pause。pause 前由独立 process snapshot 证明 PID 存活。
- paused 状态发布前 `host_cleanup_proven` 已按 seq 落盘，随后 helper PID 不再存在；正确
  checkpoint/digest 恢复到同一 Turn 的新 generation，重新规划并读取 PID marker 后 completed。
  canonical 只有一次 turn/started、paused、resume-authorized、completed，零 failed；
  `workspace.process/start` 恰好一次，证明已完成启动未重放、活句柄未作为恢复证据。
- 首次运行即通过；完整 `native_execution_recovery` **4/4**，最终 60 秒上限夹具同源码
  **20/20**。只新增跨层回归、`sysinfo` 测试依赖和简短进度，无生产/renderer 修改，未重复构建
  Tauri。scripted provider 用于确定 pause 边界，不计真实模型或正式 UI。
- Browser/MCP in-flight、cleanup 失败后的 owner attestation、macOS process group、真实会话 UI、
  其他角色及 N3/100 seed/LONG/99% 仍待验，不关闭完整 LIFE-016 或共享阶段。

### 共享 Retry-After 中取消（W48，基线 `c114febf3`）

- S-D09-03 / MODEL-020/022/023、LIFE-018、A08/A10/A11/A17/A19 子断言：正式 App 路由的
  scripted provider 首次返回 HTTP 429 与 60 秒 `Retry-After`；确认首个请求到达后，通过 canonical
  cancel endpoint 取消同一 Turn。独立墙钟断言在 2 秒内得到 cancelled，未等待服务端冷却。
- 取消后 checkpoint 不保留；再观察 1 秒 provider 请求数仍为 1，零第二 attempt/工具调用；
  canonical 恰好一个 cancelled，completed/failed 均为零。首次运行即通过，完整 App recovery
  **5/5**，新增产品路径回归 **20/20**；Broker 退避/取消 **3** 项、HTTP Retry-After 秒数与
  HTTP-date 解析 **2** 项通过。
- 生产取消链无需修改；本批仅新增回归与进度，未改 renderer、未构建 Tauri。真实 provider
  连接池/代理变化、cancel 与 effect receipt 竞争、正式 UI 点击停止、其他平台/角色及长期门槛
  仍待验，不关闭完整共享 Case。

### 共享 cancel 与 effect receipt 竞争（W49，基线 `7a4b3b186`）

- S-D09-04 / LIFE-019、G0-025、A04/A05/A10/A17/A19 子断言：在两连接 canonical Store 中
  同时提交 Turn cancel 与 managed effect 的成功 owner receipt。事务先后不改变归约：Turn 恰好
  一个 cancelled terminal，effect 恰好一个 succeeded receipt 并保持 returned，active Turn 清空。
- 首次短测试名配 `--exact` 实际运行 0 项；完整组第一次复跑又因夹具让 terminal 复用 started
  producer 而触发正确的 identity/idempotency 冲突；随后改成新 key 又被生命周期“必须保留原 key”
  正确拒绝。三个中间结果均保留；最终使用原 lifecycle key 与独立 owning producer，没有放宽
  产品断言或修改状态机。
- 完整 pause/effect **11/11**，新增竞争同源码 **20/20**。仅测试/进度变更，无生产、renderer、
  模型或 Tauri 验收。App owner 回执、外部 unknown effect、真实 UI、其他 Action/平台及长期门槛
  仍待验，不关闭完整共享 Case。

### 共享 cancel 后应用重启（W50，基线 `892f058a4`）

- S-D09-05 / LIFE-020、A10/A12/A17/A19 子断言：让 Turn 在 429/Retry-After 等待中进入
  canonical cancelled，随后关闭首个 AppServices/Router/数据库，并用同一隔离 data root 重建
  完整服务。`create_router` 返回前执行的 startup recovery 查询选中 0 个候选。
- 重启后 provider 请求数保持 1，GET execution 仍为 cancelled，checkpoint_retained=false，
  `execution_resumed` 事件为零；没有新模型、工具或 terminal。首次即通过，完整 App recovery
  **6/6**，新增重启回归 **20/20**。
- 生产恢复筛选无需修改；仅测试/进度变更，无 renderer/Tauri/真实模型。强杀提交窗口、
  cancelled Session 删除、其他平台/角色及长期恢复门槛仍待验，不关闭完整共享 Case。

### 共享失败即停与部分结果固定样本闭环（W51～W56，基线 `481152660`）

- S-D02-13 / CTRL-007、FILE-031/039、A17/A18：W51 复用 W42 的提示、oracle、并发 helper
  与三套独立 data/work/profile，在 W45 冻结构建 `8f43261a2137…` 上复验。`01` 通过；`02`
  在首次参数错误后继续补丁/读取，`03` 在真实部分发布失败后继续重读/补丁。三次总计 43 模型步、
  6 effects，首轮仍为 **1/3**；完整数据库、事件、模型轨迹与首次 FAIL 原样保留。
- Runtime 新增保守的 accepted-input failure-stop：只有用户同时明确错误即停及禁止重试才生效，
  后续明确允许重试可撤销。首个工具失败后完成全部上下文/Schema 构造，再把工具面收窄为仅
  `report_completion`，强制 blocked 报告并禁止重读、历史、重规划和 effect。`apply_patch` 说明
  同时补充一行替换必须使用 remove+add；context+add 只表示插入。解析器首次漏认英文
  `on error stop` 的定向 FAIL 保留，修正后 4 项定向、Runtime 169/169、4 项关键回归各 20/20。
- W53 用首版修复二进制 `4fc950c4c7df…` 正式复验，首个样本已做到一次 guarded patch、一次
  rejected effect、错误后零文件/进程动作，但模型给 blocked criterion 填入不可用
  `evidence_paths`，产生额外控制错误；修正后的摘要又没有点名两文件，严格 oracle 保留 FAIL。
  门禁因此加入待恢复目标和精确最小报告形状，要求用户可见摘要逐项说明部分结果，blocked 条目
  不得携带证据字段。对应回归和正式桌面构建通过；最终二进制为 `e2fc6f5e17ac…`。
- W56 重新从同一官方备份恢复三套隔离数据，冻结二进制、GEN Revision、route/preset、提示与
  helper。三个新 Session 均为 4 模型步、一次带两份全文摘要的多文件 patch、一次 rejected effect；
  `first.txt` 的外来并发内容及保留发布对象为 after，`second.txt` 全文 hash 不变。每次均在首个
  错误后直接 blocked 交付，零后续文件/进程动作、零额外工具错误，failed terminal 和两项 pending
  target 保留；正式 UI 同时显示部分成功、两文件结果和不重试。canonical/disk/UI 全部独立通过，
  聚合为 **N3=3/3**、12 模型步、3 effects；应用/helper/profile 全部清零并生成官方备份。
- W55 复制已恢复 data 造成 storage-root 冲突，应用未进入 Case；该准备错误单独保留且不计分母，
  W56 改为逐套官方 restore。W42/W51/W53 的历史失败不改写。完整角色、其他失败/完成分支、
  Unix/macOS、100 seed、LONG/99% 与共享阶段仍待验，不关闭完整 Case。

### 共享既有文件原生替换源补偿（W57，基线 `3192d1566`）

- S-D03-36 / FILE-020/025/038/039、A05/A07/A17：在最后一次暂存核对结束后、
  `ReplaceFileW` 实际打开 source 名称前，分别把暂存换成同字节异身份文件和原地改写同一对象。
  两项首次 FAIL 均已正确返回 outcome_unknown 并保留原件备份，却让目标实际变为外来/错误字节；
  失败夹具和完整现场保存在 W57 `01-before/`。
- 原生包装现在紧邻系统调用采集实际派发源的卷号和 128 位文件 ID。调用返回后，若目标仍是
  该派发对象，但与预期暂存 owner 或调用字节不符，则以身份句柄将它无覆盖地退回 source 名，
  再以原件句柄将 backup 无覆盖地恢复到目标。结果仍为 unknown，保留曾发生效果的事实；外来
  同字节对象回到原暂存名，自有但被改写的暂存由原 owner 清理，均不误删其他对象。
- 补调用后目标再次被并发文件接管的反例：当前目标 ID 与派发 ID 不同，补偿不触碰它，原件
  backup 与实际已发布对象都保留供核对。另模拟身份 capture 后、原生调用前的最窄换源窗口，
  同样不把目标误认成可补偿对象，继续返回 unknown；因此该最窄窗口仍未宣称零副作用。
- `nomifun-file` **551/551** 通过；五项相关回归各 **20/20**。本批是 owner 原生边界，未改
  renderer，未运行模型或 Tauri UI。Unix/macOS 的未消耗暂存清理/发布、100 seed、完整角色/UI
  与长期门槛仍待验，不关闭完整 FILE Case 或共享阶段。

### 共享无 watcher 外部变更主动刷新（W58，基线 `7b2bebd14`）

- S-D03-37 / FILE-040、A05/A17/A19：直接 FileService 首次反例在完成快照后由外部创建
  `new.txt`，第二次显式清单仍只返回 `old.txt`；FAIL 与磁盘现场见 `01-before/`。完成缓存原先
  没有失效来源时可永久存活，关闭再打开 `@文件` 菜单也无法恢复。
- 新的显式 API/Agent 清单读取会先退休无人使用的完成快照，再从磁盘扫描。仍有活动读者时不
  撤销其快照；扫描中 watcher/owner 失效、最多一次有依据重读、旧扫描不得覆盖新扫描及不同根
  失效隔离均保留。外部创建+删除的直接 owner、真实 Axum `/api/fs/list` 与根隔离三项回归各
  **20/20**，`nomifun-file` **552/552**。完整组中一次无关原生 watcher 3 秒超时原样保留；未加
  sleep 或放宽断言，定向复跑和最终完整组通过。
- 正式 Tauri 构建 `fcc93ec68118…`；`11-ui/` 使用独立 data/work/profile 和新 Session。
  初次 `@old` 只显示 `old.txt`，随后不经过 NomiFun 写路由、也不启动 watcher，由外部 PowerShell
  创建 `new.txt` 并删除旧文件。关闭重开菜单后 `@new` 只显示新文件，`@old` 显示“搜索结果为空”；
  四次真实 `/api/fs/list` 均为 HTTP 200，watch route 调用为 0，磁盘 hash 与 UIA 选项独立一致。
  准备回合 1 模型步、0 effects，canonical completed；应用/profile 清零并生成官方备份。
- 本批未修改 renderer。单次无 watcher 扫描进行中的外部竞态、完整丢批/乱序、其他角色与平台、
  100 seed/LONG/99% 仍待验，不关闭完整 FILE-040 或共享阶段。

### 共享 watcher 尾事件与停止 fence（W59，基线 `d528ce875`）

- S-D03-38 / FILE-040、AUTH-005、A09/A13/A17：构造旧单文件 native 注册已排队事件，在停止
  后以新 owner 重订同一 canonical path，再延迟执行旧 callback。旧实现共用全局 watcher/map，
  因而把旧事件投给新 owner。首版测试事件使用非 canonical path，未命中映射而假通过；夹具错误
  保留，改用真实 key 后首次产品 FAIL 见 `02-corrected-fixture/`。
- 单文件监听改为每个 canonical path 独立 native 注册，callback 捕获该注册自己的 owner fence
  与 debounce；同路径多 owner 仍共享该注册。Office watcher 也采用同一 owner fence。发送先登记
  in-flight delivery，再在互斥区外调用 event sink；停止先移除 owner，等待已开始发送结束后再
  unwatch。等待不持注册表锁，避免 sink 重入 watch route 时死锁；unwatch 未确认则恢复 owner 并
  报错。旧 callback 只能看到已退休集合，新注册不继承尾事件或抑制时间。
- 单文件迟到、Office 迟到、发送中停止三项确定性回归各 **20/20**；`nomifun-file` **555/555**，
  含真实 native 删除重建、重订、重叠 Office 根及 file_watching 14 项。首次失败与修复后日志在
  W59；本批未改 renderer、未运行模型/Tauri UI。
- 活动注册数量/资源上限、native callback 永不返回、完整 rescan/dropped/乱序 UI、其他平台、
  100 seed/LONG/99% 仍待验，不关闭完整 FILE-040 或共享阶段。

### 共享 Unix 暂存源与失败清理（W60，基线 `9ab072a4f`）

- S-D03-39 / FILE-019/020/025/028/038/039、A05/A07/A13：使用本机 WSL2 Ubuntu 6.18，
  夹具在 `/home` ext4 临时目录运行；源码来自 `/mnt/c`，文件语义断言只作用于 ext4 夹具，不把
  drvfs 行为作为验收。既有替换在最终检查后分别原地改写暂存、换入同字节异 inode，旧实现均
  返回成功并把错误对象发布到目标。首次两项 FAIL 的完整 ext4 目录已复制到 W60 `01-before/`。
- Unix 发布现在为所有暂存保留 inode 身份；rename/hard-link 前使用 `O_NOFOLLOW` 打开名称，核对
  inode 与调用字节，并通过该句柄继承权限；发布后再次核对目标名称仍指向该 inode 且字节一致。
  失败清理先核对同名对象身份，只删除本 operation 的暂存；异 inode 文件保留并设置
  `temporary_cleanup_unconfirmed`。新建目标的 hard-link 分支也使用同一规则，未扩大文件权限。
- 既有文件两项及新建文件一项 Linux 原生回归各 **20/20**；Unix `service::tests` **124/124**，
  Windows `nomifun-file` **555/555**。首次重复脚本漏 WSL Cargo PATH 的夹具错误另存并修正。
- Linux 全 crate 仍有 12 项单独失败：11 项为 WSL2 artifact 目录 fsync 的 EBADF，1 项为既有
  native watcher 时序；没有忽略或修改它们来制造全组通过。身份核对→rename、身份检查→unlink
  的最窄路径窗口、macOS 主 lane、完整 IO fault/UI/角色、100 seed/LONG/99% 仍待验，不关闭
  完整 FILE Case 或共享阶段。

### 共享 Unix Artifact 目录同步（W61，基线 `a9d145c16`）

- S-D03-40 / ART-001/003/007、A05/A13：W60 Linux 全组的 11 项 Artifact 失败均为
  `cannot sync artifact directory: Bad file descriptor`。根因是 `cap_std::Dir` 在 Linux 可持有
  `O_PATH` capability descriptor，旧代码 clone 后直接 `fsync`，内核按合同返回 EBADF。
- Unix 目录同步现在通过同一 capability Dir 的 `"."` 重新打开可读目录句柄，再调用 `sync_all`；
  没有重新解析 ambient path，也没有像 Windows 特例一样吞掉权限/IO 错误。新增 focused 回归
  **20/20**，Linux Artifact **18/18**，Windows Artifact **15/15**。
- 同源码 Linux 全 crate 从 W60 的 290/302 收敛为 **302/303**；唯一剩余失败为既有
  `inventory_refreshes_before_native_remove_delivery`，定向复核仍稳定失败，已完整保留并转 W62。
  本批未改 renderer、未运行模型/Tauri UI。
- macOS 主 lane、真实磁盘故障/断电 durability、完整 Agent/UI/角色、100 seed/LONG/99% 仍待验，
  不关闭完整 Artifact Case 或共享阶段。

### 共享 Linux 删除事件去重（W62，基线 `ce1f789e2`）

- S-D03-41 / FILE-040、A05/A17：W61 Linux 全组唯一剩余失败为
  `inventory_refreshes_before_native_remove_delivery`，定向重跑仍稳定超时。W62 保留 raw notify
  轨迹，确认 Linux 删除一个被监听文件时先发同路径 `Modify(Metadata)`，随后才发 `Remove(File)`。
  旧 debounce 只使用 path，前一个 change 占据 200 ms 窗口；接收方按合同忽略 metadata change，
  因而永远收不到删除事实。
- 单文件监听现以 `event_type + path` 为 debounce key；相同类型抖动仍合并，create/change/remove
  不再互相抑制。Linux 纯归约和真实 ext4 删除两项各 **20/20**，Linux lib **304/304**、
  file_watching **14/14**；Windows file_watching **14/14** 及真实删除回归通过。
- Linux 全 crate 随后运行到独立 snapshot 集合，`single_file_operations_use_literal_paths` 稳定失败
  49/50，原样保留并转 W63；不计本批 watcher 失败。本批未改 renderer、未运行模型/Tauri UI。
- macOS、事件洪泛/乱序、完整 UI/角色、100 seed/LONG/99% 仍待验，不关闭完整 FILE-040 或共享阶段。

### 共享 Unix snapshot 字面路径恢复（W63，基线 `9ba99719e`）

- S-D05-01 / VCS-004/005、A05/A14/A17：W62 Linux 全组的 snapshot 49/50 失败稳定落在
  `file\\1.txt`。libgit2 checkout 在 Unix 即使设置 `disable_pathspec_match(true)`，仍把反斜杠作为
  分隔符并返回成功，因此 discard 后字面文件仍是 staged。诊断前原断言未输出名称的首次 FAIL，
  补诊断后的精确 FAIL 均保留。
- 首版修复用只含目标的内存 index checkout，Windows 回归先把邻居 index 清空，再把邻居 worktree
  删除；两项中间 FAIL 已记录，方案撤回。最终仅对 Unix 含反斜杠的已验证 Git index 路径读取
  HEAD blob，通过同目录唯一临时对象原子发布；普通路径继续走原 libgit2 checkout。regular 文件
  恢复 Git executable mode，tracked symlink 恢复原 target，邻居 index/worktree 不变。
- Linux 字面路径全流程与 executable/symlink 两项各 **20/20**，snapshot **51/51**；Linux
  `nomifun-file` 首次完整 **460/460**。Windows snapshot **50/50**，证明 Unix 特例没有改变现有
  方括号/感叹号等 Windows literal 行为。本批未改 renderer、未运行模型/Tauri UI。
- macOS、非 UTF-8 Git path、更多 filemode、父目录并发置换、完整 VCS/角色、100 seed/LONG/99%
  仍待验，不关闭完整 VCS Case 或共享阶段。

### 共享 watcher 丢批/重扫系统上下文（W64，基线 `ded5cae92`）

- S-D03-42 / FILE-040、PORT-012、A08/A14/A15/A17：在 App watcher 队列内同时注入 native
  rescan、会被拒绝的 `a/../b` 与合法 `src/visible.rs`。pre-turn batch 精确保留一个合法事件、
  `dropped_event_count=1` 和 `rescan_required=true`；警告要求重读相关 workspace 状态，并说明不能
  据此判定其他路径未变化。绝对 workspace root 不进入上下文，第二次读取为空。
- 新回归继续经过 `merge_pre_turn_context` 组成实际 system prompt，确认上述 canonical JSON 与
  对账说明没有在 contributor/提示合并边界丢失。现有生产实现直接通过；本批只增加跨层回归，
  没有新增 Action、工具或权限，也未修改 renderer。
- Linux Cargo 单项 **1/1**、watcher 上下文 **12/12**、新增用例 **20/20**；Windows Cargo 单项
  **1/1**、watcher 上下文 **12/12**、新增用例 **20/20**。首次 WSL 通用命令在 lib 12 项通过后
  继续启动零匹配 integration binary，因共享 `build.noindex` 混入异平台产物触发 loader 失败；
  首次 `--exact` 过滤名不完整而执行 0 项。Windows 首次切回编译又在 `nomifun-db` 发生 rustc
  栈溢出；提高该验证进程的 `RUST_MIN_STACK` 后同命令通过。三项首次 runner/编译失败均原样保留。
- 本批是确定性的模型请求上下文组合验证，未调用模型或 Tauri UI。正式 UI 收到丢批后触发全量
  对账、模型遵循重读提醒、重复/乱序恢复、macOS、100 seed/LONG/99% 仍待验，不关闭完整
  FILE-040、PORT-012 或共享阶段。

### 共享广搜 instruction lookup 资源上限（W65，基线 `f7b9db045`）

- S-D03-44 / FILE-013/015、A15/A17/A19：正式 Tauri 基线 `5737d45caf98…` 使用独立安装、
  workspace/profile 和 StepFun Plan / `step-3.7-flash`。准备回合 1 步、零工具；随后由应用外写入
  400 个 `marker-0001.txt`～`marker-0400.txt`，内容摘要 `2854742e…ec43`，再要求同一 Session
  统计标记总数和首尾名称。完整数据库、事件、截图、模型轨迹和官方备份均在仓库外。
- 基线模型首个 `search_files(limit=200)` 后，Runtime 对每个唯一 hit 做 instruction-scope lookup，
  单次产生 **100** 个内部读取；回合合计 **415** 个内部读取、**2,313** 条 canonical 事件、11 个
  模型步。最终答复与独立磁盘一致，但 UI 曾显示工具错误，按统一规则保留 FAIL/RECOVERED，不把
  正确终值改记正向 Case 通过。65-hit 定向测试也首次失败，证明旧边界仍接受无上限 fan-out。
- Runtime 现在另设 64 个唯一 hit path 的 instruction-discovery 上限。64 项继续接受；第 65 项在任何
  逐命中读取前返回 `search_context_withheld`。原 owner 搜索仍真实执行且公开 `limit<=200` 不变，
  snippets 不会在缺少完整 instruction 核对时进入模型；没有新增工具、授权或隐式文件读取。
- Windows/Linux Runtime 在 W65 源码上各 **171/171**；新增 64/65 边界各 **20/20**。修复版正式构建
  `8e7b433e481a…` 重复同一 400 文件摘要，最终仍正确报告 400、`marker-0001.txt`、
  `marker-0400.txt` 且磁盘未变；首个搜索内部读取从 100 降到 5，总内部读取从 415 降到 68，
  canonical 事件从 2,313 降到 664。应用与 profile 子进程均清零并完成官方备份。
- 合入远端 macOS sandbox/Companion 三个提交后，Windows Runtime 再次 **171/171**，同步后的正式
  Tauri 构建 `eca9197833bf…` 通过；Linux 复验在 W65 用例前因远端 `platform/unix.rs` 漏导入
  `Path` 编译失败，完整日志保留并转 W66，不把同步前结果冒充当前 HEAD 的 Linux 通过。
- 修复样本仍有可见命令/完成报告错误并用了 16 个模型步，因此只关闭资源 fan-out 子断言；本次
  native 400 文件 burst 未独立采集 provider system prompt 或实际 dropped 数，不替代 W64 的确定性
  组合证明。完整正向搜索体验、重复/乱序、其他 Agent/macOS、100 seed/LONG/99% 仍待验。

### 共享 Unix 显式程序预检编译修复（W66，基线 `13e7349da`）

- S-D04-02 / PROC-011/012/014、A19：W65 合入远端 `b046314cd` 后，Linux 在编译
  `nomi-process-runtime` 时报告 `cannot find type Path`，W65 测试尚未启动。根因是新加入的
  `validate_explicit_unix_program` 面向所有 Unix，而 `Path` 仍只在 macOS cfg 下导入；首次编译
  失败已复制到 W66 外部证据。
- `Path` 现随 Unix 模块无条件导入，`PathBuf` 继续限定 macOS；只修构建可见性，不改进程启动、
  sandbox、权限、错误分类或清理语义，也没有 renderer/模型/UI 变更。
- Linux `nomi-process-runtime` 完整 **245/245**，当前 HEAD 的 Agent Runtime **171/171**；
  Windows process Runtime lib **120/120**。首次 Linux 完整组在 `parent_death` 的 PTY 子项出现一次
  reap 时序失败，普通进程子项及此前各组均通过；该失败保留，原测试定向复跑 **1/1**、随后
  **20/20**，最终完整组通过，未增加等待或修改断言。
- macOS 原生行为不由 WSL/Windows 代判，仍引用远端 M01-01；更多 Unix 发行版/架构、正式 UI、
  100 seed/LONG/99% 仍待验，不关闭完整 PROC Case 或共享阶段。

### 共享进程工具 host shell 字段说明（W67，基线 `c3bc6e0b9`）

- S-D04-03 / CMD-135/150、A08/A17/A19；Windows CMD-133 子断言：W65 修复版正式样本在
  `search_context_withheld` 后，首个进程调用是 `cmd: "dir /b /s burst"`。Windows 的 `cmd`
  形态经 PowerShell 执行，因 Command Prompt 参数语法失败并在 UI 留下错误。现有顶层说明已经
  声明 PowerShell，但基础句的 `cmd=ls -la` 示例与字段级泛化描述仍给模型相反暗示；首次 Schema
  回归也确认字段中没有 `PowerShell`。
- Runtime 工具基础说明不再嵌入 Unix 示例；`cmd` 字段按实际 process host 写明 Windows
  PowerShell 与 Unix `/bin/sh -c`。Windows 字段明确 `dir /b`、`dir /s` 不能直接使用，需要
  Command Prompt 时采用 `command=cmd.exe` 和独立 `args`。只改模型可见描述，不改 Schema 接受集、
  Action、权限、调用 owner 或 shell 实现。
- Windows/Linux Runtime 各 **172/172**；新增字段回归各 **20/20**。正式 Tauri 构建
  `57515a20e31c…` 使用独立 data/work/profile、StepFun Plan / `step-3.7-flash` 和同类 400 文件
  夹具。准备回合 1 步；对账回合 5 步：broad search 按 W65 边界 withheld 后，首个调用直接使用
  PowerShell `Get-ChildItem`，三个进程调用全部成功，零进程错误；最终正确报告 400、
  `marker-0001.txt`、`marker-0400.txt`，磁盘未变，canonical 两回合 completed，共 264 条事件。
- 应用与 profile 子进程已清零并完成官方备份。单个真实模型样本不计 N3；UI 仍显示预期的 broad
  search withheld，故不关闭完整正向 CMD-135/150。macOS/Linux 真实模型、其他 Agent、100 seed/
  LONG/99% 仍待验。

### 共享有界搜索 UI 分类（W68，基线 `454c8fc692`）

- S-D02-14 / FILE-015、A08/A15/A17/A19：W67 正式样本中的 `search_context_withheld` 是 Runtime
  有意保留的有界结果，canonical 为 `is_error=true` 并触发模型完整核对；UI 却显示“1 项操作出现
  异常”，把预期资源边界与普通工具失败混在一起。该首次失败、截图和数据库轨迹保留在仓库外。
- UI 归一化现在只接受本地 `search_files`、`status=error`、总长不超过 4 KiB、字段集合精确为
  `kind/notice/search_executed/snippets_withheld` 且两个布尔值均为 true 的 JSON。命中时记录
  `boundedResult=search_context_withheld`，保留 `nonFatalFailure` 供现有 Turn 状态使用；回执统计把它
  从普通非致命错误数分离，显示琥珀色“搜索结果受限，需要完整核对”。格式错误、额外字段、错误
  工具名或其他错误仍保持 fatal；没有改变 canonical、Runtime 重规划、工具权限或结果详情。
- 三个定向测试文件 **72/72**；typecheck、i18n parity、生成 key 与
  `check:desktop-ui-boundary`（1,949 个 renderer 源，最小 880×600）通过。正式 Tauri 构建
  `a1d254ff1c18…` 使用独立 data/work/profile、StepFun Plan / `step-3.7-flash` 和外部创建的
  400 文件 burst。中途 UI 显示新的有界提示而非普通异常；canonical 仍保留 `is_error=true`，模型
  随后用两个成功的 PowerShell 只读调用完整核对，最终正确报告 400、`marker-0001.txt`、
  `marker-0400.txt`。两回合 completed、共 214 条事件，工作区前后摘要相同。
- 应用与 profile 子进程已清零，官方备份完成。单个真实模型样本不计 N3；其他有界结果、其他
  Agent/平台、完整 FILE-015、100 seed/LONG/99% 仍待验，不关闭共享阶段。

### 共享 Unix 新建发布清理窗口（W69，基线 `90097d1f7`）

- S-D03-45 / FILE-019/028、A05/A13/A17/A19：在 WSL2 Ubuntu ext4 上为新建发布增加确定性
  cleanup hook。旧路径先 hard-link 暂存 inode 到目标，再按“核对 inode、按名称 unlink”清理暂存；
  hook 在核对后把该名称换成外来文件。首次运行显示目标字节正确、operation 返回成功，但外来
  `stage.tmp` 被删除，证明成功 receipt 遮蔽了额外副作用。首次日志、`new.txt`、保留的原暂存 inode
  和 observation 均在仓库外；首个脚本 CRLF 退出码问题及后续 PATH 未加引号的 runner 错误另行保留。
- Linux/macOS 新建发布改用 `nomifun-common` 已有的原生 no-replace rename。Linux 的
  `renameat2(RENAME_NOREPLACE)` 与 macOS 的 `renamex_np(RENAME_EXCL)` 都在不覆盖并发目标的同时
  原子消费暂存名，因此成功路径没有 check→unlink 窗口。没有增加权限、删除重试或失败吞并；其他
  Unix 暂时保留 portable hard-link fallback，Windows 继续使用原句柄 owner 路径。
- 修复后 Linux 精确回归 **1/1**、同一反例 **20/20**、service **125/125**；固定现场显示 hook
  不再可达，只有正确的 `new.txt`，无暂存/retained 残留。Windows `nomifun-file` lib **362/362**，
  `cargo fmt --all -- --check` 通过。本批是原生文件 owner 边界，无 renderer/模型/Tauri UI 变更。
- macOS 原生执行、其他 Unix fallback、失败路径的身份检查→unlink、既有目标 verify→rename、
  IO fault、完整 UI/角色、100 seed/LONG/99% 仍待验，不关闭完整 FILE Case 或共享阶段。

### 共享 Unix 既有目标派发源窗口（W70，基线 `df06d593e`）

- S-D03-46 / FILE-020/025/038、A05/A07/A17/A19：在 Linux ext4 夹具中，于暂存 inode 与精确
  字节已通过句柄核对之后、`rename` 实际按名称取源之前，把暂存名换成外来 7 字节文件。实际系统
  调用会发布该外来对象，原意 8 字节暂存保留到独立名称；这是 W60 尚未覆盖的最窄派发窗口。
- 现有发布后核对首次即得到正确保守终态：`published=true`、`publication_verified=false`、
  `publication_identity=None`，错误包含 `outcome_unknown` 和 `re-read before retry`；目标真实为外来
  字节，暂存原件仍可核对。但首版断言遗漏了 retained 暂存仍存活，因而把
  `temporary_cleanup_unconfirmed=false` 当成通过；该假通过单独保留。收紧断言后首次产品 FAIL 证明
  rename 成功被错误等同为“自有暂存已消费”，清理 receipt 少报残留。
- Unix 发布后现在只有目标仍匹配自有暂存 inode 时才设置 `temporary_consumed=true`。目标身份不符
  时继续保留 `outcome_unknown`、`published=true`、未验证且无 publication identity，并增加
  `temporary_cleanup_unconfirmed=true`；不会对已经被复用的暂存名称做补删。未增加重试、权限或
  乐观终态。失败现场、修复后 observation 和首次弱断言均在仓库外；一次 WSL stdout 路径采集
  导致的夹具复制失败另行保留，随后按精确目录复制。
- 修复后 Linux 精确回归 **1/1**、重复 **20/20**、service **126/126**；Windows
  `nomifun-file` lib **362/362**，`cargo fmt --all -- --check` 通过。本批无 renderer/模型/Tauri UI
  变更。零副作用补偿、失败清理 check→unlink、macOS 原生执行、IO fault、完整 UI/角色、100 seed/
  LONG/99% 仍待验，不关闭完整 FILE Case 或共享阶段。

### 共享 watcher overflow 全量对账（W71，基线 `99c827a5c`）

- S-D03-47 / FILE-040、PORT-012、A05/A15/A17/A19：新增确定性 contributor 回归，向同一
  Session watcher 队列写入 263 个唯一事件。队列保留最后 256 个，首尾为 `marker-0007` /
  `marker-0262`，精确记录 `dropped_event_count=7` 且不伪造 native `rescan_required`；生成的系统
  上下文明确批次不完整、要求重读并禁止据此推断其他路径未变，随后一次 drain 清空全部状态。
  Windows/Linux 精确测试各 **1/1**，生产代码首次即满足断言，本批只增加回归。
- 正式 Tauri 构建 `8c54fc75dd29…` 使用独立 data/work/profile、StepFun Plan /
  `step-3.7-flash`。准备回合完成后，外部并发创建 300 个含 `W71_EVENT_MARKER` 的文件并等待 native
  watcher 分发；用户只询问总数与按名首尾。模型首步即用 PowerShell 完整列举目录，随后做全文
  统计和抽样读取；三次进程调用及四次读取全部成功，零工具错误。最终 UI/canonical/磁盘一致报告
  300、`event-0001.txt`、`event-0300.txt`，并明确结果完整、未修改文件。
- 两回合均 completed，共 8 个模型 step、311 条 canonical 事件；工作区前后 count/min/max/hash
  完全一致。应用与 profile 子进程清零并完成官方备份。真实 native 300 文件样本没有独立持久化其
  精确 dropped 数，因此只把“确定性 overflow 计数 + 正式 UI 全量重读”作为组合证据；完整丢批/
  重复/乱序/rescan UI、其他 Agent/平台、N3/100 seed/LONG/99% 仍待验，不关闭完整 FILE-040。

### 正式 Runtime watcher 接线与有界诊断（W72，基线 `082d1085b`）

- S-D03-49 / FILE-040、PORT-012、OBS-018、A05/A15/A17/A19：W71 的正式 UI 样本证明模型能
  主动全量重读，却没有真实 native dropped 计数。W72 首版在 lifecycle contributor 内加入日志后，
  两个独立 300 文件 burst 均没有 watcher 启动或批次记录；数据库确认 Preset 已启用
  `workspace.files` 且 Session 持有精确 workspace binding。首次无接线现场、截图、日志和数据库均保留。
- 根因是正式 Nomi Runtime 走 `EngineKernelSession`：它只组装 initial capability context 与统一
  Plugin context，未持有 `NomiWorkspaceWatchContext`；通用 Plugin Tool Session 的 lifecycle
  contributor 只服务另一条 materialization 路径，W64 的直接单测因此不能证明正式产品接线。
  现由 `EngineKernelSession` 在 Full scope、`workspace.files` 被精确选择且 workspace 已由 host
  canonicalize 后启动 watcher，并随该 Runtime 跨 Turn 持有；`plugin_context_for_turn` 在每轮模型
  前先消费 watcher，再合并其余 context。restricted Attempt 维持原先不启用 lifecycle context 的边界。
- 新增诊断每 Session 只记录一次 watcher 启动；每个非空批次只记录 event count、known dropped、
  native rescan 与是否需要 reconciliation，不记录路径、文件名或内容。最终正式 Tauri 构建
  `46063c30f9dd…` 使用隔离 data/work/profile：准备 Turn 后外部并发创建 300 文件，实际收到 603
  个通知，保留 256、精确 dropped=347、`rescan_required=false`、`requires_reconciliation=true`。
  下一 Turn 明确禁止工具并只回复标记，UI/canonical 均完成且零工具调用，证明批次在模型前消费。
- 最终两回合 completed、43 条 canonical 事件；外部 300 文件 count/min/max/hash 保持，应用与
  profile 子进程清零并完成官方备份。Linux 精确回归 **1/1**；同步远端至 `0fee0eda69d6…` 后
  正式 Tauri 重建 `7329a6f0dda9…` 与 workspace fmt 亦通过，watcher 源码未变。首轮真实计数
  探针另暴露旧模型 `cmd` 双层 PowerShell、PowerShell 5.1 `??` 和只读请求仍写
  `check_marker.ps1` 的失败，原样保留并转后续 CMD/控制子断言；重复/乱序/rescan UI、其他 Agent/
  macOS、N3/100 seed/LONG/99% 仍待验，不关闭完整 FILE-040。

### Windows PowerShell 5.1 模型调用合同（W73，基线 `6b1cd04ea`）

- S-D04-05 / CMD-135/150、PROC-014、A08/A13/A17/A19；Windows CMD-133 子断言：W72 首次
  正式计数探针把 `powershell.exe -Command ...` 整段放进已由 PowerShell 执行的 `cmd`，`$files`
  等变量被外层 shell 提前展开而失败；改用 `command`/`args` 后又提交 PowerShell 7 的 `??`，
  System32 WindowsPowerShell v1.0（5.1）解析失败。两项真实失败和随后写入临时脚本的轨迹均保留。
- 同步 S-D04-04 的普通 executable 字面 argv 首选后，首次新回归仍因 model-facing Schema 没有
  `Windows PowerShell 5.1` 事实而失败。现明确 `cmd` 的内容由 5.1 直接执行，不能再加
  `powershell.exe`/`pwsh` 前缀；显式调用用 `command=powershell.exe` 与分离 `args`，脚本不得假定
  `??`、`??=`、三元 `? :`、`&&`、`||` 等 PowerShell 7 语法。没有改变合法参数、Action、权限或 owner。
- 精确 Schema 回归 **20/20**、Agent Runtime **173/173**、workspace fmt 与正式 Tauri 构建通过。
  正式二进制 `90a0d26b79b6…` 使用隔离 workspace/data/profile；step-3.7-flash 首次即形成
  `command=powershell.exe`、`args=["-NoProfile","-Command",...]`，仅一次 process effect，退出 0、
  `reaped=true`，正确报告 300、`ps51-0001.txt`、`ps51-0300.txt`。单回合 completed、2 个模型步、
  67 条 canonical 事件；300 文件 count/min/max/hash 不变、无额外路径，应用/profile 清零并完成
  正式备份。`cmd` 直接脚本、`start_process`、完成参数形状、只读任务写入防线、N3/100 seed/
  LONG/99% 仍待验，不关闭完整 CMD/PROC 或共享 D04。

### 明确只读任务的 workspace mutation 工具收窄（W74，基线 `4692f38cb`）

- S-D04-06 / CMD-137、A08/A11/A17/A19：W72 首个正式只读计数探针虽明确“不要修改任何文件”，
  模型仍能调用 `write_file` 并创建 `check_marker.ps1`。W74 新增跨过 accepted input 与实际模型工具
  列表的回归，首次稳定确认 `write_file`、`apply_patch`、`git_stage`、`publish_artifact` 仍全部可见；
  两个首次失败现场均保留。
- Runtime 现保守识别明确的中英文 workspace 禁写措辞；每个模型步在完整冻结工具面重新物化后，
  只隐藏 workspace files/VCS/Artifact 的非 ReadOnly binding，保留读取和 process。附加约束明确
  process 也须保持内联只读，不得重定向、运行变更命令或创建 helper/temp script。后续 accepted
  input 明确允许修改时恢复原冻结 binding；含“不允许修改”的否定句不会误撤销。没有新增 Action、
  扩大权限或改变 owner admission；opaque shell 是否写入仍不能由此在 Kernel 层确定阻断。
- 策略解析与撤销回归通过，工具面收窄/恢复 **20/20**，Agent Runtime **175/175**，workspace fmt
  与正式构建通过。正式 Tauri `10c052e32ac6…` 使用隔离 workspace/data/profile：真实模型未调用
  workspace mutation 工具，仅一次 process effect 真正执行，300 文件 count/min/max/hash 不变且
  无额外路径，应用/profile 清零并完成正式备份。首次仍把 `exec_command.args` 传成 JSON 字符串，
  owner 前置 Schema 拒绝且 UI 显示 1 项异常；第二步改成数组后正确报告 300、`readonly-0001.txt`、
  `readonly-0300.txt`，单回合 3 模型步、87 条 canonical 事件。该错误不覆盖，正式样本只验证禁写
  子断言，不计零失败 CMD PASS；收紧撤销短语后的最终重建 `baec5c5f06e2…` 通过。opaque process
  内核只读、正式撤销 UI、参数字符串修复、其他角色/平台及 N3/100 seed/LONG/99% 仍待验。

### 模型数组参数原生 JSON 形状说明（W75，基线 `155b88ff0`）

- S-D01-02 / PROC-001/003、CTRL-006/007、A02/A17/A19：W72 的完成报告把
  `requirement_ids` 传成字符串，W74 的正式回合又把 `exec_command.args` 传成包含 JSON 的字符串；
  两者均被前置 Schema 正确拒绝且没有 owner dispatch，但 UI 出现可见错误。原失败截图、canonical
  事件和模型轨迹均保留。
- W75 首次回归确认两类字段虽然已经是 `type=array`，字段说明没有显式区分 JSON 数组值与包含
  JSON 的字符串。现为 process `args` 和 completion 的 `criteria`、`requirement_ids`、
  `evidence_call_ids`、`evidence_paths` 补充实际数组形状及反例说明；没有接受字符串、自动解析或
  改动 validator，错误类型继续整批拒绝且零副作用。
- process/completion 两项精确回归各 **20/20**，Agent Runtime **175/175**，workspace fmt 与正式
  Tauri 构建通过。正式二进制 `668062e479e6…` 使用隔离 workspace/data/profile；同类只读任务中
  step-3.7-flash 首次即提交 `command=powershell.exe` 与真实 `args` 数组，唯一进程退出 0、
  `reaped=true`，UI `visible_failure_count=0`，正确报告 300、`shape-0001.txt`、`shape-0300.txt`。
  单回合 2 个模型步、67 条 canonical 事件；300 文件 count/min/max/hash 不变、无额外路径。
- 第二个隔离正式样本强制两条独立进程调用，随后真实调用 `report_completion`：`criteria` 与两组
  `evidence_call_ids` 均首次为原生数组，两个进程均成功/reaped，UI 零异常，正确报告 alpha=5、
  beta=7、合计 12；单回合 3 个模型步、121 条 canonical 事件，12 文件 tree hash 不变。两次样本均
  应用/profile 清零并完成正式备份。但 alpha criterion 错误复用了 beta 的 call ID，完成工具虽接受，
  该观察不能证明 alpha；因此本批只关闭数组形状子断言，精确 evidence 语义关联转后续 CTRL 批次。
  其他数组字段/Provider/角色/平台及 N3/100 seed/LONG/99% 仍待验，不关闭完整 REG/CTRL/PROC。

### 多命令完成证据的匹配与不可用归约（W76，基线 `148e74fe3`）

- S-D02-15 / CTRL-006/007、A05/A07/A17/A18/A19：W75 正式完成报告把 alpha criterion 与
  beta criterion 都绑定到第二条 beta call ID，数组形状合法但 alpha 没有对应证据。首版修复仅要求
  “不同命令引用匹配 ID”；正式 Tauri `d5e34c3e3b11…` 随即暴露合同冲突：第二条 process 启动后，
  第一条 command observation 按保守 workspace epoch 规则已退出 `available_evidence`。模型在匹配
  ID、重跑、历史加载和 unverified 之间反复推演，6 个模型步后因续写预算耗尽 failed。截图、完整
  reasoning/canonical、数据库和磁盘断言均保留。
- 最终 completion 说明与 `evidence_call_ids` 字段明确：只引用当前 `available_evidence` 内的匹配
  call ID；匹配的早期调用缺席时，以无证据 `unverified` 如实闭合，不得借用最新 ID、加载历史或
  未经授权重跑。一个观察确实支持多个 criterion 的合法形状仍保留；没有改变 evidence 有效期、
  Schema validator、Kernel admission 或权限。
- 最终精确回归 **20/20**、Agent Runtime **175/175**、workspace fmt 与正式构建通过。正式 Tauri
  `c90c28c56c46…` 使用隔离 workspace/data/profile：epsilon/zeta 两个进程均成功/reaped；完成报告
  dispositions 精确为 `[unverified,supported,unverified]`，evidence IDs 为 `[[],[zeta_call],[]]`，
  未伪用 epsilon 的过期 ID。最终正确报告 epsilon=3、zeta=8、合计 11，并明确披露 epsilon/合计
  未验证；单回合 completed、5 个模型步、140 条 canonical 事件、UI 零工具异常，11 文件 tree hash
  不变，应用/profile 清零并完成正式备份。多条顺序 process 结果要全部 supported 仍需可证明只读
  的 owner 合同或原子聚合；其他 Provider/角色/平台及 N3/100 seed/LONG/99% 仍待验。

### 托管进程显式取消、清理终态与轮询来源（W77，基线 `7be61a7c9`）

- S-D04-07 / CMD-139、PROC-033、A13/A17/A19：首个隔离正式 Tauri 样本按要求执行
  `start_process → poll_process → cancel_process`，实际进程输出 READY 后仍在运行，并在自然退出前
  被取消；PID 已消失且 `cleanup.reaped=true`。但 `engine_process_host` 把 `Cancelled` 落入统一
  `success=false` 分支，canonical `is_error=true`，UI 显示普通错误。该首次失败、工作区前后摘要、
  helper 快照、截图、数据库、完整事件与模型轨迹均保留。
- 首版把已回收 cancel 映射为成功后，第二个正式样本确认工具本身已是 `success=true`，但 Runtime
  仍把它计为 `failed_commands=1`、`usable_at_observation=false`，模型首次 completion 引用进程 ID
  时被动态 Schema 的 `maxItems=0` 拒绝。修正工作状态与完成证据后，第三个正式样本达到零工具错误，
  但 cancel command provenance 的 `interaction_call_ids=[]`，完成报告用最终 cancel observation
  说明两次 poll 时缺少中间来源。两项后续首次失败继续独立保留，没有覆盖前一现场。
- 宿主结果现在接收 operation：只有显式 `cancel` 且 `cleanup.reaped=true` 才为成功，未回收取消及
  `exec` 被中断仍为失败；Windows `CREATE_NO_WINDOW` 无可信 console interrupt 的诊断继续留在
  `cleanup.errors`，没有吞错或放宽清理断言。Runtime 将成功显式取消视为成功的控制结算，不计普通
  command 成功/失败，也不设置 `command_observed_after_latest_mutation`；cancel completion observation
  可用但仍保持 `was_current_at_observation=false`，不能冒充正常退出或测试通过。运行中的 poll ID
  进入同一进程的有界 interaction provenance，不推进 workspace effect epoch；完成提示明确嵌套
  launch/poll ID 仅为来源上下文，除非也是顶层 `available_evidence`，不得直接引用。
- 宿主取消边界精确回归 **20/20**；Runtime 的取消聚合/完成证据两项每项 **20/20**，最终
  Agent Runtime **177/177**，completion Schema 定向回归、workspace fmt 与正式 Tauri 构建通过。
  最终二进制 `64d2f85813a8…` 使用全新 data/work/profile 和 StepFun Plan / `step-3.7-flash`：
  `start_process`、两次 `poll_process`、`cancel_process`、`report_completion` 全部 `is_error=false`；
  cancel 返回 `state=cancelled`、`success=true`、`reaped=true`，输出只含
  `W77_PROVENANCE_READY`，无自然退出标记。单回合 completed、5 个模型步、166 条 canonical 事件；
  `failed_commands=0`、`failed_tools=0`，最终可用 cancel observation 精确保存 launch call ID 与两条
  poll call ID，完成报告一次通过且如实把自然退出标记缺席列为 unverified。worker.ps1 前后 tree hash
  一致，PID/helper 为 0，应用/profile 清零并完成官方备份。
- 未覆盖 cancel 与自然退出的真实竞态、温和中断被忽略后的强制升级、child/grandchild、PTY、应用
  强退、其他 Agent/平台及 N3/100 seed/LONG/99%；不关闭完整 CMD-139/PROC-033 或共享阶段。

### cancel 与自然退出的单一终态（W78，基线 `a0f1674a7`）

- S-D04-08 / PROC-038、A13/A17/A19：新增两个暂停时间的 supervisor owner 回归，分别固定竞争
  双方的先后顺序。cancel 先进入 stop ownership、5 ms 后进程自然 reap 时，终态稳定为
  `Cancelled`；自然 reap fact 先被 waiter 观察、终态尚在 120 ms 输出 drain 时发起 cancel，终态
  稳定为 `Exited(code=0)`。
- 两种顺序均要求 `cleanup.reaped=true`，重复 cancel 与后续 terminal poll 逐字段等于首次终态，
  输出快照不改写，底层 wait/reap 只有一次；自然退出先胜时没有发送 interrupt/terminate/kill。
  生产实现首次即满足全部断言，本批只固化回归，没有修改产品信号、清理、超时或权限逻辑。
- 首次定向 **2/2**，两项各重复 **20/20**（共 40 次），Windows process Runtime lib
  **122/122**，workspace fmt 通过。真实 OS 调度竞争、两个并发 cancel waiter、cancel/poll/close
  三方并发、child/grandchild、PTY、其他平台及长期压力仍待验；不关闭完整 PROC-038 或共享阶段。

### close stdin、cancel 与 poll 三方并发（W79，基线 `c225443d3`）

- S-D04-09 / PROC-049、A13/A17/A19：可控 owner 将已经 admission 的 `close_stdin` 阻塞在异步
  平台调用中，再对同一 Session 并发启动 60 秒 poll 与 cancel。close 未占用 registry/session 锁；
  cancel 仍发送唯一一次 interrupt 并得到 `Cancelled`、`cleanup.reaped=true`，长 poll 及时唤醒并
  返回逐字段相同的终态；此时注入的 close 仍独立阻塞，释放后正常完成。
- 底层 `close_stdin`、interrupt、wait/reap 各调用一次，无重复 signal、终态改写或 waiter 遗失。
  生产实现首次满足断言，本批仅扩展测试 owner 与新增回归，没有修改平台 I/O、清理或权限逻辑。
- 首次定向 **1/1**，重复 **20/20**，Windows process Runtime lib **123/123**，workspace fmt
  通过。真实 pipe/PTY stdin 关闭、两个以上 cancel/poll waiter、平台 close 失败、应用退出及其他平台
  仍待验；不关闭完整 PROC-049 或共享阶段。

### 多 cancel waiter 的单清理所有者（W80，基线 `f7e31b0fd`）

- S-D04-10 / PROC-038、A13/A17/A19：barrier 同时释放 8 个对同一 Session 的 cancel 调用，验证
  `begin_stop` 只产生一个 leader，其余调用走 follower 等待路径。八个调用全部返回逐字段相同的
  `Cancelled`，`cleanup.reaped=true`，输出快照一致。
- 夹具只记录一次 interrupt 与一次 wait/reap，没有重复 terminate/kill、多个 terminal 或遗失 waiter。
  生产实现首次满足断言，本批仅新增回归，未改信号、清理或 admission 逻辑。
- 首次定向 **1/1**；8 waiter 场景重复 **20/20**（共核对 160 个返回）；Windows process Runtime
  lib **124/124**，workspace fmt 通过。真实 OS helper、多 Session 混合、cancel future drop 与平台
  信号失败组合、其他平台及长期压力仍待验；不关闭完整 PROC-038 或共享阶段。

### Unix 失败清理的 compare-and-unlink 窗口（W81，基线 `dca6ad842`）

- S-D03-50 / FILE-019/020/028/038、A05/A07/A13/A17/A19：在 WSL2 Ubuntu ext4 的确定性 hook
  中，既有目标在暂存完成后消失，迫使 publication 失败；错误清理先确认 `stage.tmp` 是本次 inode，
  随后 hook 将它移到 `retained-stage` 并在原名写入同字节外来文件。旧代码继续按名称 unlink，外来
  文件被删除，原暂存仍在 retained 名下，却报告 `temporary_cleanup_unconfirmed=false`。首次失败
  日志、observation、保留文件和缺失外来名均已复制到仓库外。
- Unix 没有可靠的 compare-and-unlink 原语。现在 publication 失败且暂存未被原子消费时，不再执行
  path-based 删除；保留暂存名供 owner 显式对账，并设置 `temporary_cleanup_unconfirmed=true`，使上层
  进入 outcome unknown，而不是把 Conflict 当作已安全清理。相同策略覆盖暂存字节被改写和 rollback
  guard 失败。Windows 仍通过持有的文件 handle 删除，行为不变；未扩大权限、吞错或自动重试。
- 修复后 WSL2 精确反例 **20/20**，Linux service **127/127**；Windows `nomifun-file` lib
  **362/362**，workspace fmt 通过。固定现场保留 8 字节 `stage.tmp`，目标缺失且清理明确未确认。
  暂存后续对账/清理、macOS 原生执行、其他 Unix portable fallback、磁盘/IO fault、完整 UI/角色及
  长期统计仍待验；不关闭完整 FILE Case 或共享阶段。

### Windows 启动事务内 deadline（W82，基线 `0eb31d623`）

- S-D04-11 / PROC-047、A08/A11/A13/A17/A19：使用真实 `CreateProcessW(CREATE_SUSPENDED)` 与
  Job owner，audit facade 将 assignment 阶段延迟 200 ms，共享 process deadline 设为 50 ms。
  deadline 在进程尚未 resume 时到达；事件精确停在 `[Created, Assigned]`，从未调用
  `ResumeThread`，用户命令的 marker 没有创建。
- 事务返回可证明用户代码零执行的 `SpawnFailed`；保留的精确进程 handle 已 signaled，说明 suspended
  child 在结果返回前完成回收。首次实际耗时约 220 ms 且始终小于唯一 5 秒 setup 上限，没有在
  assignment 返回后重新授予一轮 setup timeout。生产实现首次满足，本批只新增 fault-injection 回归。
- 首次定向 **1/1**，重复 **20/20**，Windows process Runtime lib **125/125**，workspace fmt
  通过。Unix/macOS 启动阶段、deadline 刚过 resume 的 `StartLost`、清理证明失败、正式 UI/角色及
  长期压力仍待验；不关闭完整 PROC-047 或共享阶段。

### 运行中 deadline 的可观察部分效果（W83，基线 `1d20fad57`）

- S-D04-12 / PROC-048、A05/A08/A10/A11/A13/A17/A19：跨平台真实 helper 先把固定字节
  `partial effect before timeout\n` 写入工作目录，再睡眠 60 秒；process deadline 设为 1 秒，
  interrupt/terminate/reap grace 为 50/50/500 ms。
- Windows Job 与 WSL2 Linux process group 都返回 `TimedOut` 且 `cleanup.reaped=true`；独立磁盘
  oracle 在终态后仍读取到完全相同的部分文件。Runtime 没有删除该文件或把 timeout 表示成回滚。
  生产实现首次满足，本批只新增 helper 子命令与公开 supervisor 回归。
- Windows/Linux 精确场景各重复 **20/20**；Windows process contract **11/11**，Linux
  **12/12**，workspace fmt 通过。持续写入时的截断/游标、child/grandchild、macOS、正式 UI/模型
  对部分效果的披露、IO fault 与长期压力仍待验；不关闭完整 PROC-048 或共享阶段。

### 临时 cleanup 探测失败的 authority 保留（W84，基线 `fa5519a72`）

- S-D04-13 / PROC-050、A08/A11/A13/A17/A19：现有 Windows Job 故障注入确认首次 member
  snapshot、supplemental snapshot 或 terminate 失败不会被误判为永久失权；精确 process handle 与
  Job authority 保留，后续重试收敛为 exact cleanup。相关三项、架构唤醒/typed retry 合同和通用
  authority 分类器均通过。
- WSL2 Unix relay 在 group quiescence 暂未证明时返回 Retry，保持 `CleanupOwned` 和 Running
  completion；ECHILD 明确证明 exact child identity 丢失时才 quarantine，且不向缓存 PGID 发信号。
  生产实现首次满足，未修改分类、重试或清理逻辑。
- Windows 临时 snapshot 代表场景重复 **20/20**；Unix retry 代表场景重复 **20/20**，两平台的
  永久失权负向与分类器另各通过。macOS 真实 relay、跨重启 quarantine、组合故障、正式 UI/角色和
  长期压力仍待验；不关闭完整 PROC-050 或共享阶段。

### 并发 start 的 admission 前容量预留（W85，基线 `7ccf1def6`）

- S-D04-14 / PROC-042、A03/A11/A13/A17/A19：公开 supervisor 容量设为 2，以 barrier 同时释放
  32 个真实 `write-pid-then-sleep` helper。Windows Job 与 WSL2 Linux process group 都只创建 2 个
  PID marker，其余 30 个调用返回 `capacity_exhausted`；磁盘 oracle 没有发现短暂执行过的超额 helper。
- 两个已准入进程均以 `Cancelled`、`cleanup.reaped=true` 结算；随后第三个 helper 能重新 start 并
  清理，证明失败 start 不占额且终态 cleanup 释放额度。生产实现首次满足，本批只补 helper/回归。
- Windows/Linux 精确竞争各重复 **20/20**（每轮 32 个 start）；Windows process contract
  **12/12**，Linux **13/13**，workspace fmt 通过。start/shutdown 竞争、Lost/quarantine 占额、PTY、
  macOS、不同 owner 混合及 1,000 次长稳仍待验；不关闭完整 PROC-042 或共享阶段。

### 最小定向测试与首步仓库指令（W86，基线 `634508f0d`）

- S-D04-15 / CMD-138、A02/A08/A17/A19：隔离 Bun 仓库的 `AGENTS.md` 要求只运行
  `bun test tests/sum.test.ts`，目标测试输出 `W86_TARGET_PASS` 并通过；完整 suite 会运行带
  `W86_UNRELATED_TOUCHED` 哨兵的无关失败测试。首次正式 Tauri 虽最终只启动正确命令，但模型先把
  `AGENTS.md` 与源码作为普通读取提交；Runtime 此时才发现 scope，只能原子延迟整批，UI 出现一次
  异常。增强 read 工具说明后第二次仍复现；再补 system instruction 后第三次仍复现，且首次完成
  报告的 `requirement_ids=[]` 被严格 Schema 拒绝。三个首次失败及中间编译失败均独立保留在仓库外。
- 根因是 `ScopedInstructions::new` 已登记根目录，却以 `dirty=false` 开始，第一次 `before_model`
  因此不物化根指令。现让已知根在首个模型请求前经正式 `instruction_scope` 入口装载，首批源码读取
  直接复用该 observation；工具与 system 说明继续明确 scope 文件不作普通读取。completion Schema
  同时要求显式 `requirement_ids` 至少一项，覆盖全部接受需求时省略字段，不接受空数组。
- 根预载、completion Schema、system 合同与 read 工具说明四项分别重复 **20/20**，Agent Runtime
  **180/180**。最终正式二进制 `b982a32a93d69bdacb982714cbdf65139403fdb6d536e9c49e7117d7990eb6bf`
  使用全新 data/work/profile 和 StepFun Plan / `step-3.7-flash`；单回合 completed、4 个模型步、
  137 条 canonical 事件，调用精确为 `read_file(format=instruction_scope)`、一次目标
  `start_process`、一次 `poll_process`、`report_completion`，全部成功且 UI 零异常。实际测试
  **1 pass/0 fail**，无无关文件名或哨兵，进程 `reaped=true`；工作区前后 tree hash 均为
  `ad6833dc5643e0a3586a8e8bc04fa65463468286be5f1104c12e258fd68fa911`，helper、应用与 profile
  进程清零，独立断言通过并完成正式备份。
- 其他 package manager、语言、仓库布局、失败目标/超时、其他 Agent/平台，以及 N3/100 seed/
  LONG/99% 仍待验；不关闭完整 CMD-138 或共享阶段。

### session lease 到期、续期与精确回收（W87，基线 `87e7de6f3`）

- S-D04-16 / PROC-043、A03/A08/A11/A13/A19：复核公开 supervisor 的现有 lease 合同。真实进程
  即使无人 poll，持续输出也会续期；owner 认证的 poll、write、status 各自在原始 lease 过期后仍
  保持 Session。无活动 Session 到期后先转入 retirement，完成 cancel 与 exact reap 后才从 registry
  移除；进行中的动作阻止 reaper claim，未 reap 的 retirement 不释放容量。
- 生产实现首次满足，本批未修改 lease、信号或 registry 逻辑。Windows 与 WSL2 Linux 对输出续期、
  三种 owner 动作续期、到期回收、动作/回收互斥及 reap 前容量保留六项分别重复 **20/20**，每平台
  共 120 次；两平台 `session_registry` 均 **12/12**。独立日志计数确认零失败，测试结束后两平台
  `process_test_helper` 数均为 0。
- 首次 WSL 内联脚本被宿主引号解析损坏，在任何产品测试前退出；失败摘要独立保留，改用仓库外
  落盘 Bash 脚本和 Linux 专用 target cache 后通过，未把夹具失败记成产品结果。
- PTY/resize/close-stdin 的显式续期、lease 边界的真实 OS 调度竞争、macOS、应用强退、PROC-044
  的连续 1,000 个短进程、正式 UI/角色及长期统计仍待验；不关闭完整 PROC-043/044 或共享阶段。

### process tree 升级、后代与 authority lost（W88，基线 `038a30b0a`）

- S-D04-17 / PROC-034～037、A08/A11/A13/A17/A19：Windows Job 的真实 cancel 在
  `CREATE_NO_WINDOW` 无可信 console interrupt 时继续升级，并在 5 秒合同内精确回收 leader 与
  grandchild；leader 先正常退出时，Job 后代清理完成前不发布 success，最终仍如实保留 exit 0 与
  `cleanup.reaped=true`。
- WSL2 Linux process group 的普通 cancel 清理 leader + grandchild；整组忽略 SIGINT 时等待完整
  一秒 grace，再升级 SIGTERM，未提前 force kill。leader-first 场景在同组后代回收后才返回 exit 0；
  `setsid` 逃逸使输出管道无法证明 EOF 时，在一秒内明确返回 `Lost` 与 reader timeout，不伪造清理，
  随后由独立测试 oracle 清掉逃逸进程。
- 生产实现首次满足，本批未修改信号、owner 或清理逻辑。Windows 两项各重复 **20/20**，WSL2
  四项各重复 **20/20**，共 120 次；独立日志计数零失败，结束后两平台 `process_test_helper` 均为 0。
- ConPTY/PTY、SIGTERM 也被忽略后的 force-kill 级升级、macOS watchdog/session、父进程死亡、
  正式应用入口、其他 Agent/角色及长期压力仍待验；不关闭完整 PROC-034～041 或共享阶段。

### shutdown start gate 与父宿主死亡清理（W89，基线 `5c485e3f7`）

- S-D04-18 / PROC-040/041、A03/A11/A13/A17/A19：公开 supervisor 的正常 shutdown 先关闭
  start gate，清理两条真实活动 Session；报告逐一保留原 owner/session 和 Cancelled/Lost 终态，
  两个 PID 均消失。随后尝试启动写 marker 的 helper，在物理 spawn 前返回
  `supervisor_shutting_down`，marker 不存在。Windows 与 WSL2 Linux 分别重复 **20/20**。
- 父宿主直接退出的独立 harness 中，Windows 关闭带 `KILL_ON_JOB_CLOSE` 的实际 process Job 后，
  leader 与 grandchild 都终止，外层 fallback Job 变空；重复 **20/20**。Linux pipe process-group
  watchdog 与外部 PTY-session watchdog 各重复 **20/20**，subreaper 精确回收 leader、grandchild
  和 watchdog；PTY leader 只接受 SIGHUP/SIGKILL，其他 owned member 为 SIGKILL。
- 生产实现首次满足，本批未修改 start gate、Job/watchdog 或 shutdown report。独立日志计数共
  100 次零失败，结束后两平台 `process_test_helper` 与 `parent_death_harness` 均为 0。
- 真实 Tauri 主进程的正常/强退、Windows ConPTY parent-death、macOS watchdog/session、start
  事务与 shutdown 的真实竞争、其他 Agent/角色及长期压力仍待验；不关闭完整 PROC-040/041 或共享阶段。

### 连续 1,000 个短进程的资源与输出长稳（W90，基线 `c48e31f1d`）

- S-D04-19 / PROC-044、A05/A11/A13/A15/A19：新增 `process_soak` 手工回归，默认标为 ignored，
  不增加普通套件耗时；显式运行时用容量 8 的公开 supervisor 连续启动 1,000 个真实 pipe helper。
  每个 helper 必须 exit 0、signal none、`cleanup.reaped=true`，固定 32 字节输出逐字节一致且
  `dropped_bytes=0`。全部完成后 shutdown report 必须为空，证明自然终态未被误记为 shutdown cancel。
- 首次编译因夹具把 `start(self: &Arc<Self>)` 写成 `&ProcessSupervisor` 而失败，日志独立保留；改为
  公开 Arc receiver 后通过。首次和最终两轮在 Windows、WSL2 Linux 均 **1,000/1,000**，共执行
  4,000 个短进程。最终 Windows 用时 178.25 秒，handle **111→111**、thread **13→10**；Linux
  用时 171.79 秒，fd **11→11**、thread **8→8**。两端 helper 均为 0，生产逻辑无需修改。
- 默认运行在两平台均为 0 passed / 1 ignored；显式 `--ignored --exact` 才执行长稳。独立断言首版
  曾把数值 0 当成布尔 false 而错误退出 1，保留后按布尔与计数分开核对，最终 10/10 通过。
- 额外 `cargo clippy -D warnings` 被该 crate 现有的 range loop、collapsible if 和两处参数数目 warning
  阻断；失败日志保留，本批不扩大到无关生产重构。两平台 rustc 编译、workspace fmt 与 diff 检查通过。
- 并发短进程、PTY/ConPTY、macOS、真实应用/UI、系统级内存采样及更长 soak 仍待验；不关闭完整
  PROC-044 或共享阶段。

### Unix 启动事务内 setup deadline（W91，基线 `1608e82be`）

- S-D04-20 / PROC-047、A08/A11/A13/A17/A19：在 WSL2 Linux 的确定性 fault hook 中，spawn gate
  被占用 300 ms 时，新 start 只消费原 100 ms setup deadline，结果在 250 ms 内返回；blocking worker
  完成退出，watchdog PID 保持 0，用户 marker 未创建。另把 blocking transaction 卡在 fork 前，
  75 ms deadline 后保守返回 `StartLost`；释放 worker 后 leader/watchdog 仍均为 0，marker 不存在。
- 第三条在 watchdog 已创建后扣留 ACK。唯一 100 ms setup deadline 内返回 `StartLost`，总耗时小于
  350 ms，没有重新授予 cleanup budget；用户代码从未执行，exact watchdog 随后只 reap 一次并消失。
  三项生产实现首次满足，首轮 **3/3**，每项重复 **20/20**，最终残留 owner 进程为 0。
- 首次内联 WSL 命令因宿主引号损坏而把结果目录解析为空，在任何 Cargo 测试前退出；夹具失败独立
  保留，改用落盘 Bash 脚本后通过。本批未修改 Unix 启动事务。
- macOS、PTY setup deadline、fork/exec 临界点的真实调度竞争、正式 Tauri/UI/角色及长期压力仍待验；
  不关闭完整 PROC-047 或共享阶段。

### 同一回合连续 50 个基础命令（W92，基线 `68658614a`）

- S-D04-22 / CMD-143、A02/A08/A17/A18/A19：首个隔离启动直接复制旧 data，因复制了数据集/work-root
  身份而被正式冲突检查拒绝，模型调用为 0；停机和备份失败均保留。改用官方 backup restore 轮换
  storage generation 后，首次有效正式 Tauri 回合完成 W92-001～050，但用了 56 个模型步骤和 55 次
  `exec_command` 提案。step 1/18/34/36 使用无效 `cmd + args`，step 17 提交空对象；五次都在 Schema
  预检被拒且零 dispatch，实际 process effect 恰为 50。最终 completion 仍宣称“可见失败数 0”。
  Session `01a0eb2b-9768-7e01-8d08-b0a74d232b15`、2,499 条事件、截图、数据库与轨迹完整保留；50 个
  成功 marker 仍精确有序，工作区不变，cmd 残留 0，应用/profile 清零并正式备份。
- 根因有两层：`exec_command` 的严格 union 已区分 shell-script `cmd` 和 executable `command + args`，
  但字段说明缺少两个完整 JSON 形状的就地对照，step-3.7-flash 在长历史中反复回退；Runtime 已累计
  `failed_tools=5`，completion 上下文、Schema 和报告却没有该计数，恢复后可以错误写成 0。现把 args
  明确限定为 command 专属，并给出 `command=cmd.exe` 示例；completion 动态暴露并在非零时强制 exact
  const `observed_tool_error_count`，检查后持久化。最终交付由 Runtime 固定追加错误数，后续成功不抹除。
- 新增/调整六项精确回归各重复 **20/20**。Agent Runtime 首次全量因旧失败夹具未带新计数而
  **176/182**，现场保留；夹具改为披露真实累计错误并补旧报告兼容，合并远端后 **187/187**。workspace fmt、diff 与正式
  Tauri 构建通过；构建仅有既有 dead-code warning。
- 额外 `native_coding_reliability` 为 **1/2**：coding fixture 把无 tools 的压缩请求误判成 Provider 故障；
  在纯远端 `e7d6a85d0` 上同一 Case 同样失败且 Schema 更大，确认不是 W92 回归，原日志保留并继续开放。
- 合并远端并重跑 187 项后，最终二进制 `712cf9457e11f2b9f3a6295bf961d5e451c5d3ee250e6049ee52fde26dca2408` 使用
  新 data/work/profile 和 StepFun Plan / `step-3.7-flash`。Session
  `01a0eb51-96ab-7062-a108-520f33641f3f` 单回合 completed、51 个模型步骤、2,454 条 canonical 事件；
  step 1～50 各有且只有一个 `exec_command`，均为 `command=cmd.exe` 与实际 JSON args，W92-001～050
  逐项 exit 0、`reaped=true`、零 dropped，step 51 一次 completion。工具/UI 错误为 0、50 个 effect
  均 returned、completion 显式记录错误数 0、工作区 tree hash 前后同为 `23e5afef…c308`、cmd 残留 0，
  应用/profile 清零并正式备份。
- 其他基础命令语义、其他 Provider/模型/Agent、macOS、50 个独立用户回合、N3/100 seed/LONG/99%
  仍待验；不关闭完整 CMD-143/144 或共享阶段。

### 参数预检失败后的恢复与错误披露（W93，基线 `a1b4f965e`）

- S-D04-24 / OBS-002、REAL-021～024、A02/A08/A17/A18/A19：复用 W92 最终二进制
  `712cf9457e11f2b9f3a6295bf961d5e451c5d3ee250e6049ee52fde26dca2408`，以新的正式 Tauri
  data/work/profile 和 StepFun Coding Plan / `step-3.7-flash` 执行负向协议。step 1 提交规定的
  `exec_command {cmd:"cmd.exe",args:[...]}`，得到 `INVALID_TOOL_ARGUMENTS` 与
  `status=not_executed`；数据库中没有对应 effect，确认零 dispatch。step 2 使用合法
  `{command:"cmd.exe",args:[...]}`，输出精确为 `W93-RECOVERED`、exit 0、`reaped=true`，唯一
  `workspace.process/exec` effect 为 returned。
- step 3 只调用一次 `report_completion`，其 `observed_tool_error_count=1`；模型摘要明确无效调用未
  执行、恢复命令成功和可见错误数为 1，最终 UI 又显示 Runtime 固定披露：后续成功没有抹除该错误。
  Session `01a0eb67-97f3-7542-997f-4e099fb6a9d9` 单回合 completed、3 个模型步骤、98 条 canonical
  事件；独立 24 项断言通过。工作区前后仅含同一 `AGENTS.md` 且 tree hash 均为
  `e265d753a82bbfdb88c5d7c6bb599e4187e204c7ee90c5792ecd216a79c0d9a0`，匹配 cmd 残留 0，应用/
  profile 进程清零，正式备份完成。本批无需生产代码修改。
- 只覆盖一种参数预检错误及该模型/Agent；业务工具错误、权限/超时/取消/unknown、其他 Provider/
  模型/角色、macOS 与 N3/100 seed/LONG/99% 仍待验；不关闭完整 REAL/OBS 或共享阶段。

### 非零命令恢复后的失败披露（W94，基线 `a3a2b4f96`）

- S-D04-25 / PROC-015、OBS-004、REAL-004/021～024、A05/A08/A17/A18/A19：`AgentWorkStatus`
  原已把非零、timeout/lost 等终态计入 `failed_commands`，completion 上下文虽显示该数，Schema、
  持久报告和最终交付却只约束 `failed_tools`；恢复成功后可遗漏早先命令失败。新增精确回归首次
  **0/1**，确认 `failed_commands=2` 时没有强制字段。现加入动态 exact const
  `observed_command_failure_count`，非零时必填并校验，旧报告缺字段按 0 兼容；报告新鲜度同时绑定
  两种计数，Runtime 固定追加命令失败披露。多个计数同时必填时，工具说明给出含全部字段的单个
  精确 JSON 对象，避免模型只复制其中一项。
- 首个正式二进制 `743ec8da509eb24016d3b234f6075d9e0d2c562d6d36b5c8acf6dbd4a172862c`
  使用相同隔离夹具：step 1 的真实 `cmd.exe /d /c exit /b 7` 返回 exit 7、`reaped=true`；step 2
  更新计划，step 3 恢复命令输出精确 `W94-RECOVERED`。step 4 首次 completion 只带
  `observed_command_failure_count=1`，漏掉同时必填的工具错误数，被 Schema 在执行前拒绝；step 5
  补齐后完成。Session `01a0eb76-5f90-7fa2-9167-1e2135cb96f2`、158 条事件和两次报告完整保留，
  分类 `FAIL_RECOVERED`，不以最终成功覆盖。
- 合并计数提示后的二进制 `32d59edc3f352f6b324ffa7f4374e512530c788048a081fcb3f50f2a0affedd7`
  使用完全相同的 AGENTS/prompt、全新 data/work/profile 和 StepFun Coding Plan / `step-3.7-flash`。
  Session `01a0eb7c-64ac-7ac2-8aa4-f590a215cdcf` 单回合 completed、4 个模型步骤、147 条 canonical
  事件；调用精确为失败命令、一次 `update_plan`、恢复命令和一次 `report_completion`。首次报告即
  同时提交 tool/command 两个计数 1；最终 UI 固定显示两种失败均未被后续成功抹除。两个 process
  effect 均 returned，工作区前后 tree hash 同为 `3175c7dc…99ef`，cmd、应用和 profile 进程清零，
  两次运行均正式备份；最终独立 **28 项**断言通过。
- 三项新增精确回归各 **20/20**（60 次断言执行），Agent Runtime **190/190**，workspace fmt、
  diff 和正式 Tauri 构建通过；仅有既有 warning。其他非零码/信号、timeout/lost、多个失败、其他
  Provider/模型/角色、macOS 与 N3/100 seed/LONG/99% 仍待验；不关闭完整 PROC/REAL/OBS 或共享阶段。

### 原生 Coding 回归中的压缩请求分类（W95，基线 `93b5f58e9`）

- S-D01-03 / LONG-008、ACOD-017、A05/A09/A15/A17/A19：重跑 W92 开放的
  `native_coding_reliability`，当前基线仍为 **1/2**。Coding Case 首个任务请求有 22 个工具、Schema
  26,518 bytes；写入后的合法 compaction 请求按设计不带工具，旧 scripted provider 在取
  `body.tools` 时 panic，三次 provider 重试后 Turn 暂停为 `EXECUTION_MODEL_PROVIDER_UNAVAILABLE`。
  首次失败日志完整保留。
- 夹具现只在请求含压缩专用提示且工具面为空时走摘要响应，并把 task request 与 compaction request
  分开计数。首版修补进一步暴露压缩后原始 `read-game` tool message 已被移出当前窗口；该中间
  `FAIL` 及一次测试编译 typo 均保留。最终逻辑优先核对原 tool message；若已压缩，则要求确有压缩
  请求且完成上下文仍列出 `available_evidence` 和 `gomoku/index.html`。既有磁盘全文、单次写入、
  非 Git 工作区、completion、unverified 披露、effect/Turn 终态和 Browser 零启动断言没有移除。
- 最终 no-capture 轨迹为 3 个任务请求 + 1 个 24,559-byte compaction 请求；Provider 压缩计数与
  canonical `compaction_started` 精确相等且不超过 1。完整测试文件 **2/2**，原失败 Coding Case
  **20/20**，workspace fmt 与 diff 检查通过；仅有既有 warning。本批未修改生产代码、未调用真实
  模型或 UI。20 次连续压缩、真实 Provider、pause/restart、其他 Agent/平台及 LONG/99% 仍待验，
  不关闭完整 LONG-008/ACOD-017 或共享阶段。

### 运行中 timeout 恢复后的失败披露（W96，基线 `c2eeab594`）

- S-D04-26 / G0-023、PROC-048、REAL-021～024、A05/A08/A11/A13/A17/A18/A19：复用 W94
  最终二进制 `32d59edc3f352f6b324ffa7f4374e512530c788048a081fcb3f50f2a0affedd7`，以新的
  正式 Tauri data/work/profile 和 StepFun Coding Plan / `step-3.7-flash` 执行真实 deadline 负向。
  step 1 精确调用 `cmd.exe /d /c "ping -n 30 127.0.0.1 >nul"`、`timeout_ms=250`；终态为
  `timed_out`，`interrupt_attempted=true`。CREATE_NO_WINDOW pipe 没有可信 console interrupt 后
  升级 `terminate_attempted=true`，未 force kill，1,174 ms 内 `reaped=true`。
- step 2 按 Runtime 要求单独 `update_plan`，step 3 恢复命令 exit 0 并输出精确
  `W96-RECOVERED`；step 4 首次且仅一次 `report_completion` 同时提交
  `observed_tool_error_count=1` 和 `observed_command_failure_count=1`。最终摘要明确 timeout 已回收，
  UI 固定追加两种累计失败披露。Session `01a0eb8c-1f34-72f1-9591-bee302dac00e` 单回合 completed、
  4 个模型步骤、147 条 canonical 事件；两个 `workspace.process/exec` effect 均 returned。
- 独立 **35 项**断言通过；工作区前后 tree hash 均为 `f32f2489…c186`，`ping.exe`/匹配 cmd 残留 0，
  应用、profile 和 Vite 进程清零，正式备份完成。本批无需产品代码修改。未覆盖外部文件部分效果、
  force-kill/lost、其他命令/Provider/模型/角色、macOS 与 N3/100 seed/LONG/99%；不关闭完整
  PROC-048/REAL 或共享阶段。
- W103 后续语义审计确认 W96 的 timeout criterion 和 recovery criterion 均引用 recovery call ID；
  因此 W96 的 35 项只通过计数、终态与清理，完成证据关联子断言追记为失败并转 S-D02-19 修复。

### typed non-start 后的控制可见性与失败披露（W97，基线 `45aef48b3`）

- S-D04-27 / PROC-011/012、CTRL-006/007、REAL-011/021～024、A02/A05/A08/A17/A18/A19：首次
  正式 Tauri 中，`w97-command-does-not-exist.exe` 返回
  `nomifun.process-start-observation.v1`、`state=not_started`、`user_code_started=false`，没有 shell
  fallback。该事实按设计不失效工作区证据、也不要求 replan，但 TaskLedger 同时保持隐藏；模型在
  step 3/5 分别用 ToolSearch 查找 `update_plan`/`report_completion`，均得到无匹配。第一次恢复命令
  已 exit 0 后，模型仍在 step 6 重放 non-start，随后才得到控制工具并又执行一次恢复。
- 失败 Session `01a0eb90-ce3d-72a0-a6f9-62891bc6e8eb` 最终 completed，但用了 9 个模型步骤、
  252 条 canonical 事件和 4 个 process effect；non-start 与恢复各执行两次。完成报告的机器字段正确
  为 tool error=2、command failure=0，模型摘要仍写 1，Runtime 固定披露纠正为 2。完整截图、事件、
  数据库、两组相同 input digest、工作区和零进程残留均保留，分类 `FAIL_RECOVERED`。
- 根因是 `failed_process_observation` 有意排除可证明 non-start，后续 ledger 激活也因此被跳过。现只在
  typed non-start owner result 后激活 Ledger/Completion 控制；不设置 `needs_replan`，不把它计为
  command terminal，也不改变权限、effect 或证据失效规则。新回归首次 **0/1**，修复后 **20/20**，
  Agent Runtime **191/191**，workspace fmt、diff 和正式 Tauri 构建通过；仅有既有 warning。
- 修复后二进制 `ec25c3407f810cb5d7252d2d72d6adaecb5be38ffef1f622e04fb92955093abb`
  使用完全相同的 AGENTS/prompt、全新 data/work/profile 和 `step-3.7-flash`。Session
  `01a0eb99-c4f9-70f3-9411-becab7ea0c13` 首次精确执行 non-start→`update_plan`→一次恢复→一次
  completion，共 4 个模型步骤、137 条事件、2 个 returned effect；报告和 UI 一致显示 tool error=1、
  command failure=0。独立 **31 项**断言、工作区 tree hash `c19cd9c3…7a5b`、应用/profile/Vite 与
  匹配进程清零、正式备份均通过。其他 spawn permission/format/cwd、Provider/模型/角色、macOS 与
  N3/100 seed/LONG/99% 仍待验；不关闭完整 PROC/REAL 或共享阶段。

### General 条件工具面下的压缩与发现回归（W98，基线 `1372d1c86`）

- S-D01-04 / REG-005/008、LONG-008、AGEN-002/005、ACOD-017/019、A01/A05/A09/A15/A17/A19：
  首次用 `browser-use,computer-use` 编译并运行完整 `native_coding_reliability`。四个非忽略场景分别
  覆盖 Coding 嵌套文件、General 原生伪 tool-call 纠错后写入、General 先用 ToolSearch 发现
  `browser/navigate` 再完成文件任务，以及 delegate child provider 失败及时传回 scheduler。
- 首轮 **4/4**；两个需要显式本地 Provider 且会产生模型用量的 live 用例保持 ignored，没有折算为
  通过。条件套件随后完整重复 **20/20**，共 80 个非忽略测试执行零失败。各轮保持 selected workspace
  隔离、单次 write、压缩请求与 task request 分账、completion/unverified 披露、暂停错误分类和
  Browser Runtime 启动计数 0；绑定/发现能力没有提前启动重型运行时。
- 本批只做验证，无源码修复或真实模型/UI 调用；首次条件编译 3 分 34 秒，仅有既有 warning。
  被忽略的真实 Provider wire/live Case、真实 Browser/Computer Action、正式 UI、多次压缩、其他
  Agent/平台与 N3/100 seed/LONG/99% 仍待验；不关闭完整 Case 或共享阶段。

### StepFun streaming/non-streaming 原生工具 wire（W99，基线 `cfb4cfbd0`）

- S-D01-05 / MODEL-002/003/029/034/035、A01/A02/A09/A16/A17/A19：使用 W97 修复后隔离数据中的
  加密 StepFun connection 和 Session binding。read-only probe 通过正式 App 路由/Provider 编码器先把
  原请求发到本地 capture server，再把完全相同的 body 收窄为唯一 `exec_command`，分别用
  streaming 与 non-streaming 直连 `step-3.7-flash`。测试代码只解析返回 wire，不把 tool-call 送入
  Kernel/owner，避免任何命令副作用。
- 两种请求均 HTTP 200、`finish_reason=tool_calls`，各恰好返回一个原生 `exec_command`；函数参数均为
  可解析 JSON，`text_tool_markup=false`、`content_bytes=0`、`tools_executed=0`。streaming 用时约
  1,398 ms，non-streaming 约 992 ms，独立 **18 项**断言通过；`wire-shapes.json` SHA-256 为
  `4ec3314323456c28ba3fc16f96e53fdd5037fc53e9b87d285bc6ae8379391fdb`。
- 凭据只从源数据的加密配置在进程内解密，没有进入环境变量、命令行、日志或制品；输出仅含脱敏
  形状统计。本批无源码修复或 UI/工具执行。仅一次真实 Provider 样本，未保留脱敏原始 frame，
  其他协议/模型、正式 UI/owner、N3/100 seed/failover 与 LONG/99% 仍待验；不关闭完整 MODEL Case
  或共享阶段。

### StepFun 原生工具 wire N3（W100，基线 `474a95ef6`）

- S-D01-06 / MODEL-002/003/029/034/035、A01/A02/A09/A16/A17/A19：以两个新的隔离 App、
  capture server 和输出目录重复 W99 的 exact `step-3.7-flash` / `exec_command` 只读 probe。每个样本
  仍分别发送 streaming 与 non-streaming 请求，源数据库只读、凭据仅在进程内解密，所有返回工具
  只解析而不进入 Kernel/owner。
- 连同 W99 共 **3/3** 独立样本、6 个真实 Provider 请求：全部 HTTP 200、
  `finish_reason=tool_calls`、每次一个原生 `exec_command`、arguments JSON 合法、
  `text_tool_markup=false`、`content_bytes=0`、`tools_executed=0`，共 **54 项**断言通过。streaming
  用时 1,398/1,302/1,407 ms（中位 1,398），non-streaming 为 992/1,587/1,712 ms（中位 1,587）。
- **仅该精确 route/model/function 的 wire-only 场景达到 N3**。未覆盖 owner 工具执行、其他工具/
  Provider/协议/模型、脱敏原始 frame、正式 UI、100 seed/failover/LONG/99%；不关闭完整 MODEL Case
  或共享阶段。本批无源码修改。

### 连续命令的精确完成证据关联（W101，基线 `cbe731f1f`）

- S-D02-17 / CTRL-006/007、A05/A09/A15/A17/A18/A19：正式 Tauri 依次执行
  `echo W101-ALPHA` 与 `echo W101-BETA`，两者均首次 exit 0、`reaped=true`。首次二进制中第二个
  command 推进 workspace epoch 后，第一条已结算命令不再进入 `available_evidence`；模型虽保留两段
  结果正文，却只能看到 BETA 的顶层 call ID，并在推理中把它误认成 ALPHA。step 3/4/5 各耗尽
  4,096 output tokens，最终 `NOMIFUN_TASK_INCOMPLETE`，没有 completion delivery。
- 失败 Session `01a0ebaa-07d6-7ee1-95ff-01bc10cefd80` 共 5 个模型步骤、142 条 canonical 事件、
  3 次 `model_output_truncated`；两条命令各只执行一次、2 个 effect 均 returned，工作区和进程清理
  正确，但产品 Case 仍为 `FAIL`。截图、数据库、reasoning/事件轨迹和正式备份均独立保留。
- 根因是 `CompletionTracker::is_usable` 将所有 command observation 与当前 workspace epoch 绑定。
  现只让当时已经可用、exit 0、cleanup 已证明、无省略交互且 identity/epoch 自洽的命令结果跨后续
  command 保持可引用；该引用仅证明自身 scope/exit/output，不延续文件或当前 workspace 状态，也不
  复活旧 completion。新增回归首次 **0/1**，修复后 **20/20**，Agent Runtime **192/192**，workspace
  fmt、diff 及合并远端 macOS 文件修复后的正式 Tauri 构建通过；仅有既有 warning。
- 修复后二进制 `4df5759d4b921e43332f089c1224f5b8a5aaf74fc20680d88ca79ae9a57b2918`
  使用完全相同的 AGENTS/prompt、全新 data/work/profile 和 `step-3.7-flash`。Session
  `01a0ebbf-60c1-7c71-9e8a-4b6a149565ec` 精确 3 步：ALPHA、BETA、一次 completion；ALPHA criterion
  只引用 `chatcmpl-tool-b8eb8d9d75106d28`，BETA 只引用 `chatcmpl-tool-af6387d3195fe2e9`，未交换、
  合并或复用。132 条事件、零截断/错误、2 个 returned effect，独立 **31 项**断言，工作区 tree hash
  `2c0f7ae0…fdf1`、应用/profile/Vite 与匹配进程清零、正式备份均通过。非零/取消/交互链、64 项淘汰、
  压缩恢复、其他 Provider/角色/平台及 N3/100 seed/LONG/99% 仍待验；不关闭完整 CTRL 或共享阶段。

### 预期非零命令作为精确完成证据（W102，基线 `c46c4a8cb`）

- S-D02-18 / CTRL-006/007、PROC-015、OBS-004、REAL-004、A05/A08/A09/A17/A18/A19：Completion
  原只允许 `successful=true` observation 进入引用集合，结构化 exit 7 即使 `cleanup.reaped=true` 也只能
  标成 unverified。新回归首次 **0/1**。首版按“已结算命令”开放后，合成夹具 **20/20**、Runtime
  **193/193**，但真实正式 Tauri 揭示 `is_error=true` 的 launch 不取得 workspace-current provenance；
  `report_completion.evidence_call_ids` 仍被动态 Schema 设为 `maxItems=0`。
- 中间二进制 `5e863ad2d73e54330b3bd74df9b3375a57aa9c9b3bde5478977a242d1e1716e1` 的 Session
  `01a0ebc9-f3cd-7bb0-9678-f79c6335842b` 首次 report 正确引用 exit 7 call，却在预检拒绝。模型随后
  4 次只读 `search_tool_history`、7 次 4,096-token 截断，并在 step 12 违反“不重试”再次执行 exit 7；
  为控制模型用量在 step 17 从正式 UI 停止。289 条事件、2 个相同 input digest effect、截图/数据库/
  轨迹和取消终态均保留，分类 `FAIL_CANCELLED`；首次停应用时 1 个 WebView 短暂残留也保留，3 秒后
  精确清零并正式备份。
- 最终以同一 launch call ID、结构化已知 exit、`cleanup.reaped=true` 和无省略交互认定命令终态事实，
  不再要求 workspace-current 标记。该证据只支持命令自身 scope/exit/output；非零不变成成功，也不
  延续文件状态。修正后的生产形状回归 **20/20**，Agent Runtime **193/193**，workspace fmt、diff 与
  正式 Tauri 构建通过；仅有既有 warning。
- 修正后首个二进制 `a21c1ac9…e50` 的 Session `01a0ebd5-c7f3-7173-9c21-251f0d202bd0` 已精确
  通过。随后同步远端 macOS supervised hook 的共享 host 改动并重建；最终二进制
  `5942c2a581421d25c4d3a08a9805101e6f35712f51cdde6e1588192c50a4718e` 使用相同 AGENTS/prompt、
  另一套全新 data/work/profile 和 `step-3.7-flash`，Session
  `01a0ebe1-e458-70c2-b161-e66f2a4685d2` 仍精确为一次 exit 7→一次 `update_plan`→一次 supported
  completion，共 3 个模型步骤、100 条事件、1 个 returned effect；criterion 只引用失败 command call，
  tool/command 失败计数均为 1，零历史检索/截断/重试。独立 **27 项**断言、工作区 tree hash
  `f45956b6…a90d`、应用/profile/Vite 与匹配进程清零、正式备份均通过。timeout/cancel/交互终态、
  其他 Provider/角色/平台及 N3/100 seed/LONG/99% 仍待验；不关闭完整 CTRL/PROC/REAL 或共享阶段。

### timeout 终态作为自身完成证据（W103，基线 `1ae305be8`）

- S-D02-19 / CTRL-006/007、G0-023、PROC-048、REAL-023、A05/A08/A09/A11/A13/A17/A18/A19：
  审计 W96 Session `01a0eb8c-1f34-72f1-9591-bee302dac00e` 时发现，timeout criterion 和 recovery
  criterion 都引用了 recovery call ID；原独立断言遗漏逐 criterion 的 call/result 匹配。W96 的计数、
  timeout/reaped 和清理仍成立，但证据关联子断言追记失败，不以当时 completed 覆盖。
- 新回归首次 **0/1**：`state=timed_out`、`cleanup.reaped=true`、同一 launch identity 已确定，仍因没有
  exit code 被排除。现将已回收 timeout 纳入命令自身的不可变 terminal/output 证据；它只证明 timeout
  失败，不证明成功或当前 workspace 状态。精确回归 **20/20**，Agent Runtime **194/194**，workspace
  fmt、diff 与正式 Tauri 构建通过；仅有既有 warning。
- 二进制 `c11540bd33ae194588f8aa9ee9c625c6bf2e8e54b3cb13d81253f065a052034f` 使用全新
  data/work/profile 和 StepFun Coding Plan / `step-3.7-flash`。Session
  `01a0ebee-23ee-70c3-8cfd-9be3210867a0` 精确为一次 250 ms timeout→一次 `update_plan`→一次
  supported completion，共 3 个模型步骤、101 条事件、1 个 returned effect；criterion 只引用
  timeout call，tool/command 计数均 1，零恢复命令/历史检索/截断。Windows pipe 在 1,152 ms 内经
  interrupt→terminate 收敛，`reaped=true`、未 force kill。独立 **29 项**断言、工作区 tree hash
  `9d167b72…8e88`、应用/profile/Vite 与 ping/cmd 清零、正式备份均通过。
- cancelled/lost/force-kill、外部部分效果、其他 Provider/角色/平台及 N3/100 seed/LONG/99% 仍待验；
  不关闭完整 CTRL/PROC/REAL 或共享阶段。

### 托管进程 start/poll/cancel 的精确链证据（W104，基线 `034896f29`）

- S-D02-20 / CTRL-006/007、CMD-139、PROC-033、A02/A05/A09/A13/A17/A18/A19：正式 Tauri
  以 StepFun Coding Plan / `step-3.7-flash` 首次精确调用 `start_process`，启动
  `cmd.exe /d /c "echo W104-READY & ping -n 30 127.0.0.1 >nul"`；start 返回 running 和 exact
  process ID。下一步仅一次 `poll_process(wait_ms=2000)`，同一 ID 返回 running，输出去除行尾空白后
  精确为 `W104-READY`；再仅一次 `cancel_process`，返回 cancelled、`cleanup.reaped=true`。
- Windows CREATE_NO_WINDOW pipe 无可信 console interrupt，cancel 在 1,160 ms 内升级 terminate，未
  force kill。一次 `report_completion` 的唯一 supported criterion 按顺序且只引用 start、poll、cancel
  三个顶层 call ID，并在 rationale 中绑定同一 process ID、READY、cancelled 和 reap；零 ToolSearch/
  update_plan/exec、零截断和零工具错误。
- Session `01a0ebf7-12a4-7e62-abc8-456dc676bc14` 单回合 completed、4 个模型步骤、143 条
  canonical 事件；`workspace.process/start` 与 `/cancel` 两个 effect returned，poll 不制造 effect。
  二进制 `c11540bd33ae194588f8aa9ee9c625c6bf2e8e54b3cb13d81253f065a052034f`，独立
  **30 项**断言、工作区 tree hash `28cb3214…6c30`、应用/profile/Vite、leader/ping/cmd 清零和正式备份
  均通过。本批无源码修改。PTY/stdin/resize、失败 poll/cancel、其他 Provider/角色/平台及 N3/100 seed/
  LONG/99% 仍待验；不关闭完整 CMD/PROC/CTRL 或共享阶段。

### stdin 原始字节、显式游标与完整链证据（W105，基线 `36b0ab836`）

- S-D02-21 / CTRL-006/007、PROC-025/027、REAL-005、A02/A05/A09/A13/A17/A18/A19：首次正式
  Tauri Session `01a0ebfe-9b61-7c02-bc02-bab0cf3402d3` 精确执行 start→input→close→poll→completion，
  但 canonical poll Schema 与 App host 没有 `cursor`，close 消费回显后 terminal poll 只返回空文本；
  completion 又因 epoch 只允许 close/poll 两个顶层 ID，遗漏 start/input。首次夹具编译错误及修正后的
  产品断言 **0/1** 均保留，未覆盖首次失败。
- poll canonical Schema 现与模型工具一致地暴露非负 `cursor`、默认 0；App host 调用新增的
  `ManagedEngineProcessOwner::poll_from`，即使 terminal 已缓存也从请求游标重读。完成账本仅在同一
  process、连续 provenance、零省略交互且 cleanup 已证明时，将该精确 start/input/close/poll 链延续
  到终态 epoch；不恢复同 epoch 的无关旧观察。精确链回归 **20/20**。
- 两次修复中间 Session `01a0ec19-1637-7fe2-8e66-8f5769fad276`、
  `01a0ec21-7692-7373-b25a-72c1df0565ee` 已证明 cursor=0 与四 ID 引用，但模型仍裁掉夹具要求的尾随
  LF；强化原始字节 Session `01a0ec25-d1b3-7eb2-b138-533240a5552c` 明确回报 12 字节、无 `0A`，
  随后 3 次输出截断并以 `NOMIFUN_TASK_INCOMPLETE` 失败。三份失败均保留，不以等价 Trim 输出记通过。
- `write_process_stdin` 新增兼容的可选 `append_newline`，默认 false；true 时在未经 trim/normalize 的
  UTF-8 input 后追加且只追加一个 LF byte，预算在 dispatch 前包含该字节。最终二进制
  `af99bde37234cdeaa2c2083fef0555f626032cc32ee1c56f9ae49dcfb467effb`、全新 data/work/profile 和
  StepFun Coding Plan / `step-3.7-flash` 的 Session `01a0ec31-7baa-7842-a743-51323ae79e1f` 首次精确发送
  `input=W105-PAYLOAD, append_newline=true`；poll(cursor=0, wait_ms=5000) 返回完整
  `...-41-44-0A` 与 `W105-LEN:13`，exit 0、`cleanup.reaped=true`。唯一 supported criterion 按顺序引用
  四个顶层 call ID；5 步、177 条事件、零截断，3 个 managed effect returned。独立 **34 项**断言、
  Runtime **195/195**、Wave2 **22/22**、Engine **29 通过 / 1 ignored**、App host **3/3**、正式构建、
  工作区不变、匹配进程及应用/profile/Vite 清零、正式备份均通过。
- 未覆盖 PTY/resize、大块与 1 MiB 边界正式 UI、close/input/poll 并发、retained-base loss、其他
  Provider/角色/平台及 N3/100 seed/LONG/99%；不关闭完整 CTRL/PROC/REAL 或共享阶段。

### stdin 大小、owner 与终态拒绝边界（W106，基线 `a3e810c00`）

- S-D04-28 / PROC-029/030/031、A11/A13/A17/A19：新增 Engine 真实 pipe 反例直接跨过 owner。
  精确 1 MiB 单次 stdin 写入正常结算并由独立计数进程回报 `1048576`；1 MiB+1 返回稳定上限错误，
  随后同一进程只回报允许的 `safe` 4 字节，确认超限在 transport 前拒绝且没有部分交付。
- 已自然退出并 reaped 的 Session 拒绝迟到 stdin；拒绝后 `poll_from(cursor=0)` 仍重放相同 exited
  终态与原输出，没有复活或污染。App host 另将 `append_newline=true` 的一个 LF 纳入 dispatch 前预算：
  精确 1 MiB input 可写，精确上限再追加 LF 与 1 MiB+1 均拒绝。
- 三项新 Engine 反例重复 **60/60**；Engine 完整 **32 通过 / 1 ignored**、App process host
  **4/4**。既有 process Runtime 的未知 Session/错误双 owner 与终态 PTY resize 三项定向检查通过，
  均无 owner 写入或 Session 状态改变。首次即通过，无失败可保留；本批没有模型调用或正式 UI。
- 未覆盖多次分块累计流量策略、input/close 并发、终态拒绝的完整 UI/错误披露、其他平台及
  N3/100 seed/LONG/99%；不关闭完整 PROC 或共享阶段。

### PTY resize 的跨平台可执行上限（W107，基线 `b90ddb060`）

- S-D04-29 / PROC-032、A11/A13/A17/A19：标准工具和 canonical start/resize Schema 的
  `cols/rows` 上限为 65535，Engine request 也只拒绝 0；但 Windows ConPTY 使用 signed 16-bit
  `COORD`，32768 会通过 Schema 与 App journal 后才在平台 owner 中变成 I/O/unknown。领域 Schema、
  模型 Schema 和 Engine 校验三项首次反例均 **0/1**，日志独立保留。
- process Runtime 新增共享 `MAX_PTY_DIMENSION=32767`，resize 在调用平台 owner 前拒绝 0/32768；
  Engine start 校验、App journal/dispatch 前校验和两套模型 Schema 使用同一上限。合法 132×43
  ConPTY 仍完成 poll/write/resize/cancel，cancel 得到 cleanup/reaped；越界后 Session 保持运行且 owner
  resize 调用数为零，终态 resize 仍明确失败而不复活。
- 合法/越界 ConPTY 组合重复 **40/40**；process Runtime **125/125**、PTY contract **8/8**、
  Agent Runtime **195/195**、Wave2 **22/22**、Engine **33 通过 / 1 ignored**、App host **5/5**，
  fmt/diff 通过。本批没有模型调用或正式 UI。
- Windows ConPTY generic `close_stdin` 仍按既有合同返回“无法证明通用 EOF”，未把该错误伪装为
  成功；PROC-028、resize 后真实尺寸的应用级观测、其他平台及 N3/100 seed/LONG/99% 仍待验。

### 进程模型工具与 canonical Schema 同构守卫（W108，基线 `df8a580ec`）

- S-D01-07 / REG-008、G0-027、PROC-001～032 Schema 子断言：W105/W107 已分别证明缺失
  `poll.cursor` 和 PTY 上限漂移可以越过旧 `full_surface` 测试；旧测试只核对 Action ID、object 根与
  `additionalProperties=false`，没有比较真正的参数合同。
- 新回归从正式 Wave2 workspace registration 解析每个 canonical schema ref，逐一覆盖模型侧
  exec/start/poll/input/close_stdin/resize/cancel 七个工具。属性集合、required、type、oneOf、const/
  enum/default 必须一致；模型最小值不得更小、最大值不得更大，保留 `start_process.wait_ms=0` 这种
  明确减权。当前基线首次 **20/20**，Agent Runtime **196/196**，fmt/diff 通过。
- 本批只补必要回归，没有新产品失败、模型调用或正式 UI。文件/VCS/Artifact、其他 Wave/动态工具
  Schema 及单一生成源仍未覆盖，不关闭完整 REG/G0 或共享阶段。

### 全 workspace standard exposure 与 canonical Schema 同构（W109，基线 `8f4fa17b6`）

- S-D01-08 / REG-008、G0-027、FILE-017/024/032、ART-004、VCS-011：把 W108 守卫扩到
  Wave2 workspace 的文件、VCS、进程和 Artifact 全部 19 个工具。首次 **0/1** 精确落在
  Runtime 候选 `read_file.path` 缺少 canonical `\\S`；完整走查又发现其他文件/Artifact 路径、search query、
  Artifact page default 与 VCS push refspec/force 的同类漂移。首败日志保留。
- Runtime 候选 Schema 现对路径/查询应用 canonical 非空白约束；Artifact read 默认 limit 从 65536 对齐
  16384；push refspec 收窄到 1024 和 `HEAD|refs/heads/...:refs/heads/...` pattern，force 固定 false。
  patch 行改用与 owner 相同的 kind enum，仍只接受 context/add/remove 和无 CR/LF/NUL 的精确文本；
  status/diff 的空 required 显式化，不改变 admission。正式 App 在 Snapshot 编译时会以 canonical
  替换候选 Schema，本批未观察到正式 UI dispatch 失败；该修复收紧测试与未来直接集成的边界。
- 递归同构回归覆盖属性、required、type、union、pattern、const/default 及数值范围；候选可有安全
  收窄，不能比 canonical 更宽。修复后 **20/20**，Agent Runtime **197/197**，标准工具组 11/11、
  同步远端 `d4dcae8a3` 后复验及 fmt/diff 通过。本批没有模型调用或正式 UI。
- 未覆盖其他 Wave、动态/MCP/Plugin Schema 及单一生成源；
  不关闭完整 REG/G0 或共享阶段。

### local push 成功后的 settlement 丢失（W110，基线 `9d1de5fcb`）

- S-D05-08 / VCS-013、A05/A13/A17/A19：在隔离 worktree 与 bare local remote 中先完成真实
  push，确认 remote main 指向第一提交；随后故意丢弃尚未确认的 durable settlement，并在本地创建
  第二提交。首次结果即符合合同：owner 固定进入 `OutcomeUnknown`，第二次 push 在接触 remote 前
  拒绝，remote main 保持第一提交；`ensure_settled` 等待 worker 后仍不能自行清除未知 fence。
- 新故障窗口 **20/20**，push owner 全组 **10/10**。host 现有成功 receipt 同 key 重放与
  not-applied failure 重放 **2/2**，共同覆盖持久化前后两侧的当前进程行为。生产代码无需修改；
  临时仓库全在外部证据目录，无模型调用、正式 UI、网络 remote 或凭据。
- 未覆盖应用强退后 push pending effect 的专项恢复、主动 remote-ref 对账、远端 ref 被第三方并发
  删除/重写、其他平台及 N3/100 seed/LONG/99%；不关闭完整 VCS-013 或共享阶段。

### local push pending receipt 的跨 host fence（W111，基线 `924127be8`）

- S-D05-09 / VCS-013、LIFE-006/007、A05/A13/A17/A19：canonical Store 先 reserve exact
  external push effect，owner 再把第一提交真实推到隔离 bare remote；测试故意不写 terminal receipt，
  随后在本地创建第二提交并销毁首个 `Wave2ApplicationHost`。
- 使用同一 Store 重建 host 后，同 operation 首次即返回 durable pending，新的内存 push owner 未接触
  remote；remote main 保持第一提交，第二提交只留在本地，Effect 仍为 Pending。新回归 **20/20**，
  host push **3/3**，owner push **10/10**；生产代码无需修改，无模型/正式 UI/网络 remote 或凭据。
- 本批复用了同一内存 Store，尚未覆盖数据库连接关闭后重开、应用/进程强退、启动时 pending→unknown
  归约、主动 remote-ref 对账、第三方并发改写及 N3/100 seed/LONG/99%；不关闭完整 VCS/LIFE。

### local push pending receipt 的 SQLite 重开（W112，基线 `e06f84e06`）

- S-D05-10 / VCS-013、LIFE-006/007、A05/A13/A17/A19：把 W111 改为独立磁盘 SQLite。
  在 remote main 已更新、exact external Effect 仍为 Pending 时释放 reservation、host 和 Store，关闭
  所有数据库连接，再从同一路径重新打开数据库、Store 与 host；本地第二提交在关闭前已创建。
- 首次及 **20/20** 均由重开后的 durable pending 在新 owner 物理调用前拒绝；remote main 保持第一
  提交、第二提交只在本地，Effect ID 与 Pending 状态未改。相邻 host push **3/3**，fmt/diff 通过；
  生产代码无需修改，无模型/正式 UI/网络 remote 或凭据。
- 未覆盖 Git worker 或 SQLite terminal commit 中的真实进程强杀、完整 App startup pending→unknown
  归约、主动 remote-ref 对账、第三方并发改写、其他平台及 N3/100 seed/LONG/99%；不关闭完整 VCS/LIFE。

### 完整 App startup 的 Pending→Unknown 归约（W113，基线 `f73e6af30`）

- S-D05-11 / VCS-013、LIFE-006/007、OBS-005/016、A05/A13/A17/A19：在现有
  `native_execution_recovery` 磁盘 crash image 中，通过 canonical Store 给当前 running Turn 写入
  `workspace.vcs/push` 的 `ExternalUncertainEffect` Pending 事实；随后令旧 lease 过期并保留
  reconciliation head，从正式 `AppServices`/`create_router` 启动入口执行恢复。
- 启动恢复原子写入 `effect/uncertain`，Pending 变为 Unknown，同时只写一个
  `runtime/execution-recovery-blocked`，暂停原 Turn、保留 checkpoint，不启动模型；新 Turn 被明确拒绝。
  再次关闭数据库并完整启动后仍为一个 Unknown/一个 blocked 事件，没有重复隔离或重放。新增断言
  首次及连续 **20/20**；完整 App 恢复夹具 **6/6**，Store 原子隔离两项 **2/2**，相邻 host fence
  **1/1**，fmt/diff 通过。生产代码无需修改。
- W110～W112 已保留真实 bare remote push、settlement 丢失和 SQLite 重开现场；本批只补正式 App
  startup 归约，不重复物理 push。未覆盖 Git worker/SQLite terminal commit 中的真实进程强杀、主动
  remote-ref 对账、第三方并发改写、正式 UI/模型、其他平台及 N3/100 seed/LONG/99%；不关闭完整
  VCS/LIFE 或共享阶段。

### owner remote-ref 核对后恢复（W114，基线 `b96426178`）

- S-D05-12 / VCS-013、CTRL-009、LIFE-006/007/016/017、OBS-008/014、A05/A13/A17/A19：
  在 W113 的完整 App crash image 上增加真实 local bare remote。夹具先独立推送并读取精确
  `refs/heads/main`，再通过正式 `execution/effects` 读取唯一 Unknown push；返回不含自动重放许可。
- 错误 input digest 的核对请求被拒且 Effect 保持 Unknown。认证 owner 使用 exact digest、pause
  revision 和有界 remote-ref evidence 调用 `execution/reconcile` 后，只产生一个 attestation 与一个
  `effect/reconciled`；同 key 重送返回原回执，记录明确不是原始 push receipt。带 cleanup attestation
  恢复同一 Turn 后，checkpoint 正常完成、Pending/Unknown 清零，bare remote ref 仍为原提交。
- 新夹具首次因漏定义 execution API path 编译失败，日志保留；修正夹具后首次产品运行及连续
  **20/20** 通过，完整 App 恢复 **7/7**、Store pause/reconcile **11/11**，fmt/diff 通过。生产代码
  无需修改，无正式 UI/生产 remote。领域级自动对账按现有设计仍未实现；第三方 remote 改写、网络
  remote、UI 核对控制面、其他平台及 N3/100 seed/LONG/99% 仍待验，不关闭完整 VCS/LIFE 或共享阶段。

### 文件发布 receipt 丢失后的 SQLite 重开（W115，基线 `8d8b9bf92`）

- S-D03-51 / FILE-019/020/038、LIFE-006/007、A05/A13/A17/A19：把原有内存 Store/
  `std::fs::write` 模拟回归升级为独立磁盘 SQLite 和实际 `FileService` owner。Effect reserve 后 owner
  原子发布 `result.txt=published` 并返回 receipt；夹具故意不写 terminal receipt，释放 host/Store，
  关闭全部数据库连接，再由用户把目标改成 `user edit after lost receipt`。
- 从同一路径重开数据库、Store 与 host 后，原 key 由 durable Pending 直接拒绝；换 operation/key 的
  写入也由同一 workspace resource fence 在 owner 前拒绝。Effect 总数保持 1、状态保持 Pending，
  用户新字节未被覆盖。升级断言首次及连续 **20/20**；相邻 App 文件动作/fence **2/2**、文件 owner
  写入 **1/1**，fmt/diff 通过。生产代码无需修改。
- 首个基线命令因短测试名配 `--exact` 实际执行 0 项，日志保留；随后用完整模块名确认旧基线 1/1，
  没有把 0 项误记为通过。本批无模型/UI。未覆盖文件 worker/SQLite terminal commit 进程强杀、
  patch/delete 同类磁盘重开、文件结果人工核对、其他平台及 N3/100 seed/LONG/99%；不关闭完整
  FILE/LIFE 或共享阶段。

### 多文件 patch receipt 丢失后的 SQLite 重开（W116，基线 `d3a1ae36f`）

- S-D03-52 / FILE-027/031/038、LIFE-006/007、A05/A13/A17/A19：在独立磁盘 SQLite 中 reserve
  两文件 patch Effect，由实际 `FileService` owner 原子把 `alpha/beta` 发布为 `ALPHA/BETA`；owner
  返回两文件 receipt 后故意不写 canonical terminal，并关闭 host、Store 与全部数据库连接。
- 用户随后把两文件分别改为 `user first/user second`。重开数据库与 host 后，原 key 被 durable
  Pending 拒绝；新 operation/key 使用能合法匹配当前用户文本的 patch，仍在 owner 前被同一 workspace
  resource fence 拒绝。Effect 总数保持 1/Pending，两份用户字节均未被覆盖。
- 新增断言首次及连续 **20/20**；App patch 正常/并发/全有或全无/权限路径 **4/4**，W115 写入重开
  **1/1**，文件 owner patch **2/2**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖文件
  worker/SQLite terminal commit 进程强杀、delete 同类磁盘重开、patch 结果人工核对、其他平台及
  N3/100 seed/LONG/99%；不关闭完整 FILE/LIFE 或共享阶段。

### delete receipt 丢失后的同名重建保护（W117，基线 `51aeea7bf`）

- S-D03-53 / FILE-034/038、LIFE-006/007、A05/A13/A17/A19：在独立磁盘 SQLite 中 reserve
  文件 delete Effect，实际 `FileService` owner 删除旧 `victim.txt` 并返回路径 observation；夹具故意
  不写 canonical terminal，关闭 host、Store 与全部数据库连接，再由用户按同名重建不同字节。
- 从同一路径重开后，原 key 被 durable Pending 拒绝；新 operation/key 也在 delete owner 前被同一
  workspace resource fence 拒绝。Effect 总数保持 1/Pending，`user recreated` 字节完整保留。
- 新增断言首次及连续 **20/20**；App 普通/部分递归删除 **2/2**，FileService 删除、名称置换与
  Windows ACL **8/8**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖文件 worker/SQLite
  terminal commit 进程强杀、目录树成功删除后的同名重建磁盘重开、delete 结果人工核对、其他平台及
  N3/100 seed/LONG/99%；不关闭完整 FILE/LIFE 或共享阶段。

### 非空目录 delete receipt 丢失后的重建保护（W118，基线 `2cbe721a5`）

- S-D03-54 / FILE-034/037/038、LIFE-006/007、A05/A13/A17/A19：在独立磁盘 SQLite 中 reserve
  非空目录 delete Effect，实际 `FileService` owner 成功删除含 `nested/old.txt` 的旧树并返回路径
  observation；夹具故意不写 canonical terminal，关闭 host、Store 与全部数据库连接。
- 用户随后重建同名 `tree/recreated/user.txt`。从同一路径重开后，原 key 与新 operation/key 均在
  delete owner 前被 durable workspace fence 拒绝；Effect 总数保持 1/Pending，重建树字节完整保留。
- 新增断言首次及连续 **20/20**；W117 文件重建/部分递归删除 **2/2**，FileService 目录置换与
  Windows 空目录路径 **3/3**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖文件 worker/
  SQLite terminal commit 进程强杀、目录结果人工核对、其他平台及 N3/100 seed/LONG/99%；不关闭
  完整 FILE/LIFE 或共享阶段。

### Artifact publish receipt 丢失后的 SQLite 重开（W119，基线 `65fcef8e6`）

- S-D03-55 / ART-001/003/005、LIFE-006/007、A05/A13/A17/A19：在独立磁盘 SQLite 中 reserve
  publish Effect，实际 `WorkspaceArtifactStore` 把 `original artifact` 发布为 content-addressed blob；
  owner 返回 artifact ID 后故意不写 canonical terminal，并关闭 host、Store 与全部数据库连接。
- 源文件随后改为 `later source`。重开后原 key 与携带新 digest 的新 operation/key 均在 Artifact owner
  前被 workspace fence 拒绝；原 artifact 字节/ID 保持，目录只有一个 64 位内容对象，later digest
  对象不存在，Effect 总数保持 1/Pending。
- 首个新增断言因夹具 reserve 未采用生产序列化的 optional-null input，得到正确
  `IDEMPOTENCY_CONFLICT`；失败日志保留。修正夹具后首次及连续 **20/20**，App Artifact **2/2**、
  ArtifactStore 发布/读取/篡改/清理/并发 **15/15**，fmt/diff 通过。生产代码无需修改，无模型/UI。
  未覆盖 Artifact worker/SQLite terminal commit 进程强杀、owner 核对/恢复、Session 删除与 reader
  并发、其他平台及 N3/100 seed/LONG/99%；不关闭完整 ART/LIFE 或共享阶段。

### commit receipt 丢失后的 SQLite 重开（W120，基线 `b1335ee69`）

- S-D05-13 / VCS-006/009/014、LIFE-006/007、A05/A13/A17/A19：在独立磁盘 SQLite 中 reserve
  `workspace.vcs/commit` external Effect，经生产 `invoke_vcs_commit` 创建真实 commit；owner 返回
  commit ID 后故意不写 canonical terminal，并关闭 host、Store 与全部数据库连接。
- 用户随后修改并 stage 新字节。从同一路径重开后，原 key 与新 operation/key 均在 commit owner/hook
  前被 workspace fence 拒绝；HEAD 保持首个新 commit，历史仍只有 initial+该 commit，index blob 与
  worktree 都保留 `user staged after lost receipt`，Effect 总数保持 1/Pending。
- 新增断言首次及连续 **20/20**；相邻 commit replay/identity/scope **4/4**、stage/commit 共享门与
  并发 commit **2/2**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖 commit-intent 主动核对、
  libgit2 worker/SQLite terminal commit 进程强杀、post-commit 故障窗口的 Windows 运行、其他平台及
  N3/100 seed/LONG/99%；不关闭完整 VCS/LIFE 或共享阶段。

### stage receipt 丢失后的 index/worktree 保护（W121，基线 `09bbb6887`）

- S-D05-14 / VCS-004/014、LIFE-006、A05/A13/A17/A19：在独立磁盘 SQLite 中 reserve
  `workspace.vcs/stage` Effect，实际 `WorkspaceVcsStageOwner` 把新 worktree 字节写入 Git index；
  owner 返回 staged receipt 后故意不写 canonical terminal，并关闭 host、Store 与全部数据库连接。
- 用户随后把 index 恢复到 HEAD，同时保留新的 unstaged worktree 字节。重开后原 key 与新
  operation/key 均在 stage owner 前被 workspace fence 拒绝；HEAD/index 仍为 base，worktree 保留
  `user unstaged after lost receipt`，Effect 总数保持 1/Pending。
- 首次新增断言用早先打开的独立 `git2::Repository` 读到缓存 index，失败日志保留；改用重开仓库的
  磁盘 oracle 后首次及连续 **20/20**。相邻 App stage/commit **3/3**、VCS stage owner 锁/scope/删除/
  置换 **7/7**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖 index replace/SQLite terminal
  commit 进程强杀、stage 结果人工核对、linked worktree/submodule、其他平台及 N3/100 seed/LONG/99%；
  不关闭完整 VCS/LIFE 或共享阶段。

### 跨 Session workspace Pending fence（W122，基线 `fd0b71a98`）

- S-D09-06 / CONC-015、FILE-038、LIFE-006/007、AUTH-009、A05/A13/A17/A19：Session A 在独立
  磁盘 SQLite 中 reserve Effect 并由实际 `FileService` 发布 `first.txt`；夹具故意丢失 terminal receipt，
  关闭 host、Store 与全部数据库连接。
- 重开后创建同一认证 owner、同一物理 workspace 的独立 Session B。B 的不同 operation/key 在文件
  owner 前被全局 unsettled-resource 索引拒绝，未创建 B Effect 或 `second.txt`；A 仍只有一个 Pending，
  已发布字节不变。
- 新增断言首次及连续 **20/20**；既有 owner-scoped/跨模块 fence、W115 写入重开及 Store external
  uncertainty **3/3**，fmt/diff 通过。生产代码无需修改，无模型/UI。本批只证明同一 owner；正式 App
  多会话入口、真正同时竞争、跨 owner 错配同一路径绑定、owner 核对、其他平台及 N3/100 seed/
  LONG/99% 仍待验，不关闭完整 CONC/FILE/LIFE 或共享阶段。

### 跨连接并发 workspace Effect admission（W123，基线 `430622bb1`）

- S-D09-07 / CONC-015、FILE-038、LIFE-006、AUTH-009、A05/A13/A17/A19：同一磁盘数据库上建立
  两个独立 SQLite pool、两个 host 和两个同 owner/workspace AgentSession，同时提交不同写 Effect 的
  canonical reserve。
- 每轮恰好一个 Reserved、一个在唯一 unsettled-resource 索引处失败；只调用 winner 的实际
  `FileService` owner，因此只创建一个物理文件。关闭两套连接再重开后，loser 仍被 winner 的 Pending
  fence 拒绝，未创建 loser Effect 或文件。
- 新增竞态首次及连续 **20/20**；W122 顺序重开与既有 owner-scoped/跨模块 fence **2/2**，fmt/diff
  通过。生产代码无需修改，无模型/UI。未覆盖两个完整 host invocation 同时停在 owner 前、跨 action
  竞态、跨 owner 错配绑定、owner 核对、其他平台及 N3/100 seed/LONG/99%；不关闭完整
  CONC/FILE/LIFE 或共享阶段。

### 文件写与 VCS stage 跨 Action 并发 admission（W124，基线 `d72ce3704`）

- S-D09-08 / CONC-015、FILE-038、VCS-004/014、LIFE-006、AUTH-009、A05/A13/A17/A19：两个
  独立 SQLite pool/host/Session 同时 reserve 文件写与 Git stage。每轮唯一 workspace fence 恰好放行
  一个 Effect，随后只调用 winner 对应的实际 FileService 或 VCS stage owner。
- 关闭连接再重开后 loser action 仍被 Pending 拒绝。文件写获胜时，Git index 保持 base 且只出现
  新文件；stage 获胜时，index 为 candidate blob 且新文件不存在；HEAD 始终不变。磁盘状态只体现
  winner，没有 loser Effect。
- 新增竞态首次及连续 **20/20**，W123 同 action 竞态及两类 owner 正常路径 **3/3**，fmt/diff 通过。
  生产代码无需修改，无模型/UI。未覆盖两个完整 `invoke` 从入口并发、其他 Action 配对、跨 owner
  错配绑定、owner 核对、其他平台及 N3/100 seed/LONG/99%；不关闭完整 CONC/FILE/VCS/LIFE
  或共享阶段。

### 两个完整 patch invocation 的同源竞态（W125，基线 `f886756bb`）

- S-D09-09 / CONC-002/015、FILE-020/025、A05/A13/A17/A19：两个独立 SQLite pool/host/
  AgentSession 从正式 Wave2 `invoke` 入口并发 patch 同一文件；两边绑定相同 exact source digest，
  分别尝试写入 `LEFT` 与 `RIGHT`。
- 每轮恰好一个成功。loser 若撞到 winner Pending 则无 Effect；若在 winner 结算后获 admission，则因
  source digest 陈旧以唯一 Rejected 终结。最终文件只等于 winner 内容，无第二次覆盖；两 Session 均
  无 unsettled。关闭数据库再重开后，第三 Session 可正常写新文件，证明 fence 已释放。
- 新增竞态首次及连续 **20/20**；不同路径并发、W116 receipt-loss、FileService 外部变更回滚及 W124
  跨 Action **4/4**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖 write/delete/Artifact 等
  其他完整 invocation 配对、正式 App API 多会话、其他平台及 N3/100 seed/LONG/99%；不关闭完整
  CONC/FILE 或共享阶段。

### 两个完整 write invocation 的同路径竞态（W126，基线 `83031021b`）

- S-D09-10 / CONC-002/015、FILE-019/020/023、A05/A13/A17/A19：两个独立 SQLite pool/host/
  AgentSession 从正式 Wave2 `invoke` 入口并发向同一路径写入两份各 1 MiB 的不同字节。
- 第二请求撞到 Pending 时只有一个成功；若第一请求已完成 settlement，则两次按顺序成功。最终文件
  长度始终精确 1 MiB，全部为 `L` 或全部为 `R`，没有交错/截断；所有已创建 Effect 都是 Returned，
  两 Session 无 unsettled。关闭数据库再重开后，第三 Session 可正常覆盖，证明 fence 已释放。
- 新增竞态首次及连续 **20/20**；W125 patch 竞态、W115 receipt-loss、普通文件动作及 FileService
  原子写 **6/6**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖 8 MiB 边界并发、原生 replace
  中途强杀、write/delete 配对、其他平台及 N3/100 seed/LONG/99%；不关闭完整 CONC/FILE 或共享阶段。

### 完整 write/delete invocation 的同路径竞态（W127，基线 `02099af04`）

- S-D09-11 / CONC-002/015、FILE-019/020/034、A05/A13/A17/A19：两个独立 SQLite pool/host/
  AgentSession 从正式 Wave2 `invoke` 入口并发写入和删除同一路径；write 内容为 1 MiB。
- 允许一个请求撞到 Pending 被拒，也允许两次按某个串行顺序成功。最终磁盘只可能不存在，或存在
  精确 1 MiB 全 `W` 文件；不会保留旧 `base`、半写或混合字节。所有已创建 Effect 均 Returned，
  两 Session 无 unsettled；数据库重开后第三 Session 可正常写入。
- 新增竞态首次及连续 **20/20**；W126 write/write、W117 receipt-loss、FileService 原子写与删除置换
  **10/10**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖跨 Session 全局事件顺序投影、
  8 MiB 边界、目录 write/delete、原生调用中途强杀、其他平台及 N3/100 seed/LONG/99%；不关闭
  完整 CONC/FILE 或共享阶段。

### 两个完整 Artifact publish invocation 的同内容竞态（W128，基线 `8f30b536e`）

- S-D09-12 / CONC-015、ART-001/003/004、A05/A13/A17/A19：两个独立 SQLite pool/host/
  AgentSession 从正式 Wave2 `invoke` 入口并发 publish 同一已观察 source/digest。
- 允许一个请求撞到 Pending 被拒，也允许两次串行复用同一 content identity。所有成功回执的 artifact
  ID/sha256 均等于预期 digest，managed 目录始终只有一个 64 位内容对象；所有已创建 Effect 均
  Returned，两 Session 无 unsettled。数据库重开后第三 Session 分页读取仍为 complete/同 digest。
- 新增竞态首次及连续 **20/20**；W119 receipt-loss、App Artifact 正常路径与 ArtifactStore 并发
  publication gate **4/4**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖不同 source/digest
  并发、cleanup/Session 删除与 reader 竞态、大对象边界、其他平台及 N3/100 seed/LONG/99%；
  不关闭完整 CONC/ART 或共享阶段。

### 两个完整 Artifact publish invocation 的不同内容竞态（W129，基线 `979cb8298`）

- S-D09-13 / CONC-015、ART-001/003/004/005、A05/A13/A17/A19：两个独立 SQLite pool/host/
  AgentSession 从正式 Wave2 `invoke` 入口并发 publish 两个不同 source/digest。
- 允许一个撞 Pending 被拒，也允许两者串行成功；managed 目录中的 64 位对象集合与成功回执 digest
  集合精确相等，不存在无回执孤儿。每个对象字节与自身 digest 对应，所有已创建 Effect 均 Returned、
  无 unsettled；数据库重开后逐个成功对象仍能完整读取。
- 新增竞态首次及连续 **20/20**；W128 同内容、W119 receipt-loss、ArtifactStore 并发 gate 与 round
  trip **4/4**，fmt/diff 通过。生产代码无需修改，无模型/UI。未覆盖 cleanup/Session 删除与 reader
  竞态、不同源大对象、原生 link 中途强杀、其他平台及 N3/100 seed/LONG/99%；不关闭完整
  CONC/ART 或共享阶段。

### live Artifact reader 与 workspace cleanup 身份（W130，基线 `dec45aeda`）

- S-D03-56 / ART-005/007、LIFE-028、A05/A13/A17/A19：先由 `WorkspaceArtifactStore` 发布并缓存
  原 artifact handle，再尝试移走整个 workspace 并在同路径放入冒用旧 digest 名称的替代字节。回归
  同时定义两种安全平台结果：允许移走时旧 Store 因 root identity 改变而拒绝、新 Store 因 digest
  不符而拒绝；拒绝移走时旧 reader 只能继续读取原字节，替代 workspace 不出现。
- Windows 临时诊断明确走 `native-denied`：live pinned workspace/artifact handles 阻止目录移走；诊断
  输出随后移除。最终测试形状首次及连续 **20/20**，ArtifactStore 全组 **16/16**、managed workspace
  cleanup 与 W129 **2/2**，fmt/diff 通过。生产代码无需修改，无模型/UI。
- 未覆盖正式 AgentSession delete API 与 in-flight read 真并发、允许 rename 平台的原生分支、cleanup
  task 强杀、其他平台及 N3/100 seed/LONG/99%；不关闭完整 ART/LIFE 或共享阶段。

### 正式 Session delete 与 live Artifact owner（W131，基线 `fd7626a52`）

- S-D03-57 / ART-007、LIFE-028、OBS-004/006、A05/A13/A17/A19：既有正式 App managed-workspace
  删除测试先发布并缓存 Artifact reader，再以固定 idempotency key 删除 Session。Windows 首次返回
  **500 / INTERNAL_ERROR**（`os error 32`）；canonical Session 已正确保留 deleting fence，但错误分类
  不准确，首败日志已保留。
- 正式 HTTP 删除流程现把两条 workspace cleanup 分支统一映射为 **409 /
  AGENT_SESSION_WORKSPACE_CLEANUP_FAILED**，保留底层详情与 deleting 状态。释放 Artifact owner 后，
  同一 key 重试完成原 tombstone 和 managed workspace 清理；sibling managed workspace 和 user-selected
  workspace 均保持。
- 修复后首次及连续 **20/20**；删除顺序、managed workspace 边界与 W130 reader **3/3**，fmt/diff
  通过。首个相邻静态命令因错误 test target 执行 0 项，已用 `--lib` 完整名纠正，不计通过。无模型/
  UI。未覆盖正式 Artifact read 请求与 DELETE 真并发、启动恢复 deleting Session 的 live-handle 重试、
  其他平台及 N3/100 seed/LONG/99%；不关闭完整 ART/LIFE/OBS 或共享阶段。

### deleting Session 的正式启动恢复（W132，基线 `f9e8ad16c`）

- S-D03-58 / ART-007、LIFE-020/028、OBS-004/006、A05/A13/A17/A19：使用磁盘 App、正式
  `coding.codex` 资源集合和 managed workspace。live Artifact owner 使首进程 DELETE 精确返回 W131
  的 409 并保留 `deleting`；随后释放 handle，关闭 Router/Services/DB，不再发送 DELETE。
- 第二进程从同一 data/work 配置启动，`create_router` 在路由发布前自动恢复 deleting Session、删除
  managed workspace 并写入 `deleted`；启动后同 key DELETE 只重放原 tombstone。
- 新增场景修正资源夹具后首次及连续 **20/20**；W131 409/retry、删除顺序与 Store delete fence
  **4/4**，fmt/diff 通过。首个夹具用不消费 workspace 的 `chat.minimal` 被
  `RESOURCE_SELECTION_UNUSED` 正确拒绝，失败日志保留。生产代码无需修改，无模型/UI。未覆盖
  cleanup/recovery 进程中途再次强杀、正式 UI 删除、非 Windows live-handle 路径及 N3/100 seed/
  LONG/99%；不关闭完整 ART/LIFE/OBS 或共享阶段。

### SQLite busy effect admission（W133，基线 `d03df57ed`）

- S-D09-14 / LIFE-023、CONC-014、FILE-038、A05/A13/A17/A19：独立连接持有 SQLite writer lock
  时，从生产 Wave2 Host 发起文件写。200 ms 时任务仍等待，文件与 Effect 均不存在；约 5 秒 canonical
  busy timeout 后返回 `CAPABILITY_UNAVAILABLE`，仍为零物理副作用/零 Effect。释放锁后同
  operation/key 重试只执行一次并得到唯一 Returned。
- 首版测试误用会重复写 tool fact 的辅助 `invoke()`，在 writer lock 下由夹具 `.expect` panic；失败日志
  保留。改为直接调用生产 `Wave2HostPort::invoke` 后首次及连续 **20/20**；每轮实际等待 busy timeout，
  相邻 Store writer 等待与 W115 写入重开 **2/2**，fmt/diff 通过。生产代码无需修改，无模型/UI。
- 未覆盖 owner 已执行后 terminal settlement 遇锁、锁期间取消、进程强杀、其他 Action/平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/FILE 或共享阶段。

### 文件发布后的 SQLite busy 终态落库（W134，基线 `880425da3`）

- S-D09-15 / LIFE-006/011/023、FILE-038、CONC-014、A04/A05/A07/A17/A19：canonical Effect
  reserve 后由实际 `FileService` 发布文件并取得 receipt，再由独立连接持有 SQLite writer lock。
  `finish_wave2_effect` 等待约 5 秒后精确返回 `CAPABILITY_UNAVAILABLE`；数据库仍为 Pending，文件
  保持已发布，未回滚或假报可安全重放。
- 释放锁后用户修改文件，关闭并从同一路径重开数据库。原 key 由 durable Pending 拒绝，新 key 由
  workspace resource fence 拒绝；用户修改保持，数据库只有一条 Effect。首次产品运行及连续
  **20/20**，W133 admission 与 W115 receipt-loss 相邻回归 **2/2**，fmt/diff 通过。首个证据目录
  命令因 PowerShell 参数错误未启动 cargo，已在 metadata 留痕，不计产品样本。生产代码无需修改，
  无模型/UI。
- 未覆盖终态等待期间取消、精确边界强杀、DB 磁盘满/IO fault、其他 Action/平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/FILE 或共享阶段。

### SQLite busy admission 等待期间取消（W135，基线 `f462b1ea4`）

- S-D09-16 / LIFE-023、CONC-014、FILE-038、A04/A10/A11/A19：独立连接持有 SQLite writer lock
  时，从生产 Wave2 Host 发起文件写；200 ms 时仍为零文件、零 Effect，随后取消并等待 Host task
  得到 cancelled。释放锁并留出迟到完成窗口后仍为零副作用、零 Effect；同 operation/key 的显式
  重试成功一次，最终只有一条 Returned Effect。
- 首次产品运行及连续 **20/20**，W133 busy timeout 与 W134 terminal busy 相邻回归 **2/2**，
  fmt/diff 通过。生产代码无需修改，无模型/UI。
- 未覆盖正式 Runtime/UI cancel 传播、终态落库等待期间取消、精确边界强杀、其他 Action/平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/FILE 或共享阶段。

### SQLite busy 终态落库等待期间取消（W136，基线 `120341f63`）

- S-D09-17 / LIFE-006/011/019/023、FILE-038、CONC-014、A04/A07/A10/A11/A17/A19：Effect
  reserve 且实际 `FileService` 已发布文件后，以独立 SQLite writer lock 阻塞 terminal settlement；
  确认仍为 Pending 后取消并等待 settlement task 得到 cancelled。
- 用户修改文件后释放数据库锁并留出迟到完成窗口，Effect 仍为 Pending、用户内容保持；关闭并从同一
  路径重开数据库后，原 key 与新 key 均被 durable/resource fence 拒绝，只有一条 Effect。首次产品
  运行及连续 **20/20**，W134 terminal timeout 与 W135 admission cancel 相邻回归 **2/2**，
  fmt/diff 通过。生产代码无需修改，无模型/UI。
- 未覆盖正式 Runtime/UI cancel 与 Turn 终态的事务顺序、精确边界强杀、DB 磁盘满/IO fault、其他
  Action/平台及 N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/FILE 或共享阶段。

### 文件发布后的 terminal Store 不可用（W137，基线 `40fc56863`）

- S-D09-18 / LIFE-006/011/024、FILE-038、A04/A07/A17/A19：canonical Effect reserve 且实际
  `FileService` 已发布文件并取得 receipt 后关闭数据库连接池。`finish_wave2_effect` 精确返回
  `CAPABILITY_UNAVAILABLE`，已发布文件保持。
- 用户修改文件后重新打开同一路径数据库，原 Effect 仍为 Pending；原 key 由 durable Pending 拒绝，
  新 key 由 workspace resource fence 拒绝，用户内容保持且只有一条 Effect。首次产品运行及连续
  **20/20**，W115 receipt-loss 与 W134 terminal busy 相邻回归 **2/2**，fmt/diff 通过。生产代码
  无需修改，无模型/UI。
- 未覆盖真实磁盘满/写入 IO fault、WAL/fsync 故障、正式应用 shutdown 竞态、其他 Action/平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/FILE 或共享阶段。

### admission 前 Store 不可用（W138，基线 `55ff16b25`）

- S-D09-19 / LIFE-003/024、A04/A05/A17/A19：创建 Session/Turn/tool causation fact 后关闭 canonical
  Store，再从生产 Wave2 Host 发起文件写。Host 在 Effect ledger read/admission 边界精确返回
  `CAPABILITY_UNAVAILABLE`，磁盘零文件、数据库零 Effect。
- 从同一路径重开 Store 与 Host 后，同 operation/key 才首次执行并得到唯一 Returned Effect。首次产品
  运行及连续 **20/20**，W137 terminal Store close 与 W133 busy admission 相邻回归 **2/2**，
  fmt/diff 通过。生产代码无需修改，无模型/UI。
- 未覆盖真实磁盘满/IO fault、连接池关闭与在途 admission 竞态、正式应用 shutdown、其他 Action/平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE 或共享阶段。

### Store close 与 busy admission 竞态（W139，基线 `c6dda2293`）

- S-D09-20 / LIFE-003/023/024、CONC-014、A04/A11/A17/A19：独立连接持有 SQLite writer lock 后，
  从生产 Wave2 Host 发起文件写并确认请求卡在 admission，再启动 canonical Store close。请求按约
  5 秒 busy timeout 返回 `CAPABILITY_UNAVAILABLE`，close 随在途连接归约完成，磁盘零文件。
- 释放锁并从同一路径重开后，数据库仍为零 Effect；同 operation/key 才首次执行并得到唯一 Returned。
  首次产品运行及连续 **20/20**，W138 pre-closed Store 与 W133 busy timeout 相邻回归 **2/2**，
  fmt/diff 通过。生产代码无需修改，无模型/UI。
- 未覆盖 terminal settlement 与 Store close 竞态、正式应用 shutdown 顺序、真实磁盘/IO fault、其他
  Action/平台及 N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC 或共享阶段。

### Store close 与 busy terminal 竞态（W140，基线 `cd7f56002`）

- S-D09-21 / LIFE-006/011/023/024、FILE-038、CONC-014、A04/A07/A11/A17/A19：Effect reserve 与
  实际 `FileService` 发布完成后，由独立连接持有 SQLite writer lock 阻塞 terminal settlement，再启动
  canonical Store close。settlement 按约 5 秒 busy timeout 返回 `CAPABILITY_UNAVAILABLE`，close
  随在途连接归约完成；文件保持已发布。
- 释放锁、用户修改文件并从同一路径重开后，Effect 仍为 Pending，原 key 与新 key 均被 durable/
  resource fence 拒绝，用户内容保持且只有一条 Effect。首次产品运行及连续 **20/20**，W139
  close-admission 与 W134 busy-terminal 相邻回归 **2/2**，fmt/diff 通过。生产代码无需修改，
  无模型/UI。
- 未覆盖正式应用 shutdown 的 Runtime/Store/owner 顺序、close 与 cancel 组合、真实磁盘/IO fault、
  其他 Action/平台及 N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/FILE 或共享阶段。

### 已知 owner 失败的 terminal settlement（W141，基线 `44633a0cf`）

- S-D09-22 / LIFE-011/024、FILE-038、OBS-006/015、A04/A07/A08/A17/A19：从正式
  `workspace.files/write` handler 把“目标为目录”的确定 owner 失败暂停在 terminal settlement 前，再
  以独立 SQLite writer lock 令落库超时。旧代码丢弃 settlement 错误并只返回普通 owner 错误，数据库
  却保持 Pending；首次 FAIL 日志保留。
- 新增统一 `finish_wave2_failed_effect`，覆盖通用 managed effect 及 write/delete/artifact/stage/commit/
  push 的已知失败分支。落库成功仍返回并重放原 owner 错误；落库失败则有界返回
  `CAPABILITY_UNAVAILABLE`，保留原错误 code/message，并明确 terminal observation 未提交、durable
  effect 未决且禁止自动重试。
- 修复后首次及连续 **20/20**；健康 write Rejected/replay、patch、delete、commit、push 与成功后
  terminal-busy 相邻回归 **6/6**，fmt/diff 通过。首个 macOS 专属 commit 过滤命令执行 0 项，已用
  Windows 可执行反例纠正，不计通过。无模型/UI。
- 未覆盖其他 Action 的逐分支 fault injection、正式 API/UI 投影、真实磁盘/IO fault、其他平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/FILE/OBS 或共享阶段。

### managed-effect owner 失败的 terminal settlement（W142，基线 `1f24c27f0`）

- S-D09-23 / LIFE-011/024、PROC-014/039、OBS-004/006/015、A04/A08/A11/A17/A19：从生产
  `invoke_managed_effect` 入口 reserve `workspace.process/start` Effect，夹具 owner 返回确定
  `PROCESS_EXIT_NON_ZERO`，同时由独立 SQLite writer lock 阻塞 terminal settlement。
- W141 的统一 helper 有界返回 `CAPABILITY_UNAVAILABLE`，完整保留原 error code 与 exit 原因并明确
  terminal observation 未提交；Effect 保持 Pending。关闭并重开数据库后，同 key 与新 key 均在 owner
  closure 前被 durable/resource fence 拒绝，两个独立调用计数保持 0。
- 首次产品运行及连续 **20/20**，W141 write 未结算与健康 Rejected/replay 相邻回归 **2/2**，
  fmt/diff 通过。生产代码无需修改，无真实进程/模型/UI。
- 未覆盖正式 Runtime process owner、cancel/kill 与 terminal 的事务顺序、API/UI 投影、其他平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/PROC/OBS 或共享阶段。

### 跨 owner 失败结算的有界脱敏投影（W143，基线 `ff12a1887`）

- S-D09-24 / LIFE-011/024、BROW-011/012、COMP-010、SSH-003/008、OBS-009/015/016、A08/A15/
  A16/A17/A19：以 secret 开头、随后 2,000 个 emoji 的 owner 错误触发 terminal Store failure。旧聚合
  消息达到 **4,291 bytes**，超过 Kernel 2 KiB 投影上限且未在聚合边界再次脱敏；首次 FAIL 保留。
- action/code/owner/settlement 现分别先脱敏、过滤控制字符并按 UTF-8 字节预算截断；最终消息
  ≤2,048 bytes，仍保留原 code、脱敏标记和禁止自动重试结论。共享 helper 覆盖 Browser、Computer
  Role 与 SSH 的确定失败；Role 新增 `EffectSettlementFailure/CAPABILITY_UNAVAILABLE`，Store/
  admission/terminal 错误不再误标 provider failure。
- 修复后首次及连续 **20/20**；settlement **8/8**、Browser feature **1/1**、Computer feature **1/1**、
  W142 与 push 相邻 **2/2**，fmt/diff 通过。Browser 首个未启 feature 的过滤命令执行 0 项，纠正后
  不计通过。无真实 Browser/Computer/SSH、模型/UI。
- 未覆盖三类 owner 的逐入口 fault injection、uncertain settlement、外部依赖与其他平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/BROW/COMP/SSH/OBS 或共享阶段。

### uncertain terminal 未提交时保留 owner 原因（W144，基线 `fd7657d5c`）

- S-D09-25 / LIFE-007/011/024、VCS-013、BROW-012/013、COMP-010、SSH-005/008、OBS-006/015/016、
  A07/A08/A15/A16/A17/A19：reserve external Effect 后模拟 owner 已判定 `EFFECT_OUTCOME_UNKNOWN`，
  再关闭 terminal Store。旧路径只返回 closed-pool 错误，丢失 remote 已收字节后断连的未知结果原因；
  数据库实际仍为 Pending，首次 FAIL 保留。
- 新增有界脱敏的 `finish_wave2_uncertain_effect`；uncertain terminal 无法提交时同时保留 action、原
  error code/message 和落库失败原因，并明确 durable Effect 仍 Pending、禁止自动重试、恢复前必须
  核对 external owner。helper 覆盖 VCS commit/push、Browser、Computer Role 与 SSH uncertain 分支。
- 修复后首次及连续 **20/20**；Pending/Unknown 与 push **2/2**、Browser feature **1/1**、Computer
  feature **1/1**，fmt/diff 通过。无真实 remote/Browser/Computer/SSH、模型/UI。
- 未覆盖各 owner 的逐入口 terminal fault injection、正式 reconcile/UI 投影、外部依赖与其他平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/VCS/BROW/COMP/SSH/OBS 或共享阶段。

### success terminal 未提交时明确效果已发生（W145，基线 `0aeea5cac`）

- S-D09-26 / LIFE-006/011/024、FILE-038、VCS-013、BROW-012、COMP-010、SSH-005、OBS-006/015/020、
  A04/A05/A07/A17/A19：从正式 `workspace.files/write` handler 发布完整文件后暂停在 terminal
  settlement，并以 SQLite writer lock 令落库超时。旧路径只返回 `database is locked`，调用方无法
  判断物理效果已发生，Effect 实际为 Pending；首次 FAIL 保留。
- 新增 `finish_wave2_succeeded_effect`：只记录 canonical result digest，不复制潜在敏感结果正文；落库
  失败时明确 owner 已报告成功、terminal observation 未提交、durable Effect 仍 Pending、禁止自动
  重试并要求重读 owner 状态。helper 覆盖 managed effect、write/delete/artifact/stage/commit/push、
  Browser、Computer Role 与 SSH 成功分支；patch 保留更强的逐文件重读诊断。
- 修复后首次及连续 **20/20**；managed success/replay、workspace replay、bare push replay 与 busy
  fence **4/4**，Browser feature **1/1**、Computer feature **1/1**，fmt/diff 通过。无正式 UI/真实
  外部 owner。
- 未覆盖各 Action 的逐入口 terminal fault、API/UI 投影、其他平台及 N3/100 seed/LONG/99%；不关闭
  完整 LIFE/FILE/VCS/BROW/COMP/SSH/OBS 或共享阶段。

### Kernel 区分三类 settlement loss（W146，基线 `ead23eb2d`）

- S-D09-27 / LIFE-006/007/011/024、FILE-038、OBS-006/009/015/016/020、A05/A07/A08/A15/A16/A17/A19：
  将 success/failed/unknown 三类内部 settlement 错误送入正式 `kernel_error_for_action` 文件写投影。
  旧路径全部压成普通 “Workspace file operation failed”；success 丢失“效果已发生/不可重试”，known
  failure 还可能因 `changed` 被误判为 source precondition，首次 FAIL 保留。
- Kernel 现先识别三组稳定内部 marker，再返回固定、≤2 KiB 且不含 host 路径/secret 的恢复指引：
  success 要求不得声称未变并重读；failed 明确 failure receipt 未落库且仍 Pending；unknown 要求核对
  external owner。
- 修复后首次及连续 **20/20**；既有普通 file、结构化 patch、process 指引 **3/3**，fmt/diff 通过。
  无完整 Runtime invocation/API/UI。
- 未覆盖非文件 Action 的模型安全投影、正式 tool row/UI、其他平台及 N3/100 seed/LONG/99%；不关闭
  完整 LIFE/FILE/OBS 或共享阶段。

### 非文件 Action 保留三类 settlement loss（W147，基线 `0815291d4`）

- S-D09-28 / LIFE-006/007/011/024、PROC-039、BROW-012、SSH-008、OBS-006/009/015/016/020、A07/A08/
  A15/A16/A17/A19：将 success settlement loss 送入 process 投影，failed/unknown 分别送入普通
  Browser/SSH 类投影。旧 process 文案把已成功效果改写为“owner 未完成、可能不确定”，普通 capability
  则只剩 `handler failed with CAPABILITY_UNAVAILABLE`；首次 FAIL 保留。
- 三类稳定 marker 识别提升到 Action 特判之前，统一返回固定、无 host 路径/secret 的 success/failed/
  unknown 指引；普通 process spawn/cwd/control 指引和其他 typed error 保持原顺序。
- 修复后首次及连续 **20/20**；W146 文件三态、process launch 与 active-execution 安全投影 **3/3**，
  fmt/diff 通过。无完整 Runtime invocation/API/UI。
- 未覆盖正式 tool result/event/UI、Browser/SSH/Computer 真实入口、其他平台及 N3/100 seed/LONG/99%；
  不关闭完整 LIFE/PROC/BROW/SSH/OBS 或共享阶段。

### Runtime Kernel 完整投影三类 settlement loss（W148，基线 `ea34983cd`）

- S-D09-29 / OBS-001/006/009/015/016/020、REG-004/007、A01/A03/A08/A15/A16/A17/A19：使用三套
  真实编译 Snapshot/ActiveSet/ToolPlan 和正式 `KernelAgentToolInvoker`，分别注入 success/failed/
  unknown settlement loss。
- 三次均只 dispatch 一次；Runtime typed error 保持 `CAPABILITY_UNAVAILABLE`，W147 固定恢复语义、
  ≤2 KiB 上限和 host 路径/secret 隔离完整穿过 Kernel 与 Agent Runtime 适配。
- 新增场景首次及连续 **20/20**；正常 Kernel invocation、未选择 capability 的 plan 拒绝及 W147
  Engine 投影 **3/3**，fmt/diff 通过。生产代码无需修改，无事件 Store/API/UI。
- 未覆盖正式 tool result/event 持久化、tool row/UI、不同 Action/角色及其他平台、N3/100 seed/
  LONG/99%；不关闭完整 REG/OBS 或共享阶段。

### Runtime ToolResult 与 ToolCompleted 投影（W149，基线 `4f9eaac58`）

- S-D09-30 / OBS-001/006/009/014/015/020、A03/A08/A09/A15/A16/A17/A19：将 success/failed/unknown
  三类 `CapabilityKernel` typed error 送入正式 `record_tool_result`。
- 每类均生成同 call ID、`is_error=true`、≤2 KiB 的模型 observation，保留
  `CAPABILITY_UNAVAILABLE` 与 W147 固定恢复语义；EventSink 各收到唯一、递增 step 的
  `ToolCompleted`，没有把错误变成 completed success 或丢失 call 关联。
- 新增场景首次及连续 **20/20**；W148 Runtime Kernel 与正常 effectful ToolResult/Event 相邻回归
  **2/2**，fmt/diff 通过。生产代码无需修改，无持久 AgentSession Store/API/UI。
- 未覆盖正式事件落库/重连、tool row、模型后续行为、其他平台及 N3/100 seed/LONG/99%；不关闭
  完整 OBS 或共享阶段。

### canonical Store 的 ToolResult 冷读投影（W150，基线 `dd319c5b6`）

- S-D09-31 / OBS-001/006/009/014/015/018/020、A03/A08/A09/A15/A16/A17/A19：让正式
  `EngineToolHost` 的 inner Kernel 返回 success settlement loss，经 `host_tool_dispatch/settled` 与生产
  `EngineTurnJournal` 写入 canonical AgentSession Store。
- 数据库中恰好一条 `tool/call-started` 和一条 `tool/result-recorded`，共享 correlation、result 因果
  指向 call；output 为 null、error 保留 `CAPABILITY_UNAVAILABLE` 安全指引。两个独立 Store 冷读均
  只重建一条 `state=recorded` tool projection，无重复 row。
- 新增场景首次及连续 **20/20**；取消后 owner settlement、history 排序、AgentSession projector
  **3/3**，fmt/diff 通过。生产代码无需修改，使用内存 SQLite，无进程重启/API/UI。
- 未覆盖磁盘重开、cursor 分页/重连、正式 tool row、其他平台及 N3/100 seed/LONG/99%；不关闭完整
  OBS 或共享阶段。

### 磁盘重开后的跨 Store cursor 分页（W151，基线 `083de4d32`）

- S-D09-32 / LIFE-011、OBS-014/018/019/020、A03/A08/A09/A15/A17/A19：把 W150 的正式
  EngineToolHost/Journal 场景改为磁盘 SQLite；写入 settlement error projection 后释放 owner、Journal
  和全部旧连接，再从同一路径初始化数据库。
- `message_history_before(limit=1)` 每页使用新的 Store，并以上一页最老 `first_seq` 为 cursor；完整遍历
  后恰好一条 `recorded` error tool projection、两个 canonical tool event，total 恒定且 projection ID
  无重复。
- 新增场景首次及连续 **20/20**；W150 memory projection、AgentSession cursor rebuild 与 cold history
  **3/3**，fmt/diff 通过。生产代码无需修改，仅抽取共用 test fixture，无正式 HTTP/Realtime/UI。
- 未覆盖分页期间并发新事件、cursor 重连传输、tool row、其他平台及 N3/100 seed/LONG/99%；不关闭
  完整 LIFE/OBS 或共享阶段。

### HTTP message-history cursor 传输（W152，基线 `68bfe029d`）

- S-D09-33 / OBS-014/018/020、PORT-012、A03/A08/A09/A15/A17/A19：通过正式 Axum Router、本地信任
  认证和产品 API 创建 Provider/Preset/AgentSession，再由 canonical Store 写入已验证的 settlement
  error tool call/result。
- `GET message-history?page_size=1` 逐页返回稳定 total/has_more；按正式
  `<created_at>:<message_id>` 生成 cursor 后完整收敛，所有 message ID 唯一，恰好一条
  `type=tool_call`，顶层与 content 均为 error，output 保留 “Do not retry” 指引。
- 新增场景首次及连续 **20/20**；W151 磁盘/cursor 下层相邻回归 **1/1**，fmt/diff 通过。生产代码
  无需修改，无 Realtime/UI。
- 未覆盖分页期间并发新事件、Realtime 重连、正式 renderer tool row、其他平台及 N3/100 seed/
  LONG/99%；不关闭完整 PORT/OBS 或共享阶段。

### 正式 events cursor 断连重放与游标隔离（W153，基线 `40e0e286e`）

- S-D09-34 / OBS-014/018/020、PORT-012、A03/A06/A08/A09/A17/A19：复用 W152 正式 Axum
  Router/本地信任 API 与 W151 磁盘 SQLite 模式；canonical Store 依次写入 turn/started、
  tool/call-started、含 CAPABILITY_UNAVAILABLE 的 tool/result-recorded 与 turn/failed。
- 消费方只读完第一页 `GET events?limit=1` 后，Router/Services/数据库连接全部关闭并按同一
  磁盘路径重建，模拟断连后独立重连。按 `after_seq` 续读到空页：seq 严格连续递增、event_id
  全部唯一，tool call/result 与 turn/failed 各恰好一条，result payload 保留
  CAPABILITY_UNAVAILABLE 与 “Do not retry” 指引；相同 append 的写侧重放只回 duplicate，
  事件行数不变。
- 事件 `after_seq` 与 `<created_at>:<message_id>` history cursor 双向混用均被 400 拒绝，
  超前 cursor 同样 400 fail-closed；重连后 `message-history?page_size=1` 收敛仍恰好一条
  error tool row。
- 新增场景首次及连续 **20/20**；W152 history cursor 相邻回归 **1/1**，fmt/diff 通过。生产
  代码无需修改，无正式 Realtime 传输/renderer UI。
- 未覆盖分页期间并发追加新事件、WS 推送消费方、正式 Tauri renderer tool row、PORT-012
  watcher 丢批/乱序对账、其他平台及 N3/100 seed/LONG/99%；不关闭完整 PORT/OBS 或共享阶段。

### 分页期间并发追加的 cursor 稳定性与正向投影漏读修复（W154，基线 `3bc79657f`）

- S-D09-35 / OBS-014/018/020、PORT-012、A03/A06/A08/A09/A17/A19：复用 W152 正式
  Axum Router/本地信任 API；同一 Turn 内写入两个 tool call 并乱序结算，再分别以
  `GET /messages?limit=1` 正向分页与 `GET events?limit=2`、`message-history?page_size=2`
  走查，翻页间追加新 Turn。
- **首败已保留**（`phase-2-3\2026-09-27\windows\W154\run-1-first-failure.log`）：`/messages`
  正向 cursor 按 `last_seq` 过滤却按 `first_seq` 排序，`take(limit)` 截断后以页内
  `max(last_seq)` 推进游标，使第二个 tool 投影（先结算、`last_seq` 落后）被永久跳过，
  committed 5 行仅交付 4 行。
- 修复：`messages_after_tx` 改按 `last_seq ASC` 排序（`last_seq` 每投影唯一且与游标同键），
  所有调用方均为 find/max_by_key/自行重排，无序敏感。
- 修复后正向分页完整交付全部投影且中途追加的 Turn 恰好一次；事件 feed 并发追加时 seq 严格
  连续；history 锚定窗口排除 walk 开始后提交的行，新 Turn 的 turn_summary 与源消息只在下一次
  fresh 读出现。两场景首次及连续 **20/20**，W153 相邻 **1/1**，`nomifun-agent-session` 库
  **76/76**，fmt/diff 通过。
- 未覆盖 WS 推送消费方、正式 Tauri renderer tool row、PORT-012 watcher 丢批/乱序对账、
  history cursor 与 turn_summary 边界的更大并发矩阵、其他平台及 N3/100 seed/LONG/99%；
  不关闭完整 PORT/OBS 或共享阶段。

下一步优先共享：完成证据及其他恢复/范围变更矩阵、FILE 发布/回滚的剩余竞态、watcher rescan/dropped 的完整 UI 对账与丢批/乱序，以及 S-D01～11 剩余合同、恢复、资源和产品
入口；相关 Windows 行为一起验证。共享阶段验收后再继续 Windows 专属余项。完整 N3/LONG/99%
门槛保留，不重建 2,374 行日志/状态文件到 Git。
