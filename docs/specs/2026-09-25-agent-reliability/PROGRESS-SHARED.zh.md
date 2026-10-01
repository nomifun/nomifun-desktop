# 命令与会话可靠性共享进度

更新：2026-10-01。调度规则见 [实施计划](IMPLEMENTATION-PLAN.zh.md)。
2026-09-30按用户明确目的收敛为[八个命令问题簇、三组正式会话](IMPLEMENTATION-PLAN.zh.md)。
本轮处理简单命令、步骤衔接、过程状态及结果可信性；通用/编程入口各保留真实执行证据。
旧675共享＋82 Windows全产品队列停止排程，原目录/首败/已修代码保留；范围外、手测和复用分开记录。
核心共同链路达标后单独通知，再补其中尚缺的Windows命令/native断言，不以全产品发布认证阻断收尾。

## 当前活动问题簇

| 簇 | 可复用基线 | 下一缺口 |
| --- | --- | --- |
| C01 命令选择与参数 | W73/W92；W184相对cwd/PowerShell；W185系统CMD单脚本文本引号；MAC-C01-01宿主示例/字面恢复 | GEN/COD原生CMD已各有样本；综合A其他普通命令与macOS首发/N3待验 |
| C02 读取与搜索 | 工作区读搜、路径/输出回归 | 综合场景A的中文/空格、零匹配与只读结果 |
| C03 Git观察与小测试 | W86；W184正式GEN的git_diff path=.及定向测试结果 | 综合A尚有未闭合调用；不因小复验通过结案整组 |
| C04 文件与步骤结果 | 既有File/Artifact根因修复及定向回归 | 综合场景B的实际步骤、字节/hash与最终回答 |
| C05 进程与停止 | W105、process/PTY/cancel/host清理回归；W194输入/EOF；W195取消提示；W196正确游标和Stop5秒N1；W197准入归因 | B完整文件步骤/N3、其他连续首发与等待播报 |
| C06 过程与交付真实性 | W93/W94；W184业务退出码；W185未执行/完成历史；W187非零直接报告；W197本地配置拒绝冷读 | 失败/未验披露及连续任务重复/误报；新拒绝端到端、A/B/C仍待验 |
| C07 连续会话与纠正 | W95/W98；W182取消冷读；W185完成历史读取及同步后一次实压缩；744ca440b保留已结算检查 | 最新用户纠正、较长连续任务及完整C场景 |
| C08 模型协议接合 | W99/W100；W184 GEN；W185 GEN/COD真实StepFun→CMD/控制引用owner | 各入口定向小样本已有；完整连续命令与N3门槛未达 |

本表只列候选复用与缺口；是否受新代码影响需核对具体基线，不把源码测试存在当已跑通过。
本轮结案按A/B/C正式任务、核心bad-case闭环及独立结果，不再按原Case×角色×OS计算完成率。

### C04-01 / C05-04 精确末尾 LF 与模型参数（2026-10-01，macOS MAC-B-01；正式修后待验）

- 正式 COD/StepFun，16 requests / 531 events / 一轮压缩，文件创建/精确 patch/copy/move/read/
  hash/delete 及 managed start/poll/input/close/poll 已实际执行，原件和操作次数保持。独立严格
  首败：模型 content 少一个末尾 LF，stdin 的 input 自带 LF 又 append_newline=true，得到两个
  LF；owner 依照实际参数，原观察/hash/原生 ECHO/独立 helper receipt 相互一致，非平台改字节。
- 最终 completed 报告只披露 31-byte hash 并称调用均成功，未识别要求的 32-byte 内容/单 LF。
  不能把进程 exit0/reaped、正确复制和限定删除或后续说明测试当整个任务完成；旧首败不覆盖。
- 仅改善现有 write_file / stdin 字段说明与 JSON 例子，不补写文件末尾、去重输入、扩大权限、
  放宽字节断言或加入新控制表单。复用 C05-03 的模型说明投影，扩展限定 content/input/
  append_newline description；canonical 注册合同与贡献指纹不改，default/required/范围/新字段
  均不复制。input 已带 LF 且 append=true 仍合法，owner 回归继续要求两个 LF。
- 最小说明/真实 App 投影首败保留；修后 Runtime 13/13，App newline/owner bytes/budget 3/3、
  既有 cursor/期限投影 2/2；
  这是接口说明与合同不变的确定性证据，尚未证明真实模型修后遵循。Mac 锁定阻断终态 UI/
  Cmd-Q，owned TERM0 只作清理。完整 B/长 helper/GEN/N3 和公开语言仍待验，不改 Windows 结果。
  完整参数、首败、模型轨迹、源字节、WAL-aware 531-row 快照在
  `2026-10-01/macos/mac-b-files-stdin/`，未再调用模型或扩预算。

### C05-01 进程总期限提示与正式停止（2026-10-01，W194）

- 正式GEN/StepFun的hold首发省略timeout_ms，默认30秒后timed_out并清理；轮询不会续期。
  工具此前未说明该总期限，现start说明与timeout_ms元数据明示默认30000、最大600000及超时/用户停止的区别。
  未改变owner默认值、硬限、参数必填/接受集合或取消策略。只有提示根因已修，不声称模型等待合同全部达标。
- 正式echo链只启动一次：start/poll/stdin/close/poll/report；UTF-8中文加LF共16 bytes与实际文件一致，EOF/exit0/清理正确。
  后续真实UI停止前独立证明父子均存活、父子关系正确；停止后canonical cancelled/host_cleanup_proven、两个PID消失、心跳停止，echo结果保留。
- 首次30秒失败、第二次操作者未及时点击导致120秒先超时均保留，不能算用户停止成功。第三次明确600000的有界窗口仅用于允许用户操作，未扩产品硬限。
- 仍有真实缺口：模型遗漏cursor，wait_ms=30000仍反复读相同READY输出，39秒内14步/12次poll/一次压缩；取消卡片下还显示此前“进程仍在运行”的模型文本。
  停止独立oracle 15/16：机器a11文本未含截图上可见的“已取消执行”，该失败原样保留，视觉结果不覆盖机器断言。
  正式UI清理5秒时限、完整B/N3、超时后完成复核的计划恢复及macOS仍待验；不关闭共享阶段。
  Windows既有native stdin/后代清理2/2、工具生命周期/宿主schema2/2、fmt/diff通过；证据在2026-10-01/windows/w194-process-stdio-stop。

### C05-02 取消前文字与重复轮询（2026-10-01，W195；UI子根因已验，模型等待仍失败）

- 已取消卡片下的最后一段模型进度文字可能还说“进程仍在运行”，容易被当成当前状态。
  现仅在同Turn的已关闭取消记录且仍有回复文本时标明“下方是停止前尚未完成的回复”；
  原文字、已完成效果、复制/展开、canonical事件及错误仍保留，运行/完成/失败与无回复的取消不加此提示。
- UI最小反例修前失败，修后76/76；类型/i18n/桌面边界及正式构建通过。正式Tauri冷读旧取消及新取消均看到提示，
  独立旧事件digest、原件hash、早先echo和停止后父子/心跳核对通过。新正式任务总体12/15，仍FAIL。
- 首败仍在：新Turn先poll上回合process_id被owner明确拒绝；随后只start一次，但27次poll仍无cursor，29步/四次压缩。
  已停止并保留完整现场，不因UI提示或后代最终清理把模型等待任务记PASS，不追加模型循环掩盖失败。
- poll说明已明示next_cursor→cursor、旧输出会立即返回、只报告一次准备状态。代码复核发现App装配以canonical schema替换展示层属性；
  因此总期限/cursor/wait说明还补入wave2 canonical schema。默认值、必填字段、接受集合及owner回放行为不变；
  canonical3/3与展示schema admission subset1/1通过。该补充在上述正式构建之后，实际模型是否正确等待仍待重验。
- 未覆盖：完整B/C/N3、新Turn正确首发、游标连续等待、正式UI清理5秒时限、timeout完成恢复及macOS。
  证据在2026-10-01/windows/w195-poll-cursor-cancelled-reply；只关闭取消前文字未标注的UI子根因。

### C05-03 提示投影与冻结合同兼容（2026-10-01，W196）

- W195把说明补入canonical schema后，workspace.process贡献指纹变化；本批正式旧Session在模型/工具前被准确拒绝为provenance drift。
  此回归由上一批提示落点不当引入，首败保留：0模型步/0工具/0命令，不能归因于StepFun或用刷新授权绕过。
- 现canonical process_schema恢复与43cd16bee完全相同；App装配只在模型展示定义投影timeout_ms/cursor/wait_ms的description。
  不复制default/required/范围/新字段，不改注册指纹、Snapshot/贡献锁、准入或owner回放语义。说明有了正式入口且旧冻结合同保持有效。
- App说明丢失的最小断言先红后绿，新增2/2；Runtime canonical admission subset1/1、fmt/diff及正式构建通过。
  原Session经正式UI重试恢复：先start一次、首poll cursor0，后续cursor25/wait30000，READY只读一次；7模型步/一次实际压缩/6次poll，零工具结果错误。
  最后一poll随UI Stop中断，无伪造结果；canonical cancelled与host_cleanup_proven、独立父子/心跳/原件/原echo及旧事件检查均通过。
  点击前父子实际存活，点击起1,208.76ms内CIM确认两PID消失；独立oracle21/21，只记此GEN N1。
- 未覆盖：N3/其他入口、完整B/C、其他历史合同、timeout后完成恢复；等待期间仍有重复播报/内部游标，准入失败UI仍误归“上游Agent或模型服务商”。
  不由本次N1关闭共享阶段。原失败/新结果在2026-10-01/windows/w196-owned-poll-context分别保存，W195记录不改写为成功。

### C06-04 本地配置拒绝与错误归属（2026-10-01，W197）

- W196真实合同漂移在本地Kernel准入阶段被拒，UI却写“上游Agent或模型服务商出错”。KernelError被压成普通Conflict，
  派发失败又把已有结构化错误重建成Conflict，丢失了原code/归属/恢复建议；首个拒绝与错误标签继续保留。
- Capability/Skill的ProvenanceDrift现生成专门本地类型，HTTP409/NOMIFUN_SESSION_CONFIGURATION_CHANGED，归属nomifun、禁止原样重试、建议新会话。
  其他Kernel拒绝不借此归为配置变化；新的分类依赖类型，不从供应商诊断里的同名文字推断本地来源。
  派发失败持久化保留既有结构化错误；不可恢复原生owner用已有NATIVE_RECOVERY任务未完成分类，仍须核对先前效果。
- 旧UNKNOWN_UPSTREAM仅对精确本地准入漂移签名做只读UI兼容；错误状态、原诊断、身份和canonical原件不改。
  正式Tauri冷读W196：显示“会话配置已变更”、应用归属和新会话建议，可展开原详情，不给盲重试；独立oracle15/15，旧事件集合/内容和磁盘结果全同，新增模型步0。
- UI最小反例首败保留；修后21/21（含HTTP平铺/嵌套新码一致的禁止重试建议），错误分类43/43、App类型/冷读2/2、类型/i18n/桌面边界、fmt/diff及正式构建通过。
  不以冷读证明新拒绝全链路或N3；其他本地拒绝、provider故障正式UI、其他平台及完整A/B/C仍待验。
  证据在2026-10-01/windows/w197-local-admission-error，原W196失败没有被重写为新的错误码或成功。

### C07-01 固定开销与压缩后原收据（2026-10-01）

- macOS MAC-A-01 的真实 StepFun 首败：两个指定测试已 exit 0/1，三次压缩却只留下 accepted input，
  原工具交换消失、任务重复检查后触及冻结 cap。第一轮/最小反例均保留，不由修后成功抹除。
- ContextLifecycle 把不可压缩 instructions/schema 计入 75% 软字节触发点；固定开销本身较大时持续
  再摘要。预留最大摘要长度又排除实际仍放得下的有界后缀，模型只能依赖会失真的派生说明。
- 现对该固定开销窗口保留剩余硬字节余量的一半供历史增长；摘要完成后按实际大小核对最多三个完整
  原交换、原 call/result ID 和顺序。失败结果/原要求、token/byte/message 硬限、图片保护、权限及
  freshness 规则不变，不取回未知 archive，不执行摘要文本，不重标旧观察为当前证明。
- 最小反例修前失败、修后通过；压缩 **15/15**、Runtime **200/200**。macOS 正式修后有两次实际压缩
  保留精确 failed-check 与 plan 回执，11 requests 内完成，原件/测试次数/历史错误独立核对。
  只关闭此公共根因；完整连续/纠正、其他平台、首发参数及新发现的 App 退出等待未由该证据代判。

### C03-01 Git diff 的分类标记不是补丁正文（2026-10-01）

- MAC-A-01 真实 Git 回执出现 `Fdiff/H@@`；App 与 VCS 工具的 formatter 对所有非 NUL origin 加前缀，
  把 libgit2 file/hunk/binary/EOF 分类误当 unified diff 字符。逐 byte Git CLI 最小反例两路径均首败。
- 现只有 context/add/delete 内容行补标准 sigil，其他完整记录原样返回；不改路径解析/owner 过滤、
  admission、权限、文件/index 或 1 MiB/UTF-8 上限。真实 F/H 内容、EOF 与 binary notice 不被替换。
- macOS App staged/unstaged/EOF/binary CLI oracle **1/1**、嵌套 workspace 正负向 **1/1**、VCS 工具
  **6/6**；日志在 `2026-10-01/macos/c03-diff-render/`。只关闭格式化根因，正式 UI/live 及其他平台
  原生结果不代判，完整综合 A/C03 未因该组件结果通过。
- Windows W186 并行发现的是同一格式化根因，统一引用本项；保留双方独立首次结果、CLI byte oracle
  与 patch 解析断言，不重复建立公共修复任务或互相移植平台 PASS。

### C01-01 宿主命令示例与启动失败恢复提示（2026-10-01）

- MAC-A-01 已有整行 `/bin/ls -a` 被按字面查找、not_started 的真实反例。暴露给 macOS 的公共
  exec 示例仍混入 Command Prompt，属性示例同时列出两平台命令；not_started 提示先推荐 shell。
  本项只修提示一致性，不据此断言已证明模型选错的唯一原因或真实首发已修复。
- 通用示例改为合法 JSON 的 Git argv；宿主提示/属性按本机给出普通 executable 与分离 args，
  macOS 明示 `/bin/pwd` + `["-P"]`、`/bin/ls` + `["-a"]`。保留 Windows PowerShell 5.1 的专属
  合同；失败反馈优先纠正字面 argv，只有确需 shell syntax 才用 cmd。未改 Schema 接受集合、
  owner、权限、自动拆参或错误/收据语义。
- 两个最小回归首败保留；macOS tools/Schema **12/12**（含 canonical admission subset）、
  process host **10/10**、既有 native 字面特殊 token **1/1** 通过。错误整行仍未启动，正确 argv
  后续 exit 0/reaped；日志在 `2026-10-01/macos/c01-literal-guidance/`。正式 UI 被当前锁屏阻断，
  不以组件结果关闭 C01/MAC-A/N3，不代判 Windows 原生验收，也不把模型预算恢复为 0。

### C07-02 动态前缀与压缩余量（2026-10-01，确定性子根因已修，正式重验待补）

- macOS MAC-A-03 已在当前 `38c0a0df6` 正式 Tauri/StepFun 上复现：首发 literal pwd/ls 正确，
  但测试前重复目录观察；15 次请求中六次压缩仅保留 accepted input，最终 `compaction cannot fit`
  failed，无完成报告。完整原始请求/SSE/事件/UI 在 `2026-10-01/macos/mac-a-current/`，不追加
  模型重试或通过扩 context/byte/输出额度制造通过，先定位实际 ContextLifecycle 的失败条件。
- 固定工具从 22/26,239 bytes 增至 27/约 38,729 bytes，system 前缀从 5,344 增至 20,777 bytes；
  这些是原 OpenAI 请求的字段尺寸，不是 tokenizer 或 SDK encoded_size 的替代。八个主请求 usage
  input 最大 14,977 tokens，冻结 context/output 仍 32,768/4,096。现有 byte trigger 修复未覆盖
  当前动态前缀/余量现场；不能用 C07-01 的旧组件 PASS 关闭此新反例。soft token/硬字节、摘要大小、
  suffix 保留与动态完成数据的具体因果仍需最小确定性反例核对，不先声明唯一根因。
- 复核共享新提示时发现一个旧测试仍要求单个 criterion，已对齐八 ID 分组并补九 ID 拒绝，
  completion **29/29**；原 stale/missing/failed/no-evidence admission 断言不变。这只修回归
  文案，不是 C07-02 产品修复，不代判 Windows 的完整正式场景。

- MAC-C07-01 原生确定性反例进一步确认：replacement **64,371/65,536 bytes**、估算输入
  **21,457 tokens** 加原输出/预留仍在冻结 **32,768** 内，却按 **21,120** 软触发点拒绝。
  这证明同类 soft/hard 混用，不声称已逐字段重放 MAC-A-03 或证明其所有失败条件都已消失。
- 固定前缀连摘要预留已占满软触发时，现在仅在原硬 token 余量内留一半给历史；成功 replacement
  作为 byte floor，避免有效摘要本身立即再触发压缩。原 output、硬 byte/message/token、未读图片、
  provider usage 正偏差与 typed prompt-overflow 收紧约束不扩；新错误只补有界尺寸，不含正文/参数。
- 首败保留；拟合的原 failed receipt、原 call/result 身份及 accepted input 保留，小幅续行不再摘要；
  硬 byte 超限、实测 usage 余量和 typed rejection 均保持拒绝/原状态/零摘要调用。压缩 **18/18**、
  Runtime **217/217**。旧预算对照另复现已有 review 夹具越过“仅 report”展示的失败；仅校正夹具并
  增强终结控制断言，未改 review 产品逻辑。日志 `2026-10-01/macos/c07-prefix-headroom/`，
  无新模型调用；完整 MAC-A/B/C、实际模型压缩后续行/N3 与 C05 原生退出仍待验。

### C06-01 完成参数纠正不重开已完成工作（2026-10-01，提示已修，正式修后待补）

- MAC-A-04 正式 StepFun 已完成读搜/Git/两指定测试，各真实 exit 0/1；首次完成报告却把三条
  当前不可用的读搜 ID 标 supported，并填累计 tool error **0** 而不是当前 const **1**。原
  preflight 正确拒绝，无报告被接受；随后模型重开整个计划、重复目录检查并触及本地 cap。
- 保留所有 schema/计数/freshness 拒绝，单独非法 report 的反馈现在明确只修完成参数、不要
  重跑已结算命令/测试或把计划步骤重置。历史观察与当前缺证据分开，当前未证实项用 unverified
  且无证据；不能用无关 eligible ID 替代。混合未执行效果批次仍用整批纠正指引，不能跳过其写入。
- 最小反例首败保留；validation **7/7**（含两错误同拒/隐私、整批持有与合法修正）、settled-failure
  report gate **1/1**。本项只修反馈，不自动把旧 ID 变新、不抹去故意非零的真实观察，也未证明
  模型今后一定遵循。证据 `2026-10-01/macos/mac-a-post-headroom/`；完整完成/N3 仍待正式重验，
  不代判 Windows 或以原提议但被拒的 summary 宣称交付。

### C06-02 被拒终结账号的窄路径审查（2026-10-01，结构修复已验证，正式修后待补）

- MAC-A-05 已加载 C06-01 专门反馈与计数单值提示：两个计数正确为 1，首次 report 仍因两个
  历史搜索 ID 被拒。摘要保留“不要重跑”文字，但随后计划可重置 pending、普通工具可继续，
  后续又重复十次检查并触及 16-request 本地 cap。提示没有闭环，不用继续堆叠提示/扩大预算。
- 确定性反例首败证明该结束路径仍广告 update_plan、exec/read/history。现仅在单独非法结束
  report、revision=0、proved settled_failure_gate、无运行进程/未决补丁时复用 report-only review；
  错误参数只能修账号、披露 unverified/blocked，不能重开计划或执行额外检查。其他有显式计划、
  未决效果/进程/补丁、真实新输入路径不由此假定已完成；所有 schema/引用/计数拒绝继续有效。
- 首败和中间反例均保留；被拒计划重置仍计入真实失败总数，修正后计数 2、诊断仅一次、终结
  工具唯一展示、没有额外 owner dispatch。Runtime **220/220**。日志及真实失败数据在
  `2026-10-01/macos/mac-a-report-repair/`；结构改动未进入冻结 live App，完整 MAC-A/B/C、交付
  与 N3 待正式重验，未以组件结果或被拒 summary 关闭场景，也未移植 Windows 平台 PASS。

- macOS MAC-C06-01 补反向边界 **1/1**：显式未完成计划下的非法 report 不收掉已授权修复，
  write 实际一次，闭合计划后才报告；已有 terminal、running-process、unresolved-patch guards
  各 **1/1**。首个 fixture 使用未广告控制的错误顺序保留/纠正，未修改产品逻辑或降低断言。
  当前锁屏只阻断正式 UI/live，组件结果不替代修后 N3；证据 `2026-10-01/macos/c06-terminal-boundary/`。

- macOS MAC-A-06 正式修后 **N1 子样本**：首个历史引用仍拒绝，后续原生请求只展示 report、
  Specific tool_choice；9 次请求接受修正报告，六个已结算命令/两测试未重复，completed/head ready。
  两个当前缺证据条目保持 unverified，计数 1/1 未抹去。只验证此纠错/防重开链；全任务交付仍缺
  头尾与搜索历史结果，完整 N3/A/B/C 不因 canonical completed 通过。证据 `macos/mac-a-terminal-review/`。

### C06-03 历史实际结果与当前证明的交付缺口（2026-10-01，歧义说明已修，正式重验待补）

- MAC-A-06 的真实 read_file/search_files 已完成、独立源 hash/字节/匹配正确；后续 opaque commands
  使其失去当前资格。修正报告正确不借旧 ID 作 current supported，却连原头尾/行数/两搜索结果
  也未交付，输出内部 stale_file_paths/unverified 与英文 rationale。外部完整任务验收明确失败。
- 当前资格丢失不表示历史观察不存在或命令没执行。后续须解决“如实交付已观察历史事实，分开
  当前状态未复核”的数据/表达链路；不得放松 current freshness、借无关 ID、伪造检查或用
  提议但未接受的报告关闭场景。不靠重跑所有检查、扩大 context/调用预算填补。
- 本项只记录真实缺口，未提交额外产品假设或宣称根因已修。日志/原请求/SSE/UI/DB/严格首败在
  `2026-10-01/macos/mac-a-terminal-review/`。C06-02 正式子链有证据不替代此交付门槛或 N3。

- MAC-C06-02 核对原 request 06～09，四行原文与两个搜索名称仍在每个模型上下文中，排除
  “压缩已把结果清空”的归因。完成接口一方面要求完整 summary，另一方面把缺当前资格表述成
  “此结果未验证”，且称 rationale 不展示但实际会追加 warning，存在历史交付/当前证明歧义。
- 仅替换 summary 属性与既有引用说明：已知较早实际结果仍须按用户要求交付，清楚标时点，
  后续状态未核对另行披露；不拿历史事实作当前证明，不把未知结果编造出来，rationale 也按
  用户语言写。无新增提示层、字段、原文副本、工具、权限或 schema 接受集合变化。
- 最小说明回归首败保留；completion **31/31**。历史结果 summary+unverified/no evidence 已
  被原提交逻辑允许；stale ID 的 current supported 仍拒绝、历史 epoch 不变，正文/环境/输入
  排除与计数保护不改。这验证接口分工/说明，不证明模型一定遵循；无新 live 请求。
  证据 `2026-10-01/macos/c06-historical-delivery/`，完整交付/N3/A/B/C 仍待正式修后验证。

- macOS MAC-A-07 正式修后 **N1 子样本**：九次请求、一轮压缩、六命令无重复，历史四行原文/
  行数 4/查有 1/零匹配 0 已按较早观察交付，当前缺资格项仍 unverified/no evidence，未借旧 ID。
  提示的历史内容部分已有真实证据，不声称所有模型稳定遵循。整体独立验收仍失败：cwd 实际路径
  没交付、内部术语未消失；原请求 06～09 包含路径，不能归因为信息丢失。证据
  `2026-10-01/macos/mac-a-historical-results/`，路径/自然语言/N3/A/B/C 门槛不缩减，未再扩预算。

## 历史全产品口径快照（已停止本轮排程）

- 公共 P0 为 **5/5 任务已验证**。本文213个去重问题编号属于问题簇，不是已通过的Case数量。
- Windows正文的明确批次范围标注涉及171个共享Case；该提取只证明文档引用，不能证明实际执行、
  PASS或完整覆盖。其余Case不能据此直接记NOT_RUN；此为历史统计，不另追加675 Case核账任务。
- 旧全角色/20重复/100 seed/LONG/99%要求留在发布认证计划；用户已将其移出本轮结束条件。
  本轮安全及结果断言不缩减，具体执行以最新实施计划为准，不再补全不相关业务来填旧分母。
- 本次只读统计与依据在仓库外 `2026-09-30/progress-audit/`；Git仅维护此摘要，不生成逐槽索引。

## 当前 P0 批次

| 任务 | 对应 Case / 本批子断言 | 排查与修复 | 状态 / 验证 |
| --- | --- | --- | --- |
| S-P0-01 清单与固定合同 | REG-001/008/011/017；G0-027；AUTH-001/002 | 官方 seed、Action/effect/resource、Schema/digest 唯一；保留精确 GEN/COD 补项，不扩大条件权限 | 已验证：合同 check、官方 catalog 3/3 |
| S-P0-02 控制协议与整批预检 | G0-003～005/010；CTRL-001/002/006/007 | 采用上游已修复的可选 explanation 与一致执行、顺序控制批次、共享要求证据语义；补整批非法参数反例 | 已验证：Runtime 136/136，保留零 dispatch、幂等与正确完成状态 |
| S-P0-03 exact 产品资源 | AUTH-004/005；APAL-001；AMUL-001 | 新 Canvas Session 复用旧 product binding 时先检查精确目标与资源唯一性；修复创建入口绕过 resolver，保留合法其他资源及旧 Session | 已验证：修复前反例失败；完整 route 36/36 含最终反例通过 |
| S-P0-04 Skill 与动态上下文 | AUTH-008；EXT-006/010/012；CTRL-018/019 | 真实 Skill 正文/digest/来源/依赖/active set；steering 正文与用户账本隔离，恢复不能只用 ID | 已验证：合并后 route 正负向及 Runtime Skill 隔离/恢复测试通过 |
| S-P0-05 owner 边界与错误 | REG-014/015；G0-008/027；OBS-005/016 | 三项 owner 边界检查；Schema 漂移返回既有类型化错误和 expected/actual digest，不再只报泛化字符串 | 已验证：三项检查、digest/未知工具反例通过；不代替 UI 全验收 |

P0 验收命令：`cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check`、
`bun run check:process-runtime-boundary`、`bun run check:agent-vocabulary`、
`bun run check:unified-plugin-boundary`。新增缺陷再运行直接关联的确定性测试。
全部日志放在外部 `phase-2-3/2026-09-27/shared-p0/`。

本批结果：Runtime 136、App route 36、official catalog 3 项通过；合同 check、三项边界检查、
desktop-ui-boundary、相关 Rust 格式检查及 diff 检查通过。没有新增付费模型调用。
Canvas 首次 readiness 修复未覆盖创建入口，重新编译后仍失败；补齐实际入口后才通过，
上述中间失败保留在 `canvas-pre-fix-regression.log`、`canvas-post-fix-single-rebuild.log`，
最终证据为 `app-route-and-catalog-final.log` 与 `runtime-merged-final.log`。
合并时上游新增的 steering 测试夹具漏了本地新增字段，已补空值并完整重跑 Runtime。

## 历史全领域参考（不作为活动队列）

下表是原全产品计划的冻结领域分配，675个Both ID各有一个主领域；仅作历史参考，当前不全量排程。

| 队列 | 共享 Case 数 | 测试 → 排查 → 修复任务 | 后续门槛 / 状态 |
| --- | ---: | --- | --- |
| S-D01 | 85 | 工具/模型协议、注册/激活与版本；先找 Schema/admission 断点，再做生产 owner 修复 | P0 本轮；完整传输/故障矩阵待走查 |
| S-D02 | 70 | 控制状态、完成证据、事件和真实体验；保留首次失败，核对 canonical/UI 一致性 | Windows GEN 文件/产物/删除及失败停止场景 N3 通过；S-D02-22/24 已修 startup quarantine 单播及页外暂停证据投影，M04-26 正式 UI 复核通过；完整控制/观测矩阵待验 |
| S-D03 | 45 | 文件/Artifact 合同、原子边界、source digest、负向隔离 | Windows 新建/既有暂存源等定向回归；macOS APFS identity、Unix 清理与扩展 ACL 保留已有原生验证；完整入口/矩阵待验 |
| S-D04 | 120 | 公共 command/args/cmd Schema、进程 owner 与清理协议 | P0 边界本轮；原生实现转 W01/M01 |
| S-D05 | 27 | Git/SSH 授权与副作用核对；独立 remote/host 夹具 | Unix/Git 与 macOS 四类 hook/receipt 已验；隔离 macOS loopback sshd 的 transport/owner/搜索已验，外部 host 与正式 UI 条件资源准备后继续，禁止共享生产 remote |
| S-D06 | 67 | Skill/MCP/Plugin/Browser/Computer 的发现、冻结与生命周期 | S-D06-01 修复 A11y/截图媒体耦合；S-D06-02 保留 Computer plan discoverability；S-D06-03 区分 proven stale rejection 与 uncertain effect；S-D06-04 缺失绝对 launch 路径 fail closed；S-D06-05 修复 input timeout 脱管及 release 义务，本批 live cancel 仍开放 |
| S-D07 | 53 | 领域 owner/cardinality、跨实例与精确目标绑定 | Canvas/PAL 入口本轮；S-D07-01 修复画布名称上下文，其他平台/完整集合待验 |
| S-D08 | 103 | 五类 Agent 的产品入口与目标能力；逐角色验证，不互相代替 | 首批 Windows 四条路径已有结果；完整矩阵待走查 |
| S-D09 | 75 | 恢复 fence、取消、并发、压缩、预算与长稳；按状态边界注入故障 | P1 故障验证后安排 LONG/soak |
| S-D10 | 15 | PORT-001～015 内部端口、outbox、generation 与外部 grant 隔离 | PORT-012 队列、native rescan、UI 手动/重连对账子断言见 S-D03-21/22/24/25，残余丢失信号见 S-D03-52；其余端口及完整恢复待验 |
| S-D11 | 15 | 权限/资源/旧快照/撤权/secret 负向，验证拒绝前无副作用 | 本轮只验关联静态与资源断言；竞态仍待走查 |
| **合计** | **675** | 只统计共享 Case 定义 | 不增加 4,740 个平台结果槽 |

## 历史修复与未关闭项

- S-D06-05（`COMP-012` 及 A08/A13/A17/A20 的共享 input cleanup 子断言，**部分完成**）：
  `nomi-computer` 原先以 `timeout(INPUT_TIMEOUT, spawn_blocking_handle)` 包住原生输入；timeout 丢弃
  `JoinHandle` 后，已准入任务仍可在后台继续按键/拖拽，上层却已收到失败并可能过早结算 cleanup。
  新失败回归在 timeout 返回时精确观察到 `pressed=true`，完整保留于 macOS
  `m04-computer-cancel/run-004-first-failure/`。
- 现 timeout 只停止等待预算，不脱管已开始的 native worker：必须 join 同一任务后才返回明确 uncertain
  错误，说明可能已产生效果、禁止自动重试并要求重新观察。drag/key 统一用 recorded obligation guard，
  press 前先记录，按逆序释放；首次 release 失败只重试精确剩余项，连续失败继续保留 obligation 并公开
  cleanup unproven，panic/unwind 亦做 best-effort release，不吞错或伪造成功。Engine Tool Host 原有“调用方
  取消后继续持有 effect 至 owner settlement”回归复核 **1/1**。
- 最终 `nomi-computer` **99 passed / 7 ignored**，新增 timeout join、release-retry、连续失败保留三项均
  通过；fixture build、fmt/diff 通过。macOS signed Tauri 探索运行均证明真实 drag 最终
  `pressed=false` 且 mouse-up 到达，并保留 timeout unknown/cleanup pause 与本机 UI Stop 在 held mouse
  期间不可自动操作的首败；但没有得到“取消事件确实发生于 pressed=true”的最终正式样本。因此本项只
  关闭共享组件清理缺陷，不关闭 `COMP-012`、平台验收或共享阶段；Windows/Linux 亦未代判。
- 后续 macOS M04-25 通过正式 UI + 已认证产品 cancel 取得有界 drag 期间真实取消：canonical 取消后
  17 ms 目标仍 pressed，最终唯一 mouseUp/input returned/`host_cleanup_proven`，UI cancelled、零
  pending/unknown 与进程/端口残留。实测 hold 106.6 ms，因此长时 drag/key hold、原生 release-failure
  与完整 `COMP-012` 继续开放；本批没有新增公共产品修复或付费模型调用。

- S-D06-04（`COMP-006`、`OBS-005/016` 的共享 Computer launch 缺失绝对路径子断言）：macOS 正式
  Tauri 首败对明确不存在的 `.app` 绝对路径调用 `computer/launch`；`open::that_detached` 只确认成功
  派生系统 opener，owner 随即错误返回 Opened，managed effect 误结算 returned，实际路径/进程均不存在。
  现 `nomi-computer` 在 OS 调用前拒绝不存在的绝对 target 及显式 app 路径；已存在路径和相对 app 名
  保持原合同。Engine Core 只对精确 `computer/launch` 投影 bounded 安全指引，明确路径不存在、使用确切
  已安装应用或现存文件/目录、不要猜测替代或原样重试、没有成功启动；不向模型泄漏私有路径或底层诊断。
  中间正式运行已证明 owner/effect rejected，但旧 Kernel 投影只剩泛化 code，亦独立保留。
- launch **7/7**、bounded guidance **1/1**、fmt 与两次正式 Tauri build 通过；macOS 最终 2-step /
  34-event Turn completed，唯一 launch effect rejected、目标仍不存在、fixture failure=null，两 DB `ok`
  且进程/监听清零。Windows/Linux 原生 opener、相对缺失 app 名、TOCTOU 删除及完整 `COMP-006` 未由此
  代判，不关闭共享阶段。

- S-D06-03（`COMP-002/010/011`、A07/A08/A13/A17/A20 的共享 Computer stale/settlement 子断言）：
  macOS AX generation 未因外部窗口/焦点变化前进，旧 semantic ref 可向已退到后台的 TextEdit 输入；
  首修在 native owner 动作前拒绝 stale，却因共享 role host 把所有 input provider error 归为 uncertain，
  effect 仍进入 unknown/pause。现 role host 只接受 Computer 内部精确 `stale + no pixel fallback` 零效果
  证明并映射 `ROLE_HOST_STALE_OBSERVATION_GENERATION`/rejected；其他错误绝不降格。macOS actor 另核对
  observer dirty/frontmost PID。正式 Tauri 5-step Turn completed，后台 TextEdit/磁盘保持 `seed`；三个核心
  回归各 **20/20**。Windows/Linux actor、pixel/raw-coordinate、cancel/crash/soak 未由 macOS 代判。

