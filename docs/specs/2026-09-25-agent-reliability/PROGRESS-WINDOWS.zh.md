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
| D05 | 69 | W03 | 本地隔离 Git remote、SSH 夹具；取消/未知副作用 | 条件资源待准备 |
| D06 | 247 | W04/W05 | WebView2 profile、Computer A11y、MCP/Plugin/Skill | Skill 首发已回归；其余待走查 |
| D07 | 124 | W02/W04 | 精确产品目标、Knowledge/Companion/Canvas/Customer owner | PAL/MM 新 Tauri 入口已走查；修复画布名称上下文遗漏 |
| D08 | 103 | W02/W04/W05 | GEN/COD/PAL/MM/CS 分角色的正式任务 | W02 四角色有新证据；GEN 仍 FAIL，完整集合与 CS 待走查 |
| D09 | 375 | W06 | Job 子孙进程、崩溃、恢复、撤权、并发与 LONG | 先故障注入，后 soak；待走查 |
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
| W03 文件/Git/SSH | D03/D05/D11 剩余适用 Case | 先根内读写负向，再隔离 remote/SSH 与外部效果；逐簇修复 | 待走查 |
| W04 UI/扩展与恢复 | REAL、OBS、D06；旧 MM retry | 先复现旧失败会话按钮/状态/Promise 与 canonical Turn，定位 spinner，保留首次失败 | 待走查，不重写旧 Snapshot |
| W05 条件业务资源 | ACSR、媒体、Channel、Robot 及其 D07/D08 Case | 建最小正式入口/模型/测试租户；无前提不计 PASS，不擅自扩权 | 部分 BLOCKED_FIXTURE |
| W06 生命周期与长稳 | LIFE/CONC/LONG、WIN-015/016/018 | 逐状态故障注入、取消/重启/lease、宿主 UI/API 一致性，最后长稳统计 | 待走查 |

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

### W02 核心入口首批（基线 `13da6ca3f`）

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

下一步：W03 定向文件/Git/SSH；W01-B 剩余跨盘/ACL/并发与新建 alias 继续保留在该队列；
旧 MM retry 在 W04 保持待复现，不因本批通过关闭。不要重建 2,374 行日志/状态文件到 Git。
