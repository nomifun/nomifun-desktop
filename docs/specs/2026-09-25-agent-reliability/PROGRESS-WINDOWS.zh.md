# Windows 命令与会话可靠性进度

更新：2026-10-03。当前宿主 Windows。执行顺序见 [实施计划](IMPLEMENTATION-PLAN.zh.md)。
用户2026-09-30已收敛为简单系统命令、步骤、过程状态和结果可靠性；活动范围以最新实施计划为准。
旧675＋82、2,374槽保留为历史全产品参考，不再领取其全部余项。共享根因及已有Windows修复保留。

## 既有综合会话证据（追溯，不再整组复跑）

| 场景 | 入口与候选证据 | 当前状态 |
| --- | --- | --- |
| A 观察、只读、小测试 | W245正式COD十一阶段/四压缩完成，全部观察/两测试各一次，原文/SHA/搜索/Git修改已交付，具体交付4/6 | 整体FAIL：实际cwd/完整九条目仍缺，报告旧引用一拒后纠正，三个英文提示；完整首发/N3/GEN待验 |
| B 文件、进程、停止 | W209文件链N3；W250正式GEN使用来源SHA、18-byte文件链及最后回读；echo/hold各启动一次，父子取消159ms | 整体FAIL：没有stdin写入、实际EOF为0 bytes，hold重复cursor0；未交付报告后Stop，原20/28保留。W240来源保护首败不覆盖 |
| C 连续、纠正、恢复 | W248/249同Turn纠正、实际压缩、活父子Stop后385/536ms消失；无自cancel/重启，冷读无复活 | 整体FAIL：传到工具的content漏末LF、17bytes；W249未触发新的引用定位反馈，原21/24与必要N3/入口待验 |

截至W250，旧综合场景仍为0/3，不改历史断言或把子链拼成完整PASS。最新实施计划优先采用缺陷驱动收尾，
不再为这个分母串行复跑整组或固定N3/角色矩阵。实际体验仍有下面两类未完成项，保留失败与边界。

GEN/COD已各有正式执行，源码未受影响的操作/EOF/停止/冷读证据直接复用；统计稳定性及发布认证另列未认证。
现有cargo回归按受影响代码定向复用；业务UI常规手测、外部生态和旧82条专属全队列不阻断本轮收尾。
共同产品根因和核心体验要求仍须据实说明；不把生成精度残余当业务非零，也不假称已经解决。

2026-10-02同步379a54052的最新实施计划：停止按B→C→A整组排程，改为具体缺陷→最小回归→必要的一次正式UI。
同一未改根因不付费循环，暂停纯说明微调和历史矩阵扩展；只修有证据的产品原因，已通过子链复用。

### 当前交付与残余（W264；既有机制复用）

- 多项结果交付：W255正式COD短任务13秒、三model步骤，cwd/完整九条目及真实Hidden标记/文件首尾与摘要全部交付，首次report引用逐项正确、无重复观察，20/20不同定向检查。历史真实性由W254正式GEN两步/一次实际压缩/13项复用；仅关闭这两条针对性链N1，原完整A/B首败仍FAIL。
- 已修机制交付：命令形态与结果回配、非零与系统错误区分、文件原件/字节保护、实际Stop及取消冷读、压缩硬预算和历史回执保留，按各批直接证据复用。没有因本次短任务追加全仓构建、完整A/B/C、固定N3或角色矩阵。
- stdin接合：W257正式GEN短链UI18秒，六model步骤/一次压缩，完整7请求及SSE已录制；六原生调用参数与canonical逐项相同，发送/EOF/实物均18 bytes，精确游标0→11→119、exit0/reaped和中文交付成立，21项不同定向断言。只证明当前短链N1；W250旧拒绝未复现，其遗漏唯一原因仍未证。
- 提议/步骤精度：B漏stdin及重复cursor0，C末LF，W240来源SHA首败继续开放。0/17-byte实物及原调用保留，不自动补步骤/字节或改断言。W256确认关闭前摘要仍保留stdin待办、冻结快照允许input；原拒绝输出及实际请求工具表缺失，漏步骤的唯一根因仍待证，不由组件或reasoning字段代判供应商/宿主责任。

确定产品缺口：W250摘要漏拦tool_call包装、W251/253历史回执丢失。后者已修，W254正式短任务13/13，完成一次当前读取并如实交付旧EOF0；W251首败仍保留，不关闭完整B或所有生成精度问题。
共享退出码组件回归复用；W259人工托盘Quit正常空闲N1已验。W264当前正式产物自主调用已有原生退出API：活动Turn正常exit0、真实写锁清理失败exit1均已验；冷恢复未重放已完成写入，但本地模型复用旧call ID被拒绝，恢复任务完成未验。活动托盘点击、其他失败分支及完整平台认证仍未验。
上述生成精度问题仍未完成。W259确认原W250压缩摘要及保留回执含cursor25、后续完成提议仍为0，实际历史HTTP/SSE缺证；发布稳定性/旧完整矩阵另列未认证。没有新的产品反例或根因变更，不追加付费循环；不展开专属全队列或假称共享全阶段完成。

2026-10-02 W252方案纠偏：实施计划已删除残留B→C→A/N3活动门槛，按30分钟定位时间盒和
每个直接修复最多一次短UI验证收尾。W254/255各补一条已有修复的明确覆盖缺口；阶段性产品交付与生成精度必修范围待用户明确，
不因等待或方案调整把已有正式FAIL改记PASS。

2026-10-02整体复盘：最后按实施计划的四个工作包结算，预计1～2小时有效工作用于机制/残余/退出
边界与最终交付，不能解释为全部Case达标时间。W257从建目录至提交18分35秒、实际任务18秒，
夹具/观察/核账占主要时间；W256/257未关闭旧漏步骤根因，不继续补同根因正向样本。单主agent，
Cargo/UI各一个活动运行，审查和文档合并结算；新产品反例出现才修源码并最小验证。Windows退出
可达性诊断最多15分钟，未验/阻断照实交付；生成精度全部修复的完成时间尚不可可靠估计。

W258已完成上述“机制结算＋生成残余归类”：12份已有独立断言、当前源码差异和首败边界已核对，
见共享页八簇交付表；没有新模型/测试/构建。Windows正式退出入口已结算为BLOCKED_UI_REACHABILITY：
原W257隔离data/work、新profile的零模型冷实例，15分钟内未取得托盘实际Quit，实际码/清理未证。
实例一直存活，原观察器到期未重启；到点核对身份后仅清理空闲夹具。原复杂A/B/C及所有生成偏差
保持原状态，阶段交付不代判全阶段。核心机制、残余、平台阻断及发布未认证项已分列，不追加正向复跑。

用户新目标明确允许按四包结案并将退出阻断交接人工；[阶段交付总结](DELIVERY-SUMMARY.zh.md)已给出
对应产出与重新准备实例后的验证步骤。W258有限阶段交付时Windows正式Quit仍BLOCKED，
生成残余和旧完整A/B/C状态不变；当前没有活动测试/GUI或新的模型调用。
后续W259已取得正常空闲Quit的新实例正式证据，原W258阻断保留；详见本页末记录。

## 历史全产品统计口径（本轮已停用）

| 范围 | 固定Case分母 | 现有可确认信息 | 完整结案数 |
| --- | ---: | --- | --- |
| 共享 | 675 | 正文明示的批次范围引用171个ID，公共P0 5/5任务已验 | 待逐证据核账，不能记0或171 |
| Windows专属 | 82 | 正文明示的批次范围引用17个ID，专属余项未系统推进 | 待逐证据核账，不能记0或17 |

W182是批次号；Windows全部适用槽仍为2,374。引用数量不证明执行/PASS，未提及也不直接证明未跑。
该历史统计不作为新任务或本轮完成率。旧FAIL/recovered继续保留；20次/100 seed/LONG/99%归发布认证。
本轮按上述A/B/C及原痛点断言结案，缺夹具只阻断相应场景，不能用组件PASS代替正式执行。
统计依据在外部 `2026-09-30/progress-audit/`；本次仅修正进度管理，不新增测试、模型调用或大型索引。

## 历史Windows专属集合（不全量排程）

以下集合共 82 条，其余本平台任务取目录中的 Both Case；范围包含首尾。

| 家族 | Windows 专属 ID |
| --- | --- |
| CMD | 002、006～009、013～014、016、020、022、024、026、028、031～032、037、039、042、044、046、048、050、052、054、057、059、063～064、066、068、070、072、074、077、079、113、124～126、131、133 |
| PROC/TERM/FILE | PROC-005/013/021/045；TERM-011；FILE-021 |
| Browser/Computer | BROW-016；COMP-007 |
| UI/宿主 | REAL-001～015；WIN-001～018 |

## 历史全领域分配（按新计划选取）

下表是原全产品Windows槽分配；完整家族/ID以历史目录为参考，活动任务只执行新计划A/B/C命令主线。

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

## 历史任务入口（只取命令关联部分）

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

### 正式 Realtime/WebSocket 消费方的 settlement error 断连恢复（W155，基线 `10ae42f3c`）

- S-D09-36 / OBS-014/018/020、PORT-012、A03/A06/A08/A09/A17/A19：在真实 TCP 上启动完整
  产品 Router（`create_router` 内含正式 `forward_user_events` 桥接），TrustLocalToken
  认证；owner 走桌面 webview 握手（`tauri.localhost` Origin + `Sec-WebSocket-Protocol`
  密钥）连 `/ws`，第二用户持不同 user_id 的 JWT Bearer 连接。经正式 `POST /turns`
  admission 派发到注入的 Runtime mock，由 canonical Store 提交 tool/call-started →
  tool/result-recorded（CAPABILITY_UNAVAILABLE + “Do not retry”）→ turn/failed，并经
  stream relay 发对应 ToolCall/Error 帧。
- Turn 1 在 tool_call running 帧送达后断开 socket：settlement 在 owner 零连接期间落库；
  重连后 socket 无任何补推（易失总线不重放），消费方改由 `GET events?after_seq` 逐页
  replay——call/result/terminal 各恰一条、seq 严格递增无重复，`GET messages` 与
  `message-history` 各恰一条 error tool row 并保留完整恢复指引；event cursor 与
  history cursor 双向混用均 400 fail-closed。
- Turn 2 验证重连后的实时投递：同一 socket 实时收到 turn.started、tool_call running、
  含 CAPABILITY_UNAVAILABLE 与 “Do not retry” 的 tool_call error、stream error 与
  turn.completed state=error；同 idempotency key 重放返回 replayed/completed 及同一
  terminal 事实，不再推帧、不新增 canonical 事件。第二用户全程静默。
- 新增场景首次通过并连续 **20/20**；`websocket_e2e` **18/18**，W152～W154 cursor 相邻
  回归 **4/4**，fmt/diff 通过。生产代码无需修改（正式 Realtime 链路已满足语义），无正式
  Tauri renderer。
- 未覆盖正式 Tauri renderer tool row、PORT-012 watcher 丢批/乱序对账、该场景的 WS
  lag/resync 额外注入（桥接 coalesce 已有独立测试）、其他平台及 N3/100 seed/LONG/99%；
  不关闭完整 PORT/OBS 或共享阶段。

### `workspace.files/changed` 残余丢失信号（W156，基线 `9ffbb3787`）

- S-D03-52 / FILE-040、PORT-012、A05/A15/A17/A19：补齐 watcher 后端两处静默丢失。
- 发现一：native change 类事件若不带任何 path、或全部 path 落在 watched root 之外
  （可能是跨越边界的 rename 尾部），原先在 `record_native` 中不产生事件、不计
  `dropped_event_count` 也不置 `rescan_required`——变化完全消失。修复后零可归因
  in-root path 的 change 事件统一置 `rescan_required`；`.nomifun` owner 组件过滤、
  非法名（计 dropped）、空 relative（root 自身变化→rescan）仍算 handled，不放大信号。
- 发现二：`pre_turn_context` 先 drain 再 `batch.validate().ok()?`/`to_string().ok()`，
  drain 批次在投递侧失败时返回 `None`，对后续 Turn 零信号（比 overflow 丢弃更差）。
  修复后 `WatchQueue::take_batch` 在投递失败时把 `rescan_required` 滞留回队列，
  下一批强制全量对账。
- 重复/乱序语义锁定为新回归：debounce 窗口外的重复事件与同路径乱序事件按到达顺序
  原样送达、`dropped` 保持 0——批次不承诺顺序或唯一性，真实磁盘仍是唯一事实。
- **首败已保留**（`phase-2-3\2026-09-29\windows\W156\run-1-first-failure.log`）：
  仅把新测试打到旧码上运行，两条均失败（不可归因 native change 无 rescan；
  投递失败批次静默消失）；乱序/重复测试在旧码上即通过（记录既有正确语义）。
- 修复后 `nomi_core_wave2::tests` **22/22**、连续 **20/20**，`cargo fmt --check` 通过。
- 未覆盖正式 UI 全量对账、模型是否遵循重读提醒、drain 后未被消费的批次
  （contributor 为 fire-and-forget）、macOS 及 N3/100 seed/LONG/99%；不关闭完整
  FILE-040、PORT-012 或共享阶段。

### 真实磁盘满与 journal IO fault（W157，基线 `9c6cbdf97`）

- S-D09-37 / LIFE-003/006/024、FILE-038、G0-029/030、A04/A05/A07/A17/A19：以
  `PRAGMA page_size=512`+`VACUUM`+`max_page_count` 在 canonical Store 真实写路径产生
  SQLITE_FULL（非 mock），并以目录占用 `agent.db-journal` 让 rollback-journal 写事务
  在 journal 创建处拿到真实 CANTOPEN，读路径不受影响。
- 发现产品缺陷：写事务中 SQLITE_FULL 使 SQLite 自动回滚事务，被 drop 的 sqlx
  Transaction 排队的 ROLLBACK 找不到活动事务而失败，连接 worker 的
  transaction_depth 永久停留在 1——该池化连接此后所有 `begin_with` 均以
  InvalidSavePointStatement 失败，磁盘恢复后重试仍被 `CAPABILITY_UNAVAILABLE`
  （误报为 unsettled 文本）拒绝，连接不驱逐就永不恢复。
- 修复：`AgentSessionStore::begin_write_transaction` 捕获该 desync 错误后探测并
  `close()` 剔除失步连接（池自动补新连接），再重试一次；健康连接经
  BEGIN IMMEDIATE+rollback 探测后原样归还。仅扩展该唯一写事务入口，所有
  admission/terminal/reconcile/普通写共享同一恢复路径，无吞错、无权限放宽。
- 五条回归：admission 满盘 CAPABILITY_UNAVAILABLE（含真实 “database or disk is
  full” 原因）且零文件零 Effect，恢复后同 key 恰一次执行；owner 已写文件的
  success terminal 落库失败保留 Pending、期间重放由 fence 拒绝（文件未二次写）、
  恢复后显式重试恰一次 Returned；owner 失败 terminal 落库失败保留 Pending 与
  owner 错误；uncertain terminal 落库失败保留 owner 原因，恢复后进入 Unknown；
  journal 目录占位下读健康、写 CANTOPEN，移除后同连接池恢复恰一次。
- **首败已保留**（`phase-2-3\2026-09-29\windows\W157\run-2-prefix-red.log`）：
  pre-fix 3/4 在“恢复后显式重试”处失败且均报误导性 `non-zero transaction
  depth`；failed-terminal（只验证满盘时 fence 保 pending）与 journal IO fault
  两项在旧码上即通过，记录既有正确语义。
- 修复后新增 **5/5**、`agent_wave2_host` 模块 **72/72**、`nomifun-agent-session`
  lib **76/76**，`cargo fmt --check` 通过。
- 未覆盖 fsync 中途故障、WAL 文件级损坏、写锁并发+满盘、正式应用 shutdown
  竞态、多连接池拓扑下的驱逐路径、macOS 及 N3/100 seed/LONG/99%；不关闭完整
  LIFE/G0/FILE 或共享阶段。

### writer lock 后命中真实磁盘满（W158，基线 `d4d0379aa`）

- S-D09-38 / LIFE-003/023/024、CONC-014、FILE-038、G0-029、A04/A05/A07/A17/A19：独立连接持有
  真实 SQLite writer lock，同时把 production Store 单连接 pool 限制在当前 page count。正式文件写先
  在 `BEGIN IMMEDIATE` 等待；200 ms 时零文件/零 Effect。释放 writer 后同一 admission 精确命中
  `SQLITE_FULL`，仍零副作用并返回真实 full 原因。
- 解除 page budget 后，同 pool/host、同 operation/key 显式重试只执行一次并得到唯一 Returned Effect；
  W157 的失步连接驱逐没有误驱逐健康锁连接或留下 depth 污染。
- 新增场景首次及连续 **20/20**；单独 disk-full、busy-timeout、rollback-journal IO fault **3/3**，
  fmt/diff 通过。生产代码无需修改，无 UI/模型。
- 未覆盖 terminal 阶段的 busy+full、多个受限 pool 同时失步、fsync/WAL 损坏、正式 shutdown、其他
  平台及 N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/G0/FILE 或共享阶段。

### writer lock 后 success terminal 命中磁盘满（W159，基线 `7da1cb06a`）

- S-D09-39 / LIFE-006/023/024、CONC-014、FILE-038、G0-030、A04/A05/A07/A17/A19：先 reserve
  canonical Effect 并由实际 `FileService` 发布文件，再让 terminal settlement 在独立 SQLite writer
  lock 后等待；释放 writer 后同一 terminal 写精确命中 `SQLITE_FULL`。
- 返回值同时保留 owner 已成功、full 原因与禁止自动重试；Effect 仍为 Pending，文件字节保持且未
  二次发布。解除 page budget 后，同 reservation 只补写一次 receipt，Effect 唯一变为 Returned。
- 新增场景首次及连续 **20/20**；单独 disk-full terminal、W158 busy-full admission、普通 busy
  terminal **3/3**，fmt/diff 通过。生产代码无需修改，无 UI/模型。
- 未覆盖 failed/uncertain terminal 的 busy+full、多个受限 pool 同时失步、fsync/WAL 损坏、正式
  shutdown、其他平台及 N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/G0/FILE 或共享阶段。

### writer lock 后 failed/uncertain terminal 命中磁盘满（W160，基线 `943dd8035`）

- S-D09-40 / LIFE-007/023/024、CONC-014、G0-030、OBS-006、A04/A07/A08/A17/A19：managed owner
  确定失败与 external owner outcome unknown 分别 reserve Effect；terminal settlement 先等待独立
  SQLite writer lock，释放后命中真实 `SQLITE_FULL`。
- 两类均返回 full、禁止自动重试并保留各自 owner code/message；Effect 保持 Pending。解除 page
  budget 后只补 terminal receipt，分别唯一归约为 Rejected 与 Unknown。
- 首版 ASCII padding 恰好落入页内空隙，settlement 成功使夹具 `unwrap_err` 失败；首次失败日志保留。
  改用多字节大诊断强制跨页分配后首次及连续 **20/20**；相邻 failed-full、uncertain-full 与 W159
  success busy-full **3/3**，fmt/diff 通过。生产代码无需修改，无 UI/模型。
- 未覆盖多个受限 pool 同时失步、fsync/WAL 损坏、正式 shutdown、其他平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/G0/OBS 或共享阶段。

### 两个独立 Store pool 同时失步与并发恢复（W161，基线 `fb35d3b57`）

- S-D09-41 / LIFE-003/024、CONC-014、G0-029、A04/A05/A07/A17/A19：同一 SQLite 文件上的
  两个独立单连接 production Store pool 分别服务独立 Session 与 workspace resource；各自设置连接级
  page budget 后，两条 admission 均真实命中 `SQLITE_FULL`，保持零文件/零 Effect，并让两个连接失步。
- 分别解除预算后并发显式重试，两个 Store 各自驱逐自己的坏连接并各生成唯一 Returned Effect；改写
  文件后同 key 并发重放不覆盖用户字节，Effect count 仍各为 1。
- 前三版夹具依次触发精确 tool causation、连接级 PRAGMA 与物理资源唯一 fence 的正确拒绝，另一次
  补丁定位导致编译失败；日志均保留且不计产品失败。纠正后首次及连续 **20/20**；单 pool 满盘、
  busy→full、资源 fence 相邻回归 **3/3**，fmt/diff 通过。生产代码无需修改，无 UI/模型。
- 未覆盖同一 pool 内多连接同时失步、fsync/WAL 损坏、正式 shutdown、其他平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/G0 或共享阶段。

### 同 pool 内坏/健康/坏连接交错恢复（W162，基线 `d7fc83b21`）

- S-D09-42 / LIFE-006/024、CONC-014、G0-030、A04/A07/A08/A17/A19：三连接 production Store
  pool 内两条 terminal 写真实命中 `SQLITE_FULL`，各自保持 Pending；解除页限制后按坏/健康/坏顺序
  归还连接，首次重试仍以 non-zero transaction depth 失败。旧驱逐扫描在首个健康连接处提前退出。
- 修复唯一写入口：保留取得的精确连接，ping 等待排队回滚，只关闭深度仍非零的坏连接；直接在取得
  的健康连接上 `BEGIN IMMEDIATE`，有界取得替代连接。两条显式重试各补写唯一 Rejected，健康连接
  的临时表标记保留；不重放业务写事务、不吞掉真实存储错误。
- 首版 admission 夹具未跨页分配的失败、terminal 夹具的真实产品首败分别保留于外部
  `2026-09-30/windows/w162-interleaved-desynced-connections` 的 01/02 日志；修复后首次及连续
  **20/20**，Wave2 host **77/77**、Session Store **76/76**，fmt/diff 通过。无正式 UI/模型。
- 未覆盖全池失步、并发驱逐/池关闭、fsync/WAL 损坏、正式 shutdown、其他平台及
  N3/100 seed/LONG/99%；不关闭完整 LIFE/CONC/G0 或共享阶段。

### 全池失步并发恢复及 receipt 时间幂等（W163，基线 `b7bd9b231`）

- S-D09-43 / LIFE-008/011/024、CONC-014、G0-025/030、A03/A04/A06/A07/A08/A09/A17/A19：
  三连接 pool 全部因正式 terminal 写的 `SQLITE_FULL` 失步；每 seed 改变连接归还顺序与并发调度，
  解除预算后各 Session 正确归约。但相同已结算 receipt 重送因新的 `recorded_at` 被误报冲突/未决。
- 修复 Store terminal API：在同一写事务中保留首次提交时间，原精确去重继续比较全部身份及 owner
  结果。结果、operation、owner、digest、resource、producer、cause 或终态变化仍冲突；原 ack 不变。
- 首版载荷 oracle 错误与产品首败分别保留于外部 `2026-09-30/windows/w163-all-desynced-pool` 的
  01/02；02 数据库保留。独立 SQL oracle 列名错误亦保留。修复后 **100/100 seed**、完整重复
  **20/20 × 100 seed**；21 份独立 SQLite 核对各为 300 条精确 owner 原因/唯一 Rejected receipt、
  2,400 条连续事件，重放新增事件为 0；Session Store **76/76**、相邻满盘 **7/7**，fmt/diff 通过。
- 未覆盖池关闭/取消与驱逐组合、fsync/WAL 损坏、正式 Tauri shutdown、其他平台及完整
  N3/LONG/99%；本批无 UI/模型，不关闭完整 LIFE/CONC/G0 或共享阶段。

### 磁盘重开后 unknown reconciliation receipt 重放（W164，基线 `e78708426`）

- S-D09-44 / LIFE-007/008/011、G0-025、A04/A06/A07/A09/A17/A19：相邻 `reconcile_effect` 的同
  receipt 仅时间变化仍误报 IdempotencyConflict；外部 W164 `01-first-product-run.log` 保留首败。
- terminal 与 reconciliation 共用保留首次时间的事务写入口。关闭全部连接再重开磁盘 Store，同一
  核对回执返回原 record/ack；改变结果或资源仍冲突，Unknown 与 reconciliation fence 保留。
- 磁盘重开首次及 **20/20**；21 个独立 DB 核对均为唯一 StillUncertain 回执、原时间 20、8 条连续
  事件、零重放新事件；Session Store **76/76**、相邻 terminal 身份/删除 fence **2/2**，fmt/diff
  通过。日志与 DB 保留于外部 `2026-09-30/windows/w164-reconciliation-time-replay`。
- 无正式 UI/模型；confirmed outcome 完整矩阵、真实外部 owner、池关闭/取消、正式 shutdown、其他
  平台及完整 N3/LONG/99% 仍开放，不关闭完整 LIFE/G0 或共享阶段。

### 正式 Desktop host shutdown 与真实窗口隐藏/再显示（W165，基线 `48f7c6f06`）

- S-D09-45 / LIFE-015/019/029、FILE-038、A04/A06/A11/A13/A17/A19：真实 TCP 受控 provider 经
  正式 DesktopServer API/Runtime/FileService 完成唯一写入，再保持模型流；完整 shutdown 首次及
  **20/20** 为流释放、唯一 cancelled、Returned/文件保留、listener 关闭、重复 shutdown 零新事件。
  21 个独立 DB/文件核对通过，fmt/diff 通过；新增仅最小回归和等待流夹具，无产品修复。
- 最新配对前端/正式 Tauri、1280×832、隔离 data/work/profile 共三个 UI 回合；一次有效隐藏和同根
  单实例再显示后，同一运行 Turn/流、调用数和 effect 数保持。三次写入均 Returned；全部暂停回合
  经正式结束按钮取消。原 120 秒 provider 超时、过晚停止的 stale UI index、夹具借用编译失败及
  `autotests=false` 导致未注册目标的首败均保留于外部 `2026-09-30/windows/w165-tauri-shutdown`。
- UI N3/20 次及托盘真正退出未达：工具没有返回托盘窗口。最终只对匹配隔离数据根的 GUI PID 做
  强制清理，provider 通过自有 HTTP shutdown，owned/profile 进程清零；强制清理不计 graceful PASS。
  无真实模型；满盘/IO fault 下 shutdown、真实子进程、其他角色/平台、N3/100 seed/LONG/99% 仍开放，
  不关闭完整 LIFE/FILE Case 或共享阶段。

### 正式 shutdown 写锁失败与原清理见证重试（W166，基线 `846816037`）

- S-D09-46 / LIFE-019/023/024/029、CONC-014、FILE-038、G0-030、A04/A06/A07/A11/A13/A17/A19：
  唯一文件写入 Returned 后，独立 SQLite 连接持有真实 writer lock；正式 DesktopServer 第一次退出
  失败，解除锁后原实现第二次却返回成功并关闭 DB，留下 running Turn/head/owned lease。
- 修复 SDK task join 与 durable cleanup 混淆：保留 exact message/outcome/cleanup 状态，失败 flight
  不释放 Session，后续显式 teardown 只补清理/回执。host flush 队列按 ack 消费，journal uncertain
  仅接受原 cleanup/terminal 的 exact payload/kind/model identity 重试；新 progress 与不同事实仍拒绝。
  首次 terminal commit 前取消优先，已确认 outcome 不重执行；异常 task 仍尝试资源释放并保留失败。
- 首败日志及 DB 保留于外部 `2026-09-30/windows/w166-shutdown-sqlite-lock/01-first-product-run`。
  修复后首次及 **20/20**，21 个独立 DB/文件核对均为唯一 cancelled、cleanup witness、Returned、
  ready head/连续事件，零 running owned lease；AI Agent **331/331**、journal **10/10**、正常
  shutdown/pause-resume **2/2**，fmt/diff 通过。
- 无正式 UI/真实模型；真实满盘/WAL/fsync、lease 过期与跨重启、更多取消/并发驱逐拓扑、其他角色/
  平台及 N3/100 seed/LONG/99% 仍开放，不关闭完整 LIFE/CONC/FILE/G0 或共享阶段。

### Cleanup 部分投影与初始化前取消（W167，基线 `ff587afc9`，验证含 W168）

- S-D09-47 / LIFE-009/011/019/023/024/029、CONC-014、A04/A06/A07/A10/A13/A17/A19：正式
  factory/Kernel/canonical root 配合独立 SQLite writer lock 和 projection trigger，四个反例首次均失败：
  旧 step completion 丢失、bootstrap 部分提交后永久 exact mismatch、未初始化 terminal 内存假确认、
  claim 写锁错误被 cleanup 吞掉。只读复核与两条代码通道使用独立文件范围，Git 由 root 串行处理。
- 旧 cursor 及 typed 初始化队列只在持久化确认后消费；claim failure 原样返回，terminal 缺 admitted
  authority 正式拒绝。owner/snapshot/holder/fence 与原 exact retry 保留，模型和工具不重执行。
- 外部证据根 `2026-09-30/windows/w167-cleanup-projection-retry`：`01` 编译、`02` 缺 workspace
  的夹具失败分别保留；journal 最早产品首败日志在 `02`，`03` 四个产品反例的 DB/日志完整保留。
  最终 **4/4**、同构建 **20/20 × 4**，独立 SQL **84/84** 验证文本 causation 链、63 个唯一 cancelled/
  cleanup witness、ready head/连续事件及零模型 admission/effect；journal **11/11**、build identity
  **1/1**、fmt/diff 通过。正式 Desktop 健康/写锁 shutdown 相邻 **2/2**，独立 DB/文件核对 **2/2**。
  未选用 Knowledge broker 的 pipe 权限 unavailable 与既有构建 warnings 留在外部日志，本批未验证该能力。
- 未覆盖 pending steering 局部 flush、commit 后 ack 丢失、跨重启/lease 过期、真实满盘/WAL/fsync、
  正式 UI、其他平台/角色及 N3/100 seed/LONG/99%；后续继续共享队列，不关闭完整 LIFE/CONC 或共享阶段。

### 真实长 poll 与进程 shutdown（W168，基线 `ff587afc9`）

- S-D04-30 / PROC-040、LIFE-029、A03/A11/A13/A17/A19：独立 PID 标记和精确 OS handle 证明
  pipe child 存活，60 秒 poll 已进入等待；shutdown 在 6 秒内不能清理。释放 poll 后正式 shutdown
  才取得唯一 Cancelled/reaped。证据根为外部 `2026-09-30/windows/w168-process-start-shutdown`，
  产品首败保留于 `01-first-long-poll`。共享修复分开 lease/容量保护与关闭阻塞计数，仅 poll 可随原 Session 退休，
  stdin/close/resize 等写操作的关闭屏障保持；原 poll 最终观察同一清理终态。
- 旧 75 毫秒夹具重复第 13 轮返回真实 Lost/reaped=false，精确 handle 的兜底强杀单列，未计通过；
  原日志与分类保留于 `07-repeat-20/run-13`。新回归采用生产默认 1/1/3 秒预算，原 6 秒上界、
  Cancelled/reaped、唯一 owner/report、重复 shutdown 幂等及 PID 消失断言全部保留。
- 最终首次及重复 **21/21**，21 份独立 PID/report 核对、helper 残留 0；shutdown **5/5**、
  natural exit **5/5**、cancel-first **1/1**、registry **13/13**，fmt/diff 通过。两条代码通道
  使用独立文件范围，重型 Cargo 构建串行；本批没有模型/UI。
- 未覆盖真实 start/shutdown、start future drop、正式 Tauri 正常/强退、ConPTY 父死亡、macOS、
  其他角色及 N3/100 seed/LONG/99%；不关闭完整 PROC/LIFE 或共享阶段。

### pending steering 清理重试与已取消 root（W169，基线 `c3b3dc9ac`）

- S-D09-48 / LIFE-011/019/023/024/029、CTRL-018、A04/A06/A07/A10/A13/A17/A19：三项产品首败
  是 private/public 正文写失败后原记录丢失，以及 canonical 已取消、SDK 首次 driver poll 前 cancel
  被 W167 的严格 Running admission 挡住。Cleanup 正文改按 ack 消费原队列；gen0 的取消只读验证
  同 root/operation、正文、principal/当前及 accepted Snapshot-route、真实 terminal 因果链和 ready
  head，确认原 Cancelled0，零新 claim/模型/资源。不同 root/正文/步数和 Completed 仍拒绝。
- 外部 `2026-09-30/windows/w169-steering-cleanup-retry` 的 `01` 三项产品首败/DB、Deferred 空 Vec
  序列化 oracle 夹具失败和 `02` sqlx 错误映射编译失败分别保留；typed 空 Vec 断言保留原业务事实。
  Root 独立 oracle 的 partial/Cancelled completion 口径错误日志另留，不计产品失败。
