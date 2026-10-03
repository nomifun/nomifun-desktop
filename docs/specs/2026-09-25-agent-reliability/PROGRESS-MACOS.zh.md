# macOS 命令与会话可靠性进度

更新：2026-10-03。当前已由 macOS arm64 原生执行者接续；此前 Windows 结果仍只作共享历史引用。
规则见 [实施计划](IMPLEMENTATION-PLAN.zh.md)，公共根因引用 [共享进度](PROGRESS-SHARED.zh.md)。
按用户 2026-09-30 的明确目的，本轮收敛为简单系统命令、步骤衔接、过程状态和结果可信性，
与共享计划的 C01～C08、A/B/C 三组正式会话一致。停止按 M01～M06 穷举全产品余项。
旧 675 共享 + 72 macOS 专属、2,366 槽与五角色分配仅为历史口径，不再作本轮完成率或结束门槛；
范围外、复用、手测与未验分开，原 Case 定义、首败及修复证据不删除、不自动记 PASS。

## 2026-10-02 阶段性 P0 交付（用户选择方案 1）

用户明确选择先交付已修且已验证的 P0 产品机制，生成精度残余留待后续安排。
本机自动走查在本次交付后停止；不再新增付费复跑或扩展旧矩阵。此为阶段性收尾，
不宣称阶段二、三全范围、完整 A/B/C、N3 或发布认证通过，下方首败和原验收缺口保持。

| 已交付产品机制 | 源码提交 | 验证与限制 |
| --- | --- | --- |
| C06-15 历史命令结果与当前文件状态策略一致 | `ace29cdd0` | 3 项定向回归；正式 Tauri/StepFun 两命令实际结果完整交付 N1；不等于复杂 A 漏项全部解决 |
| C05-09 原生事件循环保留清理失败退出码 | `6f1aaa270` | 12 项定向退出/生命周期回归，正式冷读正常退出；失败分支修后未 live 再触发 |
| C05-10 未用 CEF 不初始化，首次使用仍走真实生命周期 | `a3a0bf001` | 4 项回归，正式未用冷读及首次 Browser 使用/清理；已用 CEF 的系统 Keychain 等待未修复 |

交付前核对当前源码与上述验证版本：C06-15/CEF 代码无差异，退出码路径保持；
文件原样写入/保留 LF 与恢复目标回归 4/4 仅证明产品保障，不替代生成精度。
证据分别在 `2026-10-02/macos/delivery-policy/`、`mac-gen-continuous/`、`cef-on-demand/`、
`file-generation-residual/run-001/`；完整日志/数据库/截图/轨迹仍在仓库外。

后续待用户安排：B09 缺末 LF/先修改后创建、复杂 A 结果漏项、已用 CEF Keychain 等待及
失败分支 live、旧 MM retry spinner、完整默认 GEN/N3/发布认证。保持未解决/未验状态；
不自动补字节、放宽断言、扩大权限或改写 Windows 结果。本次收尾仅文档，不重复构建/测试/模型。

## 剩余走查快速方案（2026-10-02，四簇执行与交付完成，残余保留）

本节响应用户后续要求，设计剩余项的高效走查/修复；不撤销已交付记录，不把方案当验证。
停止以长 A/B/C 整组、五角色或 N3 填槽推进，按下面四个问题簇执行。默认 GEN 覆盖并入 R2，
不单开第五套流程；统计重复、全生态、x86/release/soak 留在独立认证范围，未验状态保持。

| 顺序 | 目标 / 依据 | 先做的零付费定位 | 需要时的最小正式验证 / 独立判据 |
| --- | --- | --- | --- |
| R1 | 精确文件字节、创建/修改顺序；B09，FILE-019/025/029/030、REAL-016 | 复用三次 raw/canonical、B08 正链、4 项恢复/字节回归；核对冻结要求与实际展示合同，找具体矛盾，不再排查已排除的 codec 裁字节 | 只做“中文空格文件：两行含末 LF → 修改第二行 → 回读 → 报告”；独立比对前后完整 bytes/hash、首次操作顺序、原件、工具错误、UI/canonical。文件内容/hash准确，不能靠纠错后正确冒充首发通过 |
| R2 | 多项结果漏交付；A13，C06-15、OBS-020、REAL-021/024 | 从 A13 最后请求到原始 report 找具体丢失点；复用已验两命令结果，不重跑 Git/测试/搜索；检查冻结要求、证据资格、报告及 UI 投影 | 只做“cwd、含隐藏项的名称、原文件头尾”三项结果交付；各自真实值在同次报告/UI存在、调用不重做。用正式默认 GEN 入口承载；资源不满足只阻断该入口，不减模块后冒充默认 GEN |
| R3 | 旧 MM retry/spinner；M04-04，AMUL-001、OBS-008/014/020 | 复用旧失败数据和 pending/terminal 两种既有夹具，先核对 controller/panel 既有 retry 回归；旧 terminal 无按钮是入口状态断言，不是点击重试已验 | 正式 Tauri：可重试的提交未确认失败消息 → 实际点 retry → terminal → 同 data 冷读；同时旧 settled failure 不开无效 retry。记录按钮/请求/operation/Canvas pending 与错误卡片、事件增量，禁止用新 Session 首发成功关闭旧问题 |
| R4 | 已用 CEF 退出等待、失败退出分支；C05-09/10，A11/A13/A17、OBS-019 | 直接分析旧主线程/Keychain 样本、现有 shutdown/退出映射回归；不重复未用 CEF 正链，不更改钥匙串/系统保护 | 修复成立才跑一次实际 Browser 使用后关闭，采样自有 PID、真实 shutdown ack/退出码、后代及 listener。故障注入若仅覆盖协调器，明确不算已用 CEF/系统 Keychain live 证明；无法重现不记 PASS |

### 固定工作循环与停止线

1. 每簇先用已有证据定位，时间盒 30～45 分钟（是排查预算，不保证修复耗时）。输出一个
   可证伪根因或明确缺少的事实；到点不扩到相邻业务、不新造全套夹具，转下一可执行簇。
2. 发现产品缺陷：先做最小失败断言，再修源码，运行该函数/文件直接相关回归；不要整库测试。
   R1/R2 若仅是原始生成提议不合格，不把归因当解决：允许一个有明确机制假设的生成策略候选，
   先检验不扩权、不加表单负担、不丢要求/证据，再一次验证；无候选或无改善则保留残余并转项。
   禁止自动补 LF、猜 argv、放松 guard/恢复门槛，或连续纯文案微调/付费碰运气。
3. R1/R2 共享修复稳定后产出一个正式 Tauri 验证构建，两项用同包不同 Session/run；
   R3/R4 优先零付费 provider fixture。复用签名、隔离启动、owned deadline、闭库和审计脚本，
   不复制完整 runner。期间另有修复则明确新 build digest，只补受影响项。
4. 付费任务上限先冻结：R1 最多 8、R2 最多 6 个 provider requests（含摘要/纠正）；
   每任务 output_limit=4096、任务 180 秒、native 300 秒 + 5 秒清理宽限。不是永久零额度；
   所有尝试合计最多 14 requests，不自动扩额或重复任务。R3 若实际 provider 成为直接必要，
   另外先冻结最多 2 requests；总上限 16。预算到达/观察者超时都是未完成，不算通过。
5. UI 发送前一次预检模型路由/资源/桌面解锁与预算，记录 OS/架构/APFS 卷属性和必要环境；使用本机已有加密 StepFun 配置，
   不从聊天明文复制 key。正式终态截图/AX 与 Cmd-Q 放同一短控制段，保留 >=15 秒关闭余量；
   native handle 未终止前不重启任务或读 immutable DB，禁止访问已关闭 App 造成默认数据重启。
6. 每簇只记录“首败、修后、限制、提交”；修后报错立即封存，下一动作必须针对新发现的根因。
   没有新根因不重跑；正常/参数拒绝/业务非零/unknown/取消分开，A01～A20 底线全部保持。
7. 每批检查 staged、同步远端、配置身份正常 commit+push；证据在仓库外按日期/批次/run 保存。
   UI 改动才补桌面边界检查；仅计划文档不构建、不调用模型。共享根因同步共享进度，Windows
   结果与排程不改。出现一个簇的真实外部阻断继续其余簇，不反复等待同一阻断。

### 本轮交付口径

四簇各只有“修复并验证 / 原机制复用 / 仍开放且责任明确 / 资源阻断 / 独立认证待安排”之一。
不因缩短测试任务关闭原长 Case；短任务只证明其直接子断言。优先完成 R1/R2 核心命令体验，
再关 R3 的真实重试路径和 R4 的实际关闭问题；若有残余，明确列出而非宣布全范围完成。
完整默认 GEN 只能由实际默认绑定样本证明；N3/20 repeats/发布认证不是这轮快速收尾的隐藏门槛。

### 四簇最终交付状态

| 簇 | 本轮处置 | 明确保留的限制 |
| --- | --- | --- |
| R1 字节/步骤 | 当前正式文件短链N1，精确创建/局部改/回读首次正确；无有据的新执行层改动 | B09原复杂生成失败未改PASS，生成稳定性/N3未认证 |
| R2 结果交付 | 完整默认GEN未减权配置、三项实际结果正式N1；修复验证入口缺资源选择 | A13复杂漏项、全GEN其他业务/N3仍未关闭 |
| R3 MM retry/pause | 产品pause链路修复、真实失败消息retry/同key核对/冷读negative N1已交付 | 隔离等价数据，不冒充原Windows历史或全MM/N3；首次观察超时保留FAIL |
| R4 退出清理 | 原真实已用CEF证据复用；后续重开已补正式native确认丢失失败码/不重入N1，见下节 | 系统Keychain等待仍OPEN，真实物理CEF失败/阻塞未修后重现；不记完整风险PASS |

本表是最新快速方案的执行/交付结果，不是所有实际体验问题或原阶段二、三全范围达标。
本轮实际StepFun请求共9；R3/R4付费0，未扩大权限/关闭保护/放宽断言，完整证据在仓库外。
后续仅在新根因、受影响源码或明确新风险出现时重开对应簇，不恢复长A/B/C或旧全量矩阵。

### 当前重开目标：A13/B09 优先闭环（2026-10-02，实施中）

用户明确要求完成复杂结果交付、字节/步骤、真实Keychain清理和公开报告质量，不能以短链或
残余归类替代完成。付费调用次数上限已被用户明确取消；旧16次本地cap耗尽现场仍保留，
产品上下文/执行窗口、进程期限及清理保护不改。本目标仍活动；A13原完整任务本次N1见下，
B09与真实Keychain物理失败未记PASS，N3/统计稳定性保持未认证。

- B09原实际HTTP确认：被拒绝的reasoning-only假写入被回放为普通assistant content；最小
  反例首红后修，不再回放该载体，不解析/执行私密参数。私密名字收窄提示在真实混批后导致
  剩余工具误判缺失，已撤回；仅要求原生格式，原冻结操作面保持，错误修正/交付复核边界不变。
  兼容Chat编码器也不将有效轮私密思考转换为普通正文；无签名思考不计事实历史/后续预算，
  签名/加密续接、真实调用参数和回执不变。Broker18+35、Runtime相关首败/修后均外置。
- A13增加候选/最终两阶段：复杂编号任务有足够实际观察时，一次report-only交付复核；
  candidate不是CompletionReported/Delivered，不冻结终态输入、不允许新操作。冷恢复、
  流中纠正、超限/非法复核和中断不自动再试有回归。不是独立语义证明，仍需原任务验收。
- 本次三个正式原A任务均未通过，分别封存：run003本地cap16/无报告；run006冻结替换包络
  失败（不是StepFun限流）；run009普通进度被旧completion review过早关闭操作面、余项
  blocked。发现只读中文禁止句未识别、重复完成工具说明占据包络及过早收尾，新增首红后修。
  普通未完任务进度与已结算报告参数修正分开，后者保持禁止重做并保存checkpoint状态。
- 撤回私密名字推断后Runtime268/268、Broker18+35、journal6/6、显式无限调用夹具8/8；
  只证明机制边界。run012保留私密提示收窄导致混批拒绝/虚假缺工具首败；run015撤回后
  原A实际操作到达completed，11请求/9步骤、候选和最终报告各1，真实UI/正常退出/五表
  闭库通过，但严格原验收仍FAIL：最终未交付cwd和头尾原文，单次自检不足以关闭语义漏项。
  原oracle另报grep代专用搜索、git argv的`--`差异，原产物保留、不改其FAIL。
  证据`2026-10-02/macos/complex-delivery-repair/`；
  原任务、原件、字节/顺序/真假结果断言未放宽，旧失败及每次新结果分别保留。

#### 原复杂任务修后结果（2026-10-02，run023/024）

- 单次自检未解决run015漏项，替为编号交付槽及显式受限结果引用；宿主交付已保存真实值，
  不由模型重抄、不重跑、不自动公开所有日志、不赋予历史证据新鲜性。缺槽/无结果/伪造data
  拒绝；missing不能completed，later scope变更需精确后续输入引用。新交付不接受旧summary-only
  历史匹配；实际结构报告不追加候选复核。私密carrier/真实原生参数合同保持。
- **原完整A13本次N1 PASS**：正式Tauri/ad-hoc deep-strict、arm64/macOS26.6.2/25G83、APFS
  非大小写敏感Data卷；原任务13真实StepFun请求/9步骤/4压缩/540events，18原生参数全同，
  11必要进程各一次/reaped。最终正文与真实UI完整交付cwd/十名称/四行头尾和行数/正零搜索/
  Git字节/两指定测试0、1与失败断言。七原件/HEAD/status/diff不变，无哨兵/无额外文件。
  未成功6=四未执行Git提议+两真实非零，报告/UI/WorkStatus6/2；非首发全提议无错。
  candidate0/final1，正常Cmd-Q0/99.79秒，自有PID/listener消失，五表540/1/1/1/11全同且ok。
  旧accept的专用read/search工具名、Git argv、summary-only假设FAIL保留；独立验收按原正文
  和实际CompletionDelivered/UI检查，未放宽结果/保护。AX摘要截段不是真实UI遗漏。
- **B09 run016/024仍FAIL**：run016漏末LF、修改前提前复制并成功复制两次，末尾将shell串
  当command，后实际重启已完成helper；stop/Cmd-Q0/416.76秒不代判任务完成。run024最终32B/
  LF/hash正确、copy/move/delete各一次、原件不变、无helper重启；但首次直接创建after，跳过
  before33B→修改步骤，stdin含LF又append_newline=true，实物14B/双LF、stdout41B而非13/40。
  首helper真实EOF/exit0、长helper父子曾同PGID存活后均消失成立；29请求/21步骤/770events/
  九effects，最终强制上下文无摘要槽而失败、report0；不是付费次数cap/供应商限流。
  正常Cmd-Q0/198.03秒、五表全同；不以正确终版或已验helper机制抵扣错误步骤/输入。
- run016裸write参数JSON被错误接纳为摘要：现按原冻结工具schema拒绝完整匹配的动作参数，
  schema内部传递但不发送给摘要模型，禁止外部schema读取；正常JSON状态笔记允许。run024
  实际仅触发XML摘要拒绝及一次纠正，不冒充裸JSON live证据。Chat编码不再发送内部私密
  省略占位普通正文，本次public OutputTextDelta0，未复制占位但不等于有合格进度文案。
- Runtime270/270、Broker18及原35协议回归、正式App/fixture构建通过；最后另去掉交付目录
  重复精确正文/source文本投影，仅保留引用定位，原始值/宿主解析/错误计数不变。该最后
  轻量投影修复仅本机回归，尚未新formal B复验，未宣称已解除全部强制包络问题。
- 构建feature/外置runner metadata/日志路径、无fixture首次App SIGABRT、readonly DB14、
  oracle漏`./`首错全部在外另存，未发送任务的实例模型0；各结果不覆盖历史。证据
  `2026-10-02/macos/complex-delivery-repair/`。公开结果含内部元数据的呈现仍待简化；
  A13 N3/稳定性、B09生成步骤/字节及真实Keychain物理失败仍OPEN，Windows结果未改。

#### B09 计划前置与强制投影去重（2026-10-03）

- 去掉交付catalog重复argv，只留ref/tool/path；多编号summary说明改为简短结论+宿主精确
  附件，所有Schema断言/真实缓存/资格/错误计数不变。最后三read的五槽/13ref回归覆盖
  absence/32B/hash解析、不复制源正文及输出；没有靠扩大32768/4096默认包络洗绿。
- 明确编号任务首次write/patch/stdin前单独plan；无plan整批未派发，单任务/读观察/cleanup
  保持。已显式关闭计划的报告参数修正同样report-only，最小反例保持首次变更不派发及
  已结算效果不重做。plan仍是模型解释，不是用户意图/顺序或字节已正确的证明。
- 原B09正式Tauri新隔离run002：源`18d68c558`+外source.patch，本机arm64/macOS26.6.2/25G83、
  APFS非大小写敏感Data；11 StepFun请求/376events/五effects，任务/四原件保持。
  主动plan后before→guarded patch→cp→mv各一次；未触发plan门拒绝，不能冒充live拦错。
  首write仍无末LF32B，patch保持无LF而成31B，temp尚未删除就进入helper；实际exec bun
  字面argv一项、tty=true/timeout600000同步等待，只有READY\r\n，无stdin/EOF/长helper/report。
  明确合同失败后正式UI Stop+Cmd-Q0/249.47秒，非expiry/TERM/KILL；五表全同、ok、所有
  自有PID/listener已消失。**原B仍FAIL**，本次未到完成边界，不能代判包络live已解除。
- 真实摘要`tool_calls:[{name,arguments}]`（未知copy_file、无call_id）被接纳为状态；补既有
  root-only工具型摘要守卫，保留普通JSON状态、嵌套历史与空数组。未把假摘要当真实效果，
  也不归因它是后续PTY选择的唯一原因。此新守卫仅回归，尚未新live触发。
- 实际同步exec等不到后续模型输入，而工具说明未明确这一点；补exec同步等待、start先
  返回handle及READY/input/close/poll pipe链、tty=false为pipe/true为PTY。仅模型说明与
  既有字段白名单，不改owner/注册Schema/Session provenance或自动替换工具。最后这处
  合同补充仅定向回归，未再付费复跑；首LF/遗漏delete/完整报告仍需下一步处理。
- Runtime271/271及说明/摘要直接回归通过，正式构建/签名通过；日志仍在外
  `2026-10-03/macos/b09-plan-envelope/`。外置setup语法及跨根seal首拒保留，修夹具后
  冷五表376/1/1/1/5完整一致。未重复已验EOF/父子Stop正链，Windows结果未改。
- Keychain只读核对：Browser关闭之前未证明所有SSH/robot/storage已quiesce，CEF Helper也
  无精确退出库存；后台硬退不是可靠清理，原系统/物理风险仍OPEN，不改保护或记PASS。

#### B09 源绑定精确动作契约（2026-10-03，run002）

- 计划增加有限exact_actions：来源quote仅定位，不证明自然语言解释；保存参数digest/状态，
  不复制file/stdin正文/env或live handle到checkpoint。编号写/patch/stdin须先声明当前精确
  参数；整批owner前匹配，错中间状态/漏LF/双LF不归一化、不派发。stdin绑定新鲜process_id
  摘要，两种13B表达等价、另一owned目标拒绝；合法双LF声明仍合法，不改原工具字节语义。
- 先持久化reservation再admission，正向typed owner回执才推进；空/partial/lost对象不成功，
  未派发reservation与已派发未知在恢复中分开，真实回执断点恢复不重放。成功后新ID同源
  同payload不能绕once；后续用户新input可授权新动作。未完成契约不能非blocked收尾。
  最后将未知结果settled标志保持false，仅回归；正式样本旧字段仍保留，不改写历史。
- Runtime277/277与六个直接字节/零派发/once/恢复/正向回执回归通过；正式Tauri/签名及
  fixture构建通过。源`e2df86c08`+外tracked patch+exact_actions.rs（另存hash），arm64/
  macOS26.6.2/25G83、APFS非大小写敏感Data。原B task/seed不变，真实31请求/594events/
  八effects；首次两个缺LF提议0dispatch，假SHA占位与不支持cleanup契约声明拒绝保留。
- 实际创建before33B SHA f6a612…→guarded patch after32B SHA6ab0c427…，cp/mv/shasum/rm
  各一次、四原件不变。**原完整B仍FAIL**：步骤2没有内容回读便删临时；首helper虽已正确
  start pipe/wait0并观察READY，但未给timeout_ms，用原默认30秒。运行中stdin契约声明/
  五次付费压缩耗时约76.85秒，实际input距start90.69秒，回执EFFECT_OUTCOME_UNKNOWN、
  effect仍pending，实物仅ready。原13B参数正确不证明13B已送达，无EOF/长helper/最终报告。
- 暂停EXECUTION_CLEANUP_UNPROVEN/cleanup_proven=false，未知动作未复位/未重发。桌面途中
  锁定，正式UI收尾阻断；用户解锁到达时同实例已按原480+5期限TERM退出1、expired=true、
  forceKill=false（483.09秒）。失败专用冷封存五表594/1/1/1/8全同/ok，自有PID/listener
  消失；不是正常Quit，不以进程消失证明原任务/清理通过。完整失败在仓库外，不重启旧任务。
- 下一直接缺口是活动helper期间新增声明/压缩的成本与真实读/顺序覆盖；不自动延长寿命，
  不拿契约匹配当语义proof。A13旧N1不移植为新schema全认证，Keychain/公开元数据仍OPEN。
  证据`2026-10-03/macos/b09-exact-actions/`；测试夹具路径/未暴露计划首错均在外，
  Windows结果未代判、无权限/加密/保护/断言放宽。

#### B09 receiver_ref 预声明与定位反馈（2026-10-03）

- stdin可在启动前receiver_ref引用同计划前序start_process ID；仅真实匹配running回执
  绑定receiver digest，不改原生process_id/input/LF/TTL。未知/已结束start不绑定，晚引用
  拒绝，同ID重声明保留host绑定，cold关闭执行资格但历史结算可匹配；不复活句柄或重启once。
- 编号completion三处重复字段说明缩短，原types/required/enum/长度/计数/资格完全保持，
  结构等价回归通过；未改变frozen平台工具Schema。receiver及冷/错目标/重声明回归成立。
- 正式原B新隔离样本：源`29911b473`+外patch+exact_actions.rs，Tauri/签名/fixture通过，
  本机arm64/macOS26.6.2/25G83、APFS非大小写敏感Data；原task/四seed不变。9真实StepFun
  请求/6步骤/116events/0effects，连续update_plan声明拒绝→bounded no-progress失败。
  **整组仍FAIL**：虽原声明已有正确末LF和receiver_ref，但缺source、错误patch顶层path/
  hunks、cleanup/poll与PLACEHOLDER混入；反馈没有指出具体动作，后续修错处。没有file/
  helper/report效果，不能记receiver live采用、字节/完整交付已验或以零副作用作PASS。
- 正常正式UI终态+Cmd-Q0/102.47秒，无expiry/TERM/KILL；冷五表116/1/1/1/0全同/ok，
  自有App/fixture/listener消失，未遇锁屏。首声明与全部失败保存于外部
  `2026-10-03/macos/b09-receiver-ref/`，不重发同一任务或放宽4次控制停滞守卫。
- 该真实缺口修后：exact声明tool枚举明确支持范围；复用脱敏schema issue反馈，给动作
  索引/参数路径与schema自有expected字段，不回显文件/stdin/私密属性值。成功同源stdin
  换receiver别名不能新ID重入。Runtime279/279及定位/脱敏/别名负例通过，最后两处仅回归，
  尚未新formal采用；原Task语义/回读/EOF/长helper与报告保持OPEN。Windows结果未代判。

#### B09 声明定位正式采用与真实清理失败（2026-10-03）

- clean源`5de6798ec`正式Tauri/签名/fixture，原task/四seed不变；arm64/macOS26.6.2/25G83、
  APFS非大小写敏感Data。32 StepFun请求/24步骤/591events/六effects，定位反馈实际采用后
  plan接纳；但源引用合法不代表解释正确：首唯一write直接after32B、没有before33/patch。
  cp/mv的首shell串误放command拒绝保留，修cmd后各真实一次；真实read内容/hash后才delete，
  此段顺序成立。四原件保持，不能用正确最终32B覆盖缺失的原要求中间状态。
- 首helper正确start pipe/wait0且实际poll READY；没有预声明stdin，输入提议门前拒绝，
  运行中修plan与压缩后再次input时结果未知，实物只有ready，无input/EOF/长helper/report。
  暂停EXECUTION_CLEANUP_UNPROVEN，不重放未知输入。**原完整B仍FAIL**，receiver未正式绑定，
  不把参数13B或契约接纳改记实际已送达。
- 正式UI结束回合/正常Quit请求后native exit1/197.59秒，无expiry/TERM/KILL；真实失败原因
  Agent runtime shutdown未证明、原有限重试4次后非零退出，不是CEF/Keychain（未初始化）。
  正常封存exit0首断言失败保持，另失败封存五表591/1/1/1/6全同/ok，自有PID/listener
  消失；进程消失不证明cleanup通过，不用本样本关闭系统Keychain物理风险。
- 此现场固定包络仍高，新增最小去重：仅host mandatory workspace root/default-cwd规则
  存在时，将七个标准tool重复root suffix合一；无该context时原说明保持。所有schema/
  tool name/deferred字段严格相同，规则保留一处、重复应用幂等，serialized回归减少>1.5KiB。
  Runtime280/280通过，最后去重仅回归，未再formal重跑；没有提高模型/进程预算或吞错。
- 完整首败/修正/非零清理轨迹仅外部`2026-10-03/macos/b09-contract-feedback/`。没有新
  receiver/owner字节矩阵、未扩大权限/保护/断言；语义步骤、预声明采用、EOF/最终报告及
  真实Keychain和公开元数据保持OPEN，Windows结果不代判。

- 同样本的native exit1进一步定位下层resource cleanup：首次失败Shared result永久缓存，
  上层显式teardown重试只是读取旧Err。仅完成失败的同root/原tools settlement可新flight，
  pending/success仍共享；Session release已开始不另起Turn，最终Kernel release消耗后不重入。
  新flight仍向原owner要reap/settlement，panic/unknown标志、admission fence/quarantine不清。
  旧prior失败保留，当前真实settlement可独立取得新证明；不运行run_turn或重发stdin。
- 新缓存判定1项及既有cleanup_retry邻近9项通过；stage/session诊断不输出原始model/owner
  参数或凭据。模块/Display首编译错在外保持，修后结果另记。此修复仅本机定向，尚未修后
  native失败恢复；不以PID消失洗绿、不将Agent失败缓存原因当作Keychain唯一原因。

#### B09 后台超时终态与公开交付格式（2026-10-03）

- arm64/macOS26.6.2/25G83、Data APFS非大小写敏感。复用原69.5秒晚输入失败，不再
  重查执行层改字节。原owner冻结terminal只读查询不消费running输出、不改cursor、
  不续lease、不cancel；stdin/close/resize前已有确切reap则返回真实终态/输出、本次控制
  未执行且success=false。未证明cleanup/lookup失败仍unknown，不用PID消失作证明。
- Runtime仍摄取终态、移除已回收running标记、累计超时/控制失败；未输入控制不再无谓
  推进workspace epoch。exact动作仍未成功，不复位未知状态、不重发输入或启动。
- 真实/bin/sh pipe短期限及正常交互/字面argv等定向11/11；首观察断言误要求READY
  在晚控制重复交付失败保持，修后核对start+terminal完整输出、cursor0重放和真实reap。
  此为本机native runner，不是正式Tauri/StepFun修后B或Quit失败恢复live证明。
- 公开报告加宿主格式版本，旧缺字段记录仍旧文本精确回放，新版只匹配自身完整delivery。
  原文以literal块/可逆JSON保持；已知状态/字节/hash/行数/EOF/loss简短显示，中文摘要
  采用中文显示回退（非语言语义认证），0/false/未知/失败计数保持。已知输出不倾倒owner
  句柄/cursor/null；未知类型仍安全JSON，不过滤自由正文。Runtime283/283含格式、冷读、
  markup、未知和终态epoch负例；exact邻近说明明确initial→实际receipt→独立修改，
  不新增Case识别/自动补字节，指导不算语义闭环。本批零付费复跑，日志外置
  `2026-10-03/macos/late-input-terminal/`。
- 原完整B仍FAIL；A13新源码正式交付、helper预声明采用、Keychain物理等待/清理及自由
  语言质量仍待验证，不把本机回归或旧N1记新完整PASS。Windows结果未改。

#### B09 首次完整效果链与末端包络阻断（2026-10-03）

- clean`e22e329ca`正式Tauri和当前UI重建/签名验证；arm64/macOS26.6.2/25G83、Data APFS
  非大小写敏感；原task/四seed不变，StepFun次数不设cap，4096输出/360秒请求预算及
  原helper期限未变。33组native参数逐项等于canonical，46请求/33步骤/1020events/14effects。
- **本次实际核心链成立**：before33创建→独立patch after32，cp/mv各一次，真实read内容/hash
  后仅删临时；第一helper实际READY→13B一次input→EOF→完整40B stdout/exit0/reaped；
  第一退出后第二helper按原参数启动，双READY后cancel正确owner，143ms/reaped/errors[]。
  独立witness证明父子同PGID同时存活→均消失，heartbeat各43条。最后read终版32B/末LF/
  SHA6ab0c427…，临时/副本无、四原件全同。8次门前拒绝+2次实际test exit2均保留，不记首发无错。
- **原完整B仍FAIL**：report0，最后fresh read后的protected prefix连空summary wrapper都
  装不下，未发送下一个摘要便NOMIFUN_INTERNAL_ERROR；不是凭据/网络/付费次数cap。
  实际Cmd-Q exit0/332.15秒，无expiry/TERM/KILL；五表1020/1/1/1/14同源/ok、effects全
  returned，自有App/fixture/helper/listener关闭。未初始化CEF，不以此关闭Keychain物理风险。
- 直接修复仅去重说明：真实admitted workspace已有完整root/cwd/relative-path规则时，剥
  重复tool suffix但不再追加336B独立mandatory hint；部分/无context保持原说明，不删除或
  重索引adaptive slots。schema/原文/证据/预算不变，空wrapper跨界及短摘要可容纳回归通过，
  Runtime284/284。最后修复未正式复验，不把离线尺寸或核心链成功当完整交付PASS。
- 首prepare在ready前调用及普通readonly闭库CANTOPEN14保留，后按实际ready和确认无WAL/
  写者的immutable快照封存；没有重发任务。外部`2026-10-03/macos/b09-postfix-live/`含
  独立audit、完整HTTP/事件/数据库/UI。A13现格式正式受影响验证、B最终报告、native失败
  重试live与Keychain物理阻塞仍OPEN；Windows记录未改。

#### A13 最新公开格式与修正反馈冲突（2026-10-03）

- clean`97bbbd8dc`最新正式Tauri/当前UI/签名，复用e22e329ca未变的fixture setup/转发器
  （仅instrumentation，不代替产品版本）；原A全文/七原件不变，arm64/macOS26.6.2/25G83。
  StepFun6请求/5步骤/386events/五effects。**最新A仍FAIL**：尾两行从未读取，最终只交付
  目录、行数及两个测试四bundle；cwd/头部/两搜索/Git实值未正确交付，不以测试stack中的
  路径出现冒充cwd结果。Git槽missing使TASK_INCOMPLETE终态正确，summary“全部完成”不成立。
- `plain_zh_v1`确已实际CompletionDelivered及UI采用，宿主中文状态/退出码/loss/2次工具失败
  和1次命令失败统计成立、安全数据块保留；模型label/rationale仍有英文eligibility术语。
  两指定Bun各实际一次/0和1，失败断言保留；七原件/HEAD/status/diff不变，无额外文件/哨兵。
  正常Cmd-Q0/107.20秒，无expiry/TERM/KILL；五表386/1/1/1/5同源/ok。未用CEF不代判Keychain。
- 真实合同冲突：首report仅因criteria的stale IDs被拒，但旧修正反馈要求“旧结果放summary、
  omit evidence”，与新独立delivery结果发布相矛盾；下一请求仍合法的历史refs被模型删掉。
  现仅针对criteria现状证据字段修正，明确保留合法delivery_items.results，不因不新鲜将其
  删除或标missing；不恢复current资格、不自动重跑。例句随summary作中文显示回退、公开
  label要求用户语言。尾部start_line说明给实际L/N算式，未新增API/自动推断/预算权限。
- Runtime285/285，含历史delivery保留、current引用仍拒绝、原参数不改/私值不反射回归。
  最后修复及尾部指导未live，不以组件绿关闭原A漏步骤/交付。外证据
  `2026-10-03/macos/complex-report-closure/`，首次失败全保留，Windows结果未改。
- 原B普通Retry会新建Turn重发原输入，不能作为旧失败Turn续接；terminal不可checkpoint重开，
  resume_task也不继承旧once。只读结论不冒充正式恢复；B历史结果需通过新明确report-only
  范围及等价代理恢复后验证，不覆盖旧trace/seed/原失败，仍未执行。

#### A13 修后原完整任务 N1 闭环（2026-10-03）

- clean`688d251d4`正式Tauri/签名/原全文及七原件，fixture setup/转发器仍复用未变e22
  instrumentation，产品实际为最新build。StepFun8请求/7步骤/437events/五effects；16组
  原生完整参数=canonical。五必要进程各一次/reaped，两指定测试各一次/0和1。
- **最新原完整A N1 PASS**：实际pwd及十名称、真实head50B offset0/eof=false与tail47B
  offset50/eof=true完整覆盖97B、行数4、正/零搜索、Gitstatus/diff、两测试及失败断言，
  11选中bundle逐项等于owner并实际Delivery/UI完整呈现。报告/交付/完成各1、failed0。
  三次report预检拒绝+预期exit1共4/1保留；不以旧A、stack路径或hash替代实际cwd/tail。
- 七文件/HEAD/status/diff全同，无额外文件/哨兵。正常Cmd-Q0/190.257秒、noexpiry/signals；
  源/备份五表437/1/1/1/5全行同、双integrity ok、ready/completed、无pause/checkpoint。
  plain_zh_v1真实采用，labels/宿主中文、无eligibility/owner/null/private-thinking。五个
  JSON carrier逆解等于实际原文；search/VCS仍有英文字段fallback及Git rationale术语，
  所以公开语言质量未全闭环，N3/生成统计稳定性也未认证，不再追加A正向模型样本。