- S-D06-02（`CTRL-001`、`COMP-003`、`AGEN-013` 的共享 adaptive-control 缺口）：Computer 被明确
  排除在 workspace task-ledger 自动激活之外，单个原子动作因此不会强制产生 plan；但纯 Computer
  多步任务的首个模型请求同样看不到 `update_plan`，显式尝试会先返回“not exposed”，该失败才激活
  plan/completion controls。macOS 正式 Tauri 的 TextEdit launch/A11y/7 次 input/save 最终可在
  20 steps 内以 canonical `input_0` 完成，并公开保留这 1 个 tool error；首次 fixture 误判 plan gate
  及未校验 completion 的假成功均保留。当前没有扩大所有外部原子动作的 ledger policy，也不把恢复后
  completion 写成零错误体验；需要产品层决定如何在不惩罚单次原子动作的前提下，让多步 Computer
  任务首次发现 plan。Windows/其他平台未代判，stale/cancel/crash/soak 仍开放。

- S-D06-01（`AUTH-012`、`COMP-008/009`、`MAC-011/012` 的公共 Computer contract 子断言）：
  canonical `computer/a11y.observe` 仍复用旧组合 observe，机会性 screenshot 既把 Accessibility 与
  Screen Recording authority 错误耦合，也会用 base64 文本触发无意义 compaction；单独的
  `computer/observe` 又没有把原生 JSON 内 pixels 恢复为模型 typed image。现 A11y canonical action
  固定 AX-only 并清除旧 capture；截图 adapter 只接受精确 platform-builtin Action、当前
  generation/Snapshot 与 ImageInput route，移除 JSON base64 后返回一个 bounded typed PNG，durable
  observation 只留文本描述。canonical PNG 在 Kernel JSON hop 前压到 1.5 MiB，并拒绝非法 PNG，避免
  5 MiB native 上限与 4 MiB Engine result 上限错配；没有扩大 Action、TCC、资源或默认权限。
- macOS Developer ID-signed 正式 Tauri granted Session 已让 AX-only 109-element tree 与 typed screenshot
  各执行一次并完成；fresh denied identity 的 A11y-only、screenshot-only Session 均精确
  `ROLE_HOST_PROVIDER_FAILURE`、无 fallback。确定性回归为 `nomi-computer` 94 passed / 7 ignored、两项
  核心断言各 **20/20**、typed-media 3 项 **20/20 批**及 role-host **4/4**。首次 compaction、call ID
  reuse、multipart fixture 与 plan guard 失败全部保留在 macOS M04-02 证据。Windows/其他平台未由此
  代判；live 单项 grant、撤权竞态、input/launch、长期/99% 与完整 Computer Case 仍开放。

- S-D05-08（VCS-013、A05/A13/A17/A19 push 成功后 settlement 丢失子断言）：W110 用隔离
  worktree 与 bare local remote 补物理 push→durable receipt 之间的故障窗口。首次即通过：remote ref
  已更新到第一提交后丢弃未确认 settlement，owner 固定进入 outcome unknown；本地再创建第二提交并
  重试时在接触 remote 前拒绝，remote ref 仍为第一提交。等待原 worker 结束不能清除 durable
  settlement 丢失。新回归 **20/20**，push owner **10/10**；相邻 host 已有“成功 receipt 持久后同 key
  只重放原回执”和 not-applied failure 重放 **2/2**。生产代码无需修改，无模型/UI/生产 remote。
  应用崩溃后 exact pending effect 的 push 专项恢复、主动 remote-ref 对账、远端删除/重写并发及
  N3/LONG/99% 仍开放，不关闭完整 VCS-013 或共享阶段。

- S-D05-09（VCS-013、LIFE-006/007、A05/A13/A17/A19 push pending 跨 host 子断言）：W111
  在 canonical Store 先 reserve 精确 external push effect，再真实更新隔离 bare remote，但故意不写
  terminal receipt；随后创建只存在本地的第二提交并重建 `Wave2ApplicationHost`。首次及连续
  **20/20** 均由 durable pending 在新 owner 物理调用前拦截，同 operation 不重放，remote ref 保持
  第一提交，Effect 状态仍为 Pending。相邻 host push **3/3**、owner **10/10**。生产代码无需修改，
  无模型/UI/网络 remote。当前仅重建 host 并复用同一内存 Store；真实数据库关闭/重开、进程强退、
  pending→unknown 启动归约、主动 remote-ref 对账与其他平台仍待验，不关闭完整 VCS/LIFE。

- S-D05-10（VCS-013、LIFE-006/007、A05/A13/A17/A19 push pending 跨数据库重开子断言）：W112
  将 W111 同一物理故障窗口改为独立磁盘 SQLite；remote 更新、Effect 保持 Pending 后释放 reservation、
  host 与 Store，关闭全部数据库连接并从同一路径重新初始化 Store。首次及 **20/20** 均在新 host/
  新 owner 调用 remote 前返回 durable pending，第二提交仍只在本地，remote 与 effect identity/state
  不变；相邻 host push **3/3**。生产代码无需修改，无模型/UI/网络 remote。尚未覆盖进程在 Git worker
  或 SQLite terminal commit 中被强杀、完整 App startup 的 pending→unknown 归约、主动 remote 对账及
  其他平台，不关闭完整 VCS/LIFE。

- S-D05-11（VCS-013、LIFE-006/007、OBS-005/016、A05/A13/A17/A19 完整 App 启动归约子断言）：
  W113 在 `native_execution_recovery` 的磁盘 crash image 中，通过 canonical Store 给当前原生 Turn
  写入 `workspace.vcs/push` 的 `ExternalUncertainEffect` Pending 事实，再关闭并从正式
  `AppServices`/`create_router` 启动入口恢复。启动调度经过既有 fenced recovery 后原子写入
  `effect/uncertain`，将 Pending 升级为 Unknown，写入唯一 `runtime/execution-recovery-blocked`，暂停
  原 Turn 并保留 checkpoint；模型请求数保持 0，新 Turn 被拒。再次关闭数据库并完整启动后仍只有
  一个 Unknown 和一个 blocked 事件，没有重复隔离或重放。新增断言首次及连续 **20/20**，完整 App
  恢复夹具 **6/6**，相邻 Store 原子隔离 **2/2**、host fence **1/1**。生产代码无需修改，无正式 UI/
  模型/生产 remote；W110～W112 已提供真实 bare remote push 与 receipt 丢失现场，本批只补正式启动
  归约。Git worker/SQLite terminal commit 中的进程强杀、主动 remote-ref 对账、第三方改写、其他平台
  及 N3/LONG/99% 仍开放，不关闭完整 VCS/LIFE 或共享阶段。

- S-D05-12（VCS-013、CTRL-009、LIFE-006/007/016/017、OBS-008/014、A05/A13/A17/A19 owner
  核对与恢复子断言）：W114 在 W113 的完整 App crash-image 链上增加真实 local bare remote；夹具先
  独立推送并读取精确 destination ref，再由正式 `execution/effects` 列出唯一 Unknown push，确认
  `automatic_replay_authorized=false`。错误 input digest 的核对请求在事务前拒绝且 Effect 仍 Unknown；
  认证 owner 以 exact digest、pause revision 和有界 remote-ref evidence 提交
  `execution/reconcile` 后，Store 只写一组 attestation/`effect/reconciled`，同 key 重送返回原回执，
  不伪造原 push receipt。随后带 cleanup attestation 恢复同一 Turn，checkpoint 正常完成；remote ref
  保持原提交，Pending/Unknown 清零。新增夹具首次因漏定义 API path 编译失败，修正夹具后产品断言
  首次及连续 **20/20**；完整 App 恢复 **7/7**、Store pause/reconcile **11/11**。生产代码无需修改，
  无正式 UI/生产 remote。领域级自动对账仍是文档明确未实现的限制；第三方改写、网络 remote、UI
  核对控制面、其他平台及 N3/LONG/99% 仍开放，不关闭完整 VCS/LIFE 或共享阶段。

- S-D05-13（VCS-006/009/014、LIFE-006/007、A05/A13/A17/A19 commit receipt 丢失子断言）：
  W120 在磁盘 SQLite 中 reserve `workspace.vcs/commit` external Effect，再经生产
  `invoke_vcs_commit` 创建真实 commit；owner 返回 commit ID 后故意不写 terminal receipt，并关闭全部
  数据库连接。用户随后修改并 stage 新字节。重开后原 key 与新 operation/key 均在 commit owner/hook
  前被 workspace fence 拒绝；HEAD 保持首个新 commit、历史仍只有 initial+该 commit，index blob 与
  worktree 都保留用户后写字节，Effect 保持唯一 Pending。新增断言首次及连续 **20/20**；相邻 commit
  replay/identity/scope **4/4**、stage/commit 共享门及并发 commit **2/2**。生产代码无需修改，无模型/
  UI。commit-intent 主动核对、libgit2 worker/SQLite terminal commit 进程强杀、post-commit 故障窗口的
  Windows 运行、其他平台及 N3/LONG/99% 仍开放，不关闭完整 VCS/LIFE 或共享阶段。

- S-D05-14（VCS-004/014、LIFE-006、A05/A13/A17/A19 stage receipt 丢失子断言）：W121 在
  磁盘 SQLite 中 reserve `workspace.vcs/stage` Effect，并由实际 `WorkspaceVcsStageOwner` 把 worktree
  新字节写入 Git index；owner 返回 staged receipt 后故意不写 terminal receipt，关闭全部数据库连接。
  用户随后把 index 恢复到 HEAD，同时保留新的 unstaged worktree 字节。重开后原 key 与新
  operation/key 均在 stage owner 前被 workspace fence 拒绝；HEAD/index 仍为 base，worktree 保留用户
  字节，Effect 保持唯一 Pending。首次新增断言使用早先打开的独立 `git2::Repository` 读到缓存 index，
  失败已保留；改用重开仓库的磁盘 oracle 后首次及连续 **20/20**。相邻 App stage/commit **3/3**、
  VCS stage owner 锁/scope/删除/置换 **7/7**。生产代码无需修改，无模型/UI。index replace/SQLite
  terminal commit 进程强杀、stage 结果人工核对、linked worktree/submodule、其他平台及 N3/LONG/99%
  仍开放，不关闭完整 VCS/LIFE 或共享阶段。

- S-D09-06（CONC-015、FILE-038、LIFE-006/007、AUTH-009、A05/A13/A17/A19 跨 Session
  workspace fence 子断言）：W122 在 Session A 的磁盘 Effect reserve 后由实际 `FileService` 发布文件，
  故意丢失 terminal receipt，并关闭全部数据库连接；重开 Store/host 后创建同一认证 owner、同一物理
  workspace 的独立 Session B。B 的不同 operation/key 在文件 owner 前被全局 unsettled-resource 索引
  拒绝，未创建 B Effect 或目标文件；A 仍只有一个 Pending，已发布字节不变。新增断言首次及连续
  **20/20**；既有 owner-scoped/跨模块 fence、W115 写入重开及 Store external uncertainty **3/3**。
  生产代码无需修改，无模型/UI。正式 App 多会话入口、真正同时竞争、跨 owner 错配同一路径绑定、
  owner 核对、其他平台及 N3/LONG/99% 仍开放，不关闭完整 CONC/FILE/LIFE 或共享阶段。

- S-D09-07（CONC-015、FILE-038、LIFE-006、AUTH-009、A05/A13/A17/A19 跨连接并发 admission
  子断言）：W123 使用同一磁盘数据库的两个独立 SQLite pool、两个 host 和两个同 owner/workspace
  AgentSession，同时提交不同写 Effect 的 canonical reserve。每轮恰好一个 Reserved、一个在唯一
  unsettled-resource 索引处失败；只调用 winner 的实际 `FileService` owner，因此只有一个物理文件。
  关闭两套连接再重开后，loser 仍被 winner 的 Pending fence 拒绝，未创建 loser Effect/文件。新增
  竞态首次及连续 **20/20**，W122 顺序重开与既有 owner-scoped/跨模块 fence **2/2**。生产代码无需
  修改，无模型/UI。两个完整 host invocation 同时停在 owner 前、跨 action 竞态、跨 owner 错配绑定、
  owner 核对、其他平台及 N3/LONG/99% 仍开放，不关闭完整 CONC/FILE/LIFE 或共享阶段。

- S-D09-08（CONC-015、FILE-038、VCS-004/014、LIFE-006、AUTH-009、A05/A13/A17/A19 跨 Action
  workspace admission 子断言）：W124 用两个独立 SQLite pool/host/Session 同时 reserve 文件写与 Git
  stage。每轮唯一 workspace fence 恰好放行一个 Effect；随后只调用 winner 对应的实际 FileService 或
  VCS stage owner。关闭连接再重开后 loser action 仍被 Pending 拒绝；若文件写获胜，Git index 保持
  base 且只出现新文件；若 stage 获胜，index 为 candidate blob 且新文件不存在，HEAD 始终不变。
  新增竞态首次及连续 **20/20**，W123 同 action 竞态及两类 owner 正常路径 **3/3**。生产代码无需
  修改，无模型/UI。两个完整 `invoke` 从入口并发、其他 Action 配对、跨 owner 错配绑定、owner 核对、
  其他平台及 N3/LONG/99% 仍开放，不关闭完整 CONC/FILE/VCS/LIFE 或共享阶段。

- S-D09-09（CONC-002/015、FILE-020/025、A05/A13/A17/A19 完整 patch invocation 竞态子断言）：
  W125 用两个独立 SQLite pool/host/AgentSession 从正式 Wave2 `invoke` 入口并发 patch 同一文件；两边
  都绑定同一 exact source digest，但写入不同结果。每轮恰好一个成功；loser 要么在 winner Pending
  时无 Effect 拒绝，要么在 winner 已结算后获 admission、再以 stale source Rejected。最终文件只等于
  winner 内容，无第二次覆盖；winner 为 Returned，loser 为零 Effect 或唯一 Rejected，两 Session 都
  无 unsettled。关闭数据库再重开后，第三 Session 可正常写新文件，证明 fence 已释放。新增竞态首次及
  连续 **20/20**；不同路径并发、W116 receipt-loss、FileService 外部变更回滚及 W124 跨 Action
  **4/4**。生产代码无需修改，无模型/UI。write/delete/Artifact 等其他完整 invocation 配对、正式 App
  API 多会话、其他平台及 N3/LONG/99% 仍开放，不关闭完整 CONC/FILE 或共享阶段。

- S-D09-10（CONC-002/015、FILE-019/020/023、A05/A13/A17/A19 完整 write invocation 竞态子断言）：
  W126 用两个独立 SQLite pool/host/AgentSession 从正式 Wave2 `invoke` 入口并发向同一路径写入两份
  各 1 MiB 的不同字节。若第二个请求撞到 Pending，则一个成功、一个在 admission 拒绝；若第一个已
  完成 settlement，则两次按可审计顺序成功。无论顺序，最终文件长度精确 1 MiB，全部为 `L` 或全部
  为 `R`，没有交错/截断；每个已创建 Effect 都是 Returned，两 Session 无 unsettled。关闭数据库并
  重开后，第三 Session 可正常覆盖，证明 fence 释放。新增竞态首次及连续 **20/20**；W125 patch
  竞态、W115 receipt-loss、普通文件动作及 FileService 原子写 **6/6**。生产代码无需修改，无模型/UI。
  8 MiB 边界并发、原生 replace 中途强杀、write/delete 配对、其他平台及 N3/LONG/99% 仍开放，
  不关闭完整 CONC/FILE 或共享阶段。

- S-D09-11（CONC-002/015、FILE-019/020/034、A05/A13/A17/A19 write/delete 完整 invocation 竞态
  子断言）：W127 用两个独立 SQLite pool/host/AgentSession 从正式 Wave2 `invoke` 入口并发写入和删除
  同一路径；write 内容为 1 MiB。允许一个请求撞到 Pending 而拒绝，也允许两次按某个串行顺序成功。
  最终磁盘只可能不存在，或存在精确 1 MiB 全 `W` 文件；不会保留旧 `base`、半写或混合字节。每个
  已创建 Effect 都是 Returned，两 Session 无 unsettled；数据库重开后第三 Session 可正常写入。
  新增竞态首次及连续 **20/20**；W126 write/write、W117 receipt-loss、FileService 原子写与删除置换
  **10/10**。生产代码无需修改，无模型/UI。跨 Session 全局事件顺序投影、8 MiB 边界、目录 write/
  delete、原生调用中途强杀、其他平台及 N3/LONG/99% 仍开放，不关闭完整 CONC/FILE 或共享阶段。

- S-D09-12（CONC-015、ART-001/003/004、A05/A13/A17/A19 Artifact 完整 invocation 竞态子断言）：
  W128 用两个独立 SQLite pool/host/AgentSession 从正式 Wave2 `invoke` 入口并发 publish 同一已观察
  source/digest。允许一个请求撞到 Pending 而拒绝，也允许两次串行复用同一 content identity；所有
  成功回执的 artifact ID/sha256 都等于预期 digest，managed 目录始终只有一个 64 位内容对象。每个
  已创建 Effect 都是 Returned，两 Session 无 unsettled；数据库重开后第三 Session 分页读取仍得到
  complete/同 digest。新增竞态首次及连续 **20/20**；W119 receipt-loss、App Artifact 正常路径与
  ArtifactStore 并发 publication gate **4/4**。生产代码无需修改，无模型/UI。不同 source/digest 并发、
  cleanup/Session 删除与 reader 竞态、大对象边界、其他平台及 N3/LONG/99% 仍开放，不关闭完整
  CONC/ART 或共享阶段。

- S-D09-13（CONC-015、ART-001/003/004/005、A05/A13/A17/A19 不同 Artifact 内容并发子断言）：
  W129 用两个独立 SQLite pool/host/AgentSession 从正式 Wave2 `invoke` 入口并发 publish 两个不同
  source/digest。允许一个撞 Pending 被拒，也允许两者串行成功；managed 目录中的 64 位对象集合必须
  与成功回执 digest 集合精确相等，不允许无回执孤儿。每个对象的字节与自身 digest 对应，所有已创建
  Effect 均 Returned、无 unsettled；数据库重开后逐个成功对象仍能完整读取。新增竞态首次及连续
  **20/20**；W128 同内容、W119 receipt-loss、ArtifactStore 并发 gate 与 round trip **4/4**。生产代码
  无需修改，无模型/UI。cleanup/Session 删除与 reader 竞态、不同源大对象、原生 link 中途强杀、
  其他平台及 N3/LONG/99% 仍开放，不关闭完整 CONC/ART 或共享阶段。

- S-D03-56（ART-005/007、LIFE-028、A05/A13/A17/A19 live reader 与 workspace cleanup 身份子断言）：
  W130 为 `WorkspaceArtifactStore` 增加 live reader 回归：先发布并缓存原 artifact handle，再尝试把整个
  workspace 移走并在同路径放入冒用旧 digest 名称的替代字节。若原生系统允许移走，旧 Store 必须因
  workspace identity 改变而拒绝，新 Store 必须因内容与 digest 不符而拒绝；若原生系统因 pinned
  handles 拒绝移走，则旧 reader 继续只读原字节且替代 workspace 不出现。Windows 诊断明确走后者，
  最终形状首次及连续 **20/20**；ArtifactStore 全组 **16/16**、managed workspace cleanup 与 W129
  **2/2**。生产代码无需修改，无模型/UI。正式 AgentSession delete API 与 in-flight read 真并发、允许
  rename 平台的原生分支、cleanup task 强杀、其他平台及 N3/LONG/99% 仍开放，不关闭完整 ART/LIFE
  或共享阶段。

- S-D03-57（ART-007、LIFE-028、OBS-004/006、A05/A13/A17/A19 正式 Session delete + live Artifact
  子断言）：W131 将既有正式 App managed-workspace 删除测试升级为先发布并缓存 Artifact reader，再以
  固定 idempotency key 删除 Session。Windows 首次真实返回 **500 / INTERNAL_ERROR**（`os error 32`），
  虽保留 deleting fence，但把可预期的 live-handle cleanup 冲突误分类为内部故障；首败已保留。正式
  HTTP 删除流程现把两条 workspace cleanup 分支统一映射为 **409 /
  AGENT_SESSION_WORKSPACE_CLEANUP_FAILED**，保留底层详情与 deleting 状态。释放 Artifact owner 后，
  同一 key 重试完成原 tombstone 和 managed workspace 清理；sibling managed workspace 与 user-selected
  workspace 均保留。修复后首次及连续 **20/20**；删除顺序、managed workspace 边界和 W130 reader
  **3/3**。首个相邻静态命令因错误 target 执行 0 项，已用 `--lib` 完整名纠正，不计通过。无模型/UI。
  正式 Artifact read 请求与 DELETE 真并发、启动恢复 deleting Session 的 live-handle 重试、其他平台及
  N3/LONG/99% 仍开放，不关闭完整 ART/LIFE/OBS 或共享阶段。

- S-D03-58（ART-007、LIFE-020/028、OBS-004/006、A05/A13/A17/A19 deleting Session 启动恢复
  子断言）：W132 使用磁盘 App、正式 `coding.codex` 资源集合和 managed workspace。live Artifact owner
  使首进程 DELETE 精确返回 W131 的 409 并保留 `deleting`；随后释放 handle、关闭 Router/Services/DB，
  不再发送 DELETE。第二进程从同一 data/work 配置启动，`create_router` 在路由发布前自动恢复 deleting
  Session、删除 managed workspace 并写入 `deleted`；启动后同 key DELETE 只重放原 tombstone。
  新增场景修正资源夹具后首次及连续 **20/20**；W131 409/retry、删除顺序与 Store delete fence
  **4/4**。首个夹具用不消费 workspace 的 `chat.minimal` 被 `RESOURCE_SELECTION_UNUSED` 正确拒绝，
  失败已保留。生产代码无需修改，无模型/UI。cleanup/recovery 进程中途再次强杀、正式 UI 删除、
  非 Windows live-handle 路径及 N3/LONG/99% 仍开放，不关闭完整 ART/LIFE/OBS 或共享阶段。

- S-D09-14（LIFE-023、CONC-014、FILE-038、A05/A13/A17/A19 SQLite busy admission 子断言）：
  W133 在独立连接持有 SQLite writer lock 时，从生产 Wave2 Host 发起文件写。200 ms 时任务仍等待，
  文件与 Effect 均不存在；约 5 秒 canonical busy timeout 后返回 `CAPABILITY_UNAVAILABLE`，仍为零物理
  副作用/零 Effect。释放锁后同 operation/key 重试只执行一次并得到唯一 Returned。首版测试误用会
  重复写 tool fact 的辅助 `invoke()`，在 writer lock 下由夹具 `.expect` panic；失败已保留，改为直接
  调用生产 `Wave2HostPort::invoke` 后首次及连续 **20/20**。相邻 Store writer 等待与 W115 写入重开
  **2/2**。生产代码无需修改，无模型/UI。owner 已执行后 terminal settlement 遇锁、锁期间取消、
  进程强杀、其他 Action/平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/CONC/FILE 或共享阶段。

- S-D09-15（LIFE-006/011/023、FILE-038、CONC-014、A04/A05/A07/A17/A19 终态写锁子断言）：
  W134 在 canonical Effect 已 reserve、实际 `FileService` 已发布文件并返回 receipt 后，由独立连接持有
  SQLite writer lock。`finish_wave2_effect` 等待约 5 秒后精确返回 `CAPABILITY_UNAVAILABLE`，数据库
  仍为 Pending，已发布文件未被回滚或假报失败可重放。释放锁、用户再修改文件并关闭全部连接后，
  从同一路径重开数据库；原 key 由 durable Pending 拒绝，新 key 由 workspace resource fence 拒绝，
  用户修改保持且只有一条 Effect。首次产品运行及连续 **20/20**，W133 admission 与 W115 receipt-loss
  相邻回归 **2/2**。首个证据目录命令因 PowerShell 参数错误未启动 cargo，已单独留痕，不计产品样本。
  生产代码无需修改，无模型/UI。终态等待期间取消、精确边界强杀、DB 磁盘满/IO fault、其他 Action/
  平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/CONC/FILE 或共享阶段。

- S-D09-16（LIFE-023、CONC-014、FILE-038、A04/A10/A11/A19 busy admission future 取消子断言）：
  W135 在独立连接持有 SQLite writer lock 时，从生产 Wave2 Host 发起文件写；200 ms 时仍为零文件、
  零 Effect，随后取消并等待 Host task 得到 cancelled。释放锁并留出迟到完成窗口后仍为零副作用、
  零 Effect；同 operation/key 的显式重试随后成功一次，最终只有一条 Returned Effect。首次运行及
  连续 **20/20**，W133 busy timeout 与 W134 terminal busy 相邻回归 **2/2**。生产代码无需修改，
  无模型/UI。正式 Runtime/UI cancel 传播、终态落库等待期间取消、精确边界强杀、其他 Action/平台及
  N3/LONG/99% 仍开放，不关闭完整 LIFE/CONC/FILE 或共享阶段。

- S-D09-17（LIFE-006/011/019/023、FILE-038、CONC-014、A04/A07/A10/A11/A17/A19 终态 future
  取消子断言）：W136 在 Effect 已 reserve、实际 `FileService` 已发布文件后，以独立 SQLite writer
  lock 阻塞 terminal settlement；确认仍为 Pending 后取消并等待 settlement task 得到 cancelled。
  用户随后修改文件，释放数据库锁并留出迟到完成窗口，Effect 仍为 Pending、用户内容保持；关闭并
  重开数据库后原 key 与新 key 均被 durable/resource fence 拒绝，只有一条 Effect。首次运行及连续
  **20/20**，W134 terminal timeout 与 W135 admission cancel 相邻回归 **2/2**。生产代码无需修改，
  无模型/UI。正式 Runtime/UI cancel 与 Turn 终态的事务顺序、精确边界强杀、DB 磁盘满/IO fault、
  其他 Action/平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/CONC/FILE 或共享阶段。

- S-D09-18（LIFE-006/011/024、FILE-038、A04/A07/A17/A19 terminal Store unavailable 子断言）：
  W137 在 canonical Effect 已 reserve、实际 `FileService` 已发布文件并取得 receipt 后关闭数据库连接池；
  `finish_wave2_effect` 精确返回 `CAPABILITY_UNAVAILABLE`，已发布文件保持。用户随后修改文件，重新打开
  同一路径数据库后原 Effect 仍为 Pending；原 key 由 durable Pending 拒绝，新 key 由 workspace
  resource fence 拒绝，用户内容保持且只有一条 Effect。首次运行及连续 **20/20**，W115 receipt-loss
  与 W134 terminal busy 相邻回归 **2/2**。生产代码无需修改，无模型/UI。真实磁盘满/写入 IO fault、
  WAL/fsync 故障、正式应用 shutdown 竞态、其他 Action/平台及 N3/LONG/99% 仍开放，不关闭完整
  LIFE/FILE 或共享阶段。

- S-D09-19（LIFE-003/024、A04/A05/A17/A19 admission Store unavailable 子断言）：W138 在创建
  Session/Turn/tool causation fact 后关闭 canonical Store，再从生产 Wave2 Host 发起文件写；Host 在
  Effect ledger read/admission 边界精确返回 `CAPABILITY_UNAVAILABLE`，磁盘零文件、数据库零 Effect。
  从同一路径重开 Store 与 Host 后，同 operation/key 才首次执行并得到唯一 Returned Effect。首次运行及
  连续 **20/20**，W137 terminal Store close 与 W133 busy admission 相邻回归 **2/2**。生产代码无需
  修改，无模型/UI。真实磁盘满/IO fault、连接池关闭与在途 admission 竞态、正式应用 shutdown、其他
  Action/平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE 或共享阶段。

- S-D09-20（LIFE-003/023/024、CONC-014、A04/A11/A17/A19 Store close 与 busy admission 竞态
  子断言）：W139 先由独立连接持有 SQLite writer lock，再从生产 Wave2 Host 发起文件写并确认请求
  卡在 admission；此时启动 canonical Store close。请求按约 5 秒 busy timeout 返回
  `CAPABILITY_UNAVAILABLE`，close 随在途连接归约完成，磁盘零文件；释放锁并从同一路径重开后数据库
  仍为零 Effect，同 operation/key 才首次执行并得到唯一 Returned。首次运行及连续 **20/20**，W138
  pre-closed Store 与 W133 busy timeout 相邻回归 **2/2**。生产代码无需修改，无模型/UI。terminal
  settlement 与 Store close 竞态、正式应用 shutdown 顺序、真实磁盘/IO fault、其他 Action/平台及
  N3/LONG/99% 仍开放，不关闭完整 LIFE/CONC 或共享阶段。

- S-D09-21（LIFE-006/011/023/024、FILE-038、CONC-014、A04/A07/A11/A17/A19 Store close 与
  busy terminal 竞态子断言）：W140 在 Effect reserve 与实际 `FileService` 发布完成后，由独立连接持有
  SQLite writer lock 阻塞 terminal settlement，再启动 canonical Store close。settlement 按约 5 秒
  busy timeout 返回 `CAPABILITY_UNAVAILABLE`，close 随在途连接归约完成；文件保持已发布。释放锁、
  用户修改文件并从同一路径重开后，Effect 仍为 Pending，原 key 与新 key 均被 durable/resource fence
  拒绝，用户内容保持且只有一条 Effect。首次运行及连续 **20/20**，W139 close-admission 与 W134
  busy-terminal 相邻回归 **2/2**。生产代码无需修改，无模型/UI。正式应用 shutdown 的 Runtime/Store/
  owner 顺序、close 与 cancel 组合、真实磁盘/IO fault、其他 Action/平台及 N3/LONG/99% 仍开放，
  不关闭完整 LIFE/CONC/FILE 或共享阶段。

- S-D09-22（LIFE-011/024、FILE-038、OBS-006/015、A04/A07/A08/A17/A19 已知 owner 失败的
  terminal settlement 子断言）：W141 从正式 `workspace.files/write` handler 把“目标为目录”的确定
  owner 失败暂停在 terminal settlement 前，再以独立 SQLite writer lock 令落库超时。旧代码丢弃
  settlement 错误并只返回普通 owner 错误，数据库却保持 Pending；首次 FAIL 已保留。现新增统一
  `finish_wave2_failed_effect`，覆盖通用 managed effect 及 write/delete/artifact/stage/commit/push 的
  已知失败分支：落库成功仍返回并重放原 owner 错误；落库失败则有界返回 `CAPABILITY_UNAVAILABLE`，
  同时保留原错误 code/message、明确 terminal observation 未提交、durable effect 未决且禁止自动重试。
  修复后首次及连续 **20/20**，健康 write Rejected/replay、patch、delete、commit、push 与成功后
  terminal-busy 相邻回归 **6/6**。首个 macOS 专属 commit 过滤命令执行 0 项，已用 Windows 可执行反例
  纠正，不计通过。无模型/UI。其他 Action 的逐分支 fault injection、正式 API/UI 投影、真实磁盘/IO
  fault、其他平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/FILE/OBS 或共享阶段。

- S-D09-23（LIFE-011/024、PROC-014/039、OBS-004/006/015、A04/A08/A11/A17/A19 managed-effect
  失败结算子断言）：W142 从生产 `invoke_managed_effect` 入口 reserve `workspace.process/start` Effect，
  夹具 owner 返回确定 `PROCESS_EXIT_NON_ZERO`，同时由独立 SQLite writer lock 阻塞 terminal settlement。
  W141 的统一 helper 有界返回 `CAPABILITY_UNAVAILABLE`，完整保留原 error code 与 exit 原因并明确
  terminal observation 未提交；Effect 保持 Pending。关闭并重开数据库后，同 key 与新 key 均在 owner
  closure 前被 durable/resource fence 拒绝，两个独立调用计数保持 0。首次运行及连续 **20/20**，
  W141 write 未结算与健康 Rejected/replay 相邻回归 **2/2**。生产代码无需修改，无真实进程/模型/UI。
  正式 Runtime process owner、cancel/kill 与 terminal 的事务顺序、API/UI 投影、其他平台及
  N3/LONG/99% 仍开放，不关闭完整 LIFE/PROC/OBS 或共享阶段。

- S-D09-24（LIFE-011/024、BROW-011/012、COMP-010、SSH-003/008、OBS-009/015/016、A08/A15/
  A16/A17/A19 跨 owner 失败结算投影子断言）：W143 以 secret 开头、随后 2,000 个 emoji 的 owner
  错误触发 W141 helper 的 terminal Store failure；旧聚合消息达到 **4,291 bytes**，超过 Kernel 2 KiB
  投影上限且未在聚合边界再次脱敏，首次 FAIL 已保留。现对 action/code/owner/settlement 分别先脱敏、
  过滤控制字符并按 UTF-8 字节预算截断，最终消息 ≤2,048 bytes，仍保留原 code、脱敏标记和禁止自动
  重试结论。共享 helper 现覆盖 Browser、Computer Role 与 SSH 的确定失败分支；Role 新增独立
  `EffectSettlementFailure/CAPABILITY_UNAVAILABLE`，canonical Store/admission/terminal 错误不再误标
  provider failure。修复后首次及连续 **20/20**；settlement **8/8**、Browser feature **1/1**、Computer
  feature **1/1**、W142 与 push 相邻 **2/2**。Browser 首个未启 feature 的过滤命令执行 0 项，纠正后
  不计通过。无真实 Browser/Computer/SSH、模型/UI。三类 owner 的逐入口 fault injection、uncertain
  settlement、外部依赖与其他平台、N3/LONG/99% 仍开放，不关闭完整 LIFE/BROW/COMP/SSH/OBS。

- S-D09-25（LIFE-007/011/024、VCS-013、BROW-012/013、COMP-010、SSH-005/008、OBS-006/015/016、
  A07/A08/A15/A16/A17/A19 uncertain settlement 子断言）：W144 reserve external Effect 后模拟 owner
  已判定 `EFFECT_OUTCOME_UNKNOWN`，再关闭 terminal Store。旧路径只返回 closed-pool 错误，丢失 remote
  已收字节后断连的未知结果原因；数据库实际仍为 Pending，首次 FAIL 已保留。新增有界脱敏的
  `finish_wave2_uncertain_effect`，在 uncertain terminal 无法提交时同时保留 action、原 error code/
  message、落库失败原因，并明确 durable Effect 仍 Pending、禁止自动重试、恢复前必须核对 external
  owner。该 helper 覆盖 VCS commit/push、Browser、Computer Role 与 SSH 的 uncertain 分支。修复后
  首次及连续 **20/20**；Pending/Unknown 与 push **2/2**、Browser feature **1/1**、Computer feature
  **1/1**。无真实 remote/Browser/Computer/SSH、模型/UI。各 owner 的逐入口 terminal fault injection、
  正式 reconcile/UI 投影、外部依赖与其他平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/VCS/BROW/
  COMP/SSH/OBS 或共享阶段。