- 最终新旧 **9/9**、新五项 **20/20 × 5**，独立 SQL **105/105**：63 个正文/Deferred/Cancelled
  场景、21 个 SDK pre-cancel 零增量场景、21 个 Completed 拒绝；journal **11/11**、build identity
  **1/1**、fmt/diff/安全复核通过。最终 W169+W170 源组合的正式 Desktop shutdown **2/2**、两份独立
  DB/文件核对通过；源码及二进制身份在外部记录，完整日志/DB未进 Git。
- 未覆盖其他消息字段、claim 后尚未安装 ActiveTurn、跨重启/lease、正式 UI、其他平台角色及完整
  N3/100 seed/LONG/99%；不关闭完整 Case 或共享阶段。

### 原生 start/交付丢弃与关闭 fence（W170，基线 `c3b3dc9ac`）

- S-D04-31 / PROC-039/040/042、CONC-004、A03/A10/A11/A13/A17/A19：原生 await 已创建 child，
  caller drop 后 shutdown 空报告且 exact handle 仍 live。宿主持有 start worker、准入 read/预留与
  结果 ACK；未 ACK 同一 Session 继续清理，Windows resume_gate/Unix flag 原生取消和预算保留。
  中间实现的 quiesce 空 exact fence 后旧 start 继续执行首败另留，最终 public first poll 已取得
  read_owned/预留并移交 worker，ACK 阶段释放 gate。
- 外部 `2026-09-30/windows/w170-process-start-shutdown-race` 的 `01` 原始产品首败、`02` 测试
  借用编译失败、`04-first-quiesce-regression` 引入失败均保留。最终 `05` **5/5**、`06` **20/20 × 5**，
  独立 PID/磁盘 **105/105**、自有 helper 0；原生取消/deadline/关闭相邻 **11/11**、API **21/21**、
  WSL Linux lib 单包兼容编译及 fmt/diff/安全复核通过，不计 macOS 验收。
- 未覆盖 startup failure/unknown/worker panic、ConPTY 本组真实 start 竞态、正式 Tauri、其他
  平台/角色及完整 N3/100 seed/LONG/99%；不关闭完整 Case 或共享阶段。

### 取消回执附件/技能提示/origin 匹配（W171，基线 `254d3c3f1`）

- S-D09-49 / G0-025、CTRL-018、LIFE-019/029、A03/A06/A09/A10/A14/A17/A19：同一 cancelled
  root 的 files/inject_skills/origin 改动后，原 cleanup 与 terminal均返回成功；两反例 **0/2** 首败
  保留于外部 `2026-09-30/windows/w171-cancel-delivery-identity/01-first-product-run`，DB/日志不覆盖。
- gen0 取消证明复用有界 delivery 解析器，精确比较完整数组/顺序及 origin Option；合法附件引用和
  origin 经真实 SDK send→cancel→teardown保持原取消，无附件读取、Skill激活、claim或新模型。
  既有 legacy text-only fixture 载荷保持，explicit 空数组/null 与非空附件/origin独立验证。
- 修后首次 **2/2**、同构建 **20/20 × 2**，42份独立 SQL 核对原 terminal/source metadata、连续事件、
  ready head、gen0/无owner、零模型/effect/增量；原清理相邻 **9/9**、fmt/diff通过。日志/DB/源码和
  binary身份均在外部，单 agent 仅两次定向构建；未跑无关全库或正式 UI。
- wrapped delivery、非空已选择 Skill、claim 后未安装 ActiveTurn、其他平台/角色和完整
  N3/100 seed/LONG/99%仍开放；不关闭完整 Case 或共享阶段。

### 已 claim 的准备等待方取消与原 owner 收尾（W172，基线 `c5d615d60`）

- S-D09-50 / LIFE-011/013/014/019/029、G0-025/030、A03/A04/A06/A07/A10/A12/A13/A14/A17/A19：
  原实际 SQLite await 中 generation=5/owner 已持久化但 ActiveTurn 为空；canonical cancel 后
  caller drop，原 cleanup 无法重新解析 Running authority。首次 **1/2**，日志及 DB保留于外部
  `2026-09-30/windows/w172-claimed-preparation-cleanup/01-first-product-run`，普通 drop 对照旧码已通过。
- 宿主持有 prepare task，cleanup 先等完成；原 journal 在后续 budget/receipt await 前安装，
  取消后只保留清理身份并跳过资源打开，原 root/holder/generation/fence不替换。不同 execution
  owner 仍拒绝；read/claim 错误、预算、公开 open_journal 和已有恢复入口保持严格行为。
- 最终 **3/3**、同构建 **20/20 × 3**，独立 SQL **63/63** 验证42个原 owner cleanup witness、
  21个foreign拒绝、唯一 claim、fence0、ready head/连续事件、零running lease/模型/effect。原清理/
  metadata相邻 **11/11**，实际 API pause/resume及冷启动恢复 **2/2**，fmt/diff通过；完整证据及
  binary/源码身份在外部，单 agent仅直接相关构建/检查。
- preparation panic/commit ack失落、长附件准备与超时、更多恢复并发、正式 UI、其他平台/角色及
  N3/100 seed/LONG/99%仍开放；不关闭完整 Case 或共享阶段。

### 文件恢复最后名称窗口（W173，基线 `82478c3e3`）

- S-D03-59 / FILE-020/025/039、A05/A13/A14/A17/A19：在 native restore 最后身份检查后尝试
  原 backup 重命名和外来对象覆盖 backup，真实 POSIX remap 均以sharing violation=32拒绝；
  同一 remap 在 guard释放后成功，证明夹具能实施竞态。并发创建的foreign hardlink target
  使no-replace恢复返回AlreadyExists，原 backup与并发对象字节/身份不变。
- 首次 **2/2**、同构建 **20/20 × 2**，42份独立磁盘/目录项及hardlink身份核对通过；cleanup
  模块 **4/4**、fmt/diff通过。现有生产实现首次即满足，本批只有最小回归，无新产品FAIL/模型/UI。
  全部现场、日志、源码/binary身份保留于外部 `2026-09-30/windows/w173-file-recovery-window`。
- 未覆盖真实IO fault与更多partial restore组合、macOS、完整应用/UI/角色及长期门槛；不关闭
  完整FILE Case或共享阶段。单agent只做一次小package定向构建，其余复用binary。

### Artifact 同对象/同长度错误字节发布（W174，基线 `4fc8ac8dd`）

- S-D03-60 / ART-001/002/005、FILE-038/039、A04/A05/A07/A13/A14/A17/A19：stage完成后或
  hardlink之后，把同对象的8字节改为8字节corrupt内容；原实现两项 **0/2**，仍返回成功/旧digest。
  证据根为外部 `2026-09-30/windows/w174-artifact-publication-bytes`，反例、错误返回及坏blob
  保留于 `01-first-product-run`，没有覆盖首次失败。
- 新发布在返回receipt/缓存前复用reader完整SHA-256与chunk index验证，继续核对原staged对象/
  大小。确认rollback后known rejection，其他对象或清理未确认仍unknown；原源文件不变。
- Windows模块 **18/18**、新反例 **20/20 × 2**、42份独立磁盘/目录项核对通过；WSL Ubuntu
  ext4模块 **21/21**、两原生场景独立核对通过，fmt/diff通过。Windows/LINUX结果不折算macOS。
  新发布增加一次full scan，计数如实；原pages/cache复用断言保持，未扩大输出或权限。
- staging/drop 名称复用、rollback check→unlink、并发增长预算、真实满盘/fsync、正式UI/模型/其他角色平台及
  N3/100 seed/LONG/99%仍未覆盖；不关闭完整ART/FILE Case或共享阶段。

### Artifact 源/blob并发增长读取预算（W175，基线 `003f12a78`）

- S-D03-61 / ART-002/004/005/006、A05/A07/A13/A14/A15/A17/A19：源暂存和完整blob验证
  都在metadata捕获后读取到EOF；8字节样本增长为131,080字节时，原实现拒绝前完整扫描新增数据。
  证据根为外部 `2026-09-30/windows/w175-artifact-growth-budget`，两项 **0/2**首败、磁盘和实际IO计数
  保留于 `01-first-product-run`。metadata/read的同步故障hook不引入新的等待点或模型权限。
- 两路径使用观察大小+1字节的真实reader上界，增长立即Conflict；不额外暂存/hash/cache，
  不返回截断内容。原变化后源/blob保留，publication temp清零；既有最大大小和正常完整性检查保持。
- Windows模块 **20/20**、新两项 **20/20 × 2**、42份独立磁盘/IO核对通过（131,080→9字节）；
  WSL Ubuntu ext4模块 **23/23**、两原生场景独立核对、fmt/diff通过。正常pages/cache断言未变，
  无UI/模型调用，未计macOS；全部日志、源码/binary身份和现场在外部。
- 连续多进程写、512MiB边界样本、staging/drop与rollback名称竞态、真实IO fault、完整应用/UI/
  角色及N3/100 seed/LONG/99%仍开放，不关闭完整Case或共享阶段。

### Artifact stage名称复用与明确cleanup unknown（W176，基线 `295686954`）

- S-D03-62 / ART-001/002/005/007、FILE-038/039、A04/A05/A07/A13/A14/A17/A19：Drop清理
  原stage名时误删已占用该名的外来对象（不同字节/同字节异inode），public staging failure又吞
  清理未确认。三个产品首败及坏现场留在外部 `2026-09-30/windows/w176-artifact-stage-cleanup`
  的 `01-first-product-run`、`02-first-public-product-run`，没有覆盖。
- stage保留identity到删除结束；Windows通过pinned Dir相对打开DELETE/metadata guard并拒绝
  delete sharing，核对后对原handle删除。publish与stage error显式cleanup，异常返回unknown原原因
  并保留原stage/外来对象，Drop只做同样的有身份清理并保留诊断。未增加模型权限或忽略失败。
- Windows模块 **24/24**、新四项 **20/20 × 4**、84份独立磁盘/对象核对；原生核对→delete的
  POSIX remap被32拒绝，guard释放后相同操作成功。WSL Ubuntu ext4模块 **26/26**、3原生场景
  核对及fmt/diff通过；未计macOS。原publishing/hash/growth/page/cache断言保持，完整日志/源码/
  binary和磁盘在外部，无模型/UI；脚本格式失败另留工具输出，不计产品FAIL。
- startup stale cleanup、Unix最终check→unlink、rollback名称窗口、真IO fault、完整UI/平台/角色
  及N3/100 seed/LONG/99%仍开放；不关闭完整Case或共享阶段。

### Artifact rollback最后核对→删除窗口（W177，基线 `532f15e63`）

- S-D03-63 / ART-001/002/005/007、FILE-038/039、A04/A05/A07/A13/A14/A17/A19：真实native
  identity后的POSIX remap把已发布路径置换成foreign对象，旧回滚误删foreign并报confirmed；首次
  **1/2**，preexisting foreign对照已通过。日志/实际误删现场保留于外部
  `2026-09-30/windows/w177-artifact-rollback-window/01-first-product-run`。
- Windows通过相对Dir打开DELETE/metadata且不share delete，持续保留原目标句柄到unlink；
  identity核对同一handle后原生删除，不再二次解析目标名称。最窄remap以32拒绝，guard释放后
  同一remap成功；preexisting foreign仍拒绝，missing/unknown判定保持。
- 模块 **26/26**、新两项 **20/20 × 2**、42份独立磁盘/目录项核对、fmt/diff通过；WSL Ubuntu
  ext4原模块 **26/26**兼容回归通过，未验证新Unix窗口/代判macOS。无模型/UI；完整现场与源码/
  binary身份均外部，生产修改仅Windows删除分支与保留原核对handle。
- startup stale cleanup、Unix stage/rollback check→unlink、更多IO fault、完整应用/UI/角色平台
  及N3/100 seed/LONG/99%仍开放；不关闭完整Case或共享阶段。

### Artifact 512MiB+1真实文件准入（W178，基线 `ad9836e19`）

- S-D03-64 / ART-005/006、A05/A13/A14/A15/A17/A19：NTFS sparse source/blob真实逻辑大小
  536,870,913字节，超过512MiB一字节；源publish以BadRequest、blob校验以Conflict拒绝，
  full-scan计数均0，对象/大小不变、无publication temp或false success，权限和断言未放宽。
- 首次 **2/2**，最终源独立三样本 **3×2/2**、六份外部磁盘/稀疏属性核对；Windows模块
  **28/28**、WSL Ubuntu ext4模块 **28/28**、两原生超限文件及IO核对、fmt/diff通过。
  原实现首次满足，无新产品FAIL或生产源码变更，只补边界回归；无模型/UI。现场、日志、源码/
  binary身份在外部 `2026-09-30/windows/w178-artifact-size-admission`，Linux不计macOS。
- 未覆盖恰好512MiB完整发布、持续增长、startup stale cleanup、Unix最终窗口、真IO fault、
  正式UI/平台/角色及N3/100 seed/LONG/99%；不关闭完整Case或共享阶段。

### Artifact 恰好 512MiB 完整发布与分页（W179，基线 `11c2627c8`）

- S-D03-65 / ART-001/004/006、A05/A13/A14/A15/A17/A19：真实 536,870,912 字节 source
  发布成功，512 页拼接 digest 与独立预计算值一致，EOF 为空，receipt/磁盘大小一致；分页
  full scan 不增加、page IO 恰好为文件大小、零 publication temp。原实现首次满足，无新
  产品 FAIL/生产修复；只补显式运行的 ignored 重型回归，未把跳过计作通过。
- Windows **3/3**（1,536 页），独立磁盘 hash **6/6**；相邻超限 **2/2**；WSL Ubuntu
  原生 ext4 **1/1**，源/blob 独立 hash **2/2**，fmt/diff 通过。完整日志、现场、源码/binary
  身份与计数在外部 `2026-09-30/windows/w179-artifact-exact-budget`，未调用模型/UI。
- 持续增长、startup stale cleanup、Unix 最终窗口、真实 IO fault、正式 UI/模型/角色/macOS
  及 N3/100 seed/LONG/99% 仍开放，不关闭完整 Case 或共享阶段；全程单 agent。

### Artifact 冷清理身份与既有 owner 对账（W180，基线 `95525a679`）

- S-D03-66 / ART-001/005/007、A05/A07/A13/A14/A17/A19：冷重开把已保留的 foreign stage
  按文件名删除并允许发布，首次 **0/2**；修复后又发现旧 Store 的 cleanup_complete 缓存绕过
  新 unknown，另两项首败保留。证据根为外部 `2026-09-30/windows/w180-artifact-cold-cleanup`，
  原失败在 `01-first-product-run`、`05-first-preexisting-owner`，没有覆盖。
- stage 持久化 original hardlink 与 native ID/birth 记录，重开核对同一对象后原生删除；
  无见证的旧 temp、外来 stage/见证保留并公开 unknown，新发布被拒，已有产物仍可诊断读。
  每次发布在共享 lease 内对账，64 项上限保持。三项旧夹具改用真实子进程无析构退出，
  正向回收、PID 名复用及 65→1→0 断言保留，并验证 temp 已删而 witness 遗留的恢复。
- Windows 模块 **33/33**，新五项 **3×5/5**；独立磁盘/native ID/birth 核对 **21/21**。
  WSL Ubuntu ext4 模块 **33/33**、五场景原生核对 **5/5**，fmt/diff通过；两项 ignored
  不计通过，退出夹具由父测试显式调用。辅助 oracle 两次长路径接口失败已留存，修正后通过。
- 见证创建/fsync中途断电、见证及目录同时伪造、Unix最终check→unlink、正式应用host fence/UI、
  macOS/其他角色及 N3/100 seed/LONG/99% 仍开放；不关闭完整 Case/共享阶段，无模型/UI。
  所有现场、日志及构建身份在外部，单 agent 只运行直接相关模块和复用 binary 的 N3 样本。

### Artifact cleanup unknown 的 canonical fence 与诊断读（W181，基线 `b6a0c78ea`）

- S-D03-67 / ART-007、LIFE-007、A05/A07/A13/A14/A17/A19：真实 owner 的无见证 temp
  经应用宿主返回 EFFECT_OUTCOME_UNKNOWN，唯一 canonical Effect 保留 pending、无终态。
  同 key/新 key、DB 真正关闭重开、磁盘残留修复、其他 Session 及文件写均拒绝，原 effect
  全记录不变；已有产物读回原字节，后来的源 blob/写目标均未创建，零额外 effect/人工 override。
- 首次 **1/1**，增加保留外来原对象的独立证据后 **1/1**；复用 binary 补 **3/3**，四份
  独立 SQL/磁盘/native ID 核对通过。相邻六项通过；误把两过滤词当 AND 而实际 OR，运行了
  79项并于93秒自然完成 **79/79**，保留日志、复用六项结果，无强停或重复测试。无新产品 FAIL。
- 仅最小宿主回归，未修改生产行为；fmt/diff通过。完整 DB、现场、日志及源码/binary身份在外部
  `2026-09-30/windows/w181-artifact-host-cleanup-fence`；两次构建分别144秒/20秒，N3复用既有binary。
- 正式 Tauri UI/Runtime/真实模型、其他角色、macOS及完整N3/100 seed/LONG/99%仍开放；
  不关闭完整 Case 或共享阶段；应用宿主组件结果不代替正式产品入口。

### 正式 Tauri cancelled 冷读与页面重连（W182，基线 `dbb786ebf`）

- S-D09-51 / LIFE-020、OBS-014、A05/A10/A13/A17/A19：复用 W165 原隔离 data/work/profile，
  保存完整 DB 基线后，以最新配对前端/正式 Tauri custom-protocol 在1280×832完成 **1次冷启动、
  1次页面重连**。三个 Turn 显示“已取消执行”，展开仍保留原文件完成回执，Session空闲，无结束/
  恢复按钮；原provider端口以健康受控监听器证明新增模型请求0。
- 独立只读 DB/文件核对：3 cancelled、3 returned、213 events及head全记录摘要不变，
  文件digest不变，零running/paused Turn、pending/unknown effect或新执行；没有重放/重复工具行。
  UI截图、accessibility、完整 DB、provider记录、构建身份及日志均在外部
  `2026-09-30/windows/w182-tauri-cancelled-cold-load`，原W165失败记录保留。
- 首次前端构建因锁定的plugin-fs未安装失败，冻结安装补4包后构建通过；包/锁文件及产品源码
  未变。辅助PS parser/日期类型护栏错误保留，修正后保持原15秒PID身份护栏；桌面边界/diff通过。
- 托盘窗口仍不可操作，quiescent GUI只做已核对PID/路径/创建时间/数据根的强制清理，provider
  经HTTP正常关闭；最终GUI/profile/provider均0。graceful quit、N3冷启动、真实模型、其他角色/
  macOS及LONG/99%未验，不关闭完整Case/共享阶段；只有简短进度入Git。

执行模式：本轮两代码通道存在上游依赖和共享 Cargo 缓存，重复编译/协调抵消并发收益。按用户要求，
现有子 agent 已完成当前批次、清理并交接，停止派发新工作；后续由单 agent 继续共享队列，只在确认
任务可独立交付且有实际收益时才并发。

下一步按最新实施计划核对并推进A/B/C命令主线。已有稳定的File/Store/process组件只在新风险或相关
变更时回归，不继续扩大业务/角色/平台矩阵；本轮不生成2,374槽索引，也不要求全产品发布统计。

### W184 只读命令首败、两根因修复与非零结果UI（2026-10-01）

- Case/子断言：C01/C03/C06/C08；CMD-134/137/138、REAL-003/004，正式Tauri通用入口、StepFun Coding Plan/step-3.7-flash。
- 首败：74f8f7e31当前宿主中git_diff path=.两次INVALID_PAYLOAD；原生Get-FileHash缺失但脚本继续并返回0。
  独立原生回归先红，根因是继承PowerShell 7模块路径。首败DB/截图/事件及04/05日志保留，不覆盖或改记PASS。
- 修复：Git根路径保留workspace/repo子目录边界；仅系统PowerShell 5.1重建默认模块环境；明确cwd相对路径合同。
  UI业务非零显示“命令已结束，退出码1”，原始error/exit/output仍可展开；参数检查显示未执行。
  恢复历史用“曾有…次尝试未成功”，完成披露保留累计数并说明包含参数检查/命令结果。
- 验证：UI 90/90、Rust 6项定向检查通过；typecheck、i18n、desktop边界、fmt及当前UI/Tauri构建通过。
  02-fixed综合A已实际显示诊断exit=1；随后因新失败通过正式停止入口取消，30模型步/6压缩，不记完整通过。
  新独立小会话只验git_diff path=.和原生Get-FileHash：3模型步/零工具错误/零压缩，最终完成；hash与独立磁盘值一致。
  两会话前后9文件hash一致、无unrelated哨兵、Git仅保留原tracked-note变更。正式GUI小复验为N1。
- 未覆盖：cmd.exe带中文/空格路径的引号、错误进程ID的准确归因、编码及综合A余项；GEN/COD N3、B/C仍待验。
  轮询现场为模型把返回ID中的f抄成e，尚未证明轮询能力缺失；当前可用性/未知效果错误提示需另查。
- 证据：仓库外2026-09-30/windows/w184-focused-command-a；源码同步至0c960b328后含本批改动，
  frontend d538b067-60cf-46a2-a383-04a2626c5dd0；宿主/patch身份、分会话DB、完整输出和机器oracle均在该目录。
  应用仅在已取消/完成且无活动Turn后按精确PID/启动时间清理；不声称正式应用优雅shutdown竞态已验。

### W185 原生命令参数与已完成历史（2026-10-01；基线c880a62d3）

- Case/子断言：C01/C05/C06/C07/C08；PROC-001/005/007、CMD-139、REAL-004/005。
- 首败与修复：合法type中文/空格路径因CRT转义返回1；仅系统cmd.exe的/c、/k单脚本文本改用原生引号。
  错误process_id此前返回CAPABILITY_UNAVAILABLE且暗示效果不明；改为未执行控制的类型化事实，保留原进程和计数。
  真实host检查还验证外属scope不可cancel、错误stdin未传入、正确输入后原进程退出0且reaped。
- 新首败：第二个真实GEN回合0模型步失败，原因是completion delivery历史校验漏掉失败披露；
  共用生成/校验文本，精确兼容旧版披露，缺失计数、改写计数/摘要及不完整历史仍拒绝。原失败回合不重写。
- 验证：native3、host1、policy9、history/turn定向检查通过；UI91、类型/i18n/desktop边界/fmt及正式UI/Tauri构建通过。
  GEN/COD各一个实际两调用样本通过；CMD exit0、43字节与磁盘一致；各一次引用拒绝仍保留is_error=true。
  原GEN会话在保留首败和重启后恢复响应，但只复述旧结果，不计重新执行样本。
- 未闭合：COD连续样本12模型步/3压缩、重复读/拒绝各4次，附带history组合预检2次失败；
  最终回答“未重试”与轨迹不符，不因terminal=completed或重复成功记N3。远端744ca440b须正常同步后定向复验。
  模型未验说明仍有技术术语；既有Native会话原位切角色不支持，现场另存，COD通过正式创建入口完成。
  完整A/B/C、完整交互/停止、更多命令形态及发布认证未关闭。
- 证据：仓库外2026-10-01/windows/w185-cmd-handle-contract，原生/host首红、完整DB/事件/截图/结果均保留。
  两次构建和宿主patch/binary身份在目录内；全回合terminal后按精确PID/启动时间清理，profile子进程0。

W185同步复验：062fc9f2e＋正常合并8ffc71ba5；没有覆盖远端或改写共享历史。
合并后正式原COD会话：3模型步/1压缩，CMD与负向poll各1次，无额外工具错误，无新helper；
三个实际执行样本（GEN 1＋COD 2）模型步2/2/3、压缩0/0/1，CMD均exit0且43字节与磁盘一致；
预期拒绝各1次、原is_error=true保留。仅复述历史的样本不计执行，失败/重复样本不覆盖或计PASS。
19项本批不同Rust回归及新增压缩回归通过；合并后3项相关检查、i18n/desktop边界、UI/Tauri构建通过。
六个正式回合最终4个实际执行、1个只复述、1个模型前失败，其中重复执行回合仍为验收FAIL；全终态且profile进程0。
完整A/B/C、交互停止、纠正跨压缩与其余基础命令仍待验；两修复点N3不替代整体共享门槛。

### W186 综合A：完整路径、隐藏属性与可解析Git diff（2026-10-01）

- Case/子断言：C01/C02/C03/C06/C08；CMD-134～138、REAL-003/004，正式Tauri/StepFun、原五项prompt不缩减。
- 首败：01-first GEN7模型步/1压缩，Get-Location对象表格把真实长cwd截为workspa...，最终回答漏掉项目目录；
  .hidden-note真实Archive却称隐藏。五项结果虽有完成终态仍FAIL。修复Windows指引为(Get-Location).Path和属性布尔JSON。
- 02-fixed GEN8模型步/1压缩：cwd精确到任务A repo，8项根目录含.git Hidden和dotfile非Hidden；文件43字节/4行/hash、
  查有1/查无0、Git状态变更及两个Bun测试exit0/1均正确，测试各1次、无unrelated哨兵、9文件hash不变。
- 新首败：owner Git patch多加Fdiff/H@@，最小解析回归红（no patch found）；修复回调内容行前缀，
  未暂存/已暂存有效patch及子目录隔离2项绿；当前正式COD调用的patch与独立Git输出字节一致。
- 定向检查：原生长路径/隐藏标记、工具暴露合同、Git两项，共4项Rust通过；fmt/当前正式构建通过。
- 未闭合：COD完整五项实际命令已有结果，但完成报告不断补查/校验、31模型步后报compaction无法容纳固定上下文及保留交换。
  首次报告missing/stale plan、closed plan预检、后续参数/历史调用及最终失败全部留存；本批不记完整A N3或GEN/COD整组PASS。
  下一批优先报告计划/引用与压缩预算；B/C仍未整组完成，不转旧全产品Windows队列。
- 证据：仓库外2026-10-01/windows/w186-full-command-a，01-first/02-fixed独立data/work/profile、完整日志/DB/截图及模型轨迹。
  实际模型始终复用加密StepFun配置，无原会话/明文凭据入Git；9文件不变、Git只保留原tracked-note变更。
  全终态后按精确PID/启动时间停止测试GUI，不声称正式shutdown竞态已验。

### W187 COD已结算非零与报告参数修正（2026-10-01）

- Case/子断言：C06/C07/C08；CTRL-006～008、REAL-004、A08/A17/A18/A19。继续原五项综合A，不缩减prompt。
- 首败与修复：最小回归在已reaped非零后报告仍被缺失计划拒绝；只为已证明结束、空可选计划提供直接报告路径，
  后续副作用门槛保留。首次真实修后又发现错误计数参数修正会清掉该来源，改为只使旧报告失效；原失败保留。
  已有恢复/未知效果/信号/清理及计数保护不放宽；最多8个证据ID的指引与实际Schema一致。
- 验证：7项不同Rust定向回归、fmt及两次正式构建通过。隔离Tauri/StepFun COD两轮completed，
  模型步9/7、压缩1/2；第二轮report_completion首次实际提交即接受，无报告/计划循环。
  每轮两个Bun测试各执行1次、退出0/1；9文件hash不变，无unrelated哨兵；两回合终态后profile进程0。
- 未闭合：第二轮先把cd && dir /a交给PowerShell 5.1而退出1，纠正后执行0；模型最终仍把Archive dotfile称隐藏项。
  第二轮累计工具失败11、命令失败2如实保留，未将预检或后续成功抵消历史。完整A及N3仍FAIL/待验，B/C仍开放。
  本批只关闭已结算结果的报告收尾根因，不代判正式shutdown、其他平台或发布认证。
- 同步：正常快进至86b0d2c40，保留远端Git格式、平台示例及macOS进度；同步后的相关回归另记，不移植旧正式运行身份。
  同步后报告/保护7项加平台示例1项共8/8通过；runner参数误放在Cargo选项侧的未执行记录另存，修正后定向运行。
- 证据：仓库外2026-10-01/windows/w187-completion-loop；01-settled-report-first-red.log及01-first完整DB、
  事件、模型轨迹、两轮UI截图、构建/patch身份、最终磁盘和清理结果。无凭据或完整日志入Git。

### W188 首次本机命令与GEN重复检查（2026-10-01；基线513e2d94b）

- Case/子断言：综合A的C01/C02/C03/C06/C07/C08；CMD-134～138/147、REAL-004，不缩减原五项任务。
- 调整：本机shell说明前置，cmd与cmd.exe给出合法JSON形态；实际Hidden/System标记与目录名称区分。
  没有自动改写模型命令、扩大权限或改变错误/成功判断。
- 验证：工具合同3项、真实PowerShell cwd/Hidden对照1项、fmt及正式Tauri构建通过。
  新COD Session6模型步/1压缩，完整文件43字节/4行/hash、查有1/查无0、Git CLI patch、8项目录属性及最终结果一致；
  两测试各一次exit0/1，14个机器断言全绿。report_completion首发计数错误后修正，原拒绝保留，不宣称零工具错误。
  正式过程区把业务非零显示为“命令已结束，退出码1”，参数拒绝显示为“操作未执行”，没有统称系统异常。
- 新首败：GEN初始五项正确，完成复核后反复补查；把项目名当cwd的三个调用及echo均未启动，之后读搜/Git和两测试重跑。
  派生压缩说明出现“新请求”，但没有新增用户任务；20模型步/3压缩后通过正式停止入口取消，首败轨迹和取消UI保留。
  两Session最终completed/cancelled，9文件hash不变、无unrelated哨兵；不记完整A N3或共享达标。
- 未闭合：复核/压缩后的任务身份和重复执行、首次报告参数、最终英文计数披露；B/C及其独立验收仍开放。
- 证据：仓库外2026-10-01/windows/w188-shell-guidance，01-first内01-cod-terminal/02-gen-stopped分别保存事件、
  模型轨迹和不可覆盖oracle，UI截图、独立磁盘/Git/属性结果及构建身份另存。完整日志和凭据不入Git。

### W189 GEN完成复核、压缩与重开计划（2026-10-01；基线a206c9ff0）

- Case/子断言：C06/C07/C08，CTRL-006～008、REAL-008及A01/A02/A05/A08/A17/A18/A19。
- 首败与修复：压缩丢掉普通复核消息；前置宿主提示仍出现重做。只指定报告工具仍允许模型重开计划、再检查。
  最终把已结算收尾阶段限制为已有报告控制，整批暴露检查先于dispatch；活进程控制、权限、失败计数及证据校验保留。
  无执行的拒绝只使报告失效，不改写已证明的门槛来源或重新暴露动作；新输入/真实owner观察及上下文改变撤销旧复核。
- 验证：11项不同Rust回归、fmt及正式构建通过；两个结构反例加重开计划反例修前红、修后绿，编译/runner中间失败另存。
  五个真实GEN Session独立留存：原任务5步/1压缩完成；只加提示29步/4压缩后取消；只选报告的普通答复8步/0压缩完成；
  仅文本答复19步/4压缩虽completed仍重做FAIL；最终相同仅文本任务13步/1压缩，1次复核后仅report_completion，未重跑。
  最终四exec、两search、Git status/diff各一次；两个Bun各一次exit0/1；全轮9文件hash不变，无unrelated哨兵。
- 未闭合：最终搜索/Git披露unverified；文件criterion引用目录枚举call ID，不能证明文件断言，仍记完成证据关联缺陷。
  最终oracle保持13/14，词面exit格式断言未通过，不改断言制造全绿；实际exit0/1另外核对。共享/完整A N3未达，B/C开放。
  最后追加的“未dispatch拒绝仍会重新暴露动作”反例首红已保留，持续边界修后通过；正式样本没有发生该额外拒绝。
  此批没有新COD、macOS、正式shutdown或发布认证；仅把重做子断言记GEN正式N1，不把其他运行移植为该修复N3。
- 证据：仓库外2026-10-01/windows/w189-completion-review-compaction；01-first内01～05分别保存原始事件、模型轨迹及oracle，
  多次构建/patch/宿主身份、终态DB、UI及独立磁盘/Git/属性结果均保留，完整日志/凭据不入Git。

### W190 调用范围、旧记录与编码首败（2026-10-01；基线b0ab39f19）