- 尝试原B同Session新report-only消息：不点击会重发原任务的Retry。等价旧loopback端点
  通过仅变换fixture本地认证的relay恢复，原data/work/seed/历史不改，未运行旧prepare；
  但真实Cua检测Mac锁定，任务未填/未发送、StepFun0请求。原480秒native期限到达后
  TERM、exit0/expired=true/noKILL，不记正常UI退出或CasePASS；relay/transport已关闭。
  闭库核对原1020events/14effects全行同、oldfailed未改、integrity ok。解锁请求已发给用户，
  解锁后才允许新隔离观察运行，不能把静态转发器或旧效果当B报告已补。
- Keychain只读架构核对：须Browser resource settle与native FFI分段，Robot目前abort无
  join/detached任务，CEF缺精确Helper注册/guardian；不能以名称/PID扫描或硬退宣称已清理。
  无新物理系统风险PASS。完整外证据`2026-10-03/macos/complex-report-closure-fixed/`，
  首败/未发送阻断保持，Windows结果不改。

#### B09 解锁后只补报告与历史入口 bootstrap（2026-10-03）

- 用户解锁后使用新观察实例但原Session/data/work不变，正式UI只发新report-only范围，
  不点击旧Retry，不重放旧任务；等价relay仅改fixture内部本地认证，转发body原样，
  真实StepFun仍由原Rust客户端转发/录制。3请求/3步骤/新增45events、零新effects。
  原1020events/14returned effects全行未变，旧failed保持；新Turn也failed、报告0。
- 三条实际SSE均finish length，末usage completion_tokens=4096（初usage=0不可冒充末值），
  reasoning_content13466/14225/16191B、public/tool均0；没有扩大4096预算或续写2次上限。
  实际Cmd-Q0/159.308秒/noexpiry/signals，闭库五表1065/1/1/2/14同源/ok，转发器关闭。
- 确定产品根因：宿主已有鉴权history_port，初始LOAD却被ToolHistory activation挡住；
  ToolSearch不能发现engine control，因此“先读旧回执再激活”循环。现有port即首轮曝光
  LOAD，成功LOAD/SEARCH/READ才激活ToolHistory；不激活TaskLedger、不调用owner、不把
  旧记录提为current证据。原single-call、context/agent floor、scope/binding/import检查保持。
- reasoning-only length使用专门无副作用续写说明，不再把纯报告错误引向HTML/文件分块；
  保持旧输出/步骤/续写上限，不复制思考内容、不改变模型thinking设置。Runtime288/288，
  三项bootstrap/noport/无owner及思考截断不回放/不涨预算回归通过；最后未正式验证，
  不宣称入口曝光就能跨build加载或B报告已恢复。日志仍外置complex-report-closure-fixed，
  公开语言及真实Keychain/物理native风险保持OPEN，Windows结果不代判。

#### B09 历史 loader 正式使用与不透明游标（2026-10-03）

- clean dc686a7dd正式同Session只补报告，精确标明最初原B operation而不重发操作。
  第一请求已有LOAD，实际LOAD1及SEARCH1，但LOAD把完整operation截为末UUID，端口拒绝；
  SEARCH又以operation作全文词，归档0/hits0，无READ。后3响应thinking-only length，
  末completion各4096、reasoning12190/15365/14914B、public/tool0，5请求/5步骤/63新events。
  **报告仍FAIL/0**；不能把调用LOAD或SEARCH成功当已加载原回执，binding import尚未进入。
- 旧1020及本轮前1065events全值前缀保持，14effects全值保持，无重放/新effect。中途Mac
  再锁、首次观察文件EEXIST保持；解锁后真实UI终态/正常Cmd-Q0/454.34秒、noexpiry/signals。
  五表1128/1/1/3/14同源/ok，原failed不改，App/fixture/relay关闭。A13已通过N1不重跑。
- LOAD参数现明确opaque/exclusive游标：从latest prior遍历，只复制完整next_before_turn，
  不截UUID/不作为目标selector；错误返回history_not_loaded/0及omit-to-restart说明。
  不自动补/归一化cursor、不回显私值/存储错、不扩大scope/binding/预算。Runtime289/289
  含传给port的原身份及失败archive不变回归；最后未formal，B完整报告仍OPEN。
- 外证据2026-10-03/macos/history-report-bootstrap保留全部首次失败/HTTP/UI/数据库，
  不以exit0关闭原任务或Keychain物理风险，不改Windows结果。

#### B09 新游标版本观察与公开载体 v2（2026-10-03）

- clean `033677469` 正式同 Session 只补报告，3 请求/3 步骤/44 新 events；三条末
  completion_tokens 均 4096，reasoning 16130/15325/15365B、public/tool 均 0，未调用
  LOAD。因此报告仍 FAIL/0；本轮不证明也不否定游标修正的实际采用，不重复原文件/helper。
- 正常 Cmd-Q0/166.761秒、无 expiry/TERM/KILL；闭库五表1172/1/1/4/14同源/ok，
  原1128 events及14 effects保持，原 failed不改；App/fixture/relay关闭。观察器首次
  sqlite输出 ENOBUFS 保留，仅提高外部读取buffer后续读同一实例，未重发任务或扩大产品预算。
- 新交付持久化为 plain_zh_v2/plain_en_v2：已知 search/Git 包装字段及状态说明中文化，
  实际路径/query/snippet/patch/hash、0/false/空集合保留，未知字段完整回落 JSON；旧 v1/None
  不变，冷读回放和精确匹配仍按记录版本。修复 v2 原文大小写 Skill 标记被 renderer 提前
  删除的问题；只转义标记开头而非 JSON 结构括号，逆解值不变，process/read/fallback 同覆盖。
- Runtime291/291、现有 renderer parser 实际调用及独立只读复核通过；这是确定性修后结果，
  v2 尚无正式 UI/模型采用证据，不以它关闭自由报告语言或原 B。A13原完整 N1继续复用，
  N3、真实 Keychain/物理 native 清理失败仍 OPEN。下一 B 样本的隔离 Session 思考深度已
  请求用户选择；未收到前不在未改 Auto 条件下继续付费循环，4096/上下文/权限上限不变。
- 完整证据外置 `2026-10-03/macos/history-cursor-live/`，首败不覆盖，Windows结果未改。

#### B09 历史构建身份兼容与固定读取截止点（2026-10-03）

- 零模型核对发现确定阻断：原 B 与三次补报告的 Session/runtime/build_id/snapshot 相同，
  无 clear-context/Agent transition，但实现 build_digest 不同；旧 archive 全 binding Eq
  必然拒绝原 B。游标及低思考均不能绕过这个产品矛盾，故先停止准备付费复跑。
- 历史文本导入现保留原 source_binding/turn 并计入 archive ID 摘要；仅实现 digest 可不同，
  其他身份及完整 journal 校验保持。不是把旧 binding 改为当前，不更改 EngineBinding Eq、
  checkpoint/recovery、current evidence、权限或进程控制资格；owner-auth port 仍决定来源。
  Native/message history 的 supplied cursor 同时拒绝当前/未来 Turn，固定 accepted-root cutoff。
- Runtime294/294、1项显式外部夹具测试默认 ignored；本次另运行该项，用原 B 原库只读派生
  的634事件，33/33记录原正文/错误标记/原 binding/完整READ逐项相等、无截断，模型/owner0。
  App history 5/5与独立只读复核通过。首次兼容失败及首次损坏夹具假设错误分别保留，修后另记。
- 正式原B报告仍 OPEN，不能把原记录读取回归当模型实际采用。Mac再次锁定、下一任务未启动/
  未发送；已请求解锁。完整外证据 history-cursor-live，下一隔离目录 history-upgrade-report
  仅预备，无新模型调用；Windows验收未改，A13原完整N1继续复用。
- 合并远端进程lease修复后源 `2e21af9e4`：macOS新增lease-retirement回归1/1、Runtime294
  再通过。正式Tauri debug构建及ad-hoc deep-strict包已准备（非release/notarization），
  产品hash55aab3f2…、未改UI hash4ab19979…，完整元数据外置history-upgrade-report；
  App/relay/fixture均未启动、下一任务未发送。构建不替代正式UI/模型验收，等待手动解锁。

#### Keychain 清理前置：Robot 的真实本地 join（2026-10-03）

- 原 RobotServices 只 abort accept-loop，source/session、旧turn及 run_device_turn bridge
  另行 detach，不能证明 SQLite 消费者已停止。本批仅补这个必要前置，不声称物理 CEF 已修。
- Gateway/source/session/writer/pacer/turn/discovery 与桥接 worker 现在由 retained completion
  清单持有；停止封准入、请求同一任务取消并 join，替换 owner保留。5秒等待超时/调用者取消
  不丢完成凭据；panic/异常取消/锁中毒不能转成功。ASR停止不发布晚文本，dispatch 已接受
  后停止保留结果并取消同一request，不开启语音投影；业务失败与本地任务清理分别记录。
- 生产 host 先确认 Robot/SSH再进入Browser，仍独立等待 canonical Agent runtime；Robot/
  SSH未确认则保留Browser，任何清理错误不关DB。兼容 dormant fallback仅换真实join，
  不冒充全部资源/远端provider/物理Robot已停止，更不以abort或exit1宣称CEF物理完成。
- 最终Robot lib153/153，App bridge2/2、Robot失败保持Browser/DB隔离1/1、既有Agent失败
  重试保持Browser/DB1/1及独立复核通过。初版151及首次App测试清理代码编译失败保留，
  修后另记；四个现有非本簇warning未吞/未改。无新模型、正式UI、native物理故障样本。
- 真正系统 Keychain等待、Helper精确登记/guardian、Browser资源与物理FFI分段及修后live
  仍 OPEN；本次不能标整个风险PASS。完整日志外置history-cursor-live，Windows验收不改。

#### Keychain 清理前置：Browser/存储分段与迟到消费者（2026-10-03）

- R4/C05-09/10：Browser资源关闭与工厂物理关闭拆分；先批量封资源准入，全部页面/文件/
  operation关闭成功才放行native。关闭调用者取消保留同一物理worker，成功不重入，失败
  才允许同owner显式重试；runtime_changes关闭期间不得懒创建。21项workspace检查通过。
- 存储独立承诺默认false，仅macOS CEF工厂true；生产host按真实消费者/Browser资源→
  Companion+SQLite关闭确认→CEF tail，其他工厂仍native→storage。存储worker被保留，
  调用者超时/取消不会丢失；DB关闭后native失败的重试不重新运行producer或读已关闭DB。
  Gateway ingress独立先停，任何前置错误都拦native/DB；实际Native计数器保持失败分支0。
- 两个首次红确认BackgroundTaskRegistry在外层取消时丢join句柄/已观察panic。RAII drain
  现在在释放shutdown owner前归还句柄与未送达错误；7项回归通过，非吞错或删除失败。
  Knowledge取源/启动恢复也曾detach且晚写SQLite；现封准入、publication fence与同任务
  join，owned local I/O等真实worker结束，普通请求预算不变。6项定向及旧create/resume通过。
- CEF真实FFI返回立即独立标记；ack-loss继续failed/不重入，但不再显示native仍运行。
  native状态4/4、App带Browser22/22及默认21/21、独立复核通过。新增原生guardian尚未接入；
  本段只认证机制，不认证系统等待/物理退出。首次后台失败与Gateway误当native的旧spy
  失败分别保留，修后以独立native断言验证；原正常native/ack-loss live不移植为新源码PASS。
- 本轮未发送模型或正式UI任务。A13原完整N1继续复用，B09报告仍OPEN/等待手动解锁；
  日志外置history-cursor-live，无凭据/大索引入Git，Windows新共享路径未代验。
  macOS桌面目标定向cargo check通过，未吞既有warning；不把编译当新正式native验收。

#### Guardian 所需的精确 Helper 身份基础（2026-10-03）

- 新macOS-only opaque注册子进程authority：只从实际Unix socket内核peer取audit token，
  强制same UID、真实PPID、父启动代次和必需的精确可执行路径；不能填任意PID/token，
  无组杀/名称扫描/PID fallback。缺native symbol拒绝，正数errno正确解码。
- 原生8项实际子进程检查通过，另2项fixture入口在父测试运行中为no-op；unit2/2通过。
  精确登记执行代终止并确认absent、无关sibling仍活、完成exec后的新程序仍活且旧token
  不命中、parent/path/alias边界成立。不把执行代absent说成全部PID被回收，不声称exec前
  已发信号不会随pending signal继承。全部目标来自本次自有Child，无权限/保护扩张。
- 仅基础API，尚未由CEF Helper/独立guardian消费：launch nonce、初始化前guardian启动、
  精确库存与后台失败退出仍待接入。不能由这组真实kernel测试关闭Keychain/system-wait
  或正式Tauri Case。完整日志外置history-cursor-live；Windows路径未改变/未代验。

#### Guardian 接入与真实本地清理边界（2026-10-03）

- C05-09/10、A11/A13/A17/A18 的退出子断言：CEF 加载前由受管进程启动独立 guardian；
  私有 socket、内核 peer PID/UID、Main birth/path、5种打包 Helper 路径及一次性 launch nonce
  共同登记真实 Helper。Helper 在 Sandbox/CEF 前登记，未知角色/身份/库存失败关闭；不按
  名称或 PID/进程组扫描扩大终止范围。正常返回、初始化失败及30秒 native期限后共用5秒
  Helper清理预算，未决声明/迟到证明不能转完整清理。guardian本身确认 Stop 后才 join。
- 仅宿主真实消费者/资源/存储关闭后的 Rust 入口可启用独立失败 monitor；普通关闭、其他
  平台默认路径及已有 ordinary flight 不自动升级。实际FFI返回与ack/完成分开；主线程仍
  在FFI且登记执行代全部absent、guardian已join时才允许进程自行非零退出。未知证明不退出，
  不重入CEF、不把timeout当完成，不扩大权限或改动Keychain/TCC/Seatbelt。
- 首次真实 Unix RPC 在对端关闭后设置读超时返回 EINVAL，最小 socket 测试首红保留；
  改用同一绝对期限内的非阻塞读写/poll，读取关闭前缓冲，保持内核peer及birth/path检查。
  ACK夹具首败是关闭前未证明服务器接纳；修后仅读响应帧头1字节、丢弃正文，再独立Status
  核对解除期限，不以未接纳请求放宽身份校验。首次协议严格字段拒绝失败也保留。
- 本机 macOS26.6.2/25G83 arm64、APFS非大小写敏感/owners enabled。生产server/client真实
  子进程夹具四路径：正常返回1.745秒、30秒超时31.305秒、确认正文丢失2.721秒、初始化
  失败1.689秒，均exit0/noexpiry/noTERM/noKILL（指外部supervisor）；登记Helper被精确清除/
  回收、无关同路径sibling两次PING仍活、guardian回收/socket移除。超时receipt仍明确
  native_running=true；夹具没有加载CEF、访问Keychain或执行生产紧急exit1，不能代判R4。
- 定向验证macOS lib32/32、Browser workspace23/23、App存储关闭后重试1/1、desktop check、
  process-runtime边界及实际Helper/example构建通过。模型调用0，完整首败/修后/环境/制品hash外置
  `2026-10-03/macos/guardian-native/`。真实打包CEF Helper登记、系统Keychain等待、主线程
  物理阻塞后的非零自行退出/冷库仍未live；release/notarization/N3未验，Windows结果未改。
- 同时只读刷新原B：1172events/14returned effects，0running/accepted/completed，新
  history-upgrade-report仅准备文件，最新真实请求仍旧cursor轮的3次thinking-only。解锁
  问题内“已发送/loader已采用”的描述不符合本次新轮证据，已纠正；未重发原操作/新增请求。
  本次桌面接口仍报locked，正式B报告和CEF UI验证待手动解锁，不把这一阻断扩为确定性阻断。
- 正式验证包已准备：Tauri debug/no-bundle源`2d8366dbc`，未改UI复用；使用既有CEF装配
  入口更新全部5种Helper（旧history-upgrade-report包未覆盖）。Main签后hash7442854f…，
  5Helper逐项签名/源Mach-O UUID核对、deep-strict及CEF smoke示例编译通过；完整receipt在
  外部`guardian-formal/run-001-build/`。下一B runner已指向此包，但App/relay未启动/请求0，
  构建不抵扣正式验收。Session-only Low现有合法入口及wire增量已只读确认；尚未设置或验证
  StepFun实际采用，不扩大4096输出/上下文。桌面仍locked，不追加新夹具或重复构建等待解锁。

#### B09 Low 首次公开回答与历史作用域首败（2026-10-03）

- 桌面解锁后同步`be2bdd246`（仅Windows定向测试/进度，Mac运行路径未改），正式Tauri
  源`2d8366dbc`包复用；UI进入原Session并只将该Session思考深度设为低，未换模型/默认。
  真StepFun接受1请求，wire reasoning_effort=low/max_tokens=4096；1步、31新events、
  Turn completed，但原B严格 **FAIL**：没有调用已展示的loader，公开回答谎称文件创建/
  修改/字节/hash/路径记录不存在，遗漏原10次工具失败总数；helper结果及2次exit2部分交付。
- 独立33记录原B审计与最终32-byte/LF/hash证明文件结果实际可读；请求中仍有旧回合
  119-byte history读取失败与空archive，首次当前reader状态说明缺失。不能把旧错误当当前
  不可用/文件不存在，也不把1次低思考回答当稳定性或完整报告通过。
- 修正closed-history的LOAD/SEARCH/READ投影：标记原source_turn、过期archive record IDs，
  操作cursor仍需当前平台核验；原output/error保留，普通owner结果、archive codec校验和
  checkpoint路径不重解释。当前已授权loader首次展示时补其reader作用域说明，实际读取后
  由真实archive状态替换；无port或工具被门控隐藏不冒充可用。没有自动读库/扩大权限/预算。
- 两语义首红后修，Runtime296/296、默认ignored的原B634事件/33完整记录回归另显式通过；
  测试枚举字段及文案大小写oracle首次错误也另存，断言保持。修后模型采用尚未验证。
- 实际Cmd-Q exit0/128.292秒，无expiry/TERM/KILL；封存1203/1/1/5/14五表与源库逐项同，
  原1172 prefix及14effects精确同、双库integrity ok，新owner效果0。A13原完整N1继续复用，
  原B failed Turn和全部首败保留；CEF/Keychain未用，不代判其风险。完整日志/截图/数据库/
  public reply/HTTP在外部history-upgrade-report；Windows结果未改，不重发旧操作。

- 修后正式源`30c274dee`再次封存 **FAIL**：2请求（1摘要+1主模型）、1步、35新events，
  scoped旧回复确实进入摘要输入、当前reader说明与LOAD schema确实进入主请求，但模型
  仍零历史查询并重复错误文件缺失结论。不是修复包错版或loader不可见；无新机制不再付费
  复跑/纯文案微调。后续聚焦检索/缺失结论路径，不能靠旧摘要或普通完成终态判完整交付。
- 此次正式Cmd-Q0/52.473秒/noexpiry/noTERM/noKILL，1238/1/1/6/14五表与源库全同、
  原1203 prefix及14effects精确同/双库integrity ok；新owner效果0。外部history-scope-recheck
  独立审计/公开回复/HTTP/UI/源码包与旧失败分开。原A N1复用、B完整报告/公开语言及真实
  Keychain仍OPEN，不代判Windows或恢复阶段全范围认证。

### 用户重开第 1 批：Keychain 风险与 native 清理失败（2026-10-02）

- 范围 C05-09/10、A11/A13/A17/A18 的退出子断言。先同步远端 `513a8efcd`；本机仍为
  macOS26.6.2/25G83 arm64、APFS 非大小写敏感 Data 卷/普通用户。保留原 Keychain/exit0
  首败；不恢复旧矩阵，不把复杂 A13/B09 的生成残余改为 PASS。
- 真实风险边界进一步确认：旧样本的 CEF worker 在 `SecItemCopyMatching → SecurityServer`
  decrypt RPC 等待，`cef_shutdown` 占用应用主线程。后端30秒 timeout 和 `app.exit` 排队
  **不能取消这次 FFI，也不保证主线程卡住时按期限退出**。本次未安全重现系统等待；没有改
  Keychain、加密、Sandbox/TCC，未将 shutdown 移出要求的主线程或跳过物理清理。此风险未修。
- 补可重复的最小 native 验证入口：仅 debug、有效 UUID 与当前隔离数据根标记同时匹配，
  **实际 `cef_shutdown` 返回后**才丢弃完成确认；保持未确认状态，所有重试禁止再次进入 CEF。
  release 不包含入口，原正常机制不变。这是确认丢失故障，不伪称物理 CEF 故障或 Keychain 阻塞。
- 源 `513a8efcd` + 外置 source.patch 的正式 Tauri/adhoc strict 包，两独立 Session 均实际
  打开 Browser、导航本地页、真实点击；独立 witness trusted/count1/model0，终态截图后 Cmd-Q。
  正常 PID36232：CEF return51ms、native exit0、Quit后约127ms；确认丢失 PID36834：实际
  return57ms、原有限重试4次，native **exit1**、Quit后约3.195秒，无虚假清理成功/原生重入。
  两样本均无 expiry/TERM/KILL，模型调用0；生命周期分别73.69/82.74秒，未增加180+5期限。
- 3项 native 状态/故障根绑定与2项正式进程退出码回归通过。闭库后五表源/备份逐行全同，
  integrity ok，Turns/effects0、精确3个 bootstrap lifecycle events，无模型事件；两独立
  Principal/Session profile 的真实URL仅在绑定History而非全局Default。App/Helper/fixture/
  listeners全部消失。首地址粘贴拒绝、首次导航未结算、首审SQLite14及错误events0假设分别
  保留，后续输入/观察/只读审计另记，不重复任务、不覆盖首败或放宽产品断言。
- 结果：**native 确认丢失 → 不重入 → 未确认退出码1**正式 N1 已补；真实系统 Keychain
  等待/物理清理失败仍 OPEN，不可由本样本关闭。复杂 A13/B09 尚未在本批复跑或修复；
  无权限绕过或新的共因，故共享/Windows结果不改。完整证据
  `2026-10-02/macos/native-cleanup-failure/`，Git仅保留2处最小debug验证源码和本进度。

### 快速执行 R1：精确文件短链（2026-10-02）

- 用现有证据确认仍无 owner/codec 改字节根因，不为凑 PASS 改执行层。源 `9889e3a22` 正式
  Tauri/adhoc strict，本机 macOS 26.6.2/25G83 arm64、APFS 非大小写敏感 Data 卷；拆短自然任务
  只创建→局部修改→回读→报告，不附带命令/helper/取消。冻结 8/4096/180，native300+5。
- 实际 StepFun 5 请求、163 events、4 原生参数与 canonical 全同；write 首次含末 LF/34 bytes，
  patch 一次用原 write receipt 的 existing SHA guard，仅改第二行，最终回读/磁盘 33 bytes、
  SHA `e90623013b002aa3e515c47b73879eceae781b38ce946ded8d35532541824213`，完整内容/末 LF 保持。
  report 首发接受、工具错误0/命令失败0、实际两效果各一次 returned，两原件不变、无额外文件。
- 正式 UI 终态截图与 Cmd-Q 同控制段，native exit0/91.90秒（含准备）、无 expiry/信号，
  CEF unused_closed；fixture shutdown200/exit0，自有 App/Helper/listener 消失。完整只读快照
  events/heads/turns/effects 为 163/1/1/2，源/备份逐行相同、integrity ok；凭据审计0/117。
- 只记文件操作/字节子断言 N1，未改产品源码；B09首败、完整B/N3保持。报告仍有工具/EOF及
  `earlier observations`措辞，未宣称完整公开语言验收。外置 closure 首审误用不存在表名保留，
  查实际 schema 后另记全表核对，不重跑模型。下一项 R2 默认GEN/多结果；本批构建可复用。
  证据 `2026-10-02/macos/r1-exact-file-short/`，Windows结果未改。

### 快速执行 R2：默认 GEN 三项实际结果（2026-10-02）

- 修复验证入口缺口：现有 General fixture 会减去 Computer/自动化，新增明确 opt-in
  `--live-default-general-commands`，原减权模式不变；默认模式保持完整模板文档，只选择正式
  `local-desktop`/`installation-scheduler` 两个必需资源，不授予系统权限或执行其动作。
  模式断言首红保留，模式/全文保持/资源选择/原预算四个定向回归均1/1。
- 第一次完整配置预检422 `RESOURCE_SELECTION_REQUIRED`（缺computer/scheduler）保留；
  未运行模型/Turn/events均0。按产品实际资源ID补齐后在独立run建立正式会话，不重置付费
  任务预算；复用R1正式签名App，fixture源码/patch单独冻结，6/4096/180、native300+5。
- 完整默认General配置选择经正式Tauri入口验证：15个capability selections及Skill与官方
  `assistant.general` manifest逐项全同，完整revision/Session绑定前后不变，5个正式资源
  精确绑定。真实StepFun4请求/150events/4原生参数全同；`/bin/pwd -P`、`/bin/ls -a`各一次
  exit0/reaped，指定原文一次读取、report首发接受/错误0。cwd、五名称（含`.`/`..`/hidden）、
  文件首尾原文均在报告/真实UI完整交付，三原件全同、无额外文件、两命令证据分别引用正确。
- 终态截图/正常Cmd-Q同段，native0/163.86秒（含准备）、无expiry/信号/CEF初始化；fixture
  shutdown200/exit0、自有App/Helper/listener消失。闭库七表逐行全同/ok，核心四表150/1/1/2；
  凭据0/149。仅记默认GEN命令入口和三项结果N1，原A13复杂漏项/全A/N3/默认GEN其他业务
  未关闭，Computer/自动化实际动作未验。本轮共9个模型请求，不为同根因再补重复。
  证据`2026-10-02/macos/r2-multi-result-default-gen/`；下一簇R3实际旧MM retry，Windows结果不变。

### 快速执行 R3：真实重试与暂停不冒充运行（2026-10-02）

- 新本地HTTP拒绝现场揭示真实缺口：Runtime已记`EXECUTION_MODEL_INVALID_REQUEST`暂停，
  Creative transport却丢弃pause信息、port当unknown处理。复用严格canonical pause helper，
  增加非终态paused观察/消息行，显示正式暂停提示、不显示运行spinner；pending/发送屏障和
  原Stop确认保留，不自动重发/取消/完成，匹配恢复才继续，迟到旧内容不抬回running。
  最小pause反例首红保留；直接UI/port/共享pause回归41/41（含迟到内容）、类型/桌面边界/构建通过。
- 单独复现实际失败消息retry：debug opt-in仅匹配指定提交UUID、同一loopback turn端点，
  真实鉴权请求成功后只丢一次返回，不改参数/权限/receipt；release无此注入。Rust范围1/1、
  六种JS行为断言通过。正式Tauri出现“提交结果尚未确认”及真实retry按钮，实际点击同一
  消息再沿原key核对；仅1个Turn/operation、1次本地模型调用、0效果，30events/completed，
  pending清除/两canonical消息、空画布不变。不是新Session首发成功或UI绘图代验。
- 另一个HTTP暂停子样本：正式UI显示已暂停、无retry、pending保持；显式Stop才cancelled/
  ready，28events/0效果。本地模型1次，native0/22.49秒；重试native0/18.37秒，修后冷读
  同data0/15.83秒、原30events/同Turn/operation保持、本地调用仍1，无旧任务复活或再次发送。
  两源/备份八表逐行全同/ok，App/Helper/listener无残留。真实StepFun/付费请求0。
- 首次240秒native观察超时TERM0、首次90秒冷读超时TERM0均FAIL保留；控制段修正后另记，
  不增加时限洗绿。外置首审误将暂停当turn.state=paused、把scope key当裸UUID亦保留，按
  真实schema/owner/session构造完整key后另审，不改数据/断言目标/重跑模型。仅本机隔离等价
  失败/实际retry及pause子断言N1，不冒充原Windows数据、N3/完整MM；下一簇R4。
  证据`2026-10-02/macos/r3-mm-actual-retry/`，原旧MM首败/历史冷读记录未改。

### 快速执行 R4：退出边界复核与残余归类（2026-10-02）

- 旧真实线程样本仍证明CEF worker停在`SecItemCopyMatching→CSSM_DecryptDataFinal→SecurityServer`
  RPC，native退出等待；未用CEF的初始化已由C05-10避开、失败码由C05-09修复，原首败不删除。
  不把OS等待当新的argv/文件/会话根因，不跳过CEF shutdown、不改系统钥匙串或线程要求。
- CEF engine/lifecycle/host自`a3a0bf001`逐字节无差异，退出码方法与`6f1aaa270`全同。只读复核
  已有正式Browser真实点击/独立trusted witness、精确principal/session/binding profile，URL不落
  Default；actual native_return119ms、正常退出/无expiry或信号仍是有效复用，不追加同构native。
- 当前`process_exit`保留清理失败、未验证不宣称零退出、held native cleanup不可由另一waiter
  判成功三项定向各1/1。首次native过滤命令实际0 tests保留，不计通过；核对全名后另跑1/1。
  不把模拟held cleanup或协调器断言代替真实CEF/Keychain故障分支。新模型/native运行均0。
- 本簇结果：已验机制复用，系统Keychain等待OPEN/未修，修后失败分支live未覆盖；没有新有据
  的产品改动，不扩为Browser进程隔离、权限变更或全生态认证。按方案停止线如实交付残余，
  完整R4风险不记PASS。证据`2026-10-02/macos/r4-closeout/run-001/`，Windows结果未改。

## 上次活动问题簇（阶段性交付后不再自动排程）

### 2026-10-02 再收敛：当前只处理两项核心缺口

用户要求停止低效发散。C05按需CEF已交付后，准备中的MAC-C-04追加复跑在发送前关闭：
StepFun请求0、原生App未启动、fixture shutdown200/exit0、PID/listener消失；不记验收PASS。
后续按具体缺陷而非“复杂A/B/C整组全绿”推进，不再为N3/角色矩阵机械复跑已有子链。

| 活动缺口 | 现有事实 | 下一步边界 |
| --- | --- | --- |
| 结果完整交付 | C06-15修高优先级策略与历史命令资格矛盾，日常两命令实际结果已完整交付N1；A13复杂样本首败仍保留 | 本直接产品矛盾已修/针对性验证，不循环整组A或宣称全部模型漏项解决；无新事实不追加 |
| 精确文件字节/步骤 | B09三次原始write均缺末LF，参数原样；修改hunk先于创建，源guard实际可选；现有字节/恢复回归4/4 | 用户选择方案1：未解决的生成精度残余后续安排；不改执行层凑PASS、不复跑同根因 |

直接复用已有相关源码未受影响的证据：A13真实命令/两指定测试，B08精确文件链、B03stdin/EOF，
GEN-C02纠正/活体Stop/冷读，C05-09退出状态传递及C05-10未用/首次使用CEF。它们是各自子链
的有效证据，不拼成旧整组Case/N3全PASS。已用CEF的Keychain等待、完整默认GEN、旧MM等风险
另列，不继续展开生态认证。每个有证据的新修复只做最小回归及必要的一次正式UI验证。

| 簇 | macOS 候选复用证据（保留原构建及覆盖限制） | 本轮下一缺口 |
| --- | --- | --- |
| C01 命令选择与参数 | M01-03 正式 `ls -a`；M01-05 host `pwd -P`；MAC-C01-01 宿主示例/字面恢复 | MAC-A 的普通命令首发/N3，真实 GEN/COD 入口；不补全全部 CMD 语料 |
| C02 读取与搜索 | M02-02 正式 StepFun 单次精确读取；文件路径/读搜组件 | MAC-A 的中文/空格、头尾/计数、查有与零匹配、源文件不变 |
| C03 Git观察与小测试 | M03 Git 只读/身份边界组件及 native coding 回归 | MAC-A 首次 `status/diff`、按 AGENTS 指定小测试、预期非零的准确说明 |
| C04 文件与步骤结果 | M01-02/M03 文件原子性、权限和真实 receipt 回归 | MAC-B 连续写改/复制移动/回读/hash/精确删除，最终字节与回答一致 |
| C05 进程与停止 | M01-04 正式 start/poll/cancel；M06-01～09 精确 owner/清理 fence | MAC-B 交互 stdin/close 与长 helper/后代停止；不继续组合穷举启动故障 |
| C06 过程与交付真实性 | M01-04 取消成功投影、M02 completion；M04-26/27 正式暂停/取消 | 在 MAC-A/B/C 同次核对 UI/API/canonical、错误类别、实际结果与完成证据 |
| C07 连续会话与纠正 | MAC-A-01 旧正式压缩/完成链；MAC-C07-01 修 soft/hard 余量及有效摘要重复触发 | MAC-A-03 同类失败正式修后重验、MAC-C 纠正/取消冷读仍待补，不由组件或单样本代替 |
| C08 模型协议接合 | M01/M02 正式 StepFun → 实际 owner；Schema/decoder/预算护栏 | 与 MAC-A/B/C 同次核对原生 tool 参数和结果回配，不另跑 wire-only 扩样 |

以上是候选复用，不代表三组综合任务已通过。先核对原始断言、制品和相关源码差异；只有受变更影响、
真实现场反例或明确新风险才运行最小回归，不重跑全仓/全领域或新建同构底层测试。

## 三组正式会话与原验收状态（保留）

| 场景 | 本机正式任务及独立断言 | 状态与执行顺序 |
| --- | --- | --- |
| MAC-A 观察、只读、小测试 | 小 repo、中文/空格文件；cwd/隐藏项、读搜、Git 只读、指定通过及预期非零测试；检查原件与无关哨兵未执行 | MAC-A-13合并复验7请求/两压缩、全部实际操作N1、六命令各一次exit0/1/reaped；report修正无重做、中文UI计数准确，但漏cwd/隐藏项名称/原文头尾，完整A/N3仍FAIL |
| MAC-B 文件、进程、停止 | 同一任务连续写改回读与字节/hash；交互 helper 的 stdin/close、长 helper/后代的 stop 与清理；保留已完成效果 | MAC-B-09 完整链24-call样本仍FAIL：先patch后create、写入漏LF、只到cp；未到helper/report且观察者超时；转C，不循环B，旧03/04子链保持 |
| MAC-C 连续、纠正、恢复 | 连续命令、追加约束、一次实际压缩、取消冷读；要求/证据不丢、结果不串、已完成操作不重放、旧 Turn 不复活 | GEN-C-02补准备公开一次/GEN连续链N1；冷退出实际CEF/Keychain等待及未验证清理exit0首败已采样，C05-09修失败码传递，正常native另验；卡顿/失败分支live/N3未闭 |