- S-D09-26（LIFE-006/011/024、FILE-038、VCS-013、BROW-012、COMP-010、SSH-005、OBS-006/015/020、
  A04/A05/A07/A17/A19 success settlement 子断言）：W145 从正式 `workspace.files/write` handler 发布
  完整文件后暂停在 terminal settlement，并以 SQLite writer lock 令落库超时。旧路径只返回
  `database is locked`，调用方无法判断物理效果已发生，Effect 实际为 Pending；首次 FAIL 已保留。
  新增 `finish_wave2_succeeded_effect`：只记录 canonical result digest，不复制潜在敏感结果正文；落库
  失败时明确 owner 已报告成功、terminal observation 未提交、durable Effect 仍 Pending、禁止自动重试
  并要求重读 owner 状态。helper 覆盖 managed effect、write/delete/artifact/stage/commit/push、Browser、
  Computer Role 与 SSH 的成功分支；patch 保留其更强的逐文件重读诊断。修复后首次及连续 **20/20**；
  managed success/replay、workspace replay、bare push replay 与 busy fence **4/4**，Browser feature
  **1/1**、Computer feature **1/1**。无正式 UI/真实外部 owner。各 Action 的逐入口 terminal fault、
  API/UI 投影、其他平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/FILE/VCS/BROW/COMP/SSH/OBS。

- S-D09-27（LIFE-006/007/011/024、FILE-038、OBS-006/009/015/016/020、A05/A07/A08/A15/A16/A17/A19
  Kernel settlement 投影子断言）：W146 将 W141/W144/W145 的 success/failed/unknown 三类内部错误送入
  正式 `kernel_error_for_action` 文件写投影。旧路径把三者都压成普通 “Workspace file operation failed”；
  success 丢失“效果已发生/不可重试”，known failure 还会因诊断中的 `changed` 被误判为 source
  precondition，首次 FAIL 已保留。现先识别三组稳定内部 marker，再返回固定、≤2 KiB 且不含 host
  路径/secret 的独立恢复指引：success 要求不得声称未变并重读；failed 明确 failure receipt 未落库且
  仍 Pending；unknown 要求核对 external owner。修复后首次及连续 **20/20**；既有普通 file、结构化
  patch、process 指引 **3/3**。无完整 Runtime invocation/API/UI。非文件 Action 的模型安全投影、正式
  tool row/UI、其他平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/FILE/OBS 或共享阶段。

- S-D09-28（LIFE-006/007/011/024、PROC-039、BROW-012、SSH-008、OBS-006/009/015/016/020、A07/A08/
  A15/A16/A17/A19 非文件 settlement 投影子断言）：W147 将 success settlement loss 送入 process
  投影、failed/unknown 分别送入普通 Browser/SSH 类投影。旧 process 文案把已成功效果改写为“owner
  未完成、可能不确定”，普通 capability 则只剩 `handler failed with CAPABILITY_UNAVAILABLE`，首次
  FAIL 已保留。三类稳定 marker 识别现提升到 Action 特判之前，统一返回固定、无 host 路径/secret 的
  success/failed/unknown 指引；普通 process spawn/cwd/control 指引和其他 typed error 保持原顺序。
  修复后首次及连续 **20/20**，W146 文件三态、process launch 与 active-execution 安全投影 **3/3**。
  无完整 Runtime invocation/API/UI。正式 tool result/event/UI、Browser/SSH/Computer 真实入口、其他
  平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/PROC/BROW/SSH/OBS 或共享阶段。

- S-D09-29（OBS-001/006/009/015/016/020、REG-004/007、A01/A03/A08/A15/A16/A17/A19 Runtime
  Kernel 投影子断言）：W148 用三套真实编译 Snapshot/ActiveSet/ToolPlan 和正式
  `KernelAgentToolInvoker` 分别注入 success/failed/unknown settlement loss。三次均只 dispatch 一次，
  Runtime typed error 保持 `CAPABILITY_UNAVAILABLE`，W147 固定恢复语义、≤2 KiB 上限和 host 路径/
  secret 隔离完整穿过 Kernel 与 Agent Runtime 适配。新增场景首次及连续 **20/20**；正常 Kernel
  invocation、未选择 capability 的 plan 拒绝及 W147 Engine 投影 **3/3**。生产代码无需修改，无事件
  Store/API/UI。正式 tool result/event 持久化、tool row/UI、不同 Action/角色及其他平台、N3/LONG/99%
  仍开放，不关闭完整 REG/OBS 或共享阶段。

- S-D09-30（OBS-001/006/009/014/015/020、A03/A08/A09/A15/A16/A17/A19 Runtime ToolResult/Event
  子断言）：W149 将 success/failed/unknown 三类 `CapabilityKernel` typed error 送入正式
  `record_tool_result`。每类均生成同 call ID、`is_error=true`、≤2 KiB 的模型 observation，保留
  `CAPABILITY_UNAVAILABLE` 与 W147 固定恢复语义；EventSink 各收到唯一、递增 step 的
  `ToolCompleted`，没有把错误变成 completed success 或丢失 call 关联。新增场景首次及连续
  **20/20**；W148 Runtime Kernel 与正常 effectful ToolResult/Event 相邻回归 **2/2**。生产代码无需修改，
  无持久 AgentSession Store/API/UI。正式事件落库/重连、tool row、模型后续行为、其他平台及
  N3/LONG/99% 仍开放，不关闭完整 OBS 或共享阶段。

- S-D09-31（OBS-001/006/009/014/015/018/020、A03/A08/A09/A15/A16/A17/A19 canonical tool
  projection 子断言）：W150 让正式 `EngineToolHost` 的 inner Kernel 返回 success settlement loss，经过
  `host_tool_dispatch/settled` 与生产 `EngineTurnJournal` 写入 canonical AgentSession Store。数据库中
  恰好一条 `tool/call-started` 和一条 `tool/result-recorded`，共享 correlation、result 精确因果指向
  call；output 为 null、error 保留 `CAPABILITY_UNAVAILABLE` 安全指引。两个独立 Store 冷读均只重建
  一条 `state=recorded` tool projection，无重复 row。新增场景首次及连续 **20/20**；取消后 owner
  settlement、history 排序、AgentSession projector **3/3**。生产代码无需修改，使用内存 SQLite，
  无进程重启/API/UI。磁盘重开、cursor 分页/重连、正式 tool row、其他平台及 N3/LONG/99% 仍开放，
  不关闭完整 OBS 或共享阶段。

- S-D09-32（LIFE-011、OBS-014/018/019/020、A03/A08/A09/A15/A17/A19 磁盘重开与 cursor 子断言）：
  W151 将 W150 的正式 EngineToolHost/Journal 场景改为磁盘 SQLite；写入 settlement error projection 后
  释放 owner、Journal 和全部旧连接，再从同一路径初始化数据库。`message_history_before(limit=1)` 每页
  都使用新的 Store，并把上一页最老 `first_seq` 作为 cursor；遍历后恰好一条 `recorded` error tool
  projection、两个 canonical tool event，total 恒定且 projection ID 无重复。新增场景首次及连续
  **20/20**；W150 memory projection、AgentSession cursor rebuild 与 cold history **3/3**。生产代码无需
  修改，仅抽取共用 test fixture。无正式 HTTP/Realtime/UI。分页期间并发新事件、cursor 重连传输、
  tool row、其他平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/OBS 或共享阶段。

- S-D09-33（OBS-014/018/020、PORT-012、A03/A08/A09/A15/A17/A19 HTTP history cursor 子断言）：
  W152 通过正式 Axum Router、本地信任认证和产品 API 创建 Provider/Preset/AgentSession，再由 canonical
  Store 写入已验证的 settlement error tool call/result。`GET message-history?page_size=1` 逐页返回稳定
  total/has_more；客户端按正式 `<created_at>:<message_id>` 生成 cursor 后完整收敛，所有 message ID
  唯一，恰好一条 `type=tool_call`、顶层与 content 均为 error，output 保留 “Do not retry” 指引。
  新增场景首次及连续 **20/20**；W151 磁盘/cursor 下层相邻回归 **1/1**。生产代码无需修改，无
  Realtime/UI。分页期间并发新事件、Realtime 重连、正式 renderer tool row、其他平台及 N3/LONG/99%
  仍开放，不关闭完整 PORT/OBS 或共享阶段。

- S-D09-34（OBS-014/018/020、PORT-012、A03/A06/A08/A09/A17/A19 events cursor 重连子断言）：
  W153 复用 W152 正式 Axum Router/本地信任 API 与 W151 磁盘 SQLite 模式；canonical Store 依次
  写入 turn/started、tool/call-started、含 CAPABILITY_UNAVAILABLE 的 tool/result-recorded 与
  turn/failed。消费方只读完第一页后，Router/Services/数据库连接全部关闭并按同一磁盘路径重建，
  模拟断连后的独立重连。按 `after_seq` 续读到空页时 seq 严格连续递增、event_id 全部唯一，tool
  call/result 与 turn/failed 各恰好一条，result payload 保留 CAPABILITY_UNAVAILABLE 与
  “Do not retry” 指引；相同 append 的写侧重放只回 duplicate，事件行数不变。事件 `after_seq`
  与 `<created_at>:<message_id>` history cursor 双向混用均被 400 拒绝，超前 cursor 同样 400
  fail-closed；重连后 `message-history?page_size=1` 收敛仍恰好一条 error tool row。新增场景
  首次及连续 **20/20**；W152 history cursor 相邻回归 **1/1**，fmt/diff 通过。生产代码无需修改，
  无正式 Realtime 传输/renderer UI。分页期间并发追加新事件、WS 推送消费方、正式 Tauri renderer
  tool row、PORT-012 watcher 丢批/乱序对账、其他平台及 N3/LONG/99% 仍开放，不关闭完整
  PORT/OBS 或共享阶段。

- S-D09-35（OBS-014/018/020、PORT-012、A03/A06/A08/A09/A17/A19 分页期间并发追加与正向
  截断子断言）：W154 在 W152 正式 Axum Router/本地信任 API 上让同一 Turn 内两个 tool call
  乱序结算（后建的 call 先落 result），再用 `GET /messages?limit=1` 正向分页并在翻页间追加
  新 Turn。**首次失败已单独保留**：正向 cursor 按 `last_seq` 过滤却按 `first_seq` 排序，
  `take(limit)` 截断后游标取页内 `max(last_seq)`，使 `last_seq` 仍落后于页内最大值的早建
  投影被永久跳过——committed 5 行只交付 4 行（证据 `run-1-first-failure.log`）。修复
  `messages_after_tx` 改按 `last_seq ASC` 排序：每个事件只更新一个 projection，`last_seq`
  天然唯一且与游标判定同键，截断页不再丢行；全部 `messages_after` 调用方均为
  find/max_by_key/自行重排，无 first_seq 排序依赖。事件 feed 在分页间追加 Turn 时 seq 严格
  连续、追加事件恰好交付一次；history 锚定窗口不受后续追加影响，新 Turn 的 turn_summary 与
  源消息只在下一次 fresh 读出现。修复后两场景首次及连续 **20/20**，W153 相邻回归 **1/1**，
  `nomifun-agent-session` 库 **76/76**，fmt/diff 通过。WS 推送消费方、正式 Tauri renderer tool
  row、PORT-012 watcher 丢批/乱序对账、history cursor 与 turn_summary 边界的更大并发矩阵、
  其他平台及 N3/LONG/99% 仍开放，不关闭完整 PORT/OBS 或共享阶段。

- S-D09-36（OBS-014/018/020、PORT-012、A03/A06/A08/A09/A17/A19 正式 Realtime/WebSocket
  消费方子断言）：W155 在真实 TCP 上启动完整产品 Router（含正式 `forward_user_events`
  桥接），owner 以桌面 webview 握手（`tauri.localhost` Origin + `Sec-WebSocket-Protocol`
  本地信任密钥）连接 `/ws`，第二用户持不同 user_id 的 JWT Bearer 连接。经正式
  `POST /turns` admission 派发注入的 Runtime mock，由 canonical Store 依次提交
  tool/call-started、含 CAPABILITY_UNAVAILABLE 的 tool/result-recorded 与 turn/failed，
  并经 stream relay 发对应 ToolCall/Error 帧。Turn 1 在 running 帧送达后断开 socket：
  settlement 在 owner 零连接期间落库，重连后 socket 无任何补推，消费方改由
  `GET events?after_seq` 逐页 replay——call/result/terminal 各恰一条且 seq 严格递增，
  `GET messages` 与 `message-history` 各恰一条保留 CAPABILITY_UNAVAILABLE 与
  “Do not retry” 指引的 error tool row；event cursor 与 history cursor 混用仍 400。
  Turn 2 验证重连后的实时投递：同一 socket 依次收到 turn.started、tool_call running、
  携带完整指引的 tool_call error、stream error 与 turn.completed state=error；同
  idempotency key 重放返回 replayed/completed 及同一 terminal 事实，不再推帧、不新增
  canonical 事件。第二用户连接全程静默。新增场景首次通过并连续 **20/20**；
  `websocket_e2e` **18/18**，W152～W154 cursor 相邻回归 **4/4**，fmt 通过。生产代码无需
  修改（正式 Realtime 链路已满足语义），无正式 Tauri renderer。renderer tool row、
  PORT-012 watcher 丢批/乱序对账、该场景的 WS lag/resync 注入、其他平台及 N3/LONG/99%
  仍开放，不关闭完整 PORT/OBS 或共享阶段。

- S-D09-37（LIFE-003/006/024、FILE-038、G0-029/030、A04/A05/A07/A17/A19 真实磁盘满与
  journal IO fault 子断言）：W157 以 `PRAGMA page_size=512`+`VACUUM`+`max_page_count`
  在 canonical Store 的真实写路径产生 SQLITE_FULL（非 mock），并以目录占用
  `agent.db-journal` 让 rollback-journal 写事务在 journal 创建处拿到真实 CANTOPEN。
  测试发现真正的产品缺陷：写事务中 SQLITE_FULL 时 SQLite 自动回滚事务，被 drop 的
  sqlx Transaction 排队的 ROLLBACK 找不到活动事务而失败，连接 worker 的
  transaction_depth 永远停留在 1——该池化连接此后所有 `begin_with` 都以
  InvalidSavePointStatement 永久失败，磁盘恢复后重试仍被 `CAPABILITY_UNAVAILABLE
  （误报为 unsettled 文本）`拒绝。修复：`AgentSessionStore::begin_write_transaction`
  识别该 desync 错误后探测并剔除失步的池化连接，再重试一次；健康连接经
  BEGIN IMMEDIATE+rollback 探测后原样归还。五条回归：admission 满盘时
  CAPABILITY_UNAVAILABLE 且磁盘零文件、零 Effect，恢复后同 key 恰一次执行；
  owner 已写文件的 success terminal 落库失败保留 Pending 且重放由 fence 拒绝（文件
  未被二次写），恢复后显式重试结算恰一次；owner 失败 terminal 落库失败保留 Pending
  与 owner 错误；uncertain terminal 落库失败保留 owner 原因，恢复后进入 Unknown；
  journal 目录占位下读路径健康、写事务 CANTOPEN，移除后同池恢复恰一次。首败保留
  （pre-fix 3/4 在恢复重试处失败、报误导性 depth 错误），修复后新增 **5/5**、
  `agent_wave2_host` 模块 **72/72**、`nomifun-agent-session` lib **76/76**，fmt 通过。
  fsync 中途故障、WAL 文件级损坏、写锁并发+满盘、正式应用 shutdown 竞态、多连接池
  拓扑、其他平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/G0/FILE 或共享阶段。

- S-D09-38（LIFE-003/023/024、CONC-014、FILE-038、G0-029、A04/A05/A07/A17/A19 writer lock +
  disk full 子断言）：W158 使用独立连接持有真实 SQLite writer lock，同时把 production Store 的单连接
  pool 限制在当前 page count。正式文件写先在 `BEGIN IMMEDIATE` 等待，200 ms 时零文件/零 Effect；
  释放 writer 后同一 admission 精确命中 `SQLITE_FULL`，仍零副作用且返回真实 full 原因。解除 page
  budget 后，同 pool/host、同 operation/key 的显式重试只执行一次并得到唯一 Returned Effect，证明
  W157 的失步连接驱逐在 busy→full 组合下没有误驱逐健康锁连接或留下 depth 污染。新增场景首次及
  连续 **20/20**；单独 disk-full、busy-timeout、rollback-journal IO fault **3/3**。生产代码无需修改，
  无 UI/模型。terminal 阶段的 busy+full、多个受限 pool 同时失步、fsync/WAL 损坏、正式 shutdown、
  其他平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/CONC/G0/FILE 或共享阶段。

- S-D09-39（LIFE-006/023/024、CONC-014、FILE-038、G0-030、A04/A05/A07/A17/A19 busy terminal +
  disk full 子断言）：W159 先 reserve canonical Effect 并由实际 `FileService` 发布文件，再让 terminal
  settlement 在独立 SQLite writer lock 后等待；释放 writer 后同一 terminal 写精确命中
  `SQLITE_FULL`。返回值同时保留 owner 已成功、full 原因与禁止自动重试，Effect 仍为 Pending，文件
  字节保持且未二次发布。解除 page budget 后，同 reservation 只补写一次 receipt，Effect 唯一变为
  Returned。新增场景首次及连续 **20/20**；单独 disk-full terminal、W158 busy-full admission、普通
  busy terminal **3/3**。生产代码无需修改，无 UI/模型。failed/uncertain terminal 的 busy+full、多个
  受限 pool 同时失步、fsync/WAL 损坏、正式 shutdown、其他平台及 N3/LONG/99% 仍开放，不关闭完整
  LIFE/CONC/G0/FILE 或共享阶段。

- S-D09-40（LIFE-007/023/024、CONC-014、G0-030、OBS-006、A04/A07/A08/A17/A19 failed/uncertain
  busy+full terminal 子断言）：W160 对 managed owner 确定失败与 external owner outcome unknown 分别
  reserve Effect，再让 terminal settlement 先等待独立 SQLite writer lock、释放后命中真实
  `SQLITE_FULL`。两类均返回 full、禁止自动重试并保留各自 owner code/message；Effect 保持 Pending。
  解除 page budget 后只补 terminal receipt，分别唯一归约为 Rejected 与 Unknown。首版 ASCII padding
  恰好落入页内空隙，settlement 成功导致夹具 `unwrap_err` 失败；首次失败保留，改用多字节大诊断强制
  跨页分配后首次及连续 **20/20**。相邻 failed-full、uncertain-full 与 W159 success busy-full
  **3/3**。生产代码无需修改，无 UI/模型。多个受限 pool 同时失步、fsync/WAL 损坏、正式 shutdown、
  其他平台及 N3/LONG/99% 仍开放，不关闭完整 LIFE/CONC/G0/OBS 或共享阶段。

- S-D09-41（LIFE-003/024、CONC-014、G0-029、A04/A05/A07/A17/A19 多 Store pool 失步恢复
  子断言）：W161 在同一 SQLite 文件上建立两个独立单连接 production Store pool、两个 Session 与
  两个物理 workspace resource，分别设置连接级 page budget；两条 admission 均真实命中
  `SQLITE_FULL`，保持零文件/零 Effect，并让各自 sqlx 连接失步。分别解除预算后并发显式重试，两个
  Store 各自驱逐自己的坏连接并各生成唯一 Returned Effect；随后改写文件再同 key 并发重放，用户字节
  保持且 Effect count 仍各为 1。前三版夹具依次触发精确 tool causation、连接级 PRAGMA 与物理资源唯一
  fence 的正确拒绝，另一次补丁定位导致编译失败；日志均保留且不计产品失败。纠正后首次及连续
  **20/20**，单 pool 满盘、busy→full、资源 fence 相邻回归 **3/3**。生产代码无需修改，无 UI/模型。
  同一 pool 内多连接同时失步、fsync/WAL 损坏、正式 shutdown、其他平台及 N3/LONG/99% 仍开放，
  不关闭完整 LIFE/CONC/G0 或共享阶段。

- S-D09-42（LIFE-006/024、CONC-014、G0-030、A04/A07/A08/A17/A19 同 pool 交错失步连接
  子断言）：W162 在三连接 production Store pool 内让两条 terminal 写真实命中 `SQLITE_FULL`，
  保留各自 Pending；解除限制后按坏/健康/坏顺序归还连接，首次重试仍报 non-zero transaction depth。
  根因是 W157 丢失报错连接所有权后另行扫描，遇首个健康连接即退出，下一次取连接仍可命中坏连接。
  `begin_write_transaction` 现保留取得的连接，ping 等待排队回滚，再只关闭深度仍非零的连接；同一
  健康连接直接进入 `BEGIN IMMEDIATE`，有界取得替代连接，业务写事务及错误不重放、不吞掉。
  两条显式 terminal 重试各归约为唯一 Rejected，健康连接的临时表标记保留。首版 admission 夹具
  未跨页分配的失败及 terminal 夹具的真实产品首败分别保留于外部 W162；修复后首次及连续
  **20/20**，Wave2 host **77/77**、Session Store **76/76**，fmt/diff 通过。无正式 UI/模型。
  全池失步、并发驱逐/池关闭、fsync/WAL 损坏、正式 shutdown、其他平台及 N3/100 seed/LONG/99%
  仍开放，不关闭完整 LIFE/CONC/G0 或共享阶段。

- S-D09-43（LIFE-008/011/024、CONC-014、G0-025/030、A03/A04/A06/A07/A08/A09/A17/A19
  全池失步与 terminal receipt 重放子断言）：W163 每 seed 让三连接 pool 全部因正式 terminal 写的
  `SQLITE_FULL` 失步，解除预算后改变连接顺序与调度时长，并发恢复三个独立 Session。恢复正确，
  但再次提交相同已结算 receipt 因新的 `recorded_at` 被误报 IdempotencyConflict，且称 effect 未决。
  `record_effect_terminal` 现于同一写事务中保留首次提交时间，再由原精确去重比较全部身份及 owner
  结果；八类身份/结果/终态变更仍冲突，原 ack 与时间不变。首版载荷 oracle 错误及产品首败分别保留于
  外部 W163 的 01/02；独立 SQL oracle 列名错误亦保留。修复后 **100/100 seed**、完整重复
  **20/20 × 100 seed**；独立核对 21 份 SQLite 快照，每份 300 条精确 owner 原因/唯一 Rejected receipt、
  2,400 条连续事件，重放新增事件为 0。Session Store **76/76**、相邻满盘 **7/7**，fmt/diff 通过。
  无正式 UI/模型。池关闭/取消与驱逐组合、fsync/WAL 损坏、正式 shutdown、其他平台及完整
  N3/LONG/99% 仍开放，不关闭完整 LIFE/CONC/G0 或共享阶段。

- S-D09-44（LIFE-007/008/011、G0-025、A04/A06/A07/A09/A17/A19 reconciliation receipt 时间
  重放子断言）：W164 在相邻 `reconcile_effect` 复现相同回执仅时间变化就误报 IdempotencyConflict，
  首败保留。terminal 与 reconciliation 现共用保留首次时间的事务写入口，全部身份和核对结果仍精确
  去重。关闭所有连接后重新打开磁盘 Store，同 receipt 返回原 record/ack，结果或资源变化仍冲突，
  Unknown 与 reconciliation fence 保留。磁盘重开首次及 **20/20**，独立核对 21 个 DB 均为唯一
  StillUncertain 回执、原时间 20、8 条连续事件、零重放新事件；Session Store **76/76**、相邻
  terminal 身份/删除 fence **2/2**，fmt/diff 通过。无正式 UI/模型；confirmed outcome 完整矩阵、
  真实外部 owner、池关闭/取消、正式 shutdown、其他平台和完整 N3/LONG/99% 仍开放。

- S-D09-45（LIFE-015/019/029、FILE-038、A04/A06/A11/A13/A17/A19 Desktop shutdown 顺序
  子断言）：W165 用正式 `DesktopServer`、真实 TCP provider/产品 API/Runtime/FileService，在唯一写入
  Returned 后保留活动模型流，再走完整 `shutdown_all`。首次及 **20/20**：流释放、唯一 cancelled、
  Returned/文件字节保留、listener 关闭，重复 shutdown 不增事件；21 个独立 DB/文件核对通过。
  正式 Tauri 1280×832、隔离 data/work/profile 的三个 UI 回合另保留：一次有效隐藏/单实例再显示时，
  同一 Turn/模型流继续且没有新调用/effect；全部三次写入最终保留，暂停回合经正式结束按钮取消。
  provider 的正常 120 秒超时、过晚停止导致的 stale UI index、夹具借用编译失败/未注册测试目标均保留，
  不计作 UI N3/完整退出通过。新增仅为最小回归及等待流夹具，无产品修复/真实模型。工具未暴露托盘
  窗口，托盘退出仍缺夹具；隔离 GUI 的最终强制进程清理单列，不代替 graceful shutdown。满盘/IO fault
  下 shutdown、真实子进程、其他角色/平台及完整 N3/100 seed/LONG/99% 仍开放，不关闭共享阶段。

- S-D09-46（LIFE-019/023/024/029、CONC-014、FILE-038、G0-030、A04/A06/A07/A11/A13/A17/A19
  shutdown 写锁与清理重试子断言）：W166 在正式 DesktopServer 的唯一写入 Returned、模型流等待后，
  用独立连接持有真实 `BEGIN IMMEDIATE` writer lock。首败：第一次退出失败，解锁后重试却成功关闭
  DB，Turn/head/owned lease 仍为 running。SDK 将 task join 当作完整清理，丢失未确认 outcome；host
  flush 队列被消费，journal uncertain 又永久拒绝原写。现 SDK 保留 exact message/outcome/cleanup
  见证，失败 flight 不释放 Session，后续显式 teardown 只补清理/回执；host 按 ack 消费队列，journal
  仅接受原 cleanup/terminal payload、kind、model identity 的相同重试，模型/工具及新记录继续拒绝。
  取消在首次 terminal commit 前仍优先，已确认 outcome 不重执行。首败与 DB 保留于外部 W166。
  修复后首次及 **20/20**，21 个 DB/文件独立核对唯一 cancelled/cleanup witness/Returned、连续事件与
  ready head；AI Agent **331/331**、journal **10/10**、正常 shutdown/pause-resume **2/2**，fmt/diff
  通过。无正式 UI/真实模型；真实满盘/WAL/fsync、lease 过期/跨重启恢复、更多取消/并发驱逐拓扑、
  其他角色/平台及 N3/100 seed/LONG/99% 仍开放，不关闭完整 LIFE/CONC/FILE/G0 或共享阶段。

- macOS M06-01 原生复核上述 DesktopServer 正常退出/SQLite writer lock 场景，前后两个源码快照各
  **2/2**，4 个独立 DB/唯一文件及 cleanup witness 一致；后续 `S-D09-47` host/journal 定向 **4/4**。
  尚无正式 renderer 或真实模型验证，未修改 Windows 原结果。

- S-D09-47（LIFE-009/011/019/023/024/029、CONC-014、A04/A06/A07/A10/A13/A17/A19 清理投影及
  初始化前取消子断言）：W167 只读并发复核发现并执行四个反例：跨 step 的旧 completion 写失败后
  cursor 提前丢失；bootstrap 第二条失败后按 sequence 跳过重试；无 active 的 terminal 仅内存确认；
  writer lock 下 cleanup 吞 claim 错误并返回成功。最早 journal 首败在外部 W167 `02`，三个 host
  首败在 `03`；编译/缺 workspace 的夹具失败单列保留。现旧 cursor 与初始化队列按持久化 ack 消费，
  preclaim 错误继续返回，无 admitted authority 的 terminal 走正式拒绝，snapshot/owner/lease 不变。
  最终 **4/4**、同构建 **20/20 × 4**；独立 SQLite **84/84** 证明文本 causation 链、63 个 host 的
  唯一 cancelled/cleanup witness、ready head、零新模型/effect；journal **11/11**、build identity **1/1**，fmt/diff
  通过。正式 Desktop shutdown 健康/写锁相邻 **2/2**，独立 DB/文件核对 **2/2**。
  pending steering 的局部 flush、commit 后 ack 丢失、跨重启/lease 过期、真实满盘/WAL/fsync、正式 UI、
  其他平台/角色和 N3/100 seed/LONG/99% 仍开放；不关闭完整 LIFE/CONC 或共享阶段。

- S-D09-48（LIFE-011/019/023/024/029、CTRL-018、A04/A06/A07/A10/A13/A17/A19）：W169
  的 pending steering 关闭路径仍把缓冲正文消费进临时 Vec，private/public 写失败及 pause 写失败后
  丢失 exact retry 来源；仅 Cleanup 分支改为按 ack 消费原保留队列。另执行 W167 相邻回归：canonical
  Cancelled 已提交、SDK 首次 driver poll 前取消时，严格 Running admission 拒绝合法收尾。新增只读
  取消证明，要求同 session/root/operation、正文、principal、frozen route/Snapshot、真实因果链、
  ready head 及 generation 0；仅确认 Cancelled0，不新 claim 或打开资源。原 Running/不同 root/
  正文/Completed 拒绝保留。三个产品首败、serde 空 Vec oracle 和编译夹具失败均保留于外部 W169。
  最终新旧相邻 **9/9**，新五项 **20/20 × 5**；独立 SQLite **105/105**，journal **11/11**、
  build identity **1/1**、fmt/diff 通过；最终组合 Desktop shutdown **2/2** 与 DB/文件独立核对通过。
  其他消息字段、claim 后未安装 ActiveTurn、跨重启、正式 UI/其他平台及完整长期门槛仍开放。

- S-D09-49（G0-025、CTRL-018、LIFE-019/029、A03/A06/A09/A10/A14/A17/A19 取消回执输入匹配
  子断言）：W171 原 gen0 取消证明只核对正文，同一 cancelled root 的附件、技能提示和 origin 被改动
  后，cleanup/terminal 仍成功确认。证据根为外部 `2026-09-30/windows/w171-cancel-delivery-identity`，
  两个首败及 DB 保留于 `01-first-product-run`。现复用 accepted delivery 的有界解析器比较
  files/inject_skills 的完整数组及顺序、origin 的精确 Option；只有 absent/null 表示原 None，
  不读附件、不授予 Skill 或打开资源。合法原取消经真实 SDK send/cancel/teardown保持原 terminal。
  首轮 **2/2**、同构建 **20/20 × 2**、独立 SQLite **42/42**，原清理相邻 **9/9**、fmt/diff通过。
  全部恢复为原 ready head、gen0/无 owner、零模型/effect/事件增量；首次失败不改写。wrapped delivery、
  非空已选择 Skill、claim 后未安装 ActiveTurn、正式 UI/其他平台及完整长期门槛仍开放。

- S-D09-50（LIFE-011/013/014/019/029、G0-025/030、A03/A04/A06/A07/A10/A12/A13/A14/A17/A19
  claim 与 Runtime 准备取消子断言）：W172 手动 poll 原准备 future 命中实际 SQLite await，claim
  已落盘、ActiveTurn 尚未安装；canonical cancel 后丢弃 caller，原 cleanup 再读 Running receipt
  失败，无法收尾。证据根为外部 `2026-09-30/windows/w172-claimed-preparation-cleanup`，
  首败日志/DB留在 `01-first-product-run`。
  准备改为宿主持有的任务，清理先等其 completion；claim journal 先发布到 ActiveTurn，再 await
  budget/重新验 receipt。取消不新开资源，原 root/holder/generation/fence 结算；公共 open_journal
  仍刷新预算，未放宽 gen0 特例或借用另一个 execution owner。
  首次 **1/2**（普通 drop 旧码即通过），最终 **3/3**、**20/20 × 3**、独立 SQLite **63/63**：42个
  原 owner cleanup witness、21个 foreign owner 拒绝、唯一原 claim、零新模型/effect；相邻 **11/11**、
  实际 pause/resume 和冷启动恢复 **2/2**、fmt/diff通过。未覆盖 preparation panic/commit ack失落、
  长附件准备与超时、更多并发恢复拓扑、正式 UI/其他平台角色和完整长期门槛；共享阶段继续开放。

- S-D09-51（LIFE-020、OBS-014、A05/A10/A13/A17/A19 cancelled 冷读与页面重连）：W182
  正式 Tauri 配对构建在 W165 隔离 data/work/profile 原样本上完成一次冷启动、一次页面重连。
  三个已取消 Turn 显示“已取消执行”，原写入显示“已编辑1个文件”；3 cancelled/3 returned/
  213 events及head完整摘要、文件digest均不变，原provider端口健康监听且新增模型请求0。
  首次UI构建因已声明的plugin-fs未安装失败；冻结锁文件安装4包后构建通过，首败及辅助
  脚本错误留于外部 `2026-09-30/windows/w182-tauri-cancelled-cold-load`，无产品首败或源码修复。
  GUI在quiescent后做自有进程强制清理，provider正常HTTP关闭，GUI/profile/provider均0；
  graceful quit未验。N3冷启动、真实模型/其他角色/macOS及长期门槛仍开放，不关闭完整Case。

- S-D03-59（FILE-020/025/039、A05/A13/A14/A17/A19 recovery 名称窗口子断言）：W173 对
  Windows 原生恢复的最后 identity check→rename 窗口补两项回归；原实现首次 **2/2**，无产品
  新首败或修复。backup 原对象在恢复过程中两种 POSIX remap 均被 sharing violation=32 拒绝，
  释放 guard 后相同 remap 确实可执行；并发建立的 foreign hardlink target使恢复原子拒绝，原 backup
  与并发对象身份/字节保留。首次及 **20/20 × 2**，42个独立磁盘/目录项及hardlink身份核对通过，
  cleanup 模块 **4/4**、fmt/diff通过；外部证据 `2026-09-30/windows/w173-file-recovery-window`。
  仅补最小回归，不把 Windows 结果折算为 macOS 或完整 FILE Case；真实 IO fault、更多恢复组合、
  正式 UI/角色及 N3/100 seed/LONG/99%仍开放，共享阶段继续推进。

- S-D03-60（ART-001/002/005、FILE-038/039、A04/A05/A07/A13/A14/A17/A19 Artifact 发布字节
  子断言）：W174 两个首次反例均失败；暂存或 post-link blob 同 inode/同长度改成错误字节后，
  原 publish 仍返回成功及旧 digest/chunk index。证据根为外部 `2026-09-30/windows/w174-artifact-publication-bytes`，
  首败及坏 blob 留在 `01-first-product-run`。新发布复用现有 reader 完整 hash 校验及大小准入边界，
  再比 staged 身份/大小；仅真实验证结果进入 receipt/cache。原确认 rollback 与 unknown 分支不变。
  Windows 模块 **18/18**、新两项 **20/20 × 2**、独立磁盘 **42/42**；WSL ext4 模块 **21/21**、
  两个原生场景独立核对通过，fmt/diff通过，不代判macOS。分页复用仍无额外full scan；新发布增加
  一次完整校验并如实计入IO计数。staging/drop 名称复用、rollback check→unlink、并发增长预算、真实满盘/fsync、
  正式 UI/模型/其他角色平台和完整长期门槛仍开放，不关闭完整Artifact或共享阶段。