- Case/子断言：C02/C03/C06/C07/C08，CTRL-006/007，A05/A08/A17/A18/A19；复用原五项和仅文本收尾任务。
- 修复：账本增加有界原请求参数及owner文件hash/字节信息，旧记录明确是否尝试、是否返回成功及当前不具证据资格。
  env/stdin/文件与patch正文不进入新范围信息；大参数整体省略、窗口与旧记录总量有界。当前证据及精确计数校验不放宽。
- 验证：3个新回归加5个相关保护共8/8通过，fmt及正式构建通过；范围信息缺失的首红及编译中间失败另存。
  正式Tauri/加密StepFun GEN11模型步/2压缩、1报告首次接受；文件criterion对应实际Get-Content/Get-FileHash命令，未借目录ID。
  两个Bun各执行一次、exit0/1；Git原始status/diff与独立CLI一致，磁盘9文件hash不变，无unrelated哨兵。
- 新首败及未闭合：Get-Content默认解码把UTF-8中文变为乱码，最终回答原样误报内容；SHA/行数正确不代表内容正确。
  Select-String使用.\**\*及2>$null，未证明完整搜索且隐藏错误，不接受该0 exit为完整搜索通过。
  7条exec分别对应cwd/list/读/hash/两个搜索/两测试，没有重复执行；原oracle仍失败，不改只认read_file/search_files的断言制造通过。
  Git仍标unverified、语言仍含内部术语；本批只记录引用范围子断言GEN N1，完整A/N3与B/C、UI体验继续开放。
- 证据：仓库外2026-10-01/windows/w190-completion-operation-scopes，01-first独立data/work/profile及01-gen-terminal；
  编码与搜索首败、报告到原调用的映射、真实UI、原事件/模型轨迹、不可覆盖oracle及最终磁盘结果均保留，未将completed记整组PASS。

### W191 中文原文、无匹配与错误（2026-10-01；基线38c0a0df6）

- Case/子断言：C01/C02/C06/C08，CMD-135/136、FILE读搜，A05/A08/A15/A17/A19；原五项及仅文本收尾要求不缩减。
- 调整：说明PowerShell 5.1默认解码不能证明UTF-8文本正确；明确已知UTF-8原生读法，优先产品read_file/search_files。
  不吞stderr或把通配符范围当完整递归搜索；仍核对截断/未完整原因。W190真实首败继续保留。
- 验证：native1含UTF-8/CRLF字节精确、中文literal匹配、0匹配与文件缺失非零/错误保留；工具合同3项、fmt及正式构建通过。
  正式GEN8模型步/1压缩、1报告首次接受；原文43字节及SHA精确，搜索1/0匹配、truncated=false/incomplete_reasons=[]/files_skipped=0。
  三exec为合并cwd/list、两个PowerShell形式Bun：结果0/0/1，各测试一次；9文件hash不变，无unrelated哨兵。
  独立操作断言11/11通过；原oracle要求分开cwd/list及command/args仍失败，原结果不改，不把它当整组PASS。
- 未闭合：最终未复述原文，且读搜/Git标unverified并暴露eligible/ineligible字段及英文失败计数；当前状态证据与历史操作说明需优化。
  本批编码/完整搜索只记GEN正式N1，不代替COD/N3、A全体验、B/C、macOS或正式shutdown验收；共享门槛未达。
- 证据：仓库外2026-10-01/windows/w191-utf8-read-search，01-first独立data/work/profile、01-gen-terminal及independent-observations.json，
  原始DB/事件/模型轨迹/截图、构建身份、原oracle及补充操作核对分开保留；原生测试不改变文件，缺失错误未静默。

### W192 非零诊断及完成统计展示（2026-10-01；基线6f89612ed）

- Case/子断言：C06/C07/C08，REAL-004、CTRL-006/007、A08/A09/A17/A19；正式入口冷读和双诊断小任务。
- 修复：最终消息的Runtime英文统计在展示层本地化，原文/canonical计数保持不变。只有同回合原生已退出、
  无信号且清理已证明的非零结果能对应全部统计，才显示命令非零；混合/缺少证据保留次数与错误详情。
  用户文本、代码示例、被改写尾段和超范围数字不转换；不把真正基础设施/参数/清理错误改成正常业务结果。
  模型说明增加自然语言要求，不以available/ineligible字段诊断系统问题；Runtime仍自动交付历史累计计数。
- 验证：UI72、Rust计数/收尾3、类型/i18n/desktop边界及正式UI/Tauri构建通过；Shadow DOM展示核对通过，runner中间失败另存。
  使用W191隔离data/work、新W192 profile冷读旧回合；UI显示exit1、1/1统计，原事件逐字一致，不计新的实际执行样本。
  原正式GEN会话新双诊断任务completed，7模型步/3压缩；两次实际执行exit0/1、均reaped、9文件hash不变，无unrelated。
  模型额外提议2次检查均未dispatch；2次report计数参数拒绝保留；最终正文简短中文说明正常退出/清理，UI保留调用3/命令1。
- 未闭合：报告首发计数错误和多余提议仍待修；旧A消息中的当前证据/内部字段警告不重写，完整A N3及B/C未完成。
  本批只有GEN展示N1与真实原会话连续样本，无新COD/macOS/正式shutdown/发布认证；共享阶段未达标。
- 证据：仓库外2026-10-01/windows/w192-completion-outcome-ui，01-ui-replay保存旧DB快照/宿主/冷读图，02-live-result保存
  新原始事件、调用、DB、独立退出/清理/磁盘及旧事件比较结果；数据仍在独立W191夹具，新操作未触及原用户数据。
- 同步结算：源码6495729a6；正常合并远端5466de420为2f6c9442d，未覆盖远端。合并后证据上限与精确历史计数3项通过，
  fmt/diff通过；远端只改相关测试/进度，不重新编译无变化的产品行为，正式样本仍保留原构建身份；最终profile子进程0。

### W193 预期非零后的固定计数首发（2026-10-01；基线bee211a5d）

- Case/子断言：C03/C06/C07/C08，CTRL-006/007、REAL-004、A02/A08/A17/A19；相同双诊断任务，首败及历史不重写。
- 调整：在固定const上补enum/default及计数语义说明，不把“符合预期”转为计数0；输入缺字段/值错误仍拒绝，验证不填默认值。
- 验证：单值提示反例修前红；缺字段、错误值、抹零、证据上限及收尾保护共5项不同回归通过；fmt与正式构建通过。
  原GEN连续两回合及正式新COD，模型步4/3/5、压缩1/1/0，三个report_completion首次接受、无参数拒绝/报告重试，精确1/1。
  每个样本两个指定Bun均各一次exit0/1并reaped；旧canonical事件逐字不变，9文件hash不变，无unrelated。
  UI均显示已结束的非零结果及真实1/1；GEN两样本仅2条实际命令，未提出额外检查。
- 新范围失败：COD在指定测试前Get-ChildItem列root和tests，实际4条命令；独立“仅两条实际命令”检查FAIL，不放宽或删除反例。
  第二GEN说明仍带reaped=true等字段；语言要求不代替结果验证。计数点N3不等于双诊断整组/N3或完整A达标。
- 未覆盖：其他真实异常/多进程计数拓扑、更多模型/平台、完整A读搜/Git证据及B/C；仍留共享队列。
- 证据：仓库外2026-10-01/windows/w193-fixed-outcome-counters；01-live记录构建/profile/旧DB快照，02/03/04分别为GEN/GEN/COD，
  原事件、调用、完整模型轨迹、报告映射、不可覆盖检查、UI及磁盘/旧事件比较均保留；复用W191隔离data/work，新profile独立。
- 同步结算：源码f6d93ba00，正常合并远端4f13f7b2e为8980fdd19，保留固定前缀压缩余量实现及macOS记录。
  合并后单值计数/抹零/压缩收据/typed overflow/usage余量/硬限8项回归通过，fmt/diff通过；正式样本仍是合并前构建。
  最终profile子进程0，所有旧/新Turn终态；新合并压缩的真实Windows长任务另验，不将计数N3等同共享达标。

### W194 正式进程输入、总期限及UI停止（2026-10-01；基线978cd5ae9）

- Case/子断言：B/C05/C06/C08；PROC-027/033/036、REAL-005/010及A04/A05/A08/A10/A11/A13/A17/A19的相关子断言。
- 发现与修复：start_process未说明owner默认30秒总期限，GEN首发hold省略timeout_ms后超时。
  现说明默认30000、poll不续期、按已接受任务明确总期限及最大600000；超时区别于用户停止。
  timeout schema只新增default/description元数据，owner参数及权限/硬限/错误语义不变。
- 验证：既有native pipe stdin/EOF与5秒后代清理2/2、工具生命周期/宿主schema2/2，fmt/diff及正式Tauri构建通过。
  新隔离data/work/profile通过已有加密StepFun配置正式GEN：echo 7步/0压缩，精确6调用链，独立oracle 11/11；
  UTF-8中文+LF 16 bytes、hex、EOF、exit0/reaped、最终说明及原件hash一致，没有预读、listing或重复启动。
  后续UI停止前CIM独立确认父/子均存活和亲缘；约39秒时点击产品停止，canonical cancelled/host_cleanup_proven、两PID消失、心跳不再变化，echo原结果保留。
  停止独立oracle 15/16；a11文本缺截图上可见的“已取消执行”，机器UI断言FAIL保留，不能记录整组PASS。
- 首败：02-hold-timeout-first-fail保留默认30秒超时与完成复核阻塞；03-hold-timeout-before-stop-fail保留120秒过期后操作者才点击Stop。
  后者不是产品停止证明，不抹成修后成功。04-user-stop-terminal另记真实Stop；600000为现有硬限内显式操作窗口。
- 新缺口：04模型14步/1压缩，12次poll均省略cursor，重复READY输出；用户明确wait_ms=30000仍不足以避免忙轮询。
  取消卡片正确可见，但此前“进程仍在运行，等待点击停止”的文本继续显示；超时分支仍出现update_plan未暴露的自诊断及内部字段。
- 未覆盖：正式UI清理5秒时限、正确游标等待、取消文本一致性、timeout后计划/完成恢复、B文件步骤及N3、完整C/macOS。
  不以native通过替代正式UI时限，不关闭完整PROC/REAL或共享阶段。
- 证据：仓库外2026-10-01/windows/w194-process-stdio-stop；首败/修后分目录，完整events/tools/model/DB/UI、
  构建/源码身份、stdin原字节、磁盘hash和独立CIM/心跳核对均保留；Git只含提示源码及本页/共享简短进度。
- 同步结算：源码75a4f7127，正常合并远端03a508be6为1a1122104；保留他人的完成参数修复及显式未完成计划保护。
  合并后rejected_ 6/6与固定计数1/1通过，fmt/diff通过；上述正式UI仍是合并前构建，不代判新完成恢复路径。

### W195 取消前回复标注与轮询提示（2026-10-01；基线43cd16bee）

- Case/子断言：B/C05/C06/C07/C08，REAL-005/010及A08/A10/A13/A17/A19的对应子断言。
- 发现与修复：已取消回合仍显示最后的“进程运行中”文字。现同Turn取消且有回复时显示“下方是停止前尚未完成的回复”。
  原文字、已发生效果/错误保留；其他终态及无回复取消不加该提示。只关闭该UI子根因，不能证明进程结果由文字给出。
  poll工具说明next_cursor推进、wait有界、零游标重放会立即返回，不重复播报运行状态。
- 验证：UI首败01-ui-first-red.log保留；修后UI76/76、工具4/4，类型/i18n/桌面边界、fmt/diff及UI/Tauri构建通过。
  正式Tauri复用W194隔离data/work，新W195 profile：旧取消冷读和新取消均清楚显示历史回复提示；原事件digest和原件hash未改，16 bytes echo保留。
  当前helper只启动一次，Stop前父子存活/关系确认，Stop后canonical cancelled/host_cleanup_proven、两PID消失、心跳停止。
- 实跑首败：新Turn先引用旧process_id，owner拒绝且未执行；随后27次poll仍省略cursor，29模型步/四次压缩，整体独立oracle12/15。
  没有删失败、松断言或把“正确取消”算“等待任务成功”；停止后不再付费重复此失败。即时CIM仍存活与后续消失分开保存，不证明5秒时限。
- 定位补充：App装配用canonical schema覆盖展示层属性，单改standard_tools属性说明不会完整到达正式定义。
  本次还将timeout默认30000及cursor/wait提示补到process_schema；原接受集合不变，canonical3/3与admission subset1/1通过。
  此补充晚于正式binary，后续实跑另记；不能断言已证明旧引用/游标失败的唯一原因。
- 未覆盖：canonical提示补充后的正式等待、旧引用首发修复、完整B文件步骤/C/N3、timeout完成计划恢复、UI清理5秒及macOS。
- 证据：仓库外2026-10-01/windows/w195-poll-cursor-cancelled-reply；首败/修后日志、源码/UI/binary身份、旧DB基线、
  新回合完整事件/调用/模型进度、DB、截图、独立CIM/心跳和12/15 oracle均保留；旧W194首败未覆盖。

### W196 进程提示投影及旧合同恢复（2026-10-01；基线3ffffaaf6）

- Case/子断言：C05/C07/C08，PROC-024/025/030/033/036、REAL-005/010及A01/A02/A03/A05/A08/A10/A13/A17/A19的相关断言。
- 发现与修复：上一批修改canonical说明改变了注册贡献指纹；01-live正式旧Session在模型/工具前拒绝provenance drift，0模型步/0调用。
  首败02-schema-current-first-result保留。现process_schema与43cd16bee逐文件完全相同；只把三个过程参数的description投影到App模型定义。
  投影不复制default/范围/required/新字段，注册/Snapshot/贡献锁及owner校验照常，不通过扩权或忽略漂移恢复。
- 验证：展示说明丢失断言04先红后绿，新App2/2、Runtime admission subset1/1，fmt/diff通过；正式Tauri构建通过。
  02-live-presentation复用W194隔离data/work、新profile；同一旧Session通过正式“重试”回填/发送原要求恢复准入，旧失败Turn仍为failed，新Turn独立cancelled。
  实际start一次，poll0读READY_PARENT/CHILD，后续都cursor25/wait30000且无重复READY；7模型步/一次压缩/6次poll，零工具结果错误。
  最后一次poll未结算由用户Stop中断，未把缺结果补成成功。Stop前CIM确认父子存活/亲缘；点击起1,208.76ms内两PID消失，心跳随后不变。
  canonical cancelled/host_cleanup_proven、无取消后新启动、原echo16 bytes、原件hash、旧事件digest及取消前回复标注均通过；独立oracle21/21，仅GEN N1。
- 未覆盖：N3/COD及其他旧冻结合同、完整B文件步骤/C、timeout完成恢复；等待仍重复播报/暴露游标。
  原准入失败UI写“上游Agent或模型服务商出错”，实际是本地合同漂移；此归因另修，不把业务非零或模型问题混记本地故障。
- 证据：仓库外2026-10-01/windows/w196-owned-poll-context；01-live拒绝原始事件/截图、04断言首败、02-live-presentation修后模型轨迹/事件/调用/DB、
  时间戳/独立CIM/心跳/磁盘/旧事件核对及构建/源码身份分别保存。03测试夹具编译错误另保留，不计产品反例。
- 同步结算：源码0583086f9，正常合并远端a14b09cbb为9c84c36d4；远端只补macOS/共享进度，源码未变，不重复构建或模型调用。

### W197 本地配置拒绝的错误归因（2026-10-01；基线4cd83a3aa）

- Case/子断言：C06/C07，OBS-005与REAL错误归因及A08/A17/A19的相关断言；明确区分本地准入与供应商失败，不更改业务非零结算。
- 发现与修复：W196真实provenance drift被Conflict字符串分类成未知上游；派发失败又丢失结构化错误。
  新Capability/Skill漂移以本地类型返回HTTP409和NOMIFUN_SESSION_CONFIGURATION_CHANGED，nomifun归属、非重试、新会话建议；持久化保留原错误结构。
  不恢复原生owner时保留任务未完成/核对语义；网关补齐新错误类型的穷举映射，不放过准入检查或改权限。
  旧UNKNOWN_UPSTREAM只对精确本地漂移签名投影新标题，原状态/详情/身份/canonical文本不写回；provider诊断不能冒充新的本地类型。
- 验证：01 UI说明反例先红后绿，分类43/43、App类型/冷读2/2、UI21/21（含新HTTP平铺/嵌套错误的一致恢复建议），类型/i18n/桌面边界、fmt/diff及正式UI/Tauri构建通过。
  原Session隔离data/work、新W197 profile正式冷读：标题“会话配置已变更”，展开显示应用归属、恢复建议/新码与原始provenance诊断，无原样重试。
  独立oracle15/15：旧事件集合及digest逐字一致，原失败仍failed/旧码仍UNKNOWN_UPSTREAM_ERROR，原件hash与echo16 bytes保留，全Turn终态，新增模型/执行事件0。
- 首次/修后分开：原UI错误在W196保留，01-ui-first-red及02分类探针保留；最终分类改由Kernel类型承载，供应商同名文本负例另验。
  04/09的测试查询名称/组合详情匹配错误及08网关新变体编译缺口另保留，10/13/17记录修后，不把这些夹具/编译错误算新增产品失败。
- 未覆盖：新分类拒绝的正式端到端/N3、其他Kernel/权限/资源拒绝、provider故障正式UI/macOS、完整A/B/C及等待重复播报。
  本次关闭已知旧本地漂移误归上游的UI子根因；冷读不能增加新的执行样本，不关闭共享阶段。
- 证据：仓库外2026-10-01/windows/w197-local-admission-error；01-live含旧DB/event基线、正式摘要/详情截图、源码/构建身份与15/15核对，
  原失败/修后日志分文件，完整DB与事件留外部；Git只含必要分类/投影源码、最小回归与进度。
- 同步结算：源码7dfff1060，正常合并远端df49ded29为4f6cd455a；保留历史结果交付修复及Canvas并发工作。
  合并后历史结果定向1/1、本批UI21/21、类型/i18n/桌面边界和fmt/diff通过；Canvas正式业务验收仍由其工作负责。
  本批正式冷读是合并前构建；最终HTTP平铺处理及供应商同名诊断隔离分别由21 UI/43分类检查覆盖，不把冷读移植为新拒绝的端到端证明。

### W198 首次标题恢复与正式综合A（2026-10-01；基线45c874db1）

- Case/子断言：C01/C02/C03/C06/C07/C08，CMD-131/135/136/137/138、REAL-003/004及A05/A08/A17/A19的相关断言。
- 发现与修复：新隔离主页把完整长消息当Session标题；末尾换行使创建边缘空白校验HTTP400，且标题更新有200 UTF-8 bytes上限。
  首次拒绝0 Session/0模型/0工具保留；一次零执行复现也保留。只派生trim首行及完整Unicode字符的200 byte有界标题，原input不截断。
  不放宽后台合同/断言，不改权限、资源、模型预算或用户任务；仅关闭标题派生根因。
- 验证：05标题最小反例780 bytes首败保留，修后标题/发送链30/30，类型/桌面边界、UI/正式Tauri构建通过。
  新data/work/profile、已有加密StepFun配置，只复制provider/model四表，未带旧Session。临时Git含10个文件、真实Hidden与非Hidden点文件、既有tracked-note修改。
  修后正式GEN完整五项：9步/一次实际压缩，4 exec（cwd/list/两Bun）、样本完整读一次、两个目标搜索各一次、status/diff各一次；
  源中文43 bytes/4行/SHA与receipt一致，Git patch与CLI逐byte一致，两Bun依序exit0/1且reaped，原件/Git状态保留，unrelated未跑，五项最终事实已交付；独立22/22。
- 首发报告仍失败：首次report_completion带不再可用的file/search/Git当前引用，被schema拒绝；后续用历史结果说明及unverified/no evidence修正，未重做命令。
  整体A门槛FAIL、只记操作事实与标题恢复GEN N1；未把一次修后接受计报告首发/N3通过。
- 其他结果：新任务标题有效；初始消息待发缓存单测原文逐字保留。正式编辑器与夹具末尾换行出现LF(10)→CR(13)，严格原文比较FAIL保留；
  528字符内容只有末尾换行不同，正文无遗漏的附加核对另存，不覆盖严格断言。首段默认读AGENTS回执、英文earlier observation/资格警告及参数拒绝计数仍需复核。
- 未覆盖：报告首发/N3/COD、上述语言/计数/末尾换行归因、完整B/C/macOS；普通系统调用本批未先试错，但不能代判整个共享阶段。
- 证据：仓库外2026-10-01/windows/w198-comprehensive-a；creation-first-fail/API/04截图、05首红、14修后完整事件/模型/调用/DB/UI、
  样本/Hidden/Git/原件/22断言及严格/附加输入核对分别保留。07/09的测试Recorder命名错误另留，10记录修后，不计产品反例。
- 同步结算：源码597e16066，正常合并远端929dae3bb为9ebb679ad；保留文件/stdin精确换行的展示提示及宿主反馈。
  合并后展示3/3、换行说明1/1、标题/完整消息30/30、桌面边界和fmt/diff通过；上述正式综合任务仍是合并前构建，未代判新换行提示的live效果。

### W199 报告拒绝计数及修正反馈（2026-10-01；基线2d368080c）

- Case/子断言：C06/C07/C08，CTRL-006/007、REAL-004及A08/A17/A19；正常业务非零保留，系统拒绝如实计数。
- 发现与修复：内部控制参数拒绝未进入失败总数；现从配对结果统一记一次，原未暴露控制分支移除重复计数。
  参数未执行不增加命令失败数、工作区版本或owner调用，不重开已结算任务。最小反例01先红后绿。
  首轮正式实跑发现旧schema的expected计数每次落后新拒绝一次，四次报告拒绝后failed；完整首败04保留。
  现反馈另给拒绝整批之后的宿主计数，指明旧issues的时点；严格const、证据资格、整批拒绝及控制上限均不变。
- 验证：反馈最小反例05先红后绿；14项不同Runtime定向回归通过，fmt/diff及正式Tauri构建通过。
  同隔离data/work、新profile正式GEN/StepFun：13步/两次压缩，cwd/list、Git status/diff及两个Bun各一次；
  退出依序0/1且reaped，无命令重放。首次报告因旧读搜引用拒绝，反馈2/1，第二次报告及canonical/UI均精确2/1。
  十文件hash、Hidden、中文43 bytes/4行/整文件SHA、两个搜索、Git输出逐byte、原事件digest、旧failed均保持，独立25/25。
  首次oracle用系统默认编码读取UTF-8基线，中文路径误解导致24/25；原文件保留，显式UTF-8的独立核对另存，断言未放宽。
- 未覆盖：完整A报告首发/N3/COD、内部call ID警告及过程语言、B文件步骤/N3、C纠正恢复/macOS。
  本次只关闭计数遗漏及计数修正滞后的GEN N1子根因；第一次报告拒绝仍为FAIL，不关闭共享阶段。
- 证据：仓库外2026-10-01/windows/w199-rejected-control-accounting；01/05首红、01-live/04失败及
  02-feedback-live/10修后完整模型/事件/调用/DB/UI和严格/编码核对分开保留；Git只含源码、最小回归及进度。
- 同步结算：源码0bec4031c，正常合并远端d8423c219为ab1b32a24；保留macOS启动前程序校验及历史首败。
  合并后计数/修正配对回归1/1、fmt/diff通过；正式GEN保留合并前构建身份，不代判Unix或完整场景。

### W200 业务非零与参数拒绝的分类说明（2026-10-01；基线01330bfba）

- Case/子断言：C06，REAL-004、A08/A17/A19；减少把正常业务非零当作NomiFun系统故障的展示误解。
- 发现与修复：同回合有普通非零和参数拒绝时，最终尾段只有合并统计。现仅按同Conversation/Turn的真实回执，
  完整对账后分别说明“命令已结束/退出码”和“参数检查未通过/未执行”；纯参数拒绝也明确未执行。
  远端、信号、未证明清理、其他实际故障及无法匹配命令计数仍保留原统计/错误，不从模型文字推断来源。
- 首败保留：01最小反例先红后绿；正式首次冷读11/14，内部report控制没有历史tool projection，无法形成完整分类。
  11补充反例先红后绿；现只说明有证据的普通非零，其余结果明确“类别尚未确认”，不猜参数故障、供应商或应用归属。
  原canonical文本/计数、错误、用户正文不改，不将历史失败抹成成功。
- 验证：最终相关UI40/40（含中英文真实组件及缺记录/跨Turn/远端/信号/清理负例），类型/i18n/桌面边界、diff及UI构建通过。
  tauri/custom-protocol正式构建，复用隔离data/work、新profile冷读W199：明确exit1、另一条类别未确认、总数2，独立14/14。
  原事件集合/digest、首次failed、三Turn终态、十文件hash、Git status/diff均保持，新增模型及执行事件0。
- 未覆盖：内部控制的完整历史详情、call ID/英文过程文字、完整A首发/N3、B/C及macOS。
  本批只关闭结果分类说明子根因；冷读不是新模型样本，不关闭共享阶段。
- 证据：仓库外2026-10-01/windows/w200-command-and-argument-outcomes；01/11首红、01-cold-read首败和
  02-cold-read-fixed修后DB/UI/独立核对/构建身份分开保留，Git只含展示源码、最小回归及进度。
- 同步结算：源码7342ebb43，正常合并远端9aab28584为184742201；保留通用原生命令验收夹具和macOS记录。
  本批UI源码hash与正式冷读构建完全一致，diff通过；远端仅夹具/进度，未重复模型或无关套件。

### W201 未派发调用的冷读详情（2026-10-01；基线ada7c932e）

- Case/子断言：C06/C07，CTRL-007、A08/A09/A17/A19；参数预检拒绝及内部控制结果在冷读中可追溯。
- 发现与修复：历史只显示owner派发投影，参数拒绝没有owner派发，因而缺卡片/详情。01最小反例先红后绿；W200正式缺分类首败保留。
  现从同Session/Turn/模型步的已配对Runtime调用与结果派生只读历史项，排除未结算提议、跨步结果、指令内部调用及已有owner投影。
  结果保留真实成功/拒绝；历史ID按完整身份稳定派生，纳入分页、总数及按ID读取，原权限/执行身份不变，不写回旧事件或投影。
- 验证：App历史5/5、Session历史/身份/既有准入4/4，fmt/diff及tauri/custom-protocol正式构建通过；前端沿用W200未变构建。
  原隔离data/work、新profile正式冷读W199：两条report结果补回，拒绝卡片显示参数未通过/未执行；可展开原参数及完整拒绝输出。
  最终区分命令exit1与参数拒绝，总数2/1，独立19/19；原事件及投影逐字、首次failed、三Turn终态、十文件/Git状态不变，新增模型/执行事件0。
  page_size=1无漏项/重复、owner卡片不重复、跨Session详情不可取、跨步骤结果不配对、重复读取ID一致均通过。
- 未覆盖：内部工具名/英文过程及call ID警告、长期历史读取性能、其他历史协议/完整A首发/N3、B/C及macOS。
  本批只关闭拒绝/控制结果的冷读详情缺失，不由冷读增加执行样本或关闭共享阶段。
- 证据：仓库外2026-10-01/windows/w201-rejected-tool-cold-history；01首红、04/05修后、01-cold-read DB/UI/原始详情/19断言及构建身份分别保留。
- 同步结算：源码ae19fdc14，正常合并远端e6cc5c7c0为db6758da1；远端仅macOS进度，本批源码hash与正式冷读构建一致，diff通过。

### W202 正式Coding文件链首败（2026-10-01；基线ca03343c9，仍失败待修）

- Case/子断言：C01/C04/C06/C07/C08，CMD-140/141、REAL-001/002/006及A05/A08/A17/A19。
- 首次正式Coding/StepFun：23步/5压缩，模型给路径加项目名前缀，实际产物位于额外嵌套目录；移动exit1后自行建目录重试。
  原件/相似名保留，18 bytes/3行/LF/SHA在错误路径正确，但最终宣称用户要求的路径已完成；严格oracle6/14，FAIL。
  额外搜索、空操作patch、补目录和重试不抹掉；完成报告首次接受不等于任务已完成。
- 调整：标准工具说明补充“已选工作区就是项目目录、默认cwd同根、保留用户相对路径、不附加项目显示名”。
  04最小反例先红后绿；05初版破坏宿主说明前缀的回归首败另留，06保持原前缀后14/14通过，fmt/diff及正式构建通过。
  仅修改模型说明，canonical Schema、权限、owner及参数执行方式不改；目前没有真实修后成功证据，不称根因已闭环。
- 新目录/新profile、完全相同任务的第二轮仍失败：项目名前缀、错误来源SHA、把Copy-Item当可执行程序、PowerShell/cmd引号试错及重复探测。
  31步/12压缩、12个exec提议后通过正式Stop结束为cancelled；11个已退出命令均reaped/无清理错误，原件/相似名及旧事件保持。
  严格oracle5/14，FAIL，未交付最终报告；停止前文字已标注未完成。08文件名虽带terminal，实际为running快照，09才是cancelled终态。
- 未覆盖：工作区说明实际接合/有效性、Cmdlet与字面argv形态、patch来源摘要/压缩、重复操作及完成语义，完整B/N3/GEN/C/macOS。
  已停止继续付费重试；下一批从实际模型定义和上述真实反例定位，不加预算、补目录、松路径/次数断言或删除首败。
- 证据：仓库外2026-10-01/windows/w202-files-step-chain；01-first与02-fixed的原任务、磁盘/严格oracle、事件/模型/调用/DB/UI及构建身份分别保留。
- 同步结算：源码1695b3277，正常合并远端b530d751f为a435abe35；远端仅进度，本批源码hash与正式构建一致，两个FAIL及覆盖限制保持。

### W203 参数说明的正式装配投影（2026-10-01；基线0d423eaf7）

- Case/子断言：C01/C04/C08，MODEL工具定义、CMD-140、FILE-025及A01/A02/A19；沿W202真实反例检查接合，不再盲跑模型。
- 发现与修复：顶层description在OpenAI编码器原样保留；App换入canonical Schema时却只投影少数参数说明，cmd/command/args/cwd及嵌套patch说明未到参数位置。
  01最小装配反例在exec_command/cmd处先红后绿。现固定白名单JSON pointer只复制description到已存在字段；包含路径、来源SHA及hunk说明。
  标准参数说明补充Cmdlet使用cmd脚本、LiteralPath/引号/错误保留、准确相对路径及完整SHA逐字复制；不改参数、Schema约束、owner或权限。
- 验证：App装配10/10、Runtime工具合同14/14、fmt/diff通过；去说明后结构与原canonical逐项完全相同，原注册Schema再读取仍相同。
  恶意默认/模式/required/上限及不存在的authority字段不能混入，其他Module不投影；旧换行回归恢复新增路径description后保持完整结构断言。
  02的旧回归说明恢复列表缺path所致首败另留，03修后通过，不放宽字节/字段结构断言。本批未构建新Tauri或调用真实模型。
- 未覆盖：新参数定义的正式模型效果、W202前缀/SHA/Cmdlet/重复及完成语义、完整B/N3/GEN/C/macOS。
  仅关闭已证实的参数说明投影丢失；不声称它是W202所有失败的唯一根因，不用24项组件检查代替正式任务通过。
- 证据：仓库外2026-10-01/windows/w203-model-parameter-guidance；01/02首败及03/04修后日志分开，W202两轮FAIL和原件不变。
- 同步结算：源码ea8e85a99，正常合并远端1095a185e为3b1b06c3c；远端仅进度，参数投影源码未变，不重复构建或模型请求。

### W204 参数投影后的正式文件效果（2026-10-01；基线1c901a175，整体仍FAIL）

- Case/子断言：C01/C04/C06/C07/C08，CMD-140/141、REAL-001/002/006及A05/A08/A17/A19。
- 正式tauri/custom-protocol构建、新目录/新profile、原样任务及已有加密StepFun/step-3.7-flash，Coding 14步/两次压缩后completed。
  创建/完整读取/一次来源保护patch/复制移动/限定删除/最终读取实际正确：没有给效果路径添加项目名前缀，完整SHA逐字传递。
  Copy-Item/Move-Item使用cmd脚本、LiteralPath、引号与ErrorAction Stop，合并一次exec，Remove-Item一次；三条已执行命令exit0/reaped/无清理错误。
  最终正确路径18 bytes/3行/UTF-8无BOM/LF及末尾LF、全文SHA、源/副本消失、原件/相似名/AGENTS及旧事件保持，文件效果仅Coding N1。