GEN/COD 各保留实际入口样本，按实际用途分配 A/B/C，不机械执行三场景 × 五角色 × 两 OS。
MAC-GEN-01 的目录观察、GEN-C-02 的连续纠正/Stop N1均为通用模板仅减去 Computer/自动化资源模块；不代表完整默认
通用配置、完整 A/B/C 或 N3。原 persona/instructions/模型路由/其余授权逐项保持，未扩权限。
已知高频 bad-case 和核心正向任务按共享计划取 N3；无具体风险不新增 20 repeats/100 seed。
M04-27 的真实 StepFun 首次 transport failure 保留，无 HTTP 认证/额度证据；切网后真实请求已恢复。
MAC-A-03～06 已经正式 UI 运行；解锁后 MAC-A-06 完成 C06-02 正式子样本及 completed-session Cmd-Q。
每次 live 先冻结次数/输出/时限，不把已授权模型预算设为永久 0，不由组件或回包代判通过。

## 收敛处置与原综合验收条件（保留）

- **复用**：Schema/整批预检/decoder/CAS、argv/PTY/游标、APFS/文件原子性、权限、lease/fence/receipt、
  secret 隔离等已有定向回归。保留覆盖边界，受影响才最小重跑，安全和真实性断言不缩减。
- **合并**：CMD/PROC/REAL/AGEN/ACOD 的同一命令链、CTRL/OBS 的同一终态和 LIFE/CONC 的同一恢复，
  在 A/B/C 同次运行核对各关键断言，不重复扩角色/层级/故障组合来填原槽数。
- **手测建议/移出活动队列**：模型管理 CRUD、伙伴/创作/客服、Knowledge/Canvas/Office/媒体等普通业务；
  Browser/Computer/SSH/MCP/渠道/设备生态、Git 发布、Schedule/Requirements/委派全功能。已有缺陷保留为
  模块回归；确实影响当前命令链的共同根因仍处理。旧 MM retry spinner 未关闭，需另行按钮/事件复现，
  不能用新 Session 首发成功关闭，但不阻断本轮命令收尾。
- **独立发布认证**：x86_64、其他 OS、signing/notarization、全 CMD/五角色矩阵、每 Case 20 次、全部竞态
  100 seed、LONG/4h/8h soak、408 样本/99% 声明保留原标准，移出本轮结束条件，不宣称已认证。
- **原综合结案条件**：A/B/C 的本机关键链与 GEN/COD 实际入口达标，核心命令 bad-case 根因闭环，UI/API/canonical/
  exit/stdout/stderr/磁盘结果一致，取消无新副作用或孤儿，历史失败如实披露。尚未达到该条件。
  用户本次选择阶段性 P0 交付，不将尚未达到的综合条件改记达标。

### 本次收敛执行（2026-10-01）

- 已同步并核对远端 `6fbed902e`：实施计划/共享进度已收敛，本页原活动队列未跟进；现按同一方案调整。
- 优先执行候选复用核对：M01-03 的 exact 目录输出/exit/reaped 与 M01-04 的原库 start/cancel 同 handle、
  completed/head ready、helper digest 复核通过。仅复用原构建子断言，不拼成当前 MAC-A/B/C PASS。
- MAC-A live 前置的无凭据 HEAD 再次 TLS error（curl 35）；未新建模型 Session、未发付费模型请求，
  只阻断相关 live。继续证据核对和范围整理，未扩大权限、关闭 TLS 校验或改写模型额度。
- 完整依据/只读核对在 `2026-09-30/macos/command-scope-cleanup/`（批次跨午夜，保留开始日期）。
  本次只改排程文档，不再重复 Cargo/build 或增加底层矩阵；Windows 结果和下方历史批次原样保留。

## 历史 macOS 专属集合（不作为活动队列）

以下保留原全产品冻结集合与范围，不继续全量排程，也不将未跑项改为通过。

| 家族 | macOS 专属 ID |
| --- | --- |
| CMD | 001、003～005、012、015、017～019、021、023、025、027、030、033～034、036、038、041、043、045、047、049、051、053、055～056、058、061～062、065、067、069、071、073、076、112、120、122～123、127、132 |
| PROC/TERM/FILE | PROC-006/014/046；TERM-012；FILE-022 |
| Browser/Computer | BROW-017；COMP-008～009 |
| UI/宿主 | REAL-016～019；MAC-001～018 |

## 历史全领域分配（已停止本轮排程）

下表为原全产品计划的历史分配；状态是当时各模块的覆盖说明，不能据此启动完整领域余项。
当前调度仅取上面的 C01～C08/MAC-A～C；共享组件证据仍不替代正式产品验收。

| 领域 | 槽数 | 批次 | 原生测试、排查与修复任务 | 状态 |
| --- | ---: | --- | --- | --- |
| D01 | 425 | M02 | Session/Broker、系统代理/loopback、模型/工具 Schema 与冻结版本 | M02-01 冻结/准入与 M02-02 正式 UI/live 只读 Session 子断言通过；完整领域未验收 |
| D02 | 291 | M02/M04 | 控制/完成、原生 UI、停止/纠正及错误可见性 | M01-03/04 正式 Tauri `CMD-132/139`；M02-01/02 计划、完成与 UI 投影，M04-13 冷暂停列表及 M04-26 页外暂停证据正式 UI 子断言通过；完整 Case 未验收 |
| D03 | 145 | M01/M03 | APFS 大小写/NFC/NFD、权限、原子文件与 Artifact | M01-02、M03-01、M03-05～07 APFS/发布/ACL/xattr/immutable 子断言已验证；其余待走查 |
| D04 | 476 | M01 | /bin/sh/zsh、字面 argv、PTY/process group、Seatbelt 与退出码 | M01-01/03～05 已验 owner、`CMD-132/139`、host 映射及 login-shell 子断言；CMD 全集待验 |
| D05 | 69 | M03 | 隔离 Git remote/SSH、凭据/权限与未知结果 | M03-02/03/08～13 本地 Git/四类 hook/receipt 与隔离 loopback sshd 已验；消息 hook 的 production dependency 亦经正式 desktop build 复核；外部 host/UI 待准备 |
| D06 | 250 | M04/M05 | WKWebView、A11y/Screen Recording、MCP/Plugin/Skill | M04-01～04 已验 native CEF、Computer 权限/media/input 与 MM cold history；M04-05～08 已验 stale A11y ref，M04-09～11 已验 raw pointer、缺失绝对 launch 与用户并发输入；M04-12 修复 input cleanup 组件缺陷，M04-13～22 已验 owner crash/result-loss、Unicode、大型 A11y、Computer soak/click variants、packaged CEF 动态签名、native Browser soak/window reopen、正式 packaged Browser UI/close-to-tray；M04-23 真实 StepFun Browser/Workspace 功能链通过但限额暂停，terminal gate 保持开放；live held-cancel/IME、nested frame 与扩展待验 |
| D07 | 124 | M02/M04 | 精确伙伴/画布/知识/客服 owner 和资源 | M02-01 伙伴/画布精确绑定与 Skill 锁定向通过；正式入口 UI 待验 |
| D08 | 103 | M02/M05 | 五类 Agent 专属入口/任务；独立产物断言 | M02-01 覆盖 GEN/COD/PAL/MM 的 Session/入口子断言；原生全矩阵待走查 |
| D09 | 375 | M06 | watchdog、setsid/丢失 ownership、sleep/wake、恢复/并发/LONG | M06-01～09 已验 shutdown/写锁、start 取消/IO failure/deadline/commit failure、pre-exec 辅助清理、caller-drop/quiesce 与 Engine 精确收据衔接及 macOS fork 初始化子断言；其余故障边界及 LONG 待验 |
| D10 | 33 | M01/M06 | MAC-001～018、PORT；arm64 主 lane，x86 按发布范围 | MAC-005～010/013/015/017 的本批功能断言已验；其余待走查 |
| D11 | 75 | M01/M03 | symlink、Seatbelt/ACL、旧授权和秘密隔离 | M01-01/02、M03-04～07 已验 Seatbelt/symlink/mode/旧授权/ACL/xattr/uchg；末端竞态仍待验 |
| **合计** | **2366** | M01～M06 | 平台结果独立保留 | 本 Windows 执行者不代判 PASS |

## 历史 M01～M06 任务映射（停止穷举接续）

下表只保留历史来源。未完任务须先归入 C01～C08 的实际命令缺口，否则作为模块回归、手测建议
或独立发布认证搁置；不再按 M01→M06 逐领域补完。

| 任务 | 对应 Case | 测试 → 排查 → 修复安排 |
| --- | --- | --- |
| M01 路径与进程 | MAC-001～010/013/015/016；FILE-022；PROC-006/014/046；TERM-012；CMD-132/139/146～150 | 先重跑 macos process 合同；补卷属性/NFC/NFD/argv/group/Seatbelt 夹具，再做 Tauri 命令首发 |
| M02 Session 与角色核心 | D01/D02；AGEN-001；ACOD-001；APAL-001；AMUL-001 | 应用真实 owner 链路与本机代理；复验共享计划、完成、精确绑定和 Skill，最后真实模型 |
| M03 文件/Git/SSH/授权 | D03/D05/D11 剩余 | 独立工作区/remote，权限及原子性负向先行；有外部效果必须带唯一 owner 回执 |
| M04 UI/Browser/Computer/扩展 | BROW-017；COMP-008/009；REAL-016～019；MAC-011/012/017；D06 | 验原生 surface 生命周期/权限与取消，MM retry 需单独按钮/事件复现 |
| M05 条件业务 | ACSR、媒体、Channel/Robot、D07/D08 剩余 | 逐项建正式资源与安全测试账户；缺资源记阻断，能力缺项进入共享问题簇 |
| M06 生命周期与长稳 | LIFE/CONC/LONG；MAC-014/018；PORT | sleep/wake、主进程死亡、故障窗口、恢复 fence 与幂等；然后长稳统计 |

## 历史阶段性 P0 收尾（2026-09-29）

用户因当前算力额度要求先做阶段性 P0 收尾，余项等待后续安排。本轮按已交付证据结算，不把未跑槽
降级为通过：M01 已完成 process/APFS/argv/shell/PTY/group/Seatbelt 与正式 Tauri 关键子断言；M02
完成 Session 冻结、计划/完成、精确伙伴/Canvas/Skill 和一次正式 StepFun 新 Session；M03 完成
macOS 文件发布、ACL/xattr/immutable、Git local/remote/四类 hooks/receipt 与隔离 loopback SSH；
M04-01～03 完成 native CEF 前置、Computer 权限/media 及 TextEdit launch/input 正向。

P0 收尾保留：CEF storage timeout **1/5**、Computer optional plan discoverability 1 个公开 tool error。
M04 其余 stale/cancel/reopen/extension、全部 M05 条件业务与
M06 LIFE/CONC/LONG/soak 均明确延期；真实 Provider N3/20/99%、release signing/notarization、x86_64
也未达门槛。因此本页只表示阶段性交付点，绝不声明 2,366 槽、任一完整领域或阶段二/三全部完成。
用户后续已恢复 macOS 工作；新增批次逐项记录在下文，不倒改此处当时的 P0 结算边界。

## 已导入的 macOS 历史证据

本轮通过远端 `8228b61c3` 导入以下报告，没有在 Windows 上冒充重跑，也不将报告范围扩大到全部 Case：

- [macOS 排查记录](../../reviews/2026-09-26-macos-agent-reliability.zh.md)：本地 HTTP/SSE provider 的真实 owner 链路，
  进程 `macos_` 四项、系统代理、应用暂停恢复等；这些是组件/产品机制证据，非原模型任务全量验收。
- [公共工具与完成协议记录](../../reviews/2026-09-26-agent-tool-contract-reliability.zh.md)：最终构建 `81227bc9d6aa`，
  StepFun 贪吃蛇完成但有 1 次参数拒绝，Agnes 五子棋完成但有 3 次拒绝；另有独立浏览器产物检查。
  中间失败均保留，不能写成 first-attempt 零失败或 99% 可靠性。
- 报告所列 `/tmp/nomifun-agent-followup-*` 是原 Mac 证据位置，本 Windows 未逐文件核验原始证据。
  下一位 macOS 执行者先校验这些记录和当前提交，再给具体 Case 绑定结果。
- 当前新合并构建的原生验收待 macOS runner；依赖主机的任务标为阻断/待运行，不影响 Windows 独立推进。

完整新证据默认存仓库外 `~/code/temp/nomifun-agent-reliability/phase-2-3/<date>/macos/<batch>/<run>/`。
Git 只更新本页的批次结论与必要代码/测试，不提交完整日志或展开索引。

## 已有原生批次与后续聚焦记录

下列旧批次中的“完整领域/阶段/20次/N3/99%仍开放”等为当时结论，不再整体作为本轮前置门槛。
保留首次/修后记录及其原构建；只有当前活动范围的关键链继续推进，移出的模块不追认 PASS。

- **M01-01 process owner / Seatbelt**（PROC-006/011/012/014/016/033～041/046、TERM-012，
  MAC-005～010/015 的本批子断言）：宿主为 macOS 26.6.2 / Darwin 25.6.0、原生 arm64、
  `sysctl.proc_translated=0`，工作区位于 APFS Data 卷。首次反例证明正式
  `ManagedEngineProcessOwner` 仍用 `UnrestrictedLocalOwner`，可写绑定工作区 sibling；另保留
  Seatbelt `TMPDIR` 拒绝被误报清理未知、无执行位目标经 wrapper 过早返回活动 Session，以及
  多个临时 Tokio runtime 并跑 process/PTY 夹具时的 `PeerClosed` 首败。
- 修复后 macOS 产品 process owner 默认使用绑定 workspace 的 `MacSeatbelt` 写根；命令和环境
  preflight 在 watchdog 创建前完成，显式 Unix executable 先核对 X_OK，bare PATH 名仍保留真实
  exec/ABORT 覆盖；确定 pre-spawn 拒绝保留 `user_code_not_started`。测试夹具按单产品 runtime
  隔离，同时保留单 runtime 内 16 路 shell + 长驻 peer、8 路 PTY 并发，不以串行删除并发覆盖。
- 验证：`nomi-process-runtime --tests` **243/243**；process contract **16 项 × 20 轮**、PTY
  contract **12 项 × 20 轮**、独立 8 路 PTY 并发 **20/20**；Engine **32 通过 / 1 ignored**；
  正式 Session→Runtime→Kernel→owner 的本地 provider process 场景 **1/1**；process boundary、
  定向 fmt 与 diff 检查通过。证据：`2026-09-28/macos/m01-process-contract/`。
- 未覆盖：APFS 大小写敏感性与 NFC/NFD、文件 owner symlink/权限/原子写、quarantine 的正式产品
  失败展示、应用内 login-shell Terminal、正式 Tauri 会话区 `CMD-132/139` 首发、x86_64 lane；
  因此不关闭完整 M01、TERM-012、MAC-001～010 或任何 REAL Case。

- **M01-02 APFS identity / file permission**（MAC-001～005、FILE-022/024/031/032/036 的本批
  子断言）：主 Data 卷原生探测确认 APFS 大小写不敏感，ASCII case 变体及 NFC/NFD 名称分别指向
  同一 inode；另创建并挂载 64 MiB Case-sensitive APFS 镜像，确认 case 变体为两个 inode、NFC/NFD
  仍为同一 inode。首次反例中 case 与 normalization 双目标都在第 1 项发布后才失败
  （`published=[0]`、`retained_created=[0]`）；mode 0444 目标又因父目录可写而被 rename 成功替换。
- 修复按卷 `_PC_CASE_SENSITIVE` 构造 patch component identity：macOS 始终做 canonical Unicode
  归一化，仅在大小写不敏感卷做 case fold；等价/祖先目标在任何目录/临时文件创建前拒绝。
  Unix 既有目标在 rename 前打开并持有经 dev/inode 核对的可写句柄，mode/ACL 拒绝不再被父目录
  rename 权限绕过。根内 symlink 正向和根外、dangling、final、`.nomifun` alias 负向沿用同一 owner。
- 验证：主卷 4 项与 opt-in Case-sensitive APFS 1 项各 **20/20**；`nomifun-file --tests`
  **464/464**，另 1 个 opt-in ignored 已显式运行；正式 Session→Runtime→Kernel→file/process owner
  场景 **1/1**。测试卷 `/dev/disk5` 已精确卸载，64 MiB 镜像保留。证据：
  `2026-09-28/macos/m01-apfs-paths/`。
- 未覆盖：更多 Unicode case-fold 特殊组、ACL 动态撤权、immutable flags、目录/目标在最后系统调用
  前的剩余竞态、磁盘/IO fault、正式 Tauri UI 与 100 seed；因此只关闭本批子断言，不关闭完整
  FILE/MAC Case 或 M01。

- **M01-03 formal Tauri command-first**（`CMD-132` 及 `CMD-146～148` 的 macOS 子断言，
  `TERM-012` 组件子断言）：使用 arm64 `NomiFun Dev.app`、`com.nomifun.desktop.dev`、
  隔离 data/workspace 与已有加密 StepFun Plan / `step-3.7-flash` 配置。两个新 Session 的首次
  反例均原样保留且未点击重试：首个用 `cmd:"ls -a"`，且成功命令被误判为必须交付
  文件；首轮修复后已改为字面 `command`/`args`，但 planner 的“在输出中…业务文件”变体
  仍触发同一 Artifact 误判。
- 共享修复将普通单 executable 的 `command` + 字面 `args` 设为模型 Schema 首选，`cmd`
  只用于 pipeline/redirection/glob/compound 等真正 shell 语义。Artifact 推断现区分
  `完整/原始/命令…输出` 及 `在/从/于输出中/里/内` 的观测名词，并在动词序列中移除该 token；
  `输出一个文件`、`保存命令输出为文件` 等真交付仍 fail closed。
- 修复后以第 3 个新 Session 重新首发（不重试旧失败）：Execution/Step/Attempt 均
  `completed`，`delegation_policy=disabled`，2 个模型 step，恰好 1 个 process effect，调用为
  `{"command":"ls","args":["-a"]}`；退出 0、`reaped=true`、无 tool error，原始输出精确为
  `.`、`..`、`.hidden-case`、`inside-link`、`visible.txt`、`中文 space.txt`，UI 绿色 1/1 且
  `visible_failure_count=0`。symlink 仅列名未跟随，fixture hash/链接目标不变，应用退出后端口/进程清零。
- 确定性验证：Agent execution **112/112**、同步远端后 Agent runtime **173/173**、Artifact 定向
  **15/15**；Terminal lib + parent-death **148/148**，两项 process-group 关键用例各 **20/20**；
  合并上游 host-shell/UI 修复后的精确 `.app` 仍 build/start 通过，arm64 二进制
  `fff3fa60359e…`，退出后进程/端口为 0。为遵守冻结的 3 个新 Session 预算，同步后未创建第 4 个
  付费模型 Session；最终真实模型轨迹来自紧邻同步前修复版，同步组合以全组回归和正式启动覆盖。
  证据：`2026-09-28/macos/m01-tauri-command/`。未覆盖
  `CMD-134/139/149/150`、应用内 login-shell Terminal、quarantine/App Translocation 及 x86_64 lane；
  因此不关闭完整 M01、`TERM-012`、`MAC-013`、`REAL-016/017` 或 CMD 家族。

- **M01-04 formal Tauri managed process lifecycle**（`CMD-139`，`PROC-014/034～037/046`、
  `REAL-017`、`TERM-012` 的本批 process 子断言）：宿主为 Darwin 25.6.0 / arm64、APFS，使用
  正式 `NomiFun Dev.app`、隔离 workspace 与已有加密 StepFun Coding Plan / `step-3.7-flash`。
  八个未重试旧 Session 的首次失败分别保留在 run-001/005/008/011/013/015/021/023：包括把同一
  handle 拆成四个 Attempt、用 `/tmp`/raw PID/shell kill 替代 owner、`wait_ms=1000` 后跳过 poll、
  instruction/completion 参数拒绝、取消终态误算失败、完成报告空 `requirement_ids`，以及模型先做
  `read_file`/字面 `ls -la` 探查后恢复；最终完成均不能覆盖这些 `FAIL_RECOVERED`。
- 修复后 planner 对明显的托管生命周期只持久化一个 Agent step，并把完整
  `start_process + poll_process + cancel/close` spec 标记为 `managed_process_only`。该标记只减权：
  Attempt 仅物化 start/poll/input/close/resize/cancel，不能看到 File、`exec_command`、VCS、Artifact、
  discovery 或 delegation；未增加 Snapshot Action 或权限。Runtime/Kernel 统一把
  `state=cancelled + cleanup.reaped=true` 的 cancel/terminal poll 作为成功控制观察，不计失败命令；
  同一 process chain 的精确 start/poll/cancel receipts 可在相同 workspace epoch 完成引用。
  无已发布 requirement ID 时，完成 Schema 不再暴露该字段；有 ID 时只允许精确枚举，严格数组与
  非空断言未放宽。
- 最终 run-025 新 Session 首发通过：Execution/Step/Attempt 均 `completed`，单 step、4 个模型 step，
  持久 profile 为 `managed_process_only=true`；工具序列精确为一次 start、一次 poll、一次 cancel，
  同一 `process_id`，无 read/search/exec/VCS/Artifact 调用，无 tool/control error，两个 managed effect
  恰为 start/cancel。poll 观察 `READY pid=88007 child=88008`，cancel 返回 `STOPPED signal=INT` 与
  `cleanup.reaped=true`；父子 PID 均消失、helper digest 不变。UI 绿色 **1/1**、
  `visible_failure_count=0`，应用退出后相关 listener/process 为 0。
- 确定性验证：Agent execution **114/114**、Agent runtime **184/184**、API types **517/517**、
  Engine **32 通过 / 1 ignored**（既有 Bun PATH live 检查）；Plugin/Kernel 减权、历史取消投影定向
  回归通过，process boundary、Agent vocabulary、fmt 与 diff 检查通过；正式 arm64 `.app`/DMG
  build 成功。证据：`2026-09-29/macos/m01-process-lifecycle/`。未覆盖真正 PTY 的 stdin/close/resize、
  应用内 login-shell Terminal、`CMD-149/150`、quarantine/App Translocation、x86_64 及 20 次/N3；
  因此只关闭 `CMD-139` 本批单样本和相邻子断言，不关闭完整 `REAL-017`、`TERM-012` 或 M01。

- **M01-05 host OS mapping / native login shell**（`CMD-134/149/150`、`TERM-012`、`MAC-013`
  的本批子断言）：静态走查确认 process 文案取 Runtime 二进制的 `std::env::consts::OS`，不读取
  renderer/client OS，但旧 Kernel 没有核对文案所声明 OS 与执行 owner，故无法满足错配时的
  `HOST_OS_COMMAND_MAPPING_ERROR`。另一个首次失败来自正式 Terminal Shell preset：UI 持久化
  `command=$SHELL,args=[]`，后端展开 `/bin/zsh` 后仍保留空 argv，实际不是 login shell；新增失败
  回归精确得到 `left=None,right=Some("-l")`。中间原生 PTY 回归先用 `$0` 判断 login，因 zsh `-l`
  仍保留 `$0=/bin/zsh` 而失败，已保留并改为检查真实 `$options[login]`，未放宽产品断言。
- 修复后 `exec_command` 与 `start_process` 同时声明实际 Runtime process host OS；已通过完整
  Snapshot/binding 校验的调用在 owner dispatch 前核对该声明，错配返回
  `status=not_executed`、`user_code_started=false`、`HOST_OS_COMMAND_MAPPING_ERROR`，明确禁止盲试另一
  平台命令。Shell sentinel 在 Unix 且产品默认空 argv 时补 `-l`，显式 caller argv 原样保留。
- 正式 Tauri host 子断言使用“客户端声称 Windows”作为不可信输入数据，模型仍按 macOS Runtime
  host 首发 `command=pwd,args=["-P"]`；单 step、2 模型 step、1 effect、exit 0/reaped、零其他工具/
  错误，输出精确等于 Session workspace，UI 绿色 1/1。真实客户端仍是 macOS，因此只算
  `CMD-149` 子断言；错配负向由确定性门禁验证，未伪造正式 owner。
- 正式 Tauri Terminal 从“新建终端”选择隔离目录并直接启动 `$SHELL`：DB 保持 `$SHELL`/`[]`，
  原生 PTY 为 `/bin/zsh -l`，UI/scrollback 返回 `NOMIFUN_UI_LOGIN=on SHELL=/bin/zsh`。同一 Session
  relaunch 后确认 leader `45148` 与独立 job group `45373` 活跃，再用侧栏“关闭”；约 151 ms 后两
  PID 均消失，row 保留 `exited`/`exit_code=null`。随后再次 relaunch 的 login probe 正常 `exit 0`。
  含双引号 `$!` 的 `dquote>` 已在原生交互 zsh 独立复现为 shell/BangHist 语义，产品传输字节一致，
  未以吞错修复。
- 验证：Terminal **149/149**；login + job-group 两项各 **20/20**；同步远端后 Runtime **187/187**；Engine
  **33 通过 / 1 ignored**；host mismatch、AppTranslocation classifier 定向回归、process boundary、
  Agent vocabulary、fmt/diff 及正式 arm64 `.app`/DMG build 通过。当前构建无
  `com.apple.quarantine`，未实际触发 Gatekeeper/App Translocation；x86_64 仍未运行。证据：
  `2026-09-29/macos/m01-host-os/`、`m01-terminal-login/`、`m01-mac013/`。因此不关闭完整
  `CMD-149/150`、`MAC-013/016` 或 20 次正式 UI 门槛；`TERM-012` 仅关闭本批原生主 lane 子断言。

- **M02-01 Session freeze / plan-completion / official binding / Skill lock**（`AGEN-001`、`ACOD-001`、
  `APAL-001`、`AMUL-001` 及 `EXT-006/012` 的本批子断言）：同步到 `a3a2b4f96` 后在 Darwin
  25.6.0 / arm64 原生 runner 定向验证。新 Session 创建/初始 Turn/Checkpoint 准入保持原子、幂等且
  冻结 Snapshot 与资源 owner；同一官方配置可创建不同 Session，但 binding 不漂移。可选计划保留
  原始义务并正确区分幂等重放/真实 replan；`report_completion` 只接受当前证据，关闭计划后不能被
  模型重开，且累计 tool error 不能被抹除。
- 伙伴入口固定 `companion.default` 与确切 Companion/Memory/Scheduler；创作入口固定
  `creative-studio.default` 与确切 Canvas/Asset Library。已选 Skill 的正文/digest/来源进入冻结
  Snapshot 与 system context；未选 Skill 在 provider 调用前拒绝，恢复时必须按冻结内容重水化；
  stale/missing 产品选择不能让新 Canvas Session 继承错误 target。未发现需改产品代码的新根因。
- 验证：Agent Runtime 计划 **8/8**、完成账本 **7/7**、Turn/Session/Skill **6/6**；Agent Session
  冻结/准入 **5/5**；Control Plane Skill/Revision 锁 **4/4**；App canonical Session、模型持久化、
  官方 Agent 复用及伙伴/画布入口 **9/9**。同步远端 `93b5f58e9` 后，新增 command-failure 完成计数
  3 项与 active-plan repair 1 项亦通过，当前合计 **40/40**、0 failed、0 ignored。完整日志：
  `2026-09-29/macos/m02-session-core/run-002-deterministic-post-sync/`，首次伙伴/画布三项记录另保留于
  `m02-bindings-skills/run-001-deterministic/`；合并后增量见 `m02-session-core/run-003-post-remote-command-failure/`。
- 正式 Tauri 隔离夹具的两次预检分别因 dataset work-root receipt 与 work-root owner receipt 不一致而
  fail closed，未创建 Session、未消耗模型预算；未放宽保护。成对复制 data/work 身份后的 run-005
  启动、数据库完整性与端口清理通过，但当时 macOS 锁屏阻断 UI 输入/截图，已停止应用；该缺口由
  下述 M02-02 回补。M02-01 本身不关闭完整 `AGEN/ACOD/APAL/AMUL/EXT` Case。

- **M02-02 formal Tauri new Session / live StepFun**（`AGEN-001`、`ACOD-001`、D01/D02 的本批
  只读子断言）：以当前正式 `NomiFun Dev.app`、原生 arm64、成对隔离 data/work 身份与独立 workspace
  执行 StepFun Coding Plan / `step-3.7-flash`。应用二进制 SHA-256 为 `2d2863ddacca…`；Session 使用
  `assistant.general` /「通用」、精确 workspace binding，协作关闭。run-006 首发工具读取正确得到
  `M02-SESSION-CORE-第一行`，但“逐字 / character by character”被模型解释为字符间插空格，最终
  canonical/UI 文本为 `M 0 2 - S E S S I O N - C O R E - 第 一 行`；虽 UI 绿色 1/1、零工具错误和
  零副作用，业务结果不精确，记 **FAIL_RECOVERED**，原 Session 未重试且数据库/事件/推理轨迹保留。
- 修正仅收紧验收输入语义，不改产品代码或放宽断言：run-007 使用全新 data/work、Session、Execution、
  Attempt 与哨兵，明确要求回复字节级等于第一行、禁止空格/包装。新 Session 首发通过：外层 1 个计划、
  1 个 read-only step、1 个 Attempt、1 个 delivered completion；2 个模型 step，恰好 1 个模型发起的
  `read_file`，另有 3 个 step-0 instruction read 被独立计数；零 process/mutation 调用、零 effect、
  零 tool error/command failure。工具结果、Attempt/Execution summary、canonical message 与 UI 均精确为
  `M02-SESSION-CORE-RUN07-原样`，UI 绿色 **1/1**；fixture digest 前后不变，退出后 6 个 listener、
  相关进程为 0，数据库完整性 `ok`。
- 确定性 M02-01 的显式 `report_completion` 账本回归与本批正式 Session 的外层 plan/lead-report 分层
  记录，不能互相替代。完整证据：`2026-09-29/macos/m02-session-core/run-006-formal-ui/` 与
  `run-007-formal-ui/`。未覆盖 live N3/20 次门槛及完整 Agent/EXT 家族；开发 bundle 缺 CEF 只阻断
  不相关 Browser 分片，因此不关闭完整 D01/D02 或 M02。

- **M03-01 macOS Unix publication / cleanup race**（`FILE-019/020/028/031/038/039` 及
  A05/A07/A13/A17/A19 的本批子断言）：在 Darwin 25.6.0、原生 arm64、APFS Data 卷上复核共享
  S-D03-45/46/50。首次 6 个精确竞态均通过：macOS 新建发布由
  `renamex_np(RENAME_EXCL)` 原子消费自有 stage，未进入独立的 check→unlink 窗口；同字节或已修改的
  外来 staging 名均被保留。既有目标发布在核对后发生源换名时返回
  `FILE_WRITE_OUTCOME_UNKNOWN`，不生成 publication identity，并保留
  `temporary_cleanup_unconfirmed=true`；发布前目标消失时同样保留自有 stage 供显式对账。
- 三条核心原生边界各重复 **20/20**：60 份保留 observation 精确分为原子新建 20、替换 unknown 20、
  失败 stage 保留 20，额外 cleanup window 为 0。上层 uncertainty/error projection、缓存撤销及
  same-bytes foreign rollback 隔离另 **4/4**。未发现需改产品代码的新根因；完整日志和现场位于
  `2026-09-29/macos/m03-unix-publication/`。
- 未覆盖真实磁盘满/IO fault、immutable flags、准入后的动态 ACL 撤权与最后系统调用竞态、正式
  Tauri/model/UI 文件变更投影、其他 Unix fallback、100 seed/LONG 门槛；因此不关闭完整 FILE、
  AUTH、D03 或 M03。

- **M03-02 macOS VCS literal path / index isolation**（`VCS-004/005` 及 A05/A14/A17 的本批
  子断言）：在原生 arm64 / APFS 的独立临时 repo 中复核共享 S-D05-01。首次 **4/4**：stage、
  unstage、discard、reset 对 `[]`、`!`、`*`、`?` 与 Unix 反斜杠文件名均按字面处理，不改相似邻居；
  不存在的字面路径和目录请求保持 index/worktree digest；反斜杠命名 executable 恢复原字节与执行位，
  symlink 恢复链接自身和 target text，不修改目标文件；冲突/index metadata 未被单文件操作抹除。
- 综合字面路径操作与 executable/symlink mode 恢复分别 **20/20**，隔离 TMP fixture 最终为 0；
  未访问网络 remote、SSH host 或凭据，未发现需改产品代码的新根因。证据：
  `2026-09-29/macos/m03-vcs-literal/`。
- 未覆盖非 UTF-8 Git path、Git index 的 macOS case-fold alias、并发父目录置换、submodule/worktree、
  commit/hook/identity、local/file remote push、non-fast-forward/unknown result、真实 SSH 与正式 UI；
  因此不关闭完整 VCS、SSH、D05 或 M03。

- **M03-03 macOS local Git remote / effect receipt**（`VCS-001/003/004/006/009～014`、
  `AUTH-004/013` 的本批子断言）：正式 owner 的首次 **12/12** 定向通过。status/diff/stage 保持在
  绑定 repo；commit 只消费 staged 内容，同 idempotency key 重放原 commit receipt。真实本地 bare
  remote 的显式 branch refspec 只更新一次，重放返回原 effect receipt；push 等待并发 commit gate 后
  读取新 HEAD，同一 index state 的并发 commit 只有一个生效。
- 真实 non-fast-forward 保持 local/remote 历史不变；force、缺 credential authority 的网络 remote、
  credential-handle 替代和请求 secret 字段均在 remote contact 前拒绝。local push+receipt、non-FF 拒绝、
  commit→push 串行化各 **20/20**，共 60 次，隔离临时 repo 最终为 0；未连接网络或使用真实凭据。
  证据：`2026-09-29/macos/m03-vcs-local-remote/`。未发现需改产品代码的新根因。