- S-D03-61（ART-002/004/005/006、A05/A07/A13/A14/A15/A17/A19 Artifact 并发增长预算
  子断言）：W175 在真实 metadata→read 窗口把8字节源/blob追加为131,080字节，原实现虽最终拒绝，
  两条路径均完整扫描新增内容，首败 **0/2** 与磁盘/IO计数留在外部 W175。现 stage与verified load
  都按观察大小加1字节封顶；检测增长立即Conflict，禁止额外暂存/hash/cache，不把截断当成功。
  首轮模块 Windows **20/20**、新两项 **20/20 × 2**、独立磁盘/计数 **42/42**，每项131,080→9字节；
  WSL ext4模块 **23/23**与两原生场景独立核对、fmt/diff通过，不代判macOS。原增长数据保留、
  零publication temp，正常分页/cache复用不变。证据 `2026-09-30/windows/w175-artifact-growth-budget`。
  连续多进程写、上限512MiB样本、staging/drop及rollback名称竞态、真实IO fault、完整UI/角色平台
  和长期门槛仍开放，不关闭完整ART或共享阶段。

- S-D03-62（ART-001/002/005/007、FILE-038/039、A04/A05/A07/A13/A14/A17/A19 stage名称
  清理子断言）：W176 三项产品首败：旧 StagedArtifact Drop 按名称删除，外来异字节或同字节异
  inode 都被误删，公开 staging失败仍known rejection。首败/现场留在外部 W176 `01`/`02`。
  stage清理保留原identity，拒绝外来名称；Windows以relative Dir打开deny-delete guard，身份核对后
  原生handle删除；Unix核对observed inode但最窄check→unlink仍未关闭。publish及stage failure显式
  检查cleanup，未确认返回原unknown前缀并保留原原因/残留，Drop不再盲删或吞掉失败。
  Windows模块 **24/24**、新四项 **20/20 × 4**、独立磁盘/identity **84/84**；WSL ext4模块
  **26/26**及3个原生场景核对、fmt/diff通过，不代判macOS。Windows最后核对→delete remap以32
  拒绝，释放后相同remap可执行。startup stale cleanup、Unix最终窗口、rollback check→unlink、
  真IO fault、完整UI/模型/其他平台角色和长期门槛仍开放；不关闭完整Case或共享阶段。

- S-D03-63（ART-001/002/005/007、FILE-038/039、A04/A05/A07/A13/A14/A17/A19 rollback
  最后名称窗口）：W177 首次 **1/2**，原 helper核对identity后仍按路径unlink；真实POSIX remap
  置换目标后删除外来对象却返回confirmed。证据根为外部 `2026-09-30/windows/w177-artifact-rollback-window`，
  首败/磁盘留在 `01-first-product-run`，已有foreign precheck旧码通过。Windows
  回滚改为pinned Dir相对DELETE/metadata、deny-delete guard，核对同句柄identity后原生删除；
  保留preexisting foreign拒绝和missing target/unknown分支，未扩大权限。
  Windows模块 **26/26**、新两项 **20/20 × 2**、42个独立磁盘核对、fmt/diff通过；WSL ext4原
  模块 **26/26**兼容回归通过，不计Unix新窗口或macOS。失败原源保留、零unowned deletion。
  startup stale cleanup、Unix stage/rollback check→unlink、真IO fault、完整UI/角色平台和完整
  长期门槛仍开放，不关闭完整Case或共享阶段。

- S-D03-64（ART-005/006、A05/A13/A14/A15/A17/A19 512MiB超限准入）：W178真实 sparse
  source/blob各536,870,913字节，均在full scan前按原上限拒绝，源为BadRequest、blob为Conflict，
  不是unknown/截断成功；对象和大小保留，零publication temp。首次 **2/2**、最终源三个独立
  样本 **3×2/2**及六份磁盘/稀疏属性核对，Windows模块 **28/28**、WSL ext4模块 **28/28**和两
  原生超限文件核对、fmt/diff通过，不代判macOS。无新产品FAIL或生产修复，仅必要边界回归。
  完整现场/IO计数/源码及binary身份在外部 `2026-09-30/windows/w178-artifact-size-admission`。
  恰好上限512MiB完整发布、持续增长、stale cleanup/Unix最终窗口、真IO fault、正式UI/模型/角色
  平台及长期门槛仍开放，不关闭完整ART或共享阶段。

- S-D03-65（ART-001/004/006、A05/A13/A14/A15/A17/A19 上限正向子断言）：W179 对恰好
  536,870,912 字节的真实文件完成发布、512 页回读及空 EOF；拼接 digest、receipt 和独立磁盘
  hash 一致，分页未增加 full scan，page IO 恰好等于文件大小，publication temp 为零。
  Windows 三个独立样本 **3/3**、相邻超限 **2/2**，WSL Ubuntu ext4 **1/1**，八次外部文件
  hash 及 fmt/diff 通过。首次即通过，无新产品 FAIL/生产修复；重型回归默认 ignored，本批
  显式执行，未将跳过计作通过。证据在外部 `2026-09-30/windows/w179-artifact-exact-budget`。
  持续增长、startup stale cleanup、Unix 最终窗口、真实 IO fault、正式 UI/模型/角色/macOS
  及长期门槛仍未覆盖；不关闭完整 ART Case 或共享阶段。

- S-D03-66（ART-001/005/007、A05/A07/A13/A14/A17/A19 冷清理身份子断言）：W180 冷重开
  删除 live cleanup 已保留的外来 stage，同字节异对象也被误删；首次 **0/2** 保留于外部
  `2026-09-30/windows/w180-artifact-cold-cleanup/01-first-product-run`。另保留预先打开的 Store
  缓存完成标记而继续发布的两项首败于 `05-first-preexisting-owner`。
  stage 创建时持久化 hardlink 见证及 native ID/birth identity；重开核对记录和原对象，缺失、
  替代或无见证的旧 temp 保留并阻止发布，已有产物可诊断读。每次发布在共享 lease 内重新核对，
  清理上限仍为 64；真实子进程无析构退出验证正常遗留及 orphan witness 回收。
  Windows 模块 **33/33**、新五项 **3×5/5**、21份独立磁盘/native ID 核对；WSL ext4 模块
  **33/33**及5份原生核对、fmt/diff通过。辅助 oracle 两次长路径失败另存，修正路径接口后通过。
  未验见证创建/fsync中途断电、见证及目录同时伪造、Unix最后unlink窗口、正式host fence/UI、
  macOS/其他角色及长期门槛；不关闭完整Case或共享阶段，无模型/UI调用。

- S-D03-67（ART-007、LIFE-007、A05/A07/A13/A14/A17/A19 宿主 cleanup unknown fence）：
  W181 使用真实 Artifact owner、canonical 磁盘 DB 和 Wave2ApplicationHost 调用；unwitnessed
  temp 返回 EFFECT_OUTCOME_UNKNOWN，唯一 Effect 保留 pending、无 terminal/settled_at。
  原/新 key、冷 DB 重开、文件现场修复、其他 Session 及文件写均被 fence；已有产物诊断读正确，
  原 effect 完整记录不变，无后来源 blob 或额外 effect。首跑 **1/1**，补独立原对象证据后 **1/1**、
  N3 **3/3**；四份独立 SQL/磁盘/native ID 核对通过。相邻六项结果通过；过滤词按 OR 误扩大
  到79项，93秒自然结束 **79/79**，日志留存并复用相关结果，没有再次执行。
  原实现首次满足，无新产品 FAIL/生产修复，仅最小宿主回归，fmt/diff通过；证据在外部
  `2026-09-30/windows/w181-artifact-host-cleanup-fence`。正式 Tauri UI/Runtime/模型/其他角色、
  macOS及长期门槛仍未覆盖；不关闭完整 Case 或共享阶段。

- S-D03-52（FILE-040、PORT-012、A05/A15/A17/A19 watcher 残余丢失信号子断言）：W156 修复
  `NomiWorkspaceWatchContext`/`WatchQueue` 两处静默丢失。其一：native change 事件不带任何
  path、或全部 path 落在 watched root 之外时（可能是跨越边界的 rename 尾部），原先不产生
  事件、不计 dropped 也不置 rescan——变化完全消失；现 `record_native` 对零可归因 in-root
  path 的 change 类事件置 `rescan_required`（`.nomifun` owner 组件、非法名计数 dropped、
  空 relative→rescan 等已归因路径仍算 handled，不放大信号）。其二：`pre_turn_context`
  drain 后 `validate`/`serialize` 失败曾直接返回 `None`——已 drain 批次在投递侧丢失且对
  后续 Turn 零信号，比 overflow 丢弃（保留 dropped 计数）更糟；现 `WatchQueue::take_batch`
  在投递失败时把 `rescan_required` 滞留回队列，下一批强制全量对账。新增三回归：
  零 path/全 out-of-root/mixed path 的归因语义；debounce 窗口外的重复与乱序事件按到达
  顺序原样送达且不产生 dropped（批次不承诺顺序/唯一性，磁盘仍是唯一事实）；注入一个
  schema 拒绝事件模拟 drain 后投递失败，断言下一 Turn 批次携带空 events+dropped=0+
  rescan_required=true。首败先保留（仅测试打回旧码两条均失败），修复后 **22/22**、
  连续 **20/20**，fmt 通过。正式 UI 全量对账、模型是否遵循重读提醒、批次 drain 后未被
  消费（context contributor 为 fire-and-forget）、macOS、100 seed/LONG/99% 仍开放，
  不关闭完整 FILE-040、PORT-012 或共享阶段。

- S-D03-51（FILE-019/020/038、LIFE-006/007、A05/A13/A17/A19 文件发布 receipt 丢失子断言）：
  W115 将原先只复用内存 Store、并用 `std::fs::write` 模拟发布的回归升级为磁盘 SQLite 与实际
  `FileService` owner。canonical Effect reserve 后文件 owner 成功原子发布并返回 receipt，夹具故意
  不写 terminal receipt，释放 host/Store 并关闭全部数据库连接；随后用户把目标改成新字节。重新打开
  数据库与 host 后，原 key 由 durable Pending 直接拒绝，新 key 由同一 workspace resource fence 在
  owner 前拒绝；数据库仍只有一个 Pending Effect，用户新字节未被覆盖。升级断言首次及连续
  **20/20**，相邻 App 文件动作/fence **2/2**、文件 owner 写入 **1/1**。最初短名配 `--exact` 导致
  0 项执行，已保留并用完整模块名纠正，不计通过。生产代码无需修改，无模型/UI。文件 worker 或
  SQLite terminal commit 中的进程强杀、patch/delete 同类磁盘重开、文件结果人工核对、其他平台及
  N3/LONG/99% 仍开放，不关闭完整 FILE/LIFE 或共享阶段。

- S-D03-52（FILE-027/031/038、LIFE-006/007、A05/A13/A17/A19 多文件 patch receipt 丢失子断言）：
  W116 在独立磁盘 SQLite 中 reserve 一个两文件 patch Effect，再由实际 `FileService` owner 原子把
  `alpha/beta` 发布为 `ALPHA/BETA`；owner 返回两文件 receipt 后，夹具故意不写 canonical terminal。
  关闭全部数据库连接后，用户把两文件分别改成新的字节；新 host 中原 key 由 durable Pending 拒绝，
  新 key 使用能合法匹配当前用户文本的 patch，仍由 workspace resource fence 在 owner 前拒绝。
  Effect 总数保持 1/Pending，两份用户字节均未被覆盖。新增断言首次及连续 **20/20**；App patch
  正常/并发/全有或全无/权限路径 **4/4**，W115 写入重开 **1/1**，文件 owner patch **2/2**。
  生产代码无需修改，无模型/UI。文件 worker/SQLite terminal commit 进程强杀、delete 同类磁盘重开、
  patch 结果人工核对、其他平台及 N3/LONG/99% 仍开放，不关闭完整 FILE/LIFE 或共享阶段。

- S-D03-53（FILE-034/038、LIFE-006/007、A05/A13/A17/A19 delete receipt 丢失子断言）：W117
  在磁盘 SQLite 中 reserve 文件 delete Effect，再由实际 `FileService` owner 删除旧 `victim.txt`；owner
  返回路径 observation 后故意不写 terminal receipt，并关闭全部数据库连接。用户随后按同名重建不同
  字节。新 Store/host 中，原 key 由 durable Pending 拒绝，新 operation/key 也在 delete owner 前由
  workspace resource fence 拒绝；Effect 保持唯一 Pending，同名重建文件完整保留。新增断言首次及
  连续 **20/20**；App 普通/部分递归删除 **2/2**，FileService 删除、名称置换及 Windows ACL **8/8**。
  生产代码无需修改，无模型/UI。文件 worker/SQLite terminal commit 进程强杀、目录树成功删除后的
  同名重建磁盘重开、delete 结果人工核对、其他平台及 N3/LONG/99% 仍开放，不关闭完整 FILE/LIFE
  或共享阶段。

- S-D03-54（FILE-034/037/038、LIFE-006/007、A05/A13/A17/A19 非空目录 delete receipt 丢失
  子断言）：W118 在磁盘 SQLite 中 reserve 非空目录 delete Effect，由实际 `FileService` owner 成功
  删除含嵌套文件的旧树；owner 返回路径 observation 后不写 terminal receipt，并关闭全部数据库连接。
  用户随后重建同名多层目录和不同内容。重开后原 key 与新 operation/key 均在 delete owner 前被
  durable workspace fence 拒绝；Effect 保持唯一 Pending，重建树完整保留。新增断言首次及连续
  **20/20**；W117 文件重建/部分递归删除 **2/2**，FileService 目录置换与 Windows 空目录路径
  **3/3**。生产代码无需修改，无模型/UI。文件 worker/SQLite terminal commit 进程强杀、目录结果
  人工核对、其他平台及 N3/LONG/99% 仍开放，不关闭完整 FILE/LIFE 或共享阶段。

- S-D03-55（ART-001/003/005、LIFE-006/007、A05/A13/A17/A19 Artifact publish receipt 丢失
  子断言）：W119 在磁盘 SQLite 中 reserve publish Effect，由实际 `WorkspaceArtifactStore` 把源文件
  发布为 content-addressed blob；owner 返回 artifact ID 后故意不写 terminal receipt，并关闭全部数据库
  连接。源文件随后改成不同内容。重开后原 key 与携带新 digest 的新 operation/key 均在 Artifact owner
  前被 workspace fence 拒绝；原 artifact 字节/ID 保持且目录只有一个 64 位内容对象，新 digest 对象
  不存在，Effect 保持唯一 Pending。首个新增断言因夹具 reserve 未使用生产规范化的 optional-null input
  而得到正确 `IDEMPOTENCY_CONFLICT`，失败已保留；修正夹具后首次及连续 **20/20**。App Artifact
  **2/2**、ArtifactStore 发布/读取/篡改/清理/并发 **15/15**。生产代码无需修改，无模型/UI。Artifact
  worker/SQLite terminal commit 进程强杀、owner 核对/恢复、Session 删除与 reader 并发、其他平台及
  N3/LONG/99% 仍开放，不关闭完整 ART/LIFE 或共享阶段。

- S-D04-23（`CMD-149/150`、A02/A08/A13/A17/A19 process host OS 映射子断言）：旧标准工具
  description 在 Runtime 侧写入 host OS，renderer/client OS 不参与，但 Kernel 在 owner dispatch 前
  没有一致性核对；故意错配只能靠实际命令失败暴露，无法记录要求的稳定错误。现让 exec/start 两个
  launch 工具都携带同一 Runtime host 声明；完成 Snapshot、active set 与 binding 校验后、任何 owner
  调用前比较声明与当前执行 host。错配返回 `HOST_OS_COMMAND_MAPPING_ERROR`、
  `status=not_executed`、`user_code_started=false`，并明确不得盲试另一平台；合法 Schema、权限和 Action
  不变。Engine **33 通过 / 1 ignored**、同步远端后 Runtime **187/187**。macOS 正式 Tauri 中，即使 accepted input
  声称客户端为 Windows，首次仍使用字面 `pwd [-P]`，单 effect/零错误且输出为精确 workspace。
  真实异构客户端、远端 owner OS attestation、Windows/Linux 负向正式 UI、N3/100 seed/LONG/99%
  仍开放，不关闭完整 CMD-149/150 或共享 D04。

- S-D04-21（`CMD-139`、`PROC-014/034～037/046`、A01/A02/A11/A13/A17/A19 托管进程
  生命周期子断言）：macOS 正式 Tauri 连续保留了 planner 拆分 handle、阻塞 start、跳过 poll、
  shell/raw PID 替代、取消终态误报，以及模型在合法启动前调用 File/`exec_command` 探查的首次失败；
  后者把字面 `ls -la` 当 executable，虽随后恢复，正向 Case 仍失败。现由 planner 在同一 Agent step
  spec 同时包含 start/poll/cancel-or-close 时持久化 `managed_process_only` 减权标记；Session 只物化
  已继承的 start/poll/input/close/resize/cancel Action，File、exec、VCS、Artifact、发现和委派均不进入
  工具面。Kernel 与 Runtime policy 对 reaped cancellation/terminal poll 使用相同 action-specific
  语义，不再把 `success=false` 的“已不运行”事实误算成命令失败或强制重规划。四层减权回归及
  Runtime/Engine 全组通过；macOS 新 Session 精确 start→poll READY→cancel/reaped、零工具错误、
  2 effects、UI 1/1。Windows、交互 stdin/PTY、20 次/N3、100 seed 与 LONG/99% 仍开放，不关闭共享 D04。

- S-D02-16（`CTRL-006/007`、`CMD-139`、A02/A05/A17/A18/A19 process-chain 完成证据子断言）：
  macOS 首次完成链先因 start/poll 在 cancel 时已不位于顶层可用证据而反复失败，后又把尚无合法 ID 的
  可选 `requirement_ids` 发送为空数组；严格 Schema 正确拒绝，但形成正向 UI bad case。现仅在同一
  process、同一 workspace epoch、零省略交互且 terminal cancel 明确 `cleanup.reaped=true` 时延续该
  chain 的精确 observation；其他旧/失败/跨 epoch 证据仍失效。动态完成 Schema 在尚无 requirement
  ID 时移除该属性，由 owner 收尾时生成完整输入义务；已有 ID 时只暴露精确 enum，显式空数组、未知
  ID、错误类型仍拒绝。Runtime **184/184**；macOS 修复后一次 completion report 成功、4 模型 step、
  零 control error。其他多命令/并发链、Provider/角色/平台及完整 CTRL 矩阵仍开放，不关闭共享 D02。

- S-D02-17（CTRL-006/007、A05/A09/A15/A17/A18/A19 多命令证据关联子断言）：W101 首次正式
  Tauri 中 ALPHA/BETA 两个命令均一次 exit 0/reaped，但第二个 opaque command 推进 workspace epoch
  后，第一条已结算命令从 `available_evidence` 消失。模型能看到两个结果正文，却只得到 BETA 顶层
  call ID 并误认作 ALPHA；连续三次耗尽 4,096 输出 token，Turn 以 `NOMIFUN_TASK_INCOMPLETE` 失败，
  旧 W75 语义关联缺口得到完整复现。现仅将“当时已可用、exit 0、cleanup 已证明、无省略交互”的
  命令自身 scope/exit/output 保持为不可变证据；它不延续任何文件或当前工作区状态。新回归首次
  **0/1**，修复后 **20/20**，Agent Runtime **192/192**。同一 UI 夹具最终 3 步完成、132 条事件，
  两个 criterion 各只引用匹配 call ID，零截断、2 个 returned effect，独立 31 项断言、工作区和清理
  均通过。非零/取消/交互进程链、64 项窗口淘汰、压缩恢复、其他 Provider/角色/平台仍开放，不关闭
  完整 CTRL 或共享阶段。

- S-D02-18（CTRL-006/007、PROC-015、OBS-004、REAL-004、A05/A08/A09/A17/A18/A19 失败命令
  证据子断言）：W94/W96 的非零/timeout 只能用 unverified 描述，因为 CompletionTracker 把
  `is_error=true` 与“不是证据”混为一体。W102 新回归首次 **0/1**；首版修复虽在合成夹具通过，正式
  Tauri 的真实非零命令没有 workspace-current provenance，动态 Schema 仍给 evidence IDs
  `maxItems=0`。模型一次 report 被拒后进行 4 次只读历史检索、7 次输出截断，并在 step 12 重跑命令；
  为控制用量在 step 17 从正式 UI 取消，完整 `FAIL_CANCELLED` 保留。最终按同一 launch call identity、
  已知 exit 和 `cleanup.reaped=true` 认定结构化终态事实；非零只证明该失败，不证明任务成功或当前文件
  状态。修正后的真实形状回归 **20/20**，Agent Runtime **193/193**。最终 UI 一次 exit 7、一次计划、
  一次 supported completion，共 3 步/100 事件，criterion 精确引用失败 call，tool/command 计数均 1；
  合并远端共享 host 改动后以另一隔离 Session 重跑得到相同结果。独立 27 项断言、工作区和清理
  通过。timeout/cancel/交互终态、其他 Provider/角色/平台与长期矩阵仍开放，不关闭完整
  CTRL/PROC/REAL 或共享阶段。

- S-D02-19（CTRL-006/007、G0-023、PROC-048、REAL-023、A05/A08/A09/A11/A13/A17/A18/A19
  timeout 终态证据子断言）：W103 后续审计发现 W96 的 timeout criterion 与 recovery criterion 都引用
  恢复命令 call ID；原 35 项断言只证明计数/清理，未证明语义关联，因此该子断言追记失败。新回归
  首次 **0/1**，确认 `timed_out + cleanup.reaped=true` 仍不在证据枚举。现让同一 launch call 的已回收
  timeout 保留为自身 scope/terminal/output 事实；不把 timeout 变成成功，也不延续文件状态。修复后
  **20/20**，Agent Runtime **194/194**。正式 Tauri 只执行一次 250 ms timeout，随后一次计划、一次
  supported completion，共 3 步/101 事件；criterion 精确引用 timeout call，tool/command 计数均 1，
  零恢复命令/历史检索/截断，独立 29 项断言、工作区和清理通过。cancelled/lost/force-kill、其他
  Provider/角色/平台及长期矩阵仍开放，不关闭完整 CTRL/PROC/REAL 或共享阶段。

- S-D02-20（CTRL-006/007、CMD-139、PROC-033、A02/A05/A09/A13/A17/A18/A19 Windows 托管进程链
  子断言）：W104 用正式 Tauri 和 `step-3.7-flash` 补 S-D02-16 的 Windows 路径。模型首次精确执行
  `start_process`→`poll_process`→`cancel_process`→`report_completion`；同一 process ID 的 poll 输出
  `W104-READY`，cancel 返回 `state=cancelled`、`cleanup.reaped=true`。唯一 supported criterion 按顺序
  引用 start/poll/cancel 三个顶层 call ID，没有复制最新 ID、搜索历史或重复清理。单回合 4 步、143 条
  canonical 事件，start/cancel 两个 managed effect returned，poll 不制造 effect；独立 30 项断言、
  工作区、进程和应用清理及正式备份通过。生产代码无需修改。PTY/stdin/resize、失败 poll/cancel、N3、
  其他 Provider/角色/平台与长期矩阵仍开放，不关闭完整 CMD/PROC/CTRL 或共享阶段。

- S-D02-21（CTRL-006/007、PROC-025/027、REAL-005、A02/A05/A09/A13/A17/A18/A19 stdin 链、
  游标与完成证据子断言）：W105 首次正式 Tauri 的 start→input→close→poll 已退出 0/reaped，但 canonical
  poll Schema/宿主丢掉模型侧已有的 `cursor`，close 先消费回显后 poll 得到空输出；完成账本又只延续
  当前 epoch，导致 start/input 两个精确链调用不在顶层证据。现 canonical poll 贯通非负 cursor，核心
  owner 支持从显式 cursor 重放保留输出，终态缓存也按请求游标重读；只有同一 process 的完整、连续、
  零省略 provenance 链可跨自身 interaction epoch 延续，其他旧观察不恢复。另将单行输入建模为可选
  `append_newline=true`，宿主在原始 input 后只追加一个 `0x0A`，默认 false 且 1 MiB 上限含追加字节。
  原始及强化字节夹具保留三次模型省略 LF 的失败，其中原始 12 字节被宿主如实回报，证明传输没有
  隐式补齐；最终正式 Tauri 由 `step-3.7-flash` 精确发送 flag，poll(cursor=0) 返回末尾 `0A`、长度 13，
  completion 按顺序引用 start/input/close/poll 四个顶层 call ID。精确链回归 **20/20**，Runtime
  **195/195**、Wave2 **22/22**、Engine **29 通过 / 1 ignored**、App host **3/3**；正式 Session
  5 步/177 事件、独立 34 项断言、工作区/进程/应用清理与备份通过。PTY/resize、并发 close/poll、
  retained-base loss、其他 Provider/角色/平台及 N3/100 seed/LONG/99% 仍开放，不关闭完整 Case 或共享阶段。

- S-D02-22（OBS-008/014、LIFE-015/019、COMP-010 的冷启动暂停投影子断言，**已验证**）：
  macOS M04-13 的正式 owner-crash 冷启动在第三次受控 recovery admission 失败后，已原子
  提交 `effect/uncertain`、recovery-blocked 和 `turn/paused`；会话详情主动重读 canonical 状态，正确显示
  “执行已暂停”并禁用发送，但同一截图中的侧栏悬浮卡仍显示“活跃状态 运行中”。根因是 startup
  quarantine 只持久化暂停，没有向精确 owner 发送 `turn.paused`；侧栏在 quarantine 前已完成首次
  `/api/agent-sessions` 读取，因而没有触发现有 canonical refresh。静态 HoverCard 测试用预填
  `execution_phase=paused`，未覆盖这个 lifecycle 缺口。首次正式证据保留于
  `2026-09-30/macos/m04-computer-crash/run-005-formal-crash/`。
- 现抽取并复用唯一 canonical pause wire helper；仅当 `quarantine_native_recovery` 确认提交后，使用
  原 Turn 的 `source_message_id` 向 Session owner 单播 `turn.paused`，不广播、不生成新输入，也不改变
  effect/Turn 状态机。startup recovery 集成回归在 route 创建前订阅 user event，验证 exact owner、
  Session、36 字符 root message ID、`execution_phase=paused` 及 `can_send_message=false` **1/1**；暂停
  notice/HoverCard/侧栏 canonical refresh **4/4**。Developer ID-signed arm64 post-fix App 已从原 seq-67
  crash image 复跑：夹具首次因复制旧 work-root marker 被产品 fail closed，修正隔离数据绑定后 DB
  再次到 paused/unknown，日志在第三次拒绝后立即出现新的 `/api/agent-sessions` 读取，证明 Realtime
  事件已触发现有侧栏 refresh。解锁后同一正式 UI 的详情页仍显示暂停/发送禁用，侧栏悬浮卡由首败的
  “活跃状态 运行中”变为精确“活跃状态 执行已暂停”。产品按钮结束回合后 head ready、seq 71
  cancelled，原 unknown receipt 保留；最终 DB `ok`，App PID 与 listener 归零。其他平台、断线时事件
  丢失与完整 OBS/LIFE 矩阵仍开放。

- S-D02-23（`OBS-006/007/014/015`、`LIFE-015/016` 的暂停清理状态投影子断言，**代码与确定性
  回归已验证**）：macOS M04-23 的正式 StepFun 限额暂停已持久化
  `execution_pause.cleanup_proven=true`、Runtime idle/head paused，UI 却显示“资源清理状态尚未确认”。
  根因不是清理失败，而是 `turn.paused` 后的立即 authority GET 可先读到结构完整但清理字段尚未升级的
  暂停投影；reconciler 原先在首个 pause snapshot 立即退出，后续 canonical `true` 再无机会被采用。
- authority reconciler 现只暂存首个未证明快照；同一 Turn/reason/paused-at 的下一次 snapshot 若已
  cleanup-proven 则以升级值为准，若连续两次仍未证明才按真实未知状态展示。不同暂停会重新确认，
  durable 未证明状态不会被吞掉，既有 pause fence、发送禁用与 capped backoff 均不放宽。首次错误测试
  调用因未加载 UI Happy DOM 出现 2 个 `document is not defined`，原样保存在
  `2026-09-30/macos/m04-pause-cleanup-projection/run-001-tests/`；改用仓库 `ui/bunfig.toml` 后相关
  reconcile/hook/notice **18/18**、UI typecheck、desktop UI boundary 与 diff check 通过。
- 本批未再调用付费 Provider，也未以组件测试代替 post-fix 正式 Tauri 视觉复核；因此只关闭公共竞态
  根因及确定性子断言，M04-23 的 terminal、N3/20/99%、断线/多窗口投影与完整 D02/OBS/LIFE 仍开放。

- **S-D02-24**（`OBS-006/007/014/015`、`LIFE-015/016` 的页外暂停证据子断言）：后续 M04-26
  纠正 S-D02-23 的竞态假设。canonical head/事件页来自同一事务，`get()` 却只读前 500 事件；
  M04-23 pause 在 seq 635，页外记录一直缺失，重复 GET 无法升级。新原生反例确认 pause 已 cleanup-proven，
  `/projection.extra.execution_pause` 仍为 null，首败保留。现 Store 按 exact Session/Turn、已观察
  head cursor 单独读取至多一条 immutable pause event；不扩大分页、不读全历史或取更乐观状态。
  同一 Turn 的后续 pause 也不会替换旧 cursor 对应证据。撤去前端双读推测，真实 unproven 仍立即展示。
- pause/resume/唯一写入原回归修复后首次 + 20 repeats **21/21**，Store exact/cursor/foreign/false
  子断言 **1/1**，相关 UI **18/18**、typecheck、desktop boundary、正式构建和签名通过。loopback
  Browser 夹具先保留 ID 重用与压缩后缺历史的失败，随后使用单调 call ID 与不依赖旧观察的 readonly
  调用；正式 Tauri 前后均 pause seq 605 / 41 model steps / 48 local requests / 4 returned effects，
  首败三个 projection 均缺字段，修复后三个均为精确 reason + cleanup-proven。正式 UI 从错误清理提示
  改为任务未完成/操作保留，暂停发送禁用，UI 结束回合后 cancelled/ready/可发送；DB/备份 `ok`，
  owned App/Helper/fixture/listener 0。证据 `2026-09-30/macos/m04-pause-event-page/`；付费调用 0。
  其他平台/角色、多窗口/断线、真实 Provider terminal、N3/99% 与完整 Case/阶段仍开放。

- S-D03-48（`ART-001/003/007`、`CTRL-007`、`CMD-132`、A01/A05/A17 文字观测与 Artifact
  边界）：macOS 正式 Tauri 的两个独立首发中，`ls -a` 均已成功且模型正确解释，但
  step spec 的“捕获完整输出…业务文件列表”和“在输出中…业务文件”均被旧词法合同
  误判为必须交付持久文件，导致 completed Turn 被翻成不可重试失败。现仅对
  `完整/原始/命令…输出` 与 `在/从/于输出中/里/内` 排除名词用法，并在确定 next-verb 前移除
  token，因此“保存命令输出为文件”仍能看到前一真实动词的 target。“输出一个文件”、
  显式格式和计数仍严格。Agent execution **112/112**；修复后正式 Execution/Attempt
  completed、`output_files=[]`、UI 1/1，旧失败 Session 未重试且保留。完整 Artifact/角色/平台
  矩阵、100 seed 与 LONG/99% 仍开放，不关闭共享 D03。

- S-D03-49（FILE-040、PORT-012、OBS-018、A05/A15/A17/A19 正式 Runtime watcher 接线与
  诊断子断言）：W71 的正式 300 文件样本虽靠模型主动重读得到正确终值，却没有独立 native dropped
  计数。W72 首轮增加有界日志后，两次真实 burst 均没有 watcher 启动/批次记录；`workspace.files`
  已在冻结 Preset 中启用且精确绑定 workspace，说明此前 unit contributor 从未进入正式 Runtime。
  根因是正式 Nomi 走 `EngineKernelSession`，只消费 initial capability/Plugin context；通用 Plugin Tool
  Session 的 lifecycle contributor 并不驱动该 Runtime。现由 `EngineKernelSession` 在 Full scope 且
  精确选择 `workspace.files` 时创建并跨 Turn 持有 `NomiWorkspaceWatchContext`，每轮模型前先消费；
  启动和非空批次只记录 event/dropped/rescan/reconciliation 四项计数，不记录路径或内容。最终正式
  Tauri `46063c30f9dd…` 中，300 文件产生 603 个通知，日志精确为 retained=256、dropped=347、
  rescan=false、requires_reconciliation=true；两回合 completed、43 条事件、零工具调用，磁盘 300
  文件摘要不变，应用/profile 清零并正式备份。Linux 精确测试 **1/1**；同步远端至
  `0fee0eda69d6…` 后正式 Tauri 重建 `7329a6f0dda9…` 亦通过，watcher 源码未变。
  首轮模型命令错误及违背只读要求写入脚本的失败另存并转后续 CMD 子断言；restricted Attempt、
  重复/乱序/rescan UI、其他 Agent/macOS、N3/100 seed/LONG/99% 仍开放，不关闭完整 FILE-040。

- S-D03-47（FILE-040、PORT-012、A05/A15/A17/A19 watcher overflow 全量对账子断言）：W71
  新增跨过 `WatchQueue`、`ContextContributor` 与 canonical JSON 的溢出回归：263 个唯一变更保留
  最后 256 个，首项为 `marker-0007`、`dropped_event_count=7`、`rescan_required=false`，上下文明示
  批次不完整并要求重读，且只消费一次；Windows/Linux 各 **1/1**。现有生产路径首次即通过，本批
  只补回归与正式证据。正式 Tauri `8c54fc75dd29…` 使用隔离 data/work/profile 和 StepFun Plan /
  `step-3.7-flash`，Session 建立后从外部并发创建 300 个文件；下一回合首个工具调用即为完整
  PowerShell 清单重读，随后所有进程/读取调用均成功，最终磁盘/UI/canonical 一致报告 300、
  `event-0001.txt`、`event-0300.txt`。两回合 completed、311 条事件、零工具错误，工作区摘要未变，
  应用/profile 清零并完成官方备份。真实 native 样本的精确 dropped 数未独立落盘；丢批/重复/
  乱序与 rescan 的更多 UI 组合、其他 Agent/平台、100 seed/LONG/99% 仍开放，不关闭完整 FILE-040。

- S-D03-46（FILE-020/025/038、A05/A07/A17/A19 Unix 既有目标派发源窗口子断言）：W70 在
  Linux ext4 上把同名源替换精确放到暂存 inode/字节句柄核对之后、实际 `rename` 之前。系统调用
  因此把外来 7 字节对象放入目标，原意 8 字节暂存被保留到独立名称；发布后核对正确返回
  `outcome_unknown`，但旧代码仍以 rename 成功等同“自有暂存已消费”，错误记录
  `temporary_cleanup_unconfirmed=false`。首版弱断言假通过与收紧后的首次 FAIL 都保留。现改为只有
  目标仍匹配暂存 inode 才确认消费；身份不符时保留 unknown、无 publication identity、要求重读，
  同时显式标记临时清理未确认且不再按已复用名称清理。新增 Linux 回归 **20/20**，Linux service
  **126/126**，Windows `nomifun-file` lib **362/362**，workspace fmt 通过。该窗口的零副作用补偿、
  失败清理 check→unlink、macOS 原生执行、完整 IO fault/UI/角色与长期门槛仍开放，不关闭完整
  FILE Case 或共享阶段。