- 首次范围失败保留：先执行了未要求的Get-ChildItem；收尾又提议read_file/列目录，被当前完成复核表面拒绝，两项未dispatch结果都计入2/0。
  完成报告首次接受，但summary宣称“未执行额外操作”与已执行listing不符；内部available_evidence警告仍出现，整体不能记PASS。
  沿用原14个断言并增加禁止listing、操作次数、进程结算及无假报断言，严格15/18，FAIL；不覆盖W202两轮6/14与5/14。
- 未覆盖：明确范围/禁止额外检查的遵守及交付真实性、完整B/N3/GEN、进程综合和C/macOS。
  本批只新增W203提示后文件/命令效果首次正确的正式N1证据，不称所有模型行为根因已修复，不关闭共享阶段。
- 证据：仓库外2026-10-01/windows/w204-parameter-guidance-live；03中途及04 completed完整events/tools/model/DB/UI、源码/binary身份和18断言分开保存。

### W205 明确范围与收尾补查指引（2026-10-01；基线4527e45cf）

- Case/子断言：C06/C07/C08，REAL连续/交付、CTRL-006/007及A08/A17/A19；沿W204额外listing和不实“无额外操作”首败收敛。
- 调整：最小执行政策明确用户路径/顺序/禁止项包含只读探测；工具例子仅示范输入，不能变成任务。
  最终检查改为用户范围内才做，不为刷新报告证据重读；收尾明确action工具关闭时只提交已有report，不另探读/列表。
  summary说明要求按已记录调用披露范围偏离，包括未执行提议，不无依据宣称无额外操作。
  不变更权限、工具表/选择/拒绝、Schema接受集合、计数/证据及派发；这是说明改进，尚未证明真实模型行为闭环。
- 验证：实际压缩回归1/1、工具合同14/14、拒绝/计数7/7、关闭复核零派发1/1，共23项不同检查，fmt/diff通过。
  压缩前后的范围/报告阶段指令逐字保留，原输入仅一次、工具表不变，关闭/新观察清除阶段正常。
  01/02测试夹具误加到另一段50k输入，目标夹具未加载最小政策，失败另留；03修正夹具通过，不算新增产品根因或放宽压缩断言。
- 未覆盖：正式Tauri/StepFun范围遵守与无假报、完整B/N3/GEN/C/macOS；W204整体15/18及历史FAIL仍保持。
  本批无新模型调用、不重复构建；下一正式样本再验证指导效果，不用23项组件结果代判整体通过。
- 证据：仓库外2026-10-01/windows/w205-explicit-scope-guidance；01/02夹具失败及03～06修后定向日志分开保存。
- 同步结算：源码cbdaa5783，推送遇远端更新后正常合并a860eb019为162da2daf；远端仅进度，本批代码未变，未重复测试或模型请求。

### W206 健康链报告拒绝后的重复写入（2026-10-01；基线42f63a358，正式样本FAIL）

- Case/子断言：C04/C06/C07/C08，CTRL-006/007、REAL连续/交付及A06/A08/A17/A19。
- 正式Coding/StepFun原样文件任务：初始路径/SHA/一次patch/复制移动/删除/最终读正确，没有listing，但有未要求的cwd探测。
  首次report参数因旧证据ID拒绝后重新write源文件两次，已删除文件复活；16步/6压缩后completed，summary又把来源文件的值=1误说成最终文件内容。
  正确最终文件仍为值=2/18 bytes/3行/目标SHA；原件与旧事件保持，严格12/18仍FAIL。旧脚本计数断言只认Remove-Item，不识别本轮合法delete_path，原oracle另留，不因此放宽整体结果。
- 根因与修复：旧终态参数修正保护依赖settled_failure_gate，健康多步链无先前失败命令时，参数拒绝没有进入报告阶段，动作表继续暴露。
  现只有无显式计划、有已成功观察的命令/修改、单个内部report参数拒绝且无活进程/未决patch时进入现有报告复核。
  严格参数/证据/计数及派发拒绝不变；不把拒绝当命令失败，不重开原效果，不改权限。显式未完成计划的合法修复保留。
- 验证：11有效多步修前反例额外write确实dispatch，12修后仅原创建/命令dispatch；拒绝结果计2/0，报告修正可完成。
  新回归1/1、已有拒绝/恢复7/7、实际压缩1/1，共9项，fmt/diff通过；06～09单命令夹具未暴露report的误匹配另留，10改多步后验证，未算新产品根因。
- 未覆盖：此新防重开分支的正式模型/N3、无命令的纯读取任务、cwd范围遵守及摘要/交付一致性，完整B/GEN/C/macOS。
  修复晚于本次正式binary，未移植样本为修后PASS；运行已终态，没有为停止终态任务发送Stop，没有再付费重试。
- 证据：仓库外2026-10-01/windows/w206-explicit-scope-live；03中途、04首次重复、05 completed及DB/UI/严格oracle，06～14原生回归/诊断分文件保存。

### W207 已准入工作区数据与供应商暂停（2026-10-01；基线5c9d9d9fa，模型效果阻断）

- Case/子断言：C01/C06/C07/C08，工作区默认cwd、MODEL接合及A08/A17/A19；沿W206未要求的pwd和报告重开继续验证。
- 发现与修复：正式prepare_turn向模型加入权限/能力上下文，却没有直接给已准入workspace根信息。
  现从EngineTurnReceipt的已验证Session workspace提供JSON数据（root、默认相对cwd、宿主OS）；路径值保持数据、正确转义，不做文件探测或扩大权限。
  保留用户明确要求pwd/listing的场景；这是已知环境数据，不替代用户结果验证，不改用户任务/Schema/owner。
- 验证：数据编码1/1、正式宿主prepare_turn接合1/1，后者上下文仅一次/模型请求0/效果0，fmt/diff及正式tauri/custom-protocol构建通过。
  新目录/profile、原任务正式Coding/StepFun仅进入首个模型步，因EXECUTION_MODEL_PROVIDER_UNAVAILABLE暂停，无任务工具调用或效果。
  UI明确“模型服务暂时不可用，任务尚未完成”，host_cleanup_proven、原件/旧事件不变、无假交付等独立9/9；不能计任务PASS或修后有效性样本。
- 状态核对：初始外部脚本只读agent_turns.state=running，未反映turn_paused/head.paused；后以最新事件/Head及UI确认暂停，未重启或重发。
  06暂停DB/UI已保留，再通过正式“结束本回合”清理隔离测试为cancelled，07终态另存；这不是暂停整个目标。
- 未覆盖：已知root减少探测的模型效果、W206新防重开分支的真实/N3、完整B/GEN/C/macOS，首发引用与交付一致性。
  模型不可用只阻断这次正式Case，2项组件与9项暂停核对分开记录，不扩大预算或不断付费试探。
- 证据：仓库外2026-10-01/windows/w207-admitted-workspace-context；01/02组件、03构建、05中途/06暂停/07取消、事件/DB/UI与独立核对均在外部。
- 同步结算：源码381c17fd3，正常合并远端f1f0a8dd8为df596caf9；保留已确认缺失文件上下文及macOS进度。
  合并后缺失状态/否定与健康报告修正2项通过，fmt/diff通过；本轮正式binary为同步前，供应商阻断及模型效果未验状态保持。

### W208 命令未启动的准确展示（2026-10-01；基线335e33c6b）

- Case/子断言：C06、A08/A09/A17/A19；冷读W202真实Copy-Item启动失败，沿用户澄清优化错误分类，不要求业务命令永远成功。
- 修复：仅本地exec_command/start_process完整PROCESS_NOT_STARTED宿主记录、user_code_started=false且无进程/退出/信号矛盾时显示“命令未启动”。
  详情明确尚未执行，保留status=error、真实失败状态及原输入/输出；远端、信号、清理和不完整记录不获此分类，不臆测故障来源。
  继续执行后的折叠摘要原先被通用“曾有尝试未成功”覆盖，现保留准确启动状态，其他失败和重试历史仍可展开。
- 验证：UI定向98/98及相关MessageList结构18/18，类型、i18n、desktop边界、diff检查与正式custom-protocol构建通过。
  正式Tauri、新profile冷读原cancelled会话，独立15/15：摘要/详情/原始诊断/业务exit1区分、8回合终态、事件/消息/效果及所有文件hash不变；新模型/工具事件0。
  首红1/30、首个冷读14/15（摘要遗漏）分开保留；构建因未停占用exe失败，核验闲置实例身份后关闭再构建，未改产品权限或放宽断言。
- 未覆盖：没有新增真实模型/N3；W202文件链FAIL、W206交付/防重开正式验证、完整A/B/C及共享门槛继续未闭合；供应商不可用只阻断相关模型样本。
- 证据：仓库外2026-10-01/windows/w208-process-not-started-ui；日志、原始DB/事件/磁盘基线、02摘要首败与03修后UI/oracle独立保存。

### W209 正式文件链三样本（2026-10-01；基线26b900c4e）

- Case/子断言：C01/C04/C06/C07/C08，CMD-140/141、REAL-001/002/006及A05/A06/A08/A09/A17/A19；只关闭文件链效果与交付N3子项。
- 验证：正式Tauri、原完整任务、已有StepFun Coding Plan/step-3.7-flash，新workspace/profile三例COD/GEN/COD完成，7/8/8模型步、1/2/2实际压缩。
  每例创建/源回读/整SHA保护patch/本机复制移动/限定删除/最终回读各一次；没有项目名前缀、cwd/listing/额外读取/测试/重建。
  原18项严格oracle各18/18，增加初始参数字节/来源SHA/精确读次数/顺序/调用结果配对/报告计数/UI交付后各31/31。
  原件/相似名/AGENTS、旧事件保持，最终文件18 bytes/3行/LF含末尾LF及整SHA一致，源/副本消失；命令exit0/reaped，最终中文值=2与实际磁盘一致。
  报告均首次接受，工具/命令失败计数0/0、无完成后的新效果；三个原失败现场及W207供应商暂停保持。
- 本批无产品修改，复用W208正式binary（D0132243…FB2B0）与对应当前源码，不重复编译或cargo套件；模型服务已恢复实际响应。
- 未覆盖：没有触发W206报告拒绝后的修正分支，不把三次健康收尾当该分支端到端证明；纯读任务、新拒绝、进程/超时/N3、完整A/C及共享门槛仍缺。
  UI第三例把同一路径的write/patch概括为“已编辑2个文件”，实际是两次操作；摘要计数文案另待修，不影响已核对的磁盘/调用次数事实，不宣称完整UI/B通过。
- 证据：仓库外2026-10-01/windows/w209-files-no-replay-live；01/02/03独立夹具、原参数/结果、事件/DB/模型轨迹、UI与两套oracle保留，Git仅短进度。
- 同步结算：证据提交a736283d0，正常合并远端c4456fa41为03e164aad，保留报告Schema说明及macOS进度。
  合并后当前/旧证据校验、防重复写入、正式宿主准入root接合3/3及fmt/diff通过；原三样本的合并前构建身份不重标，没有重复模型运行。

### W210 文件调用次数的摘要单位（2026-10-01；基线f2774ea2c）

- Case/子断言：C06、A05/A17/A19；W209同文件write/patch两次，原UI却显示“已编辑2个文件”，原截图/树另存首败。
- 修复：工具摘要按逻辑调用数使用“已编辑文件2次/已读取文件1次”，运行中也明确操作次数；中英文同步。
  按实际文件清单/去重目标统计的详情继续使用文件数量；原调用/重试计数、状态、输入/输出和canonical保持，不据文案推断更多文件效果。
- 验证：67项既有相关UI检查、类型/i18n/desktop边界、diff与正式custom-protocol构建通过。
  正式Tauri、新profile冷读W209原completed会话，独立14/14：同路径两调用、准确摘要、可展开详情、最终字节/hash说明、11回合终态、全事件/投影/效果和三组磁盘不变，新增事件0。
- 未覆盖：运行中及多文件patch没有新增模型样本，相关标签/文件清单分支由源码与既有检查覆盖；W206拒绝修正真实触发、进程/超时、完整A/C和共享门槛仍缺。
- 证据：仓库外2026-10-01/windows/w210-file-operation-count-ui；首败、定向日志、before/after DB、03/04正式UI与oracle独立保留，无新增付费调用。

### W211 综合A首败与历史记录ID说明（2026-10-02；正式基线a2d1d0777，首败始于10-01）

- Case/子断言：C01/C02/C03/C06/C07/C08，CMD-134～138、REAL-003/004与A05/A08/A09/A17/A19；原五项任务和严格断言不缩减。
- 正式GEN/StepFun新workspace/profile：cwd/Hidden、中文读搜、Git及两个Bun的实际结果正确，两测试各一次exit0/1、全部4命令reaped，十文件/Git状态不变。
  显式计划后收尾重复读样本两次，再将chatcmpl-tool调用ID当read_tool_history的记录ID，九次预检拒绝；最终report还引用旧搜索证据被拒，20步/10压缩后正式Stop为cancelled。
  原22项oracle17/22、报告首发拒绝，整体FAIL保留；读取/hash两断言因重复读失败，不能用首次内容正确代判通过。业务exit1是正常诊断，另10次参数拒绝不混成业务失败。
- 修复：历史工具顶层及字段说明区分hits[].id（64位记录ID）、hits[].call_id（原调用/搜索过滤）和next_after_id（翻页游标）；已知调用先query空串+call_id搜索，再复制记录ID。
  仅description改变，pattern/required/范围、查询/读取/身份隔离、原失败和freshness保持，不自动转换调用ID、不重新执行原工具。
- 验证：模型实际工具面最小回归先红后绿，ID/游标正负Schema与冻结工具表2/2，fmt/diff通过；正式停止/host清理、exit0/1、旧事件和无假交付独立8/8。
- 未覆盖：新说明的真实模型效果；显式计划收尾/重复回读/旧引用、完整A首发/N3及B进程/超时/C仍未闭合。未重复付费循环或把取消/组件通过当A PASS。
- 证据：首败在仓库外2026-10-01/windows/w211-comprehensive-a-closure（03/04中途、05终态DB/轨迹/严格oracle、06 UI）；修前/修后和清理核对在2026-10-02/windows/w211-tool-history-id-guidance。
- 同步结算：源码7f986ee01，正常合并248feb461为041d3564e，保留退出清理证明与macOS进度；受影响的桌面未验证退出/重启状态两项通过，fmt/diff通过。
  正式A样本仍为同步前FAIL，退出状态机测试不替代真实应用关闭/进程清理验收，未再调用模型。

### W212 预期非零与历史输出说明、压缩后重做首败（2026-10-02；基线462515003）

- Case/子断言：C06/C07/C08及综合A的CMD-134～138；原五项任务不缩减，W211 FAIL不覆盖。
- 代码核对：有效report已可关闭常规显式计划，不能将W211额外回读误归为必须重新执行的计划合同。
  说明改为预期非零是已结算观察，只有任务授权才检查/修复；已暴露历史工具可以找回已见输出用于用户要求的summary，但不能升级为当前证据或重跑原效果。
  计划/状态机、Schema接受集合、证据资格/计数、工具面与权限不变；这是指导修正，不能单凭文字声明模型根因闭环。
- 验证：两项首红保留；非零直接报告、旧结果/新鲜度、实际压缩保留阶段、显式待办合法修复、失败测试后的授权修复和历史ID共6项定向通过，fmt/diff及正式custom-protocol构建通过。
- 正式GEN/StepFun、新workspace/profile、原任务：9步/4压缩，cwd和目录枚举各重复两次，尚未走到搜索/Git/两测试及报告；及时正式Stop为cancelled。
  四个已执行命令均exit0/reaped、原目录属性正确，但零工具error不是任务成功。独立7项任务检查未达，整体FAIL；11/11安全/清理核对通过，十文件/Git、旧事件及无假交付保持。
- 未覆盖：本指导对诊断/历史收尾的真实效果、首次完整A/N3/COD及B进程/C；重复在报告前发生，下一根因核对压缩后的输入和已执行进度，不继续付费循环或用组件代判。
- 证据：仓库外2026-10-02/windows/w212-diagnostic-report-guidance；01/02首红、03～05定向、06构建、07取消事件/DB/原参数/失败oracle及08正式UI保存。
- 同步结算：源码10bdefd79，正常合并783b49a2c为6865cb180，保留macOS独立deadline夹具及进度；本批Rust源码未变，未重复构建/模型请求。
  新macOS原生deadline夹具未在Windows代跑，不影响上述6项定向和正式失败的覆盖结论；diff检查通过。

### W213 指令范围用途与压缩重做（2026-10-02；基线8ea8b9a9b，正式仍FAIL）

- Case/子断言：C02/C06/C07/C08、A05/A08/A17/A19；核对W212原摘要，entries_scanned=0的指令定位元数据被误当目录条目缺失，随后要求重新枚举并称没有实际命令。
- 修复：真实File owner返回observation_kind=instruction_scope、is_directory_listing=false，保留原kind/路径/权限/递归和计数；明确非递归零扫描不是空目录。
  摘要模型区分指令定位、计划状态与实际调用回执；截断提示移除无条件重读/重跑，继续保留未知、原限制和安全重放约束，不改硬预算/证据资格/工具面。
- 验证：三个首次失败另存；非空目录/文件/缺失元数据、根路径/隐藏及链接边界、实际压缩/截断、指令缓存和硬预算共9项通过，fmt/diff及正式custom-protocol构建通过。
- 正式GEN/StepFun、新workspace/profile、原五项任务：8步/5压缩后正式Stop，cwd3次、listing2次、样本读2次、搜索4次、Git status/diff各2次，仅第一个Bun一次exit0；整体FAIL、未交付。
  原oracle11/22保留；搜索按query建字典掩盖重复调用，另增原调用次数审计为10/22，不覆盖原结果或松断言。6命令exit0/reaped，十文件/Git/旧事件与取消清理等9/9保持。
- 新证据：五个ContextCompacted的retained_context只有原输入、无原工具交换，摘要将已做事项重新列待办，另一次产出伪工具调用JSON；不把这些文本当新授权或已执行证明。
- 未覆盖：本轮模型未调用instruction_scope，新增用途字段只有真实owner回归证明；元数据/摘要说明没有闭合整体连续任务。下一步核对固定上下文占用与原回执保留，完整A/N3/COD及B进程/C仍缺，不继续付费循环。
- 证据：仓库外2026-10-02/windows/w213-instruction-scope-compaction；01～03首红、04～08定向、09构建、10事件/DB/原oracle及加强audit、11正式取消UI分开保留。
- 同步结算：源码626091f23，正常合并b53fce0b4为204703da1，保留历史恢复字段/上下文一致性及macOS记录；合并后历史交付/严格证据两项通过，fmt/diff通过。
  正式样本仍为同步前FAIL，不重标构建身份、不重复模型调用。

### W214 固定前缀下最新回执的软余量（2026-10-02；基线47339c827）

- Case/子断言：C06/C07，压缩/重放与A08/A09/A15/A17/A19；沿W213无原工具交换证据做确定性定位，不再付费盲重试。
- 首败与根因：76,000-byte固定指令、80KiB原资源上限的有效调用/非零回执，在实际摘要后能完整装下，却被额外retained_token_limit软余量排除，只留下摘要和原输入。01首次失败保留。
- 修复：只对最新完整文本交换，实际摘要已知后按原input_limit/byte/message上限判断是否保留；更旧交换仍按已有软余量。调用/结果必须配对、原输入仅一次，失败和媒体原始保护不变，不取回未知archive或升级观察资格。
- 验证：修后101,256→79,148 bytes，原上限81,920，完整exit1/is_error=true及原调用ID保留；ContextCompacted持久化/restore逐字一致，模型摘要请求一次。
  同夹具更大回执仍不能越上限；新正/反回归加固定前缀、真实usage余量、无后缀的复核阶段、大摘要、必需上下文超限和typed prompt-overflow原限制共7项不同检查通过，fmt/diff通过。
  03移动局部变量时误放进另一个相似代码块导致编译失败，修正后04/06通过，原编译失败另留；未放宽断言或改变产品限制。
- 未覆盖：正式Tauri/StepFun修后连续任务/N3，超过硬容量的整个大批次仍只能摘要；本项不证明W213所有回执丢失均由软余量造成，也不关闭完整A/B进程/C或共享阶段。图片分支未改、未新增实际图片样本。
- 证据：仓库外2026-10-02/windows/w214-latest-receipt-budget；首红、修后完整交换/重放/超限反例及各定向日志分开保留，零新模型/命令执行，Git仅源码/最小回归/短进度。
- 同步结算：源码0b7bd7dc1，正常合并c2aa06028为8651083ec，保留macOS锁屏首发夹具记录；产品代码未变，不重复构建/模型，后补typed overflow原上限检查通过。

### W215 最新回执保留的正式样本（2026-10-02；基线7b2817641，整体FAIL）

- Case/子断言：综合A、C06/C07/C08及A05/A08/A09/A17/A19；新workspace/profile、原五项任务，正式Tauri/StepFun/step-3.7-flash。
- 验证：W214已进正式custom-protocol构建。GEN 11步/6压缩，前五个ContextCompacted分别保留2/2/3/2/2条原交换消息及原输入一次；最后一次大批次没有原交换。只新增最新回执保留N1，不代判连续任务或N3。
- 首败：第一次摘要是伪<tool_call><exec_command>文本；后续两个未要求的PowerShell脚本重新读取样本，按末尾换行拆分得到LINE_COUNT=5、末行空，真实文件仍43 bytes/4行/omega末行。
  搜索4次、Git status/diff各2次；两Bun各一次exit0/1，6原生命令均已退出/reaped，无清理错误。工具仅有正常诊断exit1，但步骤/结果错误仍使整体FAIL，正式Stop为cancelled。
- 原22项oracle11/22保留；加强原调用计数审计10/22另记，搜索字典去重错误不掩盖重做。十文件/Git/旧事件、实际测试序列、取消/host清理与无假交付独立10/10。
- 未覆盖：伪摘要的协议拒绝/有界恢复、重复补查及错误行数的根因、全批次保留、完整A首发/N3/COD及B进程/C。摘要文本不是真实调用或新授权；没有删除错误脚本输出，没有再付费循环。
- 本批无新源码，正式构建与历史首败分开。证据在仓库外2026-10-02/windows/w215-latest-receipt-live；01构建、02完整事件/模型/DB/原oracle/加强audit、03取消UI与原任务基线保存。
- 同步结算：证据be1819d61，正常合并8ae0f1197为352b58219，保留macOS锁屏审计；远端仅平台进度，本批源码未变，不重复编译/测试/模型请求，diff通过。

### W216 伪摘要拒绝与一次纠正（2026-10-02；基线d4ee526a4）

- Case/子断言：C06/C07/C08、MODEL协议及A01/A02/A08/A09/A17/A19；沿W215伪<tool_call>及W213裸调用JSON，不把摘要文本当任务授权。
- 首败：AgentCompactionSummary原先把裸XML/JSON工具调用当合法summary；01最小反例失败另留。修复摘要校验，原生ToolCall事件也进入同类拒绝，正常说明/代码示例/任务JSON仍可用。
- 修复：同次prepare仅一次协议纠正，原source消息不缩减、工具空/choice=None；有效草稿才写ContextCompacted，二次协议拒绝返回明确错误并保持原live input，不提交坏文本、不继续自重试。
  新CompactionSummaryRejected记录operation_id及安全reason，不存伪参数；Store取消后只收观察、接管fence仍拒旧writer，恢复仅将此无效果事件作为可丢弃model前缀，不跨可能效果重放。
- 验证：XML/JSON/原生调用三类各正/反纠正、拒绝事件codec/恢复、Store checkpoint接管/取消、模型端无工具、字节/空白/输出限与最新回执共8项不同定向通过，App编译检查及fmt/diff通过。
  02新增记录时在移动request后读取operation_id编译失败，捕获ID后03通过，原编译日志另留；未放宽断言/权限/预算或隐藏失败。
- 未覆盖：正式Tauri/StepFun修后/N3、非裸协议形态、已持久化旧伪摘要的历史处理、任务重做/错误统计的其他根因及完整A/B进程/C。二次拒绝不记任务完成，W215整体FAIL不覆盖，未新增付费模型或真实命令。
- 证据：仓库外2026-10-02/windows/w216-compaction-protocol；01首红、02编译、03～09修后/恢复/Store/集成及既有边界分日志保留，Git只收源码/最小回归/短进度。

### W217 正式执行链与报告关联缺口（2026-10-02；基线06714bc83）

- Case/子断言：综合A、C01/C02/C03/C06/C07/C08及A05/A08/A09/A17/A19；原五项任务、新workspace/profile、正式Tauri/StepFun/step-3.7-flash。
- 实跑GEN 12步/4压缩后completed：cwd/listing/样本读/两搜索/Git status/diff各一次；两个Bun通过start/poll各一次，原process_id配对、第一次exit0/reaped后才启动第二次exit1/reaped，未重复/修复/追加检查。
  4次压缩原输入各一次，保留2/4/2/2条原交换；没有伪摘要或协议拒绝，因此不将此样本当W216纠正分支真实触发/N3。
- 验证：独立执行合同16/16，真实UTF-8/43 bytes/4行/hash、Hidden标记、Git patch、测试序列/清理、1/1失败计数、原件/旧事件及中文非零UI一致；报告首次接受，仅有正常业务exit1。
- 原oracle16/22保持：它把四启动限定为exec_command，不能识别合法start/poll；新合同审计另存，不移除旧断言或将旧结果改PASS。
  真正交付缺口仍在：摘要只说当前目录为项目根，未给完整实际cwd；未区分旧观察与当前未复验状态。五项criteria都引用两个测试poll的ID，前四项的证据范围不匹配；账本接受不等于独立语义验收。
- 未覆盖：完整A首发/N3/COD及报告内容/证据关联、摘要错误分支真实恢复、B进程/超时/C；只新增不重做/正确执行链GEN N1，不关闭完整A或共享阶段，无第二次付费重跑。
- 本批无源码改动；仓库外2026-10-02/windows/w217-summary-protocol-live保留01正式构建、02事件/DB/轨迹/原oracle/独立合同audit、03完成UI与基线，Git仅短进度。

### W218 已结算poll的精确终态引用（2026-10-02；基线fd140acc3）

- Case/子断言：C05/C06/C07/C08、CTRL-006/007及A05/A08/A09/A17/A19；核对W217原调用，不新增付费盲重跑。
- 证据更正：W217五项criteria实际都引用cwd/目录枚举的两个exec_command ID，并非测试poll；目录项匹配，读搜/Git/测试四项范围不匹配。上一批相关文字误判，原制品及FAIL保留。
- 根因与修复：不可变命令终态原只接受launch_call_id等于结果call_id，排除了合法start→poll；现在仅保留已清理的精确退出/超时poll，要求原launch仍在窗口、过程action/process_id匹配、最后交互/结果ID/epoch一致且无省略。只证明该终态，不刷新文件/启动/其他交互或赋予重做权限。
- 验证：01最小反例首红保留；实际WorkStatus/CommandTracker链的exit0/exit1/timeout、15种不完整/错绑反例及既有终态、stdin链、Schema/历史和文件失效共9项不同定向通过。无新模型、正式UI或构建样本。
- 未覆盖：修后正式GEN/COD/N3；W217读搜/Git错引、完整cwd和历史/当前披露仍需闭环，缺旧scope的记录仍保守；完整A、B进程/超时及C/共享门槛未达。
- 证据：仓库外2026-10-02/windows/w218-terminal-poll-evidence，原调用关联审计、首红与修后日志分开；Git仅源码、两项最小回归和短进度。

### W219 正式Coding综合A与压缩后重复（2026-10-02；基线57329c83c，FAIL）

- Case/子断言：综合A、C01/C02/C03/C06/C07/C08及A05/A08/A09/A17/A19；正式custom-protocol构建、新workspace/profile、已有隔离data/work和StepFun/step-3.7-flash，原五项任务。
- 正式COD 10步/4压缩：cwd/隐藏枚举各一次；两Bun依序各一次exit0/1，四原生命令全部exited/reaped，无cleanup错误。首个测试后、第二个测试前重复样本读/两搜索/Git，合计读2、搜索4、status/diff各2；重复早于报告拒绝，不能归为拒绝后的效果重放。
- 首次report引用已失效的读搜/Git ID，整批预检拒绝，计数由1/1更正为2/1；最终summary还缺具体cwd/条目及历史/当前披露。及时正式Stop为cancelled，未交付假完成；原oracle14/22与一次报告拒绝保留，不记A PASS。
- 独立磁盘/Git/完整读/hash、两测试一次顺序、四命令清理、旧事件及业务非零/参数未执行UI共9项通过。第10项AX树缺截图可见的取消文字，原9/10不改；另附截图人工复核和canonical cancelled/host_cleanup_proven，未将截图补充混成自动通过。
- 四压缩保留原输入各一次、原交换2/0/0/0；实际压缩后输入74,477/74,898/75,898/78,131 bytes。没有伪摘要/协议拒绝；测试使用exec而非poll，因此W216纠正分支与W218新分支均未触发，不代判其正式有效性/N3。
- 未覆盖：完整A/N3、B进程/超时/C及共享门槛；下一步核对固定上下文/工具定义开销与最新完整交换，不放宽预算、不追加付费循环。本批无新源码/重复全仓检查。
- 证据：仓库外2026-10-02/windows/w219-coding-a-terminal-report，01构建、02首败DB/事件/模型/原oracle/独立audit、03/04正式取消UI及05时序/06复核分开保留；Git只更新短进度。

### W220 私有推理挤占最新回执（2026-10-02；基线be5e1f878）

- Case/子断言：C06/C07/C08、压缩/重放及A05/A08/A09/A15/A16/A17/A19；沿W219实际回执尺寸与消息构造定位，不增加付费样本。
- 根因证据：W219第5步测试结果JSON仅683 bytes/正文528 bytes，同轮ReasoningDelta却有6,453 UTF-8 bytes；最近交换把私有推理和调用/结果一起原样计费/保留，超过余量就整组丢弃。durable compaction原已将私有推理改为省略标记，实时保留路径却未同步，最小反例01首红保留。
- 修复：可选文本交换按持久化重放的相同私有推理标记计量并构造派生上下文；借用原字段测量，避免为预算探测复制大媒体/私有块。完整call/参数/结果/error、accepted input及其顺序保留，原日志/事件不改；unseen-image原交换分支不投影，原token/byte/message/text-tail限与证据资格不变。
- 验证：普通推理、带签名/密文的推理、供应商签名块和供应商遮蔽块四种夹具由100,849～101,405降至77,119 bytes，原上限81,920；完整exit1及原ID保留，持久化restore与派生上下文逐字一致，摘要请求无私有载荷。真实大输出仍不能装下，不能截短回执制造通过。
  新三项及既有软余量/硬预算/observed usage/typed overflow/报告复核/协议纠正/未压缩签名共10项不同定向通过；fmt配置检查与diff通过。00诊断脚本缺省字段KeyError另记为夹具错误，不当产品首败。
- 未覆盖：正式Tauri/StepFun修后/N3、W219全部重复和交付根因、真实签名供应商/媒体，完整A/B/C及共享门槛。只关闭确定性私有推理占用子根因，未把W219 FAIL改PASS；本批无新模型/命令/正式构建。
- 证据：仓库外2026-10-02/windows/w220-context-footprint，原轨迹仅尺寸审计、首红、四类修后、32KiB文本/未见图片与原预算/协议保护分日志；Git仅源码、三项最小回归和短进度。

### W221 私有后缀正式样本与Hidden位误判（2026-10-02；正式基线ab4678717，FAIL）