- 未覆盖 identity 缺失、hook 拒绝、commit/push 后 result-loss 注入、外部本地编辑并发、
  submodule/worktree、HTTPS/SSH transport、host key/SFTP/远端 shell、正式 UI 授权旅程及长期门槛；
  因此不关闭完整 VCS、SSH、AUTH、D05 或 M03。

- **M03-04 authorization lease / stale Snapshot / exact resource**（`AUTH-001/002/004～008/014/015`
  的本批子断言）：macOS 原生 runner 上 Common scoped authority **8/8**、Agent Kernel
  authority/preflight **8/8**。expired/future/cross-Session claim、错误 Principal/owner/resource/action、
  active-set drift 及 stale dependency graph 均在 owner dispatch 前 fail closed；explicit revoke 从任意
  clone 立即撤销共享 lease，replacement issuance 撤销同 Session 旧 lease，root rotation 后不能续期。
  child/dependency 关系不隐式扩权，冻结 Snapshot 不受无关 registry publication 漂移且拒绝 foreign ID。
- revoke、replacement、cross-Session rejection 与 owner/action/resource/active-set drift preflight 四条
  竞态边界各 **20/20**；撤权/重发及 replacement lease 均不能重置仍存活的共享 request budget。
  未发现需改产品代码的新根因。证据：`2026-09-29/macos/m03-auth-revocation/`。
- 未覆盖原生文件 ACL 在准入后/最终系统调用前动态变化、TCC/Accessibility/Screen Recording、真实
  secret-bearing owner、应用重启后持久旧 grant、正式 UI 恢复提示及高并发 soak；因此不关闭完整
  AUTH、D11 或 M03。

- **M03-05 macOS extended ACL preservation / dynamic revoke boundary**（`MAC-005`、
  `FILE-020/022/025/038/039`、`AUTH-006/014` 的本批子断言）：原生探针和首次产品回归证明 APFS 上
  普通 `rename` 替换 inode 会丢目标 extended ACL；成功 write/patch 均把
  `group:everyone deny execute` 静默移除。修复在目标 dev/inode、stage identity/bytes 核对后及最终
  pre-publication hook 之后，通过已持有 descriptor 执行 `fcopyfile(COPYFILE_ACL)` 并 `sync_all`；
  ACL 复制失败在 rename 前停止，不吞错或扩大权限，Linux/Windows 路径未改。
- 动态夹具在最终 hook 新增 `group:staff deny execute`，发布后新旧 ACL 均保留；deny-write ACL 下
  write/patch 保持旧字节与 ACL，并按共享 Unix 安全清理合同保留两个自有 stage、返回 outcome unknown、
  发送两个无内容对账事件。全组同时保留旧 mode 0444 结果并把过期“立即删 stage”断言收紧为精确
  retained-stage/reconciliation 断言。首次 ACL 丢失、旧断言失败及中间诊断均独立保留。
- 最终 ACL 保留、deny-write、动态 hook 各 **20/20**；相邻 Unix 发布 **4/4**；`nomifun-file` lib
  **308/308**，macOS workspace **6 passed / 1 ignored**，fmt/diff 通过。证据：
  `2026-09-29/macos/m03-macos-acl/`。未覆盖 ACL copy 后至 rename 的 syscall-sized 竞态、xattr/
  resource fork/immutable flags、非 APFS 卷、IO fault、正式 UI 变更投影及长期门槛；因此不关闭
  完整 FILE/AUTH、D03/D11 或 M03。

- **M03-06 macOS xattr / resource-fork preservation**（`FILE-020/025/038/039`、D11 的本批子断言）：
  首个 `xattr -p` oracle 因 CLI 展示换行失败并保留；校正后真实首败确认成功 write/patch 会删除
  `com.nomifun.reliability.fixture`。macOS descriptor-bound metadata copy 扩展为
  `COPYFILE_ACL | COPYFILE_XATTR`，仍在最终 hook 后、rename 前并 `sync_all`；失败在名称改变前停止，
  不把任何真实用户 xattr 值写入日志，其他平台路径不变。
- 普通 xattr 与 `com.apple.ResourceFork` 的 write/patch 保留 **20/20**；final hook 新增的 before/after
  xattr **20/20**，20 份 observation 独立保留。最终 `nomifun-file` lib **309/309**、macOS workspace
  **7 passed / 1 ignored**，ACL 保留/拒绝回归与 fmt/diff 同时通过。证据：
  `2026-09-29/macos/m03-macos-xattr/`。
- 未覆盖 metadata copy 后至 rename 的最后窗口、copyfile 故障注入、超大 resource fork、immutable
  flags、非 APFS 卷、正式 UI 变更投影、100 seed/LONG；因此不关闭完整 FILE、D03/D11 或 M03。

- **M03-07 macOS immutable target refusal**（`MAC-005`、`FILE-020/022/025/038/039` 的本批
  子断言）：首次夹具把 `chflags` 写成不存在的 `/bin/chflags`，在产品调用前失败并保留；改用本机
  权威 `/usr/bin/chflags` 后，`uchg` 目标的 write/patch 首次即拒绝，旧字节与 `uchg` 均保持到测试
  明确恢复 flag。两个自有 stage 各保留拟写字节并返回 outcome unknown，两个无内容事件要求对账，
  未把拟写字节投影成成功或尝试提权/移除系统保护。
- immutable 精确回归 **20/20**；最终 macOS workspace **8 passed / 1 ignored**，fmt/diff 通过。
  证据：`2026-09-29/macos/m03-macos-immutable/`。未覆盖需提权的 `schg`（不设置、不绕过）、最终窗口
  内 flag 变化、成功替换时非阻断 flags、非 APFS、正式 UI 与长期门槛；因此不关闭完整 FILE、
  D03/D11 或 M03。

- **M03-08 macOS Git identity / pre-commit hook / uncertain fence**（`VCS-006/008`，以及
  `VCS-009/014`、`AUTH-009/013` 的本批不确定结果、精确范围和脱敏子断言）：首次产品反例中，绑定
  repo 的可执行 `pre-commit` 明确 `exit 7`，现有 libgit2 commit 仍创建提交，确认 hook 被绕过；首次
  Seatbelt 夹具又误把 sibling 放进系统信任的 `TMPDIR`，两次失败均独立保留。身份缺失路径原已拒绝，
  本批把签名核对提前到 tree 写入之前并保留 HEAD/index。
- 修复后 macOS 仅在 default/`core.hooksPath` 存在可执行 `pre-commit` 时，先核对绑定 repo、staged
  scope 与 identity，再以字面 `/usr/bin/git hook run --ignore-missing pre-commit` argv 进入共享
  `ProcessSupervisor`，使用 30 秒 deadline、完整进程树回收及只允许精确 repo 写入的 Seatbelt；hook
  返回后 commit 路径重新核对 index/scope。嵌套 workspace 不执行根 repo hook；诊断有界且 secret
  脱敏。拒绝、启动失败或超时按 `ExternalUncertainEffect` 写入终态 unknown，唯一资源 fence 同时阻止
  同 key 和新 key 盲重放；无可执行 hook 的 repo 保留不依赖 Git CLI 的既有 libgit2 路径。
- 拒绝 hook、脱敏及持久 unknown/replay fence **20/20**；成功 hook 的非 TMP sibling 越界写被
  Seatbelt 拒绝 **20/20**；无效 identity 保持 HEAD/index **20/20**；相邻 VCS host **15/15**、监督
  shell/direct-program **3/3**，字面元字符未执行，fmt/diff 通过。证据：
  `2026-09-29/macos/m03-vcs-hooks/`。未覆盖 `prepare-commit-msg`/`commit-msg`/`post-commit`、自定义
  stdin、timeout/descendant/HEAD-mutation 故障注入、commit 成功后 result-loss 对账、Windows/Linux
  hook、正式 Tauri/model commit 旅程及长期门槛；因此不关闭完整 VCS/AUTH、D05 或 M03。

- **M03-09 macOS committed receipt / response-loss replay**（`VCS-009/014` 的本批持久 receipt、
  HEAD/tree 与本地编辑隔离子断言）：在真实成功 `pre-commit` 后完成 commit，并故意丢弃返回值；独立
  确认 canonical effect 已为 `returned` 后销毁 host，再写入一份用户 worktree 修改并以同一 Session
  Store 重建。相同 idempotency key 只重放原 receipt，hook marker 仍为一个字节；独立 Git 核对的
  HEAD OID、tree OID、commit count、已提交 blob 与 terminal event identity 均不变，后写的用户修改
  也未被覆盖。首次注入即通过，未发现需改产品代码的新根因。
- 精确 response-loss/restart 回归 **20/20**，相邻 VCS host **16/16**，fmt/diff 通过；证据：
  `2026-09-29/macos/m03-vcs-result-loss/`。本批只覆盖 terminal effect receipt 已持久后的响应丢失；
  physical commit 已成功但 terminal receipt 尚未写入的窄窗口仍只保留 pending fence，尚无自动
  commit-intent 对账。push result-loss、其余 hook、正式 Tauri/model UI、非 macOS 与长期门槛也未覆盖，
  因此不关闭完整 `VCS-009/014`、D05 或 M03。

- **M03-10 macOS prepare-commit-msg / commit-msg**（`VCS-006/008/014`、`AUTH-009/013` 的
  消息 hook 顺序、拒绝、边界与脱敏子断言）：首次反例只配置 `commit-msg` 并 `exit 9`，旧路径仍直接
  创建 commit，hook marker 不存在，确认 libgit2 同样绕过消息 hook。修复将 macOS hook 计划扩为
  `pre-commit → prepare-commit-msg → commit-msg`，支持 default 与 `core.hooksPath`；全部通过字面
  `/usr/bin/git hook run` argv、共享 supervisor、每 hook 30 秒 deadline、完整进程树回收和精确 repo
  Seatbelt 写根执行。
- 消息 hook 使用 Git metadata 内唯一临时文件；host 持有 descriptor 并核对 dev/inode，拒绝替换或
  symlink 后的读取，限制 64 KiB、UTF-8 与产品 512 字符消息边界。hook 前后重新核对 identity、HEAD
  parent 与 staged path 集合；成功 hook 若改变 HEAD/path membership，或其后 commit 步骤失败，一律
  持久化 external-unknown 并保留资源 fence。诊断与后续失败原因有界脱敏，不吞掉原错误类别。
- `commit-msg` 拒绝/脱敏、相对 `core.hooksPath` 下两类消息 hook 顺序与改写、消息文件 identity 拒绝、
  成功 hook 后 index/path 漂移四条各 **20/20**；相邻 VCS host **20/20**，fmt/diff 通过。首次产品
  失败与一次错误 test-binary 路径的夹具失败均保留；证据：
  `2026-09-29/macos/m03-vcs-message-hooks/`。未覆盖 `post-commit` 通知、discovery→exec hook 文件置换、
  timeout/descendant fault、linked worktree/common Git-dir、非 macOS、正式 UI 与长期门槛；因此不关闭
  完整 VCS/AUTH、D05 或 M03。

- **M03-11 macOS post-commit notification**（`VCS-006/008/009/014`、`AUTH-013` 的本批提交后
  通知、失败可见性、HEAD observation 与幂等子断言）：首次反例中 libgit2 commit 已成功，但真实
  可执行 `post-commit` 完全未运行，marker 与 hook 内 HEAD 证据均不存在。修复在 commit ID 创建后才
  以字面 `/usr/bin/git hook run --ignore-missing post-commit` 执行，复用 default/`core.hooksPath`、
  supervisor、30 秒 deadline、完整进程树回收和精确 repo Seatbelt。
- `post-commit` 非零退出不把已经发生的 commit 伪报成失败；成功 effect receipt 内新增有界脱敏的
  hook status/exit code、`retry_allowed=false` 及独立 hook 后 HEAD 核对。成功 hook 若改写 HEAD，
  receipt 明确 `head_matches_commit=false`、`requires_reconciliation=true`；同 key 只重放该 receipt，
  不再次执行 commit 或通知 hook。非零通知失败与 HEAD 改写两条各 **20/20**，相邻 VCS host
  **22/22**，fmt/diff 通过；证据：`2026-09-29/macos/m03-vcs-post-commit/`。
- 未覆盖 physical commit 或 post hook 已发生但 terminal receipt 尚未落库的崩溃窗口、hook
  discovery→exec 置换、timeout/escaped descendant fault、linked worktree/common Git-dir、非 macOS、
  正式 UI 与长期门槛；因此不关闭完整 VCS/AUTH、D05 或 M03。

- **M03-12 macOS native SSH / BSD grep fallback**（`SSH-001～012` 的本批 transport/owner 子断言，
  其中 connect/auth/host-key/SFTP/shell/sudo/reconnect/cancel/concurrency 为真实 loopback sshd）：本机
  `/usr/sbin/sshd` 夹具使用随机高位端口、每测独立 host/client key、authorized_keys/known_hosts、内存
  host book 与加密凭据，不读写 `~/.ssh` 或生产 host。首次 backend 全组在 39/41 时稳定出现两项
  macOS BSD grep 失败：单文件 fallback 带出 `./-:`、`a'b:` 文件名前缀，破坏既有 `line:content`
  结果合同。
- 修复在无 `rg` 时按远端实际类型选 argv：目录继续 `grep -rnE` 保留递归路径，单文件改为
  `grep -nEh`，在 BSD/GNU 都抑制单文件名前缀；扩展 regex、leading dash、shell quote、no-match=1、
  invalid-regex status、timeout 与“不换另一引擎重试”均保持。另加真实 sshd 的 `-`、`a'b` 与递归目录
  旅程。首个 live 断言误把本地 raw stdout 的尾换行用于持久 RemoteShell，既有合同正确去掉一个尾
  换行；该夹具失败独立保留，断言按正式 shell 形状校正，内容/路径要求未放宽。
- shared `nomi-ssh` **83/83**、backend `nomifun-ssh` **96/96**（含真实 pool lifecycle **19/19**），
  全部零 SKIP；三条本地 grep 形状各 **20/20**，真实 sshd grep **20/20**。最终远端 fixture、sshd
  进程与 listener 均为 0；首次失败遗留的一个精确 `/private/tmp/nomifun-ssh-grep-*` 目录检查后移入
  macOS Trash，可恢复。fmt/diff 通过；证据：`2026-09-29/macos/m03-ssh-native/`。
- 未覆盖独立管理的非 loopback host、正式 Tauri/model/UI 授权旅程、真实应用进程重启下的 remote
  in-flight、远端 write 发布后断线的独立 digest 对账、escaped remote descendant 及 LONG/flood 门槛；
  因此不关闭完整 SSH、D05/D11 或 M03。

- **M03-13 message-hook production dependency gate**（`VCS-006/008` 的正式产品构建子断言）：进入
  M04 原生 CEF smoke 的首次 current-source desktop example build 在启动前失败；M03-10 使用的
  `tempfile::NamedTempFile` 只存在于 `nomifun-app` dev-dependencies，故 unit tests 可编译而 production
  dependency build 无法解析 crate。完整首败保留于 `2026-09-29/macos/m04-cef-native/run-001/`。
- 将同一 workspace-pinned `tempfile` 从 dev-only 移至 normal dependency，不新增版本或 lock 漂移，
  保留原有安全临时文件实现。随后正式 `cargo build -p nomifun-desktop --example browser_cef_smoke
  --no-default-features` 通过，相邻 VCS host **22/22**、fmt/diff 通过；证据：同批
  `run-002-prod-dependency-fix/`。该结果只修复产品构建门槛，CEF native conformance 尚未在本批执行，
  不关闭 BROW/M04。

- **M04-01 native CEF/Tauri conformance prerequisite**（`BROW-017`、`MAC-017` 的本批 native
  child-surface/lifecycle 前置子断言）：current source 构建 arm64 Tauri example、固定 CEF 152.0.6 /
  Chromium 152.0.7977.83 与五类独立 helper，并以 fresh bundle/profile 运行。修复 M03-13 的 production
  dependency 后，首个真实 native run 已到 `renderer_crash_settled`，随后 storage context A 的
  `Page.navigate` 30 秒 settlement timeout；结果 `passed=false`、`shutdown_error=null`，main/helper 均
  清零，首败独立保留，未放宽 timeout/assertion。
- 后续四个独立 bundle/profile 连续 **34/34**、`shutdown_complete=true`：覆盖 native child、trusted
  Unicode/click/drag/wheel、input gate、dialog、frame input/geometry/upload、stale chooser、file picker
  cancel、download/cancel、permission deny、semantic observation/viewport capture、RunGuard stop、popup/profile、
  download publication、renderer crash、Conversation storage isolation/tab recreation/clear 与跨 context 保留。
  最终 owned process/listener 为 0，harness 未改工作区。证据：
  `2026-09-29/macos/m04-cef-native/`。
- 脚本明确 `productAcceptance=false`；当前保留的 **1/5** storage navigation timeout 也使长期/99% 门槛
  继续开放。尚未覆盖正式 packaged NomiFun Browser Resource/Agent Session、Tauri window close/reopen 后
  新 surface identity 与旧 frame/session fence、真实模型、release signing/artifact 或 soak；因此不关闭
  完整 `BROW-017`、`MAC-017`、D06 或 M04。

- **M04-02 macOS Computer permission/media**（`AUTH-012`、`COMP-008/009`、`MAC-011/012` 及
  `AGEN-013` observe-only 子断言）：在 Darwin 25.6.0 / macOS 26.6.2 arm64、APFS/owners enabled
  主机上，以 Developer ID 签名正式 Tauri app、每次独立 `NOMIFUN_DATA_DIR` 和独立 loopback vision
  断言执行。首次稳定 dev identity 已有 Screen Recording grant，旧 `--computer-denied` 假设失效；
  随后的 A11y 成功结果又夹带约 857 KiB screenshot，触发无工具压缩，而夹具拒绝。后续还依次保留
  压缩重用 call ID、typed multipart 解析缺失，以及当前 plan/completion guard 首次拒绝等失败，均未
  以放宽断言覆盖；证据：`2026-09-29/macos/m04-computer-permissions/run-003～009-*`。
- 根因一是 canonical `computer/a11y.observe` 复用旧组合 observe 并机会性采屏，把 Accessibility 与
  Screen Recording TCC、上下文预算错误耦合；现 canonical A11y 只返回 AX tree/ref cache、清除旧
  capture geometry，像素必须显式走 `computer/observe`，旧组合工具仍保留 overlay。根因二是原生
  screenshot 以 base64 JSON 文本穿过 Kernel；现只对精确 platform-builtin `computer/observe +
  screenshot`、当前 generation/Snapshot 及 ImageInput route 恢复单一 typed PNG，文本/持久 history
  不再携带像素。正式 canonical PNG 在 JSON hop 前压到 1.5 MiB（base64 ≤2 MiB），拒绝空、超限、
  非法 base64/PNG；避免旧 5 MiB 上限先被通用 4 MiB result 截成坏 JSON，旧 direct 路径不变。
- granted 正向 `run-010-final-ui`（Session `01a0ec40-9fb5-7603-8e8e-3920b1c01d88`）中设置 UI 明确
  两项已授权；5 个模型步精确调用一次 A11y（109 elements、零 pixels）和一次 screenshot，typed PNG
  到达 vision fixture，67-event durable projection 无 base64，`turn/completed`、fixture failure=null。
  UI/最终完成仍公开记有初次 plan guard 的 1 个工具错误。fresh bundle
  `com.nomifun.desktop.reliability.denied` 未点击授权：`run-012-denied-a11y` 和
  `run-013-denied-screen` 各自只启用一个 Action，工作台分别只列 Accessibility / Screen Recording
  缺口；两次精确调用均以 `ROLE_HOST_PROVIDER_FAILURE` 结束、无输入/截图 fallback，普通会话仍可开始，
  各 2 个模型步并 `turn/completed`。
- `nomi-computer` **94 passed / 7 ignored**；A11y no-pixels 与 macOS 权限维度各 **20/20**，Computer
  typed-media 3 项 **20/20 批**，role-host fence **4/4**，高熵截图 transport bound、正式 fixture
  `browser-use,computer-use` check、UARC boundary、fmt/diff 通过。三份 backend/companion DB 共六次
  immutable integrity 均 `ok`，最终 app/fixture process/listener 为 0；凭据前缀扫描为 0。最终证据与
  verdict：`2026-09-29/macos/m04-computer-permissions/run-010-final-ui/`、`run-012-denied-a11y/`、
  `run-013-denied-screen/`、`run-015-transport-hardening/`、`verdict.md`。
- 尚未通过 System Settings 人为制造“一项已授予、另一项拒绝”的 live TCC 组合；当前独立性来自
  可注入 permission 回归、两项 fresh-denied 正式 UI 和 granted 正向。撤权/热变更、真实 input/launch、
  stale window、多显示器/scale、held-input cancel、100-cycle/soak、release、真实 StepFun 及完整
  `AGEN-013` 仍开放，因此不关闭完整 `COMP-008/009`、`MAC-011/012`、D06 或 M04。

- **M04-03 macOS Computer launch/input**（`COMP-002/003/006`、`MAC-011`、`AGEN-013` 的本批
  正向子断言）：使用同一 exact-source Developer ID-signed `com.nomifun.desktop.dev`，每次 fresh
  data/Session、loopback fixture 和唯一 `computer-input.txt`。Mac 锁屏连续三轮时未绕过 UI：fixture
  保持 0 model call、文件 `seed`、DB 完整并清零进程；解锁后才由正式 Tauri 工作台选择仅允许
  A11y/input/launch 的 Agent 并发送验收请求。
- 首个解锁运行错误假设 atomic `computer/launch` 必须先被 plan gate 拒绝；Runtime 合同明确 Computer
  原子动作不自动激活 workspace ledger，故一次 launch 正确以 returned `managed_effect` 结算，fixture
  独立失败且零 input。第二次运行 8 个 effect 已正确执行并保存 `alpha XbetaY`，但 fixture 未校验被
  schema 拒绝的 `report_completion`，UI 正确显示 `NOMIFUN_TASK_INCOMPLETE`；两个首败均原样保留。
- fixture 修正为区分 optional plan 未暴露与产品权限错误，最终使用 Runtime canonical `input_0`、一个
  evidence-backed criterion 及 `observed_tool_error_count=1`，并在输出最终文本前验证 completion，拒绝
  `INVALID_TOOL_ARGUMENTS`/`not_executed`/unexposed-tool 假成功；未改产品 Action、权限、effect 或 planning
  policy。
- 最终 `run-008-final-ui`（Session `01a0ec7b-2f2d-7c30-86e3-8fbfe9aa9186`）为 20 model steps /
  299 events：1 次 launch、9 次 fresh A11y observe、7 次 input；依次 set `alpha beta`、`cmd+right`、
  `option+left`、type `X`、`ctrl+e`、type `Y`、`cmd+s`。8 个唯一 effect 全为 returned，未重放；
  TextEdit AX 与磁盘均为 `alpha XbetaY` 且无 edited marker，后续普通 X/Y 输入也证明 modifier 已释放。
  `completion_reported` 为 plan revision 1 / observation revision 19，覆盖 `input_0` 并公开唯一初始 plan
  tool error；`turn/completed`。fixture `input_verified=true`、failure=null；两 DB `ok`，TextEdit/app/
  fixture/listener 清零。确定性辅助：key **12/12**、launch **6/6**、main-queue/drag 各 **1/1**、
  role-host fence **4/4**。证据：`2026-09-29/macos/m04-computer-input/`。
- 仍未覆盖长 drag cancel、注入式 key/button release failure、用户并发焦点、Unicode/IME/layout、
  缺失 app、owner crash/result-loss、
  撤权、真实 StepFun 与 soak；窗口移动/缩放、same-PID 窗口切换及跨 App 焦点变化后的 stale 拒绝另由
  M04-05～08 覆盖，初始
  optional plan 仍以一次公开错误激活，不满足零工具错误体验。
  因此不关闭完整 `COMP-002～006/010～014`、`MAC-011`、`AGEN-013`、D06 或 M04。

- **M04-04 old MM failure/retry cold-history**（`AMUL-001`、`OBS-008/014`、`MGMT-013` 的 macOS
  历史恢复子断言）：只读扫描本机 10 个 NomiFun 数据库，未发现 Windows 原
  `WIN-MM-PROVISION-01` 或等价多模/创意失败 Session，故未冒充同一历史。新增隔离
  `--creative-failure` fixture，经正式 Tauri `creative-studio.default` Canvas 产生真实失败，再冷启动
  同一安装；不以成功首发替代失败历史。
- 首个 HTTP 400 夹具失败得太早，UI 显示静态 retry，但 Canvas `pendingTurn` 尚在、canonical Session
  已 ready，属于合法的提交未确认重试而非目标旧失败，完整保留于 `run-003-failure-ui/`。随后改为已
  接纳 SSE 的确定性协议终态；`run-005-terminal-failure-ui/` 精确产生 1 个 model call、1-step
  `turn/failed`，错误为 tool-call finish without Tool Calls，Session 回到 ready、Canvas
  `pendingTurn=null`，两 DB `ok`。停止并冷启动后相同 Turn/event/Canvas terminal state 不变，未产生
  第二个模型请求，owned process/listener 清零。
- 用户恢复后以同一 data、同一 failed Turn 再次正式 Tauri 冷启动：Canvas 显示原用户消息、
  `MM_HISTORY_FAILURE_FIXTURE` 和公开错误卡片；不存在 running spinner 或“重试这条消息”按钮，全程
  未点击 retry。冷读只把 canonical 两条消息 ID 一次性 reconcile 到 Canvas `messageIds` 并前进
  `updatedAt`；Agent events 仍逐字节相同，Turn/events/effects 保持 **1/30/0**，nodes/connections 与
  `pendingTurn=null` 不变，3 秒稳定窗口无循环写或新请求。最终两 DB `ok`、process/listener 为 0。
  Creative UI **23/23**、failed-history backend **1/1**；未改产品 retry/persistence 实现。证据：
  `2026-09-29/macos/m04-mm-retry/`。该等价隔离旧失败子断言 **PASS**；原 Windows 数据仍未冒充为
  macOS 复跑，且不关闭完整 AMUL/OBS/MGMT、D06 或 M04。

- **M04-05 macOS stale Accessibility observation**（`COMP-002` 前台焦点变化及 `COMP-010/011` 的
  零盲目输入/结算子断言）：正式 Tauri 首败在 TextEdit generation 2 observe 后切回 NomiFun，旧 ref
  仍成功对后台 TextEdit 执行 `SetValue("STALE_SHOULD_NOT_APPEAR")`；TextEdit AX 和最终 autosave 均
  证明错误输入，effect 误为 returned。根因是 macOS AX invoke 只比较 generation，未消费 observer
  dirty，也未核对 frontmost PID。
- macOS AX semantic invoke 现于动作前拒绝 generation 漂移、dirty window/focus/layout/value、无 live
  observer 或 frontmost PID 变化；显式 PID snapshot 不因无关前台 App 失效，但仍要求 observer/clean/
  generation。首修已阻止输入，却被 host 当 external unknown，产生 `cleanup_unproven` 暂停；进一步只对
  精确“stale + no pixel fallback”内部签名映射 `ROLE_HOST_STALE_OBSERVATION_GENERATION`，effect 结算
  rejected，其他 provider/join/backend 错误仍保持 uncertain。
- 最终 `run-011-final-ui`（Session `01a0ecf6-662c-7702-bdbc-dea173da1262`）在相同焦点切换后一次旧
  ref 调用即被拒绝；1 个 launch returned、1 个 input rejected、零 unknown/pending/pixel fallback，
  Turn **5 model steps / 69 events** completed。UI 明确报告未向后台 TextEdit 输入；TextEdit AX 与磁盘
  均精确 `seed`，fixture `stale_input_rejected=true`、failure=null。AX fence / 零效果 classifier /
  typed host mapping 各 **20/20**，相邻 AX **3/3**、Computer host **5/5**；两 DB `ok`，进程/监听清零。
  证据：`2026-09-29/macos/m04-computer-stale/`。
- 尚未覆盖 OCR/pixel-only ref、right/double-click pixel fallback、用户并发输入、
  held-input cancel、crash/result-loss 与 soak；因此只关闭本批
  `COMP-002` frontmost semantic-ref 子断言，不关闭完整 Computer、D06 或 M04。

- **M04-06 macOS same-PID window-move stale observation**（`COMP-002` 窗口移动子断言）：复用
  M04-05 的 exact-source Developer ID-signed 正式 Tauri app，但使用新的隔离 data/Agent/Session
  `01a0ed01-31b1-7ae0-b47d-8bcdce884dd0` 和一次性 TextEdit 文件。Runtime 在 TextEdit 同一 PID、同一
  前台窗口中取得 generation 2 / `textarea="seed"` 后暂停；随后只通过原生标题栏 drag 移动该窗口，
  TextEdit 在移动前后均保持前台和 focused，未切回 NomiFun，再释放旧 ref input。
- macOS observer 已订阅 `AXWindowMoved`；本次 generation 与 frontmost PID 均未改变，窗口移动通知使
  Snapshot dirty，旧 ref 在任何 AX/pixel 动作前以 `ROLE_HOST_STALE_OBSERVATION_GENERATION` 被拒绝。
  1 个 launch effect returned、1 个 input effect rejected，零 pending/unknown/pixel fallback；Turn
  **5 model steps / 69 events** completed。正式 UI 显示旧 observation 已拒绝；TextEdit AX 与磁盘均为
  `seed`，fixture `stale_observed=true`、`stale_input_rejected=true`、failure=null；两 DB `ok`，最终
  TextEdit/app/fixture process 与两个 listener 均为 0。证据：
  `2026-09-29/macos/m04-computer-stale/run-014-window-move-ui/`。
- 本批未发现新的共享或产品根因，故不改 `PROGRESS-SHARED`；fixture 完成文案仍沿用“切换前台窗口”的
  M04-05 描述，判定依据为独立 UI 动作、typed error/effect、事件、AX/磁盘与清理证据，不用该文案代替
  断言。窗口 resize / same-PID 多窗口另由 M04-07/08 覆盖；上述其余 Computer 边界仍开放，因此
  不关闭完整 `COMP-002`、Computer、D06 或 M04。

- **M04-07 macOS same-PID window-resize stale observation**（`COMP-002` 窗口缩放子断言）：以新的
  隔离 data/Agent/Session `01a0ed10-7d70-7703-b7b1-b645f659218b` 复用同一签名正式 Tauri app。
  Runtime 在前台 TextEdit generation 2 / `textarea="seed"` 后暂停；保持同一 PID、同一前台 focused
  窗口，从右下角以 window-relative `[670,438] -> [560,350]` 执行原生 drag，截图尺寸由约
  **673×439** 变为 **562×351**，再仅释放一次旧 ref input。
- `AXWindowResized` 通知使 Snapshot dirty；input 在任何 AX/pixel 动作前以
  `ROLE_HOST_STALE_OBSERVATION_GENERATION` 拒绝。launch/input effect 分别为 returned/rejected，零
  pending/unknown/pixel fallback；Turn **5 model steps / 69 events** completed。TextEdit AX/磁盘仍为
  `seed`，fixture `stale_observed=true`、`stale_input_rejected=true`、failure=null；运行中及停止后两 DB
  均 `ok`，最终 TextEdit/app/fixture process 和两个 listener 为 0。证据：
  `2026-09-29/macos/m04-computer-stale/run-016-window-resize-ui/`。
- 本批不新增共享结论或产品代码；UI 完成文案的旧“切换前台窗口”表述仍只作展示，不作为 resize 判定。
  same-PID 多窗口身份另由 M04-08 覆盖；用户并发输入、held-input cancel、crash/result-loss 与 soak
  仍开放，不关闭完整 `COMP-002`、Computer、D06 或 M04。

- **M04-08 macOS same-PID multi-window stale observation**（`COMP-002` 窗口切换子断言）：以新的隔离
  data/Agent/Session `01a0ed15-2ed6-7e00-bd34-a0e481e34cb4` 复用同一签名正式 Tauri app。Runtime 在
  `computer-input.txt` 前台窗口取得 generation 2 / `textarea="seed"` 后暂停；随后对仍在前台的
  TextEdit 执行 `cmd+n`，创建并聚焦第二个“未命名”空窗口，再释放第一窗口的旧 ref input。
- OS 进程快照证明两窗口始终只对应唯一 TextEdit PID `97002`；`AXCreated` / `AXMainWindowChanged` /
  `AXFocusedWindowChanged` 观察使 Snapshot dirty，旧 ref 以 `ROLE_HOST_STALE_OBSERVATION_GENERATION`
  在任何 AX/pixel 动作前拒绝。launch/input effect 分别 returned/rejected，零 pending/unknown/pixel
  fallback；Turn **5 model steps / 69 events** completed。第二窗口仍空白；以 `cmd+grave` 切回第一窗口
  后 AX 与磁盘均为 `seed`。fixture stale 两项为 true、failure=null；运行中/停止后两 DB 均 `ok`，最终
  TextEdit/app/fixture process 与两个 listener 为 0。证据：
  `2026-09-29/macos/m04-computer-stale/run-017-same-pid-multiwindow-ui/`。
- 本批不新增共享结论或产品代码；上述可审计动作、唯一 PID、两个窗口的独立内容及 typed effect/event
  共同判定，不以 fixture 沿用的旧完成文案代替断言。用户并发输入、held-input cancel、crash/result-loss
  与 soak 仍开放，不关闭完整 `COMP-002`、Computer、D06 或 M04。

- **M04-09 macOS raw pointer input**（`COMP-003` 的 move/click/scroll、`COMP-001` 同窗口
  A11y/screenshot 坐标关联及 `COMP-013` bounded screenshot 回归子断言）：新增最小
  `--computer-pointer-input` 确定性夹具模式；仓库外构建 arm64 AppKit 目标，以独立状态文件记录原始
  NSEvent。每次 raw input 前必须取得 fresh `computer/observe` Screenshot generation，并把
  **1568×882** screenshot 坐标映射回 **2560×1440** 屏幕；A11y generation 只用于确认目标窗口，不能
  给 raw pointer 授权。
- 首轮目标 App 只接受 argv、被 `computer/launch` 无参数启动后退出；修为隔离 Info.plist 状态路径。
  随后两次错误使用 A11y generation，均由 role host 以 `ROLE_HOST_STALE_OBSERVATION_GENERATION`
  在动作前拒绝，目标计数全零；另保留 typed screenshot JSON 形态误判、AppKit inactive first-click、
  launch/前台 settle 窗口不足及 view-only scroll 断言等夹具失败。click/scroll 最终以目标的本地/全局
  原始 NSEvent 监控判定，未以 tool returned 代替命中事实；所有中间运行、DB 与零残留清理均保留于
  `2026-09-29/macos/m04-computer-pointer/`。
