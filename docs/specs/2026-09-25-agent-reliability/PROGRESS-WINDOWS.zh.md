# Windows Case 处理进度

更新：2026-09-28。当前宿主 Windows。执行顺序见 [实施计划](IMPLEMENTATION-PLAN.zh.md)。
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

下一步优先共享：FILE 既有文件发布、扁平遍历/侧栏及 watcher 接线，以及 S-D01～11 剩余合同、恢复、资源和产品
入口；相关 Windows 行为一起验证。共享阶段验收后再继续 Windows 专属余项。完整 N3/LONG/99%
门槛保留，不重建 2,374 行日志/状态文件到 Git。