- Case/子断言：综合A、C01/C02/C03/C06/C07/C08及A05/A08/A09/A15/A17/A19；正式Tauri、原五项任务、新workspace/profile、已有隔离data/work、StepFun/step-3.7-flash。
- 新首败：目录脚本将Hidden位掩码与FileAttributes.Normal比较，Normal实际为128；9条目录记录全部报Hidden=true，和独立属性基线的7条非Hidden项冲突。真实enum/正确Boolean掩码另存05，不把exit0当结果正确。
- 正式COD 10步/6压缩后产品Stop为cancelled：样本读/两搜索/Git status各一次；两Bun各一次依序exit0/1，最后以原生git diff -- tracked-note.txt首次完成指定只读diff，五命令均exited/reaped。该原生Git是已授权的首次观察，不误算为额外重做；无报告交付，整体A FAIL保留。
- W220正式子证据：原输入六次各一次、保留原交换2/3/0/2/2/0，读搜未重做；仍两次零保留，不能关闭完整连续性/N3。独立磁盘/Git/字节/hash/搜索范围、测试顺序、五命令清理、旧事件及取消无假交付10/10；旧helper只认Hidden/System字段及git_diff工具，不将其形状限制代判本次效果。
- 指引调整：已有正确Windows示例后补充枚举位必须转Boolean或比较零，Normal=128而非零；不改脚本/输出，不自动修正结果、限制任意脚本或改变canonical schema/权限。既有宿主shell、字面argv及canonical admission subset三项定向通过，fmt配置/diff通过；未给该提示新增同构测试。
- 未覆盖：新提示的正式模型效果/修后N3；完整cwd/Hidden交付、报告、零保留剩余根因、完整A/B/C与共享门槛。提示晚于正式binary，本批不追加付费重跑或把FAIL改PASS。
- 证据：仓库外2026-10-02/windows/w221-private-tail-live，01构建、02进行中首败、03取消DB/事件/轨迹/独立audit、04 UI、05位掩码及06～08合同检查；Git仅一处提示和短进度。

### W222 最新回执的软token阈值（2026-10-02；基线ce958a2ed）

- Case/子断言：C06/C07/C08、压缩与A05/A08/A09/A15/A17/A19；沿W221零保留做确定性排查，不再先跑付费模型。
- 首败与根因：最新完整非零回执及摘要仅81,057 bytes/27,019估算tokens，低于原81,920 bytes及28,160输入tokens硬上限，却仍被token_trigger软阈值排除。W214只绕过旧历史余量，input_limit仍混入软触发值，01首红保留。
- 修复：只对摘要后最新完整文本交换使用原硬输入余量（扣除实际observed extra）；其他历史仍用软余量。实际PromptTooLong后的恢复上限继续更严格，未见图片、前置mandatory核对、字节/消息/文本限及证据资格不变；最终核对与错误日志使用实际选定上限。
- 验证：原95,628→81,057 bytes，完整exit1回执保留；provider实际用量额外1,500 tokens时同回执仍排除，结果78,571/26,191，原4,096输出及512安全预留保持。typed拒绝后70,062 bytes本可装入普通硬限，却正确受20,708 token恢复上限排除，摘要后59,571 bytes。
  两项新回归及既有硬字节/usage/typed拒绝/私有投影/报告复核/未见图片共9项不同定向通过，首败/修后分日志；fmt配置/diff通过。无新真实模型/命令/构建样本。
- 未覆盖：正式Tauri/StepFun修后/N3及W221两次零保留的完整归因；Windows属性提示效果、完整A/B/C和共享门槛仍缺。不用组件通过覆盖W221首败，也不称所有压缩丢失已修。
- 证据：仓库外2026-10-02/windows/w222-latest-hard-token-envelope；01首红、02实际余量及03 typed恢复、04～12定向日志，Git仅源码、两项回归和短进度。

### W223 正式回执保留与轮询期限混用（2026-10-02；正式基线b4e91e80b，FAIL）

- Case/子断言：综合A、C01/C02/C03/C05/C06/C07/C08及A05/A08/A09/A15/A17/A19；正式Tauri、原任务、新workspace/profile、已有隔离data/work、StepFun/step-3.7-flash。
- 首败：将start的timeout_ms=120000复制为poll的wait_ms=120000，超过原maximum=30000，整批预检拒绝/无owner执行。后续30000修正取得exit0；第二Bun一次exit1/reaped。报告又以supported引用旧读搜/Git ID被拒；原两参数拒绝及业务非零分开保留，及时正式Stop为cancelled，整体A FAIL。
- 操作事实：一个本机脚本合法合并实际cwd与9项枚举，Hidden/System全对；完整43 bytes/4行/hash、status/diff各一次且字节正确，两测试启动各一次、终态按原process_id配对/依序0/1，无原件或Git变化。搜索各一次且结果正确，但path用了父目录而非指定文件，独立范围断言FAIL不放宽；原审计12/13保留。
- 正式子证据：8步/6压缩保留原交换7/2/2/2/2/2、原输入各一次；本例没有零保留/读搜重做，W221 Hidden误判未重现，仅N1。首次报告Schema的eligible IDs包含两个真实terminal poll，W218终态保留分支实证N1；无伪摘要，不代判W216纠正分支。
- 指引调整：poll wait描述明确0～30000、独立于进程总期限，给0/1000/30000合法例值；search path明确文件/目录及只搜指定文件时不得用父目录。范围/默认/权限/原接受集合不改，不截断参数或改写输出；既有lifecycle/shell/admission三项通过，搜索描述变更后admission另验通过，fmt配置/diff通过，无新同构测试。
- 未覆盖：新说明正式效果/首发N3、精确搜索范围、历史结果/报告资格交付及完整A/B/C/共享门槛；提示晚于正式binary，修后不再付费循环，不把恢复或组件通过当整组通过。
- 证据：仓库外2026-10-02/windows/w223-hard-envelope-live，01构建、02原事件/DB/参数/独立audit、03/04取消及混合错误UI、05～08合同；一脚本合并cwd/list与start/poll合法，不由旧位置假设或字段名限制代判。

### W224 早先结果的最小报告形态（2026-10-02；基线473bd7938）

- Case/子断言：C06/C07/C08、CTRL-006/007及A05/A08/A09/A17/A19；沿W223有真实读搜/Git结果、却引用旧ID并声称supported的拒绝，不更改证据资格。
- 调整：disposition参数说明提供已有合法形态的最小JSON例：早先具体结果仍在summary交付，后续状态未复核的criterion用unverified/rationale且省略证据字段；示例须改为用户语言，不替代实际结果。未完成必要工作/未知效果仍需blocked，旧或无关ID仍拒绝。
- 验证：复用早先结果交付/旧资格拒绝、missing/stale无法关闭计划、pending patch必须blocked、固定计数及历史失败不抹除共5项既有定向通过，fmt配置/diff通过。只新增description，没有enum/默认/条件/验证器/权限变更或同构测试。
- 未覆盖：正式Tauri/StepFun是否采用该形态、报告首发/N3与完整A/B/C；W223原拒绝与搜索越范围保留，不用说明或组件通过声称模型根因已闭环。本批无新付费/真实命令/构建。
- 证据：仓库外2026-10-02/windows/w224-historical-report-shape；01～05定向日志，Git仅一处参数说明及短进度。

### W225 报告修正借用cwd证据与反馈定位（2026-10-02；正式基线5233e9cb7，整组FAIL）

- Case/子断言：综合A、C06/C07/C08、CTRL-006/007及A05/A08/A09/A16/A17/A19；正式Tauri原五项任务、新workspace/profile、已有隔离data/work、StepFun/step-3.7-flash。
- 正式COD 9步/4压缩completed，原交换2/3/2/2、所有操作各一次，目录Hidden/点前缀正确、读搜精确文件、Git字节保持、两Bun依序exit0/1且四命令reaped，操作合同独立13/13；只有预期非零及一次报告参数拒绝，修后无补查/重做。
- 首败/修后：首个report引用旧读搜/Git ID被拒；第二个把第1/2/3项criteria替换为cwd-only ID而不是unverified，Schema接受但范围不支持文件/搜索/Git。W224示例未被采用；账户接受不是独立语义通过，完整A仍FAIL。最终还省略完整cwd/9项条目及后续状态未复核披露，原事件/两报告不改。
- 根因证据：原拒绝仅给通用路径和当前enum ID列表，容易被当作任意替换集合。改进仅作用于单独报告的引用拒绝：标出最多16项的零起始criterion索引，移除引用enum错误的替代ID列表，附已有合法unverified/rationale例；其他schema值/计数、原拒绝、整批预检/权限保持。不自动改参数、不验证自由文本的语义。
- 验证：04最小反例首红保留；修后定位[1,2,3]、不复制私有summary/rationale/原ID、不改原参数，更新后的精确计数下合法修正通过。参数预检模块8项与历史资格1项共9项不同定向通过，fmt配置/diff通过；06首次源码写入失败另记为工具错误，源码完整核对后一次重试。
- 未覆盖：新反馈的真实采用/首发N3、summary信息丢失/压缩错误“未执行”状态、报告范围及完整A/B/C/共享门槛。修复晚于正式binary，不将恢复/N1计完整A，未再付费循环。
- 证据：仓库外2026-10-02/windows/w225-report-shape-live，01构建、02拒绝后完成DB/事件/两报告/独立audit、03正式UI、04首红/05修后/06编辑失败/07～08合同；Git仅反馈代码、最小回归和短进度。

### W226 受控timeout与不支持的清理阶段（2026-10-02；正式基线65f18d071，恢复完成/整组FAIL）

- Case/子断言：C05/C06/C07/C08、PROC期限/清理、LIFE/OBS关键终态及A05/A08/A10/A11/A13/A17/A19；正式GEN/StepFun、独立工作区，原W194 helper，start一次timeout_ms=6000/wait_ms=0，poll0→25、wait30000。
- 实跑8步/4压缩：READY父/子各一次，真实timed_out，无exit_code/信号，reaped=true；独立CIM20样本确认同一父子存活/亲缘及消失，心跳停止，3保护文件/旧事件保持，无原生PID操作/外部kill/重复进程。原独立12/13保留：清理errors含“CREATE_NO_WINDOW pipe无真实控制台interrupt”，不能因最后reaped抹去。
- 首败：report先引用READY等当前不合格ID被拒，W225新反馈定位[1]生效；随后因缺少/失效plan又拒绝，update_plan后第三report完成。无新副作用，但2次拒绝和内部available_evidence/call_id警告仍为FAIL；不记首发/N3或完整B。
- 产品修复一：已清理、无清理错误、无矛盾exit_code的明确timed_out与非零一样允许optional plan只报告；后续effect仍需原重规划，lost/unreaped/清理错误/未解决patch继续阻断。04测试桩用尽首错保留，05三步限最小反例确认失败；修后start/poll/report三步直接收尾且1/1超时计数保留。
- 产品修复二：平台owner声明interrupt支持度，Windows pipe=false、ConPTY=true；清理选支持的阶段后再构造同一StopBudget，不先尝试必失败中断。不吞返回错误，显式pipe interrupt仍诚实失败，Job terminate/force权限与精确reap保持；Unix默认真实interrupt路径不改。
- 验证：Agent超时/非零/反例/副作用gate/patch/终态资格8项通过；native期限首红15保留，修后16保留超时前文件效果且无伪interrupt错误；pipe Job父子5秒、显式不支持、ConPTY中断/父子reap及真实信号错误保护3项通过，共12项不同定向，fmt配置/diff通过。
- 未覆盖：修后正式Tauri/N3、READY历史资格/计划后终态引用、内部警告语言、完整A/B/C及共享门槛；源码修晚于本次binary，原报告拒绝/清理errors不改PASS，不增加长期/发布矩阵。
- 证据：仓库外2026-10-02/windows/w226-process-timeout-live，01构建、02原DB/事件/参数/独立audit、03UI、CIM witness、04/05/15首次失败及06～19修后；Git只留相关源码、最小回归和短进度。

### W227 干净timeout与直接报告正式GEN N1（2026-10-02；基线49a777d16）

- Case/子断言：C05/C06/C07/C08、PROC期限/清理及A05/A08/A11/A13/A17/A19；原六秒诊断、正式Tauri/StepFun、新workspace/profile、同隔离data/work。共享trait变更触发所需desktop依赖重编译2m07，不重复测试全仓。
- 正式GEN 5步/0压缩completed：start一次、poll0→25两次、READY父子各一次、timed_out/reaped；interrupt_attempted=false、terminate=true、errors=[]、cleanup.elapsed_ms=187。独立CIM18样本证明同一父子存活关系及消失/心跳不再变化、原件/旧事件保持，无外部kill。
- report一次首发接受，无update_plan/参数拒绝/新副作用；原超时1/1计数保留。这是W226清理选择和optional plan直接收尾的正式GEN N1，不将W226两拒绝/unsupported interrupt错误重写成成功。
- 原复制oracle12/13保留：最后一项限定旧回答“没有 exit_code”的确切文字，新回答为“未报告退出码0或具体信号值”，另存04审计说明。动作/终态正确，但退出码/信号无值的说明间接，READY不可当前验证的警告仍出现“可用证据”等内部措辞；整个体验不记PASS。
- 未覆盖：首发/干净清理N3/COD、READY历史回执资格、公开语言及完整A/B/C/共享门槛。本批无新源码、无需重跑已过的定向检查，正式样本与旧首败分开。
- 证据：仓库外2026-10-02/windows/w227-timeout-cleanup-recheck，01构建、02完整事件/DB/原参数/独立audit、03正式UI、process witness及04审计复核；Git仅短进度。

### W228 早期READY回执的历史资格（2026-10-02；基线090427d96，正式待验）

- Case/子断言：C05/C06/C07/C08、CTRL-006/007与PROC关键poll/终态、A05/A08/A09/A17/A19；沿W227真实READY被排除的问题，不增加付费盲重跑。
- 首红：实际WorkStatus/CommandTracker按read→start→READY poll→terminal→later command记录，已清理exit0后的早期成功poll仍不能引用；01原失败保留，不改W227的12/13或公开语言记录。
- 修复：复用原精确terminal谓词，仅对同process_id、成功且实际尝试、完整保留互动/启动/终态及scope的早期poll赋予历史事实引用；关联settled_process_poll的terminal ID，明确其当时状态/输出不证明当前运行或文件内容。缺链/未清理/lost/矛盾终态/省略交互继续拒绝；不扩valid_through或权限，不自动改report。
- 验证：三类真实tracker终态（exit0/非零/timeout）经后来命令后早期输出/terminal均可准确引用，文件与启动仍失效；21种反例拒绝。新两项及相关completion模块36/36，fmt配置/diff通过，首红/修后分日志。
- 未覆盖：正式Tauri/StepFun采用该资格及公开回答、取消历史poll、进程首发N3/COD、完整A/B/C与共享门槛。本批只有确定性源码修复，不以单元结果覆盖正式失败，无新模型/进程/构建样本。
- 证据：仓库外2026-10-02/windows/w228-historical-process-poll，01首红、02修后及03～05配置/diff/同步；Git仅源码、两项最小回归和短进度。

### W229 READY历史引用正式GEN N1（2026-10-02；基线1524c2498）

- Case/子断言：C05/C06/C07/C08、CTRL-006/007及A05/A08/A09/A11/A13/A17/A19；正式Tauri/StepFun、原六秒任务、独立workspace/profile、已有隔离data/work，所需desktop增量构建35.62秒。
- 正式5步/0压缩completed，start一次/poll0→25两次/report一次，无参数拒绝、update_plan、新进程或补查；criteria分别引用READY poll、精确timeout poll，W228新历史资格分支实证GEN N1，原W227警告/首败不覆盖。
- 独立断言在发送前记录，动作/回配/文件/原事件/报告引用17/17；CIM18样本确认同一父子存活亲缘及消失、心跳停止，实际timed_out/exit_code和signal均null，reaped=true/errors=[]、清理187ms，无外部kill。poll数量按返回cursor链而非固定次数判定。
- 体验仍未闭合：最终回答夹杂完整process_id/cursor/cleanup字段，未明确无signal；展开过程和页尾仍以“曾有1次尝试未成功”及通用调用/命令计数归纳预期timeout。人工复核另记FAIL_VISIBLE_UX，不把17/17当完整任务体验PASS。
- 未覆盖：公开语言/typed timeout汇总、首发N3/COD、取消历史poll、完整联合B/A/C及共享门槛。无新源码/重复cargo检查，不追加相同付费循环。
- 证据：仓库外2026-10-02/windows/w229-ready-timeout-live，01构建、02不可变事件/DB/调用/audit/人工复核、03折叠UI与04展开UI、独立process witness；Git只留短进度。

### W230 超时分类摘要与原始回执（2026-10-02；基线8bb9226b3，正式冷读12/12）

- Case/子断言：C05/C06、OBS结果分类与A05/A08/A09/A13/A17/A19；沿W229阶段“曾有1次尝试未成功”和页尾通用计数，不新增付费模型。
- 首红：normalize、completion展示及可展开详情三条新回归全部失败，01保留。修复仅识别严格原生timed_out且success=false/无矛盾exit与signal、已reaped/无清理errors及完整阶段字段的回执；保留status=error、原正文/输出/计数和Turn状态，未设nonFatalFailure或假报成功。
- 展示：同会话/Turn及原计数完整对齐时页尾说明达到运行时限/结束并清理，阶段和工具行同义；未知/缺失/清理失败/MCP同名/混合结果保持通用披露。已timeout的历史重试记录保留，后来成功不移除原回执。
- 验证：三个首红修后52项、邻近83项共135项不同UI检查通过；新增测试类型错误05另留，修后类型检查通过并单验该项。中英文i18n、最小880x600边界、diff及正式UI/desktop构建通过；不跑全仓。
- 正式隔离profile冷读W229，页尾/阶段/详情均为时限分类，原READY/timeout和compact JSON回执可展开；events/turns/sessions行数与全字节、全部6个夹具文件保持，模型/事件新增0。首audit11/12因03截图尚未展开，原记录保留；03b展开后另审12/12，断言不改，不把采集错误当产品失败。
- 未覆盖：新实时timeout、混合汇总的正式UI、模型公开内部字段/无signal说明、首发N3/COD、完整A/B/C及共享门槛。本批只关闭分类冷读子缺口，不把原W229整体体验改PASS。
- 证据：仓库外2026-10-02/windows/w230-timeout-outcome-ui，01首红/02修后/03～12相关检查与构建，01-cold的before/after DB、原11/12与复核12/12、折叠/展开/原始回执UI；Git仅相关源码、五项最小回归和短进度。

### W231 错摘要之后仍可读的原生输出（2026-10-02；基线8cca7d4b7，正式待验）

- Case/子断言：C01/C06/C07/C08、CMD-134/CTRL-006/007及A05/A08/A09/A15/A16/A17/A19；先审W225原轨迹，不继续付费盲重跑。
- 根因证据：原cwd/list stdout分别138/778 UTF-8 bytes；首压缩摘要有完整cwd及九项列表，后三摘要却改为pending或未执行。对应两个exec的精确身份/终态仍可引用，但完成上下文只给scope/terminal，没有原输出；模型最终只说“已列出”而漏具体值。01最小首红确认原生结果在context中缺失，不覆盖W225语义FAIL。
- 修复：仅保留同调用、同owner的完整小原生output.text和原cursor/retained/dropped/encoding元数据，作为当前已合格call的observed_output数据；优先记录事实而非冲突摘要，缺字段表示未保留。非派发、错result ID/owner、别的capability/action不提供该数据；env/stdin/文件正文/额外字段不复制。
- 边界：每条2KiB、总计4KiB，仍纳入原32KiB/64观察限制，超限整条省略、随观察一起淘汰。原日志/失败计数、freshness/路径/报告Schema/权限及token/byte硬限不变，旧调用资格和unknown工作不因输出缓存升格。
- 验证：实际WorkStatus/CommandTracker经过后来非零命令仍保留完整中文cwd/list及原exit/失败计数；九类错配/未派发/未证明/非JSON反例拒绝，Unicode大输出及聚合限制/淘汰无剪裁。completion39/39（含新三项）及mandatory/fixed-prefix/observed-usage三项，共42项不同检查通过；首红/修后、fmt配置/diff分日志。
- 未覆盖：正式Tauri/StepFun是否用原输出抵抗错摘要及交付cwd/列表、报告语义/引用范围、N3/完整A/B/C。本批无新模型/命令/正式构建，只记确定性子根因修复。
- 证据：仓库外2026-10-02/windows/w231-retained-command-output，00原轨迹尺寸/摘要哈希、01首红、02/03修后和04/05及三项预算保护；Git仅源码、三项最小回归及短进度。

### W232 综合A重复cwd/list及预算失败（2026-10-02；正式基线d76730452，FAIL）

- Case/子断言：综合A、C01/C02/C03/C06/C07/C08与A05/A08/A09/A15/A17/A19；正式Tauri/StepFun/COD、原五项任务、新workspace/profile，复用W230 frontend，仅desktop所需构建45.77秒。
- 首败：原正确cwd/九项目录后，第三次压缩后的第5步以完全相同参数各执行一次cwd/list，再做首次Git status/diff。读样本及两次单文件搜索此前正确且各一次，原件/十文件/Git未变；四native均exit0/reaped/errors=[]，未启动两个Bun，也没有report/假完成。独立操作原11/15、最终六项交付缺失及重复调用人工审计保留。
- 随后回合自行failed，错误归NomiFun：compaction替换83,428 bytes、27,810估算tokens超过27,731实际输入余量79 tokens，未违反字节/消息限。正式UI“应用处理失败”；发现时已终止，没有执行取消或重试，没有外部kill或新付费循环。
- 原三次成功压缩before82,174/81,123/81,702→after79,234/78,280/80,743，保留1/1/2调用。不能以W231组件PASS或缓存存在代判模型效果；报告具体值/引用仍缺。下一步按实际mandatory前缀余量核对summary上限，冻结token/byte/输出及accepted input不变。
- 未覆盖：修后压缩预算、W231真实输出采用、禁止重做/完整A首发与N3、B/C及共享门槛。本批无新源码/重复测试；拟Stop前已failed，历史不记cancelled。
- 证据：仓库外2026-10-02/windows/w232-comprehensive-a-output，01构建、02-first-replay原DB/事件/调用/11-15 audit/compaction尺寸/人工复核、03失败UI；无模型请求完整投影夹具，不声称已核对observed_output全部实际发送内容，Git仅短进度。

### W233 合法摘要仍超替换余量的有界纠正（2026-10-02；基线5203270e3，正式待验）

- Case/子断言：C06/C07/C08、compaction输入边界及A08/A09/A15/A17/A19；沿W232实际79-token超限，先补最小确定性反例，无新付费循环。
- 首红：4,000-byte摘要本身小于原8KiB硬限，但62,000-byte固定前缀加accepted input后整体66,571超过65,536，原prepare直接失败；01保存。新校验按完整replacement的实际序列化byte、token estimate/message及必须缩减条件，尚未适配的draft不会先写入replacement。
- 修复：记录原operation的REPLACEMENT_CONTEXT_BUDGET摘要拒绝，整个prepare仅一次同一完整source的更短、无工具纠正；原protocol/output纠正、分片及调用上限不扩大。第二次仍不能适配则明确失败并保留原context，不剪裁draft、丢accepted input或放宽硬限/actual usage余量。
- 验证：新三项通过：字节近固定前缀一次恢复、500-token实际额外余量保持、二次失败只有两次请求且原input逐字一致/两条拒绝事件/零ContextCompacted。旧fixed-prefix/usage/硬token/typed恢复/mandatory/协议/输出过长/耗尽八项通过，共11项不同检查；fmt配置/diff通过，首红/修后分日志。
- 未覆盖：正式Tauri/StepFun采用新纠正、完整输入/原输出保留的模型效果、W232重做及A最终引用/交付、N3/完整B/C/共享门槛。组件恢复不关闭W232首败，本批无新模型/命令/正式构建。
- 证据：仓库外2026-10-02/windows/w233-summary-fit-recovery，01首红/02修后、token及二次失败原上下文与03事件、八项邻近检查及04/05配置/diff；Git仅两处源码、三项最小回归与短进度。

### W234 全部操作完成但摘要仍不适配（2026-10-02；基线ed7a3f680，正式FAIL）

- Case/子断言：综合A、C01/C02/C03/C05/C06/C07/C08与A05/A08/A09/A15/A17/A19；正式Tauri/StepFun/COD、原五项任务、新workspace/profile，desktop所需构建41.55秒，复用同frontend。
- 8步/六压缩：cwd/list、43 bytes/4行/hash完整读、精确文件两搜索、Git status/diff各一次；两Bun按原顺序start/poll各一次exit0/1，四个精确process_id最终exited/reaped，无重做或参数错误，十文件/索引/原事件保持。1/1预期非零是合法诊断，不混成系统崩溃。
- 完成前W233新分支两次REPLACEMENT_CONTEXT_BUDGET拒绝，回合自行failed：83,993 bytes/27,998估算tokens超过27,876余量122 tokens，原context保持。一次纠正和拒绝记录正式触发，但不是恢复PASS；UI如实显示“应用处理失败”，未执行取消/Retry或追加模型。
- 原独立操作13/15保留：first report和canonical completed未达到，最终六项具体交付均缺失。不能由动作正确、不重做或有界失败关闭完整A/N3；下一步缩减重复完成说明的固定开销，硬预算/required input/资格仍保持。
- 未覆盖：缩减后正式效果、成功摘要纠正N3、A引用/交付、完整B/C与共享门槛。本批无新源码/重复组件检查，W232原首败不改。
- 证据：仓库外2026-10-02/windows/w234-summary-fit-live，01构建、02-first-fail原DB/事件/参数/13-15 audit/人工复核与六次压缩/两拒绝、03失败UI；Git仅短进度。

### W235 完成context说明精简（2026-10-02；基线d4d856276，正式待验）

- Case/子断言：C06/C07/C08、完成/上下文边界与A08/A09/A15/A17/A19；沿W234真实122-token余量不足，减少不可压缩的重复说明，无新付费循环。
- 调整：completion.context英文说明由3,856缩至2,400 UTF-8 bytes，减少1,456；原context JSON/原生输出缓存、eligible集合/epoch、required input、Schema/权限/计数/硬预算全同。指引仍明确只用匹配scope的top-level证据、历史/当前状态分开、历史恢复不升格、unknown/未完成保持blocked、不重做、每criterion八ID与全需求覆盖。
- 验证：复用completion39项（含旧引用/借用范围提示/历史/失败计数/文件/进程与输出边界）及mandatory预算、实际usage余量、健康报告修正不重做三项，共42项不同检查通过；fmt配置/diff通过。只改说明，无新增同构测试或语义NLP校验。
- 未覆盖：正式Tauri/StepFun实际余量、是否减少压缩/完成交付、W234成功恢复及N3、A引用语义/完整B/C/共享门槛。尺寸减少不是正式任务PASS，未放宽assert或预算、未改原首败。
- 证据：仓库外2026-10-02/windows/w235-completion-context-copy，01/02原新说明、copy-size-audit及03组件/三项邻近与04/05配置/diff；Git只保留一行模型说明和短进度。

### W236 具体结果交付及零起始索引误修（2026-10-02；基线d5fdbd765，recovered/整体FAIL）

- Case/子断言：综合A、C01/C02/C03/C05/C06/C07/C08与A05/A08/A09/A15/A17/A19；正式Tauri/StepFun/COD、原五项任务、新workspace/profile，所需desktop构建1m06，复用同frontend。
- 12步/七压缩completed：合法一命令打印cwd再JSON列9项，真实Hidden/System全对；instruction_scope与完整文件读分开，实际读/两单文件搜索/两Bun各一次，tests依序start→poll exit0/1，三native精确process均reaped。文件/Git/原事件保持，实际业务非零1/1。
- W233正式恢复N1：末次一条REPLACEMENT_CONTEXT_BUDGET拒绝后成功继续，after77,977/77,290/79,607/79,643/81,132/82,788/83,619，保留1/1/2/2/1/3/1调用；无应用预算失败。最后六项具体交付存在检查全绿，包括完整cwd/9名字和元数据；早期事实/后续未核验披露保留。
- 首败仍在：git_status以{}调用两次且结果相同；首report的旧文件/搜索/Git引用拒绝提示零起始[1,2,3]，模型误改第1/2/3项、漏第4项，后续两拒绝才修准。第四report四项unverified、测试项supported，精确两poll范围正确，未借cwd ID。三参数错误/四累计工具失败与原1命令非零保持。
- 原audit12/15不改：目录一个false来自合法cwd+JSON两行输出的整串JSON解析限制；另存第二行原9项与Win32基线严格复核，重复status及首report不通过是真失败。最终四条英文“后续未复验”rationale在中文回答中重复，公开体验仍FAIL，账本completed不是完整A/N3或共享达标。
- 未覆盖：反馈位置/用户语言修后首发与N3、重复Git恢复根因、完整A首发/GEN及B/C。下一批只修有证据的零/一起始歧义与语言提示，不重跑模型或吞掉历史失败。
- 证据：仓库外2026-10-02/windows/w236-compact-guidance-live，01构建、02-first-duplicate完成事件/DB/四报告/原audit/属性及scope复核与澄清、03正式UI；本批无新源码/重复测试，未执行Stop/Retry。

### W237 报告纠正的明确项号和路径（2026-10-02；基线79db35856，正式待验）

- Case/子断言：C06/C08、CTRL-006/007与A02/A08/A09/A16/A17/A19；沿W236误把零起始索引当一起始项号的两次额外报告拒绝，只修反馈，不新增付费循环。
- 首红：既有五criteria夹具仅第2/3/4项旧ID不合格，原payload没有一起始项号/精确JSON位置；01保留。新增两字段分别给[2,3,4]与/criteria/1、/criteria/2、/criteria/3，原零起始字段保持；均从最多16项的数字位置生成，原私密参数/ID不回显。
- 说明明确按精确路径修正，保留已合格criteria；历史实际结果可交付但不升格current，禁止借用无关ID或重做。rationale须使用summary语言并描述实际不确定性，不逐字复制示例英文。原validator接受集合/Schema、失败/纠正计数、整批无派发与输入参数保持，不自动改报告或翻译自由文本。
- 验证：参数预检8/8、正常同步5f639b7d8后的completion40/40及健康报告纠正不重做/重复拒绝上限两项，共50项不同检查通过；位置精确、原report不变及private字段不泄漏，首红/修后、fmt配置/diff分日志。加强既有回归，不新建同构测试。
- 未覆盖：正式Tauri/StepFun采用新项号/路径/用户语言，原Git重复根因、完整A首发/N3/GEN、B/C及共享门槛。说明/定位数据不是模型行为PASS；W236三拒绝、四英文警告及整体FAIL保留。
- 证据：仓库外2026-10-02/windows/w237-explicit-criterion-locations，01首红、02预检修后、03同步completion与两项闭环检查、04/05配置/diff；Git仅两位置字段/说明、既有回归断言和短进度。

### W238 完整B文件与进程衔接（2026-10-02；基线35806c910，正式FAIL）

- Case/子断言：完整B、C04/C05/C06/C07/C08、PROC-027/030/033/036与A05/A08/A09/A10/A13/A15/A17/A19；正式Tauri/StepFun/GEN，独立workspace/profile、已有隔离data/work，构建26.04秒。
- 文件链按序写18-byte三行LF无BOM、read真实digest、只改第二行、原生cmd copy/move一次及精确del一次；源文件删除、终版/echo输入字节均alpha/值=2/omega及一个末LF，保护四文件hash全同。合法cmd /d /c未被当作PowerShell语法失败；未列目录、探程序、读helper或执行测试。
- echo一次start→READY→一次input18 bytes/append_newline=false→close→poll退出0/reaped；STDIN_EOF的18-byte/hex在close回执，下一poll cursor119正确，零工具结果错误。hold一次start/两READY→同process_id cancel一次，cancelled/reaped/errors=[]、182ms；独立CIM50样本证明同一父子亲缘及消失/心跳停止，无外部kill。
- 16步/九压缩后回合自行failed：两条REPLACEMENT_CONTEXT_BUDGET拒绝，最后84,385 bytes/28,129估算tokens超过27,885余量244；最后moved-file read/report未到达，UI“应用处理失败”，未取消回合或Retry。原操作成功不代替最后交付，完整B仍FAIL。
- 原audit20/28不改：copy/delete两项只识别PowerShell，echo两项忽略stdin/close的output游标及EOF位置；另审精确cmd参数/结果和完整交互/磁盘回执均正确。真正的final read、report、canonical completed与全顺序缺口保留。00夹具源生成嵌套引号首错独立记录，发生在模型/应用前，不混成产品失败。
- 未覆盖：完整B交付/修后预算、N3/COD、其他停止/timeout连续首发、完整C及共享门槛。下一步只减掉模型context重复空metadata，不放宽硬限、证据、参数或源任务；无新源码/重复组件测试。
- 证据：仓库外2026-10-02/windows/w238-combined-b，00夹具错误、01构建、02-first-fail原DB/事件/参数/20-28 audit及精确复核、03失败UI、独立process witness；Git仅短进度。