- 最终 `run-023-final-ui`（Session `01a0ed44-c5d6-7420-bfd6-a4c1557e873c`）为 **13 model steps /
  164 events**：5 次 A11y 等待/稳定观察、3 次 bounded screenshot、1 次 launch 及 move/click/scroll
  各 1 次；4 个 effect 均唯一 returned，零 rejected/pending/unknown。产品映射精确为 move
  `(177,719) -> screen (289,1174)`、click `(312,643) -> (510,1050)`，scroll 最终光标 `(740,929)`；
  目标记录 move=2 个 OS motion event、click=1、scroll=1、delta=-3，fixture `pointer_verified=true`、
  failure=null，Turn completed，正式 UI 明确完成。durable events 中 inline PNG 为 0；运行中/停止后两
  DB 均 `ok`，最终 target/app/fixture process 和两个 listener 为 0。结合 M04-03 的 key/text/modifier，
  `COMP-003` 所列 action 集在 macOS 正向路径通过。
- 本批只扩展确定性夹具，未发现需改共享层或产品输入实现的新根因，故不改 `PROGRESS-SHARED`。尚未覆盖
  right/middle/double/triple click、drag cancel、OCR/pixel-only refs、多显示器/DPI、用户并发输入、
  crash/result-loss 与 soak；因此不关闭完整 Computer、D06 或 M04。合并并发共享 settlement 修复后，
  macOS post-merge Computer role-host **9/9**、owner-success settlement **1/1**、fixture build 与 fmt 通过。

- **M04-10 macOS missing absolute Computer launch**（`COMP-006` 缺失应用路径及 `OBS-005/016` 错误
  可见性子断言）：正式 Tauri 首败请求唯一、明确不存在的隔离 `.app` 绝对路径；旧
  `open::that_detached` 仅成功派生 `/usr/bin/open` 即返回，UI 随后暂停，managed launch effect 错误
  returned，而路径、应用进程和窗口均不存在。完整首败保留于
  `2026-09-29/macos/m04-computer-missing/run-002-first-ui/`。
- 产品在 Computer launch owner 的 OS 调用前新增 absolute target/app existence gate，不猜替代路径；
  Engine Core 对精确 `computer/launch` 的 provider failure 投影固定安全指引，不含真实私有路径，明确
  “target path does not exist / Do not guess / No successful launch”。首修正式运行已让 effect rejected，
  但旧 Kernel 仍只给模型泛化 `ROLE_HOST_PROVIDER_FAILURE`；该 31-event 中间失败独立保留于
  `run-006-final-ui/`，未用 code-only 结果关闭 Case。
- 最终 `run-010-final-ui`（Session `01a0ed6c-7414-7b12-8ed8-9734f4195e26`）为 **2 model steps /
  34 events**：唯一 launch effect 为 rejected，bounded observation 保留 owner 的精确不存在原因；模型
  可见 tool error 三处持久投影均含安全恢复指引，Turn completed。正式 UI 明确“不存在的应用路径已被
  拒绝，未猜测替代路径”；目标始终不存在，无替代 app/process/window，fixture
  `missing_launch_rejected=true`、failure=null；运行中/停止后两 DB 均 `ok`，app/fixture process 和两个
  listener 为 0。launch **7/7**、guidance **1/1**、两次正式 Tauri build、codesign、fmt/diff 通过。
- 本批公共根因另记 `S-D06-04`。相对缺失 app 名、安装后又删除的 TOCTOU、显式 opener 异步失败、撤权、
  Windows/Linux 原生 opener 与 soak 仍开放，因此只关闭 macOS 缺失绝对路径子断言，不关闭完整
  `COMP-006`、Computer、D06 或 M04。

- **M04-11 macOS user/Agent concurrent input**（`COMP-011` 焦点/内容变化检测、零争抢循环及
  `COMP-010` 零盲目重放的本批子断言）：新增最小 `--computer-concurrent-user` 确定性夹具模式，复用
  M04-05 已交付的 macOS AX observer dirty / generation fence；正式 Tauri 使用 Developer ID-signed
  arm64 app、隔离 data/Agent/Session `01a0ed96-3270-7ac1-8235-06082022ddcd`。Runtime 在 TextEdit
  `textarea="seed"` 上取得旧 ref 后暂停；外部用户动作只通过原生 TextEdit AX 把前台值改成精确
  `USER_OWNED`，释放旧 ref 前磁盘仍为 4-byte `seed`，未用磁盘写入冒充 UI 并发。
- 两个夹具首败独立保留：`run-002-final-ui`（Session
  `01a0ed8a-226b-7530-9719-97b614b136a4`）把 AX `setValue` 与 `cmd+s` 合并后，TextEdit 阻塞于
  `_NSDocumentSerializationSemaphore wait`，未释放 input effect；`run-003-final-ui`（Session
  `01a0ed90-8831-70a3-ad76-b56db2753ec7`）又被外部文件写入附加换行污染，触发 TextEdit 冲突提示及
  夹具 exact-byte 失败，虽产品已拒绝旧 input，仍不计通过。两次均保留事件/样本/DB，并完成精确清理。
- 最终 `run-005-final-ui` 为 **5 model steps / 69 events**：唯一 launch effect returned；用户值变化使
  observer dirty，旧 `set_element_value("STALE_SHOULD_NOT_APPEAR")` 在任何输入前以
  `ROLE_HOST_STALE_OBSERVATION_GENERATION` rejected，零 pending/unknown/重试循环，Turn completed。
  夹具只在结算时重新读取磁盘仍为 exact `seed` 后才置
  `stale_input_rejected=true`、`concurrent_user_preserved=true`；随后 TextEdit 正常 autosave 于 effect
  结算约 15 秒后把磁盘变为 exact 10-byte `USER_OWNED`，最终 AX/截图与磁盘一致，且均无 stale 文本。
  正式 UI 明确显示用户内容保持；运行中 DB `ok`，停止后以 immutable 只读方式复核两项均 `ok`，最终
  TextEdit/app/fixture process 和两个 listener 为 0。证据：
  `2026-09-29/macos/m04-computer-concurrent/`。
- 本批未发现新的共享或产品根因，只提交可重复的正式 UI 夹具与本页结论，故不改
  `PROGRESS-SHARED`。`COMP-012` 的 held drag/key cancel 与 release cleanup、`COMP-010` owner crash/result
  loss、right/middle/multi-click、OCR/pixel-only、多显示器/DPI 和 soak 仍开放；因此只关闭
  `COMP-011` 本批 macOS 子断言，不关闭完整 Computer、D06 或 M04。

- **M04-12 macOS held-input cleanup hardening**（`COMP-012` 的 timeout/release 组件子断言，
  **部分完成**）：走查确认原生 Computer 输入以 10 秒 timeout 包住 `spawn_blocking`，超时后直接丢弃
  handle；已开始的 drag/key worker 可继续持有输入并产生迟到效果，但上层已经收到失败。新增回归在
  10 ms 测试 deadline 返回时仍精确观察 `pressed=true`，首败保留于
  `2026-09-29/macos/m04-computer-cancel/run-004-first-failure/`，未用 sleep 或放宽断言掩盖。
- 产品修复使 timeout 后继续 join 同一已准入 worker，原生任务未退出前 Engine 不能得到 cleanup
  completion；返回仍是明确 uncertain error，要求重新观察且禁止自动重试。drag/key 新增统一 pressed
  obligation guard：OS press 前记录，逆序释放，首次 release failure 只重试精确剩余项，连续失败保留
  unproven obligation 并进入 Drop best-effort，不吞错。长拖拽确定性夹具另提供 dispatch 前显式 hold，
  便于后续物理/可用 out-of-band cancel runner 复验，不自动宣告通过。
- 验证：`nomi-computer` **99 passed / 7 ignored**；timeout join、release retry、连续 release failure 保留
  三项新增回归通过；Engine Tool Host 的 cancelled caller retained effect/settlement **1/1**；最终 fixture
  build、fmt/diff 通过。Developer ID-signed 正式 Tauri 与 arm64 AppKit 目标的多次隔离运行均实际记录
  mouse-down/drag/mouse-up，最终 `pressed=false`；其中 `run-015-formal-ui` 还保留超时后 effect unknown、
  `cleanup_unproven` 暂停及失败退出残留的完整证据，后续精确清理且两 DB `ok`。
- 但本机自动化 runner 在 held mouse 期间不能对退到后台的 NomiFun Stop 控件执行 AX/坐标动作：分别
  fail closed 为 stale element、no frame/no window；合成按键也不能替代真实物理全局 hotkey。相关时序
  失败完整保留于 `m04-computer-cancel/run-011/015/021/023/025/027/029/035/040/042/`，没有一次被改记
  PASS。因尚无正式证据证明 cancel event 发生于 `pressed=true`，本批不关闭 `COMP-012`、Computer、
  D06 或 M04；key-hold、真实 out-of-band cancel、OS release failure 当时仍待后续 runner，
  crash/result-loss 另由 M04-13 覆盖。

- **M04-13 macOS input owner crash / result loss**（`COMP-010` 的本批完整 macOS 子断言）：新增
  `--computer-input-crash` 确定性夹具模式及独立 arm64 AppKit 文本目标。Developer ID-signed 正式
  Tauri app 以隔离 data/Agent/Session `01a0ee3c-6854-7f83-8273-263152077159` 启动目标并取得稳定
  A11y observation；夹具随后唯一 dispatch 65,536 个 `X`。目标首次记录 **20** 字符时，runner 在
  精确核对 executable path 后只向 Nomi owner PID `36389` 发送 `SIGKILL`，不终止目标或 fixture。
  macOS 已排队输入最终稳定在 **44,480 chars / 2,224 change events**，因此不是“零副作用”；crash
  image 则精确止于 **5 model steps / 67 events**、`tool/call-started` + `effect/started`，Turn/head
  running、唯一 `computer/input` 为 `external_uncertain_effect/pending`，无 terminal/tool result，DB `ok`。
- 同一签名 app、同一 data 冷启动没有重放。该 Computer-only Session 没有 canonical workspace，自动
  recovery 三次明确拒绝后按既有有界策略原子提交 seq 68 `effect/uncertain`（`outcome=unknown`、
  `process_restart_external_reconciliation_required`）、seq 69 recovery-blocked 与 seq 70 turn/paused；
  pending=0、unknown=1、`cleanup_proven=false`。正式 UI 显示“执行已暂停 / 资源清理状态尚未确认 / 当前
  会话暂不接受新消息”，输入与发送禁用并提供“结束本回合”。连续 10 秒 target **44,480→44,480**、
  fixture model calls **5→5**、events **70→70**、input effects **1→1**；无第二次 dispatch、模型调用或
  输入。产品按钮结束回合后 head ready、Turn cancelled（seq 71），unknown receipt 仍保留；最终 DB
  `ok`，target/app/fixture 及 `57744/60965/57261` listener 均为 0。
- 定向对照：startup pending-external-effect quarantine **1/1**、暂停 UI **2/2**、fixture build、fmt 与
  diff 通过。没有发现新的共享产品根因，故不改 `PROGRESS-SHARED`；完整证据在
  `2026-09-30/macos/m04-computer-crash/`。本批关闭 `COMP-010` 的 macOS owner crash/result-loss
  子断言；`COMP-012` 真实 held cancel/key hold、其他手势/显示器/OCR 与 `COMP-014` soak 仍开放，
  不关闭完整 Computer、D06 或 M04。

- M04-13 后续 UI 审计修正上述“无共享根因”结论：同一正式截图虽证明主会话页暂停与发送禁用，侧栏
  悬浮卡却仍显示“活跃状态 运行中”。公共根因及修复记为 `S-D02-22`：startup quarantine 成功后向
  精确 owner 补发 canonical `turn.paused`，让既有列表订阅重读 durable Session；不改变
  `COMP-010` 的 unknown/零重放结论。集成回归 **1/1**、暂停/HoverCard/列表 refresh **4/4**。
  post-fix Developer ID-signed arm64 App（executable SHA-256
  `ec6caacdba1a81c42ab5acbbceba0b5f4d81237a334d6ffa016d709a92d40eb5`）从原 seq-67 crash image
  冷启动；复制旧 work-root marker 的首次夹具启动被产品 fail closed，纠正隔离绑定后再次得到
  paused/unknown，且 `turn.paused` 后出现新的 `/api/agent-sessions` canonical refresh。解锁后同一
  正式 UI 的详情页仍显示暂停/发送禁用，侧栏悬浮卡精确显示“活跃状态 执行已暂停”；产品按钮结束
  回合后 head ready、seq 71 cancelled，unknown receipt 保留。最终 DB `ok`，App 与 `63143` listener
  为 0。证据在 `m04-computer-crash/run-009～011-*`；该 OBS 冷启动投影子断言通过。

- **M04-14 macOS Unicode direct text input**（`COMP-004` 的 Unicode scalar、组合字符、emoji 与零
  shortcut 副作用子断言）：新增最小 `--computer-unicode-input` fixture 与仓库外 Developer ID-signed
  arm64 AppKit 文本目标；目标以 exact content、UTF-8 bytes、Unicode scalar、UTF-16 units、文本变化、
  modifier/shortcut 事件作独立 oracle。输入固定为 precomposed `é`、decomposed `e + U+0301`、中文、
  かな、普通/ZWJ+肤色 emoji 及非 BMP 音符的混合串，不切换系统输入法或键盘布局。
- 首个 data setup 因 fixture 漏编 `computer-use` feature 而被 `CAPABILITY_NOT_MATERIALIZED` 拒绝，零
  模型/目标副作用，保留于 `run-002-formal/`。feature-correct 首次正式 Turn 已物理输入正确内容，但
  fixture 错把 A11y 对不可见 U+0301/U+200D 的可审计 `\\u{...}` 展示转义当作内容丢失，Turn 以
  `EXECUTION_MODEL_INVALID_REQUEST` 暂停；target 此时已精确记录 **58 UTF-8 bytes / 31 scalars /
  36 UTF-16 units / 2 changes / 0 modifier / 0 shortcut**，失败完整保留于 `run-004-formal/`。修复仅把
  A11y 断言改为精确匹配其既有转义合同，原始 target content/三种长度与零副作用断言不放宽。
- 最终 `run-006-formal`（Session `01a0f016-f09d-7922-94cc-0372b8044a6e`）为 **9 model steps /
  113 events**；唯一 launch/input effect 各 returned，input args 保留原始 scalar，最终 A11y 结果保留
  `\\u{301}` / `\\u{200d}`，target exact content 与三种长度全部一致、modifier/shortcut 仍为 0。
  Turn completed，正式 UI 明确完成，运行中及停止后 DB `ok`，target/app/fixture 与 `53368/53299`
  listener 均为 0。fixture 双 feature build、fmt/diff 通过；完整证据：
  `2026-09-30/macos/m04-computer-unicode/`。
- 本批未发现共享产品代码根因，不改 `PROGRESS-SHARED`。真实 IME composition/候选窗、切换输入源、
  Dvorak/其他 layout、dead key 与 shortcut 组合仍开放；因此只关闭 `COMP-004` 的 macOS Unicode 直接
  文本子断言，不关闭完整 `COMP-004`、Computer、D06 或 M04。

- **M04-15 macOS large Accessibility tree**（`COMP-013` 的 node-budget/incomplete 子断言）：
  新增 `--computer-large-a11y` fixture、仓库外 Developer ID-signed arm64
  AppKit 目标及最小组件回归。独立目标实际创建 **200** 个可操作按钮，记录首项
  `AX_ITEM_000` 与尾部哨兵 `AX_OMITTED_SENTINEL_199`；current-source signed Tauri 的 canonical
  `computer/a11y.observe` 只返回 **120 elements / 3,409 chars**，明确包含
  `a11y tree truncated to the node budget` 与首项，不包含已由 target 证明存在的尾部哨兵，且零 pixels。
- 正式 Session `01a0f029-0ec1-7442-87a7-132f0e7a2867` 已 completed，**8 model steps / 100 events**，
  唯一 launch effect returned；A11y read 不制造 effect。fixture `large_a11y_verified=true`、failure=null，
  运行中 DB `ok`。新增组件回归及 `nomi-computer` 全组 **100 passed / 7 ignored**，同时证明 ref 120
  可用、ref 121 不存在。解锁后正式 UI 明确“结果标记 incomplete，不能把省略节点当作不存在”；原生
  目标尾部截图直接显示被 canonical snapshot 省略的 sentinel。最终 DB `ok`、SHA-256
  `0b8029c1d01e0b03306457969516ca1bb0f02b830f65676e37ea3dcec9a0cbd3`，target/app/fixture 与
  `55520/55439` listener 均为 0。证据在 `2026-09-30/macos/m04-computer-large-a11y/`。本批关闭
  `COMP-013` 的 macOS A11y node-budget/incomplete 子断言；OCR-large、截图之外的 pixel-only 目标及
  真实模型如何使用 incomplete 仍开放，不关闭完整 `COMP-013`、Computer、D06 或 M04。

- **M04-16 macOS Computer 100-cycle soak**（`COMP-014`）：新增 `--computer-soak` 确定性 fixture 与
  仓库外 Developer ID-signed arm64 AppKit 文本目标；固定执行 100 组 fresh A11y observe → 单字符
  type，每组 call ID 和 Effect operation ID 唯一。目标逐轮核对完整前缀，并独立记录 content、change
  events、modifier/shortcut 与 window focus-loss；任何重复、遗漏、陈旧 generation 或焦点漂移均当轮
  fail，不等最终状态掩盖。
- 首次 fixture build 因既有 `/status` 单个 `json!` 已逼近宏递归上限，新增字段后在编译期失败；完整
  保留于 `m04-computer-soak/run-001-fixture-build/first-failure.log`。修复把同一顶层 status JSON 分成
  三段 Map 合并，字段和 wire 形状不变，未提高 recursion limit 或删减断言；修复后双 feature build
  通过。
- 最终 `run-003-formal`（Session `01a0f052-59c5-7a72-80dd-a0d078912552`）由 current-source
  Developer ID-signed Tauri 执行：fixture **210 model calls**，canonical **207 model steps / 2,510
  events**；其中 3 次 compaction，仍精确保留 **100 distinct observe + 100 distinct input**，严格
  `observe_000→input_000 … observe_099→input_099`，order/index violation 均 0。唯一 launch effect 与
  100 个 input effects 全 returned，101 started/succeeded、0 failed/uncertain/pending/unknown。
- 独立 target 最终为 exact 100-byte/100-scalar 模式串、**100 change events / 0 modifier / 0 shortcut /
  0 focus loss**；Turn completed，正式 UI 明确 100 轮唯一结算且文本/焦点/清理稳定。运行中与停止后
  DB 均 `ok`，最终 snapshot SHA-256
  `aa8446bbe4dfd15f2ef83a9116b940a4b8c1f8de71791599dcca5ea86b051ce2`，target/app/fixture 与
  `59822/59744` listener 均为 0。证据：`2026-09-30/macos/m04-computer-soak/`。
- 本批未发现共享产品根因，不改 `PROGRESS-SHARED`。该 macOS `COMP-014` 子断言通过；更长 LONG/
  99% 统计、真实模型、并发用户干预、sleep/wake 与 release 构建仍按各自 Case 开放，不关闭完整
  Computer、D06、D09 或 M04。

- **M04-17 macOS raw click variants**（`COMP-003` 的 right/middle/double/triple 扩展子断言）：新增
  `--computer-click-variants` fixture 与仓库外 Developer ID-signed arm64 AppKit 原始事件目标；四个
  手势使用分离坐标，每次前均取得 fresh `computer/observe` screenshot generation，独立 target 按
  event type、global screen point、down 次数及最大 clickCount 判定，不以 tool returned 代替命中。
- 原生 target 首次 Swift `@main` build 漏 `-parse-as-library`，在生成 app 前失败并单独保留。首个正式
  `run-002-formal` 的 right-click effect 已 returned，映射坐标 `(177,719)→screen (289,1174)` 与 target
  `(290,1175)` 只差 1 px，但只装 local monitor 的 oracle 记录 0；Turn 暂停，完整 DB/status 保留，未改记
  PASS。新 target 复用 M04-09 已验证的 local/global 双 monitor，并按 type+timestamp+button+clickCount
  去重，以全局屏幕坐标判命中；产品 input 实现不改。
- 最终 `run-004-formal`（Session `01a0f069-d7f4-7190-af03-cd2cea2bb7eb`）为 **15 model steps /
  188 events**；4 次 fresh screenshot 与 4 次 input call 均唯一，launch + 4 个 input effects 全 returned，
  无 pending/unknown/retry。target 精确记录 right **1**、middle **1**、double **2 / max clickCount 2**、
  triple **3 / max clickCount 3**、unexpected **0**；正式 UI 明确原生命中与唯一结算。运行中及停止后
  DB `ok`，最终 SHA-256 `de64ff97b44a083b1a94645d002bc8f64aef9daf0dfb7eea9e00c01ef4047f7f`，
  target/app/fixture 与 `62477/62404` listener 为 0。证据：
  `2026-09-30/macos/m04-computer-click-variants/`。
- 当前宿主只有一个 online 主显示器（Q2790PQ，2560×1440，mirror off），因此不以本批截图缩放冒充
  `COMP-005` 多显示器/不同 scale PASS；该 live Case 明确阻断。本批无共享产品根因，不改
  `PROGRESS-SHARED`；关闭上述 macOS click-variants 子断言，held cancel、IME/layout、multi-display
  与 release 仍开放，不关闭完整 Computer、D06 或 M04。

- **M04-18 macOS packaged CEF dynamic signing prerequisite**（`BROW-017`、`MAC-017` 的正式
  product-bundle 签名/Seatbelt 前置子断言）：current-source arm64 host、固定 CEF 152.0.6 / Chromium
  152.0.7977.83 与五类 Helper 首次经 Developer ID 深度签名后，路径级及运行中
  `codesign --verify` 均通过，Helper 也实际进入 Seatbelt；但两个独立启动都在约三分钟后的 Chromium
  on-device utility 初始化时稳定报告 `errSecCSInfoPlistFailed (-67030)`。首次正式 UI 因 Mac 锁屏保持
  **0 turn / 0 effect / 0 model call** 后清理，未用 API 提交冒充 UI PASS；签名首败与锁屏运行分别保留于
  `2026-09-30/macos/m04-browser-packaged/run-002-formal/`、`run-003-signature-diagnostic/`。
- 根因是 Chromium 动态 peer 校验会把 outer bundle 的内存 `Info.plist` 重新序列化为 canonical XML，
  而原主包和脚本生成的 Helper plist 字节并非该序列化；因此普通磁盘/动态 `codesign` 可通过，带
  canonical plist bytes 的 Chromium 校验仍 fail closed。staging 现于签名前用系统 `plutil` 将主包和
  五个 Helper 的 plist 规范化为 XML，不改签名身份、entitlement、runtime hardening 或 CEF 校验。
- 修复后的独立 bundle（host executable SHA-256
  `4e4c4bcc50f141cdf39950a7b0d6ad5301bd2fd6ebccb0195af33a43a14d788f`）六份 plist 均与再次
  canonical roundtrip 逐字节相同，deep/strict 签名通过；同等 on-device utility 触发后
  `process_requirement/-67030` 为 **0**，主进程仍 dynamically valid，五类 Helper 保持 sandboxed。
  packaging contract **6/6**，Browser platform boundary 通过，停止后 App/Helper/listener 为 0、DB
  `ok`。证据：`2026-09-30/macos/m04-browser-packaged/run-004-fixed-product/`、
  `run-005-fixed-signature-soak/`。
- 本批为 macOS packaging 根因，不改 `PROGRESS-SHARED`。正式 packaged Browser AgentSession 的
  navigate/Unicode type/trusted click witness 仍须在解锁后以新隔离 UI 运行；Tauri 窗口关闭/重开、
  新 surface identity 与旧 frame/session fence 也仍开放，因此只关闭本签名/Seatbelt 前置子断言，
  不关闭完整 `BROW-017`、`MAC-017`、D06 或 M04。

- **M04-19 macOS native CEF 100-cycle soak**（`BROW-018` 的本批平台子断言）：在 Mac 仍锁屏、正式
  packaged UI 不可操作期间新增独立 `--soak-only` native runner，不用 API 提交冒充 UI。Developer
  ID-signed arm64 Tauri/CEF bundle 在同一 ephemeral runtime、同一 tab 上执行固定 100 轮
  navigate → readiness → fresh semantic observe → Unicode type → fresh observe → native click → final observe；
  下一轮还必须以精确 `StaleTarget` 拒绝上一 document 的未消费 ref。
- 首次 compile 漏引 `BrowserRuntimeFactory`；前三次正式运行又依次暴露 runner 把隔离 evaluation world
  当页面 world、以页面全局变量作 oracle，以及把跨 document fence 错写为 `StaleObservation`。产品均
  fail closed，runtime close / engine shutdown 正常；失败完整保留于
  `2026-09-30/macos/m04-browser-soak/run-001-build/`、`run-002-formal/`、`run-003-formal/`、
  `run-004-formal/`。修复后的 oracle 只从隔离 world 读取页面事件监听器写入的 DOM `data-*`，不穿透
  world；跨 document 则精确要求 `StaleTarget` 99 次，未放宽为多个可接受错误。
- 最终 exact-source `run-007-final-source` 完成 **100/100**，document generation **3→102**、旧 ref
  `StaleTarget` **99/99**；每轮页面独立证明 exact `soak-NNN-中文`、click count 1 与
  `event.isTrusted=true`。全过程始终 1 runtime / 1 tab，零 download/dialog/permission 残留、零 cycle
  error；首 20 轮 median/p95 **311/504 ms**，末 20 轮 **316/524 ms**，最大 **524 ms**，无随序号
  退化。全部 Helper 在第 10 轮前已启动，之后无新增；`shutdown_complete=true`，最终 App/Helper 为 0，
  deep/strict 签名及六份 plist canonical roundtrip 通过。
- 本批只扩展确定性 runner/页面 oracle，未发现新的产品或共享根因，故不改 `PROGRESS-SHARED`。
  runner 仍明确 `productAcceptance=false`；正式 packaged AgentSession UI、真实模型、Tauri 窗口重开
  identity/fence、睡眠唤醒及更长 LONG/99% 门槛仍开放，因此只关闭 `BROW-018` 的本批 macOS native
  100-cycle 子断言，不关闭完整 Browser、D06/D09 或 M04/M06。

- **M04-20 macOS native CEF Tauri window close/reopen**（`BROW-017`、`MAC-017` 的本批平台
  lifecycle 子断言）：新增独立 `--window-reopen-only` runner；隐藏 guard window 只用于在真实 main
  window 销毁期间保留 Tauri event loop，不承载 Browser surface。旧 main window 上创建 runtime
  generation 51 / document generation 2，取得 fresh semantic ref 后用原生 `confirm` 留下 owned pending
  work，再真正关闭该 NSWindow。
- 关闭后 cancel/finish 精确 drain pending dialog，旧 runtime 随后 close；旧 snapshot 与旧 ref action 均
  精确返回 `WorkspaceClosed`。以同一 `main` label 新建 Tauri window 时 NSWindow identity 不复用；新
  runtime generation 52 / 新 tab identity 均唯一，旧 target 对新 runtime 精确返回 `TabNotFound`。新
  surface 随后以 `reopened-中文` 和页面 DOM oracle 证明 click count 1、`event.isTrusted=true`。
- 首次 build 的 cargo check 实际通过，但外部 `tee` 目录尚未创建导致证据捕获命令非零，已单独保留且
  未冒充产品失败。正式 `run-002-formal` 首轮即全部通过，`shutdown_complete=true`；Developer ID
  deep/strict 签名通过，无 `process_requirement/-67030`，最终 App/Helper 为 0。证据：
  `2026-09-30/macos/m04-browser-window-reopen/`。
- 本批未发现产品或共享根因，不改 `PROGRESS-SHARED`。runner 仍明确 `productAcceptance=false`，且只
  覆盖 native Tauri/CEF window、runtime/tab/ref 与 pending-dialog owner；正式 packaged NomiFun
  AgentSession UI、close-to-tray renderer attach/detach、nested frame 及真实模型仍须单独验证。因此只
  关闭 `BROW-017/MAC-017` 的本批平台 lifecycle 子断言，不关闭完整 Browser、D06 或 M04。

- **M04-21 formal packaged NomiFun Browser AgentSession**（`BROW-001/003/015/017`、`MAC-017` 的
  本批正式产品子断言）：用户解锁后以 current-source Developer ID-signed arm64 NomiFun、固定 CEF
  152.0.6 / Chromium 152.0.7977.83、fresh data/Agent/Session 和正式 Tauri UI 提交
  `Verify the packaged native Browser surface.`。此前 `run-002` 锁屏运行保持零 Turn；本次首个
  `run-008-formal-ui` 已真实 navigate/type/click 并收到 trusted witness，但 fixture 的
  `report_completion` 漏必填 `observed_tool_error_count=1` 且发送空 `requirement_ids`，产品正确拒绝；
  最终 Turn failed，完整 DB/UI/status 保留，未改记 PASS。
- 同一首败还暴露正式产品的 macOS CEF profile 根因：CEF 要求 disk profile 是
  `root_cache_path` 的直接子目录；产品 root 为 `browser-v3`，canonical Session profile 却在
  `browser-v3/agent-sessions/<hash>`，CEF 因此明确记录 `Cannot create profile` 并退回 OffTheRecord。
  修复只把 macOS CEF root 移到既有 canonical profile 父目录 `browser-v3/agent-sessions`；共享
  `BrowserProfileStore`、hash identity、清理边界及 Windows 路径不变。fixture completion 另补精确累计
  tool-error count、在 summary 公开披露，并省略空 requirement 数组。
- 最终 `run-011-formal-ui`（Session `01a0f0b1-7e6d-7113-aead-5a2b099c5457`，host executable
  SHA-256 `ce1890564f853902224f83707c3afa58490885844393a385dd8d073d633a9622`）为
  **10 model steps / 148 events**；2 次 navigate、Unicode type、trusted click 四个唯一 Effect 全
  returned，零 pending/unknown。页面独立 witness 为 count 1 / exact `Agent 主界面真实输入` /
  `trusted=true`；正式 UI 同时显示计数 1、文本和“已收到真实点击”，关闭 Browser panel 后显示完成总结
  及累计 **1** 个 plan-guard tool error。
- canonical completion 为 plan revision 1 / observation revision 9，两个 criterion 均由 final observe
  `gui-native-7` 支持并覆盖 `input_0`；`turn/completed`、head ready。唯一 64-hex Session profile 已在
  新 root 直接落盘，冷读 `History` 为 `ok` 且只在该 profile 记录 loopback URL（visit count 2），global
  Default 无该 URL；App 日志不再出现 profile/CEF/process-requirement 错误。backend DB `ok`、inline PNG
  0，最终 App/Helper/fixture/listener 为 0。完整证据：
  `2026-09-30/macos/m04-browser-packaged/run-008-formal-ui/`、`run-009-fix-build/`、
  `run-010-fixed-product/`、`run-011-formal-ui/`。
- 该 profile 根因是 macOS CEF host 专属，未改共享层，故不改 `PROGRESS-SHARED`。本批关闭正式 packaged
  Browser navigation/input/trusted-page-result 与精确 persistent profile 子断言；真实 StepFun、产品
  close-to-tray renderer attach/detach、nested frame/popup 的正式 UI 以及 release/notarized artifact 仍开放，
  不关闭完整 `BROW-017/MAC-017`、D06 或 M04。

- **M04-22 formal close-to-tray Browser lifecycle**（`BROW-017`、`MAC-017`）：使用另一个 fresh
  data/Agent/Session `01a0f0bb-b05f-7442-b59f-2fb84bb4ce92`，由正式 signed Tauri UI 提交相同 Browser
  Turn；native child 已显示页面且 Agent 仍为 running 时点击 macOS 主窗口关闭按钮。产品按既有合同
  隐藏到 tray 而不退出：CUA inventory 明确 App 仍 running，隐藏期间 Turn 到达 completed/head ready，
  loopback witness 为 count 1 / exact Unicode text / `trusted=true`。
- 当前 macOS CUA 没有 `computer.launch_app` 函数，该首次 runner 尝试原样失败且未影响 App；随后按
  支持的绝对 bundle path 重新取得同一 running App，正式主窗口恢复。重开后 Browser page 精确保留原
  URL、计数 1、`Agent 主界面真实输入` 和“已收到真实点击”，没有新模型调用或 Effect。随后通过正式
  Session tool 关闭/重开 Browser panel，renderer detach/reattach 后仍只显示一个 tab 与同一页面状态。
- reattach 后 0 秒与 5 秒两次独立核对均为 **10 model calls / 148 events / 4 returned Effects / 0
  unsettled / 1 witness**，无重放或重复 surface；App 日志无 profile/CEF/process-requirement 错误。停止后
  backend DB 与 Session profile History 均 `ok`，目标 URL 仍只在绑定 profile，最终
  App/Helper/fixture/listener 为 0。完整证据：
  `2026-09-30/macos/m04-browser-close-to-tray/run-001-formal/`。
- 本批未发现产品或共享根因，只提交本页结论。结合 M04-20 的真正 Tauri window 销毁/重建、唯一
  NSWindow/runtime/tab identity、旧 target/ref fence 与 pending-dialog drain，以及 M04-21 的正式
  packaged AgentSession/profile，`BROW-017` 与 `MAC-017` 的 macOS 功能验收槽现为 **PASS**。这不关闭
  其他 Browser Case、完整 D06/M04；真实 StepFun、nested frame/popup 正式 UI、release/notarization 与
  LONG/99% 仍按各自槽开放。

- **M04-23 real StepFun Browser + Workspace functional chain**（真实 Provider 的本批单次预算冻结
  子断言）：只读本机 `NomiFun-dev` 已加密 Provider，确认 exact `stepfun-plan` endpoint 与
  `step-3.7-flash`；一次性仓库外 launcher 在内存 AES-GCM 解密后只把 key 放入既有 live wrapper 环境，
  Cargo/argv/App/fixture 文件和日志均不接收明文。`run-001` 先选到无 credential 的 production data，
  在任何请求前 fail closed；`run-002` 首次 App 又因未传 exact `NOMIFUN_WORK_DIR` 被 work-root receipt
  拒绝，均原样保留。