- S-D03-45（FILE-019/028、A05/A13/A17/A19 Unix 新建发布清理窗口子断言）：W69 在 WSL2
  Ubuntu ext4 夹具中把并发替换精确放到暂存 inode 身份确认之后、`unlink` 之前；旧 hard-link
  发布先正确生成目标，随后删除了占用原暂存名的外来文件，并仍返回成功。首次 FAIL、ext4 现场和
  runner 夹具错误分别保留。Linux/macOS 新建发布现复用共享原生 no-replace rename：Linux
  `renameat2(RENAME_NOREPLACE)`、macOS `renamex_np(RENAME_EXCL)` 在同一原子操作中完成“不覆盖
  目标”和消费暂存名，不再经过按路径清理窗口；既有目标、权限与内容核对保持不变。新增 Linux
  反例修复后 **20/20**，Linux service **125/125**，Windows `nomifun-file` lib **362/362**，
  workspace fmt 通过。Windows 发布路径未改变；macOS 原生执行、其他 Unix fallback、失败路径的
  身份检查→unlink、既有目标 verify→rename、完整 UI/角色与长期门槛仍开放，不关闭完整 FILE Case。

- S-D03-43（FILE-020/022/024/031/032/036、A02/A05/A14/A19 macOS APFS identity 与 Unix
  目标权限子断言）：Unix 批量 patch 只按词法路径判重，APFS case 或 NFC/NFD 等价目标会先发布
  第 1 项再在第 2 项冲突；POSIX rename 又能在父目录可写时替换 mode/ACL 只读目标。现由 macOS
  卷 `_PC_CASE_SENSITIVE` + canonical Unicode component key 在准备期拒绝等价/祖先目标；合法
  Case-sensitive APFS case 变体保持区分。Unix 替换前持有同 dev/inode 的可写目标句柄，权限拒绝
  保持旧字节并清理暂存。macOS 主卷/Case-sensitive APFS 新回归各 **20/20**，`nomifun-file`
  **464/464**；Linux、更多 Unicode case-fold、ACL 竞态、IO fault、正式 UI/角色及长期门槛仍开放，
  不关闭完整 FILE Case 或共享阶段。

- S-D03-44（FILE-013/015、A15/A17/A19 广搜后的 instruction lookup 资源子断言）：W65 正式
  Tauri 基线在独立 workspace 外部写入 400 个同标记文件；模型以 `search_files(limit=200)` 对账时，
  Runtime 会对每个唯一命中再做精确 instruction-scope lookup。首个搜索因此产生 **100** 次内部读取，
  整个回合累计 **415** 次内部读取、**2,313** 条 canonical 事件；最终虽正确报告 400、首尾文件且
  磁盘未变，但过程显示工具错误并用了 11 个模型步，首次 FAIL/RECOVERED 保留，不计正向 Case 通过。
  搜索结果进入模型前现独立限制最多 64 个唯一 instruction hit path；公开搜索上限和 owner 扫描不变，
  超界搜索已执行但 snippets 明确 withheld，且在逐命中读取前停止，不增加 Action、授权或 ambient IO。
  64 项接受/65 项拒绝回归在 Windows/Linux 各 **20/20**，W65 源码同步远端前两平台 Runtime
  各 **171/171**；同步远端 `b046314cd` 后 Windows 仍 **171/171**，Linux 在用例启动前被该远端
  Unix 文件新增但未导入 `Path` 的编译错误阻断，首次失败保留并单独转 W66，不混入本批修复。
  修复版正式样本使用相同 400 文件内容摘要，首个搜索内部读取从 100 降至 5（只含固定根/AGENTS
  预检），总内部读取 **68**、canonical 事件 **664**，最终磁盘/UI/canonical 仍一致。该单样本仍有
  可见命令/完成报告错误并用了 16 个模型步，只证明资源 fan-out 已有硬界；完整搜索 UX、真实 native
  dropped 数独立观测、其他 Agent/macOS、100 seed/LONG/99% 仍开放，不关闭完整 FILE Case 或共享阶段。

- S-D03-42（FILE-040、PORT-012、A08/A14/A15/A17 workspace rescan/dropped 上下文组合子断言）：
  既有 watcher 队列会分别记录已知丢弃数与原生 rescan 标记，但缺少跨过 pre-turn contributor 和
  system prompt 合并边界的证明。W64 新增组合回归，同时注入一个合法相对事件、一个被拒绝路径和
  native rescan；合并后的 canonical JSON 保留 `dropped_event_count=1`、`rescan_required=true` 与
  合法事件，并明确要求重读相关状态、不得据此推断其他路径未变化。绝对 workspace root 不进入提示，
  contributor 仍一次消费。现有生产路径直接通过，本批只补回归，未扩大 Action、工具或权限。
  Linux/Windows 各有 watcher 上下文 **12/12**，新增组合用例各 **20/20**；两平台 Cargo 单项也
  各 **1/1**。WSL 首次通用命令在已通过 lib 用例后误启动零匹配 integration binary，因共享构建目录
  混入异平台产物触发 loader 失败；首次 `--exact` 又因过滤名不完整执行 0 项；Windows 首次切回编译
  在 `nomifun-db` 发生 rustc 栈溢出。三项 runner/编译失败均保留，修正命令或提高编译线程栈后通过。
  正式 UI 全量对账、模型是否遵循重读提醒、完整丢批/重复/乱序、macOS、100 seed/LONG/99% 仍开放，
  不关闭完整 FILE-040、PORT-012 或共享阶段。

- S-D04-01（PROC-011/012/014、A11/A13 Unix 确定 pre-spawn 子断言）：显式 Unix executable
  不存在或无 X_OK 时，旧路径先创建 watchdog，再在 exec/wrapper 失败后共用 setup deadline 回收，
  压力下可从确定“未启动”退化为 `start_lost`；macOS 产品 owner 还未启用已有 Seatbelt 策略。
  现将显式路径诊断及 sandbox 环境准备移到任何物理 authority 之前，bare PATH 名继续走真实
  exec 并保留 ABORT/watchdog 回收反例；Engine 只把确定的 pre-spawn 类型标为
  `user_code_not_started`。macOS 产品接线与原生结果见 M01-01；Linux/Windows 行为未据此代判。
  macOS `nomi-process-runtime` 全组 **243/243**，显式路径及进程/PTY 压力回归通过；正式 UI、
  其他平台、完整 PROC/CMD/LONG 门槛仍开放，不关闭共享 D04。

- S-D04-02（PROC-011/012/014、A19 Unix 构建可移植性子断言）：同步远端 S-D04-01 后，新增的
  `validate_explicit_unix_program` 在所有 Unix 编译，却仍把 `Path` 导入限定为 macOS，Linux 因
  `cannot find type Path` 在任何 W65 用例前失败。W66 将 `Path` 改为 Unix 无条件导入，仅保留
  `PathBuf` 为 macOS 专用；没有改变 spawn、sandbox、权限或清理逻辑。Linux process Runtime 完整
  **245/245**，Agent Runtime **171/171**；Windows process Runtime lib **120/120**。首次 Linux
  完整组另有一次 PTY parent-death reap 时序失败，原样保留；定向复跑 **1/1**、随后 **20/20**，
  最终完整组通过，未加 sleep 或放宽断言。macOS 结果仍以原 M01-01 为准；正式 UI、更多 Unix
  发行版/架构与 LONG 门槛仍开放，不关闭完整 PROC Case 或共享阶段。

- S-D04-03（CMD-135/150、A08/A17/A19 host shell Schema 子断言）：W65 的 Windows 正式样本
  在 broad search 被有界 withheld 后，首个进程调用提交 `cmd: "dir /b /s burst"`；`cmd` 实际由
  PowerShell 执行，因而产生可见失败。顶层工具说明虽带 host OS，但共享基础句先给出 Unix
  `cmd=ls -la` 示例，字段级 Schema 又只写泛化 host shell。W67 首次回归确认 `cmd` 字段没有
  PowerShell 事实。现移除跨平台基础说明中的 Unix 示例，并在字段层明确 Windows 为 PowerShell、
  禁止直接使用 `dir /b`/`dir /s`，Command Prompt 必须显式 `command=cmd.exe` + 分离 args；Unix
  字段明确 `/bin/sh -c`。合法输入集合、Action、权限和 owner 执行不变。Windows/Linux Runtime
  各 **172/172**，新增平台 Schema 回归各 **20/20**。正式 Tauri `57515a20e31c…` 复验同类 400
  文件场景：首个及后续三个进程调用均使用 PowerShell 且零进程错误，最终磁盘/UI/canonical 一致；
  对账回合为 5 个模型步、264 条事件。broad-search withheld 仍是可见错误，因此仅关闭 host shell
  映射子断言；macOS/Linux 真实模型、完整 CMD-135/150、N3/100 seed/LONG/99% 仍开放。

- S-D04-04（`CMD-132/146/148`、`PROC-006/014`、A01/A11/A13 model-facing 字面 argv
  子断言）：process host 文案已要求 macOS 使用 `command`/`args`，但本批基线 JSON Schema
  反而把 `cmd` 标成“Preferred”，首个真实 StepFun 轨迹因而发出 `cmd:"ls -a"`。
  同步 S-D04-03 后进一步将普通单 executable 明确设为 `command` + 字面 `args`
  首选，`cmd` 只在 pipeline、redirection、globbing、compound syntax 或 shell script
  需要时使用；Windows PowerShell/Command Prompt 禁用语义与 Unix `/bin/sh -c` 字段级事实
  全部保留。Agent runtime **173/173**。修复后两个新真实轨迹均首次形成
  `{"command":"ls","args":["-a"]}`；最终样本恰好 1 个 process effect、退出 0、
  `reaped=true`、完整隐藏项且 `visible_failure_count=0`。异构 client/host 和故意错配
  `HOST_OS_COMMAND_MAPPING_ERROR`（`CMD-149/150`）未在本批执行，不关闭完整 CMD/PROC 或共享 D04。

- S-D04-05（CMD-135/150、PROC-014、A08/A13/A17/A19 Windows PowerShell 版本与调用形状
  子断言）：W72 首次正式计数探针先把 `powershell.exe -Command ...` 整段再次放进 PowerShell
  `cmd`，变量被外层 shell 展开而失败；后续改用字面 argv 后又提交 PowerShell 7 的 `??`，而
  Windows owner 固定调用 System32 WindowsPowerShell v1.0（5.1）。同步 S-D04-04 后，首次 Schema
  回归仍因没有 `Windows PowerShell 5.1` 事实而失败并保留。现明确 `cmd` 字符串已由 5.1 直接执行、
  不得再加 `powershell.exe`/`pwsh` 前缀；显式 executable 必须使用 `command=powershell.exe` 与分离
  `args`，且不得假定 `??`、`??=`、三元 `? :`、`&&`、`||` 等 PowerShell 7 语法。合法参数、权限和
  owner 执行均未改变。精确回归 **20/20**、Agent Runtime **173/173**；正式 Tauri
  `90a0d26b79b6…` 在隔离 workspace/data/profile 中首次形成字面 `powershell.exe` argv，唯一进程
  退出 0 且 `reaped=true`，正确返回 300、`ps51-0001.txt`、`ps51-0300.txt`。单回合 completed、
  2 个模型步、67 条事件、仅一项 process effect；300 文件摘要不变，应用/profile 清零并正式备份。
  `cmd` 直接脚本、`start_process`、N3/100 seed/LONG/99% 及 W72 的只读写入/完成参数失败仍开放。

- S-D04-06（CMD-137、A08/A11/A17/A19 accepted-input 只读工具面子断言）：W72 的正式只读
  计数任务仍向模型暴露文件写入工具，模型随后创建 `check_marker.ps1`；W74 新回归首次确认明确
  “不要修改任何文件”后 `write_file` 仍可见并失败，均原样保留。Runtime 现仅从明确的中英文
  accepted input 派生可撤销只读策略：每个模型步重新物化冻结工具面后，隐藏 workspace file、VCS、
  Artifact 的非 ReadOnly binding，保留读取与 process；同时要求 process 保持内联只读，禁止重定向、
  变更命令和临时脚本。后续 accepted input 明确允许修改时恢复原冻结工具，不新增 Action 或权限。
  保守解析与工具面恢复回归通过，核心收窄回归 **20/20**，Agent Runtime **175/175**。正式 Tauri
  `10c052e32ac6…` 中零 workspace mutation tool call、仅一个 returned process effect，300 文件及
  tree hash 不变、无额外路径，应用/profile 清零并正式备份；但首次 `exec_command.args` 被模型编码
  为 JSON 字符串，UI 保留 1 项可见异常，第二步修成数组后才完成（3 个模型步、87 条事件），因此
  不计零失败 CMD PASS。收紧撤销短语后的正式重建 `baec5c5f06e2…` 通过。opaque process 的内核级
  只读隔离、正式撤销 UI、参数字符串失败、其他 Agent/平台及 N3/100 seed/LONG/99% 仍开放。

- S-D05-07（`VCS-006/008` message-hook production build 子断言）：M04 首次 current-source desktop
  example build 暴露 M03-10 的 `tempfile` 仅声明在 `nomifun-app` dev-dependencies，形成“测试通过、产品
  dependency 编译失败”。现把同一 workspace-pinned dependency 移入 normal dependencies，无版本/lock
  变化；正式 desktop CEF example build 通过，相邻 VCS **22/22**。CEF native 行为另批验收，本项只
  关闭 production compile 缺口，不关闭完整 VCS/BROW 或共享 D05/D06。

- S-D05-06（`SSH-001～012` 的本批 transport/owner 子断言）：macOS 原生用完全隔离的 loopback sshd
  建立随机端口、临时 host/client keys、known_hosts 与加密 host book，不读取生产 SSH 配置。真实连接池
  18/18、shared transport 83/83 首次均零 SKIP；backend 首轮 39/41 时确认 BSD grep 对单文件仍输出
  filename prefix，导致 literal `-` 和 quoted `a'b` 的结果偏离 `line:content`。fallback 现仅在目录使用
  `grep -rnE` 保留路径，单文件使用 `grep -nEh`；rg/no-match/error/quote/timeout 单引擎合同不变。
  三条命令形状各 **20/20**，真实 sshd grep **20/20**，最终 backend **96/96**（pool **19/19**）、
  shared transport **83/83**，零 SKIP、fixture/process/listener 清零。非 loopback host、正式 UI、应用重启
  remote in-flight、write 后断线 digest 对账及 LONG/flood 仍开放，不关闭完整 SSH 或共享 D05/D11。

- S-D05-05（`VCS-006/008/009/014`、`AUTH-013` 的 `post-commit` 通知子断言）：macOS 首败确认
  libgit2 commit 成功但真实 `post-commit` 完全未运行。现仅在 commit ID 创建后以字面
  `/usr/bin/git hook run --ignore-missing post-commit` 执行，沿用 hooksPath、supervisor、30 秒 deadline、
  完整进程树回收与精确 repo Seatbelt。非零通知退出不反转已发生 commit；成功 receipt 携带有界脱敏
  的 status/exit、`retry_allowed=false` 和独立 HEAD observation。hook 成功但改写 HEAD 时明确
  `head_matches_commit=false`、`requires_reconciliation=true`；同 key 只重放 receipt，不重复 commit/hook。
  非零失败与 HEAD 改写各 **20/20**，相邻 VCS **22/22**。commit/post hook 至 receipt 的崩溃窗口、
  hook 文件置换/fault injection、linked worktree、非 macOS 与正式 UI 仍开放，不关闭完整 VCS/AUTH
  或共享 D05。

- S-D05-04（`VCS-006/008/014`、`AUTH-009/013` 的 commit message hook 子断言）：macOS 首败确认
  仅配置的可执行 `commit-msg` 被 libgit2 commit 完全绕过。现按 `pre-commit → prepare-commit-msg →
  commit-msg` 顺序支持 default/`core.hooksPath`，以字面 `/usr/bin/git hook run` argv 进入共享 supervisor、
  30 秒 deadline、完整进程树回收与精确 repo Seatbelt。消息文件位于绑定 Git metadata 内，保留打开的
  descriptor/dev/inode，拒绝置换/symlink 读取，并限制 64 KiB、UTF-8、512 字符；hook 后重新核对 HEAD
  parent 与 staged path membership，漂移或后续失败结算 external-unknown，资源 fence 不释放。错误原因
  有界脱敏但保留类别。拒绝/脱敏、相对 hooksPath 顺序/改写、identity 拒绝、index 漂移四条各
  **20/20**，相邻 VCS **20/20**。`post-commit`、hook 文件置换竞态、fault injection、linked worktree、
  非 macOS 与正式 UI 仍开放，不关闭完整 VCS/AUTH 或共享 D05。

- S-D05-03（`VCS-009/014` 的持久 commit receipt、HEAD/tree 与用户本地编辑隔离子断言）：macOS
  真实成功 `pre-commit` 后创建 commit，消费端故意丢弃返回；确认 effect 已持久为 `returned` 后销毁
  host，追加用户 worktree 修改并以原 Session Store 重建。同 key 调用只返回原 receipt，不再执行
  hook 或 libgit2 commit；HEAD OID、tree OID、commit count、committed blob 和 terminal event identity
  均保持，后写本地修改未被覆盖。首次即通过，精确回归 **20/20**，相邻 VCS **16/16**，无需产品
  代码修复。physical commit 与 terminal receipt 之间的崩溃/存储失败窗口仍只有 pending fence，尚缺
  commit-intent 对账；push result-loss、正式 UI、其他平台及长期门槛仍开放，不关闭完整 VCS-009/014
  或共享 D05。

- S-D05-02（`VCS-006/008`，以及 `VCS-009/014`、`AUTH-009/013` 的 hook/unknown/scope/脱敏
  子断言）：macOS 首败确认 libgit2 commit 绕过可执行 `pre-commit`，hook `exit 7` 后仍创建提交。
  现仅在 macOS 检出 default/`core.hooksPath` 的可执行 hook，执行前核对 exact repo、staged scope 与
  identity，再以字面 `/usr/bin/git hook run --ignore-missing pre-commit` argv 交给共享进程 supervisor；
  30 秒 deadline、完整进程树回收和精确 repo Seatbelt 写根同时生效，返回后再次核对 index/scope。
  repo 根不等于绑定 workspace 时拒绝执行 hook；输出有界且 secret 脱敏。
- hook 拒绝/启动失败/超时可能已修改 workspace，故 commit effect 使用既有
  `ExternalUncertainEffect`：写入 terminal unknown 后保留唯一资源 fence，同 key 与新 key 均不能
  自动物理重放。拒绝/脱敏/fence、Seatbelt 越界拒绝、identity 拒绝各 **20/20**，相邻 VCS
  **15/15**、direct-program/shell **3/3**。首次误用信任 TMPDIR 的隔离夹具及 ManagedEffect 不允许
  uncertain 结算的中间失败均保留。其他三类 commit hook、commit 成功后 result-loss 对账、hook
  fault injection、非 macOS 平台和正式 UI 仍开放，不关闭完整 VCS/AUTH 或共享 D05。

- S-D05-01（VCS-004/005、A05/A14/A17 snapshot 字面路径子断言）：W62 Linux 全组在
  `file\\1.txt` 的 discard 稳定失败；libgit2 checkout 即使禁用 pathspec 匹配，仍把反斜杠按路径
  分隔处理并返回成功，字面文件保持 staged。W63 诊断补出精确文件名；首版 one-entry 内存 index
  checkout 又在 Windows 删除邻居 index/worktree，两次中间 FAIL 均保留并撤回该方案。最终仅对
  Unix 含反斜杠的已验证 Git index 路径直接读取 HEAD blob，以同目录唯一临时名原子发布；普通
  路径继续使用 libgit2。tracked executable mode 与 symlink target 同时保留，邻居 index/worktree
  不变。Linux 两项关键回归各 **20/20**、snapshot **51/51**，`nomifun-file` 全组 **460/460**；
  Windows snapshot **50/50**。无 renderer/模型/UI 改动；macOS、非 UTF-8 Git path、更多 filemode/
  并发父目录竞态及完整 VCS/角色/长期门槛仍开放，不关闭完整 VCS Case。

- S-D03-41（FILE-040、A05/A17 Linux 删除事件归约子断言）：W61 Linux 全组唯一剩余失败
  是单文件删除通知稳定超时。W62 原生轨迹确认 inotify/notify 先发 `Modify(Metadata)`，紧接同路径
  `Remove(File)`；旧 debounce 只按 path，前者把真正删除抑制 200 ms，而 UI sink 有意忽略 metadata
  change。现以 `event_type + path` 作为去重身份，同类抖动仍合并，不同事实不互相吞掉。Linux
  纯归约与真实 ext4 删除两项各 **20/20**，Linux lib **304/304**、file_watching **14/14**；Windows
  file_watching **14/14** 及真实删除回归通过。Linux 全 crate 随后在无关 snapshot literal-path
  用例稳定失败并转 W63，不计 watcher 失败。无 renderer/模型/UI 改动；macOS、事件洪泛/乱序及
  完整 UI/长期门槛仍开放，不关闭完整 FILE-040。

- S-D03-40（ART-001/003/007、A05/A13 Unix Artifact 目录 durability 子断言）：W60 的 Linux
  全组中 11 项 Artifact 均在目录同步返回 EBADF；`cap_std::Dir` 可持有只用于 capability traversal
  的 `O_PATH` descriptor，旧实现 clone 后直接 `fsync`。W61 在同一已授权 Dir 内以 `"."` 重新打开
  可读目录句柄并同步，不做 ambient 路径解析，也不忽略其他 sync 错误。新增 Linux 目录同步回归
  **20/20**，Linux Artifact **18/18**，Windows Artifact **15/15**。Linux 全 crate 从 12 项失败
  收敛为 1 项已有 watcher 时序，最终 302/303；该项定向复核仍失败，转 W62，不计 Artifact 失败。
  macOS 主 lane、真实磁盘故障/断电 durability、完整 Agent/UI/角色及长期门槛仍开放，不关闭完整
  Artifact Case 或共享阶段。

- S-D03-39（FILE-019/020/025/028/038/039、A05/A07/A13 Unix 暂存源/清理子断言）：W60
  在 WSL2 Ubuntu 6.18 的 `/home` ext4 临时目录执行原生 Linux 夹具；源码从 `/mnt/c` 挂载，
  文件语义断言仅作用于 ext4 夹具，未把 drvfs 当 Unix 文件系统。最终核对后把暂存原地改写或
  换成同字节异 inode，旧 Unix 路径均返回成功并发布错误对象；两项首次 FAIL 及 ext4 现场复制到
  Windows 外部证据目录。现在所有 Unix
  暂存都保留 inode 身份；rename/hard-link 前以 `O_NOFOLLOW` 打开并核对 inode/调用字节，权限按
  已核对句柄继承，发布后再次核对目标身份/字节。失败清理仅删除仍属于该 operation 的 inode；
  外来同名文件保留并将 cleanup 记为未确认。新建分支同样覆盖，三项 Linux 回归各 **20/20**，
  Unix `service::tests` **124/124**；Windows `nomifun-file` **555/555**。
  Linux 全 crate 另保留 12 项非本批失败：11 项 WSL artifact 目录 fsync 返回 EBADF、1 项已有
  native watcher 时序，不能据此宣称 Linux 全组通过。身份核对到 rename、身份检查到 unlink 的
  最窄路径窗口、macOS 主 lane、完整 IO fault/UI/角色及长期门槛仍开放，不关闭完整 FILE Case。

- S-D03-38（FILE-040、AUTH-005、A09/A13/A17 watcher 尾事件子断言）：W59 把旧 native
  注册已排队的单文件事件延迟到同路径重订后分发，旧全局 callback 会按当前 path→owner 映射误投
  新 owner；首版夹具用非 canonical event path 未击中映射，假通过另存，纠正后首次产品 FAIL
  保留。单文件 watcher 现按 canonical path 分注册实例，各自捕获独立 owner fence 与 debounce；
  Office 注册也改用同一 fence。回调取得 delivery token 后才复制 owner，停止先移除 owner 并等待
  已开始发送结算，再撤销原生注册；等待期间不持注册表锁，event sink 可重入。旧注册 callback
  只看到已退休 owner 集，新注册不继承尾事件或 debounce；共享同路径的其他 owner 保留。
  `nomifun-file` **555/555**，单文件迟到、Office 迟到及发送中停止三项各 **20/20**，真实 native
  创建/删除/重订与 14 项集成回归通过。无 renderer/模型/UI 改动。活动注册容量、native callback
  永不返回、完整 rescan/dropped/乱序 UI、其他平台及长期门槛仍开放，不关闭完整 FILE-040。

- S-D03-37（FILE-040、A05/A17/A19 未监听外部变更子断言）：W58 首次反例证明完成的扁平
  清单缓存会永久遮蔽未启动 watcher 时的外部创建；显式第二次 `/api/fs/list` 仍只返回旧文件。
  现将完成清单限定为通知/活动读取的一致性快照：新的显式 API/Agent 读取在无人使用时退休旧
  快照并重新扫描；活动读者、扫描中失效、旧扫描不得覆盖新扫描及不同根隔离保持。外部创建和
  删除的直接 owner 与真实 Axum route 回归通过，三项关键回归各 **20/20**，`nomifun-file`
  **552/552**。一次无关的原生 watcher 三秒超时保留，未加 sleep；定向复跑及最终完整组通过。
  正式 Tauri `@文件` 菜单先读到 `old.txt`，随后由外部 PowerShell 创建 `new.txt`、删除旧文件；
  关闭重开后 `@new` 只显示新候选、`@old` 显示空结果。四次 `/api/fs/list` 均 200，watch route
  调用为 0；冻结二进制 `fcc93ec68118…`，1 模型步、0 effects，磁盘 hash/UI/canonical 一致，
  应用/profile 清零并正式备份。单次无 watcher 扫描期间的外部竞态、完整丢批/乱序、其他角色/
  平台及长期门槛仍开放，不关闭完整 FILE-040 或共享阶段。

- S-D03-36（FILE-020/025/038/039、A05/A07/A17 原生替换源间隙子断言）：W57 在最终
  暂存核对与 `ReplaceFileW` 之间分别换入同字节异身份对象、原地改写暂存对象；两项首次
  FAIL 均返回 unknown，但目标已被外来/错误字节占据。Windows 现紧邻原生调用记录实际派发源的
  卷/128 位文件 ID；返回后的目标仍是该对象却不匹配预期 owner/字节时，先按句柄无覆盖地退回
  原暂存名，再以原件身份句柄恢复备份。补偿仍保留 outcome_unknown，不把短暂发布改称零效果，
  也不允许盲重放；同字节外来源和被改写的自有暂存均恢复原目标，外来对象不删除。若调用后目标
  又被并发对象接管，身份不符则不补偿，保留并发目标、原件备份和待核对状态。另补 capture 后、
  调用前再次换源的最窄窗口，确认不会错误补偿，仍保持 unknown。`nomifun-file` **551/551**，
  5 项相关回归各 **20/20**；无 renderer/模型/UI 改动。身份 capture 与系统调用之间的最窄窗口、
  Unix/macOS 清理与发布、100 seed、完整角色/UI 和长期门槛仍开放，不关闭完整 FILE Case。

- S-D02-13（CTRL-007、FILE-031/039、A17/A18 失败即停与部分结果子断言）：W51 用 W45
  冻结构建、相同 StepFun Plan / step-3.7-flash、提示和三套隔离目录重验 W42 场景，首轮仍仅
  **1/3**；两次失败分别在参数错误或真实部分发布错误后继续读/补丁，43 模型步、6 effects，
  原失败均保留。Runtime 现只在 accepted user input 同时明确“错误即停”和“禁止重试”时启用
  failure-stop；首个工具失败后仅暴露 `report_completion`，要求 blocked 部分结果，不能重读、
  查历史、重规划或再做 effect。`apply_patch` Schema/说明补一行替换的精确 remove+add 形状，
  避免把 context+add 当替换。W53 首次正式复验又保留 blocked 报告误填 `evidence_paths`、摘要
  未点名两个目标的 FAIL；门禁随后注入待恢复目标和最小报告形状，blocked 条目不得伪造证据。
  最终 Runtime **169/169**，相关 4 项回归各 **20/20**，正式 Tauri 构建通过。W56 冻结二进制
  `e2fc6f5e17ac…`、GEN Revision、route/preset 和原提示，三个新 Session 均为 4 模型步、一次
  guarded patch、一次 rejected effect；错误后零文件/进程动作，部分回执明确 `first.txt` 已发布、
  `second.txt` 失败，canonical/disk/UI 独立断言全部通过。总计 12 模型步、3 effects，应用、helper、
  profile 清零并正式备份，**修复后固定样本 N3=3/3**。W42/W51/W53 的旧失败不改写；其他角色、
  普通完成分支的完整矩阵、Unix/macOS、100 seed、LONG/99% 及共享阶段仍未关闭。

- S-D02-14（FILE-015、A08/A15/A17/A19 有界搜索 UI 分类子断言）：W67 正式 Tauri 样本中，
  `search_files` 按既有资源上限返回精确 `search_context_withheld` 后，Runtime 正确要求模型完整
  核对，但 UI 把该预期有界结果汇总为“1 项操作出现异常”；首次可见失败和 canonical 结果均保留。
  W68 只对本地 `search_files`、`status=error`、四个精确字段且布尔值成立的结果增加
  `boundedResult` 分类；canonical `is_error=true`、模型重规划和可展开详情不变。格式错误、多字段、
  远端或其他工具错误继续按失败处理。Turn 回执不再把该结果计入普通错误数，改用琥珀色“搜索结果
  受限，需要完整核对”。三组定向 UI 回归 **72/72**，typecheck、i18n parity 与桌面 UI 边界通过。
  正式 Tauri `a1d254ff1c18…` 使用隔离 data/work/profile 与 StepFun Plan / `step-3.7-flash` 重验
  400 文件 burst：canonical 仍是有界错误，随后两个 PowerShell 只读调用成功，最终磁盘/UI/
  canonical 一致报告 400、`marker-0001.txt`、`marker-0400.txt`；工作区摘要未变，两回合 completed、
  214 条事件，应用/profile 清零并完成官方备份。其他有界结果、其他 Agent/平台、完整 FILE-015、
  100 seed/LONG/99% 仍开放，不关闭共享阶段。

- S-D02-15（CTRL-006/007、A05/A07/A17/A18/A19 多命令完成证据子断言）：W75 两条独立
  process 均成功，但 completion 的 alpha criterion 复用最新 beta call ID；该 ID 不能证明 alpha。
  W76 首版只要求匹配 ID，又与既有保守合同冲突：后续 process 启动会使前一 command observation
  退出 `available_evidence`，模型在不可满足约束中反复推演，6 个模型步后续写预算耗尽并 failed。
  两次首次失败均保留。最终说明要求仅引用当前 `available_evidence` 中的匹配 ID；若先前调用已不可用，
  必须用无证据的 `unverified`，不得借用最新 ID、加载历史或未经授权重跑。证据有效期、validator 与
  权限均未放宽。精确回归 **20/20**、Agent Runtime **175/175**。正式 Tauri `c90c28c56c46…`
  中两进程成功/reaped，completion dispositions 为 `[unverified,supported,unverified]`，证据为
  `[[],[zeta_call],[]]`；最终正确报告 epsilon=3、zeta=8、合计 11 并披露 epsilon/合计未验证，
  单回合 completed、5 个模型步、140 条事件、UI 零工具异常，11 文件摘要不变，应用/profile 清零
  并正式备份。多条顺序 process 结果要全部 supported 仍需可证明只读的 owner 合同或原子聚合；
  其他场景/Provider/角色/平台、N3/100 seed/LONG/99% 仍开放，不关闭完整 CTRL 或共享阶段。

- S-D04-07（CMD-139、PROC-033、A13/A17/A19 托管进程显式取消与来源子断言）：W77 首个正式
  `start_process → poll_process → cancel_process` 样本已得到 `state=cancelled`、
  `cleanup.reaped=true`，宿主却把所有非 `Exited` 终态统一写成 `success=false`，因此成功清理在
  canonical/UI 中显示为工具错误。首版宿主修复后，Runtime 又把同一结果计为
  `failed_commands=1` 且 completion observation 不可用，完成 Schema 收窄为
  `evidence_call_ids.maxItems=0`；继续修复后，零错误样本还暴露终态 command provenance 未携带
  中间 poll call ID。三层首次失败、截图、事件、数据库与模型轨迹均保留在仓库外。
  现仅对显式 `cancel` 且 `reaped=true` 返回成功；未回收或 `exec` 被中断仍失败，并保留
  `cleanup.errors`。Runtime 不再把成功的显式取消计成失败命令，也不把它冒充正常退出/测试成功；
  cancel observation 可作为取消/清理证据，但 `was_current_at_observation=false`。同一进程的 poll
  call ID 进入有界 interaction provenance，且不会推进 workspace effect epoch；完成说明另明确嵌套
  launch/interaction ID 只有同时出现在顶层 `available_evidence` 时才可直接引用。
  宿主精确回归 **20/20**，两项 Runtime 回归每项 **20/20**，Agent Runtime **177/177**，workspace
  fmt 与正式构建通过。最终 Tauri `64d2f85813a8…` 使用隔离 data/work/profile 与 StepFun Plan /
  `step-3.7-flash`：start、两次 poll、cancel、report_completion 五次调用全部成功；cancel 为
  `success=true`、`reaped=true`，Windows 无 console interrupt 合同的诊断仍保留。单回合 completed、
  5 个模型步、166 条事件，`failed_commands=0`、`failed_tools=0`；可用 cancel observation 精确携带
  launch ID 与两条 poll ID，完成报告一次通过。工作区摘要不变、helper 为 0、应用/profile 清零并
  正式备份。cancel/自然退出真实竞态、忽略温和中断后的升级与后代清理、PTY、其他平台/角色、
  N3/100 seed/LONG/99% 仍开放，不关闭完整 CMD/PROC 或共享阶段。

- S-D04-08（PROC-038、A13/A17/A19 cancel/自然退出线性化子断言）：W78 在暂停时间的 owner
  夹具中固定两种竞争顺序。cancel 先取得 stop ownership、进程随后自然 reap 时，唯一终态保持
  `Cancelled`；自然 reap fact 先发布、仍处于最终输出 drain 时再 cancel，唯一终态保持 `Exited`。
  两种顺序的重复 cancel 与终态 poll 都逐字段等于首次结果，`cleanup.reaped=true`，底层 wait/reap
  恰好一次；自然退出先胜时零 signal。生产实现首次即满足，未改清理、信号或权限逻辑，只新增
  两项确定性回归。首次 **2/2**、两项各重复 **20/20**，process Runtime lib **122/122**。
  真实 OS 调度竞争、并发 cancel waiter、child/grandchild、PTY、其他平台和长期压力仍开放，不关闭
  完整 PROC-038 或共享阶段。