### W239 空metadata字段的模型投影（2026-10-02；基线03562c54b，正式待验）

- Case/子断言：C06/C07/C08、上下文/完成与A05/A08/A09/A15/A16/A17/A19；沿W238已结算全部进程但最后交付仍超244 tokens，减少每轮重复的空metadata，无新付费循环。
- 首红：真实tracker的三个可引用过程观察仍输出无值wrapper字段，01保存。现available/ineligible/root metadata及scope只省略一层null；scope中的original requested_arguments及command/output/数组不递归处理，原参数里的显式null继续逐字保持，零cursor/false/非零exit/清理及精确身份均保留。
- 原Scope/observations台账、资格谓词/Schema、known/unknown效果、计数/权限/required input和冻结token/byte/message预算保持；派生context更短不等于刷新证据或补齐未执行任务。缺失字段继续表示无该metadata值。
- 验证：新回归原参数/观测不变、三个精确eligible ID/早期poll→terminal、非零及false等断言通过；completion41/41及mandatory过限不调用模型、actual usage余量、健康报告修正不重做三项，共44项不同检查。首红/修后、fmt配置/diff分日志。
- 未覆盖：正式Tauri/StepFun实际省下多少预算及B最后read/report、模型语言与N3/COD、完整A/C及共享门槛。W238真实预算failed、原20/28与复核保持，本批无新模型/命令/正式构建。
- 证据：仓库外2026-10-02/windows/w239-absent-context-metadata，01首红/02修后精确语义、03完整completion与三项邻近、04/05配置/diff；Git仅一层metadata投影、一个最小回归和短进度。

### W240 主线B来源SHA未使用（2026-10-02；基线4c367206c，FAIL/cancelled）

- Case/子断言：完整B、C04/C05/C06/C07/C08与A05/A09/A10/A13/A17/A19；收尾清单下修后正式Tauri/StepFun/GEN、新workspace/profile，所需构建40.03秒，任务不缩减，断言在运行前按实际合法shell/输出消费修正。
- 首败：第二步直接write_file(path,content)无来源SHA保护，未调用digest-protected patch；即使终版字节值=2正确，也违反明确用户要求。两个write原参数保留，不以最终字节正确代判该断言。
- 写/读/原生cmd copy/move/del、echo18-byte输入/close/exit0、holdREADY/cancel均无工具错误。保护hash及终版/echo真实字节保持；CIM92样本确认父子存活亲缘与消失，hold清理已结算。首败保存后正式Stop为cancelled，最后moved-file read在停止前已完成，但report未到达，未假报全任务成功或外部kill。
- 原audit23/28保留：来源摘要缺失、两write、未首report/未completed/整序未关闭为真实FAIL。UI取消前历史/最后残留“Private reasoning omitted from replay”占位文本，归合并展示余项；不为说明单开批次。未取得报告不能代判预算完全修复或B结案。
- 未覆盖：来源保护修复、完整B/N3/COD及最终报告/公开语言；下一主线为C连续→纠正→压缩→停止→冷读，不对同一未改B根因重复付费。本批无新产品源码/组件测试，原W238失败保留。
- 证据：仓库外2026-10-02/windows/w240-mainline-b-closeout，01构建、02-first-unprotected-write原首败、03-cancelled完整DB/事件/23-28 audit/原两write及独立物理复核、04正式取消UI、process witness；Git仅短进度。

### W241 主线C连续、纠正与冷读（2026-10-02；基线67617e1cf，FAIL）

- Case/子断言：完整C、C05/C06/C07/C08与A05/A06/A09/A10/A12/A13/A17/A19；正式Tauri/StepFun/COD，新workspace/profile，复用hash核对一致的现有正式构建，无重复编译。
- 首败：用户明确只启动一次helper、完整来源读取不重做，实际start三次/cancel两次、两份来源各读三次。追加纠正通过正式“立刻发送”进入同一回合，原turn/steer-accepted保留；不是新回合，也没有操作者Stop/Retry。
- 27步/15次实际压缩后自行failed：纠正写入被重规划门禁阻止，四次update_plan被逐字来源校验拒绝。其中“再回读该结果一次核对”多加了原输入没有的“核对”，重复原错未恢复；纠正文件和被禁止的初稿均未生成。保留原audit10/21及另审same-turn steering事实，不改历史失败。
- 保护文件全部hash不变、当前父子PID均消失、宿主清理事件存在；70条进程见证不能代判三次启动的全部亲缘。正式Stop环节因回合提前失败未到达，不能把failed计成cancelled。
- 零模型冷启动后agent_events/turns/sessions全行数/hash及全部夹具文件保持，全部Turn仍终态，无复活/新执行。但实时UI曾展示的纠正气泡在冷读会话不可见，canonical仍有纠正；记录为恢复展示缺陷，冷读完整性不计PASS。
- 未覆盖：C正确执行/实际Stop/完整冷读展示/N3，A报告纠正与重复观察，B摘要保护与最终交付，共享门槛仍未达。不付费重复未修根因；下一步合并定位这些主线阻断，不开说明微调批次。
- 证据：仓库外2026-10-02/windows/w241-mainline-c，02纠正前、04/05失败原事件/参数/DB/audit、06实时UI/冷读前全表基线、07正式冷启动、08冷读UI、09全表/磁盘/当前PID复核；本批Git仅短进度。

### W242 已接受纠正的冷读消息身份（2026-10-02；基线c2cae2177，子断言PASS）

- Case/子断言：C07/C06、LIFE冷读与A03/A09/A12/A17；直接修W241正式首败，不重启模型或重放操作。
- 根因：steer的projection_id是独立message:event UUID，correlation_id保留所属Turn operation。历史响应误把operation当消息UUID并过滤，导致原始纠正仍在DB、重启UI却消失。现accepted用户消息从已有message projection身份读取，兼容旧记录，不迁移/改写canonical事件或Turn归属。
- 验证：新增双纠正身份回归修前红/修后绿；相关历史17/17、fmt/diff及desktop边界通过。正式Tauri构建40.84秒，原W241失败会话零模型冷读7/7：纠正气泡与failed提示可见，events/turns/sessions全部行数/hash、全部磁盘字节保持，无旧Turn复活。未调用Stop/Retry。
- 未覆盖：只关闭既有纠正消息的冷读展示缺陷；W241重启/重读/逐字来源循环、C执行及实际Stop/N3仍FAIL或待验。B来源保护/交付、A重复Git/报告及共享仍未结案，下一项按固定顺序合并A复验。
- 证据：仓库外2026-10-02/windows/w242-mainline-recovery，01首红、02真实projection根因、03历史回归、04正式构建及原会话冷读UI/DB/7项审计；初始化元数据文件路径首错另存，已启动GUI直接核对恢复，未重复启动模型。Git仅消息身份修复、最小回归及短进度。

### W243 合并A复验与压缩预算首败（2026-10-02；基线77351ad09，FAIL）

- Case/子断言：完整A、C01/C02/C03/C06/C07/C08；正式Tauri/StepFun/COD，一次新workspace/profile样本，冻结4分钟观察上限，复用正式构建。
- 首败：自主PowerShell文件摘要命令把-join放在ForEach-Object管道后，真实exit1；修正提议被重规划门禁拒绝、未执行。cwd/含真实Hidden标记目录、专用read_file全文/摘要、精确两搜索、Git只读各取得实际结果，Git没有重复，第一Bun测试实际exit0/reaped。
- 约2分57秒、11步/8压缩后自行failed，第二Bun测试和report未到达；最终预算拒绝83,681 bytes/27,894估算tokens超过27,854输入限制40。UI“应用处理失败”对应真实Nomi压缩错误，未点击Stop/Retry、未消耗完整观察上限，不计报告位置/语言修复的采用或A PASS。
- 原audit10/16及交付0/6保留；初始夹具把cwd/listing强制为单一进程，原脚本另存，按两项请求可分开执行纠正合法形态，完整结果/各一次/顺序/无额外scope断言保持。被拒且未dispatch的命令不证明孤儿，实际已启动进程的exit0/清理另审；十文件/Git及原事件保持。
- 未覆盖：A完整首发/N3/GEN和报告纠正采用，B来源保护/交付、C重读重启/引用/Stop。下一步只定位正常压缩触发阈值是否误作替换接纳上限，保留冻结硬预算及供应商usage margin，不再减少说明来追逐少量token或付费循环。
- 证据：仓库外2026-10-02/windows/w243-mainline-a-closeout，02前缀首败、03终态原DB/事件/参数/audit/独立复核、04正式UI；本批Git仅短进度，共享仍未达。

### W244 压缩触发阈值与接纳上限混用（2026-10-02；基线5b1198819，预算正式N1）

- Case/子断言：C06/C07/C08与A05/A09/A15/A17/A19；沿W238/W243内部预算终止直接定位，不扩展相邻Case。
- 根因及两首红：正常压缩把留余量的soft trigger当replacement接纳上限。夹具摘要27,857 tokens在原冻结28,160硬上限内，却因27,294触发值被拒；只修接纳后下一小步又付费重复压缩。两首红分别保留，当前正常替换按原冻结硬上限扣actual usage margin验收，accepted token floor为后续小步保留余量；typed PromptTooLong严格恢复cap、字节/消息/输出限、原输入/工具/回执与一纠正上限均保持。
- 验证：新增一个完整反例同时核对接纳与下一小步无重复摘要；相关compaction19项、typed cap/mandatory各1及另外5项summary共26项不同定向通过，fmt/diff通过，正式构建45.80秒。
- 正式一次COD原完整A，约1分33秒、七步/五压缩，summary拒绝/typed恢复0，两指定Bun测试各执行一次exit0/1、清理证据成立；report一次首发接受，supported引用精确两测试，四项历史unverified使用中文，失败计数1/1保留。UI把exit1显示为命令正常结束非零，没有应用失败；未Stop/Retry/外部kill。
- 整体A仍FAIL：Git status和diff各执行两次，最终只用“沿用先前观察/当前未验证”代替cwd/九条目/完整样本与SHA/两搜索/Git修改的实际交付。原audit15/16、具体交付1/6保持，canonical completed不替代验收。测试链和预算正式N1不计完整A/N3，W243原首败不覆盖；通用计数英文尾注另归已有展示残余，不开微调批次。
- 未覆盖：完整A防重复/具体结果、B来源SHA/最终报告、C不重启重读/引用纠正/实际Stop，必要N3/入口及Windows差异，共享未达。不对这些未改根因付费循环；下批只处理主线直接阻断，复用本次预算/测试及W242冷读证据。
- 证据：仓库外2026-10-02/windows/w244-mainline-compaction-envelope，01原接纳首红、03再触发首红、04/05不同回归、06构建、07独立正式夹具、08重复Git原记录、09完整DB/事件/参数/audit/精确scope复核、10正式UI。Git仅预算修复、一个最小回归与短进度。

### W245 搜索/Git实际只读结果在压缩中丢失（2026-10-02；基线0c7d6786c，交付改善N1）

- Case/子断言：C02/C03/C06/C07/C08与A05/A06/A09/A15/A16/A17/A19；沿W244遗漏结果直接修复数据保留，不补说明微调或重复旧矩阵。
- 首红：原completion scope只保留search/git请求参数，实际匹配/零匹配/status entries/diff正文没有进入压缩后固定context，缺值只能依赖摘要。强化既有四结果回归，01保存；现仅保留已dispatch、同call返回、成功、scoped ReadOnly的三个指定action字段，单快照512 bytes、原scope2048/detail4096/台账32KiB不变，超限整项省略不截断。私密额外字段、未知记录字段、错query/身份/能力/动作、未dispatch/失败/opaque/effectful均不保留；截断/不完整事实原样保持。
- 原观察/权限/Schema/资格/epoch及计数保持，历史snapshot不是当前证据。completion45/45、mandatory硬限/合法摘要无立即再压缩/健康报告修正不重做三项，共48项不同定向通过，fmt/diff与正式构建26.63秒；一个空匹配测试filter不计验证，已另用精确名称验证。
- 正式一次COD原完整A约1分24秒、11步/四压缩，全部请求观察与两测试各一次，exit0/1，原件/Git/原事件保持，summary拒绝0。原文/43bytes/四行/全SHA、两搜索及Git修改已交付，原audit15/16、具体交付4/6；首report第2/3/4项的五个旧ID被enum预检拒绝、未dispatch，随后只改report无新平台动作，计数2/1保持。
- 整体A仍FAIL：实际cwd及完整九条目未交付，三个rationale复制英文示例；不把操作无重复或canonical completed代判完整A/N3。UI分清正常exit1与一项参数未执行，没有Nomi应用失败。未Stop/Retry/外部kill；旧W244失败保持。
- 未覆盖：完整A首发/全部结果/语言/N3/GEN，B来源SHA/最终报告、C重启重读/逐字引用/实际Stop。按收尾约束转C直接阻断，不启动未修A的付费循环；本批只关闭丢失search/Git小结果的确定性根因。
- 证据：仓库外2026-10-02/windows/w245-mainline-read-result-retention，01首红/02修后、03/04定向回归及真实原结果大小、05构建、06隔离正式夹具、07原DB/事件/参数/audit/两report与独立复核、08正式UI。Git仅数据保留/最小回归与短进度。

### W246 运行进程身份/游标被完成资格过滤（2026-10-02；基线3302b055e，连续纠正N1）

- Case/子断言：C05/C06/C07/C08与A03/A05/A06/A09/A10/A12/A13/A15/A17/A19；沿W241重启/重读查实际固定context，无新角色或旁支矩阵。
- 首红：running结果不能作完成证据，原缓存process_id/READY/next_cursor却只给eligible条目，压缩后活进程信息丢失。现原缓存里running且仍在work tracker的结果进入ineligible数据，另给last_observed_running_processes；资格仍false、无授权/刷新，旧running记录不能在tracker清空后冒充活进程。原单输出2048/aggregate4096/detail4096/台账32KiB及冻结预算保持。
- 验证：新回归修前红/修后绿，包含失去tracked状态后不展示；completion46项及三项硬预算/余量/报告不重做，共49项不同检查通过，fmt/diff与正式构建27.08秒。测试READY两个LF的正确游标25另校准；仅测试夹具修改，没有再次编译产品或额外模型循环。
- 正式一次COD C：13步/四压缩，同Turn的steer-accepted与steering_inputs均保存，一个实际helper、两来源各完整读一次、无cancel/restart；新输入使一poll/一write提议未dispatch，replan后仅一次实际写入及一次回读，18-byte UTF-8/LF内容/hash完全匹配，禁止初稿不存在、原件全保持。
- 正式Stop为cancelled，但独立277条CIM及毫秒时间核对证明父子已在helper原120秒deadline后约120.106秒消失，cleanup170ms/reaped/errors=[]，不能代判运行中Stop。下一覆盖样本应在冻结前按实测model/steering耗时给helper留足观察余量；不扩大产品硬预算，不改本次首败为PASS。
- 原audit17/21不改：计入未执行write提议、把wire turn_input_scope当纠正内容、对全部后续poll强制初始READY阶段的1000ms，均保留并以原参数/实际dispatch/steering_inputs另审；真实timeout及live Stop缺口仍在。运行审阅误把18bytes当字节首败已独立更正，原说明保留；自适应观测未记录初始start，真实owner receipt+CIM证明一次，不把缺观测当未启动。
- 零模型正式冷读，events/turns/sessions全部行数/hash及全部磁盘字节保持、纠正和正确结果/取消显示、无旧Turn复活。原AX文字审计7/8保留；移去浮层后截图明确“已取消执行”且AX有“停止前尚未完成”提示，独立视觉复核补足显示语义，没有声称原自动审计全绿。
- 未覆盖：C运行中Stop/必要N3/另一关键入口，A实际cwd/完整条目/报告首发与语言，B来源SHA/最终报告，Windows差异；共享未达。复用本批不重启/不重读/纠正和冷读证据，只补实际缺口，不重复未改根因。
- 证据：仓库外2026-10-02/windows/w246-mainline-live-process-context，01首红、02/03定向、04构建、05纠正时、06正式Stop、07/08原DB/事件/audit/实际dispatch与澄清、09暖UI/误判更正、10/11冷启动、12/13原冷读与补充截图、14物理deadline核对、15测试游标；Git仅数据保留、一个最小回归和短进度。

### W247 宿主收尾指令导致用户保持进程被取消（2026-10-02；基线4e07701ed，FAIL）

- Case/子断言：C05/C06/C07/C08与A05/A06/A09/A10/A13/A17/A18/A19；补live Stop覆盖，按W246实测冻结helper300秒/观察240秒，原任务约束和产品预算不变，复用正式构建。
- 正式COD一次、19步/八压缩，helper实际300000ms启动；来源各一次、纠正结果18-byte写入/回读且原件保持。但在操作者Stop前模型自行cancel一次，171ms/reaped/errors=[]，之后report一拒后接受、canonical completed；公开称“按系统要求cancel”。新输入造成的未执行write和旧report引用拒绝均保留，未重试模型或外部kill。
- 首败及根因：provider无tool的等待进度被当作完成复核，workflow completion_review_message明确要求“Before ending, poll or explicitly cancel running_processes; no process may survive this turn”，与用户保持到Stop矛盾。W248既有running-process回归强化后修前红；不是helper deadline、权限缺失或无法原生停止。
- 操作者停止前CIM已见父子均消失，点击先前Stop索引时回合已完成、按钮不可用，未实际停止，不生成伪Stop记录或运行依赖Stop成功的audit。原目标、旧审计与新运行前的审计语义修正均外置保留，原安全拒绝不删除；完整C仍FAIL。
- 未覆盖：修后停止保持语义/完整C/N3，A实际cwd/完整条目/报告与语言、B来源SHA/交付，Windows差异及共享门槛。下一批只修活进程等待被错误归入收尾的直接根因，普通完成/未知效果/取消/宿主清理底线保持。
- 证据：仓库外2026-10-02/windows/w247-mainline-live-stop，冻结期限/原与修正oracle、02纠正时、03/04原事件/DB/自cancel与completion_review、05原UI、CIM353样本和stop-not-executed；本批Git仅短进度。

### W248 活进程等待仍在执行，不能为收尾自取消（2026-10-02；基线e35980276，live Stop N1）

- Case/子断言：C05/C06/C07/C08与A05/A06/A09/A10/A13/A17/A18/A19；只修W247直接根因，同任务/helper300秒/观察240秒、正式Tauri/StepFun/COD。
- 修复：完成复核显式区分has_running_processes与can_report。活进程无tool的公开进度继续原执行，保留poll/原约束，不要求完成报告或把用户helper取消；宿主notice去掉“结束前明确cancel”的新指令，实际终态清理仍强制。无活进程的完成/未知patch/blocked分支、报告唯一表面、计数/权限/冻结预算不改。
- 验证：强化既有live-process回归修前红/修后绿；压缩后的复核、拒绝额外命令、报告修正不重做、两取消及blocked patch等七项不同定向通过，fmt/diff通过，正式构建49.56秒。中间把!can_report误等同活进程导致一项回归红，另记后显式拆开，原断言不改。
- 正式一次C、22步/八压缩，同Turn纠正，一helper/两来源各一次，无模型cancel/restart或额外进程；操作者Stop前CIM父子均存活，正式cancelled后约385ms均消失，419样本/亲缘/心跳停止/host_cleanup_proven与取消后零已派发观测成立。没有helper timeout、外部kill/Retry或虚报completed，关闭此live Stop子断言N1，W247自cancel首败保持。
- 整体C仍FAIL：一write因new-input/replan未派发，两次update_plan引用不逐字被拒；实际仅写一次/回读一次，但传入content已为C_FINAL_CORRECTED、缺末LF，真实磁盘17bytes，与用户18byte要求不符。原audit20/24及全部三次拒绝保留，不补写、不把结果改成PASS；只观察到工具参数阶段已缺LF，provider/codec归因未验。
- 零模型冷读三表所有行数/hash、全部文件字节保持，纠正/停前未完成提示保留，无旧Turn复活；错误17bytes也原样保留。原冷读6/8不覆盖：AX取消字样未匹配、期望LF仍缺，截图与停止提示另审，不宣称全绿。
- 未覆盖：C末LF/来源引用/必要N3/另一入口，A实际cwd/完整条目/报告首发/语言、B来源SHA/最终报告，Windows差异及共享门槛。复用本批停止/无重启重读/冷读，只修具体缺口，不重复未改根因或增加矩阵。
- 证据：仓库外2026-10-02/windows/w248-mainline-live-progress，01首红/03中间红/04七项、05构建、06纠正时、07/08原DB/事件/参数/audit/385ms复核、09正式取消UI、10～13冷启动/原冷读/原错字节；Git仅执行/收尾条件、一个既有回归强化与短进度。

### W249 被拒逐字引用的参数位置（2026-10-02；基线caf7e2e81，正式采用未触发）

- Case/子断言：C06/C07/C08、CTRL-006/007与A02/A05/A09/A16/A17/A19；W248三要求中第2项多加“核对”，反馈只给input 1，导致原错重复。先强化middle-source/完整计划保留反例，01首红保存。
- 修复：内部merge保留同一验证集合，同时返回被拒addition的数字JSON路径与来源索引；update_plan反馈给rejected_parameter_path=/requirements/1/source、source_input_index=1，原message/拒绝/计划版本/needs_replan/计数保持。旧merge的String错误API兼容，非引用失败不伪造位置；不回显引用/整段输入、不自动改quote、不接受改写或新权限。
- 验证：planning9、requirements6、报告修正不重做与mandatory硬预算各1，共17项不同定向通过，fmt/diff/正式构建2分38秒。原参数/既有计划及persist次数严格不变；只修第2来源后保留第1/3项与原输入义务，新显式坏引用仍拒绝。
- 正式一次原完整COD C，38步/15压缩，约239.114秒在冻结240秒观察限内正式Stop；helper一次、来源完整分页0→200→398（UTF-8安全返回198 bytes不是重读）/结果回读各一次，无自cancel/重启/额外进程，父子Stop前存活，canonical cancelled后CIM约536ms均消失，539样本及后续零派发/心跳/清理成立。
- 原audit21/24、三项字节失败保持：实际content仍为C_FINAL_CORRECTED，缺末LF，owner/磁盘17bytes，未补写。两update_plan省略requirements，无引用拒绝，所以不证明新位置反馈的模型采用或C首发/N3；两new-input/replan安全拒绝及计数原样保存。
- 零模型冷读三表所有行数/hash、全部文件字节不变无复活，纠正与17-byte结果保留；原6/8不覆盖，期待LF仍缺且AX取消字样未匹配。原ToolCallDelta只有空参数片段，无法与完成参数比较；只知工具参数阶段已缺LF，原HTTP/供应商/codec因果未验，不因计划文本提LF就归罪某层。review复制旧capture名首错已另存纠正，不计产品失败。
- 未覆盖：C末LF/引用反馈采用/必要N3/另一入口，A实际cwd/完整条目/报告首发/语言，B来源SHA/最终报告、Windows差异与共享门槛。本批仅一次新修复对应正式样本，不继续同一未改LF根因的付费循环；下一步集中原字节要求和A/B直接缺口。
- 证据：仓库外2026-10-02/windows/w249-mainline-citation-location，01首红/02修后/03十七项/04构建、05纠正时、06/07原DB/事件/参数/原21-24审计/536ms/空delta及澄清、08观察限、10～13冷读；Git仅位置反馈/最小回归与短进度。

### W250 主线B集中核账及包装工具摘要（2026-10-02；正式FAIL，组件修后PASS）

- Case/子断言：B、C04/C05/C06/C07/C08及A05/A08/A09/A10/A13/A17/A19；原完整任务、正式Tauri/StepFun/GEN、新workspace/profile，hash核对复用W249构建，无新模型配置或权限。
- 实际文件链：write/read/带真实整源SHA的patch各一次，原生cmd复制/移动及限定删除各一次，18-byte终版含LF、最后回读/hash及四保护文件保持。W240未使用SHA的首败不改，本次遵循不是未修根因的N3闭环。
- 首败：模型没有完成任何stdin写入调用便close，真实EOF及echo-input.bin均0 bytes；echo退出0/reaped不代判18字节任务。hold两次poll都cursor0而非第二次25，父子READY后取消一次/reaped，159ms及69条CIM证明亲缘/消失。来源/最后文件未重读，但未交付报告。
- 原audit20/28保留。20步/11压缩、三个model_response_rejected、一次output_truncated、TOOL_SHAPED_TEXT及REPLACEMENT_CONTEXT_BUDGET各一拒均保留；没有自行应用失败。预设240秒观察限，实际254.275秒才提交Stop，越限作为执行组织问题留档，不记限内PASS；canonical cancelled、host cleanup及后续零派发成立，真实UI已取消。
- 确定产品缺口：三条仅含tool_call包装的工具提议被接纳为摘要。现沿既有CompactionInvalidSummary拒绝这一个包装形态；普通文字/JSON历史与文档数据保持，原上下文、无工具摘要请求、一次纠正及硬预算保持。强化两个已有回归，真实223-byte反例首红单存；五项compaction和三项修复/源保持/硬包络检查共8项不同定向通过，fmt/diff通过。未证明该缺口导致漏stdin，也未代判修后正式采用。
- 收尾：正常同步a3a0bf001，保留macOS工作；本次正式构建不含新合并的应用退出码修复。A具体结果/引用语言、B输入/游标/交付、C末LF、摘要修后正式效果与必要N3/入口仍开放，完整场景0/3、共享未达；已集中列在页首，不扩旧矩阵或新业务Case。
- 合并验证：新退出码逻辑在Windows的两项process_exit回归通过（编译3分25秒），与本批8项共10项不同定向检查。只证明协调器失败/正常/重启码规则，不代判Windows正式应用退出；检查已结束，无后台cargo或模型任务遗留。
- 证据：仓库外2026-10-02/windows/w250-mainline-b-final保存原任务/基线/DB/事件/20-28及独立复核、首红/8项/Stop越限、正式取消UI；一次截图遮挡误取前景另存纠正，非产品失败。Git仅摘要判定、已有回归及短进度。

### W251 历史回执丢失与错误EOF交付（2026-10-02；组件修后PASS，正式仍FAIL）

- Case/子断言：C05/C06/C07/C08、历史真实性与A05/A09/A16/A17/A19；复用W250已取消会话及工作区，仅新增授权一次read_file和中文结果说明，不重做文件/进程。已有加密StepFun/GEN，独立profile，正式构建33.06秒。
- 首败：两步、UI17秒，一次真实read，当前18-byte/三行/完整SHA/LF交付成立；却把旧EOF说成18，实物/原close回执均0。原audit12/17保留；其误要求report_completion、漏自然文本的限制另审，独立12/13仅EOF失败，不用夹具限制洗掉真实错误。
- 零模型直接调用真实Runtime重放W250：仅两条消息、原ToolResult为0，错误“发送18字节”摘要仍在。确定数据保留缺口；现仅从匹配原生process准入及结果保留选定回执字段，互动须匹配process_id，经原tool_context策略，单记录2 KiB/整体连同标签4 KiB，超限整条省略并计数。仅补已移出原tool结果的历史数据，不生成工具交换、执行/当前证据或新权限；当前恢复前缀及隔离归档不补入。
- 验证：反例首红独存；历史5项、初始history预算/mandatory硬限/隐私reasoning/报告修正不重做4项，以及重复恢复/owner核对/不覆盖已完成结果3项，共12项不同定向和fmt/diff通过。真实旧回合修后重放为三消息、9回执/1省略，EOF0准确保留，错误摘要/原事件未改写。
- 修后只做一次原提示正式GEN，新profile及包含未提交修复的构建1分11秒（源hash/二进制hash已记）。一模型步、UI5秒、无新工具调用，仍复述EOF18并称本次读取；独立11/13，当前读取及EOF均FAIL，原文件/全部旧事件/Turn行保持。不得记修后模型采用、完整B或共享达标；没有再付费循环。
- 接线核对：API端口属于该新GUI PID，cargo产物路径/mtime/hash一致；同Session/Snapshot无清空或切换，初始history截断0，App使用32回合原生重放，编码保留User文本。尚未取得原始请求/HTTP，不能由源码和零模型证明实际wire收到0或归因供应商；真实采用和生成精度保持开放。Windows正式托盘退出未验，退出观察器到期没有杀进程，不代判正常退出。
- 证据：仓库外2026-10-02/windows/w251-report-closeout留前/后原文、DB/事件/原audit及独立复核、真实重放/首红/12项/构建及边界。诊断首次解析独立离线依赖锁、编译目录误落仓库，已校验路径后移出并留纠正记录；后续均仓库外，非产品失败。Git仅history源码、两项最小回归及短进度。
- 同步结算：源码c0baac7a8，合并3bc5a7fb6并保留macOS阶段记录；命令结果策略相关原回归在Windows再验1项通过，合计13项不同定向。正式样本为合并前构建，不移植macOS交付/文件短链结果或把新策略算本机live采用。

### W252 收敛方案与零模型请求边界（2026-10-02）

- Case/子断言：C06/C07/C08；复用W250和W251原记录，依正式Runtime闭合回合重放顺序构造历史，再用正式OpenAI Chat编码器检查请求body。9条源消息/9条编码消息，真实STDIN_EOF bytes=0逐字段保持，网络请求0、真实凭据读取0；没有新UI/模型样本或产品源码改动。
- 限制：这是记录重建及编码边界证明，未复原当时宿主instructions/tool定义，不能代判W251实际HTTP收到0、模型忽略正确数据或生成精度已修。W251首次/修后FAIL、当前读取缺失、原件及历史记录继续保持。
- 方案根因：整组复跑、生成误差与系统缺陷混作门槛、实际边界证据取得太晚，以及计划残留旧B/C/A/N3排程。现删除矛盾活动门槛，先零模型定位、修直接原因、最小回归及必要的一次短UI，交付和残余分开；不新增邻近Case或整组矩阵。
- 证据：仓库外2026-10-02/windows/w252-convergence-audit；探针首次ChatModelError类型转换编译失败保留，改为显式错误传播后完成。未读取/写回原数据库，未运行命令/进程/文件效果；Git仅计划与短进度，文档改动不重跑产品构建或全仓测试。

### W253 较早回执被后续回合压缩再次清掉（2026-10-02；确定性修复）

- Case/子断言：C06/C07/C08、历史真实性及A05/A09/A16/A17/A19；限定W251回执保留机制。合法零模型两回合反例：第一回合EOF0已有回执，第二回合压缩移除其tool尾部，整组重放又丢EOF0；也覆盖第一回合原ToolResult尚未压缩的情况。行为首红独存，夹具字段/事件顺序的两项首错分开，不算产品失败。
- 修复：新增整组闭合历史重放入口，暂存每回合从原准入/结果验证出的有界数据；所有原对话及未交付纠正重放完再附入，正式App改用该入口。原单回合入口兼容，原对话/事件不改写，历史数据不变成工具交换/当前证据或权限；原单条2 KiB/块4 KiB、宿主32回合窗口与原context预算保持。不同Turn可能复用call ID，不用全局同名ID排除旧回执。
- 验证：历史6项＋恢复/硬预算/报告不重做3项，共9项不同定向；App check、fmt/diff通过。后续回合不再清掉回执、未交付纠正只保留一次、错误后候选history完全不变、无新增tool交换。App check首轮1分17、最终8.83秒；没有全仓测试或正式构建。
- 复用真实W250/W251两回合经新入口及正式编码器：9条源/编码消息，EOF0保持，错误旧回答在索引6，原生历史数据在索引7、当前输入仍最后；网络/真实凭据读取0，前次输出不覆盖。数据排序是一项生成策略候选，不能证明W251原失败因果或模型采用；旧错误回答不删，当前读取/生成精度仍未解决。
- 未覆盖：本修复正式Tauri采用、实际过去HTTP、不同后续生成的语义正确性、Windows托盘退出。按停止线不新增模型循环/邻近Case，活动GUI仍为此前构建，不冒充新版本。本批证据仓库外2026-10-02/windows/w253-closed-history-receipts，Git仅三处接线/重放、一个最小回归及短进度。

### W254 历史真值正式短任务达标（2026-10-02；N1，原首败保留）