- 修正 App 启动后，`run-002` 的真实模型仍把 Workspace 看成空目录并在 29 calls 时由 UI 停止。DB
  证明 Session binding 已准确冻结 `work` root；根因是 fixture 把 `app.js` 放在上层 `work/`，而当前
  `default-workspace` 合同正确物理化为 `work/conversations/<session_id>`。fixture 现只在 exact managed
  Session child 创建源码，HTTP `/app.js` 也读取同一路径；新增原生路径回归覆盖 macOS `/var` →
  `/private/var` canonical alias，且 root-level `app.js` 必须不存在。
- 最终 `run-004` 由正式 signed Tauri UI 提交 exact loopback URL 与 repair 任务，真实
  `step-3.7-flash` 得到 trusted witnesses **[2, 2, 1]**，`changed_source_served=true`，最终绑定源码为
  exact `function nextCount(value) { return value + 1; }`。第二个 2 是模型在 patch 成功前过早 reload/click，
  未从历史中删除；一次 patch payload 被精确 rejected，随后一次 patch returned。Browser effects 为
  navigate **7 returned**、act **3 returned**，最终零 pending/unknown、inline PNG 0。
- 本轮冻结上限为最多 32 次真实 Provider 请求；Runtime 在 **30 model steps / 635 events** 后因 fixture
  call cap 投影 `EXECUTION_MODEL_RATE_LIMITED`，`host_cleanup_proven=true`、head paused。功能链完成但
  未提交 canonical completion，因此不记 terminal PASS；正式 UI “结束本回合”后 Turn cancelled/head
  ready，最终 DB `ok`。wrapper 被 Ctrl-C 后两个 detached fixture 曾各保留唯一 listener，均在核对
  exact PID/executable/argv 后定向 TERM 并清零；App/Helper/listener 最终为 0。
- 凭据审计扫描本批 **869** 个仓库外文件、Git diff 与进程 argv，exact credential match 为 0；不提交
  launcher、DB、日志或凭据。完整证据：`2026-09-30/macos/m04-browser-live-stepfun/`。本批只修 fixture
  的 exact Session source placement，不改 `PROGRESS-SHARED`；真实 Provider 功能链子断言通过，但
  terminal、N3/20/99% 与效率门槛保持开放。暂停 UI 同时把 canonical `cleanup_proven=true` 错显示为
  “资源清理状态尚未确认”，作为下一公共 D02 问题簇处理，不在本条冒充已修复。

- **M04-24 pause cleanup projection follow-up**（`OBS-006/007/014/015`、`LIFE-015/016` 的本批
  macOS 来源/共享前端子断言）：复用 M04-23 首次正式 UI、DB 与事件，不重写原结果。canonical pause
  已明确 `cleanup_proven=true`，错误文案来自 `turn.paused` 后首个 authority GET 的短暂旧投影，而非
  macOS host cleanup 失败；共享修复及边界记为 `S-D02-23`。
- 新确定性回归证明首个未证明 pause 后的 cleanup-proven snapshot 会胜出，同时证明连续两次相同未证明
  pause 仍保留 `cleanupRequired`，没有靠吞错或放宽断言制造通过。相关 reconcile/hook/notice
  **18/18**、UI typecheck、desktop UI boundary 与 diff check 通过；首次错误测试入口产生的 2 个 DOM
  harness 失败和修复后日志分别保存在
  `2026-09-30/macos/m04-pause-cleanup-projection/run-001-tests/`、`run-002-tests/`。
- 为冻结真实模型预算，本批没有再次调用 StepFun，也没有 post-fix 正式 Tauri 截图，故只记公共竞态修复
  与确定性验证；M04-23 terminal、真实 Provider N3/20/99%、完整暂停/断线 UI 矩阵仍开放。

- **M04-25 native drag cancellation/release**（`COMP-012` 的有界 drag 期间真实取消子断言）：
  macOS 26.6.2 / arm64 / APFS 上复用 Developer ID-signed 正式 Tauri App
  `ce1890564f853902224f83707c3afa58490885844393a385dd8d073d633a9622`；相关 Computer/cancel/retained
  effect 源码与当前分支无 diff。前两次 runner 分别缺 Browser feature、Computer host materialization，
  在 Turn 前 fail closed；第三次中断仅有 3 条 Session 初始化事件，首次失败均保留。
- fresh Session `01a0f112-5e48-7d80-bf2f-be65e77ea170` 由正式会话 UI 提交任务。仓库外监视器只读隔离
  owner 的既有签名凭据，在内存生成短期 JWT，经原鉴权向产品 `/turns/cancel` 发送唯一请求；未认证
  对照为 403，已认证读取/取消为 200。seq 101 已取消后的 **17 ms** 独立采样仍为 pressed；目标唯一
  mouseDown/mouseUp、7 个 drag 事件，最终释放，实测按住 **106.6 ms**。input effect returned，seq 105
  `host_cleanup_proven`，8 model steps / 106 events，零 pending/unknown；正式 UI 显示“已取消执行”
  并保留真实拖拽结果。取消后无模型重开，退出前后 DB/备份均 `ok`。
- UI runner 在关闭目标后读取 AX 曾重新启动目标，已核对唯一新 PID 后定向 TERM；最终
  App/Helper/target/fixture 与两个 listener 为 0。普通证据 45 文件签名凭据匹配为 0，全部制品在
  `2026-09-30/macos/m04-computer-held-cancel/`。本批无产品根因/源码改动、付费请求为 0；只关闭
  有界 drag 的取消/释放子断言，长时 hold、key hold、原生 release-failure、N3 与完整 COMP-012 仍开放。

- **M06-01 native shutdown impact verification**（`PROC-040`、`LIFE-019/023/024/029`、`CONC-014`
  的本批子断言）：合入 `c8fea229f/abf6ed4d9/c3b3dc9ac` 后在 macOS arm64 / APFS 复核
  `S-D04-30/S-D09-46/47`。首次普通 shutdown **3/3**；导出 PID 证据的重跑却在长 poll 前失败：
  marker 移到仓库外，helper 仍绑定仓库 cwd，Seatbelt 正确拒写。首败保留，夹具现 canonicalize 隔离
  evidence root，并将 cwd 与唯一 capability root 同时绑定该目录；生产权限/预算与 6 秒断言不变。
- 修复后原生长 poll 首次 + 20 repeats **21/21**，独立 marker/report/PID disappearance **21/21**；
  唯一 Cancelled/reaped 唤醒原 60 秒 poll，重复 shutdown 返回原报告，helper 残留 0。DesktopServer
  正常退出/真实 SQLite writer lock 两项在前后两个源码快照均 **2/2**；4 个独立 DB 和唯一文件核对
  `ok/cancelled/head ready/one returned write/one cleanup witness`，未重放写入。当前 merged host/journal
  清理投影与未准入回执拒绝 **4/4**，Rust fmt/diff 通过。
- 全证据：`2026-09-30/macos/m06-native-shutdown/`。本批只修证据夹具根绑定，付费请求 0；没有正式
  Tauri renderer、真实满盘/fsync、跨重启/lease 过期或其他角色证据，因此不关闭完整 PROC/LIFE/M06。

- **M04-26 pause evidence beyond event page**（`OBS-006/007/014/015`、`LIFE-015/016`）：修正
  M04-24 的竞态归因；实际是前 500 事件页缺少 seq 635 的 pause，公共根因/修复见 `S-D02-24`。
  原生 pause/resume/唯一写入反例首次 field=null，修复后 **21/21**；Store exact/cursor/foreign/false
  **1/1**，相关 UI **18/18**、typecheck/desktop boundary/build 通过。初始两个测试夹具编译/事件注册
  失败、GUI ID 重用/压缩后缺历史均单独保留，未改写为产品 PASS。
- 正式 Developer ID arm64 前后样本均 pause **seq 605 / 41 model steps / 48 loopback requests**，
  4 effects 全 returned、零 pending/unknown；首败三个 projection 缺 `execution_pause`，当前源码 App
  `19634d451f452d97185a4cba3f8a6f0c8e445c881adf99700c5f3e986dcd3251` 三次均返回 exact reason/
  `cleanup_proven=true`。正式截图从“资源清理状态尚未确认”改为“任务尚未完成，已完成的操作会保留”，
  暂停时输入禁用；UI 结束回合后 head ready/Turn cancelled，输入及 API 发送能力恢复。
- macOS 26.6.2 / arm64 / APFS；deep/strict 签名、DB/备份 `ok`，最终 owned App/Helper/fixture/
  listener 0。49 份普通证据的签名凭据匹配 0，付费 Provider 调用 0。全证据：
  `2026-09-30/macos/m04-pause-event-page/`；只关闭该投影子断言，M04-23 live terminal、其他角色/
  平台、多窗口/断线、N3/99%、release/notarization 与完整 D02/M04 仍开放。

- **M06-02 native start delivery/cancel**（`PROC-039/040/042`、`CONC-004`、`LIFE-029` 的本批
  子断言）：macOS 26.6.2 / arm64 / APFS 原生复核 S-D04-31。首轮 **5/5**，首次重复却 **4/5**：
  PID marker 已证明 child 执行，drop caller 后 shutdown 空报告、返回当时 child 仍活；之后由底层 poller
  回收，未用独立 fallback。首败在 `run-002-repeats/repeat-01`，未覆盖为 PASS。固定 COMMITTED
  窗口的反例另确认外层取消返回 StartLost 并丢失 native owner；新增测试的首个缺函数限定编译错误单列。
- 公共根因/修复见 `S-D04-32`：Supervisor 已持有的 Unix worker 在 commit 后继续交付精确 owner，
  注册原 Session 的 retirement/cleanup witness，独立底层 future-drop 路径不变。修复后 Pipe 五场景
  首次 + 20 repeats **105/105**、PTY 两场景 **42/42**；独立 marker/唯一启动/报告/PID 消失
  **147/147**，fallback 0。固定窗口反例 **21/21**，独立底层 drop 两相邻 **2/2**、
  deadline-before-fork **1/1**、shutdown **8/8**、registry shutdown **3/3**，fmt/diff 通过。
- 全证据：`2026-09-30/macos/m06-start-delivery/`；本批不调用模型或正式 Tauri，不改 Windows 原
  结果。native 失败/unknown、worker panic、失去 ownership、更多 descendants/角色及完整
  PROC/CONC/LIFE、N3/100 seed/LONG/99% 仍开放。

- **M06-03 committed start IO failure**（`PROC-039/040/042/047`、`CONC-004`、`LIFE-029`）：
  macOS arm64 / APFS 的固定故障窗口已让真实 child 执行，随后注入 stdio wrap failure；首败
  shutdown report 为空、返回时 PID 尚未回收，之后 poller 才完成清理。完整首败保留于
  `run-001-first-failure`；首次修复编译漏 Unavailable Drop 分支、重复第 5 轮读到未写完 PID 行均单列。
- 公共根因/修复见 `S-D04-33`：启动错误携带 native owner 回到 Supervisor，先注册原 Session，
  按原 retirement/清理预算完成或保留真实未决状态；普通 start 返回原 StartLost/code/PID，并发
  shutdown 维持取消优先并保留原 owner 清理报告。
  测试只接收换行结尾的完整 PID 行，原 2 秒发布/6 秒回收上界不变，未加 sleep 或放宽断言。
- Pipe/PTY × 普通 start error/并发 shutdown 四场景，最终首次 + 20 repeats **84/84**；
  独立 marker/报告/PID 消失 **84/84**，错误从未改写为成功，首败与 marker-failure PID 亦已消失。
  process Runtime lib **149/149**、session registry **13/13**、Rust fmt/diff 通过，付费调用 0。
  证据 `2026-09-30/macos/m06-start-failure/`。本批只有原生组件证据；setup deadline/commit handshake
  失败、worker panic、清理失败与未知恢复、更多角色/descendants、正式 Tauri 及完整 Case/LONG 仍开放。

- **M06-04 startup deadline ownership**（`PROC-039/040/047`、`CONC-004`、`LIFE-029` 的本批
  子断言）：macOS arm64 / APFS 固定原生 COMMITTED 后的 worker 窗口。首败 caller 在约 105 ms
  收到 StartLost，但 worker 仍被持有时 shutdown 已空报告返回，PID 未回收；原 worker 后续清理，
  首败日志/JSON/PID 保留于 `run-001-first-failure`。
- 公共根因/修复见 `S-D04-34`：setup deadline 通知只结束 caller 等待，Supervisor 原 worker 继续
  持有 JoinHandle/预留/准入租约；迟到 commit 只交原 owner 做 retirement，不能成为成功 start。
  standalone deadline/drop 合同和原 setup budget 不变，未开始的原 worker 放行后仍不得 fork。
- Pipe/PTY committed shutdown、PTY quiesce、Pipe pre-fork 四场景首次 + 20 repeats **84/84**；
  独立 PID **63/63**、零 fork/零 marker **21/21**，100 ms setup budget 下 caller 最大 **105 ms**，
  原 caller 350 ms/cleanup 6 秒断言未放宽。原生报告在 worker live 时均未完成，最后准确回收。
  最终 Runtime lib **153/153**、session registry **13/13**、fmt/diff 通过，付费模型调用 0。
  中间 raw PTY watchdog-after-COMMITTED 检查曾 PeerClosed（152/153），首败保留在 `run-007`，
  该独立注入窗口在本批仍开放，未用最终快照通过关闭它；接续根因与结果见 M06-05。
- 全证据 `2026-09-30/macos/m06-start-deadline/`；本批只有组件/原生证据，未跑正式 Tauri、Linux/
  Windows 原生；commit handshake 失败、worker panic、清理失败/unknown recovery、更多角色/
  descendants、完整 N3/100 seed/LONG/99% 与全部阶段门槛继续开放。

- **M06-05 macOS watchdog fork initialization**（`PROC-041/046/047` 的 native watchdog
  异常、精确回收与未开始 deadline 子断言）：宿主 macOS 26.6.2 / 原生 arm64 / APFS，付费调用 0。
  旧 raw PTY PeerClosed 单跑及 20 repeats 均通过，未据此覆盖首败；同时间系统 crash report
  证明 watchdog PID 13729 在 `fork → libSystem_atfork_child → _notify_fork_child` 内因
  `os_once_t is corrupt` 被终止，尚未进入 BootReady/COMMITTED。原日志和 crash 另存保留。
- 公共合同修复见 `S-D04-35`：两个 macOS watchdog 启动入口均在宿主、fork 前完成 libnotify
  无注册副作用的初始化，等待仍消耗原 setup budget；耗尽后零 watchdog/user fork。确定性夹具
  只固定初始化未完成窗口，不篡改私有系统锁；Pipe/PTY 修复前 **0/2**，先完成原 owner 清理再失败。
- 修复后初始化等待/预算耗尽四场景首次 + 20 repeats **84/84**；pipe/PTY COMMITTED 后
  watchdog 失效 **42/42**，其中历史 PTY 用例 **21/21**，仍为 lifecycle failure、精确 SIGKILL
  和 leader/watchdog 各一次 reap。独立进程探测 **126/126** PID 消失，零 fork 窗口 **42/42**，
  100 ms setup 下 caller 最大 **103 ms**，原 350 ms/cleanup 3 秒上界未放宽；Runtime **157/157**、
  registry **13/13**、process boundary、定向 fmt/diff 通过。证据 `2026-09-30/macos/m06-pty-watchdog/`。
- 仅收敛该首次失败的 macOS 通知库初始化根因；未验证其他系统 fork callback、真实 Tauri 强退/
  sleep-wake、失控 descendants、worker panic/unknown recovery、其他平台/角色、release 与
  N3/100 seed/LONG/99%。不关闭完整 PROC/M06 或阶段二、三，Windows 原结果未改写。

- **M06-06 executed start / commit failure**（`PROC-040/050` 的 startup error、shutdown 责任和
  清理重试子断言）：macOS 26.6.2 / 原生 arm64 / APFS，付费调用 0。PID marker 先证明 user exec，
  再固定 COMMIT/pre-COMMITTED watchdog 退出及 cleanup hold。首败 start/shutdown 提前完成，
  原 owner 不在报告、PID 尚未回收；随后底层清理完成。首败 JSON/log 保留于 `run-001`。
- 公共根因/修复见 `S-D04-36`：Supervisor 的原 start worker 接回带原握手错误的 cleanup-only
  owner，注册原 Session 后等待真实 group/leader/watchdog 回收；不发布可用 handle，不误报
  user-code-not-started。普通 start 保留原 code/PID/清理证明，并发 shutdown 保留取消优先和报告。
  独立平台 raw/drop 路径不变，无精确 group anchor 仍不能生成成功清理证明。
- 强化收据检查另两次 **2/4** 暴露正常退出等待覆盖了先前 cleanup 诊断；`run-008/009` 原样
  保留。现正常等待不覆盖实际诊断，原握手错误及清理未证明事实均写入最终收据，未放宽断言。
- Pipe/PTY × 普通错误/并发 shutdown 最终首次 + 20 repeats **84/84**；独立 marker/错误/报告
  **84/84**、物理 PID 消失 **168/168**，leader/watchdog 均恰好 reap 一次，fallback 0。
  外部 anchor 负向 **1/1**、Runtime **162/162**、registry **13/13**、boundary/fmt/diff 通过；
  原 2 秒 marker/6 秒收敛上界不变。全证据 `2026-09-30/macos/m06-commit-failure/`。
- 本批只有原生组件证据；caller drop 与握手失败组合当批未验，native 接续见 M06-07；pre-exec deferred cleanup、其他 owner
  transfer failure、worker panic/未知恢复、正式 Tauri/其他平台角色及完整 Case/LONG/99% 仍开放。
  Windows 原结果未改写，不关闭完整 M06 或阶段二、三。

- **M06-07 dropped caller / commit failure**（`PROC-039/042`、`CONC-004` 的底层 start/cancel
  与容量/fence 子断言）：macOS 26.6.2 / 原生 arm64 / APFS，付费调用 0。真实进程先写唯一 PID/
  starts marker，在 COMMIT 前 abort caller；watchdog 固定退出于 COMMIT/pre-COMMITTED，清理
  未证明窗口被持有。Pipe/PTY × 无 shutdown/quiesce fence 四项首次通过，无新增产品缺陷；仅补
  最小回归，复核 `S-D04-36` 修复影响，首次证据 `run-001/002` 保留。
- 首次 + 20 repeats **84/84**；独立磁盘证明确实只启动原/后续各一次 **168/168**，capacity probe
  零 dispatch **84/84**，原 owner 报告 **84/84**、物理 PID 消失 **336/336**，各 leader/watchdog
  恰好 reap 一次，fallback 0。无 shutdown/lease 清理 **42/42**；fence 在清理未完成时 pending
  **42/42**；原报错 `ownership_commit_failed` 不丢，清理后同一 Supervisor 容量/准入可复用
  **84/84**。Runtime **166/166**、registry **13/13**、boundary/fmt/diff 通过，原 marker 2 秒/
  cleanup 6 秒界限不变。证据 `2026-09-30/macos/m06-dropped-commit/`。
- 本批是 native process 组件，不替代真实 Engine Turn cancel、正式 Tauri/角色或完整 Case；
  pre-exec deferred cleanup、其他 transfer failure、worker panic/未知恢复、其他平台、LONG/99%
  仍开放。Windows 原结果未改写，不关闭完整 M06 或阶段二、三。

- **M06-08 pre-exec deferred auxiliary cleanup**（`PROC-040/047/050`、A08/A11/A13 的本批
  native 子断言）：macOS 26.6.2 / 原生 arm64 / APFS，付费调用 0。WithholdAck 保证 user exec
  为零，100 ms setup 后固定 watchdog 的未证明清理窗口。首败 caller 105 ms 返回原 deadline，
  shutdown 已空报告返回而 watchdog 未 reap；随后底层回收。首败 `run-001` 保留。
- 公共根因/修复见 `S-D04-37`：原 worker/准入租约/容量持有至辅助清理证明，原 caller deadline
  不延长；失败启动单列 `startups` owner/host SessionId/error/cleanup，不制造用户 PID/可用 handle。
  non-exact 启动清理保持 quarantine 和占额，已证明记录有界且可消费；普通 Session 语义不变。
- 首次重复 **3/4** 又证明 transaction 恰先于 timer 交付时，辅助等待漏原 deadline 通知；原日志
  `run-006/repeat-01` 保留。现两阶段共享同一绝对 deadline；另保留负向夹具缺 import 编译首败。
- Pipe/PTY × shutdown/quiesce 最终首次 + 20 repeats **84/84**；独立辅助 PID 消失 **84/84**，
  user exec **0**、无伪造用户 Session **84/84**，精确启动收据/容量阻止新 exec **84/84**；每个
  watchdog reap 恰好一次，fallback 0。原 setup 100 ms/caller 350 ms 下最大 **106 ms**，原 marker
  2 秒/cleanup 6 秒上界不变。quarantine/有界报告两项 **2/2**；Runtime **172/172**、registry
  **13/13**、Engine process **16 通过/1 ignored**、boundary/fmt/diff 通过。全证据
  `2026-09-30/macos/m06-preexec-cleanup/`。
- 本批是原生组件和 Engine 定向影响检查，未验真实 Engine cancel/正式 Tauri、真实永久 authority
  loss/后续恢复、其他 transfer failure、worker panic、其他平台/角色与 LONG/99%。Windows 原
  结果未改写，不关闭完整 PROC/M06 或阶段二、三。

- **M06-09 Engine unregistered-start fence**（`PROC-039/050`、A13/A18 的 App/Engine 子断言）：
  macOS 26.6.2 / 原生 arm64 / APFS。走查发现 App 的 `unregistered_start` 永久拒绝清理，未读取
  后来的精确 native 收据；组件首败 `run-001` 已保留。公共根因/修复见 `S-D04-38`：Engine 在首个
  native await 前保存宿主 owner，App cleanup 通过其 fence 核对收据；空报告、同 invocation 的
  其他 call、未证明清理不能解除 unknown，部分精确收据保留到后续重试，panic 不确定性不清除。
- Native Engine Pipe/PTY 的已 exec、未交付 handle、caller future drop 两场景首次 + 20 repeats
  **42/42**；独立 PID marker/原 owner/物理 PID 消失 **42/42**，精确 fence 后 tracking 为零。
  App process-host **8/8**，Core process **20 通过/1 ignored**，fmt/boundary/diff 通过。
  Native 夹具缺 libc/违反既有 unsafe lint 的两次编译失败单列保留；最终使用现有安全身份探测，
  未关闭或放宽 `forbid(unsafe_code)`，无新依赖。证据 `2026-09-30/macos/m06-engine-startup-fence/`。
- 本批未额外调用外部测试模型（实际 0 次）；仅 App 组件与 Native Engine adapter 验证，未替代
  正式 Tauri/live Provider、完整真实 Turn cancel、pre-exec auxiliary 故障的 App 全链路、永久
  authority loss/恢复、worker panic、其他平台/角色或 LONG/99%。Windows 原结果未改写，完整
  PROC/M06 与阶段二、三仍开放；需要 live 的 Case 另冻结小批次调用上限。

- **M04-27 real StepFun terminal / bounded live fixture**（`ACOD-002/014`、`CTRL-006` 的接续尝试，
  `MODEL-019/027/035`、`OBS-005/007` 的本批失败投影子断言）：macOS 26.6.2 / 原生 arm64 / APFS；
  fresh 正式 Tauri dev App `ca9b51e91228…`，deep/strict ad-hoc 签名通过，不作 release/notarization 证据。
  从本机 encrypted `stepfun-plan` / `step-3.7-flash` 读取凭据，仅内存解密、stdin 交给 fixture。
  冻结 **1 Session / 最多 6 次上游请求 / 每次 1024 output tokens / 180 秒模型转发窗口**；实际
  **1 次上游尝试**在 HTTP 响应前 transport failure，随后停止转发。无认证/额度 HTTP 证据，不归因为
  凭据或余额；无凭据 HEAD 在直连及现有代理路径独立得到 TLS error，未关闭 TLS 校验或改系统代理。
- fixture 原先固定 32/4096 且统一改写上游拒绝为 502；现可显式收紧调用/输出/时间上限，剩余请求
  timeout 不超过原截止点，首次上游状态保留，拒绝后本地重试不再转发。四项护栏首次 + 20 repeats
  **84/84**，完整 example **5/5**；只改测试 fixture，不把组件结果替代 post-fix live terminal。
- 正式 UI 显示 provider unavailable 暂停、任务未完成；seq 27 `cleanup_proven=true`，UI 结束本回合后
  seq 28 cancelled/head ready。独立闭库查询：effect **0**、witness **0**、源码仍为原 `+2`、无 completed；
  DB/闭库备份均 `ok`，owned App/Helper/fixture/listener **0**。普通 readonly 收集的 SQLite 14 首败
  与空输出保留，另以 immutable 只读 URI 验证，不改原 DB。凭据 exact match **0**。
- 证据 `2026-09-30/macos/m04-live-terminal/`；该 live 正向任务为 transport 阻断，M04-23 terminal、
  真实模型效率/N3/20/99%、其他角色/平台与旧 MM retry spinner 仍开放；本批没有新公共产品根因，
  不改共享进度或 Windows 结果，不关闭完整 Case、M04 或阶段二、三。

- **MAC-A-01 command / compaction continuity**（C01/C02/C03/C06/C07/C08；`CTRL-006`、
  `ACOD-017` 等对应子断言）：用户切网后同域 HTTPS 恢复；原 transport 首败未覆盖。macOS 26.6.2 /
  原生 arm64 / APFS，官方 `coding.codex`、隔离 Session/work、现有加密 StepFun Plan / `step-3.7-flash`。
  首轮及独立修后轮各冻结 **1 Session / 16 上游请求（含摘要）/ 4096 output tokens / 360 秒窗口**。
- 首轮 `run-002` 实际 **16 requests / 687 events**，三个 command 整行字面形态被真实 owner 证明
  not_started；两个指定测试各只执行一次、exit **0/1**，读搜和 Git 已有真实结果，但三次压缩只保留
  accepted input、零原工具收据，随后重复检查。fixture cap 触发 `EXECUTION_MODEL_RATE_LIMITED` /
  `cleanup_proven=true`，不是供应商实际限流或余额证据；正式 UI 结束后 cancelled/head ready。
- 公共根因/修复见 **C07-01**：固定 instructions/schema 本身超过软字节触发点时反复压缩，另以最大
  摘要预留丢弃实际放得下的已结算交换。最小反例首败 `run-003` 保留；现只调整可压缩历史余量，并按
  实际摘要保留有界完整后缀，原 token/byte/message/调用上限及失败/freshness/authority 语义不变。
  压缩定向 **15/15**、Runtime **200/200**、fixture **5/5**、正式 Tauri build 和 deep/strict 验签通过。
- 修后 App `b366fe1f091d…` 的 `run-006` 实际 **11 requests / 403 events**，两次压缩分别保留精确
  exit-1 原收据及 plan 更新；不再重复两个测试，`report_completion` 接受、UI 披露 exit **0/1** 和历史
  `2 tool errors / 1 command failure`，canonical completed/head ready。仍有 **1 not_started**，因此
  只验证修后压缩/完成链，不记首发零错误或完整 MAC-A PASS；cwd/逐项 evidence 映射仍需独立核对。
- 两轮 **7/7** 原件 hash、原 dirty Git/HEAD 均不变，禁跑哨兵不存在，DB/备份 `ok`，凭据 exact
  match **0**；请求/SSE/事件/UI/数据库在 `2026-10-01/macos/mac-a-observe/`。修后正常 App 退出另
  **30 秒 backend cleanup timeout**，TERM 后仍活；核对 exact PID/executable 后 KILL，最终 owned
  App/Helper/fixture/listener **0**。原超时与人工清理记录保留，**不算产品清理通过**，列为 C05 接续。
  不改 Windows 结果，不扩模型 context 额度或权限；首发形态、退出等待、MAC-A N3、MAC-B/C 仍开放。

- **MAC-C05-01 native shutdown acknowledgement**（C05、`LIFE-029`、A11/A13 的本批子断言）：
  接续 MAC-A-01 的真实 30 秒退出首败，线程样本定位主线程在 `Engine::shutdown → CEF shutdown`
  等待，不据此误判仍在执行测试。macOS 26.6.2 / 原生 arm64 / APFS；原现场/样本不覆盖。
- 发现并修复独立证明漏洞：原 Engine 在调用 CEF 关闭前已设置 stopped，晚到/重试可提前返回成功。
  现分别记录 native entry 与 native completion，阻止重复原生进入及关闭后的新工作；只有 CEF 真正
  返回后才发布完成，未确认重试保持错误，不把等待超时或“开始关闭”当成功。不改变预算/系统保护。
- 精确旧语义反例首败保留在 `run-002-first-failure`；修后 pending/重复进入两项与 Mac lib
  **13/13**、Browser platform 关闭顺序/取消 **2/2** 通过。无模型调用；这是证明状态修复，尚未
  证明 MAC-A-01 的 CEF 内部等待根因已消失，不记完整 C05 或正常 UI 退出 PASS。
- 空数据正式 App 对照经产品 SIGTERM observer → Tauri `ExitRequested` 正常 exit **0**，没有
  active Session；锁屏阻断 UI Cmd-Q 验收，已请求手动解锁，信号对照不替代按钮/UI 证据。完整记录：
  `2026-10-01/macos/c05-shutdown/`。Windows/WebView2 路径未改，已完成 Session 的退出重验仍开放。

- **MAC-C05-02 context-only native isolation**（C05 的本批定位，非完整 UI 验收）：现有 signed Tauri
  CEF runner 新增互斥的 `--context-shutdown-only`，只持有一个真实 persistent request context，
  零页面/导航/模型；原 180 秒 native watchdog、260 秒 runner 上限及强制清理判 FAIL 未放宽。
  首次原生运行在 shutdown_begin 等待；随后“移至 CEF runner 释放”“等待 context initialized”两项
  假设同样失败，三个独立日志与差异保留，**两项未证实产品改动已撤回，未提交为修复**。
- 原 MAC-A-01 线程样本还证明 CEF foreground worker 在 `SecItemCopyMatching → Keychain decrypt`
  的系统 RPC 等待，与 SecurityAgent 存在一致；这缩小了原生加密/授权条件范围，尚未读取或证明具体
  Keychain 授权项。工具拒绝操作 SecurityAgent，未改加密、权限、mock keychain 或系统保护来制造通过。
- 本批只收最小复现入口及脚本回归；native 三次均保留 FAIL/forced-cleanup，不能以 Mac lib 通过或
  原空数据 signal exit 0 关闭。记录 `2026-10-01/macos/c05-context-release/`；系统授权/解锁条件需
  人工核对，后续只重验该 native 子链与正式已完成 Session 的退出，不扩回 Browser/soak 矩阵。

- **MAC-C03-01 exact unified diff output**（C03、`CMD-137`、`VCS-003` 的本批渲染子断言）：
  复用 MAC-A-01 真实回执里的 `Fdiff/H@@`，当前两处 formatter 仍把 libgit2 分类标记拼成正文。
  修前 App 与公开 VCS 工具的 Git CLI byte oracle 均失败，完整首败在 `run-001-first-failure`。
- 公共修复 **C03-01** 仅为 context/add/delete 内容行补 `' '/'+'/'-'`，完整 header、EOF marker、
  binary notice 原样输出；不做字符串替换，不误删真正以 F/H 开头的内容。修后 staged/unstaged、
  无 EOF newline 与 binary 输出逐 byte 等于本机 Git CLI **1/1**；子 repo/根路径/越界 **1/1**、
  VCS 工具相邻回归 **6/6**。原文件与 staged/unstaged 状态独立核对，路径/owner 过滤、权限、
  1 MiB/UTF-8 截断规则不变，首次 FAIL 不覆盖。
- 证据 `2026-10-01/macos/c03-diff-render/`。本批未额外调用模型或重跑完整领域；正式 Tauri/live
  修后、当前 MAC-A 首发/N3 及 C05 系统授权/退出仍开放，不以组件 PASS 关闭完整场景或 Windows 验收。

- **MAC-C01-01 host-specific executable guidance**（C01、`PROC-003/004`、`CMD-132/134` 的本批
  提示与 native 恢复子断言）：接续 MAC-A-01 的真实整行 command 首败，macOS 26.6.2 / 原生 arm64 /
  APFS。模型提示中的通用 Command Prompt 及混合属性示例改为宿主适用的合法 JSON/argv；明确
  `/bin/pwd` + `["-P"]`、`/bin/ls` + `["-a"]`。公共反馈优先字面 executable/args，cmd 仅供 shell
  syntax；保留 Windows 专属 PowerShell 合同，不改 owner、输入合法集合或暗中拆参。
- 提示回归及真实 host 恢复回归各一次首败保留；修后 tools/Schema **12/12**、process host
  **10/10**、native 特殊 token argv **1/1** 通过。错误 `/bin/ls -a` 仍返回未启动且 quiescent，
  分离 argv 随后真实 exit **0**、reaped；不存在把拒绝改成成功或吞掉历史错误。
- 公共提示问题见 **C01-01**，完整日志在 `2026-10-01/macos/c01-literal-guidance/`。无额外模型
  调用或全仓构建；本次只读 UI inventory 明确 Mac locked、自动解锁失败，须人工解锁后补正式
  首发/N3。C05 Keychain 条件与退出另待验；提示回归不关闭 MAC-A、模型首发或完整 Case。

- **MAC-A-02 exact delivery/receipt audit**（C01/C02/C03/C06、`CMD-134/136`、A18 的本批
  独立复核）：只读核对 MAC-A-01 原修后 DB、403 个 canonical 事件、原请求及正式 UI 文本，
  不重跑模型或改旧数据。事件副本与闭库逐项一致；7/7 原件 hash、完整四行读取、查有/完整零匹配、
  Git 观察、两指定测试实际 exit 0/1 有证据，先前组件及压缩修复结论不改。
- 明确失败而非继续“待核对”：实际只有 not_started、`ls -la`、两个测试共四个 process 调用，
  **没有执行 cwd 检查**；报告 rationale 却称 cwd/全要求完成。最终 UI/summary 一致，但没有
  cwd、隐藏目录结果及头尾原文。仅有三条可用 ID，分别为 listing 和两个测试，不能证明整个
  input_0；旧读搜/Git 已不在最后 schema 可用集合，不可搬入或放宽 freshness 来制造支持。