- S-D04-09（PROC-049、A13/A17/A19 close/cancel/poll 三方并发子断言）：W79 扩展可控 owner，
  让已通过 admission 的 `close_stdin` 停在异步 owner 调用内，同时启动 60 秒 poll 与 cancel。
  阻塞 close 不持有 registry/session 锁；cancel 仍以一次 interrupt 取得 `Cancelled`、
  `cleanup.reaped=true`，poll 在期限前被唤醒并逐字段得到同一终态，随后释放的 close 独立完成。
  底层 close/interrupt/wait 各一次。生产实现首次即满足，本批只补回归与测试夹具。首次 **1/1**、
  重复 **20/20**，process Runtime lib **123/123**。真实 pipe/PTY stdin 关闭、多个 cancel/poll waiter、
  平台 I/O 失败与应用退出仍开放，不关闭完整 PROC-049 或共享阶段。

- S-D04-10（PROC-038、A13/A17/A19 并发 cancel follower 子断言）：W80 用 barrier 同时释放 8 个
  cancel waiter，只有一个调用成为清理 leader，其余 follower 等待同一 terminal。八个结果逐字段
  相等且均为已回收 `Cancelled`，只发送一次 interrupt、只执行一次 wait/reap。生产实现首次满足，
  本批仅新增回归。首次 **1/1**，8 waiter 场景重复 **20/20**，process Runtime lib **124/124**。
  真实 OS helper、多 Session 混合、cancel future drop 与平台信号失败组合、长期压力仍开放，不关闭
  完整 PROC-038 或共享阶段。

- S-D03-50（FILE-019/020/028/038、A05/A07/A13/A17/A19 Unix 失败清理子断言）：W81 在 WSL2
  ext4 上于失败清理确认 `stage.tmp` inode 后、`remove_file(path)` 前，把该名称换成同字节外来文件。
  旧实现删除外来对象，保留原暂存 inode 到独立名称，却返回
  `temporary_cleanup_unconfirmed=false`；首次失败日志、observation、保留 inode 与缺失外来名均在
  仓库外保留。Unix 没有可信 compare-and-unlink，因此未消费的失败暂存现不再按名称删除，而是保留
  供显式对账并返回 `temporary_cleanup_unconfirmed=true`；上层据此进入 outcome unknown。Windows
  继续使用 handle 绑定删除。修复后精确反例 **20/20**，Linux service **127/127**，Windows
  `nomifun-file` lib **362/362**。保留暂存的后续 owner 对账/清理、macOS 原生执行、其他 Unix
  portable fallback、磁盘/IO fault、完整 UI/角色和长期统计仍开放，不关闭完整 FILE Case 或共享阶段。

- S-D03-51（`FILE-020/022/025/038/039`、`AUTH-006/014`、`MAC-005` 既有目标扩展 ACL 子断言）：
  macOS APFS 原生探针与产品首败确认普通 `rename` 替换 inode，会让成功的 write/patch 丢失目标
  extended ACL；旧实现只复制 POSIX mode，因此 `group:everyone deny execute` 在返回成功后消失。
  现于目标 dev/inode 与 stage identity/bytes 均核对后、最终 pre-publication hook 之后，使用目标/stage
  descriptor 调用 `fcopyfile(COPYFILE_ACL)` 并 `sync_all`，复制失败在 rename 前按权限/内部错误停止，
  不忽略 ACL 错误；Linux/Windows 路径不变。hook 中后加的 `group:staff deny execute` 与原 ACL 一并
  保留。deny-write ACL 保持旧字节/ACL，按 S-D03-50 精确保留两个自有 stage、返回 unknown 并发两个
  无内容对账事件；macOS 旧“必须立即删 stage”断言据此收紧而非放宽。
- 修复后 ACL 成功保留、deny-write、最终 hook 动态 ACL 各 **20/20**；`nomifun-file` lib **308/308**，
  macOS workspace **6 passed / 1 ignored**（既有 opt-in case-sensitive APFS），fmt/diff 通过。APFS 在
  ACL copy 后至 rename 的最后系统调用窗口、xattr/resource fork/flags、非 APFS 卷、IO fault、正式
  UI/角色和长期门槛仍开放，不关闭完整 FILE/AUTH Case 或共享 D03。

- S-D03-52（`FILE-020/025/038/039`、D11 既有目标 xattr/resource-fork 子断言）：M03-06 首轮夹具
  因 `/usr/bin/xattr -p` 展示换行失败，纠正 CLI oracle 后命中真实产品反例：成功 write/patch 返回
  success，却删除 `com.nomifun.reliability.fixture`。S-D03-51 的 descriptor-bound macOS 元数据复制现
  扩展为 `COPYFILE_ACL | COPYFILE_XATTR`，仍位于最终 hook 后、rename 前并执行 `sync_all`；任何复制/
  持久化失败在目标名称改变前停止，不记录真实 xattr 值，Linux/Windows 路径不变。原生断言同时覆盖
  普通 xattr、`com.apple.ResourceFork`，以及 final hook 新增的 before/after xattr。
- 修复后普通 xattr + resource fork **20/20**、最终 hook 动态 xattr **20/20**；`nomifun-file` lib
  **309/309**，macOS workspace **7 passed / 1 ignored**，既有 ACL 回归与 fmt/diff 通过。metadata copy
  后至 rename 的最后窗口、copyfile 故障、超大 resource fork、immutable flags、非 APFS、正式 UI/
  角色及长期门槛仍开放，不关闭完整 FILE Case 或共享 D03。

- S-D04-11（PROC-047、A08/A11/A13/A17/A19 Windows pre-resume deadline 子断言）：W82 用真实
  Windows suspended process，把 Job assignment 延迟 200 ms，并把共享 process deadline 固定为
  50 ms。事务跨期后从未调用 `ResumeThread`，用户 marker 零创建；精确进程句柄在返回前已终止，
  结果为可证明 pre-spawn `SpawnFailed`。首次耗时约 220 ms，未重新获得 5 秒 setup timeout。
  生产实现首次满足，本批只扩展 audit facade 和回归。首次 **1/1**、重复 **20/20**，process Runtime
  lib **125/125**。Unix/macOS 启动阶段、刚过 resume 的 StartLost、清理失败与完整 UI/角色仍开放，
  不关闭完整 PROC-047 或共享阶段。

- S-D04-12（PROC-048、A05/A08/A10/A11/A13/A17/A19 运行中 deadline 部分效果子断言）：W83
  新增跨平台真实 helper，先写入固定 `partial effect before timeout` 文件，再保持运行 60 秒；统一
  deadline 为 1 秒，清理阶段分别为 50/50/500 ms。Windows Job 与 WSL2 Linux process group 均返回
  `TimedOut`、`cleanup.reaped=true`，磁盘保留逐字节一致的部分文件，没有删除或声称回滚。生产实现
  首次满足，本批只补 helper 与回归。两平台各重复 **20/20**；Windows process contract **11/11**，
  Linux **12/12**。持续写入截断量、后代进程、macOS、正式 UI/模型部分效果披露和 IO fault 组合仍
  开放，不关闭完整 PROC-048 或共享阶段。

- S-D04-13（PROC-050、A08/A11/A13/A17/A19 cleanup authority 分类子断言）：W84 复核既有
  两平台故障注入。Windows Job 的首次 member snapshot、supplemental snapshot 与 terminate 失败都
  保留精确 handle/Job authority 并在重试后完成清理；Unix relay 在 group quiescence 未证明时保持
  `CleanupOwned` 且 completion 仍为 Running，只有 ECHILD 的明确身份丢失才隔离，并且隔离前不向
  缓存 PGID 发信号。分类器仅把 `Unsupported` 视为永久 authority lost。生产实现和既有回归首次
  全部通过，本批只补进度证据；Windows 临时 snapshot 代表场景 **20/20**，Unix retry 代表场景
  **20/20**。macOS 真实 relay、跨重启 quarantine、组合故障与长期压力仍开放，不关闭完整
  PROC-050 或共享阶段。

- S-D04-14（PROC-042、A03/A11/A13/A17/A19 并发 start 容量预留子断言）：W85 用 barrier 在公开
  supervisor 上同时释放 32 个真实 helper start，并把容量固定为 2。Windows 与 WSL2 Linux 都恰好
  允许 2 个 helper 写出 PID，另外 30 个在物理 spawn 前返回 `capacity_exhausted`；独立目录中没有
  超额 marker。两个已准入进程均 cancel/reaped 后，新 start 能复用额度。生产实现首次满足，本批只
  新增 helper 与回归。两平台各重复 **20/20**；Windows process contract **12/12**，Linux
  **13/13**。start/shutdown 竞争、Lost/quarantine 占额、PTY、macOS 与 1,000 次长稳仍开放，不关闭
  完整 PROC-042 或共享阶段。

- S-D04-15（CMD-138、A02/A08/A17/A19 最小定向测试子断言）：W86 的隔离 Bun 仓库规定只运行
  `bun test tests/sum.test.ts`；目标测试通过，无关测试带独立失败哨兵。前三次正式 UI 均选对命令，
  但首次模型请求前未装载根 `AGENTS.md`，模型把它与源码当普通读取，Runtime 只能在 dispatch 前
  延迟整批并形成可见异常；只补工具说明及 system instruction 仍复现，第三次另保留显式空
  `requirement_ids` 被拒现场。根因是新 `ScopedInstructions` 初始 `dirty=false`，首个 model boundary
  无法物化已知根 scope。现初始标记为 dirty，在第一次模型请求前注入根指令；说明仍明确 scope
  读取合同，completion 的显式 requirement 数组至少一项，覆盖全部需求时必须省略字段。四项精确
  回归各 **20/20**，Agent Runtime **180/180**。最终正式 Tauri `b982a32a93d6…` 只有 scope 读取、
  一次目标 start、一次 poll 和完成报告，目标 **1 pass/0 fail**、无关哨兵未触发、UI 零异常；工作区
  不变，进程与 profile 清零并正式备份。其他 package manager/语言/仓库、其他 Agent/平台、N3/
  100 seed/LONG/99% 仍开放，不关闭完整 CMD-138 或共享阶段。

- S-D04-16（PROC-043、A03/A08/A11/A13/A19 session lease 子断言）：W87 复核 supervisor 已有
  到期与续期合同。无人 poll 的真实进程输出会续期；owner 认证的 poll、write、status 各自跨过
  原始 lease 后仍保持 Session。无活动 Session 到期后先进入单一 retirement，执行 cancel 并取得
  exact reap，再从 registry 移除；进行中的动作阻止 reaper claim，未 reap 的 retirement 继续占用
  容量。生产实现首次满足，本批未改 lease/清理逻辑。Windows 与 WSL2 Linux 六项精确场景各
  **20/20**（每平台 120 次），`session_registry` 各 **12/12**，两平台 helper 均为 0。首次 WSL
  内联脚本因宿主引号损坏而在测试前退出，作为夹具失败独立保留后改为落盘脚本。PTY/resize/
  close-stdin 的显式续期、macOS、应用强退、1,000 进程长稳与正式 UI/角色仍开放，不关闭完整
  PROC-043/044 或共享阶段。

- S-D04-17（PROC-034～037、A08/A11/A13/A17/A19 process tree 子断言）：W88 复核真实
  Windows Job 与 WSL2 Linux process group。Windows cancel 在无可信 console interrupt 时升级并在
  5 秒合同内精确回收 leader + grandchild；leader 先退出时，Job 后代清理完成前不发布成功。Unix
  普通 cancel 清整组，无视 SIGINT 的整组在完整 grace 后升级 SIGTERM；leader-first 仍保留真实
  exit 0，`setsid` 逃逸则在有界时间返回 `Lost`，未假报 EOF 或 cleanup 成功。逃逸进程由测试 oracle
  单独清理。生产实现首次满足，本批未改信号或 owner 逻辑。Windows 两项各 **20/20**，WSL2 四项
  各 **20/20**；共 120 次且两平台 helper 为 0。ConPTY/PTY、强制 kill 级升级、macOS watchdog/
  session、父进程死亡、应用入口及长期压力仍开放，不关闭完整 PROC-034～041 或共享阶段。

- S-D04-18（PROC-040/041、A03/A11/A13/A17/A19 shutdown/parent-death 子断言）：W89 的真实
  supervisor shutdown 先关闭 start gate，清理两条活动 Session 并按原 owner/session 报告终态，
  shutdown 后的 marker 命令在 spawn 前被拒；Windows 和 WSL2 Linux 各 **20/20**。宿主直接退出
  时，Windows `KILL_ON_JOB_CLOSE` 回收 leader + grandchild，Linux process-group watchdog 与外部
  PTY-session watchdog 均由 subreaper 精确观察并回收；Windows 一项 **20/20**，Linux 两项各
  **20/20**。生产实现首次满足，本批未改 start gate、Job/watchdog 或报告逻辑；共 100 次且两平台
  helper/harness 为 0。真实 Tauri 主进程正常/强退、Windows ConPTY parent-death、macOS、启动事务
  与 shutdown 竞争及长期压力仍开放，不关闭完整 PROC-040/041 或共享阶段。

- S-D04-19（PROC-044、A05/A11/A13/A15/A19 1,000 短进程子断言）：W90 新增默认忽略、显式
  运行的共享 soak 回归；普通套件不承担长耗时。每轮以容量 8 的公开 supervisor 连续启动 1,000
  个真实 helper，逐个要求 exit 0、exact reap、固定 32 字节输出且 dropped=0，最后 shutdown report
  为空并比较宿主资源。首个夹具因 `start` receiver 类型错误编译失败，独立保留后修正为公开
  `Arc<ProcessSupervisor>` 合同。Windows 与 WSL2 Linux 均连续两轮 1,000/1,000 通过；最终 Windows
  178.25 秒，handle 111→111、thread 13→10，Linux 171.79 秒，fd 11→11、thread 8→8，helper
  均为 0。生产逻辑无需修改。并发短进程、PTY/ConPTY、macOS、真实应用/UI 与更长 soak 仍开放，
  不关闭完整 PROC-044 或共享阶段。

- S-D04-20（PROC-047、A08/A11/A13/A17/A19 Unix setup deadline 子断言）：W91 在 WSL2 Linux
  固定三条 pre-exec 窗口。spawn gate 排队 300 ms 时只使用原 100 ms setup budget，返回前 worker
  停止、watchdog 未 fork、用户 marker 不存在；blocking worker 卡在事务入口并越过 75 ms 后，
  保守返回 `StartLost`，释放后仍不能 fork leader/watchdog；watchdog 已建但 ACK 被扣留时，100 ms
  deadline 不叠加第二轮 setup/cleanup，用户代码零执行，exact watchdog 最终只 reap 一次。生产实现
  首次满足，本批未改 Unix 启动逻辑；首轮 **3/3**，三项各重复 **20/20**，残留 owner 进程为 0。
  首次内联 WSL 脚本引号失败发生在测试前并独立保留。macOS、PTY setup、exec 临界点、真实 UI/角色
  与长期调度压力仍开放，不关闭完整 PROC-047 或共享阶段。

- S-D04-22（CMD-143、A02/A08/A17/A18/A19 长历史命令子断言）：W92 首次正式 Tauri 回合最终
  执行了 W92-001～050，但 56 个模型步骤中共有 55 次 `exec_command` 提案；step 1/18/34/36 把
  executable 的 `args` 错挂到 shell-script `cmd` 分支，step 17 又提交空对象。五次均由 Schema 在
  dispatch 前拒绝，实际 effect 仍恰为 50；最终 completion 却把可见失败数写成 0。失败 Session、
  2,499 条事件、截图、数据库与轨迹独立保留。根因是两个输入分支虽严格互斥，模型可见说明缺少
  紧邻字段的完整 JSON 对照；Runtime 又未把已累计的 `failed_tools` 放进 completion 上下文/报告。
  现明确 `{command,args}` 与 `{cmd}` 两种形状，args 只属于 command；有错误的回合必须提交动态
  const `observed_tool_error_count`，Runtime 校验、持久化并固定追加披露，后续成功不能抹掉错误。
  六项精确回归各 **20/20**，合并远端后 Agent Runtime **187/187**；旧夹具首次 **176/182** 失败后按真实累计
  数修正，不放宽合同。合并远端后的最终正式 Tauri `712cf9457e11…` 单回合 51 步、2,454 条事件：前 50 步均为
  独立 `command=cmd.exe` + JSON args，50/50 exit 0/reaped、marker 精确有序，第 51 步完成报告，UI/
  canonical 错误均为 0；工作区不变，进程与 profile 清零并正式备份。其他命令语义、Provider/模型/
  角色/平台、50 个独立用户回合及 N3/100 seed/LONG/99% 仍开放，不关闭完整 CMD-143 或共享阶段。

- S-D04-24（OBS-002、REAL-021～024、A02/A08/A17/A18/A19 工具错误披露子断言）：W93 使用
  W92 最终二进制和新的正式 Tauri 隔离 data/work/profile，按负向协议先提交一次无效
  `exec_command {cmd,args}`，再提交合法 `{command,args}` 恢复调用。首次调用以
  `INVALID_TOOL_ARGUMENTS`、`status=not_executed` 在 dispatch 前拒绝，零 process effect；第二次
  输出精确为 `W93-RECOVERED`、exit 0、`reaped=true`，唯一 process effect 为 returned。模型在第
  3 步一次 `report_completion`，显式提交 `observed_tool_error_count=1`；最终交付保留模型说明并由
  Runtime 固定追加“本回合观察到 1 次工具错误，后续成功未抹除”。单回合 completed、3 个模型步骤、
  98 条 canonical 事件，独立 24 项断言通过；工作区 tree hash 不变，cmd、应用和 profile 进程清零，
  正式备份完成。生产代码无需再改。本批只验证一种参数预检错误及 StepFun Coding Plan /
  `step-3.7-flash`；业务错误、权限/超时/unknown、其他 Provider/模型/角色/平台与长期矩阵仍开放，
  不关闭完整 REAL/OBS 或共享阶段。

- S-D04-25（PROC-015、OBS-004、REAL-004/021～024、A05/A08/A17/A18/A19 命令失败披露
  子断言）：Runtime 原已累计 `failed_commands`，但完成报告只强制披露工具结果错误；一次非零退出
  后恢复成功时，模型可以不申报命令失败。W94 新回归首次按预期失败。现增加动态 exact const
  `observed_command_failure_count`，非零时必须提交、持久化并参与报告新鲜度判断；最终交付固定追加
  命令失败数，后续成功不能抹除。若工具错误和命令失败同时非零，模型说明给出包含全部必填计数的
  单个精确 JSON 对象。首次正式 Tauri 虽正确申报命令失败数，却漏掉同一非零命令产生的工具错误数，
  completion 先被 Schema 拒绝后恢复，保留为 `FAIL_RECOVERED`。同一夹具和模型修复后精确执行
  exit 7、更新计划、恢复命令，并在第 4 步一次报告两个计数均为 1；147 条 canonical 事件、2 个真实
  process effect、工作区和清理结果独立一致。三项新增回归各 **20/20**，Agent Runtime **190/190**，
  正式构建通过。其他非零码/信号、timeout/lost、多个失败、其他 Provider/角色/平台及长期矩阵仍开放，
  不关闭完整 PROC/REAL/OBS 或共享阶段。

- S-D04-26（G0-023、PROC-048、REAL-021～024、A05/A08/A11/A13/A17/A18/A19 运行中 timeout
  披露子断言）：W96 复用 W94 最终二进制，以正式 Tauri 和 `step-3.7-flash` 运行 250 ms deadline
  的真实 Windows pipe 命令。结果为 `timed_out`；无可信 console interrupt 后按合同升级 terminate，
  1,174 ms 内 `reaped=true`，未使用 force kill。模型随后单独更新计划、执行恢复命令，并在第 4 步
  首次 completion 同时提交 tool/command 计数 1；最终 UI 固定披露两种失败均未被恢复成功抹除。
  单回合 completed、147 条 canonical 事件、2 个 returned process effect，独立 35 项断言通过；
  W103 后续审计发现 timeout 与 recovery 两个 criterion 都误引 recovery call ID，因此上述通过不含
  证据语义关联，该子断言改由 S-D02-19 追记失败并闭环；计数、终态和清理结论仍成立。
  工作区不变，ping/cmd、应用和 profile 进程清零并正式备份。生产代码无需再改。本批未制造外部
  文件部分效果；持续写入、force-kill/lost、其他命令/Provider/角色/平台与长期矩阵仍开放，不关闭
  完整 PROC-048/REAL 或共享阶段。

- S-D04-27（PROC-011/012、CTRL-006/007、REAL-011/021～024、A02/A05/A08/A17/A18/A19
  进程 non-start 控制子断言）：W97 首次正式 Tauri 中，typed `PROCESS_NOT_STARTED` 正确证明用户代码
  零执行且不要求 replan，却也没有激活 TaskLedger；下一步看不到 `update_plan`/`report_completion`，
  模型两次 ToolSearch 均无结果，随后重放一次 non-start 和一次恢复命令才完成。9 个模型步骤、4 个
  process effect 及“摘要称工具错误 1、Runtime 实际披露 2”的 `FAIL_RECOVERED` 全部保留。现让 typed
  non-start 仅激活 Ledger/Completion 控制，不改变无需 replan、工作区证据不失效和用户代码零执行
  语义。新回归首次 **0/1**，修复后 **20/20**，Agent Runtime **191/191**。同一正式 UI 夹具随后
  精确 4 步完成：一次 non-start、一次计划、一次恢复、一次报告；tool error=1、command failure=0，
  137 条事件、2 个 returned effect，独立 31 项断言通过，工作区和进程清理一致。其他 spawn
  permission/format/cwd、其他 Provider/角色/平台及长期矩阵仍开放，不关闭完整 PROC/REAL 或共享阶段。

- S-D04-28（PROC-029/030/031、A11/A13/A17/A19 stdin 大小、owner 与终态边界子断言）：W106
  首轮新反例全部通过，没有产品首次失败。Engine owner 接受精确 1 MiB 单次写入；1 MiB+1 在进入
  transport 前拒绝，随后同一进程只收到允许的 4 字节，证明无部分写。已结算进程拒绝迟到 stdin，
  再从 cursor 0 重放仍保持原 exited 终态和输出。App host 将 W105 新增的结构化 LF 一并计入 1 MiB
  预算，精确上限加 LF 同样在 journal/dispatch 前拒绝。既有 Runtime 回归另确认未知 Session 与错误
  invocation/call owner 使用稳定拒绝码且零写入，终态 PTY resize 失败而不复活。三项新 Engine 反例
  **60/60**，Engine **32 通过 / 1 ignored**、App host **4/4**，Runtime 三项 owner/terminal 定向检查
  通过。本批只补最小回归并提取等价预算判断；未调用模型或正式 UI。多次分块累计上限、并发
  input/close、终态错误的正式 UI 呈现、其他平台及 N3/LONG/99% 仍开放，不关闭完整 PROC 或共享阶段。

- S-D04-29（PROC-032、A11/A13/A17/A19 PTY resize 上限子断言）：W107 首次反例确认标准工具与
  canonical Schema 均允许 65535，Engine 启动校验也接受 32768；Windows ConPTY 的 Win32 `COORD`
  实际只支持 signed 16-bit，越界值因此越过预检后才成为 I/O/unknown。三个首次失败独立保留。
  现以共享 `MAX_PTY_DIMENSION=32767` 统一 Runtime、Engine、App pre-journal 校验和 start/resize 两套
  模型 Schema；0、32768 及以上在 owner/dispatch 前返回稳定 invalid transport，合法 132×43 仍真实
  resize 并可取消/reap。ConPTY 合法与越界组合 **40/40**，process Runtime **125/125**、PTY
  **8/8**、Runtime **195/195**、Wave2 **22/22**、Engine **33 通过 / 1 ignored**、App host **5/5**。
  本批未调用模型或正式 UI。Windows 通用 ConPTY close 仍明确不声称可证明 generic EOF，PROC-028、
  尺寸实际生效的应用级观测、其他平台及 N3/LONG/99% 仍开放，不关闭完整 PROC 或共享阶段。

- S-D04-30（PROC-040、LIFE-029、A03/A11/A13/A17/A19 进程 owner 关闭与长 poll 子断言）：W168
  的真实 pipe child 已发布独立 PID，60 秒 poll 已进入等待后，shutdown 在 6 秒内无法发起清理；
  释放 poll 才取得唯一 Cancelled/reaped。证据根为外部 `2026-09-30/windows/w168-process-start-shutdown`，
  产品首败保留于 `01-first-long-poll`。修复 registry：所有操作仍保护 lease/容量，
  只读 poll 单独不阻塞 shutdown retirement；stdin/close/resize 等写操作保留原关闭屏障。
  默认 1/1/3 秒 stop 预算下首次及重复 **21/21**，独立 PID/report 核对 **21/21**、helper 残留 0；
  shutdown **5/5**、natural exit **5/5**、cancel-first **1/1**、registry **13/13**，fmt/diff 通过。
  旧 75 毫秒夹具重复第 13 轮返回 Lost/reaped=false，首败和独立兜底清理单列保留；恢复生产默认
  预算后仍要求原 6 秒上界、唯一 Cancelled/reaped、同一 poll/report 和精确 PID 消失，未放宽断言。
  真实 start/shutdown、start future drop、正式 Tauri、ConPTY 父死亡、其他平台/角色及完整
  N3/100 seed/LONG/99% 仍开放；只通过该子断言，不关闭完整 PROC/LIFE 或共享阶段。
- macOS M06-01 的仓库外 PID evidence 重跑首败在 poll 前：marker 不属于原 helper cwd 的 Seatbelt
  工作区。夹具现将 canonical evidence root 同时作为 cwd/唯一 capability root，未增加旁路授权。
  修复后首次 + 20 repeats **21/21**，独立 PID/report **21/21**，6 秒上界、原 poll 唤醒与相同 report
  全满足；普通 shutdown **3/3**，helper 残留 0。证据位于 `2026-09-30/macos/m06-native-shutdown/`；
  该原生子断言通过，正式 Tauri/更多拓扑与完整统计门槛仍开放。

- S-D04-31（PROC-039/040/042、CONC-004、A03/A10/A11/A13/A17/A19）：W170 手动首次 poll
  命中原生 spawn 的真实 await，PID 标记和精确 OS handle 证明 child 已执行；drop caller 后 shutdown
  空报告且 child 当时仍活，平台后续 Drop 清理不能替代该时点的证明。启动改为宿主持有的 worker，
  保留原原生取消、准入租约/预留与结果 ACK；未交付结果仍清理同一 Session，ACK 不持准入锁。
  中间实现把准入放到 worker 后又被真实 quiesce 反例揭示：空 exact fence 后旧 start 继续执行。
  该引入失败保留并修为 public 首次 poll 取得 read_owned/预留后连续移交。最终五项首轮及
  **20/20 × 5**、独立 PID/磁盘 **105/105**、自有 helper 0；直接相邻 **11/11**、API **21/21**，
  WSL Linux 单包兼容编译与 fmt/diff 通过，未代判 macOS。原生失败/unknown/worker panic 组合、
  ConPTY 本组真实竞态、正式 Tauri/其他平台角色及 N3/100 seed/LONG/99% 仍开放。证据见外部 W170。

- **S-D04-32**（`PROC-039/040/042`、`CONC-004`、`LIFE-029`，Unix post-COMMITTED start
  取消子断言）：M06-02 首次重复证明真实 child 执行后，外层取消位在 blocking transaction 末尾触发
  `post_exec_failure`；native owner 被转交平台 poller，而 Supervisor reservation 已释放，shutdown
  因而返回空报告且进程当时未消失。固定窗口新反例在 cleanup 后仍明确 FAIL，首次结果均保留。
- 对 Supervisor 已持有的 start worker，commit 后外层取消现保留并返回精确 native owner，由原
  Session 统一 retirement/回执；pre-commit 取消、deadline、独立底层 future-drop 与原清理预算不变。
  macOS Pipe **105/105**、PTY **42/42**，独立物理 PID/磁盘 **147/147**，fallback 0；新增固定窗口
  **21/21**、底层 drop 相邻 **2/2**、pre-fork deadline **1/1**、shutdown **8/8**、
  registry shutdown **3/3** 与 fmt/diff 通过。证据 `2026-09-30/macos/m06-start-delivery/`。
  仅关闭该 Unix 取消窗口根因；Linux 原生执行、native failure/unknown/worker panic、正式 Tauri、
  其他角色及完整 Case/长期统计仍开放，Windows 原结果未改写，付费模型调用 0。

- **S-D04-33**（`PROC-039/040/042/047`、`CONC-004`、`LIFE-029`，Unix 已提交进程 IO setup
  failure 子断言）：M06-03 注入真实 child COMMITTED 后的 stdio 转换失败；旧路径只把 native
  lifecycle 交给平台 poller，Supervisor 释放 reservation 后 shutdown 空报告，PID 当时未回收。
  首败保留。内部交付现区分成功与带原 startup failure 的 native owner；后者注册原 Session，
  按原 retirement 证明清理或保留 Lost/unproven，普通错误返回原 StartLost/code/PID，并发 shutdown
  保留取消优先及 owner 清理报告，未变成成功或 not-started。
  独立底层 wrap/drop 合同不变；Windows 构造只补 None，不代判其原生结果。
- macOS Pipe/PTY × 原错误返回/shutdown 四场景 **84/84**，独立 PID/磁盘/报告 **84/84**；
  Runtime lib **149/149**、registry **13/13**、fmt/diff 通过。完整 PID 行作为 marker 发布条件，
  保留空文件解析夹具首败且不改原期限。证据 `2026-09-30/macos/m06-start-failure/`，付费模型 0。
  native setup deadline/commit failure、worker panic、cleanup failure/unknown recovery、Linux 原生、
  正式 Tauri/更多角色及完整统计门槛仍开放，仅关闭该 IO 转换失败所有权窗口。

- **S-D04-34**（`PROC-039/040/047`、`CONC-004`、`LIFE-029`，Unix startup deadline 所有权）：
  M06-04 固定真实 COMMITTED worker；旧 timeout 丢弃 JoinHandle、释放 Supervisor 预留，caller
  105 ms StartLost 后 shutdown 空报告、PID 未回收。首败保留。现 deadline 通知只终止 caller waiter，
  原 worker 持有 JoinHandle/预留/准入租约直至原事务结束，迟到 owner 只做带原 deadline failure 的
  retirement。standalone bounded timeout 不变；pre-fork 事务 release 后仍受原 deadline/取消位约束。
- macOS 四场景首次 + 20 repeats **84/84**，独立物理 PID **63/63**、未开始窗口零 fork **21/21**；
  setup 100 ms，caller 最大105 ms，350 ms caller/6 秒 cleanup 原断言不变。所有 boundary 在原
  worker 被持有时保持 pending，最终 owner/清理报告一致；最终 Runtime **153/153**、registry **13/13**、
  fmt/diff 通过。证据 `2026-09-30/macos/m06-start-deadline/`，付费模型 0；仅关闭该受控窗口，
  handshake failure、worker panic、cleanup failure/unknown recovery、Linux/Windows 原生、正式
  Tauri/其他角色和完整 LONG/统计仍开放，Windows 原结果未改写。
- 中间 raw PTY watchdog-after-COMMITTED 注入曾在 spawn 返回 PeerClosed（152/153），原日志在
  `m06-start-deadline/run-007-retained-io/`，当批未关闭；接续首次失败根因见 `S-D04-35`。