- Case/子断言：C05/C06/C07/C08、当前读取/历史结果真实性与A05/A09/A16/A17/A19；源b1c21a0a5，正式Tauri/custom-protocol构建44.19秒、独立profile，复用原W250隔离data/work/workspace及原只读提示，没有给模型补充EOF0答案或缩减断言。
- 正式GEN/已有加密StepFun：两个model步骤、一次实际压缩，UI18秒后completed；read_file全文一次、无工具/模型/摘要拒绝，无进程/命令/写入或补查。当前三行18-byte/LF及完整SHA与实际文件/回执相同；公开旧echo的EOF0、exit0/reaped及hold cancelled/159ms/清理，明确是历史事实、不升格为本次验证或18字节输入已完成。
- 独立13/13与人工语义/UI核对通过：原全部文件及文件集合、全部旧事件hash和旧Turn整行均保持；原cancelled未复活，错误旧回答仍可见，未点Stop/Retry或外部kill子进程。真实产物mtime/hash、源身份及新GUI PID已记录，正式截图与完整canonical备份仓库外；复用组件回归，不再跑整组B/C或固定N3。
- 只关闭这条真实历史结果保留/交付的针对性链N1，不声称唯一因果、原完整B/C、所有生成精度或共享全阶段通过。剩余仍为结果漏项/生成步骤字节偏差及Windows正式托盘退出等未验；过去实际HTTP未取得，不追加同根因付费循环。本批仅验证，无新产品源码。
- 证据：仓库外2026-10-02/windows/w254-native-history-final；复用已有prepare/launch/inspect/review，保留before DB/字节/事件基线、原提示、build/host、真实终态/13项/正文/UI。观察器只等自有GUI退出，观测到期不杀进程、不记正常退出PASS。

### W255 三项结果完整交付，核心机制阶段收尾（2026-10-02；N1）

- Case/子断言：C01/C02/C06/C08、CMD-131/134与A05/A09/A13/A16/A17/A19；针对已同步ace29cdd0命令事实策略补Windows采用。复用W254正式b1c21a0a5产物及隔离data/work/profile，在原A工作区新建COD Session，只要求cwd、直属含Hidden条目、中文空格文件首尾/字节/SHA三项，不附带Git/测试/搜索/进程链。
- 正式已有加密StepFun：三model步骤、零压缩、UI13秒completed；两个正确PowerShell命令各一次exit0/reaped，read_file全文一次，report首次接纳、三项各引用对应真实call，错误计数0/0。公开完整cwd、九条目逐项真实Hidden（仅.git/.hidden-item.txt为true，.hidden-note为false）、alpha/omega、43 bytes及完整SHA，中文回答与UI/canonical/磁盘相同。
- 13项独立检查及另外7项参数/Hidden/清理/首report核对共20项不同断言通过；所有文件及集合（含.git）、全部旧事件hash和Turn行不变，新Session不借旧结果。源/产物身份及before/after DB/UI留仓库外；无新构建或全仓测试，没有Stop/Retry/副作用或重复观察。原W245完整A交付4/6及首拒不覆盖，短链N1不拼成完整A PASS。
- 本轮机制交付收敛：W254历史真值与W255多结果交付缺口已针对性验证，其他未受影响的命令/非零/文件/Stop/冷读直接复用。仍开放W250漏stdin/重复cursor、W249漏末LF及W240缺来源SHA的生成精度；Windows托盘正式退出因UI观察器不可达未验。旧完整A/B/C、N3/100 seed/LONG/99%未认证，共享全阶段仍不记达标；没有新根因不再付费复跑。
- 正常fast-forward同步d2f9da387并保留macOS/MM工作；本次live二进制身份仍为b1c21a0a5，不代判新业务UI采用。证据外部2026-10-02/windows/w255-multi-result-final；Git仅简短进度和清理矛盾活动门槛，不扩业务Case或大型索引。
- 夹具收尾：全部canonical Turn终态后核对自有GUI PID/绝对路径/精确启动UTC，再停止空闲测试GUI；这不是正式托盘退出PASS。首次观察器DateTime转字符串损失时区/精度而拒绝清理，未杀进程；按反序列化类型核对UTC后完成，首错单存，无cargo或模型任务遗留。

### W256 stdin遗漏的零模型责任边界（2026-10-02；原因待证）

- Case/子断言：C05/C07/C08、原W250 stdin遗漏及A02/A05/A09/A19；只复用原DB/事件/磁盘，定位时间盒内结束，无新UI/模型调用或产品改动。
- 事实：close前第3次摘要明确18字节/append_newline=false尚待执行、process_id/游标11保留；第8步ModelResponseRejected无原生派发，第9步直接close，receipt和实物EOF均0。冻结Snapshot允许workspace.process/input，process_session绑定execute；排除本记录中“摘要已把输入标为完成”和“冻结Snapshot未准入input”，不代判实际请求工具表。
- 7项直接记录/实物核对通过；仅从ReasoningDelta抽出的XML写法经当前正式PublicOutputGuard在234个UTF-8两片分割位置均拒绝文本调用并保留write_process_stdin名称，正文未公开。该重建不是原始OutputTextDelta/SSE；原tool_hint空值的原因和实际请求暴露仍缺证，不能认定宿主守卫、供应商或已修摘要包装是唯一原因，也不把stdin残余关掉。
- 停止线：保留原FAIL，不新增猜测修复、补写字节、限制合法无输入EOF或重跑整组B。现有安全拒绝/清理机制继续复用；唯一原因未证，后续若取原现场须先有实际请求/拒绝帧边界证据。首个诊断中的同文件hash自比不算不变证明，原产物保留并改为独立18-byte要求/0-byte实物核对；没有读取凭据或写回原DB。
- 证据外部2026-10-02/windows/w256-stdin-attribution，完整提取/有限探针/首个核对及修正结果在仓库外；Git仅短进度。文档改动无需产品构建/全仓检查，共享全阶段及Windows退出仍未结案。

### W257 真实stdin请求/响应接合与短链N1（2026-10-02）

- Case/子断言：C05/C06/C07/C08、CMD-139/REAL-005及A04/A05/A09/A13/A16/A17/A19；限定18-byte stdin→EOF→exit真实接合缺口。b1c21a0a5正式Tauri产物复用，新data/work/profile、官方通用入口，已有加密StepFun连接；仓库外loopback录制夹具转发实际正文/SSE、不记录认证头，不替换模型或工具owner。
- 正式一次：六model步骤＋一次压缩，共7个真实请求，UI18秒completed；start/poll/input/close/poll/report严格一次链，同一turn-owned handle。input含末LF且append_newline=false；真实参数/磁盘/EOF bytes18及hex完全相同，poll推进0→11、最终用close返回119，exit0/reaped/errors空。report首次接纳，中文交付实际输入/EOF/退出/清理，无工具/协议错误或文件探测/附带进程。
- 19项独立oracle＋真实PID消失/68个日志及wire制品无凭据共21项不同断言通过；全部6个原生提议的真实SSE参数与canonical逐项相同，7个流完整留存，执行请求真实暴露stdin。helper/AGENTS字节及文件集合保持仅新增echo-input.bin；原W255备份与当前源data的三张canonical表整行一致。不是只读wire-only、组件拼接或普通直连网络拓扑认证。
- 夹具首败保留：两次复制旧DB而缺完整dataset身份均在模型前被正式启动门禁拒绝，未删凭据/改绑定/校验。改为正式初始化全新数据，只将所选加密连接在内存解密并以新夹具key加密导入，保留原数据；之前两次拒绝、一次观察器SQL引号错误/失效AX索引分别留档，模型调用仍只属于上述一次任务。初始化/最终自有GUI及forwarder均按PID/路径/启动UTC核对后清理，不算托盘退出PASS。
- 未覆盖：原W250被拒输出/tool_hint空值原因未复现，旧完整B、生成漏步骤/重复cursor、C末LF/来源前提、Windows正式托盘退出及发布认证仍开放。本次不改产品源码/预算/权限，不重跑整组或继续相同根因付费循环。证据外部2026-10-02/windows/w257-stdin-wire；Git仅短进度，无新构建/全仓测试。

### W258 核心机制与残余最终结算（2026-10-02；阶段交付）

- Case/子断言：C01～C08已有断言核账；读取12份独立产物，按当前源码差异判断指定操作证据可复用。共享页八簇表列清合法命令/读搜/Git小测试、源保护文件链、stdin/EOF/Stop、非零/timeout/历史真值、纠正冷读及原生参数回配；不将8行计成8个完整Case PASS。
- 剩余归类：W250漏stdin/重复cursor的唯一原因待证；W249原17-byte参数/owner实物一致，但Windows旧SSE缺失；W240用户要求的source guard未遵循与原复杂交付继续开放。没有新可修产品原因，不新增模型、夹具或正向样本消除旧失败；共享全阶段仍未达。
- Windows正式退出：b1c21a0a5正式二进制、原W257隔离data/work、新profile，模型0。三表整行hash/全部文件集合与字节/无活动Turn5/5，原记录无复活；自有8进程/5监听端口在观察前记录。window2 inventory及Win+B未提供可观察的Quit，已请求一次人工操作；15分钟内无实际Quit。原10分钟观察器超时后核对同PID仍活并续观察，未重启；到点仅清理空闲自有GUI，BLOCKED_UI_REACHABILITY保持，不把kill或组件退出码回归当native PASS。
- 交付：机制结算、生成残余归类、退出边界及最终记录四包已完成，源码修复和既有直接回归按原批交付。本批Git仅两页简短结算；无新源码、构建/套件、模型调用或角色/旧矩阵扩展。全角色、N3/100 seed/LONG/99%仍未认证，完整体验残余未完成，不标记全目标完成。
- 证据外部2026-10-02/windows/w258-core-delivery：选定断言/产物hash/源码影响审查、归类、冷读DB与5项、正式UI/进程/监听及退出超时/边界。首个核账脚本误选W255集合字段另存后改为原automated_checks，断言/原产物不变。后续正式退出需要可观察托盘的环境或重新准备后人工操作；已结束实例的迟到回复不当作Quit证明。

### W259 开放项有限定位与人工托盘Quit（2026-10-02；正常退出N1）

- Case/子断言：C05/C07/C08、A05/A09/A13/A17/A19；从干净且远端一致的fb89666b9继续，仅定位开放参数/上下文和正常正式退出，不重复W258机制核账或整组A/B/C。无源码修复、Cargo/模型调用或新正向命令样本。
- W250重复cursor新边界：原seq528 poll返回25；seq564压缩摘要明确cursor=25，并保留同call原工具回执的next_cursor=25；seq570响应拒绝后，seq576完成提议仍请求同进程cursor=0。因此该次压缩记录没有完全丢失游标；参数层重复及原FAIL保持。实际历史HTTP/SSE未恢复，不能断言供应商或解码层唯一原因。漏stdin复用W256缺证边界，不重新执行探针或用W257短链覆盖。
- W249实际写入的完成参数与owner仍为17-byte无LF，旧Windows SSE缺失；W240实际第二次write_file整写替代要求的来源SHA保护，合法schema不把自然语言义务自动变成全局强制guard。原复杂任务漏项与W251失实EOF分别保留，既有数据保留修复/W254/W255指定链复用。没有新可修产品根因，不补LF、stdin、cursor或强制所有patch带SHA。
- 正式Quit：当前window2仍无托盘目标，用户明确提供人工协助后才准备新profile；复用已正式初始化的W257隔离data/work，不克隆旧DB或改身份。二进制SHA及前端build ID与b1c21a0a5原产物相同，当前退出链源码未漂移。新PID43056、启动UTC13:38:56.1249366Z，正式窗口及8进程/5监听先记录；用户确认实际托盘“退出／Quit”。观察器保留同一Process handle，13:39:34.7459333Z自然退出、实际exit0、未kill，五端口消失。首快照两个WebView后代仍活，退出后33.4秒的复核8个PID全无，窗口消失；三表整行hash、全部工作区文件和无活动Turn五项成立。原stderr的WebView class unregister Error1412与首快照原样保留。
- 验证与未覆盖：只关闭空闲隔离实例正常托盘Quit N1；W258原PID21636阻断/清理记录不改。失败退出码分支与活动Turn应用退出未做native验证，既有协调器组件证据复用；生成残余、原完整共享/Windows Case与N3/100 seed/LONG/99%仍未认证。自有GUI/后代/监听已自然清理，退出观察器已结束。
- 证据：仓库外2026-10-02/windows/w259-open-items-closeout，open-items-review.json记录原seq/参数与源hash；01-native-quit含身份、UI、实际退出码、两次进程快照、原记录核对及quit-result.json。Git仅本次简短结算，文档变更无需产品构建或renderer检查。

### W260 当前共享Creative在Windows正式采用（2026-10-02；全面走查进行中）

- Case/子断言：C06/C07/C08、AMUL-001、OBS-008/014/020与A06/A09/A10/A12/A17/A19；补后续Creative pause/retry在Windows产物中的实际采用。新目标覆盖其余共享/Windows验证，本批不代判整体完成，原生成残余及旧MM失败保持。
- 验证产物：源9118d3fab，34项Creative UI/port回归、desktop-ui-boundary、当前前端、正式`cargo build -p nomifun-desktop --features tauri/custom-protocol`及现有conversation_gui_fixture均通过。正式binary SHA前缀1CE1C5342933、frontend_build_id=4839fca4-409b-4553-a8bf-c89651730f21；新data/work由正式bootstrap初始化、新profile，不克隆用户DB或改身份。
- 暂停正式N1：本地HTTP400仅一次，canonical native_pause=EXECUTION_MODEL_INVALID_REQUEST、cleanup_proven；UI显示“执行已暂停”、无运行spinner/retry、pending保留。正式Stop后同Turn cancelled、pending清除、UI“已停止”；28events、0effects、画布0节点/连接、本地调用仍1。用户托盘Quit，PID73680实际exit0，最终后代/监听消失，退出前后四表整行hash保持。
- 实际retry正式N1：debug既有注入只匹配新夹具生成的精确key，真实鉴权提交成功后丢一次返回；UI出现“提交结果尚未确认”和实际retry按钮。正式点击后两次submit均200，仍仅原一个Turn/operation/key、本地模型仅1、0effects；释放原响应后30events/completed，pending清除、正式UI完整回复、空画布保持。PID70280用户Quit实际exit0；新PID46044同data/work、新profile冷读原画布，原30events/四表逐行hash及回复保持、无retry/重发/复活，本地调用仍1；用户Quit实际exit0，最终自有进程/监听全部消失，fixture正常shutdown/exit0。
- 首错与边界：夹具构建未完成时误启动旧helper，unsupported mode/exit1，发生在模型/产品启动前，原样保留后增加外部runner产物hash门禁；一次输入几何失败/错误前景截图与用户输入检测另留档，错误截图排除。未修改产品源码；付费/真实StepFun请求0，本地模型共2。仅关闭上述隔离negative子断言N1，不关闭原Windows旧MM/N3/全MM、真实供应商生成、W250/W249/W240、复杂任务、引用反馈采用或其余平台/发布认证。
- 证据外部2026-10-02/windows/w260-creative-adoption：构建/34回归/边界、原首错、三个GUI身份/UI/原生退出、真实点击的两submit日志、原DB/事件/完整表hash及review.json。当前无Cargo、测试GUI、fixture或其监听遗留。下一步真实模型统一使用用户指定StepFun Coding Plan / step-3.7-flash；跨盘/真实UNC缺隔离夹具保持未验，全面目标继续。

### W261 精确引用反馈真实模型采用尝试（2026-10-02；未验证/生成首败保留）

- Case/子断言：C05/C06/C07/C08、CTRL-006/007与A01/A05/A09/A13/A17/A19；目标为W249中间新增来源引用的真实拒绝→按参数位置纠正，尚未取得该断言。源/产物复用W260，正式Coding UI、独立新data/work/profile、已有加密StepFun Coding Plan / step-3.7-flash。所选连接在内存重加密导入，未复制旧DB/身份；实际loopback请求/SSE不记录认证头。
- 第一载体未达：两次实际HTTP200，初始工具表无update_plan，ToolSearch仅一次返回无匹配；模型如实说明不可用，未启动进程/副作用。源码configure_tools仅在adaptive.task_ledger激活后暴露控制，夹具错误要求首次调用未暴露控制；不归因供应商漏工具，也不扩产品工具表。修正载体为先执行实际已暴露进程动作，第一原记录保留，同PID/同Session未重启。
- 修正后的正式首败：start/poll各一次，真实READY_CITATION/cursor15；模型未收到追加输入便自行cancel一次、172ms/reaped/errors空并report收尾，违背保持运行/不自取消要求。操作者观察到READY后未在模型结束前提交steering，新增input1/目标引用拒绝均未发生；原生cancel参数与canonical一致，旧StepFun拒绝/旧Windows LF的HTTP仍未恢复。report披露引用测试unverified，所谓按回合终止规则取消的解释与实际请求08的明确等待/不取消notice矛盾。该notice原文、进程身份及游标都在真实wire中，不能归为W247已修的宿主强制cancel或丢失回执，也不把canonical completed计成任务PASS。
- 核对：合计9真实请求/SSE全部完成、5组原生调用参数与canonical逐项相同；工作区两文件完整bytes/hash/集合保持，helper实际PID76260消失，凭据审计70份文本制品0命中。无新明确产品根因/源码修复，不再同根因付费循环；精确引用反馈采用、原W250/W249/W240及复杂任务仍开放。
- 夹具/观察首错：初始化已有无关local provider，导入旧整数PK导致冲突，三次模型前失败/rollback保留，外部import改为目标生成内部PK且保留provider UUID/model/role。一次SSE choices=[]观察器IndexError、错误前景截图、目录弹窗控件缓存缺失/人工目录选择单列；后续优先自行操作，真正不可达入口保留未验继续其他项。15分钟GUI观察超时后核对同PID39400/启动UTC并续观察，未重启；两Turn均终态/helper已清理后才核对身份清理空闲GUI和forwarder，未计正式Quit。全部自有PID/监听无遗留。
- 证据外部2026-10-02/windows/w261-citation-adoption：正式初始化/连接导入与首错、两载体原提示/实际DB/UI、9份新真实wire、review.json/secret-audit/最终清理。Git仅简短失败与边界；全面目标保持active，下一步继续Windows生命周期及其他可执行平台/共享余项，未验证项不计PASS。

### W262 复杂交付采用与摘要可用空间修复（2026-10-03；机制修复，整任务未通过）

- Case/子断言：C01/C02/C03/C06/C07/C08、CTRL/OBS/REAL交付与A05/A09/A13/A17/A19；同步25c94acd8复杂交付/私有回放后，Windows Runtime270、私有回放1项通过，正式构建源a2c96fd70。原五项只读任务、中文空格/真实Hidden/Git/指定0、1测试及保护原件断言保留，已有加密StepFun Coding Plan / step-3.7-flash、新workspace/profile、正式Tauri入口。
- 首败：15真实请求均HTTP200/完整SSE，15组完成参数与canonical相同；实际cwd/目录各一次、全文一次、搜索/Git各两项、两测试各一次exit0/1，后3提议被replan拒绝、没有重放。8model步骤/5压缩后冻结替换包络失败，bytes84595/tokens28199/limit28160，原上下文保持、host cleanup成立；无最终交付，原FAIL不改。
- 有据产品原因与修复：首次摘要1462bytes符合产品1536byte提示，却无法与固定前缀同置；原提示仅按输出上限计算，未扣已选必需状态/输入/摘要wrapper。现根据完整冻结替换候选计算提示可用量，按JSON最大转义成本预留；token/byte/message上限、原始输入/回执/工具授权及一次纠正保持。短预算纠正不再被128byte下限反向抬高；实际超限摘要仍严格拒绝，无截字节或补输出。新反例修前红/修后绿，ASCII/转义及原状态保持断言；首次扩大到全部Runtime发现额外约束历史字节数造成3项旧回归失败，已改为冻结包络约束，未改原断言，最终271/271通过。正式修复构建通过，无renderer变更。
- 修后正式一次仍未达：原任务/新workspace，同16窗口/产品硬限，夹具预设24请求/240秒观察。24实际上游全部HTTP200/完整SSE，12组完成参数与canonical相同；必要操作各一次、两测试exit0/1，文件/HEAD/status/diff保持；本夹具未单存index原始字节基线，不计独立index-byte断言。最终交付前触及本地录制器上限，后3请求未派发上游；出现EXECUTION_MODEL_RATE_LIMITED暂停来源是夹具429，不归因StepFun限流。操作者超过240秒才结束回合，组织首错保持、不计限内PASS；正式cancelled，无最终report/五项交付，整任务及稳定性继续开放，不新增未改根因付费循环。
- 自主操作/退出：复用既有WebView2 CDP烟测接口，仅控制自有真实Tauri页面；测试profile书签作夹具初始化，再实际点击项目/官方Coding/输入/发送，冻结binding实际cwd与新workspace一致。其他bak/mobile实例保留；按名称阻断与静态配置推断的首错留档，实际内核互斥键未占用后才启动自有实例。两自有GUI在任务终态后调用已有Tauri process.exit API，分别PID13060/16448实际exit0、后代/监听最终清理；这是原生API退出，未冒称托盘操作。首次15分钟观察超时后核对同PID/启动UTC再续观察，无重启或kill GUI。
- 证据外部2026-10-03/windows/w262-shared-delivery-adoption：原/修后真实wire、失败UI/DB、初次回归红与中间3红、最终271/构建身份、review-corrected.json/实际派发核对/保护与清理。首个review误将无JSON的replan拒绝计成派发，原样保存后按completion_observation.invocation_attempted纠正。尚未关闭原复杂交付、W249反馈、W250/W240/生成字节、活动Turn Quit/失败退出及其余Windows/共享/发布认证，全面目标active。

### W263 真实Windows代码页管道与生产解码器（2026-10-03；native子断言通过）

- Case/子断言：WIN-012、PROC-020，复用未变生产nomi-process-runtime库，仓库外单独编译探针、自有实际子进程双pipe；只读GetACP=936，不更改OS/控制台设置。stdout真实ACP中文5bytes、stderr UTF-8含emoji16bytes逐字节读取，按实际顺序进入生产OutputBuffer；两stream raw/text完全匹配、总游标21/mixed、8byte环精确丢13bytes，空当前游标/eviction仍保存lifetime metadata，child exit0并消失。
- 无产品源码修复。decode_errors=1表示首次严格UTF-8失败后选择有效ACP的既有诊断，文本未丢失；探针初始误假设计数0的失败保留，按既有计数语义核对，未放宽任何字节/文本断言。首编译误调用supervisor-private finalize亦保留，后仅用导出API及完整字符/EOF。不是完整Supervisor/renderer验收或全部代码页；证据外部2026-10-03/windows/w263-native-encoding，Git仅本条短进度。

### W264 活动回合与清理失败的正式原生退出（2026-10-03；限定N1）

- 子断言：Desktop退出协调器的活动Turn/失败码、失败后冷恢复安全；不计完整WIN-016、活动托盘点击或全失败矩阵。当前源52bdf3ee8正式custom-protocol构建和既有conversation_gui_fixture构建均成功，binary SHA前缀2ED7321143A3，frontend_build_id=4839fca4-409b-4553-a8bf-c89651730f21。正式初始化两个新data/work、三个新profile；真实Tauri输入与已有process.exit API均自主操作，没有新增退出入口。
- 正常活动退出12/12：本地模型第二SSE保持、Turn running、唯一write_file returned，实物73bytes。PID2220请求code0后自然exit0，同Turn仅一次cancelled、65条原event逐项保持，最终71条；原文件精确字节/hash不变、调用仍2、stream释放，后代/监听全部消失。
- 真实失败退出10/10：新PID50812同样唯一写入后，外部SQLite BEGIN IMMEDIATE保持title=title的可回滚写锁。请求code0，日志明确清理失败/保留authority/有界重试耗尽，113.812秒后实际exit1；锁一直保持到终态后才rollback。原65events/returned回执/73bytes保持、调用仍2、stream释放；数据库仍running，如实保留未证清理，不改成功。退出瞬间2个WebView残余后自然消失，最终后代/监听0。
- 同数据冷启动安全9/9：释放锁后新PID50304按既有策略恢复原Turn，未新增用户输入/Turn、未重放write effect，原65events/文件保持。第三个本地模型响应复用旧tool-call identity被正确拒绝，同Turn failed、最终90events；UI显示上游错误/retry，未点击retry，恢复完成不计PASS。随后原生API自然exit0，90events/文件保持、后代/监听最终0，fixture正常shutdown/exit0。
- 无产品源码修复、付费/StepFun请求0，复用已验正常空闲托盘Quit与退出码组件规则。外部2026-10-03/windows/w264-active-native-quit保存构建身份、三个UI/实际退出、原/冷DB、日志与31项限定断言；首次观察器误读整个work中活跃锁文件的PermissionError保留，后仅核对指定文件，不声称整个work字节不变。78份文本凭据审计0命中；全部自有GUI/fixture/锁/监听、Cargo已结束。复杂交付、W250/W249/W240/引用采用及其余平台/发布缺口仍开放，全面目标active。

### W265 固定说明去重的Windows正式采用（2026-10-03；五项最终交付未达）