- 外部严格场景 oracle 仍 **FAIL / MAC_A_INCOMPLETE_DESPITE_CANONICAL_COMPLETED**；诊断模式
  输出事实不算修后通过。首个 audit 脚本误写 fixture 注释的失败也保留，改为从原件取实际注释后
  才核对场景；没有把 verifier 错误归因产品。完整记录 `2026-10-01/macos/mac-a-delivery-audit/`。
- 本批未修改产品；completion 现有合同本就区分合法引用和模型解释，不新增猜测式语义 veto。
  解锁后按原任务验证实际 pwd、头尾/列表交付与准确的缺证据披露，不重跑领域矩阵。当前 CUA
  再次明确锁屏；正式首发/N3、C05 系统条件/退出、MAC-B/C 仍开放，不改 Windows 或完整场景结果。

- **MAC-A-03 current formal command/compaction run**（C01/C03/C05/C06/C07/C08）：Mac 解锁后正常
  fast-forward 同步 `38c0a0df6` 的四项共享修复；正式 Tauri build、新 App `8bf5b41c1bc0…` 与
  deep/strict ad-hoc 签名通过，CEF helper/framework 沿用已验证的相同 pinned bundle，不作 release
  认证。本机 macOS 26.6.2 / arm64 / APFS；同一原 prompt、同一七文件隔离夹具，官方 COD 与
  加密 StepFun Plan。先冻结 **1 Session / 16 requests（含摘要）/ 4096 output / 360 秒**，未追加重试。
- 当前首发真实 `/bin/pwd` + `["-P"]`、`/bin/ls` + `["-la"]` 均成功，cwd 精确等于 Session
  workspace，**0 not_started**。实际六个 process 均 exit 0/reaped，但 pwd/listing 被重复；两
  指定测试 **0 次执行**、无 report_completion。**15 requests / 456 events / 六次应用压缩**后
  因 `compaction cannot fit…` failed/head ready，fixture failure=null、cap 未触发；不是供应商限流。
  新问题及前缀尺寸见共享 **C07-02**，不以首发子断言关闭 MAC-A/N3。
- 新 completion 提示已允许八 ID 内分组，但旧回归仍要求单 criterion，首败 **28/29** 保留。
  仅修该测试文案并增加九 ID 拒绝；中间 macro 编译错误也保留，最终 **29/29**。未改 admission/
  freshness 或把压缩产品问题伪装为已修。fixture 空目录检查、首启缺 work-dir 的拒绝和观察器
  guessed column 错误均保留/修正；没有重置 dataset、覆盖旧运行或把夹具错误归因模型。
- 正式 UI 显示应用处理失败/实际错误；Cmd-Q 清理仍未确认，线程样本再现 Keychain decrypt RPC
  等待，TERM 仍活。exact PID/path 核对后 KILL/exit **137**，owned App/Helper/fixture/listener
  最终 **0**；这是强制清理，不是 C05 PASS。已请求人工查看是否有系统提示，未操作 SecurityAgent
  或改授权/加密。独立闭库/备份 `ok`、456 events 相同、7/7 hash/Git/哨兵保护、凭据 exact match
  **0**。证据 `2026-10-01/macos/mac-a-current/`；完整 A/B/C、交付/N3、压缩和正常退出仍开放。

- **MAC-C07-01 fixed-prefix headroom**（C07、`CMD-143` 的有界续行/原回执子断言）：接续
  MAC-A-03 压缩首败，macOS 26.6.2 / arm64 / APFS，未追加 live Session 或调用。最小 native
  Runtime 反例首败保留：replacement bytes **64,371 < 65,536**，估算输入 **21,457** 连同
  4096 output/512 reserve 仍小于 32,768，却被 **21,120** soft trigger 当 hard cap 拒绝。
- 公共修复见 **C07-02**：固定前缀及摘要预留无软余量时，触发策略仅利用原硬 token 余量；
  成功压缩的 byte floor 为下一段保留半数剩余硬 byte 空间，避免有效摘要自身立即重触发。
  不改模型 metadata/default limits 或 output/byte/message 上限，不重标失败或赋予摘要权威；
  原失败回执和 call/result ID、唯一 accepted input 保留，未读图片/usage 校准/typed rejection
  继续受原硬边界保护。失败诊断仅加数值，不回显正文、路径或参数。
- 修后 near-prefix、byte hard-cap、provider usage 余量、typed overflow 单次/收紧保护通过；
  压缩定向 **18/18**、Runtime **217/217**。首次完整 Runtime 另为旧 review 夹具失败，旧预算
  对照同样失败：当前 review 只广告 report_completion，旧夹具先发未暴露 update_plan。
  仅改为有效直接报告，仍断言 current evidence、终结控制唯一暴露及零额外模型步；原对照保留。
- 完整证据 `2026-10-01/macos/c07-prefix-headroom/`。这关闭确定性子根因，不把 synthetic 几何
  反例当作原完整请求重放或正式 UI/StepFun 成功；MAC-A-03 首败、两测试/完整交付/N3、MAC-B/C
  与 C05 系统条件/正常退出仍开放。无 renderer 改动，不重跑 UI/build/全业务矩阵，不代判 Windows。

- **MAC-A-04 formal post-headroom / completion repair**（C01/C02/C03/C05/C06/C07/C08）：当前
  `4f13f7b2e` 正式 App `ed1ac7af512e…`、原生 arm64/macOS 26.6.2/APFS，deep/strict ad-hoc
  验签通过；原 prompt/七文件/官方 COD/StepFun Plan 不变。先冻结 **1 Session / 16 requests
  （含摘要）/ 4096 output / 360 秒**。fixture 并行编译首败增量对象缺失保留，正式 App 编译成功后
  单独重编恢复；未新建第二模型 Session 或放大预算。
- literal pwd/ls、完整读搜、只读 Git、两个指定测试实际结果均有独立证据；测试各 **1 次**、exit
  **0/1**、断言真实，原件 **7/7** hash/Git/哨兵保护。首次压缩 **73,305 → 65,656 SDK bytes**
  保留两个原 check ID/result；正式 resource default 为 **2 MiB/128 messages**，不可混为上一批
  64 KiB/256 的 synthetic 反例。模型冻结窗口/output 未变，无 compaction-cannot-fit 再现。
- 首次完成报告引用三条当前不可用 read/search ID，`observed_tool_error_count=0` 与 const **1**
  不符，被严格拒绝；随后 replan gate 拦截普通调用，重开计划后 pwd/listing 各重复一次。共
  **16 requests / 四次应用压缩**触及本地 cap 暂停，无已接受报告/最终交付。提议的完整 summary
  不是已交付文本，不把执行子断言或旧首败改为整组 PASS。
- 新完成参数反馈见共享 **C06-01**：仅针对单独的非法 report，修报告而不重跑已结算命令，
  分开历史观察与当前缺证据；不借用无关 ID、不改 schema/count/freshness 或混合批次整批拒绝。
  最小反馈首败保留，修后 validation **7/7**、report gate **1/1**；该提示未在本轮冻结 App 中，
  不宣称真实修后通过。所有日志/SSE/UI/DB 在 `2026-10-01/macos/mac-a-post-headroom/`。
- 正式 UI 结束暂停后 **cancelled/head ready**；这次 Cmd-Q 正常 **exit 0**，无 TERM/KILL、
  timeout/forced-exit 与 App/Helper/fixture/listener 残留。这只补“已取消 Session”退出子样本，
  不关闭此前 completed/failure Session 的 C05 Keychain 首败。普通 readonly 备份 SQLite 14/空库
  首败保留，闭库无 WAL 后 immutable 备份 **586 events** 对齐、`ok`；凭据 exact match **0**。
  正式完成提示修后、完整交付/N3、MAC-B/C 仍开放，未改 Windows 结果或额外全量走查。

- **MAC-A-05 completion repair / terminal phase**（C01/C02/C03/C05/C06/C07/C08）：正式 App
  `496fc4a1b76f…` / 源 `64eb7a2be`，macOS 26.6.2 / arm64 / APFS；顺序构建及 deep/strict
  ad-hoc 验签通过，原 task/七文件/COD/StepFun Plan 不变，先冻结 **1 Session / 16 requests
  （含摘要）/ 4096 output / 360 秒**。提示与单值计数进入该包，但未追加第二次 live 重试。
- 第一轮检查与两测试真实执行，测试各 **1 次**、exit **0/1**。首次 report 的两个计数均正确为
  **1**，只因两条历史 search ID 非当前 eligible 被严格拒绝。专门反馈在该结果中确实存在，
  但下一轮仍接受新 plan，把任务重置 pending；五次压缩后累计 **16 process**（十次重复只读
  检查，不含测试重跑）、**16 requests**触及本地 cap，无接受报告/已交付文本，不记整组 PASS。
- 公共结构根因见 **C06-02**：单独结束报告被拒、无显式计划且已有 proved settled failure 的
  窄路径进入已有 report-only 审查；无进程/未决补丁时，参数纠正不能再广告计划重置和普通工具。
  不改变其他真实未完任务/进程/补丁路径，不改 schema/freshness/count，不把拒绝变成功。
  首败保留；中间 fixture 错把被拒计划尝试计数维持 1 的失败也保留，改为真实 **2** 后通过；
  Root diagnostic 只调用一次、计划重置未应用且计数不抹去。Runtime **220/220**；修复未在本轮
  冻结 App 内，正式修后仍待验，不再追加提示层制造通过。
- UI 结束暂停后 **cancelled/head ready**。随后 Mac 锁屏阻断 Cmd-Q；已请求人工解锁，exact
  PID/path 的 TERM 仅清理测试资源、exit **0**，无 KILL，不算正式退出 PASS。App/Helper/
  fixture/listener 最终 **0**；7/7 hash/Git/哨兵不变，凭据 exact match **0**。
- WAL 存在时误用 immutable source 的首个 snapshot 留下 **940 events/running**，未作为验收；
  另以正常 readonly、包含已提交 WAL 的备份得到 **960 events**，与取消后事件逐项一致、`ok`。
  未编辑数据库状态，原失败制品保留。证据 `2026-10-01/macos/mac-a-report-repair/`；正常 UI
  退出、完整交付/N3、MAC-B/C 与 C05 原 Keychain 现场仍开放，Windows 结果未改。

- **MAC-C06-01 terminal boundary guard**（C06-02 的反向边界，非新增业务走查）：当前只读 CUA
  再次确认 Mac locked、无运行验收 App；未启动 live/构建/读取模型凭据，不尝试自动解锁。
  仅补新门控直接影响的显式未完成计划回归：报告参数被拒后仍广告/执行原授权 write_file 修复，
  修复 **1 次**，闭合计划后才接受报告，已记录非零和失败不抹去；无额外模型步。
- 最初 fixture 在控制未展示前直接发 update_plan，首败保留；按实际展示顺序先诊断、再建立
  计划后通过，不把夹具顺序错误归因新门控。新反向 **1/1**；复用 settled-terminal、live-process
  controls、unresolved-patch evidence/process 保护各 **1/1**。未改产品逻辑或任何接受集合。
- 证据 `2026-10-01/macos/c06-terminal-boundary/`；本批只补测试/短进度。完整正式交付/N3、A/B/C
  与正常 UI 退出仍待人工解锁后验证，不以 guards PASS 收尾完整 Case，不改 Windows 结果。

- **MAC-A-06 terminal review formal sample**（C01/C02/C03/C05/C06/C07/C08）：解锁后 fast-forward
  同步 `43cd16bee` 的进程寿命提示；正式 App `5bdf54681e83…`，macOS 26.6.2 / arm64 / APFS，
  顺序构建与 deep/strict ad-hoc 验签通过。原 prompt/七文件/COD/加密 StepFun Plan 不变，先冻结
  **1 Session / 16 requests（含摘要）/ 4096 output / 360 秒**，实际 **9 requests**，未加预算。
- 首发 literal pwd/ls、读搜/Git 与两指定测试均有独立结果；**六 process**、exit **0/0/0/0/0/1**、
  reaped，测试各一次，无额外命令/计划重置或本地 cap。一轮压缩、首个非法历史引用报告仍严格拒绝；
  请求 **07/08/09** 原生只广告 report_completion、Specific tool_choice，修正报告接受，
  **421 events / completed / head ready**。两固定计数真实保留为 **1/1**，supported ID 均映射原
  settled receipt，缺当前文件/搜索证据的两项保持 unverified/no evidence，不冒充当前证明。
- **完整任务仍 INCOMPLETE_DELIVERY**：最终 summary/正式 UI 漏原头尾与两个搜索实际结果，
  改成 stale_file_paths/unverified 内部术语与当前缺证据警告；真实读搜已发生并不等于从未读取。
  外部严格 acceptance 首败保留，不把 canonical completed 当整组 PASS。后续缺口见 **C06-03**。
- 这是 C06-02 防重开/纠正链的正式 **N1 子样本**，不是 full MAC-A 或 N3；首次完成尝试仍有
  参数拒绝，不声称零错误。正式 completed-session Cmd-Q 正常 **exit 0**，无 TERM/KILL、
  timeout/forced-exit、App/Helper/fixture/listener 残留；仅该现场退出子断言通过，不关闭旧 Keychain
  首败。原件 **7/7** hash/Git/哨兵保护，凭据 exact match **0**，DB/事件/交付引用独立复核。
- 普通 readonly backup 再报 SQLite 14 并生成空库，首败保留；确认 writer 已退出且无 WAL 后，
  另以 immutable source 备份 **421 events**、`ok`、completed/head ready，未编辑状态。
  完整证据 `2026-10-01/macos/mac-a-terminal-review/`；本批只更新短进度，不额外重复测试/build。
  历史结果交付/自然语言、完整 A/B/C/N3 与其余退出条件仍开放，Windows 结果未改。

- **MAC-C06-02 historical delivery contract**（C06-03、`CMD-136` 结果表达子断言）：只读核对
  MAC-A-06 原模型请求 **06～09**，四行原文及 NEEDLE-present/MAC_A_NO_MATCH 均仍存在，
  不是实际读搜未发生或压缩把所有数据清空。无新模型/Session/凭据访问、无全量 build/UI 排程。
- 修复完成接口的歧义说明，不改验收：summary 交付已知较早实际结果并标清时点，current-state
  不确定另外披露；不可用旧 ID 仍不能支撑当前 supported，也不借目录/命令 ID。rationale 实际
  可能追加用户可见警告，说明改为按用户语言表达，不再错误承诺“内部且不展示”。
- 新说明首败保留，修后 **1/1**、completion **31/31**：历史 summary 未省略、unverified 无
  引用且原 epoch 不变；stale current supported 仍拒绝。原正文/环境/输入排除断言继续通过。
  没有在 context/scopes 复制原 Tool output，没有新增字段/工具/权限，没有降低 schema 或真假
  结果断言。证据 `2026-10-01/macos/c06-historical-delivery/`；真实修后是否完整交付仍待验，
  不把说明测试记作 full MAC-A/N3，不改 Windows 验收或关闭阶段二、三。

- **MAC-A-07 historical results formal sample**（C01/C02/C03/C05/C06/C07/C08）：正式 App
  `6a4cf62eebf0…` / 源 `abb4661ee`，macOS 26.6.2 / arm64 / APFS；顺序构建、deep/strict
  ad-hoc 验签通过，原 prompt/七文件/COD/加密 StepFun Plan 不变，先冻结 **1 Session / 16
  requests（含摘要）/ 4096 output / 360 秒**；实际 **9 requests / 420 events / 一轮压缩**。
- 首个非法引用报告仍严格拒绝，后续原生 report-only 纠正接受，**completed/head ready**。
  六 process 无重跑、exit **0/0/0/0/0/1**、reaped，两指定测试各一次；历史四行原文/行数 **4**/
  NEEDLE-present **1** 与 MAC_A_NO_MATCH **0** 已进入正式 summary/UI，明确较早观察，旧 ID
  未变 current supported，unverified/no evidence 仍保留。C06-03 历史内容交付有 **N1 子样本**。
- **完整仍 INCOMPLETE_DELIVERY**：独立严格断言缺实际 cwd 路径，summary 仅“pwd 确认 cwd”；
  request **06～09** 仍包含真实路径，排除路径被清空归因。eligible citation/unverified/earlier read
  等内部术语及警告仍混入中文输出，未达到自然语言门槛。首败保留，不因头尾修后或 canonical
  completed 将整组/N3 记 PASS，不追加新模型或降低路径/语言要求。
- 正式 completed-session Cmd-Q 正常 **exit 0**，无 TERM/KILL、timeout/forced-exit 与 App/
  Helper/fixture/listener 残留。7/7 hash/Git/哨兵保护，凭据 exact match **0**。普通 readonly
  backup 再次 SQLite 14/空库，首败保留不当证据；确认无 writer/WAL 后 immutable 另备份
  **420 events**、逐项一致、`ok`、completed/head ready，未编辑状态。
- 证据 `2026-10-01/macos/mac-a-historical-results/`；本批仅更新短进度，不提交额外产品假设或
  再叠加提示。完整 cwd/公开语言/N3、GEN 入口、MAC-B/C 与其他退出条件仍开放，Windows 未改。

- **MAC-B-01 files / pipe EOF**（C04/C05/C06/C08；`CMD-140/141`、`PROC-027` 子断言）：正式
  Tauri `51e6e806ccc7…` / 源 `7f6b7bb05`，macOS 26.6.2 / native arm64 / APFS；冻结
  **1 Session / 16 requests（含摘要）/ 4096 output / 360 秒**，实际 **16 requests / 531 events /
  一轮压缩**，completed/head ready，无加预算或第二模型任务。隔离 COD 从正式输入框发任务。
- 原独立字节断言 **FAIL**：`write_file` 模型参数漏请求的末尾 LF，后续 patch/copy/move 如实
  保留，终版 **31 而非 32 bytes**；`write_process_stdin` 同时传入末尾 LF 与
  `append_newline=true`，实际得到 **两个 LF**。原 read/hash、helper receipt/ECHO 和参数逐项
  一致，非 owner 吞/改字节。首败保留，未修工作区产物或把“调用成功”当用户任务正确。
- helper **start/write/close 各一次**，最终 poll 为 exit **0** / reaped，无 cancel、信号升级或
  timeout；原件 3/3 hash 不变、只删除指定临时文件、无额外生成物。自然语言/存在状态交付
  仍不足，最后“均成功”未揭示字节不符，完整 B 未通过；长 helper/后代 stop 不在本小批。
- 只修现有工具说明：content 明示不补 LF；stdin 用两个合法 JSON 例子区分一个 LF 与两个 LF。
  App 仅给模型投影 content/input/append_newline 的 description，注册合同/贡献指纹、默认值、
  限额、接受集合和 owner 原样字节语义不变，未复制原 output 或新建提示层。回归首红保留；
  修后 Runtime **13/13**、App newline/精确字节/上限 **3/3**、既有游标/期限投影 **2/2**。
- 终态 UI 截图/Cmd-Q 前 Mac 锁定，未绕过；精确本批 App TERM **exit 0**，非 UI 退出验收。
  App/Helper/fixture/listener 无残留，真实凭据 exact match **0**。主文件-only 备份方法错误
  保留不使用；另作 WAL-aware readonly 备份，**531 events** 逐项匹配、`ok`、completed。
  证据 `2026-10-01/macos/mac-b-files-stdin/`；真实修后/GEN/N3、长 helper/完整 B/C 仍开放，
  MAC-A cwd/语言和旧 MM spinner 未关闭，Windows 结果未改写。

- **MAC-B-02 unchanged-task newline recheck**（C01/C04/C05/C06/C07/C08）：正式 App
  `267019c81a5a…` / 源 `929dae3bb`，同一原任务/三原件 hash/独立字节断言，新隔离 COD；
  冻结 **1 Session / 16 requests（含摘要）/ 4096 output / 360 秒**，实际 **16 requests /
  三轮压缩**，本机 guard 429 后正式 UI 结束，**499 events / cancelled / head ready**。
- **修后仍 FAIL**：request-01 确认新的 content/append_newline 字段说明已在正式模型请求中；
  原 SSE→canonical 的 **8 个参数对象全同**，两次 write_file 的上游 content 均缺末尾 LF。
  未见 NomiFun 解码/owner 改字节；说明回归通过不等于真实生成已修，停止叠加同类提示。
- 六个 exec 提议中五个实际 native 终态 **71/0/71/0/0**、全部 reaped；另一个正确 `/bin/ls`
  被 needs_replan 拒于派发前，独立 verifier 最初误计六个执行，首败保留后逐项纠正。两个 71
  是上游把 `ls -la`/整段 printf 脚本放入 literal command，wrapper 报 execvp 找不到程序；
  未自动拆 argv/改 shell。后续显式 cmd 仅修出 **32-byte 临时文件**，无终版/复制移动删除/
  helper 启动及有效完成报告。预算未增加，cap 429 不是本机账号额度不足证据。
- 本批再现“wrapper 已退出但请求程序未启动”进入 command failure/replan 的合同缺口；下批
  复核 wrapper 前真实可执行准备失败能否给 not_started。不能只凭 stderr 字符串推断、扩大
  权限或放宽清理/证据条件；不把未派发的正确重试当第二次实际 ls 或新增丢 receipt 故障。
- 无模型空数据预检 UI 可用，Cmd-Q 最终 exit0 却有 CEF cleanup 未确认/forced-exit 告警，
  **退出 FAIL** 保留，PID 已终止无法 sample 也保留。实跑 cancelled Session Cmd-Q **exit0**，
  无强制/timeout、App/Helper/fixture/listener 残留；不由实跑良好子样本关闭前置/旧 C05 条件。
- 原件 3/3 不变，partial 临时文件按取消合同保留；凭据 exact match **0**。普通 readonly
  backup 失败/空库保留；确认 writer/WAL 都无后另备份 **499 rows**、逐项相同、`ok`、cancelled。
  证据 `2026-10-01/macos/mac-b-newline-recheck/`；仅更新短进度，不提交新产品假设或重复测试。
  完整 B/GEN/N3/C、A cwd/公开语言与旧 MM spinner 均未关闭，Windows 验收未改写。

- **MAC-C01-02 Seatbelt bare-program pre-spawn**（C01/C06；`PROC-001/004`、启动失败恢复子断言）：
  从 MAC-B-02 两个 wrapper exit71 及随后正确 argv 的 needs_replan 拒绝定位；本批无模型/
  Session/凭据读取，macOS 26.6.2 / native arm64 / APFS。最小真实 native 首败保留：缺失裸名
  仍物理启动 wrapper，未满足零-authority preflight；先等待其 reaped 再使测试失败，未留孤儿。
- 只补 macOS Seatbelt 原有启动准备：按请求实际 PATH（override 优先、否则继承）和 cwd 做
  裸名字面 executable access 检查，缺失/不可执行在 watchdog/wrapper 前返回 spawn failure。
  不拆参、不猜 shell、不从 stderr 判未启动；成功不替换 argv，真实 execvp 仍为最后依据。
  PATH 未配置/访问 ELOOP 等不确定错误保留实际 execvp，不虚构缺失；绝对/相对/空 PATH、前项 EACCES 后项可执行及合法
  空格程序名保持原语义，未改 Sandbox/写根、注册 Schema/贡献锁或 Windows/Linux 路径。
- 原生 preflight **1/1**（含 TMPDIR/显式不可执行/裸名缺失和无执行权限，全部 watchdog/leader/
  reap/cleanup 调用为0）、macOS process **6/6**；扩展 PATH 边界另复核 **1/1**，非统计 N3。
  最终 Seatbelt 准备/不确定 access/原 profile 与 TMPDIR **4/4**，安全合同未降级。
  App 既有同 scope 字面恢复 **1/1**：裸/显式整行均 `PROCESS_NOT_STARTED/user_code_started=false`，
  正确 `/bin/ls` + args 后真实0/reaped，scope 可清理。Runtime 原非启动计数/控制 **1/1**，
  仍记录 tool error1、command failure0，拒绝假最终完成；原 direct exec ABORT/精确 reap **1/1**。
- 证据 `2026-10-01/macos/mac-c01-seatbelt-start/`。仅闭合该确定性准备/typed owner 子合同，
  不能保证 StepFun 首次参数正确，裸名 precheck 后变化、PATH 未配置和其他 wrapper 自身故障
  仍保守处理；真实 UI/live 修后、B 字节/完整链/GEN/N3/C、C05 退出和 MM spinner 均未关闭。
  未做额外全仓 build/UI/平台矩阵，Windows 结果未改写。

- **MAC-GEN-01 scoped directory observe**（C01/C06/C08、`CMD-132/134` 子断言）：正式 Tauri
  `56cdcdbde6b7…` / 产品源 `d8423c219`，新 UI 构建、隔离 data/work、StepFun Plan；先冻结
  **1 task Session / 8 requests（含摘要）/ 4096 output / 180 秒**，实际 **3 requests / 126 events /
  无压缩**。首次 native `/bin/pwd`+`[-P]`、`/bin/ls`+`[-a]` 各0/reaped，无模型 shell/读内容/副作用。
- 实际完整 cwd、四目录项（含 .hidden-case/中文空格）、各退出码已在 canonical report 和正式
  UI，completed/head ready、原件2/2 unchanged、Cmd-Q 正常0，无 TERM/KILL/强制退出或残留。
  本命令小链 **N1 子样本通过**，不关闭此前 A 的完整交付/语言、B/C、N3、C05 或 MM 缺口。
- 保留所有前置首次失败：browser-only helper 缺 computer role provider；补正式 feature 后完整
  General 要求 Computer/Scheduler；API 不能同时传编辑 document 与 fork；依赖编译未结束时的
  metadata 读取也保留。均在模型任务前，未当 StepFun/产品调用故障或“预算为0”。只给 runner
  加确切 GEN 模式、feature 前检与正常 API 的减权 General；没有添资源/系统许可或改产品合同。
- 独立 closed DB 比较证明 General persona/instructions/路由和其余能力全同，仅移除 computer/
  automation.schedule；因此是 **官方模板派生的减权 GEN**，不伪称完整默认 General 已验。
  原 oracle 首败保留：误将 host 指令读取算 model、误限 pwd 无 -P、漏中文“退出码为0”；
  用 raw SSE ID 区分并核对实际路径/列表/字节后另记通过，没有改任务、真实结果或放宽安全断言。
- 普通 readonly backup 失败/空库保留；确认 writer/WAL 无后另备份 **126 events**、逐项一致、
  `ok`；凭据 exact match0。完整证据 `2026-10-01/macos/mac-gen-observe/`；源码身份另附仅 helper
  dirty patch（不在产品 binary），helper 预算/模式/来源回归 **6/6**。Windows 结果未改写。

- **MAC-B-03 pipe EOF preflight**：当前产品源 `9aab28584` / 正式 App `8bb20b2fc160…` 已
  构建、deep/strict 验签，原 MAC-B helper/三原件不变；冻结 **1 task Session / 12 requests /
  4096 output / 360秒**，仅准备 stdin/EOF 问题簇，不把它代替失败的完整文件/B场景。
- 两次 CUA 均明确 Mac locked，未通过其他技术解锁/截图/操作系统权限。已确认 **0 requests /
  0 turns / 3 setup events / head ready**，没有发送用户任务，没有 StepFun authentication/额度故障
  证据，更不是把已授权预算设为0。现为 **UI_BLOCKED / NOT_RUN**，待人手解锁后独立 fresh run。
- 精确 owned App TERM **exit0**、fixture `/shutdown`200/exit0，App/Helper/listener 均无残留；
  这是未发任务的清理，不是 Cmd-Q或EOF通过。原件、预算、准备状态、UI拒绝和凭据 exact
  audit0保留于 `2026-10-01/macos/mac-b-stdin-eof/`；本批只有短进度，未额外修源码/重复回归/
  发模型或改 Windows 结果，MAC-B/长helper/后代/N3/C/旧失败仍开放。

- **MAC-B-03 pipe EOF live N1**：复核时对 CLOSED 旧 CUA 绑定读 AX 出现 timeout，并观察到
  旧 GEN bundle 被重启/占单例，browser handles 在默认 dev profile；不能当安全被动锁屏探针。
  未向它发任务/读取用户 DB 内容/删除目录；确认 probe-created PID6027 后 TERM 回收。后续
  只显式设置隔离 data/work、确认确切 PID 运行后才选 UI，事故/默认启动影响不作隔离 PASS。
- 隔离无模型 UI 确已可操作，锁屏不再是当前阻断；其 Cmd-Q 再现 CEF shutdown 未确认，
  sample 指向 SecItemCopyMatching→SecKeychainItemCopyContent→SecurityServer decrypt；TERM
  不退出，精确 owned PID6887 后 KILL **exit137**，保留 **C05 FAIL**。无解锁/Keychain授权绕过。
- fresh run003 使用原任务/helper/13-byte断言，正式 App `8bb20b2fc160…` / frozen产品源
  `9aab28584`，原 **1 task / 12 requests / 4096 output / 360秒**不变；实际 **7 requests /
  243 events / 无压缩**，completed/head ready。start/write/close各一次，pipe/no shell，输入
  `你好 MAC-B`+单 LF **13 bytes**；实测 READY/ECHO/EOF、最终0/reaped，无取消/信号/timeout。
- raw SSE的8个参数对象与canonical全同；helper独立receipt/原始output/byte count/report/UI
  相互一致，三原件不变，仅指定helper instrumentation生成receipt；实跑 Cmd-Q正常 **exit0**、
  App/Helper/fixture/listener无残留。本输入/EOF/终态 **N1子链通过**，原B01/B02错误不改写。
- verifier最初误加“不可读任何文件”限制，首败保留；原任务允许读规则/指定helper，按两者精确
  路径核对后另记事实，仍禁止无关读取/其他命令/编辑/kill或shell代替EOF。实际有helper预读、
  末poll未推进cursor（重放完整输出），且进度英文/summary带内部字段；效率/语言完整门槛
  **未达标**，不能记完整B/C05/N3。closed243-row快照逐项相同/ok，普通备份首败保留、key audit0。
- 证据 `2026-10-01/macos/mac-b-stdin-eof/`（UI-recheck事故及probe失败单列）；本批仅短进度，
  无新产品假设/重复底层回归。完整文件/后代停止/C/N3/C05与MM仍开放，Windows结果未改写。

- **MAC-B-04 formal UI Stop / descendants N1**（C05/C06/C08、`PROC-033/036/039` 子断言）：
  正式 Tauri `e362de1265e9…` / frozen源 `b530d751f`，当前UI构建/验签，隔离COD/加密StepFun；
  冻结 **1 task / 12 requests / 4096 output / 360秒**，实际 **7 requests / 228 events / 无压缩**。
  用户任务明确10分钟进程期限供UI Stop，不改变既有600000上限/权限，不用timeout关闭场景。
- 首次 start一次、literal bun+长helper/pipe，独立poll读READY，后续六次poll全部接正确cursor
  0→60，wait30000真实等待，无重启/模型cancel/close/其他命令或改文件。独立ps在点击前证明
  父PID34131/子34132、真实ppid关系、同PGID34131，双PID心跳增长；并非只相信helper文本。
- 正式UI Stop后独立 **141ms** 内两PID消失、随后1秒心跳不增，机器5秒断言通过。canonical
  cancelled/headready、host_cleanup_proven，受中断poll仍有原call的native cancelled/reaped/
  interrupt-only **148ms** 回执，is_error=false，原READY/CHILD_READY输出保留，无新副作用或孤儿。
  三原件hash不变、仪器文件保存，UI显示操作已取消和“下方是停止前尚未完成的回复”。
- 本停止/后代清理 **N1子链通过**；模型进度仍有英文/重复状态/内部cursor等，完整语言与
  过程体验 **未通过**，不由物理清理关闭完整B/N3/C或旧Keychain/C05。实际Cmd-Q正常0、
  App/Helper/fixture/listener无残留，key exact audit0。普通备份首败保留，闭合后无WAL另备份
  **228 rows**逐项相同/ok。编译结束前读artifact的helper前置失败也保留，0模型、不当产品故障。
- 证据 `2026-10-01/macos/mac-b-stop-descendants/`；仅短进度，无新产品假设/重复进程矩阵。
  仍缺完整文件步骤/综合B/C/N3和语言效率，MM旧retry不关闭，Windows结果未改写。

- **MAC-C-01 cancelled restart cold-read**（C07、`LIFE-020/OBS-014` 冷读子断言）：只复用原
  MAC-B-04 已隔离 cancelled Session/228事件和正式 App `e362de1265e9…`（源b530d751f），
  显式原测试 data/work 重启，未建新Turn/Session/模型请求/读取凭据，原B事件与效果作基线。
- 正式UI选择原Session、展开 cancelled poll；原READY/CHILD_READY/60 cursor、reaped/148ms
  输出全部可见，旧未完成回复提示保留、活跃状态空闲。运行时只读及关闭后另读均 **228行
  全字节一致**、唯一Turn仍cancelled/headready；心跳长度/三原件hash不变，原两PID/forwarder
  不复活。Cmd-Q正常0/无App Helper或forced/timeout，本取消重启冷读 **N1子链通过**。
- 同批只读定位英文重复：raw response04～06英文在 reasoning，Runtime/Journal正确分流为
  thinking；response07也在公开content自我规划，公开196 chars/思考194 chars、trim不全同。
  不是UI把thinking串正文，也没有精确重复的去重依据。原字段/事件/UI文字不删、不译、不
  模糊匹配隐藏以制造通过，生成层语言/叙述门槛仍 **FAIL/open**，未交新产品假设。
- 关闭后普通readonly读取再SQLite14，首败保留；确认writer/WAL无后用immutable只读原库，
  与先前live readonly结果逐项一致，未编辑DB/WAL/原cancelled记录。证据
  `2026-10-01/macos/mac-c-cancel-cold/`；完整连续追加/纠正/实压缩/C/N3和A/B/旧C05/MM仍开放。
  本批仅短进度，不复制native/UI矩阵或发新模型，Windows结果未改写。

- **MAC-B-05 isolated original file clauses / absent evidence**（C04/C06/C07/C08）：中断后的原
  build/package handles 均确认exit0，再同步W205重构当前正式App `7be990c8a04a…` / frozen源
  `42f63a358`。原B两条文件要求/三原件/32-byte及LF断言不变，只隔离文件簇；冻结
  **1 task / 16 requests（含摘要）/ 4096 output / 360秒**，未加预算或原task重跑。