- **S-D04-35**（`PROC-041/046/047`、A11/A13/A19，macOS watchdog fork 前置）：M06-05
  对齐旧首败与系统 crash，确认 watchdog 在系统 `_notify_fork_child` 的懒初始化访问中终止，
  未进入 BootReady，而非 COMMITTED 后清理错误。实现依据
  [Apple libnotify](https://github.com/apple-oss-distributions/Libnotify/blob/main/notify_client.c)：
  现两个 macOS 启动入口在父进程中以无注册副作用的负 token 查询完成一次初始化，不跳过 fork
  handlers，不重试隐藏首败；等待仍消耗原 setup deadline，耗尽后不迟到 fork。
- 未完成初始化的受控窗口（非私有 OS 锁损坏注入）首次 Pipe/PTY **0/2**，修复后等待/超时
  四场景 **84/84**；原生 pipe/PTY post-COMMITTED fault **42/42**，独立 PID 消失 **126/126**、
  零 fork **42/42**，caller 最大103 ms；Runtime **157/157**、registry **13/13**、boundary/fmt/diff
  通过。证据 `2026-09-30/macos/m06-pty-watchdog/`，付费模型 0。只关闭该 macOS 初始化根因；
  其他 fork callback、正式 Tauri/父死亡与 sleep/wake、未知恢复、其他平台/角色及完整 Case/统计
  仍开放；Linux/Windows 分支不增加该初始化，也不改写已有 Windows 验收结果。

- **S-D04-36**（`PROC-040/050`、A08/A11/A13/A19，Unix 已 exec 的 commit failure）：M06-06
  首败证明 native transaction 握手错误已转交底层清理器，但 Supervisor 丢失该 owner/预留，
  shutdown 在 PID 未回收时空报告完成。现已执行 transaction 的错误携带 cleanup-only owner，
  复用原 Session retirement；startup failure 与机器回收证明分开，不发布可用进程 handle，
  普通错误保留原 code/PID/reap，shutdown 保留取消优先及原 owner 报告。standalone raw/drop 不变。
- 清理证明只在精确 direct-child reap + group quiescence 时产生；失去 group anchor 仍为未证明。
  中间收据 **2/4** 两次首败另发现普通退出等待覆盖了历史 cleanup 诊断，已保留并修复；最终
  收据携带原握手错误和最近实际诊断，而非吞错换 PASS。macOS Pipe/PTY 四场景 **84/84**，
  独立 marker/错误/报告 **84/84**、物理 PID 消失 **168/168**，fallback 0；anchor 负向 **1/1**、
  Runtime **162/162**、registry **13/13**、boundary/fmt/diff 通过。证据
  `2026-09-30/macos/m06-commit-failure/`，付费模型 0。仅关闭该窗口；caller-drop/握手失败组合当批未验，native 接续见 M06-07；
  pre-exec deferred cleanup、其他 transfer failure、worker panic/未知恢复、Linux/Windows 原生、
  正式 Tauri/更多角色及完整 Case/长期统计仍开放，Windows 原验收未改写。
- macOS M06-07 复核 `S-D04-36` 的 caller-drop/commit failure 组合，新增四个最小 native 回归，
  无需修改产品逻辑。Pipe/PTY × 无 shutdown/quiesce 首次 + 20 repeats **84/84**；原/后续各一次
  物理启动 **168/168**、原 owner 报告 **84/84**、PID 消失 **336/336**，capacity probe 零 dispatch
  **84/84**、fence held **42/42**、无 shutdown/lease cleanup **42/42**、容量/准入复用 **84/84**，
  fallback 0；Runtime **166/166**、registry **13/13**、boundary/fmt/diff 通过。首次结果均通过且保留。
  证据 `2026-09-30/macos/m06-dropped-commit/`，付费模型 0；真实 Engine Turn cancel、正式 Tauri/
  角色、pre-exec deferred cleanup、其他 transfer failure、worker panic/未知恢复及其他平台/完整
  统计仍开放，不把该组件回归当完整 Case PASS，不改写 Windows 原结果。

- **S-D04-37**（`PROC-040/047/050`、A08/A11/A13，Unix pre-exec deferred cleanup）：M06-08
  首败在用户代码零执行时，watchdog 清理已转底层 poller，而 Supervisor 释放 startup reservation，
  shutdown 空报告先于辅助 reap。现原 worker 等待同一个辅助清理 witness，保持原准入/容量；
  原 deadline 只结束 caller 等待，未复位预算。失败启动单列 `StartupCleanupReport`，带原 owner/
  host SessionId/error/cleanup/未 exec 事实，不创建虚假用户 PID 或 process handle。
- 中间 **3/4** 首败证明 transaction 恰先于 timer 交付时，第二段辅助等待漏 deadline 通知；
  两阶段现共用原绝对 deadline。startup non-exact 保持 quarantine/占额；已证明报告按容量有界，
  quiesce/shutdown 的 exact 判定包含 startup 清理。负向夹具缺 import 编译失败亦保留。
- macOS 四场景 **84/84**，独立辅助 PID 消失/精确收据/容量阻止 exec **84/84**，user exec 0、
  假用户 Session 0、fallback 0；caller 最大106 ms（原350 ms），quarantine/有界报告 **2/2**，
  Runtime **172/172**、registry **13/13**、Engine process **16/1 ignored**、boundary/fmt/diff 通过。
  证据 `2026-09-30/macos/m06-preexec-cleanup/`，付费模型 0。仅关闭该受控窗口；真实永久
  authority loss/恢复、其他 transfer failure、worker panic、正式 Tauri/角色、Linux/Windows 原生与
  完整 Case/统计仍开放，Windows 原结果未改写。

- **S-D04-38**（`PROC-039/050`、A13/A18，Engine/App 未注册启动的迟到收据）：M06-09 首个
  App 组件反例证明，即使已有精确清理证明，`unregistered_start` 仍永久报 unknown。现 Engine
  在首个 native await 前跟踪不可变 host owner，App cleanup 接 native quiesce fence；只有对应
  owner 的已证明 Session/startup 收据能解除标记，不以空 map、外来 call 或非 exact 报告代替。
  部分精确收据跨重试保留，cleanup panic 的既有不确定性继续存在。
- macOS Native Engine Pipe/PTY caller-drop 两场景 **42/42**，独立 PID/owner/清理 **42/42**；
  App process-host **8/8**、Core process **20/1 ignored**、fmt/boundary/diff 通过。原组件首败及
  两次 native 夹具编译失败保留，最终走既有安全身份 API，无新增依赖或 unsafe lint 放宽。
  证据 `2026-09-30/macos/m06-engine-startup-fence/`，外部模型实际调用 0；正式 Tauri/live
  Provider、pre-exec auxiliary App 全链路、完整真实 Turn cancel、永久 authority loss/恢复、
  worker panic、其他平台/角色与完整统计仍开放，Windows 原结果未改写。

- S-D09-05（LIFE-020、A10/A12/A17/A19 cancel 后重启子断言）：新增完整 AppServices 重建回归。
  先在 429/Retry-After 等待中取消 Turn，确认 canonical cancelled 后关闭首个 App/数据库，再从
  同一隔离 data root 重建服务与 Router。路由发布前的 startup recovery 候选精确为 0，provider
  请求仍为 1；GET execution 保持 cancelled、checkpoint 不保留、`execution_resumed` 为零。
  首次即通过，完整 App recovery 6/6，新回归 **20/20**。生产恢复查询无需修改，本批仅补测试/
  进度；强杀中间窗口、cancelled Session 删除、真实 UI/平台及长期恢复矩阵仍开放。

- S-D09-04（LIFE-019、G0-025、A04/A05/A10/A17/A19 cancel/receipt 竞争子断言）：新增
  canonical Store 两连接事务竞争，让 Turn cancel 与同一 managed effect 的成功 owner receipt
  同时提交。最终始终只有一个 cancelled Turn terminal；已完成 effect 保留 `effect/succeeded`
  receipt 并投影为 returned，active Turn 清空，没有把 cancel 当回滚。首次短 filter 实际未命中；
  随后两次真实 FAIL 分别暴露夹具复用了 started producer、又误改生命周期 idempotency key，均保留。
  按既有合同使用原 key + 独立 owning producer 后，完整 pause/effect 11/11、新竞争 **20/20**。
  生产事务归约无需修改，本批只补回归/进度；App owner 回执、外部 unknown effect、真实 UI 与
  其他 Action/平台的 cancel 竞争仍待验。

- S-D09-03（MODEL-020/022/023、LIFE-018、A08/A10/A11/A17/A19 Retry-After 取消子断言）：
  新增正式 App 路由夹具，provider 首次返回 HTTP 429 与 60 秒 `Retry-After`，在 broker 退避期间
  通过 canonical Turn cancel 取消。Turn 在 2 秒验收预算内进入 cancelled，checkpoint 清空；再观察
  1 秒仍只有一次 provider request，零后续 model/tool attempt，且没有 completed/failed terminal。
  首次即通过，完整 App recovery 5/5，新增回归 **20/20**；Broker Retry-After 3 项和 HTTP 解析
  2 项保持通过。当前生产取消传播正确，本批只补产品路径回归/进度，无生产或 renderer 修改。
  尚未覆盖真实 provider 连接池/代理切换、取消与 effect receipt 竞争、UI 点击停止及其他平台/角色。

- S-D09-02（LIFE-016、PROC-033/039、CONC-004、A12/A13/A17/A19 进程 in-flight 暂停子断言）：
  新增正式 App 路由夹具，通过实际 `start_process` 启动写入 PID 后长驻的 OS helper，并在下一次
  provider 请求等待时发起 owner pause。独立进程快照先证明 PID 存活；paused 发布前 canonical
  `host_cleanup_proven` 已落盘，随后 PID 消失。恢复使用同一 Turn 的新 generation，保留已对账的
  start/read 历史，重新规划和读取 marker 后 completed；`workspace.process/start` 全程恰好一次，
  活句柄未进入 checkpoint，也没有重放。完整 App recovery 4/4，新增回归同源码 **20/20**。
  当前生产清理/恢复实现通过，本批仅补跨层真实进程回归及测试用 process snapshot 依赖，没有
  修改产品逻辑或 renderer。测试采用 scripted provider 做故障边界，不计真实 StepFun/UI；Browser、
  MCP in-flight、清理失败/人工证明、macOS 原生进程组及完整 LIFE-016 仍开放。

- S-D09-01（CTRL-009/010、LIFE-015/017/020/021/022、CONC-007/008、A06/A10/A12/A17/A18/A19
  pause checkpoint 子断言）：现有 owner 级 pause/resume 生产路由与 canonical Store 已做完整定向
  复核。合法恢复保持同一 Turn，只增加 execution generation/fence；暂停期间禁止新 Turn，旧
  producer 被 fence，已完成 write 不重放。错误 digest/revision、owner、Snapshot、build、active
  set、预算和 cleanup 证明均在提交前拒绝。completed 后的新 resume key 与 cancelled 后的 stale
  preparation 都拒绝；旧 key 只返回原幂等授权回执，不新增事件、模型请求或写入。新增三命令
  竞争证明 cancel 终态唯一且不可逆，pause/checkpoint/active Turn 均清空；两项新增回归各
  **20/20**。相关 Session pause 10、checkpoint 8、App 恢复 3、投影 2、UI 暂停状态 9 项通过。
  两次首次失败均为新增夹具错误（terminal head 应为 ready；terminal 后重新 prepare 本应先被
  fence），已原样保留，未修改产品状态机。本批未发现需改的生产缺陷，只补缺失回归。
  当前 pause checkpoint 授权来自 owner `/execution/resume`，与 W45 model-facing closed-turn
  `resume_task` 不同；会话区只验证暂停提示/发送阻断/停止，尚无 owner resume 正式 UI 入口，
  因此不关闭完整 CTRL-009/010、OBS-007、LIFE-016 或真实 UI/平台矩阵。

- S-D02-12（CTRL-001/006/007、REAL-013、A05/A07/A17/A18/A19 已闭合任务显式继续子断言）：
  W45 从 W44 completed Turn 的正式备份跨构建继续，要求导入全部旧要求、先更新计划，只读两个
  文件且不重放已完成修改。首次确定性 FAIL 证明历史任务仍被旧 build digest 拒绝；现仅允许在
  Session、runtime binding 与精确 Snapshot 不变时跨 engine build 导入，来源 Turn、记录格式和
  当前引用仍严格校验。`resume_task` 只在尚未规划/观察的入口暴露，成功后立即移除；导入计划
  必须先通过仅暴露 `update_plan` 的控制轮次，不能依赖 provider 对 tool choice 的自愿遵守。
  supported completion 现在必须至少引用一项当前证据；分页 `read_file` 也明确说明任意成功页的
  sha256 是整份源文件摘要，避免为取摘要读取全文。
  早期真实样本依次保留重复 resume、计划参数错、无证据完成、重复读取、provider 忽略选择和
  摘要语义误解；其中一份旧二进制虽通过 oracle，代码复核发现门禁会被通用选择归一化覆盖，
  未作为最终证明。最终 Runtime **167/167**，8 项关键回归各 **20/20**；正式 Tauri 5 模型步，
  顺序为 resume→plan→两次目标读取→plan→completion，零工具错误/效果，两文件 hash/mtime
  不变，canonical completed，严格 verdict 为 `PASS_TASK_CONTINUATION`。本项是 closed-turn
  task continuation，不是 CTRL-009/010 的 pause checkpoint/终态恢复；后两项仍开放。W42 N3
  仍为 1/3，完整角色/平台、范围变更、100 seed、LONG/99% 与共享阶段均未关闭。

- S-D02-11（CTRL-006/007、FILE-031、A05/A07/A17/A18/A19 重启后精确修复子断言）：W44 从
  W43 failed Turn 的正式备份继续，先后保留 completed-only 来源拒绝、跨构建 recovery/history
  等值拒绝，以及空计划被恢复上下文刷新误置 `needs_replan` 后首个补丁提案被拒的 FAIL。版本化
  patch 状态与只读闭合历史现可跨应用构建读取，但仍校验记录格式、Session、来源 Turn 和精确/
  模型兼容 Snapshot；运行中 checkpoint 继续绑定原构建。恢复读回只使既有计划失效，不再凭空
  创建计划门禁。18 项去重定向通过，4 项新增各 20/20；最终正式 Tauri 5 模型步，先后各读两
  文件、仅一次 second 补丁及一次成功 effect、零工具错误，first 摘要/mtime 不变，second 精确
  修复，v2 recovery 清空且 canonical completed。该结果记 recovered PASS；W42 首轮 N3 仍为
  1/3，完整角色/平台、范围变更、100 seed、LONG/99% 与共享阶段仍开放。

- S-D02-10（CTRL-006/007、FILE-031、A05/A07/A17/A18 部分任务完成判定）：W42 第 3 次
  首次 FAIL 证明多文件补丁部分发布后，只要重读目标并把事实写成 supported，原任务就会
  误结 completed。根因是恢复状态把“已重读”和“未发布的任务义务”混为一体。v2 状态现
  分别持久化待重读、未完成目标及接纳输入边界；结构化发布索引只结算已确认发布项，读取
  不能结算未发布项。精确目标的成功 write/patch 或有后续用户引用的 scope_changed 才可
  解除；不确定结果保守，零发布负向检查在重读后仍可按原合同完成。161 项去重定向通过，
  11 项新增各 20/20。正式 Tauri 4 模型步/1 rejected effect，无后续操作，second.txt 保持
  原文并列为 unresolved，终态 failed、部分正文可见。普通文本分支实测；误 supported 分支
  为确定性回归。完整角色/平台、N3/长期门槛仍开放，共享阶段未完成。

- S-D02-09（CTRL-007、FILE-031/039、A17/A18 固定样本验收）：W42 使用同一构建、GEN Revision
  和模型配置完成三个独立正式 Tauri 样本。仅第 1 次首轮通过，补到普通文本结束分支的 UI
  证据；第 2 次 blocked 报告使用 `call_...` 占位引用被正确拒绝，修正后结束；第 3 次只重读后把
  部分完成报告标为 supported，canonical Turn 错结 completed。三次各只发布一次补丁，磁盘
  保护及最终正文展示均可核对，共 16 模型步、3 rejected effects；**N3 未通过**，不合并
  recovered 与首轮结果。当前完成账本只验证引用/ID 覆盖，没有证明原任务目标已满足。
  下一批优先补部分任务误判完成的反例与修复，保留未知/失效引用的拒绝规则。W42 未改
  产品/未放宽断言，完整证据见 Windows W42；共享仍未完成。

- S-D02-08（CTRL-007、FILE-031/039、A07/A17/A18 失败交付子断言）：W41 反例确认补丁仍待
  核对时，普通最终回答先触发 completion review，再拒绝成功，重新打开模型工作。现前置
  已有失败检查，保留结果正文及恢复义务；已重读的正常完成路径仍要求有效证据。正式 Tauri
  本次走有效 blocked 报告分支，4 模型步、1 rejected patch、无后续文件操作，但结果正文
  被错误卡片折入过程区。保留 UI FAIL 后，让错误前最后结果与失败提示同时可见；历史游标
  与显示时间混用造成的首版冷加载 FAIL、迟到旧文本反例也保留并修复。123 项去重定向、
  5 项新增各 20/20 通过；同一失败会话正式冷加载正文可见，canonical 四类记录完全不变，
  0 新模型步/效果。普通文本结束分支仍只有组件证据，完整角色/平台、N3/长期门槛及其他
  完成控制问题未关闭；不把 W39/W40 的失败或整个共享阶段改记通过。

- S-D02-07（CTRL-007、FILE-031/039、A07/A17/A18 失败收尾子断言）：W39 的部分补丁失败后，
  Runtime 连 blocked 报告也要求先重读全部目标；与用户错误即停止约束冲突。W40 确定性
  首次 FAIL 保留。现校验受阻报告后将未完成步骤结为 blocked，保留全部输入义务与待核对
  目标，交付摘要并以 failed 结束；成功/仅 unverified、陈旧证据、运行中进程仍不能绕过。
  35 项定向、5 项新增各 20/20 通过。首版真实 UI 又被无效证据引用阻挡，补待核对时实际
  工具说明中的受阻格式；两次真实 UI 仍重读/重试，均停止且保留 FAIL，共 26 模型步、
  4 rejected effects。**仅组件分支已修，产品闭环未完成**；不再将增加提示视为修复证明。
  下一步核对正式模型请求的控制定义、历史/压缩后约束保留与失败报告选择；完整角色、
  原生平台及 N3/LONG/99% 门槛仍开放，不关闭 S-D02-01 或共享阶段。

- S-D03-35（FILE-031/033/039、A05/A08/A17 多文件补偿子断言）：前项发布后被同字节异身份
  文件接管，旧回滚仅比正文，误覆盖外来文件。W39 首次 FAIL 保留；现从暂存创建句柄保留
  发布身份，补偿时同时核对身份/字节，Windows 最后目标句柄再次核对，确认过的迟发失败
  也保留身份；未确认项沿用禁止回滚。真实 Tauri 首轮另发现普通 IO 错误误报平台不支持，
  模型因此声称零修改；现分类为执行失败，并给模型明确发布/恢复数量、零基索引及部分效果
  说明，仍剥离私有诊断。79 项去重定向、10 项新增各 20/20 通过。最终 UI 磁盘/回执和
  部分结果说明一致，但模型随后违反不重试约束再次调用补丁，已停止并保留 FAIL，接续
  S-D02-01 控制链；两次共 23 模型步、3 rejected effects，不计完整 UI/Case 通过。
  原生间隙、Unix/macOS、完整角色与 N3/100 seed/LONG/99% 仍未验，共享阶段未完成。

- S-D03-34（FILE-020/025/038、A05/A07/A17 替换目标确认子断言）：最后校验后目标名称被
  外来文件接管，虽已返回 unknown 并保留备份，仍把预期正文标为确认，允许当前项自动回滚。
  W38 不同内容/相同内容异身份两项首次 FAIL 保留。现持有原目标保护句柄，核对备份仍为
  该对象后才确认发布；内部标记改为 publication_verified，未确认项只请求重读并排除回滚。
  56 项定向、4 项新增各 20/20 通过，含真实原生置换后的通知/回滚与现有映射的零发布拒绝。
  0 模型/UI。文件 symlink 夹具返回 1314，单独阻断；原生间隙零副作用、换走后换回、其他
  映射组合、Unix/macOS、完整角色与长期门槛仍未证明，不计完整 Case 或共享阶段完成。

- S-D03-33（FILE-040、A05/A14/A17 清单对账子断言）：API 与 Agent 文件实例各持缓存，
  通知期间仍可读到旧清单；写/删仅失效当前根，重命名没有失效。W37 四项原生首次 FAIL
  及关闭 Office 自动预览时的正式 Tauri 首次 FAIL 保留。清单状态现由 AppServices 共享，
  各读取先按自身路径授权校验；发布/删除同步失效相交根，重命名覆盖旧名和新名。
  31 项定向、7 项新增各 20/20 通过，含热缓存越权拒绝、另一实例写入时旧扫描失效及真实
  路由接线。正式 Tauri 新建后恢复新候选、保留旧候选；前后共 6 模型步、2 returned effects。
  无 watcher 的外部变更、原生发布窗口、完整丢批/乱序、其他角色/平台及长期门槛仍开放，
  不计完整 FILE-040 或共享阶段完成。

- S-D03-32（FILE-018/038/039/040、A05/A07/A08/A09/A17 预览对账子断言）：未确认发布没有
  重读通知，无内容事件又被前端当成空白更新；保存回包还会把后续编辑误标已保存。W36 首次
  失败保留。现以不带正文的通知要求按原工作区重读，迟到回包按页签/来源/尝试隔离，保留脏
  草稿、明确读失败与重试，保存只确认实际送出的内容；补关闭重开激活及失败保存的加载状态。
  首版正式 Tauri 仍失败，进一步确认 Wave2 宿主丢弃事件；生产构造现必须接入应用的用户事件
  总线，仍按 owner 投递。72 项去重定向通过，18 项前端及 3 项原生/宿主关键回归各 20/20；
  正式 Tauri 新隔离会话的 unknown 重读、读失败告警与手动重试三项通过；三次会话合计
  9 模型步、3 pending effects，未把预览恢复记成写入成功。其他预览类型/角色、丢批乱序、
  Unix/macOS、原生发布窗口与长期门槛仍开放；不计完整 Case 或共享阶段完成。

- S-D03-31（FILE-020/038/039、A05/A07/A17 暂存发布证据子断言）：原生替换返回成功即删
  备份，暂存被改写或换成同内容外来对象仍成功；未确认结果还广播预期字节。W35 首次失败
  均保留。Windows 现按创建时身份和调用字节核对发布前后对象，确认前保留原件备份；无法
  确认则返回 unknown、记录未确认索引并排除该项自动回滚。仅有字节证明才发内容事件，其他
  不确定状态失效缓存；清理/恢复不确定性统一保留防重放保护。65 项去重定向、11 项各 20/20
  通过，含消息完整性及重启/换 key 拒绝；0 模型/UI。本批未证明原生间隙零副作用；消费者
  重读通知、目标名称/映射写入、Unix、完整 UI/角色及长期门槛仍待验，共享阶段未完成。

- S-D03-30（FILE-020/025/026 既有目标字节窗口子断言）：全文校验释放读取句柄后，最终替换
  仍允许普通写入者改动目标；W34 两项首次 FAIL 均返回成功并覆盖并发内容，日志保留。
  Windows 现通过拒绝写共享的目标句柄完成最后字节核对，并持有至原生替换返回；拒绝最终
  reparse/非普通对象，沿用 ReplaceFileW 和既有备份/unknown 恢复规则。52 项去重定向通过，
  3 项新增回归各 20/20，含同长度变更；另覆盖旧读句柄、写/删共享拒绝、ACL/命名流和恢复。
  暂存源、目标名称置换、映射写入、Unix、正式 UI/完整角色及长期门槛仍开放；未关闭整条 Case。

- S-D03-29（FILE-018、A08/A09/A17 文件候选失败子断言）：引用菜单把请求失败显示成零匹配，
  加载时仍可选择旧候选。W33 两项组件首次 FAIL 和正式 Tauri 的 409→空结果首次 UI FAIL
  保留。现显示中英错误及重试，加载/失败时不提供旧候选；请求按会话、工作区和尝试隔离，
  关闭/卸载或迟到响应不能回填。12 项定向及各 20/20、类型/翻译/桌面边界和 Tauri 构建通过；
  新隔离 Tauri 验证错误、修复夹具后重试、保持查询及继续输入，前后共 2 模型步、0 effects。
  完整权限/网络错误分类、实际多会话竞态 UI、全部角色/平台与长期门槛仍待验，共享阶段未完成。

- S-D03-28（FILE-016/018、AUTH-010 扁平清单/规则边界子断言）：旧遍历器导入根外父目录和
  Git 排除规则，跳过坏规则后仍成功，根置换还泄漏根外名称。W32 四项首次 FAIL 及正式 Tauri
  `@visible` 被父规则误滤的首次 UI FAIL 保留。清单现复用受限遍历，保留隐藏文件及根内规则
  优先级；gitdir/commondir 仅解析根内授权目标，链接项跳过，遍历/规则/预算失败不发布缓存。
  41 项定向通过，清单相关 20 项各 20/20，两万文件精确边界及超限各 20/20；正式 Tauri
  文件候选恢复、根内忽略、根外 junction 跳过三项通过。前后共 2 模型步、0 effects。
  文件候选的失败可见状态、Unix 竞态、跨重启根身份、全部角色与长期门槛仍待验；根外 Git
  元数据须由明确资源授权承载，不从工作区链接推导额外权限，共享阶段仍未完成。

- S-D03-27（AUTH-010、FILE-018 侧栏目录边界子断言）：单层目录接口以链接的根内名称放行
  根外目标，校验后还用普通路径重新打开目录。W31 直接 junction 及目标/祖先/根替换四项
  原生首次 FAIL、正式 Tauri 展示根外文件名的首次 UI FAIL 均保留。现要求 canonical 目标
  留在工作区，复用受限目录读取；普通项取枚举类型，根内链接仍可展开，根外/私有/断链保留
  为不可展开项，枚举失败不再当作完整快照。15 项定向通过，侧栏 13 项各 20/20；正式 Tauri
  的根外链接、根内链接及缓存目录转向后 403/告警三项通过。前后共 2 模型步、0 effects。
  Unix 竞态、接纳前/跨重启根身份、权限错误完整分类、扁平遍历、其他入口/角色及长期门槛仍待验。

- S-D03-26（FILE-018/040、A08/A17 读取失败可见性子断言）：文件树读取失败被返回为空数组，
  UI 只留控制台错误，旧列表没有过期提示。W30 两项组件首次 FAIL 和正式 Tauri 首次 UI FAIL
  保留。现以 null 表示无可用结果，保留失败状态并展示告警/重试；重试保留原搜索条件、重读
  失败分支，其他目录成功及较旧请求不能清掉新错误。初版同步重试残留错误及类型错误另留痕。
  24 项定向、9 项各 20 次、类型/翻译/桌面边界与 Tauri 构建通过；新隔离 Tauri 的旧快照
  告警、无快照失败、恢复后重试三项子断言通过。两次准备共 2 模型步、0 effects。
  原生权限/网络错误完整 UI 矩阵、其他角色与平台、100 seed 和长期门槛仍待验，共享阶段未完成。

- S-D03-25（FILE-040、PORT-012 消费者重连子断言）：共享传输层已发送重连/积压恢复信号，
  会话文件树却未订阅。W29 首次组件 FAIL 与正式 Tauri 在真实断线后保留旧文件树的首次 UI
  FAIL 均保留。接入既有 conversation.reconnected，复用当前工作区的有界合并与卸载清理。
  22 项定向、3 项各 20 次、typecheck、桌面边界与 Tauri 构建通过；另一个隔离 Tauri 在网络
  子进程中断后自动重连、重读并与磁盘一致，主应用/浏览器/renderer 保持不变。两次准备共
  2 模型步、0 effects。真实服务端积压、连接内 native rescan、乱序/重复矩阵、完整角色、
  其他平台与长期门槛仍待验，不计完整 FILE-040/PORT-012 或共享阶段完成。

- S-D03-24（FILE-040、PORT-012 / A17 文件树重读子断言）：成功的空目录响应被 UI 忽略，根
  刷新又直接复用已加载子目录的旧节点。W28 三项组件首次 FAIL，以及正式 Tauri 中根/子目录
  已空但刷新后仍显示旧文件的两项首次 FAIL 均保留。现接受空结果，重读已加载分支，旧请求与
  卸载后的响应不得回填；不扩大读取到未加载目录。13 项定向检查、9 项各 20 次、typecheck、
  桌面边界及 Tauri 构建通过；新隔离 Tauri 的根清空、子目录清空、空目录再新增三项手动对账
  通过。两次正式会话准备共 2 模型步、0 effects。自动丢批/重连触发、失败重读的可见诊断、
  100 seed、完整角色、其他平台和长期门槛仍待验，不计完整 Case 或共享阶段完成。

- S-D03-23（FILE-040 原生监听与清单恢复、A13/A17 子断言）：真实 Office 创建/文件删除通知
  送达期间，清单 API 仍返回旧缓存；首次监听和重订还复用订阅空窗前的旧结果。W27 三项首次
  FAIL 保留。App 现将监听接到实际文件路由的 cache owner，回调在事件过滤/debounce/发送前
  失效相交目录清单，原生错误/rescan 和订阅成功同样失效；弱引用不延长 owner 生命周期。
  Windows watcher/缓存 39、原生/路由接口 19、App 监听上下文 12 项通过，新增 6 项各重复 20 次。
  未覆盖未监听期间的主动刷新、完整 UI 丢批对账、迟到回调/尾事件、扁平遍历边界、
  其他平台和长期门槛，不计完整 FILE-040 或共享阶段完成。

- S-D03-22（FILE-040、PORT-012 原生事件缺口子断言）：native rescan 标记被丢弃，监听错误又
  将未知损失量记为 1。W26 两项首次失败保留；新增可选 rescan_required，未知缺口与可计数的
  本地丢弃分开，空事件也交付对账提示。按路径分量严格编码，不能表示的名称计 dropped，避免
  有损转换指向另一文件。旧四字段批次仍可读取，普通输出保持原编码，新字段经发布 Schema 校验。
  Windows 16 项定向通过，新增 6 项各重复 20 次；生成检查另发现此前 Cargo.lock 摘要漂移，
  保留失败并由正式生成器刷新三个摘要文件后通过。无模型/UI；重订、传输/消费者全量对账、其他
  平台与长期验收仍待验，不计完整 FILE-040/PORT-012 或共享阶段完成。

- S-D03-21（FILE-040、PORT-012 队列子断言）：单个不可表示路径会使已排队的正常事件也被
  整批丢弃且不留计数；交付后 debounce 仍会吞掉同路径的新变化。W25 两项首次失败保留。现按
  原有路径合同入队，拒绝项单独计 dropped，正常事件保留；交付时清合并状态，批内仍可合并。
  Windows 队列/原生监听 7 项、事件合同 1 项通过，7 项各重复 20 次；编译器一次访问违规崩溃
  另保留，同源码重试后通过。未覆盖 native rescan 信号、完整传输/消费者恢复、UI 与长期统计。

- S-D03-20（FILE-040 watcher 事件隔离支撑、AUTH-005 子断言）：Office watcher 共用全局
  debounce，同一物理文件在重叠工作区只通知其中一个；停止重订也继承旧抑制状态。W24 两项
  首次失败保留；debounce 现归各 workspace 注册所有，最后 owner 停止时清理。Windows 41 项
  通过，新增 3 项各重复 20 次；真实双 watcher 分别收到正确 workspace，停一留一继续工作。
  活跃订阅容量/抑制尾事件、缓存接线、批次队列和真实 UI 仍待验。

- S-D03-19（FILE-040 watcher 生命周期支撑、AUTH-005/A13 所有者与清理子断言）：停止监听时
  重新解析已删除文件/工作区，找不到原 canonical key，却返回成功并留下登记。W23 两项首次
  失败保留；现保存按 owner 隔离的请求别名，停止用原目标，改指向的活动别名先要求停止原订阅。
  原生撤销仅成功或 WatchNotFound 才清登记，未知错误保留并报告。Windows 38 项通过，新增
  4 项各重复 20 次，删除重建后的真实事件恢复；不代表缓存接线、队列/落后 UI 或完整 FILE-040 通过。

- S-D03-18（FILE-040 的失效/重读、并发与取消子断言）：已失效的旧扫描会重建旧缓存，甚至覆盖
  较新扫描；写/删事件又早于失效发出，订阅者拿到旧列表。W22 三项首次 owner 失败保留。缓存
  条目身份现在控制发布资格，失效撤销旧扫描；最多一次有依据的重读，持续变化返回冲突，取消/失败
  释放未发布条目。变更事件先失效再发送，保持不同根隔离。Windows 33 项通过，新增 6 项各重复
  20 次通过；未覆盖底层清单遍历竞态、外部 watcher 接线、事件洪泛/落后 UI 与完整生命周期统计。

- S-D03-17（AUTH-009/010、FILE-016/018 目录读子断言）：两级目录树在校验后、子目录预取前换链
  均返回根外名称；子目录拒绝列举还被表示为空。W21 三项首次 owner 失败保留。树及子项分类现
  复用受限目录游标，无法完成预取则明确失败，保留真实授权根而非把显示根当作权限。Windows
  17 项定向检查通过，新增 5 项各重复 20 次通过；合法其他授权根、隐藏项、根内链接入口与排序
  保留。扁平清单/缓存、独立侧栏入口、其他平台/文件系统、正式 UI 与统计验收仍待验。

- S-D03-16（AUTH-009/010、FILE-011/018 元数据子断言）：元数据和非递归指令范围仍在校验后按
  路径 stat；短暂父目录换链分别返回根外大小、把根内文件报告为目录。W20 两项首次 owner 失败
  保留；两入口现复用受限元数据句柄，并仅把真实的缺失祖先/目标分类为 missing。Windows 108 项
  通过，新增 5 项各重复 20 次通过；父目录拒 RD/S、文件拒数据读的合法 metadata/子目录搜索通过，
  校准后的拒绝不伪装缺失。初版 ACL 夹具未形成原生拒绝及其恢复失败均保留，临时 ACL 已恢复。
  普通清单、既有文件发布窗口、其他平台/文件系统、完整 UI/角色及统计验收仍待验。

- S-D03-15（AUTH-009/010、FILE-013～018 子断言）：旧搜索 walker 即使关闭 parent matching，
  仍打开工作区上方的 ignore 文件；旧配置组件探针另确认子目录置换可提供根外规则。W19 保留
  owner 首次失败、组件反例及大小写兼容中间失败；根直接换链原已有 incomplete，其严格抛错
  夹具判定过严，已保留并纠正分类。搜索现按受限目录遍历，只在绑定根内探测仓库标记，规则通过
  有界 reader 读取并计入字节/解析预算；不能完整验证规则时停止该子树并明确 incomplete。
  Windows 109 项通过，新增 9 项各重复 20 次通过；保留 ignore 优先级/嵌套仓库/显式文件、隐藏
  属性、大小写敏感性与准确 offset/SHA。普通清单、更多权限/文件系统/Unix、真实 UI 和统计仍待验。

- S-D03-14（AUTH-009/010、FILE-011/012 的递归枚举子断言）：`instruction_scope` 校验后临时换链
  再恢复，工作区根及子目录两项均把 13 项根外元数据记为完整扫描。Windows W18 两项首次 owner
  失败及夹具 UUID 错误保留；枚举现沿受限根打开目录并持有原生只读游标，名称和类型来自同一
  目录对象，身份/边界冲突直接拒绝。原有隐藏/忽略目录发现、链接 incomplete 和预算语义保留。
  Windows 99 项通过，新增 5 项各重复 20 次通过；文件清单/搜索 walker、Unix、其他文件系统、
  100 seed、完整模型/UI/角色及长期验收仍待验。

- S-D03-13（AUTH-009/010、FILE-034～039 子断言）：校验后替换父目录可删除根外文件/目录，
  递归目标在 DELETE 检查后改名也会误删并发目录。Windows W17 四项首次失败保留；删除现沿
  已核对的根句柄逐段打开目标，递归使用有界原生目录游标及逐项 DELETE 句柄，不跟随嵌套链接。
  空目录不要求列举权限，部分失败保留 unknown/fence。Windows 99 项通过，新增 7 项各重复
  20 次通过；中间目录游标拒绝错误及修正证据保留。其他文件系统/Unix、IO 故障、100 seed、
  完整 UI/角色与长稳仍待验；此结论不覆盖普通文件枚举或既有文件发布。

- S-D03-12（FILE-034/035/036/037/038 子断言）：删除权限检查后仍按路径删除，最终文件名被普通
  或 POSIX 改名置换时会误删并发文件。Windows W16 两项首次失败保留；普通文件分支现持有拒绝
  删除共享且不跟随末端链接的 DELETE 句柄，以同一句柄删除，保留只读/ACL/共享拒绝及旧读者。
  Windows 77 项通过，新增 4 项各重复 20 次通过；打开前父目录/目标置换、递归删除及其他平台仍待验。

- S-D03-11（AUTH-009/010、FILE-018/025/026/031 子断言）：补丁预检与内容匹配各自直接按路径
  读取，父目录临时换链可接受根外同内容来源，读取中的同名置换也未拒绝。Windows W15 四项首次
  失败保留；预检、回滚和发布前匹配现复用受限、有界且核对文件身份的 reader，保留原 guard。
  Windows 95 项通过，新增 4 项另重复 20 次通过；最后一次读取到原生替换之间的竞态、Unix、
  目录枚举/删除、完整 UI/角色及统计门槛仍未关闭。

- S-D03-10（FILE-019/028/038/039 子断言）：新建发布在暂存后关闭写句柄，再按文件名 hard-link；
  普通改名、POSIX 替换和原地改写均可使其发布错误字节，三项首次失败保留于 Windows W14。
  Windows 新建分支现持有拒绝其他写入/删除共享的创建句柄，以同一句柄不覆盖重命名，失败按句柄
  清理；发布后错误仍保留 unknown。Windows 93 项定向检查通过，新增 4 项另重复 20 次通过。
  仅关闭新建分支的暂存源窗口；既有文件 ReplaceFile、Unix、100 seed、完整 UI/角色及长稳仍待验。

- S-D03-09（FILE-019/020/025/028/038/039、OBS-016 子断言）：失败临时文件和成功后的备份名
  被换成其他文件时，旧清理会误删；部分替换失败还会把其他文件当原件恢复。三个首次失败现场
  保留于 Windows W13。共享发布逻辑不再清理已经消耗的临时名；Windows 保留原对象的元数据
  句柄及卷/128 位文件 ID，清理、恢复先核对身份，再对该句柄操作，恢复不覆盖并发出现的目标。
  不匹配/清理失败保留 unknown 和原有 fence。Windows 95 项定向检查通过，日志见 W13；
  发布前最终文件/暂存文件置换、Unix 未消耗临时文件的清理竞态、正式 UI/其他平台仍待验。

- S-D03-08（AUTH-009/010、WIN-007、FILE-019/020/024/025/028 子断言）：父目录准备只返回路径，
  句柄在发布前释放；Windows 三种写入在目录换链后均修改根外文件。准备结果现持有经身份核对的
  目录句柄及按关闭删除的空保护文件，覆盖发布和清理；保留原子替换的 ACL/ADS 与失败结算。
  新 ACL 反例同时发现目录打开额外申请 SYNCHRONIZE，读取/写入路径解析改用原生元数据权限。
  Windows 97 项定向回归通过，首次和中间失败及原子权限夹具校正见 W12；最终文件/临时文件置换、枚举/删除、
  跨重启根身份、正式 UI 及其他平台仍待验，不计完整 Case 或全部文件竞态关闭。

- S-D03-07（AUTH-009/010、WIN-007、FILE-018 子断言）：路径校验后短暂置换父目录 junction，
  旧 reader 实际读到根外字节，恢复路径后仍返回成功；同名不同文件替换也未被识别。Windows
  内容读取改为相对受限目录句柄逐段不跟随新链接，并核对句柄的实际路径及读前/读后文件身份。
  文本/二进制、搜索和补丁读取复用此边界，旧 Agent 全文入口也统一到 8 MiB 合同。Windows
  81 项通过，首次失败和原生夹具见 W11。目录枚举、写/删竞态、跨重启根身份及其他平台仍待验。

- S-D03-06（FILE-032/039、OBS-016、LIFE-006/007 子断言）：Artifact 未知结果判断扫描完整错误
  文本，普通目录名/临时文件冲突中的相同字样也触发 unknown，补丁错误因此无法正常结算。
  改为只识别 Artifact owner 的 Conflict 错误及明确前缀；原始文件和不属于本次操作的临时文件
  保留，已知失败结算 rejected，重开 host 后新操作可执行；真实 post-link 置换/回滚不确定仍
  为 unknown。Windows 50 项定向检查通过，首次失败见 W10；无模型调用、无完整 UI/macOS 结论。

- S-D01-01（REG-005/008、整批预检）：Windows W07 后两次真实 GEN 中，一次 8 步零错误，
  另一次在第 2/5 步将 ToolSearch 与 write/delete 混批，产生 4 个拒绝结果，随后恢复。
  实际工具说明已有互斥规则。W09 补可选的并行调用偏好，仅 ToolSearch 与其他工具同时暴露时
  请求单调用；映射到 OpenAI/Responses 与 Anthropic 系协议，原整批拒绝、授权和普通只读并行
  保留。Windows 199 项定向检查及同构建 GEN 8/9/8 步三次真实场景通过；25 次 HTTP 200，
  实际请求均带 false，每模型步一调用，文件/回读/答复完整。旧 FAIL 不改写。
  **该 GEN 场景 N3 通过**；Gemini 当前无对应请求字段，其他模型/角色/macOS 与长期统计仍待验。

- S-D01-02（PROC-001/003、CTRL-006/007、A02/A17/A19 模型数组参数形状子断言）：W72 的
  `report_completion.requirement_ids` 与 W74 的 `exec_command.args` 均曾把数组编码为 JSON 字符串；
  前置 Schema 正确拒绝且零 dispatch，但合法任务出现可见错误。W75 两项首次说明回归确认字段虽为
  `type=array`，模型说明没有明示“实际 JSON 数组、不得传字符串”。现只增强 process `args` 以及
  completion 的 `criteria`/`requirement_ids`/`evidence_call_ids`/`evidence_paths` 模型可见说明，
  保留严格类型校验、整批原子拒绝与零字符串强转。两项精确回归各 **20/20**，Agent Runtime
  **175/175**。正式 Tauri `668062e479e6…` 中 step-3.7-flash 首次形成真实 `args` 数组，唯一进程
  退出 0 且 `reaped=true`，UI 零异常，正确报告 300、`shape-0001.txt`、`shape-0300.txt`；单回合
  2 个模型步、67 条事件。第二个隔离样本按要求执行两条命令后，`report_completion.criteria` 和两组
  `evidence_call_ids` 首次均为真实数组，UI 同样零异常，3 个模型步、121 条事件；12 文件摘要不变，
  两次均应用/profile 清零并正式备份。但 alpha criterion 错把 beta call ID 作为证据，因此只通过
  数组形状子断言，精确 evidence 语义关联另转后续 CTRL 批次。其他数组字段/Provider、N3/100 seed/
  LONG/99% 仍开放，不关闭完整 REG/CTRL/PROC 或共享阶段。

- S-D01-03（LONG-008、ACOD-017、A05/A09/A15/A17/A19 压缩夹具子断言）：W92 额外
  `native_coding_reliability` 长度压力回归为 **1/2**；W95 在当前基线原样复现。产品发出合法的
  no-tools compaction 请求，scripted provider 却要求每个请求都有原生工具并 panic，重试后将
  Execution 错误暂停为 provider unavailable。夹具现按压缩专用提示和空工具面区分请求，用独立
  task/compaction 计数响应有界摘要；压缩后原始 tool message 已移出当前窗口时，要求
  `available_evidence` 仍含当前 `gomoku/index.html` 路径，磁盘、effect、完成和未验证范围断言均保留。
  最终一次请求轨迹精确为 3 个任务请求 + 1 个 canonical compaction，请求数与事件数相等，写入只
  执行一次且 Turn completed；完整文件 **2/2**，原失败 Case **20/20**。生产 Runtime/Provider
  编码无需修改。本批只修验证夹具并证明一次压缩；20 次连续压缩、真实 Provider/UI、pause/restart、
  其他 Agent/平台及 LONG 统计仍开放，不关闭完整 LONG-008/ACOD-017 或共享阶段。

- S-D01-04（REG-005/008、LONG-008、AGEN-002/005、ACOD-017/019、A01/A05/A09/A15/A17/A19
  条件工具面子断言）：W98 用 `browser-use,computer-use` 构建运行 W95 已修的完整
  `native_coding_reliability`。Coding、General 原生伪调用纠错、General ToolSearch 动态发现及委派
  provider 暂停四项均通过；Browser/Computer 只绑定或发现，重型 Browser Runtime 启动计数保持 0。
  selected workspace、单次文件写、压缩后 evidence、未验证范围和暂停错误分类原断言全部保留。
  首轮 **4/4**（另 2 个显式真实 Provider 用例按设计 ignored），随后完整条件套件 **20/20**，共
  80 个非忽略测试执行零失败。生产代码无需修改。本批没有运行被忽略的真实 Provider、真实 Browser/
  Computer 动作或正式 UI；多次压缩、其他角色/平台及 LONG/99% 仍开放，不关闭完整 Case 或共享阶段。

- S-D01-05（MODEL-002/003/029/034/035、A01/A02/A09/A16/A17/A19 真实 Provider wire 子断言）：
  W99 从既有加密 StepFun 配置和已固定 Session 只读复制 route，先以正式 App 编码器把生产请求送入
  本地 capture server，再将同一请求分别以 streaming/non-streaming 方式直连 `step-3.7-flash`；工具面
  收窄到精确 `exec_command`，所有返回工具仅解析、不执行。两种请求均 HTTP 200、
  `finish_reason=tool_calls`，各恰好一个原生 `exec_command`；arguments 均为有效 JSON，文本 tool markup
  为 false、普通 content 为 0 bytes、实际执行工具数为 0。独立 18 项断言通过，streaming/non-streaming
  分别约 1,398/992 ms；凭据和原始模型正文未写入制品或日志。生产代码无需修改。本批仅一次真实
  Provider 样本，未保留脱敏原始 frame，也未走正式 UI/owner dispatch；其他协议/模型、N3/100 seed/
  failover 与 LONG/99% 仍开放，不关闭完整 MODEL Case 或共享阶段。

- S-D01-06（MODEL-002/003/029/034/035、A01/A02/A09/A16/A17/A19 精确 wire N3 子断言）：
  W100 在两个新的隔离 App/capture server 中重复 W99 的只读 StepFun probe，与 W99 合计三个独立样本。
  3 次 streaming 和 3 次 non-streaming 请求全部 HTTP 200、`finish_reason=tool_calls`，每次恰好一个
  原生 `exec_command`，arguments 均为有效 JSON，零文本伪调用、零普通 content、零工具执行；共 54 项
  独立断言通过。streaming 耗时 1,398/1,302/1,407 ms（中位 1,398），non-streaming 为
  992/1,587/1,712 ms（中位 1,587）。**该精确 route/model/function 的只读 wire 场景 N3 通过**；
  不外推到 owner 执行、其他工具/协议/模型、原始 frame、正式 UI 或完整 MODEL Case，共享阶段未完成。

- S-D01-07（REG-008、G0-027、PROC-001～032 进程工具 Schema 同构子断言）：W105 的 model
  `poll_process.cursor` 在 canonical Schema 缺失、W107 的两侧 PTY 上限又同时宽于实际 owner，说明原
  `full_surface` 只核对 Action 名称与 object 外壳，不能阻止两份 Schema 漂移。新增逐 Action 回归，
  从正式 Wave2 workspace registration 的 schema ref 解析 canonical exec/start/poll/input/close/
  resize/cancel，并与 7 个模型工具递归比较属性集合、必填字段、类型、union 及约束。模型侧只允许
  有意收窄数值范围，例如 start 的 wait_ms=0；缺字段、放宽 max/min 或结构变化直接失败。当前修复
  基线首次 **20/20**，Agent Runtime **196/196**。本批只补防漂移回归，无新产品失败、模型调用或
  正式 UI；文件/VCS/Artifact 及其他 Wave Schema、生成时跨 crate 单一来源仍待后续，不关闭完整 REG。

- S-D01-08（REG-008、G0-027、FILE-017/024/032、ART-004、VCS-011 workspace Schema 同构
  子断言）：W109 将 W108 守卫扩到全部 19 个 Wave2 workspace 工具，首次 **0/1** 于
  `read_file.path`：Runtime standard exposure 候选 Schema 缺 canonical `\\S`。
  继续走查还确认 write/patch/delete/diff/stage/publish 同类路径缺口、search 全空白 query、Artifact
  read 默认页长 65536/16384 分歧，以及 push 允许 4096 字符任意 refspec 和 `force=true`，canonical
  实际只允许 1024 字符的显式本地分支 refspec 且 force=false。现统一这些约束和默认值；patch 行由
  三个等价 oneOf 改用 canonical kind enum，保留 context/add/remove 与严格文本合同；空 required
  数组也显式一致。正式 App 当前在 Snapshot 编译时会以 canonical 替换该候选 Schema，因此没有
  已观察的 UI dispatch 失败；修复防止 Runtime 测试、未来宿主或其他直接集成重新暴露放宽合同。
  递归守卫允许候选侧增加安全收窄，但禁止缺 canonical 字段/约束或放宽范围。
  修复后完整 workspace 同构 **20/20**；同步远端 `d4dcae8a3` 后 Agent Runtime **197/197**，
  push/空白路径直接反例通过。
  本批无模型调用或正式 UI；其他 Wave、动态/MCP schema 及跨 crate 单一生成源仍待验，不关闭完整 REG。

- S-D02-06（OBS-008/014、LIFE-015/019 子断言）：原生暂停已持久化，前端却忽略暂停通知并
  持续转圈。现按通知重读 canonical 状态，停止活动显示、呈现公开原因并阻断新发送，保留原
  回合及队列所有权；结束回合复用 cancel/释放确认。Windows 99 项定向检查和原失败 Session
  的 Tauri 冷读/结束回合通过，0 新模型步，原事件/效果/快照与文件字节保留。热态有实际 hook
  回归，新模型故障的正式 UI 热态、resume 授权/预算及其他角色/macOS 待验；GEN 原失败仍保留。

- S-D02-05（CTRL-007、AGEN-014/017 子断言）：受保护的 Artifact 发布也误使源文件观察失效。
  owner 回执补绑定根摘要，Runtime 仅延续同根且确定未修改的文件/产物观察；不透明/失败效果
  保留失效边界。summary Schema 明确其为唯一最终答复；Windows 191 项定向检查通过，实际
  StepFun 发送记录确认当前证据及说明未被编码器丢弃。新 GEN 在第 6 步因供应商不可用暂停，
  已完成的 4 项文件效果正确，但 UI 仍转圈且刷新后未显示暂停原因，最终交付未验，仍 FAIL。
  该暂停投影缺口转 W04，原三次 GEN FAIL 保留；证据见 Windows W07，不计 macOS 或 N3 通过。

- S-D02-04（CTRL-007、WIN-005/007 子断言）：完成证据只比较模型参数路径，中文不相干改动会
  误使证据失效，参数中的不同名称也不能证明 junction 别名不重叠。文件 owner 现在返回根摘要及
  实际解析路径；Runtime 只用该观察保留确定不相交的文件证据，缺失/畸形/跨根、失败与不透明
  mutation 均保留失效边界。Windows 200 项定向检查通过，证据见 W07 owner-paths；
  Artifact 证据有效期及最终交付仍需继续修复，GEN 原三次 FAIL 不关闭，macOS 未验。

- S-D02-03（REAL-010、OBS-008/014）：真实命令已取消/reaped，但历史仅接受 Runtime ToolCompleted，
  漏掉取消后的宿主结算，UI 显示“已运行”且无输出。补相同 Turn/call 的有界宿主结算读取、原生
  已清理 process 取消语义、前端终态/标题和停止确认后的历史刷新。Windows 118 项定向检查及
  旧记录冷加载、新 Tauri COD 停止样本通过；历史 canonical 行/绑定/Snapshot 不变。首次 UI
  失败保留，其他角色/复杂清理/macOS 未验，证据见 Windows W04。
- S-D09-01（取消统计）：W04 两个真实样本各有 2 个 model_step_started，但 SDK 外层取消固定
  model_steps=0。W06 补失败反例并按 Turn 保留已记录进度，输出关闭后冻结，旧写入不计入新 Turn；
  准备/一模型步/两模型步取消及 cleanup/late writer 共 14 项通过。无新增付费调用，旧历史不回写；
  修复后的真实模型计数、强杀和跨重启统计仍待验，不计 N3 通过。

- S-D03-05（FILE-037/038、LIFE-006/007 子断言）：Windows 非空目录根拒删仍先删除子项；递归
  中途错误被结算普通 failed。新增原生 DELETE 权限预检；递归错误/任务异常作为删除结果未知，
  保留 pending fence 并清除旧文件列表缓存。Windows 原生 ACL、重启/同 key/新 key/其他写拒绝、
  诊断读与替换保留 DACL/命名流共 60 项定向回归通过。首次失败见 `windows/w03-acl/`；
  其他平台、完整 UI、进程强杀与目录/ACL 并发仍未验。

- S-D03-04（WIN-005、FILE-028/032）：Windows 尚不存在的大小写别名绕过字节路径去重，导致
  multi-file patch 首项已发布、后项才失败。准备期按原生父目录大小写规则逐段判重，拒绝别名及
  祖先冲突，保留 case-sensitive 目录中的合法不同文件。根消失/查询失败不向根外回退或猜测。
  Windows 非管理员原生夹具及定向 50 项通过，首次和中间失败见 `windows/w03-case-alias/`；
  目录置换、其他文件系统/Unicode 等价组、正式 UI 与 macOS 仍未验。

- S-D02-02（AMUL-001、OBS-008/014、MGMT-013 子断言）：旧 Canvas 失败历史硬编码 complete，
  无 pending 的已结算消息仍显示无效 retry。改为由 canonical Turn 补终态并保留消息 ID/公开
  错误；前端恢复 failed/stopped 不冒充完成，重试仅开放给未确认 pending 的末条失败消息。
  Windows 59 项定向测试及原旧会话 Tauri 冷加载/刷新通过；原 Snapshot/绑定、canonical 行及
  画布图未变，0 新模型步/效果。旧“spinner”实际复现为静态重试图标与空白错误卡片，历史失败
  保留；只关闭该展示/无效入口问题，其他恢复与 macOS 未验。证据见 Windows W04。
  macOS M04-04 以隔离等价 MM 失败样本证明 terminal `turn/failed`、Canvas `pendingTurn=null`；同一
  data 冷启动正式 Tauri 显示失败卡片且无 spinner/无 retry，Turn/events/effects 保持 1/30/0。冷读只
  一次性 reconcile canonical message IDs，不改 Canvas 图或 terminal history；原 Windows 数据未冒充复跑。

- S-D03-03（FILE-032/039、OBS-005/016）：W03 复现路径中的标识文本触发错误 unknown 分类，
  以及文件/其子文件同批创建时先发布再失败。前者改为识别 owner 错误前缀，后者准备期检查
  canonical 文件目标的重复/祖先关系；不削弱真实不确定发布的保留与核对。Windows 32 项回归
  通过，首次失败见 `windows/w03-file-errors/`；新建大小写别名和其他平台仍未验证。

- S-D02-01（CTRL-007、AGEN-014/017、OBS）：Windows 三个新 GEN 回合的副作用均完成，
  但都出现一次失效完成引用及后续恢复。提示增强未解决；随后把当前有效 path/call ID 编入
  Runtime 控制工具 Schema，复用实际暴露 Schema 的整批预检。补失效路径提示，保留旧证据 epoch、
  失效/缺失/失败引用拒绝和 Kernel 授权。Runtime 138 项通过；真实模型仍提交不允许的路径，
  最后一次被预检拦截，且最终摘要遗漏所需细节。**问题仍开放**，需继续核对传输与模型交付策略。
  不将守住拒绝边界或最终 recovered 当作正向体验 PASS。证据见 Windows W02。
- S-D07-01（AMUL-001）：画布规划上下文未携带已有名称，真实模型因此无法回答。现在仅携带
  相同 Canvas ID 对应的有界 title；未知时保留 null，节点选择/资源权限不变。context 6 项、
  TypeScript/桌面边界及 Windows 新 Session 同提示回归通过。旧 MM retry spinner 与该问题分开。

- S-D03-02（AUTH-009/010、FILE-036、WIN-007/017）：文件删除入口在 canonicalize 后才删除，
  根内 junction 会被替换成目标目录，空相对路径则指向整个工作区。改为先拒绝根删除并检查原始
  entry 的链接类型；递归删除普通父目录仍由原生 API 只移除内嵌链接。发布临时名不再拼接目标
  basename，避免合法的 255 字符文件名使临时名超限。Windows 首次 3 FAIL 及修复后证据见
  `phase-2-3/2026-09-27/windows/w01b-links-length/`；并发路径置换、macOS/正式入口仍未验。

- S-D03-01（FILE-019～021/038/039；WIN-006/014）：W01-B 的真实 handle 反例确认 Agent
  `write_file` 仍经 `std::fs::write` 原地截断，deny-delete 下错误返回成功；覆盖既有文件还误报
  `created=true`。改为与 patch 共用完整临时文件发布，明确创建/覆盖意图，保留字节上限与失败清理。
  发布或清理结果不确定时保留 durable pending fence，换 key/重启不能盲重放。
  Windows 用保留 ACL 的原生替换并先验证 write/delete 访问；旧文件备份只在成功后清理，部分
  原生失败仅向不存在的目标恢复，出现并发目标则保留原件并要求核对。原始及中间失败见
  `phase-2-3/2026-09-27/windows/w01b-atomic/`；本轮证据仅支持 Windows 组件子断言。

- 2026-09-26：PAL 身份与绑定、MM Skill 首发、进程启动、工具可见性、模板 Action 已有针对性修复；
  Windows 真实路径及其构建身份见 Windows 文档；旧失败证据仍在仓库外。
- 2026-09-27：远端 `8228b61c3` 已提供控制预检、首次计划说明、完成直接收尾、工具参数与模型截断修复。
  [已有公共合同报告](../../reviews/2026-09-26-agent-tool-contract-reliability.zh.md)作为历史证据导入，
  新增修复先检查是否已被该实现覆盖，避免重复或倒退。
- MM 旧失败会话的空白卡片/静态无效 retry 已由 Windows W04 原会话复现并修复；原失败不改记成功，
  M04 及其他实际运行中停止/恢复仍待验。
- 11 个条件能力候选等待对应产品/资源/授权前提，不为完成列表扩权。
- 本批共享 P0 已随 `85a079fc0` 提交并推送；Windows W01 已接续，见平台进度。

### W184 命令合同与业务结果展示（2026-10-01）

- Case/子断言：C01/C03/C06/C08；CMD-134/137/138、REAL-003/004、A05/A08/A14/A17/A19相关子断言。
- 公共根因：git_diff合法根路径`.`被文件路径解析器拒绝；改为当前workspace scope，仍保留repo子目录及越界保护。
  process schema补明cwd为相对目录。UI识别真实exited且清理已证明的原生退出码，业务非零不再统称异常；
  本地参数预检明确未执行。真实系统/信号/清理失败仍保留失败状态，原始输出、canonical错误及累计历史均保留。
- Windows根因：宿主继承PowerShell 7的PSModulePath，破坏Windows PowerShell 5.1的Get-FileHash加载；
  仅系统5.1启动重建默认模块环境，显式调用者覆盖仍保留。原生首败及正式入口小复验见Windows W184。
- 定向回归：UI 90/90；原生hash、Git子目录边界、完成历史及整批预检共6项Rust通过；类型、i18n、desktop边界、fmt通过。
  正式StepFun/GEN小复验N1：3模型步、零工具错误，真实Git/hash及磁盘oracle一致；综合A仍失败待修，不能结案共享链路。

### W185 CMD引号、错误进程引用及完成历史读取（2026-10-01）

- Case/子断言：C01/C05/C06/C07/C08；PROC-001/005/007、CMD-139、REAL-004/005相关子断言。
- 修复：系统cmd.exe的/c或/k后一个脚本文本按原生规则保留引号；普通程序仍用CRT argv规则。
  错误/外属process_id返回精确未执行控制的owner事实；不清理或替换原进程，不把无效引用当能力故障或unknown效果。
  Runtime保留失败计数、原句柄和工作区证据，UI明确“未找到本次执行的进程，操作未执行”。
- 新首败：含失败计数的已完成会话在下一轮、模型启动前被history错误拒绝；修复为共用完成文本构造，
  精确接受当前/旧版披露，摘要/计数被改写或披露缺失仍拒绝，历史不重写。
- 定向验证：原生3项、真实host原进程/跨scope保护1项、policy9项、history/turn相关回归均通过；UI91项通过。
  正式GEN/COD各一个实际执行样本通过、字节与磁盘一致，预期引用拒绝各1次仍记error；原会话冷读后可继续响应。
  连续COD样本重复检查及history组合预检失败，首败保留、不记N3；同步744ca440b后须另记复验，完整A/B/C未结案。

W185同步结算：源码062fc9f2e，正常合并09357314f为8ffc71ba5，保留744ca440b与其他贡献者修改。
合并后压缩/完成历史/失败恢复3项定向回归通过，i18n与desktop边界通过。
正式原COD会话再次调用两个工具，3模型步/1实际压缩，每项只调用一次，无额外失败；
与此前GEN、独立COD组成两个修复点的N3执行证据，各有一次预期引用拒绝仍保留is_error=true，磁盘字节一致。
该N3只覆盖CMD引号和进程引用拒绝；历史首败及重复调用均保留，完整A/B/C与共享命令链路未结案。

### W186 完整只读场景的路径、属性及Git格式（2026-10-01）

- Case/子断言：C01/C02/C03/C06/C08；CMD-134～138、REAL-003/004及A05/A08/A14/A17/A19。
- 首败：五项命令执行后，最终cwd从PowerShell默认表格的省略路径误推成父目录；Archive dotfile误称Windows隐藏项。
  新指引使用完整路径字符串及Hidden/System布尔字段；不从名称推断属性。原生长中文路径/真实Hidden对照已验。
- 另一个公共根因：Git diff回调的F/H标签被误加进实际patch，解析返回no patch found；仅上下文/增/删内容行加前缀。
  未暂存/已暂存patch解析及repo子目录边界通过，正式新输出与独立git diff一致。
- 验证：4项不同Rust定向检查、fmt及正式构建通过。GEN修后8模型步/1压缩、五项语义正确，Bun各1次、exit0/1。
  COD实际读搜/Git/测试均有结果，但报告流程31模型步后压缩预算失败，不能计完整A通过；首败、预检拒绝及轨迹均保留。
- 未闭合：完成报告的计划/引用流程、固定上下文与压缩预算、完整A N3及B/C；不因terminal或局部成功关闭共享阶段。

### W187 已结算非零结果的报告收尾（2026-10-01）

- Case/子断言：C06/C07/C08；CTRL-006～008、REAL-004及A08/A17/A18/A19相关子断言。
- 首败：无显式计划的任务在收到已退出、非零且reaped的命令结果后，真实报告被missing/stale plan拒绝，
  COD反复建计划、补查，W186最终触及压缩预算。最小回归修前失败，历史轨迹保留。
- 修复：记录该已知结束结果的临时门槛来源，允许无运行进程、无未决patch的空可选计划直接提交报告；
  新副作用仍须replan。未知效果、信号、清理失败、显式计划及上下文/恢复边界不获此例外。
  整批参数拒绝只废弃旧报告，保留已证明的门槛来源；精确错误计数、输入覆盖及当前证据校验不变。
  指引说明每项最多8个evidence_call_ids及合理拆项，Schema上限不变。
- 验证：7项相关Rust回归通过，fmt及正式构建通过；真实COD两轮9/7模型步、1/2压缩完成，
  第二轮报告首次提交即接受，指定Bun测试每轮各1次、exit0/1，磁盘及无关哨兵不变。
- 新失败仍保留：第二轮模型把cmd脚本写成PowerShell 5.1不支持的&&，修正后执行；dotfile仍被误述为隐藏项。
  终态完成不证明综合A通过，完整A N3及B/C、用户语言和首次命令选择仍待闭合；共享门槛未达。
- 证据：仓库外2026-10-01/windows/w187-completion-loop；两轮完整轨迹、原生首红、截图和磁盘oracle均留存。
- 同步86b0d2c40后，7项报告/保护回归及1项平台示例回归共8/8通过；不重标同步前的正式样本身份。

### W188 本机脚本形态与重复检查现场（2026-10-01）

- Case/子断言：C01/C02/C03/C06/C07/C08；CMD-134～138/147、REAL-004及A05/A08/A17/A19。
- 调整：工具说明先给本机shell和实际JSON示例，明确cmd字段使用PowerShell 5.1，Command Prompt使用command/args。
  目录名称和Windows隐藏属性分开说明；删去重复说明，执行器、权限及Schema不变。
- 验证：现有工具合同3项、原生cwd/隐藏属性1项通过；fmt及正式构建通过。正式COD五项结果14个机器断言全绿，
  6模型步/1压缩，两测试各一次exit0/1；一次报告参数修正保留。过程UI显示“命令已结束，退出码1”和“操作未执行”。
- 新首败：GEN前五项实际结果正确，但完成复核后把原任务当新请求；三次压缩、额外未启动检查及两个测试重复执行。
  正式停止入口结束为cancelled，20模型步，9文件hash不变、无无关哨兵。此样本FAIL，不用COD结果代替GEN/N3。
- 未闭合：连续/压缩后的完成复核与原输入身份、报告参数首发、用户语言及完整A/B/C；共享门槛未达。
  下一批以该GEN现场定位重复任务根因，不扩大业务矩阵或重复运行已通过的底层套件。
- 证据：仓库外2026-10-01/windows/w188-shell-guidance；两个Session分开归档，原始失败、停止及机器断言均保留。

### W189 完成复核阶段的任务重做（2026-10-01）

- Case/子断言：C06/C07/C08；CTRL-006～008、REAL-008及A01/A02/A05/A08/A17/A18/A19。
- 根因与首败：压缩可丢掉完成复核的普通消息；原输入随后被当作新任务。只保留提示仍重复，
  只指定tool_choice仍可提议update_plan重开任务。结构反例四次首红及真实29步/4压缩、19步/4压缩失败均保留。
- 修复：复核阶段进入必须保留的宿主指令；无运行进程/未决patch时仅暴露已有report_completion控制，
  偏离收尾的整批调用在dispatch前拒绝。Frozen ToolPlan/权限不变，新输入/观察清除旧复核；运行进程仍可poll/cancel。
  未暴露调用被整批拒绝时只废弃报告，保留已证明的门槛来源；未dispatch的拒绝不能重新暴露动作。
  精确失败计数、当前证据和恢复保护不放宽。
- 验证：11项不同定向回归通过，含真实压缩、活进程边界、忽略选择后重开计划/命令仍零dispatch、参数/上下文保护；
  fmt及正式构建通过。最终GEN走到复核，13模型步/1压缩，复核后只调用报告，四命令及两个指定测试均各一次exit0/1。
- 未闭合：最终报告对搜索/Git标unverified；文件criterion错引目录枚举调用，真实字节正确仍不能认定证据关联正确。
  只关闭本批重做的N1子断言，不记完整A/N3；完成证据关联、业务失败的用户语言及B/C继续保留。
- 证据：仓库外2026-10-01/windows/w189-completion-review-compaction；所有阶段首败、修后、DB、截图与不可覆盖oracle分开保存。

### W190 完成账本的调用范围信息（2026-10-01）

- Case/子断言：C02/C03/C06/C07/C08，CTRL-006/007及A05/A08/A17/A18/A19。
- 根因：账本只含call ID、退出码及少量路径，压缩后目录命令与文件命令难以区分；被排除的旧记录缺少说明。
  最小反例修前失败，W189错引及本批首红保留。
- 修复：增加原请求范围及有界owner元数据；只保留白名单参数，不复制env/stdin/文件或patch内容。
  每条参数1KiB、整体观察窗口沿用32KiB；旧记录说明最多8条/4KiB，显式记录省略量。未升级旧记录为当前证据。
- 验证：新增3项及计数/Artifact/压缩/收尾保护共8项定向回归通过，fmt及正式构建通过。
  正式GEN11步/2压缩，文件criterion引用真实读取/hash命令；两个Bun各一次exit0/1，没有重复执行，磁盘9文件不变。
- 新首败：Get-Content未声明UTF-8，实际中文输出及最终内容乱码；搜索脚本用2>$null隐藏stderr，范围完整性未证。
  Git仍披露unverified，最终语言仍含内部字段；完整A/N3及B/C不结案。下一批优先编码/搜索形态及展示说明。
- 证据：仓库外2026-10-01/windows/w190-completion-operation-scopes；相同五项正式任务、原始结果/截图/轨迹及独立oracle保留。

### W191 UTF-8读取及搜索错误可见性（2026-10-01）

- Case/子断言：C01/C02/C06/C08，CMD-135/136、FILE基础读搜及A05/A08/A15/A17/A19。
- 调整：优先使用read_file/search_files；明确PowerShell 5.1默认ANSI可能误解无BOM UTF-8文件，原生读取须按已知编码显式指定。
  搜索须核对截断/未完整原因，不用通配符代替递归范围，不隐藏错误制造无匹配。没有改系统默认编码或放宽结果判断。
- 验证：原生UTF-8/CRLF整字节、匹配、无匹配和缺失错误共1条回归通过；工具合同3项、fmt及正式构建通过。
  正式GEN8步/1压缩，读搜使用产品能力；43字节/4行/hash、中文匹配1/无匹配0及完整性字段正确，两测试各一次exit0/1。
- 限制：最终仍把已执行的读搜/Git按当前证据资格标为unverified，并显示内部字段和英文计数；完整A/N3与B/C未关闭。
  W190乱码/吞错首败不覆盖；本批只把编码/完整搜索的真实GEN子断言记N1，不声称全部体验通过。
- 证据：仓库外2026-10-01/windows/w191-utf8-read-search；原oracle不改，合并cwd/list与cmd形式Bun的独立操作断言另存。

### W192 完成统计的用户语言与退出分类（2026-10-01）

- Case/子断言：C06/C07/C08，REAL-004、CTRL-006/007及A08/A09/A17/A19。
- 修复：展示层仅识别完整已知Runtime统计尾段；按同Conversation/Turn的原生退出/清理事实分类普通非零结果。
  记录不足、计数不匹配、信号、清理或参数故障保留普通未成功统计；用户文本、代码示例、原canonical文本/计数不改。
  两种语言提供结果说明；模型summary/rationale避免把账本字段或业务非零计数描述为应用故障，JSON精确计数仍必填/校验。
- 验证：相关UI72项、计数/收尾Rust3项、类型、i18n、desktop边界及正式UI/Tauri构建通过。
  正式Tauri冷读W191原回合显示命令结束/exit1及1/1统计；旧事件逐字不变，不把冷读计新执行样本。
  新GEN小任务7步/3压缩，两个Bun仅实际执行一次exit0/1，清理证明；混合拒绝保留3/1统计及原详情。
- 未闭合：新任务有2次报告计数参数拒绝、2次额外检查提议被阻止；非零与拒绝未抹除、不记无错误/N3。
  原A读搜/Git的当前证据警告与内部字段仍在旧消息中；该历史不重写，完整A/B/C和共享门槛未达。
- 证据：仓库外2026-10-01/windows/w192-completion-outcome-ui；冷读、新执行和旧事件比对分开留存。
- 同步结算：源码6495729a6，正常合并5466de420为2f6c9442d，保留远端证据上限回归及macOS记录；合并后证据/计数3项通过。

### W193 固定结果计数的原生Schema提示（2026-10-01）

- Case/子断言：C03/C06/C07/C08，CTRL-006/007、REAL-004及A02/A08/A17/A19。
- 根因与调整：模型把预期非零误算成工具失败0；固定host计数补充单值enum/default，并说明普通非零结果也在总数内。
  const、条件必填及真实累计值均保留；default只是展示提示，缺字段仍拒绝，不自动补参数或改写原失败。
- 验证：新回归修前红/修后绿；计数、证据上限、偏离收尾保护共5/5通过，fmt及正式构建通过。
  正式GEN/GEN/COD三个实际样本，4/3/5模型步、1/1/0压缩，报告均首次接受且精确1/1；Bun每样本各一次exit0/1，清理证明。
- 限制：只关闭计数首发子断言N3。COD先额外列root/tests目录，不满足只执行指定命令，整体样本范围FAIL保留。
  参数提示不是所有模型的强制生成保证；A读搜/Git当前证据问题、语言细节及完整B/C仍待验，共享门槛未达。
- 证据：仓库外2026-10-01/windows/w193-fixed-outcome-counters；三样本分开保存，W192两次报告拒绝及追加提议不改记成功。
- 同步结算：源码f6d93ba00，正常合并4f13f7b2e为8980fdd19；合并后计数与压缩/硬限保护8项通过。
  三个正式样本保留合并前构建身份；不由这些样本代判新压缩实现的正式Windows验收。