- 子断言：88ab60155固定工作区说明去重采用与复杂交付，原五项只读提示/文件/测试全部保留，仅一次正式COD/StepFun Coding Plan、step-3.7-flash。源1d3712209当前前端构建成功；首个正式Cargo在nomifun-db rustc中STATUS_ILLEGAL_INSTRUCTION/exit101、无源码诊断，首错保留，同源同命令重编及初始化helper均exit0。binary SHA前缀B24066F375C5、前端e247ffd0-e4df-4804-98ec-d4e5453b9fad，与实际Tauri页面1280×832/嵌入manifest一致。
- 新data/work正式bootstrap、0本地模型、正常shutdown后仅内存重加密导入既有选定连接；新项目/profile、书签夹具后实际UI选择/输入/发送均自主操作。真实首请求的root说明仅一处instructions、tool description重复0，未更改schema/工具权限或产品硬预算。W264同源合并后的Runtime281/native_pause13/desktop-ui-boundary直接复用。
- 独立核对16/16：实际cwd及完整9条/真实Hidden、完整43bytes/原SHA、两个限定搜索、Git status/diff、指定Bun两测试按序各一次exit0/1，四个command均reaped；无重复观察，所有非Git文件/HEAD/status/diff与原始index字节保持。10组原生完成参数与canonical相同，其中9个实际owner派发、1个本地update_plan。6模型步/4次压缩；没有report_completion或最终五项交付，不把实际操作/去重/安全检查计整任务PASS。
- 正式首败：12真实HTTP200，11完整SSE；第11摘要请求max_tokens4096、提示204 UTF-8 bytes，供应商实际finish_reason=length、completion_tokens4096、15309bytes reasoning_content且正文0。产品对同源只做一次更短纠正，第12未完成的SSE在Stop后中断；无录制器cap429，不归因旧W250/W249缺失wire，也不同于W262合规摘要无法装入的原根因。自动观察器120秒到限，正式Stop前观测120.214秒（0.214秒界面/截图延迟，如实记录，不计严格限内完成），同Turn cancelled、无重新派发/实例重启。
- 推理配置核对：摘要clone沿用原路由，真实wire无reasoning_effort，原能力traits=[]。官方[Step 3.7说明](https://platform.stepfun.com/docs/zh/guides/models/step-3.7-flash)把low列为摘要用途，但尚未证明修改该冻结配置能修复本次失败；不盲加none/enable_thinking、扩大输出限或再付费循环。产品唯一根因/复杂最终交付继续开放，未新增源码修复。
- PID46748任务终态后已有原生API自然exit0，退出瞬间2个WebView随后自然消失、最终后代/监听0，原记录不变；这是API退出，未计新托盘点击。forwarder按路径与启动UTC ticks核对后清理；首个ISO字符串与PowerShell自动DateTime比较误拒清理保留，实际身份未变。93份文本凭据审计0命中，bootstrap/watchdog/GUI/forwarder/自有Cargo全部结束。外部2026-10-03/windows/w265-context-delivery-adoption保存首编译/实际身份、新wire/原参数/DB/UI/summary-wire-analysis/review；全面目标active，原失败和平台/发布缺口保持。

### W266 过期进程的Windows owner真实终态（2026-10-03；pipe/ConPTY native子断言）

- 新共享c58378d25修复的实际Unix回归为cfg(unix)，Windows缺覆盖；本批源cc6a32494只新增两个Windows测试，生产执行代码不改。用实际Bun子进程分别走pipe与ConPTY，保持原1000ms期限；原owner已完成而scope尚无terminal缓存，直接查询必须不消费cursor，随后stdin/close_stdin/resize均返回原timed_out/reaped、PROCESS_ALREADY_TERMINATED、success=false/control_applied=false，未重启或重放输入。
- `cargo test -p nomifun-app --lib --no-default-features engine_process_host::tests -- --test-threads=1 --nocapture` 11/11通过，其中新增两项native。pipe精确READY+LF/游标6，ConPTY保留原ANSI/CRLF及游标194；三份迟到控制output逐字段相同，cursor0完整回放等于start原片段+冻结终态片段，没有ECHO或迟到输入标记，scope quiescent/cleanup成立。pipe实际terminate/reaped155ms、ConPTY interrupt/reaped171ms，errors空；不是通用ConPTY EOF通过。Runtime关于拒绝控制不推进epoch但接收实际timeout的284回归按未变源码复用。
- 首错仅新测试Vec类型缺注释导致E0282，产品/子进程尚未执行；日志保留，补显式serde_json::Value后原断言不变、首次native全部通过。没有模型、正式UI或全仓套件；W250旧漏stdin/游标、原B和真实模型对新终态的采用、完整ConPTY/Windows专项与发布认证仍开放。外部2026-10-03/windows/w266-expired-owner-windows保存首编译、11回归和两个完整native receipt/review；自有Cargo/子进程已结束。

### W267 普通用户管理员查询的原生权限边界（2026-10-03；WIN-013子断言）

- 文件deny-write/deny-delete、共享锁原件/暂存保护、DACL/ADS和部分失败fence已有W03/W34/W35及W36正式预览证据，相关文件执行层未变，本次直接复用；不重复文件链/锁矩阵。新增缺口是标准Windows token运行管理员查询的owner结果，仅补一个显式opt-in环境回归；不修改生产执行层，不触发UAC/自动提权、不启动或改系统服务。
- 先只读确认当前token非管理员、LanmanServer已运行；新回归再次确认同样前提，否则明确失败，默认ignored防止管理员CI假通过。实际App EngineProcessScope只运行一次net.exe/args=[session]，返回exited、exit2、success=false、OS错误5/拒绝访问，cleanup.reaped=true/errors空；不是PROCESS_NOT_STARTED或平台不可用，没有第二session/权限回退，scope quiescent/cleanup成立。
- 当前源4e951dc82加测试的`nomifun-app --lib --no-default-features`精确ignored回归1/1通过。完整34bytes未丢失，真实Windows-936/严格UTF8失败后fallback诊断1保留；不改代码页或错误断言。模型/正式UI0、无全仓套件；本批只新增环境受限测试。外部2026-10-03/windows/w267-windows-permission-boundary保存前提/实际日志/native receipt/review，全部自有进程结束。实际UI呈现、其他管理员操作与完整权限/角色/Windows认证仍未验，原生成与复杂交付残余不改PASS，全面目标active。

### W268 正式进程树故障样本与观察首败（2026-10-03；WIN-016未验）

- 目标为实际Tauri父/子/孙在运行时强杀GUI并同数据冷启动，未取得此断言：两个故障观察器均在强杀前失败，不计强杀/冷恢复PASS。源da8bd70a0正式custom-protocol构建成功，binary SHA前缀D98DD201566C、前端e247ffd0-e4df-4804-98ec-d4e5453b9fad。只新增既有conversation_gui_fixture的crash-tree模式，生产执行/退出入口不改；真实新库、新work/profile、正式UI输入均自主操作，本地模型4次/付费0。
- 初始化两次首败分别RESOURCE_SELECTION_UNUSED和RESOURCE_SELECTION_REQUIRED/process_session，均模型/GUI前exit1；原库/日志保持，按现有合同声明files/read、process/start及managed-process-session，未伪造authority/移除校验。新helper实际创建一次Bun三层树、原30000ms期限不变；准备脚本只作夹具，模型不创建文件。冷推理受控400分支用于防止夹具重复提议，未实际进入冷启动。
- 第一正式样本取得canonical READY；观察器比较正斜杠/Windows路径误拒同一PID71964，启动UTC ticks/实际完整路径复核相同，未执行kill，原树随后按期限结束。观察器路径比较改为GetFullPath后一次新隔离样本PID40436，当前canonical仍有READY，但重用的find_tree.py固定指向首个Session，15秒观测失败；再次未kill/未重启/未延长寿命。两次观察组织失败原样保留，本问题定位时间盒已停止，不能靠下一批重置继续重复。
- 正式异常另存：两例出现EXECUTION_CLEANUP_UNPROVEN/cleanup_proven=false，而现场原树已无活动PID；没有据此把清理改成功。实际UI“结束本回合”后同Turn cancelled、唯一start effect仍returned；已有原生API请求code0最终各exit1，日志保留process_owner/prior_turn_settlement未证及有界重试耗尽。缺精确原owner为何不能给出cleanup witness的唯一根因，后续须从这些原记录定位，不宣称已修或把组件PASS覆盖。
- 两个GUI/fixture/watchers/原Bun树及监听均已结束。完整证据外部2026-10-03/windows/w268-formal-crash-restart含两个初始化失败、两个正式原DB/UI/参数/真实退出和review；无强杀意图文件、无冷重启制品。最终helper修正崩溃等待文案并编译通过，未再次正式运行；原运行文案“文件已写入”是夹具复用失实，未计文件效果。WIN-016、真实UI进程清理原因及其余原生成/复杂/平台认证仍开放，全面目标active。

#### W268 原owner终态见证在lease退休后丢失（2026-10-03；有据机制修复）

- 复用原数据定位：原日志只保留process_owner阶段，底层错误全文缺失，不能重建为历史真值。代码显示EngineProcessSession只持ID/PID/cursor，原30秒进程的idle lease为90秒，registry退休会移除已reaped记录；未缓存terminal的scope再cancel便无法取回原证明。原两例暂停发生在此时间窗之后；这是机制对应线索，不冒充原历史唯一原因。
- 新原生反例先红：实际短命令exit0/reaped、完整输出已冻结，测试使用公开policy的2秒lease加速退休，registry确实移除后原Engine owner清理报session_not_found。修复保留同一次admission绑定的只读终态数据见证，仅原cleanup.reaped=true才用于原owner结算；expired写入仍拒绝，观测不消费cursor、不恢复取消/续lease权限、不改生产期限/lease/预算、不按PID存在与否猜结果。
- 首实现误持整个Session，扩大回归命中原evicted-session生命周期断言；首失败保留后改为仅共享SessionState中的终态/输出/cleanup数据，进程/Job/Session仍按原规则回收，原断言不改。新反例转绿、原Session回收通过，Process Runtime131/132；剩余real_start_pending_admission_cannot_cross_quiesce的物理marker超时在仓库外未修改HEAD基线同样失败，独立记录，不计绿或放宽等待。Engine Core39/39通过、1项原ignored保持。
- 当前生产代码与必要回归三文件；正式desktop/helper构建成功，桌面SHA前缀3EC4C51952C6、前端身份仍e247ffd0，未改renderer。制品继续同W268根保存06首红、07首绿、08错误持有Session、12最终Runtime与14/15当前/原基线失败、16Core及17/18构建身份。没有新正式UI/模型或WIN-016重跑；W268修复后的实际退出采用、原历史唯一原因、强杀/冷重启与全部生成/平台认证仍未验，全面目标active。

#### W268 修后lease退休窗口的正式退出采用（2026-10-03；限定N1）

- 只补6c3fdb6e1对应正式采用，未重跑原强杀样本。夹具新增显式lease-retirement模式，原crash-tree默认30000ms不变；新用户输入/实际原生start参数均声明1000ms期限，生产lease仍按原规则=期限+60秒、sweep30秒，未扩生命周期/模型预算或修改退出入口。新data/work/profile正式初始化，完整同一回合留到100.544秒，超过61秒lease及一轮sweep且在120秒观察内；不是原30秒故障样本或WIN-016认证。
- 当前源9250009b4正式桌面与helper构建成功，binary SHA前缀B9757B30E417、前端e247ffd0-e4df-4804-98ec-d4e5453b9fad。正式UI输入一次；原start回执有真实三层树READY/PID、唯一effect returned、本地请求2/第二SSE保持，原树按期限结束无重启。100.544秒时同Turn仍running、native_pause=null；自主调用已有Tauri process.exit(0)，PID70300实际exit0，同Turn仅一次cancelled、无cleanup_unproven暂停，原65events保持/最终71events、唯一effect/915bytes脚本保持，SSE释放、调用仍2，最终后代/监听0。
- 独立14/14限定核对、退出后3表冷读保持；fixture正常shutdown/exit0，观察器全部结束。付费0、无新生产修复/全仓套件，上一批Core/Runtime/baseline失败证据复用；源码只新增明确短期测试模式。制品仍同W268根04-lease-fixed保存前/后实际UI/DB、原参数、100秒观察、actual exit及review。此N1关闭当前短期限lease退休退出采用；原两例的历史唯一原因、原30秒参数同场景复验、强杀/冷启动、实际托盘点击、原生成/复杂和完整平台/发布边界仍开放，全面目标active。

#### W268 当前正式强杀与冷隔离恢复（2026-10-03；应用故障N1及warmup阻断修复）

- 复用已核对的B9757B30E417当前生产产物；9250009b4至2f13e28f8仅helper/进度变化。观察器明确传本次run路径并用GetFullPath/启动UTC ticks核对；原30000ms/原PIDs/原失败均保留。新正式PID77676在canonical READY、真实三层父子关系/路径/出生UTC、running Turn及第二本地SSE保持后，仅对精确GUI PID作Stop-Process故障注入。实际GUI exit -1、非Quit；原parent61308/child38860/grandchild73956的已持有句柄55/56/56ms终态，树/后代/监听最终0，无替换进程。
- 第一同data/work、新profile冷PID20144没有新模型/effect/输入或进程，但运行状态未隔离；记录两次replay boundary拒绝后，9秒UI warmup创建空闲runtime，恢复循环以get_runtime存在为由退出，原running卡住。原Checkpoint之后有工具事件，现有合同应拒绝自动回放；fence预验证使用next_fence，未删/改Checkpoint、fence、lease或放宽tail校验。
- 明确产品修复：调度入口和重试循环改用已有active_turn_generation判定本地实际driver/relay admission，空闲warmup不再阻断第三次拒绝隔离，真正active owner仍优先。现有真实warmup不产生admission/明确Turn才绑定generation两项回归通过，当前正式构建通过（新binary SHA前缀71FCB2A445DD）。按路径/UTC核对后清理空闲旧冷实例用于源码升级，旧观察器到期不是退出证据；未因此重启原任务或改原库身份。
- 修后同一原data/work、新profile PID50532：新构建与原Checkpoint不兼容时仍正确拒绝，第三次失败在warmup之后持久化NATIVE_RECOVERY_RECONCILIATION_REQUIRED、cleanup_proven=false/暂停，不假称已恢复完成。原65events完整保持，唯一start effect returned/915bytes脚本、模型调用仍2、原树无复活；实际UI结束本回合后同Turn仅一次cancelled、最终68events，原生API自然exit0，最终后代/监听0。控制HTTP400后续模型分支未派发，本批没有供应商模型请求。
- 独立16/16限定检查、two warmup回归/正式构建，原同代码冷失败与两个强杀组织首错不改PASS。制品同W268根05-crash-fixed/06-cold-crash/07-cold-fixed含实际故障意图/退出/原句柄与55ms观察、原/修后DB/UI、warmup因果日志及crash-cold-review。只关闭应用强杀树清理与受控冷隔离安全N1；OS重启、sleep/wake、成功自动续做、原完整WIN-016/所有平台/发布认证仍未验。全部自有GUI/fixture/进程/监听清理，全面目标active。

### W269 精确来源拒绝反馈的真实载体首败（2026-10-03；采用未验）

- 目标为实际StepFun在input1中间requirement的错引用拒绝→按字段反馈纠正，未取得该断言。当前f64db6365正式desktop/helper构建、新库正式bootstrap后内存重加密导入既有StepFun Coding Plan / step-3.7-flash、新workspace/profile；真实Tauri当前manifest/1280×832核对，UI项目/COD/原始输入均自主操作。初始两小文件仅作只读载体，不重跑原A/B/C。
- 观察首错：首请求录制后只等待真实UI追加input1接受再原样转发，路由/焦点过渡期间实际文本断言失败，控制器未点击发送，也无turn/steer-accepted；首个控件值/焦点快照未保存，具体差异不重建。30秒gate到限，本地503/未派发上游，随后网络重试派发；属于夹具行为，不归因StepFun。保留原error/请求及组织失败，同时间盒不再付费循环。
- 实际后续3个上游HTTP200/完整SSE、两read及一个report共3组完成参数与canonical相同，文件集合/全SHA保持；3模型步、约49秒completed只对应初始载体。input1/目标update_plan/引用拒绝与纠正均0，不把它计采用PASS；原要求等待补充前勿结束仍生成提前收尾残余，非产品丢失已接受input1。W249/W250/W240旧FAIL和缺失历史wire保持。
- 没有生产代码修复/额外测试或第二付费样本。实际SDK请求退出后PID6752/后代消失、forwarder按路径/UTC ticks清理；原10分钟观察器已到期，未在退出前保留新OS句柄，实际退出码缺证，不把SDK code0或超时计Quit通过，也没有重启任务。51份文本凭据审计0命中。制品外部2026-10-03/windows/w269-real-citation-feedback保存新真实wire、原DB/UI/控件首败、review及退出缺证；全部自有进程/监听结束，精确反馈采用与原体验/平台认证继续开放，全面目标active。

### W270 cmd持久/后台命令的owner边界（2026-10-03；原生子断言）

- WIN-009/011相关：当前合同不按`/k`或`start`名称统一拒绝；只验证受管持久命令遵守原deadline、已就绪后台子进程不能越过成功回执。没有证据支持新增语法禁令，无生产代码修改。
- 新增两项App原生回归：`cmd /d /k`保持running、原1000ms截止后timed_out/success=false/reaped=true，游标0→18→65、清理141ms；`cmd /c start "" /b`后台Bun先写实际PID并由第二进程确认就绪，返回exit0/reaped时独立OS查询证明子进程已消失。范围内断言通过；既有pipe/ConPTY两项同组通过，标准token测试本次ignored，不重复记PASS。
- 两次测试首败保留：先将有新输出的poll误当终态，再在补正循环中重复operation ID被正确拒绝；最终只修观察游标/调用ID，原截止时间和终态/清理断言不放宽。01/02日志均3通过、1失败、1ignored；03仅重跑纠正后的持久命令1通过。后台子进程独立查询隐藏窗口，无付费模型/正式UI。
- 原生证据基线b63c51a10；正常同步远端083ac0531的Browser storage-close入口后，受影响的Browser屏障两项及App存储关闭后重试一项通过，不代判新的Windows正式UI采用。制品外部`2026-10-03/windows/w270-cmd-owner-boundary`保存首败、完成参数/回执、原生结果及同步检查。该项原定位已超30分钟时间盒，停止扩展，只完成已有测试纠错与提交收尾，不另开批次重置。完整WIN-009 policy拒绝、脱离变体、正式UI及WIN-011全边界仍未验；历史生成失败、引用采用、sleep/wake、第二卷/真实UNC与发布认证继续开放。已受管PID和自有Cargo/GUI均无遗留。

### W271 混合代码页的真实supervisor及App回执（2026-10-03；WIN-012接合子断言）

- W263库级raw/decoder证据复用，本批仅补真实受管reader→终态冻结→工具回执缺口。只读GetACP=936，不更改宿主设置；真实Bun双pipe分别写ACP936中文5bytes和UTF-8中文/emoji16bytes，DBCS/emoji跨实际read分段。两项opt-in回归显式执行均通过，其他代码页不计PASS；无产品根因或生产代码修改。
- Runtime回归覆盖21/8bytes两种保留合同：真实stdout读取完成才放行stderr，逐stream原始字节完全匹配，总cursor21；小ring精确丢13bytes、文本仍正确，空终态快照保持mixed/decode_errors=1及原保留/丢失计数。诊断1为预期ACP fallback；两个原PID23072/54628均以预持OS句柄确认exit0/reaped，shutdown精确。
- App原生exec实得21bytes/mixed/完整中文及emoji，当前cursor21空poll仍保持原终态和全部元数据；scope quiescent/cleanup通过。新增仅测试及Windows测试所需Globalization feature，未修改renderer；无正式UI/模型/SSE，不代判模型报告或完整WIN-012/全部平台认证。制品外部`2026-10-03/windows/w271-mixed-stream-supervisor`保存原始回执/分流bytes、终态及构建结果，自有Cargo/GUI/原writer均无遗留。
- 当前computer-use应用/窗口两次清单均没有托盘目标；活动托盘Quit仍待人工可用条件，尚未准备新实例，不复用旧PID或改产品入口。W249/W250旧wire缺证、W240提议偏差、精确引用真实采用及原复杂交付继续开放，全面目标active。
- WIN-015仅完成只读夹具预检：宿主支持S3、当前交流电且AC允许wake timer，绝对时间timer/resume=true实际arm成功/last_error=0，随即cancel并close。原token的SeShutdownPrivilege存在但Disabled；实际系统sleep/wake尚未执行，不计Case通过，不改电源/安全设置。预检JSON随同制品保存，后续仍需实际电源周期及同一owner/句柄的终态证据。

### W272 实际S3与原owner归约（2026-10-03；WIN-015部分通过、首败保留）

- 仓库外驱动链接当前be2bdd246的生产ProcessSupervisor；初次SDK编译错误保留，修正驱动签名/枚举及输入分帧后构建通过，无产品源码修改。一次实际SetSuspendState(false,false,false)，UTC绝对wake timer；仅测试进程临时启用已有SeShutdownPrivilege并恢复原值，不改系统电源/安全设置，无正式Tauri/UI/模型请求。
- 实际墙钟65.501s、排除休眠计时3.673s，LastSleep/LastWake改变；系统Kernel-Power42及Power-Troubleshooter1均Target/EffectiveState=4，WakeTimerOwner精确指向本次驱动，证实实际S3及自有timer唤醒。SetSuspendState返回true；保存的last_error=1300不当作成功调用的失败证据。40秒设定不等于实测65.5秒，完整原日志保留。
- pipe/ConPTY分别原10s deadline、15s lease：休眠前READY及原父子OS句柄存活，唤醒后两者均TimedOut/reaped，清理404/420ms；原句柄均终止、registry退休后迟到write均SessionNotFound，无输入应用/进程重放。仅这两个受管原生归约子断言通过，W266/W268机制直接复用。
- 交互ConPTY无deadline/expire_on_idle=false、10s lease，唤醒后同一父子句柄仍活，原write(AFTER_REAL_S3_ONCE加LF)返回Ok并被console回显；5秒内应用S3_ACK缺失，因此该子断言FAIL，驱动实际exit2，不把整个WIN-015计PASS。LF/cooked-input夹具原因尚未用零休眠对照证明，唯一原因缺证；原父子随后cancel/reaped、171ms清理。定位已超过30分钟，停止对照/再次休眠，不靠新批次重置时间盒。
- 外部`2026-10-03/windows/w272-real-s3`含完整驱动/源码二进制身份、一次物理电源intent/返回/独立事件、原owner/句柄和三个结果、exit2及cleanup。六个原PID均无遗留，timer取消、测试进程token恢复；自有Cargo/GUI/驱动均结束。交互确认、正式UI采用、完整WIN-015及其余生成/平台/发布边界仍开放，全面目标active。

#### W269 控制前提核对：普通排队发送不等于即时追加（2026-10-03；供应商0请求）

- 恢复目标后先前“托盘/夹具阻断全部工作”的判断纠正；当前Tauri窗口可自主操作。正常同步cca92ee93的共享历史失败归属修复，desktop/helper正式构建成功，前端e247ffd0/1280×832核对；新库正式初始化后内存重加密导入所选StepFun连接，03-bootstrap自然exit0/模型0，不复制旧库或身份凭据。
- 同W269根08-control-premise保存组织前提失败：LF比较/焦点与实际文本核对通过，但发送前完整控件快照同时有“发送”和“立刻发送（插入正在生成的回复）”，控制器错误选择前者，仅形成UI排队；canonical steer-accepted=0、要求两个来源输入的前提不成立。此前“必须等待模型边界”的推断撤回；当前可证原因是选错按钮，未证产品丢失接纳输入。前向门所有请求均需接纳证据，30s后本地400暂停，当前1份原始请求保存、上游HTTP/SSE均0；未把本地失败归因供应商，也未启动第二模型试验。
- 已将仓库外控制器限定为实际即时追加按钮，修正后的接纳/引用拒绝/采用仍未执行，不计PASS。两carrier文件集合/SHA保持，原Turn通过实际UI结束为cancelled一次。PID58636持续预持句柄证明已有原生API自然exit0、后代/监听0；不是托盘Quit，原观察器不再因10分钟到期丢句柄。forwarder按路径/启动UTC精确清理、steerer结束，37份文本凭据审计0命中。
- 无本地生产修复；同步历史closed-turn/current-reader两个定向回归通过，原W269首败及本次组织失败保持。仅当前正式GUI控制前提变化，原W249/W250/W240、真实引用采用和全部认证仍开放；不把控制器核对算作模型修复。本项超过原时间盒，停止进一步定位/试验，完整制品保留上述同目录，全面目标active。

### W273 Step5接入及精确来源反馈的真实采用（2026-10-03；机制子断言通过、严格整链FAIL）

- 用户建议后复用Windows既有加密Coding Plan连接，单次无副作用工具探针HTTP200，requested/served均step-5-preview、ready=true/268tokens；不保存凭据或思考正文。隔离新库正式初始化/所选连接内存重加密，登记新模型context_limit=1000000、output_limit=4096及reasoning_effort=low，不改源连接/全局默认、进程期限或清理保护。当前a10933b1b正式desktop/helper构建通过，UI实际通用入口/模型/项目自主操作，前端e247ffd0/1280×832。
- 正确选择实际“立刻发送（插入正在生成的回复）”，同Turn原始input0及canonical input1各一次；预算事件实为1000000/4096，非旧32k未知模型回退。7个真实HTTP200/完整SSE均Step5/low，8组原生提议参数与canonical完全相同；首个旧read_scope提议因新输入到达未派发，模型先replan再执行，a/b实际各读一次，无写/命令，集合与SHA保持。只证明预算声明与真实路由应用，不是完整1M-token负载认证。
- 指定反馈采用已验：首次两requirement提议的第二项new_scope使用input1配input0标记整行，被正确拒绝，精确rejected_parameter_path=/requirements/1/source、source_input_index=1；下一真实调用保持input1，quote改为当前输入准确原文，原有效old_reads完整保持，plan_revision2/updated。独立review八项定向检查均成立，关闭该精准字段反馈→真实模型修正的采用缺口，W249/W250旧缺证与其他生成精度不代判。
- 严格整链保留FAIL：首项引用虽属于input0却未用指定标记行；纠正还修改new_scope.description，非仅quote变化；模型随后调用report_completion并completed，summary却称等待用户结束。当前工具描述明确validated report为terminal，产品按真实提议执行，无已证执行层根因，不自动保留运行或更改原断言凑PASS。完整共享/Windows及原A/B/C未完成，生成遵循与报告准确性继续开放。
- PID6504原生API自然exit0、预持OS句柄/后代监听0，非托盘点击；forwarder按原路径/启动UTC清理，steerer及Cargo结束。77份文本凭据审计0命中。完整制品外部`2026-10-03/windows/w273-step5-access`含单次访问探针、正式源/二进制身份、预算/接纳/UI、真实wire与八组对账、原拒绝/纠正及严格三项FAIL，未追加第二付费试验或源码修改。全面目标active。

### W274 真实运行中人工托盘Quit（2026-10-03；退出入口N1通过）

- PROC-040/LIFE-029、Windows活动Turn退出：首例PID17940实际人工托盘exit0，但冷读证明14:03:32已因第二模型请求120秒超时暂停，早于14:05:21退出；三层树也已到原30秒期限。该例只证明暂停后Quit，不计运行中树清理通过，原前提失败完整保留。
- 只修正式测试夹具：新增`--active-quit`，单次本地SSE完整返回受管`exec_command`/600000ms，执行期间没有第二悬挂模型流；后续调用明确400禁止替换树。原`--crash-tree`/lease期限不改，退出入口/执行层无生产修改。针对夹具一项回归1/1、helper构建通过（SHA前缀736D8E145E85）；复用a10933b1b正式desktop/SHA7E9231B394AD、前端e247ffd0/1280×832，后续本地差异仅夹具/文档。
- 第二新库/隔离work/profile PID64760、14:19:47启动；退出前实际UI一条运行中命令，canonical Turn running/pause0/tool_started1/tool_finished0，独立OS快照证明78632→75392→8076三层树及原路径/出生UTC。用户实际选托盘Quit，原持有OS句柄记录14:23:12.158实际exit0，没有原生API退出、窗口关闭或观察器kill。
- 同Turn仅一次cancelled，无paused/completed；原exec回执cancelled/success=false、reaped=true/errors空、清理162ms，非退出失败。模型调用仍1/等待流0、付费0，原初始事件及活动前缀逐行保持、915 bytes脚本/SHA不变，后代/监听0；fixture自然exit0。独立限定核对16/16，原通用review未计算的false占位另由`verified-live-review.json`明确解释，不改首例失败。独立核对脚本首次误读output层级失败保留，纠正观察器后读取同一库，无任务重跑。
- 完整制品外部`2026-10-03/windows/w274-active-tray-quit`含两例原UI/DB、前提失败、人工确认、实际退出码/进程身份及冷核对。关闭当前正式运行中托盘入口和活树清理缺口，复用W264真实写锁失败exit1；不代判其他退出故障分支、OS重启、完整WIN平台/共享生成/发布认证。所有自有GUI/helper/树/Cargo已结束，全面目标仍active。
- 正常合入远端386ba1e08/3f144f2c3/506ac463d，合并39f795f05；Windows配置冻结/下一Turn刷新/实时安全检查3/3、实际路由模型切换集成1/1、闭Turn引用导入1/1通过，没有重跑其他已验链。该同步不把a109正式样本身份改成合并后产物，也不代判新的配置切换正式UI或macOS退出专项。65份外部文本凭据审计0命中，后续本地测试日志未使用真实供应商凭据。

### W275 Windows literal argv与真实PowerShell管道（2026-10-03；原生子断言）

- WIN-002/008相关：文件尾点/保留名/ADS/普通workspace拒绝UNC、大小写、ACL/共享锁已有W01-B/W03/W34/W35/W36原生或正式证据，执行层未变，直接复用。第二卷/真实授权UNC仍缺夹具；本批不重跑文件链/权限矩阵。
- 加强两个现有Runtime原生回归，无生产修改：直接Program复制helper至中文/emoji/空格/单引号exe路径，同形cwd；原两段误编码夹具文本改为明确中文/emoji，argv含`$(exit 99)`、反引号、`| & ;`、双引号和尾反斜杠，env含字面`$env:USERPROFILE`及`$()`。helper独立回显的四个UTF-8字节长度/精确值相同，exit0、reaped/errors空及shutdown exact；不发生shell求值。
- 显式PowerShell Shell新增真实成功/抛错管道：Single-quoted反引号与`$()`逐字输出``literal `$() piped\r\n``，exit0；ForEach-Object抛错保留PIPELINE_FAILED、exit1。原五个native/恢复/非零分支保留7/0/7/1/1；七次均reaped/errors空及shutdown exact。实际宿主PowerShell5.1.26100.8875，两个定向回归首次2/2通过，不把八进程数当八Case。
- 基线3c4943a44加测试，测试binary SHA前缀6027B8C063E9；外部`2026-10-03/windows/w275-shell-literal-boundary`保留两份实际日志、patch/身份/独立核对。八个记录PID最终无匹配、隔离helper/cwd已随TempDir清理；模型/正式GUI0、无Cargo遗留。只补当前实机literal/显式管道子断言，不计完整WIN-008、App/Tauri实际采用、跨盘/UNC、S3交互、复杂生成或发布认证通过，全面目标active。

### W276 当前共享配置修复的Windows正式采用首败（2026-10-03；未验、原实例失败退出）

- C07/C08当前Turn参数/预算冻结及下一空闲Turn刷新：505ee1eb1正式desktop/helper构建通过，binary SHA前缀DF03D4978CCC、前端e247ffd0/1280×832；新库官方初始化/独立work/profile，本地响应载体替换已自然退出的初始化helper的同一loopback端口，未改连接/凭据身份。目标为同Turn UI追加输入后仍旧值、下一Turn用新值，实际两断言未到达，不计正式采用通过。
- 两个组织首错保留：helper Cargo仍活动时过早初始化，被binary身份门禁拒绝，DB/GUI/模型未启动；首请求观察仍返回活动exec句柄时错误推进新配置。旧65536/2048/low与新1000000/4096/high均经正式PUT API保存，但不存在目标请求或预算事件，不能从全局配置落盘断言当前Turn已冻结。未复用旧DB、删除身份保护或重启原任务。
- 原PID2668/15:07:54实例一直保留，15:09:53输入已canonical接受，运行至操作结束前仍running/5events、execution-claimed/预算/tool/effect事件0、目标模型夹具调用0；已有原始参数/DB/UI可追溯。对“warmup后同模型编辑沿用旧Snapshot”的临时公开API COD载体1/1通过，未复现本例chat.minimal；不据此修改缓存/冻结/执行层，不由其他载体绿反推正式通过。临时诊断patch/日志仅仓库外保存，定位安排已超30分钟，停止扩展，不新建批次重置。
- 同GUI真实UI Stop后仅一次cancelled，最终6events，原初始3行及停止前5行逐字保持。随后已有原生API请求exit0，但原OS句柄实际exit1/15:29:03；日志为一个Runtime未证明shutdown、owned_runtime_teardown保留quarantine，4次有界退出重试后应用失败退出。非托盘点击、非观察器kill，不把最终PID消失计normal Quit通过；该边界原因仍未证，W274原活树托盘exit0证据不改写。
- 外部`2026-10-03/windows/w276-turn-config-formal`保存首错、正式身份/输入/UI、两次配置保存、空目标请求事实、原及最终事件、临时反假设回归、actual exit1/清理日志及独立review。初始化helper/model夹具自然exit0，全部自有GUI/模型/Cargo/后代/监听0，付费0；当前正式配置采用、接受后推理前停滞及未证Runtime清理继续OPEN，唯一产品根因未定位，不追加模型/源码修复凑PASS，全面目标active。
- 收尾正常快进远端6eceb3754/d99ef2149，已结算patch跨模型数据兼容及原exact来源/Session/未决状态的三个Windows回归3/3通过；本例无patch/tool事件，不据此宣布停滞已修。正式产物仍明确属于505ee1eb1，合并后未追加UI；44份外部文本的既有真实加密连接凭据审计0命中，无凭据入Git/日志/提示。Git仅本批短结算。
- 首次push遇并行远端更新而被正常拒绝，随后正常合入eb45556ce明确寻址历史正文；Windows Runtime引用5/5及App历史固定root/范围6/6通过，实际模型采用/原复杂报告不由这些组件通过代判，未重跑本例。无force-push、共享历史改写或覆盖他人源码。

### W277 原W250关闭结果的当前正文投影（2026-10-03；组件通过、真实采用未验）

- C06/C07/C08历史数据链：针对eb45556ce/fd3305422已变更的正文投递/操作记录优先级，复用原W250的06-cancelled canonical制品，只读导出同一关闭Turn的原typed事件，不恢复或构造缺失HTTP/SSE，不调用原owner。763条canonical/463 typed事件、16条step>0已结算结果；原receipt仍cancelled/原错误标记0。host观察另计，不合成缺失结果或将提议当执行。
- 显式跑既有owned-journal ignored回归首次1/1；旧3aad3949b完整payload投影36223bytes/16条通过。正常同步ad333dc76/3c41d3bf5后复用并加强新紧凑正文回归1/1：选中/完整结果正文16/16、遗漏0、serialized26986 bytes<=原64KiB；正文/错误/通过sources还原的原binding/turn及原插入顺序逐项相同，Assistant/Text/current_evidence=false/new_user_instruction=false，未寻址不投递。planning/control提议参数明确仅在投影省略，原archive精确READ仍逐字完整；不把它们计作已投递参数，不扩大预算或放宽原body断言。
- 独立原事实核对：真实close结果仍STDIN_EOF/bytes0/hex空；echo退出和后续hold取消仍reaped，旧漏stdin/重复cursor首败不变。原DB SHA前后均2af42c806004e9148d85f268c4ce3f8e1c351b2d4e6960346edfa24e2b451777，原件只读/无重放，模型/正式UI/owner调用0；此组件数据投影不能代判真实请求包含全部正文、模型报告正确或完整B通过。
- 制品外部`2026-10-03/windows/w277-historical-body-current`保存机械导出/来源SHA/原process事实、同一回归各源码阶段日志与独立review，不计多个独立样本。同步准备时本地reverse patch未适用、merge被正确阻止，首错另留；仅撤销自有旧测试段后正常同步，未覆盖远端。当前archive七项常规回归通过，owned历史项已另外显式跑通（默认组ignored不再记未跑）。自有Cargo已结束/无新GUI或进程，当前正文的Windows正式采用仍未验；W276推理前停滞/未证清理、生成步骤/字节/完整交付及跨盘/UNC/S3余项保持OPEN，全面目标active。

### W279 全局D流ConPTY应用尺寸（2026-10-03；原生子断言通过）

- WIN-010/W107实际应用尺寸缺口：全局A规则审查、B正式产物准备、C恢复载体准备独立并行，D仅拥有PTY测试与helper候选，主agent审核/应用并独占Cargo精确运行。无生产代码修改，不重跑S3/过期控制或通用EOF。
- helper从原附属CONOUT$经GetConsoleScreenBufferInfo独立报告实际窗口/缓冲，非复述resize返回：原PID54724初始80×24，原owner resize后同PID/started_at保持、实际132×43。原raw输出/游标完整拼接至563、dropped0；ConPTY重绘旧initial行不当重启/helper重复执行。
- 本次取消interrupt/terminate、force=false、reaped=true/errors空、cleanup1.1674505s；原OS句柄终态/shutdown exact，预热后的宿主handle数131→131。新增一个明确环境opt-in回归，首次显式1/1通过，其他八项filtered不计通过。原PID及自有Cargo已核对结束，无模型/正式UI/DB改动。
- 制品外部`2026-10-03/windows/global-d-conpty-size`含隔离候选/基线SHA、实际native日志和范围核对。只关闭此次应用实际尺寸与同owner生命周期子断言，不计完整WIN-010、S3 ACK、完整Windows或发布认证；C成功冷续做与B正式采用仍在各自队列，W276/缺卷UNC和生成余项不改PASS。

### 全局C流成功冷恢复载体准备（2026-10-03；正式恢复未验）

- 与A装配审查、B产物/原件保护和D实机尺寸独立准备，仅新增conversation_gui_fixture的`--success-recovery`。唯一write returned后第二请求只SSE注释，无语义结果；旧等待流实际释放后才可arm，本地后续fresh replan/read/close/report，意外请求400且绝不再write。原模式不改，无生产恢复/lease/checkpoint/fence更改。
- 主agent审核并跑fixture合同3/3：新mode禁止活流arm/finish释放、fresh IDs和重复写入；旧active-quit/creative拒绝两项同组保持。仅夹具合同PASS，不代判用户任务完成或成功自动恢复。
- 正式采样仍须唯一write/实物26 bytes、最新生产checkpoint确在结算之后且tail仅下一model_step_started、pending/unknown0，以及实际lease_until+20秒装入固定120秒观察边界。条件不成立不kill；同binary/data/work、新profile、生产lease自然到期，不SQL改期限、原记录或权限。C占下一个正式UI槽，当前未启动该GUI/模型。
- 仓库外`2026-10-03/windows/global-c-success-recovery`保留独立候选/只读采样说明和3/3日志；实际故障/自动续做/无重放及正式退出均未验。W276/S3时间盒不重开，旧强杀清理/受控隔离/正常托盘Quit直接复用，不计新增恢复Case通过。