- 实际 **16 requests / 436 events / 二轮压缩 /14主steps**，有界续写仍length，最终typed
  NOMIFUN_TASK_INCOMPLETE / failed/headready，**整组FAIL**、无accepted报告。确切终版32bytes/
  SHA `6ab0c427…`、复制移动/临时删除实际已发生，临时/副本不存在、三原件不变；独立snapshot
  与磁盘核对相同。不用文件正确/后来修复或模型意向证明任务完成，原首败保留。
- 定位到一项具体context缺口：真实read返回 `workspace_file_absent`，non-error表示正常确认
  缺失；completion scope只剩read/path而丢kind，模型将成功read推作存在、纠结mv及资格后截断。
  最小复现首红保留，补 **有界kind/file_exists=false**，只对已派发成功的scoped text/read/
  missing_ok/原path匹配结果。没有复制content/env/stdin/output正文、增加预算或删历史失败。
- 修后 completion **32/32**：当前缺失语义明确，过期后仍不可cite（历史说明保留），错误/
  未派发/错path/假content-marker/无missing_ok/非text/非scoped均不取得absence元数据；原计数/
  freshness/引用/schema接受集合/权限不变。仅闭合这项确定性数据元信息子根因，**真实修后待验**。
- 实跑失败Session正式Cmd-Q正常0，无App/Helper/fixture/listener残留，key exact audit0；普通
  readonly备份首败/空库保留，writer/WAL无后另作436-row全相同/ok/failed快照。证据
  `2026-10-01/macos/mac-b-file-results/`，原产物未修/覆盖、Windows失败/结果未改写。
  其他wrong命令/报告生成、完整B/N3/C、A交付、语言/效率、C05/MM等仍开放，未追加paid循环。

- **MAC-B-06 原文件任务交付复测**（C04/C06/C07/C08）：同步W207，正式Tauri源`335e33c6b` /
  App `d56d34055f4f…`，原task/seed/32-byte末LF断言不变，新隔离Session；预算仍1 task /
  16 requests含摘要 /4096/360秒。实际 **12 requests /367 events /10主steps /两压缩**，
  completed/headready且一次accepted交付，终版精确32 bytes/`6ab0c427…`，临时/副本无、三原件不变。
  cp/mv/shasum/rm各一次exit0/reaped，write/patch/read各一次；10组raw参数与canonical全同，
  报告纠正后零副作用重放、无cwd/list无关probe，正式UI错误/报告及文件预览均保留。
- **整组FAIL_RECOVERED**：首次report的criteria嵌套数组并把summary/count放入其中；第二次形态
  已修但引用rm后过期path，均预检未执行。第三次引用相应历史call作用范围、保留error2/
  commandfailure0获接受；不能用最终正确覆盖首败。实际没有missing_ok read，故不代验C06-08。
  summary原含字面反斜杠n，未自动反转义/删原文，输出格式仍待验。
- 只补现有description：flat criterion objects、summary/count为根字段，无可用path则omit/[]、
  仅引用匹配eligible call范围；类型/required/接受集合/期限/权限/计数不变。首红保留，
  completion **32/32**，这项新说明尚无修后live，不宣称已解决供应商所有报告形态错误。
- W207本机定向回归首次因`/var`与`/private/var`别名错用原options路径失败；产品原准入已正确
  canonicalize。cfg(test)改用真实receipt且核对同目录，精确context一次/零模型/零效果不放宽；
  aliased/nonaliased各1通过，纯数据1通过。runner data-module预备失败另保留，0live调用。
- Cmd-Q后CEF shutdown未确认/有界清理失败，sample主线程在native cef_shutdown/Mach RPC；
  无确定RPC对象，不将本次直接归因Keychain。仅owned PID TERM后退出0，**非正常UI退出PASS**。
  fixture shutdown200/0，App/Helper/三listener无；完整367-row WAL-aware backup与原events全同/ok，
  普通readonly image SQLite14首败保留；key audit0/283。证据`2026-10-01/macos/mac-b-file-delivery/`。
  完整B/N3、A/C、格式/效率、C05/MM仍开放，未追加paid循环或改写Windows结果。

- **MAC-B-07 原文件＋stdin/EOF联合任务**：源`c4456fa41`，正式App`6522913fd1d1…`及重构UI；
  原B01 task/三原件/字节断言不变，冻结1 task/16 requests含摘要/4096/360秒。实际16 requests/
  14主steps/三压缩，528 events本地cap暂停，正式结束后529 events/cancelled/headready；
  local429不是StepFun认证/服务限流证明。终版32 bytes/`6ab0c427…`、cp/mv各一次0/reaped、
  临时/副本无、原件不变；helper一次/实际单LF13 bytes，结束后PID无/清理证明保留。
- **仍FAIL_LOCAL_BUDGET_STOP**：未close/EOF/terminal poll/报告。首write漏LF，经后续write/patch
  才修正；patch首发缺files项path，wire原schema明确必填，预检未执行。raw/canonical参数全同，
  不归因owner改字节/解码丢字段、不自动补参或放宽接受。未达到report调用，不能关闭C06-09。
- **MAC-C05-03 退出证明与失败码**：B07 Cmd-Q0却有CEF未确认/强制退出首败；Main的有界fallback
  将may-exit当cleanup_verified且沿用0。现独立forced-handoff permission，不制造清理证明；
  unverified普通0/未指定退出→1，已有非零/原intent/restart sentinel归属保留，fatal仍需真实清理。
  首红保留，新增2/2、coordinator4/4、相关cleanup15/15；原期限/重试/系统保护未改变。
- 用户报告授权后，修后正式App`cc19ba26e087…`（`c4456fa41`＋recorded Main delta）原隔离数据
  零模型冷读：原529 events逐项全同/ready/cancelled、原件/终版/helper回执不变，无新Turn/模型，
  Cmd-Q正常0且无timeout/forced/cleanup错误，旧PID/listener无。只记当前条件的正常退出观察，
  未确定系统项或修复原native RPC。整个probe准备95.9秒已超预设90秒、runner未及时终止，
  **不记probe PASS**，时间首败保留；失败码分支仍为确定性覆盖，不由正常退出代验。
- 普通closed readonly备份SQLite14/空输出另保留，writer/WAL/shm无后另作529-row全同/ok快照。
  证据`2026-10-01/macos/mac-b-combined-recheck/`（跨午夜），key audit0/399，App/helper/fixture均无。
  本批不追加模型循环/全矩阵，完整B/N3、A/C、报告说明live、原生条件/MM仍开放，Windows不代判。

- **MAC-C05-04 独立限时冷读退出**（2026-10-02，runner子根因）：保留上一probe95.9秒/90秒首败，
  新增小型owned-child supervisor，从spawn起单调计时90秒，超时只TERM自己的child、5秒后必要
  KILL；超时/信号/observer失败即使exit0也不通过，无shell/任意PID清理或产品期限/权限变化。
  正常0、忽略TERM、超时后0三项定向回归 **3/3**，非新增native矩阵/付费模型。
- 复核当前Main与签名制品字节一致，复用正式App`cc19ba26e087…`/原隔离B07数据，零模型/新Turn；
  正式UI冷读后Cmd-Q，PID33738 **exit0 /56.88秒 /未expired /零TERM/KILL**，无清理timeout/forced
  告警。前后529 events全字节相同、唯一cancelled/ready、原件/32-byte终版/helper回执均不变，
  App/helper/fixture/PID残留无。writer/WAL/shm无后final readonly完整快照同/ok，截图/原native日志在
  `2026-10-02/macos/native-cold-deadline/`。仅本有界冷读退出 **N1子链通过**，旧首败不覆盖。
- UI各调用先检查监督器live状态/剩余15秒且单调用5秒限制，结束后不访问closed绑定；可选
  listWindows在本机不可用的准备限制另保留，使用已确认live的明确App路径，不重启默认profile。
  不由此代验forced错误码分支、旧native RPC根因/授权项、完整A/B/C/N3或MM；Windows结果未改写。

- **MAC-B-08 原文件任务报告说明live**（C04/C06/C07/C08）：新正式Tauri源`783b49a2c` /
  App`23f153672553…`/新隔离Session，原task/seed/33→32-byte及LF断言不变；冻结1 task /
  16 requests含摘要/4096/360秒，独立App480秒含UI准备/关闭。实际 **9 requests/305 events/
  8主steps/一压缩**，首report接受且一次delivery/completed/headready，错误/命令失败均0。
  write/patch/read各一次，cp/mv/rm各一次0/reaped，终版32/`6ab0c427…`、临时/副本无、原件不变。
- 7组raw/canonical全同，criteria flat objects/summary root/无过期path；C06-09形态/空path
  修后正式 **N1子断言通过**，不声称唯一因果或N3。没有missing_ok/archive调用，不代验对应live。
  公开summary/rationales仍有available_evidence/read_file/earlier与三未验提示，原模型公开段
  和最终控制summary均保留，非UI泄露private reasoning；交付语言/体验未达，不净化原文制造通过。
- **整批FAIL_PUBLIC_DELIVERY_AND_RUNNER_TIMEOUT**：旧bytes/hash/状态/counter oracle首报PASS仅
  覆盖那些字段、缺公开语言/原生期限断言，原结果保留，补充严格结果不放宽字节/安全。
  任务已结束后观察者错过480秒，独立supervisor owned TERM/exit0/expiredtrue，仍FAIL；late
  capture/Cmd-Q在UI API前被guard拒绝，未重启默认profile。最终截图/AX缺失另记，不伪造。
- App47353/fixture47318/listener无，fixture shutdown200/0；完整WAL-aware305-row快照与原events
  全同/ok、completed/ready，key audit0/264。证据`2026-10-02/macos/mac-b-report-recheck/`。
  本批无可证实新产品根因，只收短进度；未新增模型或扩大预算，完整B/N3/A/C/公开语言、
  C06-08/forced native branch/旧CEF/MM仍开放，不改写Windows结果。

- **MAC-C06-10 历史恢复说明一致性**（2026-10-02，零模型）：复核W212发现tool总说明允许用
  已展示history找回请求摘要所需既有输出，但evidence_call_ids说明及每轮Completion accounting
  仍把history恢复和新观察/效果重放混禁，形成真实提示矛盾；不宣称它是B08语言问题唯一根因。
- 只统一这两处description：仅already-advertised history/既有输出/请求summary；恢复不取得
  current/eligible资格，不为修account重复观察/效果。类型/required/accept/epoch/计数/权限/
  budget及dispatch未改，不翻译/删原文或复活旧证据。复用既有stale-report回归，首个测试签名
  编译错误另保留，语义首红保留；修后completion **32/32**，old-read supported仍拒绝、原epoch
  不变、historical unverified交付仍合法。证据`2026-10-02/macos/historical-guidance-consistency/`。
  本批无native/StepFun重跑，正式交付/公开语言/完整A/B/C/N3及旧问题仍开放，Windows记录未改。

- **MAC-A-08 当前综合任务前置**（2026-10-02）：源`b53fce0b4`，正式App`2a6596e9e800…` /
  已核对UI，原A任务/7文件/dirty Git/两指定测试/独立断言不变，新隔离Session；冻结1 task/
  16 requests含摘要/4096/360秒、独立App480秒/5秒grace，构建/runner/签名均完成。
- 正式UI工具返回 **Mac locked，需人工解锁**；没有发送任务，0上游请求/0Turn，仅3初始化
  events/ready。记 **BLOCKED_HOST_LOCK_PRE_SEND**，不算provider/认证/额度故障或PASS，已请求手动
  解锁，未绕锁屏/改系统保护。exact owned App TERM0、fixture shutdown200/0，无App/helper/
  listener残留；信号清理不替代Cmd-Q/UI验证，隔离首败保留。
- readonly WAL-aware完整3-row快照同/ok，7原件hash/GitHEAD/status不变，sentinel无，key audit0/283；
  证据`2026-10-02/macos/mac-a-current-delivery/`。继续复核相关既有nonzero直接report、settled
  report不重放、explicit open-plan授权修复三项 **各1/1**，无新源码假设或全矩阵重复。
  仅该live前置受阻，实际cwd/中文交付及完整A/B/C/N3/旧问题未闭合，Windows结果未改写。

- **MAC-A-08 前置复核／再次锁定**（2026-10-02）：只读console标记不足证明已解锁；owned
  零模型run004实际UI一度可访问，截图/AX保留，但Cmd-Q未在90秒内结束、TERM/KILL，仍FAIL。
  随后新源码`c2aa06028`/正式App`52197280cdd8…`/新隔离run006再次明确Mac locked，未发送任务。
  两份前置均0请求/0Turn/3初始化events，原16-call预算未消费/扩大；7原件/Git/完整快照同/ok，
  exact owned清理与listener无确认，key audit0/523，首败/短暂可访问/强制退出各自保留，不混为PASS。
  证据沿用`2026-10-02/macos/mac-a-current-delivery/`；需人工持续解锁后再live，不再无依据重复
  构建/启动/同构测试，未绕保护/假报provider故障或改Windows结果；未验项保持原状态。

- **2026-10-02 阻断审计**：只读系统会话metadata明确`screen_locked=true`，同一手动解锁前置
  连续三goal turns阻断正式UI；已有相关确定性检查和远端修复已核对，无新安全工作需重复。
  停止自动续跑，目标标记blocked而非complete/paused；待人工持续解锁并继续后恢复，未验/首败
  保留，0新模型/用户数据变更。依据`run-008-blocked-audit/`，不追加构建/UI启动/重复测试填进度。

- **MAC-A-09 恢复后当前命令链**（2026-10-02）：当前UI可访问的零模型探针与90秒退出失败
  各自保留；同步W215～229后Runtime243/243、Mac process6/6、原生deadline1/1及Store fence1/1。
  新正式Tauri源`8bb9226b3`/App`64015995097a…`/新隔离Session，原任务/7原件/约束与
  1task/16requests含摘要/4096/360秒/独立App480秒不变；旧前置均0请求，无预算重置扩大。
- 实际 **8 requests/一压缩/202 events**，partial report接受/completed/ready，**整组FAIL**。
  pwd/ls各一次0/reaped、一次97-byte/4行回读正确、真实cwd已交付，原件/Git/sentinel不变；
  搜索/Git/两指定测试未执行，工具错误7。11组raw/canonical全同，六提议argv为JSON字符串
  是另外的真实参数问题，未默拆/放宽。模型公开summary/rationales及原错误保留。
- 特定产品根因：private reasoning-only stop/零公开text/零call被当闭合答复，原动作表下一请求
  收窄report-only，继续原任务的read/exec因此未执行。最小语义首红保留；仅该active-ledger
  空答复走既有协议有界纠正，不执行文本/复制思考、不加authority、不重放已结算命令。
  原2-consecutive/8-total/Turn预算、patch及report参数纠正护栏不改；重复空答复失败。
  测试桩编译/计数/路径/返回约定错误分别另存并按真实接口修正，非产品断言放宽。
- 修后剩余read/不重放及boundedempty正反通过，Runtime **244/244**；真实修后尚未重跑。
  正式终态截图/AX先保存，Cmd-Q0/312.44秒/无expiry/TERM/KILL/cleanup错误，fixture shutdown200/0，
  App/listener无。writer/WAL/shm无后完整202-row只读快照同/ok，keyaudit0/299；证据
  `2026-10-02/macos/current-command-sync/`。公开语言/完整A/B/C/N3及旧CEF/MM仍开放，Windows不代判。

- **MAC-A-10 空公开stop修后正式样本**（2026-10-02）：源`ef626f626`/正式App`0470356677ee…`/
  新隔离Session，原A任务/7文件/独立断言与1task/16requests含摘要/4096/360秒/独立App480秒不变。
  实际 **16 requests/4压缩/396暂停→397取消events**，localcap后正式End Turn，cancelled/headready，
  无report/两指定测试未运行，**整组FAIL**，local429不是provider认证/额度故障证明。
- C06-11分支实际 **N1**：两次零公开text/零call/仅private reasoning的stop被有界纠正；
  后续请求仍展示原exec/read且required，下一read/exec实际结算，工具错误0，无未暴露拒绝。
  同一Task两分支不当N2/N3，也不据此保证所有空答复/完整A或模型无重做；原A09首败保持。
- 原cwd/隐藏项/97-byte4行回读/精确两搜索/Git观察已执行；模型后来新提议sh脚本重做这些
  观察并grep||true，原参数和偏差保留；另有一次TOOL_SHAPED_TEXT摘要拒绝，纠正计入原budget。
  11组raw/canonical全同，7原件/Git/sentinel保持，不以唯一action统计稀释重复或假报测试通过。
- 暂停/取消UI截图/AX先保存，Cmd-Q0/280.27秒/无expiry/TERM/KILL/cleanup错误，fixture200/0，
  无App/helper/listener残留。post-Q inspector SQLite14首败保留，writer/WAL/shm无后另作完整
  397-row只读快照/ok；final仅从该快照导出，不误用空库。证据`2026-10-02/macos/empty-stop-live-recheck/`。
  本批无新可证实产品根因，只短进度/不追加模型；公开语言/重复根因/完整A/B/C/N3及旧问题
  仍开放，Windows记录未改写。

- **MAC-C06-12 完成工具静态说明减重**（2026-10-02，零模型）：MAC-A-10真实请求15的
  27 tools共50,892 wire JSON bytes，system共23,911；四次压缩仍保留5/5/1/1原call IDs，
  未证明回执全部丢失。仅压缩重复静态说明3,247→2,385 bytes；参数Schema、动态提示、
  Completion context/计数/资格/接受规则源字节不变，不扩预算/权限/删结果或放宽断言。
- 首红保留，修后completion **40/40**；旧wire仅替换该字符串的模拟tools50,892→50,030
  （约1.7%），非新的provider/UI结果，不宣称重复根因全解。原A10整组FAIL及完整A/B/C/N3、
  公开语言/旧MM仍开放。证据`2026-10-02/macos/completion-prompt-footprint/`；Windows不代判。

- **MAC-A-11 合并减重后的正式原任务**（2026-10-02）：源`11b7d0164`/新正式Tauri制品/
  新隔离COD Session，原任务/7文件/断言与1task/16requests含摘要/4096/360秒/native480秒保持。
  实际 **7请求/2压缩/338events**，UI41秒completed/headready；九组raw/canonical参数全同。
  pwd/ls/Git status/diff/两指定Bun各一次，exit **0/0/0/0/0/1**、全部reaped，未重做或宽测试。
- 整组仍 **FAIL**：两搜索从未派发；最终漏cwd和头尾原文且英文。最后请求仍有完整输入/四项/
  语言规则；四条supported共引input_0并非语义完整证明。第二压缩后旧文件原文不在最终请求，
  已展示history工具但未调用；不能归为全部要求丢失或猜测性修复。未再付费重跑/扩大预算。
- Cmd-Q前完成UI/展开进度截图AX保存，native0/156.81秒/无expiry/TERM/KILL/CEF清理错误；
  fixture200/0、App/helper/listener无。writer/WAL/shm无后只读另存338-row完整库同/ok，原件/
  Git/sentinel保持；具体tool row展开未截图，dev updater warning保留、不当命令故障。
  证据`2026-10-02/macos/mac-a-completion-copy-recheck/`；只新增完整任务反例和命令N1事实，
  完整A/B/C/N3及公开交付仍未结案，Windows原结果不改。本批仅短进度、无猜测性源码修改。

- **MAC-C06-14 已读短文本的历史页保留**（2026-10-02，零模型）：沿A11四行在最终请求缺失，
  不改压缩硬限/强制输入；成功、scoped、call-ID匹配的ReadOnly文本页在原owner scope中保留，
  连metadata最多512 bytes，字节游标/行列/版本标记原样，超限整体不留、不裁剪或自动重读。
  原scope2KiB/历史detail4KiB/观测32KiB与64条上限不变，原观测淘汰即删；未扩权限/额度。
- 旧epoch/资格保持，历史文本不是当前文件证明，stale supported仍拒绝。语义首红保留，
  新三回归含分页/私有extra排除、13种拒留与淘汰；completion **43/43**、Runtime **254/254**。
  一次测试enum编译错误另留、按真实ManagedEffect修正，未放宽断言。Schema/提示/报告接受/
  prune源字节复核不变。证据`2026-10-02/macos/historical-file-page/`；修后正式UI/live未跑，
  A11漏搜索/交付/英文及完整A/B/C/N3仍开放，Windows独立结果不改写。

- **MAC-A-12 短页修后正式原任务**（2026-10-02）：源`685e672a2`/新Tauri制品/新隔离COD，
  原任务/7文件/断言与1task/16requests含摘要/4096/360秒/native480秒不变。实际16请求/
  5压缩/一伪摘要拒绝/491events，localcap暂停，无report/预期非零Bun未执行，整组 **FAIL**。
- C06-14正式 **N1**：05/08/10/12各后续task请求含原97-byte四行、epoch2/eligiblefalse，旧ID
  不进入current enum；13～16无工具摘要请求，不是假称最后完成请求。14组raw/canonical全同，
  两搜索/通过Bun真实执行；模型把sed脚本误装literal command→not_started、正确Bun兄弟被defer，
  后续未引号/有引号cmd两次重读；前者pipeline0但保留sed错误，不删stderr或当成功，原件/Git保持。
- 观察流程另 **FAIL**：暂停/进度/tool rows截图已存，观察者未及时End Turn/Cmd-Q，native480秒
  超时owned TERM后0仍expired，非正常UI验收；late guard拒访closed绑定，无默认数据重开。
  App/helper/listener无、fixture200/0；源WAL存在，普通readonly WAL-aware另备完整491rows同/ok。
  paused head/running Turn保持，未伪造cancel。证据`2026-10-02/macos/mac-a-file-page-recheck/`；
  本批仅短进度，不重复付费/猜修/扩权限，完整A/B/C/N3与旧MM仍开放，Windows不代判。

- 同步远端`67617e1cf`的主线收尾约束：按B→C→A推进；同一未改根因不付费循环，暂停纯说明
  微调/相邻边界/历史矩阵扩展。A12为已完成的新修复正式样本，不继续加跑A；原失败保持。
  同步的null元数据省略只复核完成合同，未递归删原参数/输出的null、0或false，不代判live。

- **MAC-B-09 完整文件/EOF/父子取消/末尾回读**（2026-10-02）：源`7033e254e`/正式Tauri，
  一Task提前冻结24总请求含摘要/4096/360秒/native480秒；在原B07文件与EOF要求上合并长helper，
  不改32-byte/13-byte或清理断言。run002锁屏前置0请求/0Turn，人工明确解锁后新隔离run003，
  同冻结额度未重置；旧绑定闭包误读旧control被guard拒绝、另以新绑定继续，未重开默认数据。
- 实际 **24请求/5压缩/546events/17组参数全同**，整组 **FAIL**：先对不存在target读/patch，
  缺expected_source；missing_ok/replan后才实际write，全部write提议漏末尾LF，patch/cp后临时/
  副本31 bytes而非32，未move/delete/helper/EOF/父子取消/report；五失败结果保持，四原件不变。
- 观察者再未及时结束UI，native480秒expired/TERM0非Cmd-Q通过，终态UI缺失、headpaused/
  Turnrunning不伪装cancel。App/helper/listener无，fixture200/0，WAL-aware readonly备546rows同/ok；
  阻断run首次错误immutable源副本不作验收，正确WAL备份3rows另存；Task文件尾NL验证器错误
  保留，按实际接纳输入的外围trim核对五原文，规范化来源层未定位，FILE末尾LF仍严格失败。
  证据`2026-10-02/macos/mac-b-mainline-recheck/`；
  无猜修/第二付费任务，B/N3开放，按主线转C，Windows不代判。

- **MAC-C-02 纠正/压缩/取消冷读主链**（2026-10-02）：源`e46e63a90`/正式Tauri/StepFun新隔离
  COD，一Task/一steer提前16总请求/4096/360秒/native480秒、cold0模型/90秒。实际12请求/
  2压缩/233events/9组参数全同，正式按钮接受seq92纠正后仅写/读正确20-byte目标一次，旧目标
  不存在、三原件保持；旧poll因steer优先拒绝，不能计成功READY观察。主链 **N1**，非N3。
- 真实UI Stop→cancelled/headready，父子消失/心跳停，先存UI再Cmd-Q0/88.41秒/无expiry或信号。
  同制品/同数据零模型冷读看见两用户输入，233events逐字同、文件/旧Turn不复活；但cold
  Cmd-Q实际CEF completion未确认、90秒TERM→KILL，95.03秒 **FAIL**，非观察者拖延或正常退出。
- 上游response08公开delta.content确发39-byte英文private-omission占位，UI原样保留，不翻译/
  删除来通过；成功后续poll/完整中文交付未验证，完整C仍FAIL。冷强退WAL存在，初错immutable
  副本不作验收，普通readonly WAL备233rows同/ok另存；首个审计误用event kind失败保留后按
  turn/steer-accepted修正。证据`2026-10-02/macos/mac-c-mainline/`；无付费重跑/猜修/Windows代判。

- **MAC-C05-08 冷退出零模型定位**（2026-10-02）：同C02制品/数据，GUID直接Cmd-Q0/13.90秒、
  进入cancelled会话后Cmd-Q0/21.85秒，两不同条件均无expiry/信号，未卡住故采样0。原C02
  95秒/KILL首败不覆盖，233events/取消状态只读复核全同；未诊断为钥匙串或特定线程根因。
- 仅加10行debug阶段记录：page close/context release/native entry/native return，数目/耗时不含
  profile或凭据，无新锁/等待/CEF调用/完成判定，release不变；Mac lib13/13。路径替换造成的一次
  launcher ENOENT先留、未启动进程后按精确原制品修正；不重跑同条件循环。证据
  `2026-10-02/macos/cold-exit-stack/`；记录版正式native未重编/重验，完整C05/C/N3仍开放。

- **MAC-C-03 正式连续/Stop/冷读补缺口**（2026-10-02）：源`e2f7e47ca`，arm64/APFS，正式Tauri
  `e270148c33ff…`；沿用C02原任务/纠正与16/4096/360预算，13请求/两压缩/335events/15原生参数
  与canonical全同。先成功READY poll再实际steer；一helper/一20-byte正确写，真实回读/hash和中文
  说明后继续cursor=60/wait30000 poll，无模型cancel/report。Stop前父子存活，866ms后检查均消失，
  cancelled/host_cleanup_proven及后续零派发；三原件不变。live Cmd-Q0/124.83秒、冷0/18.77秒，
  均无expiry/信号，debug native entry/return实际成对；冷读四表/全部文件全同、UI空闲/保留纠正。
- 四拒绝原样保留：new-input阻止一poll，plan+read+poll整批三拒绝后恢复；未证明准备状态公开报告。
  首审误把伴随poll的公开回复当无tool收尾，缺新review notice断言首败保留；新分支live未覆盖，
  不代判W248修复或覆盖C02冷KILL/占位首败。完整C/N3仍开放，旧MM未关闭，不追加未改根因循环。
- 零模型复用B09：原末LF要求在接受输入和供应商请求15/17/20均保持，三原始SSE写提议已缺LF、
  canonical参数全同；不能归因owner裁剪，不自动补字节。证据`2026-10-02/macos/mac-c-live-progress/`；
  首败/修后/完整DB与UI在外，凭据审计0/348，未改Windows结果；下一步只补A/B直接缺口及必要N3。

- **MAC-A-13 共享修复合并正式复验**（2026-10-02）：复用C03正式源`e2f7e47ca`/App
  `e270148c33ff…`，确认当前仅两进度页差异；原A任务/七文件及16/4096/360预算不变，无重编。
  7请求/两压缩/385events/12原生参数与canonical全同；读搜正确，pwd/ls/Git两条/指定两测试各
  一次，exit0/1均reaped，七原件/Git/哨兵保持。report首发旧三ID拒绝后仅修报告，无重做；
  中文报告/UI保留真实非零1及预检1、计数2/1，完成/实际Cmd-Q0/97.23秒，无expiry/信号。
- **整组仍FAIL_PUBLIC_DELIVERY_3_MISSING_RESULTS**：末请求07仍含精确cwd、完整含隐藏项列表、
  原97-byte文本/两搜索，答案却省略cwd/隐藏项名称/原文头尾；不能归因输出缓存丢失，不另加
  缓存或纯文案微调、不付费循环。操作主链N1不抵扣完整A/N3，结果行详情未展开。零模型复核
  A12三后续请求六组pwd/ls全文/游标也全同。close wrapper ENOENT、snapshot把.git当文件EISDIR
  及缺status首错保留后只修外置夹具；原完整385-row备份不替换，readonly复核/前缀/ok成立。
  证据`2026-10-02/macos/mac-a-shared-results/`，无产品源码修改、Windows结果未动。

- **MAC-GEN-C-02 / C05-09**（2026-10-02）：源`e2f7e47ca`正式App/命令减权General，原C任务/
  16/4096/360预算；12请求/一压缩/302events/9原生参数与canonical全同。先READY和一次公开
  准备再实际steer；首write因需replan未派发保留，二提议/一实际20-byte写、回读/hash/中文说明/
  后续cursor poll成立。Stop前父子活、883ms后检查消失/cleanup证明；冷四表/全部文件全同，
  未复活。独立General文档只减computer/automation.schedule，其余全同，不代判完整默认配置。
- 冷首败 **FAIL_UNVERIFIED_CLEANUP_ZERO_EXIT**：0/85.13秒无信号，却四轮清理未验证；实际
  native entry等待61.851秒后才return，page/context均0。一秒自有PID样本：主线程CEF shutdown，
  CEF foreground worker停SecItemCopyMatching→CSSM解密RPC；不外推所有历史原因或改钥匙串保护。
- 锁定Tauri/Wry2.11.2的RequestExit回调收到code却用ControlFlow::Exit，App::run在Tao直接exit0。
  改run_return后按协调器终态显式退出：未验证/未获终态不为0，已验正常/原非零/失败及restart
  所有权保持；两最小回归及原邻近共12项不同定向通过、正式Tauri/签名通过。修后同数据零模型
  正式冷读0/32.51秒、CEF12.412秒返回且无清理失败、302events/四表/六文件全同；未再触发失败
  分支，不宣称卡顿根因/N3/完整C闭环。原C02强杀、GEN冷首敗及断言红均保留。
- 发送前只读SQL14/观察器动态代码拒绝/暂白屏、冷尚活时snapshot ENOENT、打包替换路径及签名
  字节比较首错均在外另存；不重启任务/扩预算，改静态观察器及正确包复制前hash/签名校验。
  证据`2026-10-02/macos/mac-gen-continuous/`，凭据0/421；Windows原生/真实重启未验、结果未改。

- **MAC-C05-10 CEF按需生命周期**（2026-10-02）：沿GEN冷样本未用Browser却在CEF/Keychain
  卡住，启动阶段现只检查原包路径/二进制；首次已授权Browser资源创建才在主线程初始化。
  宿主持有单次结果，调用者取消不丢Engine；关闭阻止迟到初始化并等已启动结果，真实初始化
  错误仍失败，已用Engine仍原真实shutdown。无加密/Sandbox/权限/清理时限改动，Windows不变。
- 本机生命周期4/4、正式Tauri/签名通过；同旧cancelled数据正式零模型冷读23.05秒/exit0，无
  CEF初始化/加载后shutdown，只有unused_closed；原302events/四表/六文件全同，无复活。
- 独立Browser会话正式UI：首次使用前vmmap无CEF，实际POST Browser后仅一次初始化；本地真实
  页面点击计数1与独立witness trusted=true一致，模型0/Turn0。URL只在精确principal/session/
  binding hash profile，Default无该URL；真实shutdown119ms、native0/114.85秒，全部自有PID/
  listener消失。证明没有通过禁用Browser洗绿，不宣称已用CEF的系统钥匙串等待/N3/完整C关闭。
- 原GEN首败/线程样本/失败码红保持；本批是mac专属根因，不另改共享/Windows结果。证据
  `2026-10-02/macos/cef-on-demand/`；未跑无关媒体/生态矩阵，无新增付费调用或范围收缩。

- **MAC-C06-15 过去命令结果/当前文件状态策略矛盾**（2026-10-02）：A13真实请求7的高优先
  policy仍说后续process使旧命令证据失效/只引最新，与已advertised的历史terminal/output相矛盾。
  仅统一这两句：当前文件需当前资格，已reaped命令是过去结果、逐项引用matching advertised
  IDs，不以generic done/最后一条替代多结果；权限/Schema/资格/epoch/计数/预算未变。
- 强化原两命令缓存回归首红保留，修后及旧stale/instruction边界3项通过；正式Tauri/签名后仅
  一次日常pwd-P/ls-a任务，冻结6/4096/180（native300+5），实际3请求/127events/3原生参数全同。
  两命令各一次0/reaped，真实cwd/五名称（含dot/hidden/中文）在report/UI全交付，分别匹配两ID，
  report首发接受、error0/无其他模型工具或副作用，三原件不变；正常Cmd-Q0/67.10秒无expiry/
  信号/CEF初始化。完整readonly快照全同/ok，自有App/fixture/listener无残留。
- 不重跑全A/B/C，不宣称该矛盾是所有漏项唯一原因，A13旧遗漏与B字节/步骤残余保持。首审误
  限dot名称Markdown格式保留，另按实际中文枚举比对完整五名称，原数据不改。证据
  `2026-10-02/macos/delivery-policy/`；公共策略根因同步共享，Windows结果未代判。

- **MAC-B-09 字节/步骤责任核对收尾**（2026-10-02）：零新增模型/UI复跑。实际请求Schema中
  `expected_source`可选，不能把未带guard归为合同缺陷；首patch是在文件尚未创建时提出修改
  hunk，零publication拒绝符合合同。后续重读/独立replan门槛未放宽，已证明零效果不产生永久
  未修复义务。三次原始write已缺末LF、canonical逐项相同，不是codec/文件owner裁掉换行。
- 原样写入、patch保留末LF、零publication恢复、成功write只解除对应目标四个已有小回归
  本机arm64均1/1；无新测试/产品改动/新增付费调用，不把这些回归替代完整B或生成准确性。
  B09首败及B08精确文件链保留；生成层是否继续纳入本轮已请求用户选择，本轮不自行扩展。
  证据`2026-10-02/macos/file-generation-residual/run-001/`；无新公共产品根因，Windows结果未改。
