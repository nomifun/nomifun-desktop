# Windows 全局操作系统能力原生接续记录

记录始于 2026-10-17（Asia/Shanghai），两个发布阻塞的接续补验完成于2026-10-18。实际 checkout 为 `C:\Users\muli\workplace-c\nomifun-desktop`，最初在 `main`、HEAD `d1e968bfe262c39deeddcc460f3df27ab26ff0f7`，tracked/staged/untracked 均无改动。该提交已经包含完整共享全局能力及 Git 整合，不恢复旧 source-review patch、不 reset/clean。原始环境命令结果保存在仓库外 `windows-global-os-initial.txt`。

本机为 Windows 10 Pro 22H2 x64、19045.6466；Rust 为 1.99.0，host/installed target 为 `x86_64-pc-windows-msvc`，Bun 1.4.2。真实登录身份为 `DESKTOP-NF7D5NP\muli`、Session 1、普通非 elevated token；USERPROFILE 为 `C:\Users\muli`。原始日志及报告位于本任务可视化目录的 `windows-native/`，JWT、私有数据集、截图、二进制和测试目录不提交。旧 macOS 的签名/原生报告仍属于[原三平台矩阵](global-os-2026-10-16.zh.md)，不计入本机结果。

## 发布阻塞修复接续

本次从干净 `codex/windows-global-os` / `59d8c0db3061684faa972528982d204b97a49631` 接续，只处理跨项目持久 native 目标冲突与 UAC typed 错误链。核实远端 main 为 `737c6046bf7c823f674ca7cbaa4ad66bbdcd22ec` 后普通合入，merge `22094a49be31a911a7a4ec2c0d5527b14887954f`，没有 rebase、force 或覆盖无关工作。同一PR26已附加，首次 API 状态为 draft/open/mergeable/clean；当时 main 未配置分支保护或 ruleset、无 review 和 check run，不能将空的状态组合说成 CI 已通过。最终普通提交、推送与合并由实际Git回执核对。

首次命令结果、PR/rules 只读回执及后续日志保存在仓库外 `C:\Users\muli\.codex\visualizations\2026\10\17\01a14a71-8603-72e1-b928-fe93567f3dae\windows-native`。真实登录 token 核对 Windows10 Pro / 19045 / x64、Session1、Rust1.99 MSVC、Bun1.4.2；CurrentUser 与 LocalMachine 均无代码签名证书。原 UAC 失败报告实际 SHA 仍为 `83d45f1170417b4123d39739b6461b0a5ed048ee31a9c6d0e26e282e1ec1074b`，没有修改历史失败或34d/BoHTo0。

源码将 Windows 本次 pinned target facts 保存在原 Effect started payload，原写事务原子检查重叠；不改 baseline/checksum，不增加目标表。publication 与动态目录显式使用保守卷范围，旧缺乏目标事实的文件 unknown 保守拒绝无法证明不相交的新文件修改。其他平台保留原文件与 project/resource 合同；任意 process/shell 仍为原 opaque Effect，不声明精确文件 footprint。详细边界见[当前架构](../architecture/global-os-capabilities.zh.md)。系统认证沿原 native→AppError→Kernel/Runtime→canonical failed Turn→API/renderer 保存 typed code、真实 OS 值与已知未启动/清理证据；unknown、普通 ACL 和真正 binding 错误分别保留。

当前已取得 Session 全套267通过/0失败、含8项新准入/别名/多实例/重开/unknown/旧事实反例；独立 UAC/Host 源码审查、定向 UI 9通过/191断言、完整 TypeScript 和 desktop boundary，以及初轮文档检查通过。最终冻结、修前/修后 App 回归、Windows UAC/文件原生、新 App/NSIS、正常退出/安装数据保留与普通 Git 合并仍按后续实际回执分别记录，当前这些检查不能替代它们。

隔离修前源码的首轮 App 测试尚未运行便在 ORT 链接失败；实际环境继承 `CARGO_NET_OFFLINE=true`，导致其占位错误符号与 `OrtGetApiBase` 未解析。保留 `cross-project-baseline-ort-link-failure.log`；只在后续构建进程移除该变量并保留 Cargo 的 `--offline` 参数，不改全局环境、依赖、ACL 或生产逻辑，也不把链接失败当成已复现目标冲突漏洞。

纠正构建环境后，隔离修前 `22094a49` 仅加入回归测试，真实普通登录 token 下实际失败：第一个项目已写入并丢失 canonical settlement，数据库关闭重开后第二个项目仍真实返回 `written=true`、`created=false` 并改写同一外部文件。测试为0通过/1失败，actual101、1.57秒；`cross-project-before-fix.log` 和 `cross-project-baseline-counterexample.json` 保存原输出、源码/tree、测试 patch 与 executable 摘要。这是真实 FileService/Store/Windows I/O 反例，不是编译失败、镜像 predicate 或旧生产二进制的修后证据；修后相同接缝及真实进程重启另验。

### 2026-10-18 明确的外部并发边界

用户明确选择“按这个边界简化实现”：NomiFun 协调自己准入的显式文件 Action；用户或其他程序并发编辑文件、链接与 staging 名称的外部竞争由用户协调。不再把抵御任意外部编辑、始终冻结后续 I/O 对象作为本轮发布条件。原 Effect、幂等、部分结果、unknown、原生清理与禁止自动重放保留；任意 process/shell 不因此获得精确文件 footprint，也没有增加权限模式或新执行设施。

该决定建立在真实独立夹具的结果之上。`windows_native.exe --file-symlink-fixture` 经用户在系统 UAC 允许后，原系统 runner 创建四组同卷/跨卷 source/target FILE symlink；C/D 的实际卷身份不同，worker32312 exit0/reaped、helper32684 exact shutdown/facility exit proven，六个唯一根保留。创建回执 `file-symlink-fixture-native.json` SHA-256 `3684b054bd22c2b32d46232c4f7284298634a734199a142ddf376f0d38adb399`，与旧 Agent 数据无关。

原严格 probe 首次执行被自动审批在派发前拒绝，理由是尚未证明精确修改目标的授权；只读 marker、owner、FileID、内容、ADS/SDDL 与创建 manifest 核对后，同范围执行获准。`replacefile-probe-owned-fixture-preflight.json` 与四份原日志/各目录 `probe-result.json` 保留，未使用旁路审批。target 两组均原生1464拒绝，外部 bytes/metadata 未变；source 同卷组实际移动了外部 sentinel，跨卷组 bytes 保持但 metadata 已变。两组 source 原严格断言实际101，均保留未验证/清理未确认结果，未伪装为成功或确定未执行；该独立库诊断没有 Agent Effect，不等于 canonical 对账。`replacefile-final-symlink-native-summary.json` 保留这两个失败，不重跑已消费链接或清理部分结果。当前 opt-in 诊断改为验证外部干预下的诚实不确定结果表达，不再要求超出新合同的外部不变保证。

整合 App 测试首次编译另发现 `caps_creative_studio.rs` 穷尽匹配漏掉新增 `AppError::SystemExecution`，在测试执行前101；已补该领域自己的明确系统执行失败显示，不将其误报 Canvas 不存在，原编译日志保留。源码首轮完整 `bun run check` 实际0；最终冻结与原生 App 的 UAC 取消/允许、跨项目围栏和安装证据仍以之后回执为准。

修复保存并普通推送为 `61e20d3adb02a826ebae92295f70a2463835c492`（tree `f48f2d986bdb83346aea5fe9e1ae9dc6a94f5254`），同PR26已更新且保持draft。完整 desktop browser/computer 特性的 App 测试编译 actual0、8m58s；该测试程序 SHA-256 `9d39dfb3864ea7a8fe563cd5641a04f927f11dfd910b19f8a6f667993b1f1a0e`。修后同一真实外部文件/后来硬链接的跨项目 pending/reopen 反例通过，真实独立两进程 restart 通过；readonly投影7项、canonical冷API历史1项通过。

进一步 Host98项首轮为82通过/15失败/1忽略、147.84s，原 `app-wave2-host-regressions.log` 保留。失败主要是已经得到正确 typed native conflict 后，旧夹具仍要求英文小写`unsettled`；另两项NUL输入已在native准备阶段拒绝，旧夹具却等待派发后的settlement hook。后续仅校准测试：Windows按精确机器码、Effect缺失及文件不变判断；NUL用明确的准备失败/无Effect/无owner派发回归，非Windows原路径保留。泛用owner错误结算测试保持，不改生产准入或自动重放。该轮曾过早生成“主动停止”元数据，但工具实际返回完整自然FAILED结果；`app-host-first-run-completion.json` 明确更正，原元数据与日志均保留。修后Host完整结果另记。

### 2026-10-18 冻结构建与最终补验

修正测试保存为 `9e9bcefccf4b60f7aaa00b5927ae7a0a4fbff84e`，随后普通合入 main 的 `ba107fa011ddc8cf92d68033504a6bdbf4f774df`，形成生产冻结 `53da7b1cf5d5bb9eca6fd3ed255625de7d3a2199`、tree `860cd8804a50311d3b601971eabc3beb1ba332ea`。该来源及锁文件、实际命令、完成码、Cargo组件输出、完整exe/NSIS摘要和排他复制均保存在仓库外固定副本的 `receipt.json`；不会把最终验收器或文档提交称作另一次产品构建。最终Git合并还须核对实际main及其相对本冻结的产品差异。

| 本轮产物 | 真实SHA-256与身份 |
| --- | --- |
| debug App | `a78a41e7466bfed71f0b0900274d57c5ace20a193c480baf30cc462006fff8b3`，312035328 bytes；`cargo build --locked --offline -p nomifun-desktop --jobs 1`实际0、19m14s。 |
| release source exe | `02d1d57dacf743e46e007818eb5f0b2b9c22946eb8d8e0e08360bcf4f8db4756`，262172160 bytes；现役`build:win x64`、Cargo release34m08s、NSIS实际0。 |
| 唯一b69b NSIS | `ff4577cc8364b547cae8fb0525354940277a9031a4bda5c46cc0be012c5adf84`，76207744 bytes；只消费本轮`b69b852613f34efa999bfa09502e95aa`，不把dist中两个旧变体混入。 |
| 实际安装exe | `72d9e82dae99219375c0119759103a86928f6693a2cf110350d6abdee653abdc`；完整源图像按官方首个27-byte UNK→NSS变化预测，与安装后全文件摘要一致，不忽略PE区域。 |
| 组件与签名 | 实际组件`4c18db657c42cd197352f3f7bebbf8175d2a7541aded985ede118e4af24b3880`已按源码重算，并在Cargo output与两exe流式核对。source/NSIS均NotSigned；无正式Authenticode发行声明。 |

源码验证分别记账：Session267通过；9e9b完整Host为98通过/0失败/1个由父测试实际调用的ignored重启子入口，binary SHA `1b06e0b26e3558c5b83723842ceeffcbc63b439f010fc060e99cb35f5b9a0cd4`。最终53da重编后，companion取消3、跨项目及真实进程重启2、readonly7、冷typed API1共13项首次通过，binary SHA `4ed5752cc11adc45cc7ca2a17c63d3bf9a16fa6e215650ecde5b7d49e027e35d`；新增scheduler取消2项通过。两次来源不同，不混成一次完整Host重跑。system-execution9、native-files12通过；未改动的process-runtime首次并行146/1原失败保留，精确故障用例与完整串行147/147通过，不猜原因或改生产deadline。UI/合并控件26项及新增取消UI9项分别通过；最终验收器10项/84断言通过。完整`bun run check`在合并源码及最终验收器修正后均实际0，保持880×600及所有Agent/进程边界。

原87f数据根的升级App15356使用原owner/JWT和Desktop230/UNC336双守卫；新App两个不同项目各提出一次同一外部missing目标写入，均在新增修改Effect前被原legacy pending拒绝。两案零Effect、marker缺失、pending0/unknown0；原336条事实逐项digest、binding/owner及pending1完全不变。`default-agent-legacy-target-fence-308a2983-ff2a-4cf9-8aa2-4bbbd134ceae.json` SHA `339277dcc8d60782f17c946566bb60ad1de06ac9bc5309389ac4d597453bf68e`与独立复核保留。此拒绝code在通用ToolCompleted中是原诊断token，不冒充独立typed canonical字段；允许内部只读及一次Host mutation intent，不称零工具dispatch。管理员Minimal/General和自然等待、精确取消3/3也在该原数据根通过。

真实UAC取消分开消费：首72ec场景中用户明确点击允许，exec Effect实际成功、exit0/reaped；取消专用provider拒绝继续而使Turn失败，原FAILED不改成功。原App正常exit0、9个owned进程gone后，仅对已证UUID marker及两个空目录做守卫式非递归清理。随后同exe/同数据根重启App32752，独立674调用实际返回`SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED`、kind=Uncategorized、raw1223、未启动用户代码与cleanup已证；同owner清理先于Runtime/canonical failed Turn，拟执行的修改无ToolStarted、dispatch、Effect或marker，不当作用户Stop或成功。

674原报告因验证器错误保持FAILED：失败API使用error Tips而非turn_summary，诊断checkpoint可保留且不授权重放，instruction_result使用当次typed carrier的digest隐私投影。只修验证器的这三处现役合同并增加回归，没有改产品来迎合断言。独立GET-only补证`uac-674-independent-get-readback-f019fc55-de44-4227-a326-8b41e8b2cd15.json` SHA `003f71bf61da6a5a2fafee79fbef74385ba3ef9b4b75a7eb5e012156a3972685`核对同一原Session的API Tips、typed canonical错误、checkpoint seq/revision/digest、noReplay和1.5秒无后续事件，实际通过，未重提Turn、认证或mutation。renderer结论来自UI/consumer测试，native结论来自实际API/canonical，不声称拍到了错误消息的原生UI渲染。

独立正向夹具使用同一debug App的8a标记数据根并正常setup/login，不能据其成功追认旧pending：普通Minimal、General及missing-default各12、UNC16、Browser13共65业务步骤通过，另有24个discovery通过。UNC四次已知派发前instruction延期均有零用户派发及参数未改证明，后续实际步骤通过，原计数保留。`positive-debug-native-53da7b1-readonly-aggregate.json` SHA `68c901044411d2e2bc04365e6c35d9ecdb21453364f444db9925e19c60e5e541`保存五案来源，均completed/answered、pending0/unknown0且owner cleanup已证；唯一UNC share95f488dd已canonical删除。Browser本次kkRpD4在App32220正常exit0及15个owned进程gone后实测absent，没有host删除；旧BoHTo0仍保留。

新b69b的canonical安装/卸载在同Session两Turn六业务通过，报告`installer-chat.minimal-1792258083859-007a102a-90cf-44b6-aaa3-b98ce7fed0c5.json`保留真实installer与复制uninstaller的exit/reaped证明。唯一对象缺失后独立安装报告0eb22ad2，再启动实际NSS安装exe：PID23640、普通RID8192、嵌入renderer `http://tauri.localhost`、1280×832、正常setup/login和Minimal12通过。App正常exit0、7个owned进程gone后，独立卸载`windows-installer-uninstall-102afe18-8bec-447e-bec7-3a4a0d787725.json`确认唯一目录/注册/快捷方式移除，真实closed SQLite完整摘要及保存哨兵不变。标准卸载证明首进程退出加最终目标缺失，不冒充另有self-copied child的retained HANDLE；前述canonical复制卸载另有更强证明。唯一currentUser变体不等于日常产品身份升级验收。

清理分别有证：四个新App的owned tree为9、7、15、7项，均在正常产品退出的retained HANDLE实取exit0后独立gone；单个close回执仍标overall_cleanup_proven=false，不替代独立tree/snapshot证明。自有Vite原31280代已退出、5173无监听；其Ctrl-C wrapper exit1不混作App失败。所有测试App退出及b69b卸载库存核对后，`nomifun://`经固定日常0.8.4完整摘要、精确任务exe、既存值guard和双视图读回恢复，实际仅写两值，回执bea5ffb6 SHA `59a6acd812e5395c9293780106af8aff88d1a5661680d8116d1058bd6ee7991a`；日常exe未启动或覆盖。已结算空夹具只做非递归清理，DB/JWT、原34d pending/文件/备份/share、BoHTo0及已消费外部链接干预的部分结果保持保留。

首次失败另保留：legacy辅助验收前缀错误在mkdir后、所有API/provider/Session前拒绝；安装报告的空注册表值名使PowerShell对象解析拒绝，改用Hashtable后才首次启动安装版；两者均不是已派发业务的重试。最终候选仍未签名，正式Authenticode证书、其他管理员账号及额外硬件矩阵不由上述回执补造。本机scripted provider证明默认Agent产品路径，不证明联网模型推理；无日常安装替换或正式渠道发布。

## 上一批交付结果：925 生产冻结

上一批925当时的完整实现与原生补验已完成，生产源码为 `925e775a1c070fbbef75d8c2db0bbb5c27ce97e3`、tree `55a42d1b26a70c28faf9b11e4709c1f7ea72f36f`，含普通合并main至2c6及真实UI准备owner修复。当批其余变化仅为等待验收器和本记录/交接，不借验证文档提交冒充另一次production build。完整提交与普通推送以 `codex/windows-global-os` 和 [PR26](https://github.com/nomifun/nomifun-desktop/pull/26) 的实际Git回执为准；没有改写main。

| 证据面 | 最新结果 |
| --- | --- |
| 源码/测试 | 925完整 `bun run check` 实际0；Rust876通过/0失败/1保留忽略，App14通过/0忽略，UI480通过/0失败/2739断言；最终等待driver44项纯检查与独立真实事实复核通过。 |
| 当前debug | 8dbc executable/component21dc：UNC16、管理员8、missing-default Minimal12、Browser13均completed/answered、pending0/unknown0、cleanup_proven。 |
| 等待与取消 | 新唯一完整3项PASS：普通/管理员由原owner自然唤醒、精确取消闭环；原process_id/输出、冻结binding、typed清理及retained HANDLE均核验，未知操作未重放。 |
| release/NSIS | 925官方构建actual0；唯一unsigned/currentUser b4变体canonical安装2步/卸载4步通过，实际NSS完整摘要/9项资源/登记/数据保留核对。 |
| 实际安装版 | 同安装NSS image正常setup/login后Minimal12通过，exact App exit0/7项owned tree gone；独立NSIS卸载actual0，唯一目标消失、真实SQLite完整SHA与sentinel保留。首次准备失败另存。 |
| 最终清理 | debug exact exit0/15项owned tree gone，新WgkGMz快照原生absent，自有Vite停止；本轮已结算gate/空目录仅非递归清理。全部测试App退出后，日常nomifun://已指回核实的0.8.4，双视图读回成立。 |
| 保留与边界 | 原34d pending1/336事实、文件/备份/share与BoHTo0保留，未伪造人类核实；媒体按用户选择未采集，正式签名/其他账号/额外硬件矩阵未认证。跨项目持久目标重叠围栏仍缺，原Store围栏保留。 |

所有native产品结论限定于本机scripted provider和对应binary，不证明真实模型推理。后继main737c未纳入本次构建，实际只读合并计算无冲突；不将不同来源的旧Desktop/SCM/Task结果借为925全矩阵认证。下文保留历史批次、首次失败、修后结果与各自摘要。

## 第一批91410的已完成结果

下表是已完成的91410生产冻结批次。PR26创建后，main继续前进到 `2c6b1df36fc5f42b08c8dc296c77b49954ce467d` 并形成三份生成合同冲突；已保留0590完整交付与全部原生证据，继续普通feature合并及新源码补验，不改写main、不将下表借用为新合并树认证。后继批次以本文新增结果和实际Git回执为准。

Windows 实现、本机原生回归、独立安装版初始化/业务/退出/卸载和限定测试资源清理已完成。生产源码冻结为 `91410efd5dd3227104b783bf794591254c014d82`，其后仅有验证脚本、测试和文档收尾；完整 Git 交付使用 `codex/windows-global-os`，不改写 main。下列 PASS 均限定于对应来源和报告，不把中间构建的旧结果合并成全平台认证。

| 证据面 | 最终结果 |
| --- | --- |
| 源码检查 | 合并生产源码 `bun run check` 退出0；Runtime458通过/0失败/1忽略，定向 knowledge/system/App33通过；最终验证驱动49通过/0失败/1 POSIX跳过、454断言。 |
| 最终 debug 产品链 | UNC16、管理员8、missing-default Minimal12、Browser13业务全部 completed/answered、pending0/unknown0、cleanup_proven；普通与高完整性桌面、SCM、跨App存续Task等较早证据保留各自binary范围。 |
| release/NSIS | 官方 `build:win` 构建成功；唯一 unsigned/currentUser变体完成 canonical同Session两Turn安装2步/卸载4步。首次链接配置错误与打包标记验证器失败保留。 |
| 实际安装版 | NSS executable完成正常初始化和登录、Minimal12业务，通过原产品退出/retained HANDLE实际exit0，再由原生NSIS卸载；唯一目录/登记/shortcut消失，真实数据库完整SHA与sentinel保持。首次准备失败单独保留。 |
| 最终清理 | debug15和安装版7个exact owned进程均gone；本次lmBztb快照absent，自有Vite停止。只对明确归属空目录作非递归清理；日常nomifun://已指回验证的0.8.4安装。 |
| 原故障保留 | 旧34d UNC pending1、336事实、文件/备份及旧BoHTo0快照作为有用证据保留；未伪造owner核实、未对账、未重放，不用新成功覆盖。 |
| 明确边界 | 本机scripted provider不证明真实模型推理；媒体采集依用户选择未覆盖，正式Authenticode/其他账号UAC/额外硬件和卷矩阵未认证。跨项目同一外部目标仍缺持久重叠围栏，原owner/project Store围栏保留。 |

历史候选 `4b6bb712…` 的真实 UNC patch 在发布后留下备份清理/补偿未确认，原 Session `01a14893-93f7-7a90-b573-bc0b31116207` 的 pending1 围栏及准确 share/文件仍保留，未重放、未清理、未通过关闭 App 或新数据集绕过。`journal_settlement: unconfirmed` 在此分支表示源码主动保留 pending，并非曾向 Store 提交 terminal 而失败。原 cleanup I/O 错误被旧代码丢弃，不能猜测为特定 OS error；当前文件、备份或独立 probe 的状态均不追认原操作结果。用户曾在该 App30552 的真实 UAC 窗口允许；对于原 patch 核实问题，用户回答“不知道”，未产生 owner verification 或 reconciliation。修后最终源码、产品回归、正常关闭及安装周期分别见本文后部；修后成功不改变这份历史失败，也不能把技术取证代写成人类核实。

## 已执行与保留的失败

| 检查 | 本机结果与范围 |
| --- | --- |
| Git/工具链初始库存 | 已保留实际输出；未应用历史 bundle |
| process-runtime boundary | 通过；首次受限沙箱 Bun 启动只读 git 子进程被 EPERM，使用正常登录 token 重跑通过 |
| Agent Session boundary | 通过；同上保留受限 token 的首次 EPERM |
| external-tools boundary | 通过 |
| Windows installer contract | 通过；保持 currentUser 安装位置与原数据保留合同，未新增 uiAccess manifest |
| 完整静态检查 | `bun run check` 退出0，包含 typecheck、880×600 desktop boundary、i18n、原 Session/process/browser/hosting 边界及 installer source；后续文档直接检查亦通过 |
| Windows script argv | 三组原生通过：Pipe/ConPTY/ChildProcessBuilder 的 cmd/bat literal roundtrip、CR/LF 启动前拒绝、PS1 literal 参数；首轮混合分隔符 launcher 与输出流 fixture 失败保留 |
| 现役 Windows process 定向 | 11 通过、1 个 opt-in 环境用例未启用；resolver 定向通过 |
| 现役 Windows process 完整 lib | `cargo test --offline -p nomi-process-runtime --lib` 在已有独立 build-dir/target-dir 下136通过、0失败、0忽略，原生测试3.66s；真实Job/ConPTY启动顺序、取消、后代、shutdown及不可证明清理合同保留；日志 `process-runtime-full-lib.log` |
| native-files Windows 专项 | 6 项真实登录 token 原生通过，含 Volume GUID、alias/ADS/device、junction 开前竞争、scope hidden/链接与真实 native5 的 source/scope ACL 拒绝。首次沙箱 token 无 WRITE_DAC 及后续 ACL fixture 精度错误分别保留，不从源拒绝推断 metadata 也必拒绝 |
| FileService 最终 Windows 专项 | 47通过/0失败/1忽略/2过滤；忽略的C/D跨卷用例单独启用后1通过，保留native17与原源文件、无复制删除。两个大小写目录用例因本机原生OS50另记未覆盖。父目录unit4通过，含RootDirectory相对rename、junction并发native1921且source不变/outside为空。真实SDDL反例证明ADD_SUBDIR不要求ADD_FILE、child DELETE拒绝也不额外创建guard。首轮37通过/8失败、后续missing-parent预检native2被误写成rename native3的夹具失败均保留；未改生产错误迎合断言 |
| Browser 文件 worker | 最终普通worker8通过，独立启用真实ACL与D盘源2通过；共享pinned handle/metadata/二次摘要/取消收尾保持原Browser预算，文件名取真实native语义。ADS无法由Web File.name准确表达时在effect前明确拒绝。真实WebView2文件input产品场景另列，worker不替代产品链 |
| system-execution 编译/协议测试 | 最后6项原生通过：2MiB 管道完整回复/ACK、OVERLAPPED timeout 的 CancelIoEx/完成 join、独立 logical client/关闭后禁止新 identity、真实 shell/terminal/cancel/reaped、未安装宿主入口不触发 UAC，以及 extension 在 quiesce 后仍可用、shutdown 后永久关闭准入。独立最小输出目录首次5项耗时56.96s，最终6项1.04s。保留首次受限 token 用户 DACL 拒绝与编译错误；不是管理员身份或产品链证据 |
| 独立真实 UAC/native 组件 | 用户在系统 UAC 窗口允许后 PASS。`system-uac-first.stdout.json` SHA-256 `28332c998d31287ce1cb942bde56fde904987480ccc664436e4359b287df803a`，component `b3c27db4cb244fd0beba88431921c9fa5b6d81a0cdb0833f90a02ec3753b22f5`。helper6608 elevated=true/同登录SID/Session1；readonly worker3912 完整 typed JSON、exit0/reaped/无stderr或丢字节；等待进程28828 cancel/reaped151ms；exact shutdown、等待runner实际退出且独立PID查询三者均消失。该次 source 为本测试普通可读文件，不能冒充管理员专属ACL fixture或默认Agent产品验收 |
| 后续 UAC 生命周期与宿主退出 | component `46d8aed557f4ece23683f329309c44616b022724ae08a4231c08c6deef101fa0` 的两次独立真实UAC报告通过。正常5组包含readonly、Pipe取消、stdin/close/natural exit与晚到终态I/O、deadline、ConPTY resize/stdin/cancel；helper及各确切worker PID独立消失。`system-native-lifecycle-069eef97-5112-44cf-9960-517bf9a3e80c.report.json` SHA-256 `0d453a0e309c6de99a180bef137254561e0c9df1a6718ab90595f49689222f83`。另一次parent17040自行exit23，没有请求cleanup；helper7668、child13100及descendant30684在原生创建FILETIME身份的首个独立观察均消失。`system-native-parent-exit-8e030ab9-ea58-438a-8796-f19506218c3b.report.json` SHA-256 `0380fd45c7c1c92a51ad1a563403e7775a2826970d11e53f481fdde4728d9418`。这两份不含canonical Agent数据，不能替代App/Store故障恢复证明 |
| 完整官方Windows Node/npm | opt-in原生测试34.47s通过：独占离线prefix、本地包安装与postinstall、真实USERPROFILE、空/引号/中文/元字符CLI及npx、distribution删除修复、runtime cache删除后CLI保留、独立进程重启和卸载；未更改日常用户CLI。完整lib首轮48通过/1失败/1忽略，失败为原Unix tar/symlink夹具在Windows先触发权限错误；改用Windows ZIP夹具保留wrong-tree摘要拒绝断言后，最终49通过/0失败/1忽略，1.65s。该忽略项就是前述单独启用并通过的完整Node用例 |
| MCP stdio 原生共享启动 | `cargo test -p nomifun-mcp --lib owner::stdio::tests -- --nocapture` 最终8通过，1.30s；实际PS1入口经共享 prepare/ChildProcessBuilder 的 explicit PATH、空/中文/quote/元字符/换行 literal argv、JSON-RPC catalog 和 managed cleanup 均通过。首次过滤器0匹配与WinPS5.1数组夹具错误分别保留；修复仅调整测试夹具，未改生产编码或用户环境 |
| 首次新App普通产品链 | 首次完整MSVC App构建退出0、13m17s；不可变验收副本SHA-256 `57a1703094e8544df4c4515ba71bf7ff8a2034c12bde540bdf41455c7dca9276`。Minimal与General完整普通链均通过，pending/unknown=0，无手补grant/resources、无UAC。Minimal报告 `files-process-chat.minimal-1792208607112-96dfa080-9159-4734-ab13-f1df2cb213ff.json` SHA-256 `3fbdc629abdcbacd5229deee8409544ca7ef19af42d7c34a67394bf5c80e0e7e`；General报告 `files-process-assistant.general-1792208652923-93a3af16-31f6-4d2e-b9ba-fbf286eb1f8f.json`。fixture PID31104经产品正常退出并独立确认消失。该副本是构建中源码更新前的中间产物，最终管理员/桌面等必须使用最终增量后的独立构建 |
| 预存旧 App 基线 | 保持 FAILED；旧 exe SHA-256 `68878c204d7838b9fa6d7364112a72a1c56111239637bcbf3a728cac1b4c5642`，自有 PID12516 的 Minimal 缺 read_file、General 要求手工补资源，未补 grant/resources。旧 debug WebView2 起初没有 devUrl 服务器，IPC 退出失败；启动本仓库 Vite 并 reload 该自有页面后通过正常产品退出，确切 PID 消失。失败数据集保留，新构建必须重跑 |

## 当前源码与验收入口

以下新增证据来自中间 App PID12892，exe SHA-256 `fa7c9c9b498b8f16ff463de2a4ea829c7a1fe73edfff5c57f6a7494cfb81365b`。它仍以真实登录用户、medium integrity、Session1 运行；用户在 Windows 原生 UAC 窗口允许后才建立管理员 runner。后续源码修复不追溯改变这个 executable 或其报告身份。

| 原生产品证据 | 实际结果及限制 |
| --- | --- |
| 管理员文件、指令与进程 | PASS：`administrator-assistant.general-1792211360956-26dd0029-7866-42e3-9263-54e2bd75f3bd.json`，SHA-256 `9325bf1c12474f4d04eeb9397bb692b28a449afe033fcb8d4d8c85b4158d1805`，Session `01a1481f-2363-7271-b485-badef0dfbc9d`。唯一 BA/SY ACL 夹具下 ordinary read/write 确实拒绝；正式 administrator read 获取源，root AGENTS 的 scope/text/指令更新先于 fresh process proposal。真实 high process PID31504 与登录 SID、Session、USERPROFILE 相同，cwd 与显式 PATH 加持久 user CLI 路径精确匹配；独立 running poll、cancel、terminal/reaped、PID 消失及夹具清理成立，pending/unknown=0。前四次 driver 的范围识别、Payload 引用、PATH 断言和高进程 PID 查询错误均保留原失败报告 |
| 普通桌面聚焦 | 原报告 FAILED/unknown=1（后经 owner 核实追加 seq79/80，当前 rejected/unknown=0，见后文）：`desktop-assistant.general-1792210779991-f7d25612-febc-49e6-8433-4b5fa7156a42.json`，Session `01a14816-4704-76e1-bb6b-125f6f48138d`。本次 Calculator PID15300/HWND14291656 的原 focus 在 seq73 started 后 seq74 返回 `ROLE_HOST_PROVIDER_FAILURE`；原 OS 前台锁拒绝路径可能已经发生窗口变化，pending=0、unknown=1。未重试输入、未关闭该 Calculator、未换数据集规避围栏。延迟观察前台属于其他进程不能证明过去结果。原 Session 全78事件前缀另有摘要保留；升级也须逐项核对。只有真实 owner 核实后经原 reconciliation 追加事实，不能用登录 JWT 或修后 mock 充当人类证明 |
| Browser 文件上传 | 实际 WebView2 文件 input 已接收绝对路径、合法 `..`、junction 三类来源，名称/修改时间/摘要及源改动、源删除后的 retained File 生命周期验证通过；独立触发和新 document dispose 完成。整份报告 `browser-assistant.general-1792211636079-3639d5fd-8ed0-43d5-b486-522b4254c552.json` 仍 FAILED：末尾正式 delete 的 instruction preflight 把链接内容 scope 与条目 scope 混用。生产修复新增 Entry 语义后须新 App 重跑 canonical unlink，不能把 worker unit 或随后 scoped process 清理算作 file tool 成功 |
| 失败 Browser 夹具清理 | 两份原失败 Session 已在其现役链中清理，无 unknown。第二次清理 PASS：`browser-scoped-cleanup-9684811e-d377-45d5-b111-1a37c9dca535.json`，SHA-256 `cd3a7f6a2f4db5971ff88028dc0bcbb4901749cae3d30cafbff7f89854a6b09f`。只对准确 UUID fixture 用非递归 process 删除 junction，核对目标快照不变，再 fresh canonical delete 自有 root；原失败和 canonical 前缀不变。第一清理业务完成但旧 driver 完成计数断言失败，原 failed 保留；只读 readback 单列成功结算、原生 absence 和0unknown |
| 当前系统任务、配置与 loopback | 中间 App 四阶段24业务步骤已 answered，pending/unknown=0：安装/配置9、HKCU/Task7、loopback6、恢复配置2。唯一 InteractiveToken/RunLevel0 Task 的真实 PID32000、同 SID/Session、PT15M 限额及准确 action/workdir 已独立核对；用户配置已恢复。`system-chat.minimal-1792212137814-6b79c1d6-00ba-4561-893d-d546808dfa76.awaiting_app_exit.json` 只是等待阶段 checkpoint，不能声明 App 退出后同 PID 存续；后续结束或清理须另记真实回执 |
| 原生媒体设备 | `windows-native-media-28468377-182f-48af-a96c-2171e8b03650.json` 的实际 WebView2 secure context、1280×832 页面中，库存 audio_inputs=0、video_inputs=0、audio_outputs=1。audio/video 两次均 `NotFoundError: Requested device not found`，无 stream、无 AudioContext，cleanup_proven=true。没有读取、保存、录制或上传媒体内容；此结果不能推断采集成功、许可拒绝或模型/ASR 成功 |
| 聚焦修复的非物理验证 | 新实现只请求一次 `SetForegroundWindow`，明确未执行与 accepted 后 uncertain 分别结算；去除 AttachThreadInput 和请求前 Show/Restore。A11y33通过、Computer124通过/7实际设备 opt-in 保留忽略。原 unknown1 曾保护同一物理桌面，未以裸 native 测试或新 Session 绕过；现已依据用户亲自核实经原 owner 链追加结算，修复后的真实普通/高完整性完整桌面仍待允许条件下验收 |

Entry/Follow 修复后的本机回归为 native acquisition7项、FileService instruction scope7项、global files3项、Windows file47项和跨卷 rename1项均通过；Agent Runtime 全库444通过/0失败/1保留忽略。正式 schema 新增 `path_semantics`，仅 instruction_scope 可使用；管理员 owner 必须向原 worker 传递并核对同一语义，错误 Follow 回执不能降级接受。bundled 合同继续沿原 idle/CAS/未知效果及 checkpoint 围栏演进，不扩展 Action、效果或资源权限。

原生构建还保留如下独立失败，不能由旧成功覆盖：原自定义深层 release 构建目录的 MSBuild CMake compiler probe 已实际生成 exe，但 tracking 阶段对存在的 tlog 目录报告 DirectoryNotFound；同目录可由 .NET Framework 只读枚举，同源短目录独立 MSBuild 构建通过，新更长对照目录另有明确 MAX_PATH 失败。因此未声称机器缺少编译器，也未修改系统 LongPaths/UAC/ACL。当前改用仓库正常较短 build layout 重新打包；安装/卸载与正式 Authenticode 尚无通过证据。

最终 process lib 复验曾为136通过/1失败：旧 deadline 测试要求事件恰好 Created/Assigned，合法的更早超时也会触发同一失败文案，故该红灯本身不能证明曾调用 Resume。独立审查另发现用两次 `now` 将原 deadline 转成 remaining 再重建会延长绝对期限，已改为单次 anchor 与原 policy deadline 直接取 min。新夹具先通过原 facade rendezvous 证明实际进入 assignment，保留同一个 suspended child 至原绝对 deadline 过期，再独立核对无 Resume、无用户代码与确切 PID 已回收；另测更早过期无 Create/Resume。定向1项及最终全库138/138通过，日志 `process-libs-final-absolute-deadline.log` SHA-256 `d6f2e8dbce830633f6ff642dcb67c9eb606f1fec1ca560a156ad431a42af5aa5`；原红和旧 exe 定向10次通过记录仍保留。

WinPS 模块兼容的真实对照也保留：原继承值能加载父脚本的唯一自定义模块但 Get-FileHash 失败；删除继承值能恢复标准模块却丢自定义模块。最终只给已选 WinPS/隐式 PS1 的 ambient 模块路径补可信 Windows 系统 Modules 前缀，保留完整旧值；显式值包括空、其他程序均不加工。修后 runtime3项原生通过，覆盖实际自定义模块与 Get-FileHash 同时可用、PS1/绝对与相对 WinPS、两种 shell、literal argv 及真实 cleanup。Terminal 内部 hook 也移除隐式策略覆盖，现役 hook8项原生通过。

系统等待阶段已经真实收尾：上述24业务步骤保留成功，但未在10分钟内切换 App，最终报告 `system-chat.minimal-1792212137814-6b79c1d6-00ba-4561-893d-d546808dfa76.json` 保持 FAILED，SHA-256 `02da51525988989fa106a9e0d1a9e0342dd2994159e5a63858a233141720991f`。同一 Session 新 cleanup Turn 已 answered/completed、cleanup_proven、pending/unknown=0；独立 `system-retained-cleanup-observation.json`（SHA-256 `668b438aa614459027f03bd9a0d5fdff34584530c76ef280bdc4b10039ff684a`）证明 Task、注册表和目录 absent，Task PID32000 与 loopback PID33108 消失，原 App12892 仍运行。没有留下 Task 或监听来等构建。

媒体库存另有无采集的 OS 证据：`windows-os-media-inventory-readonly.json`，SHA-256 `22b576958ef6c19ec240c681ff5dafa828b13f3c5d3ca55a0931c7e7f198fdd6`。经本机 SDK 核对接口定义的 IMMDeviceEnumerator 对 eCapture 计数为 active0、disabled1、not_present16、unplugged2；Camera/Image 的 PresentOnly PnP 与独立 CIM 查询均为0。仅返回计数，COM 对象已释放、cleanup_errors 为空；未读取媒体内容或改变设备/隐私状态。这是设备库存，不能代替最终 App 的采集、拒绝/撤回/恢复矩阵。

本轮9个 validation 测试文件集中54通过/1个POSIX用例跳过/0失败、303断言，31.62秒。其原生范围是准确 PID/创建时间/映像/普通 token 的只读查询、独占文件 ACL/硬链接/限额、PowerShell AST 与唯一 ServiceBase 的 .NET 编译、只读设施库存；没有注册或启动服务、安装器、SMB、媒体或桌面输入。纯协议/factory 通过不能替代这些产品链。

随后用户亲自核实原聚焦失败，通过原 Session-owner 接口单次提交 `confirmed_failed`，seq79 追加人类 attestation、seq80 追加 effect/reconciled。Store 的合法对应状态为 `rejected`，pending/unknown=0，原物理桌面 resource key 未结算计数0；原78条事实及原失败报告逐字不变，未重发 POST 或原输入。首份 observer 在成功提交后误将 Store 状态断言为 failed，其 failed 报告保留；独立只读补证 `desktop-ordinary-owner-reconciliation-readback.json` SHA-256 `40d054098fbb8dc9e546ae442a208f4e3d67b58f50690f298eb7f0a653beb398` 证明真实结算。新升级保留输入为 `desktop-ordinary-upgrade-preservation-after-owner-reconciliation.json`，SHA-256 `78e3fd17e26d74c2c87ad8725cbf0da8b060bdbbb2ad4432fbeb7c6d01b35675`，覆盖80事件完整前缀。新 fixture 必须同时看到原 uncertain 与匹配 owner attestation/reconciliation，不接受 silent clear。追加守卫后9文件55通过/1个POSIX跳过/0失败、309断言，28.90秒。

用户另外明确选择保持当前设备状态、记录采集未覆盖，故不启用禁用设备、不接替用户改变隐私配置。后续 Windows 完成声明不得把媒体采集、相机或隐私变更矩阵写成通过。

raw ChildProcessBuilder 的 WinPS 兼容也保留既有继承语义：`env_clear()` 后不读取或注入 ambient，显式 empty/remove 与同一 Command 二次准备均保持原约定。最终 process lib139/139原生通过，3.53秒；`process-libs-final-inheritance-139.log` SHA-256 `a1bd6e793880d31e63dd25a433f816d4124815115e600c7dde3ae670393826ee`。

最终源码的 runtime 完整库52通过/1个已单独执行的 Node opt-in 忽略，7.17秒；管理员 WinPS 环境2项通过。系统桥7/7通过（最终 API-success guard 后日志 `system-libs-foreground-post-api-success-7.log`，SHA-256 `da2ba9869ed4a633e49d6c0092c44eb19d9ff5775c2d33e309158494fb852403`），App 正式管理员读取投影4/4、canonical 聚焦结算测试1项5分支及 bridge10/10通过。聚焦测试的物理输入仍为 mock；实际普通 token、Default desktop 与精确 worker/PID 查询属于独立原生范围，不能据此声明 UAC 或前台委托成功。支持的前台委托只在本 App 确实为前台、Default input/thread desktop、retained helper handle 仍存活时，对该确切 PID 调用 `AllowSetForegroundWindow`，不使用 ASFW_ANY、附加输入线程或改前台锁设置。

validation 集中最终60通过/1个POSIX跳过/0失败、363断言，30.88秒，日志 `validation-consolidated-final-drivers-20261017.log`。新增 installer 检查覆盖严格唯一安装身份、PowerShell 实际解析与同 Session 的安装/卸载工厂；这是 driver 验证，尚不表示 NSIS 已实际执行。

第一轮冻结候选 debug App 的 MSVC 构建退出0、16m03s，实际 exe SHA-256 `8679118a109ac899e2fddedbbd438a3279dcc8b97e30ec7f7aa1c32815d43640`，组件身份 `3e9f91b2c6618896abc25ca37b3724a5554fc39bdba8d8962418054e7b56a590`。构建后83个生产文件逐项核对冻结摘要一致，组件身份只在实际 binary 中找到唯一匹配；记录为 `windows-final-debug-build-receipt.json`。冻结后的 validation 脚本改动单独保留，不冒充编译输入。此处是构建证据，后续产品链按实际 executable 身份另记。最后审查又发现 Terminal 合法前导空格 cwd 与 Entry 命名 AGENTS 链接两处真实缺口，已最小修复并重新构建；该候选不能冒充包含这两处修复的 executable。

最终生产源码的 `bun run check` 再次完整退出0；日志 `windows-final-source-static-check.log` SHA-256 `8b3ca4f2a032577d8077ef8a6cbd761ed56b6ce73c3b36831dae8dfe0235f6ec`。MCP 最终8/8，日志 `windows-mcp-stdio-module-path-final.log`；App readonly 最终4/4的 Cargo 原始日志为 `windows-readonly-owner-projection-final.log`，同 binary 的独立运行记录另保留于 `windows-readonly-owner-projection-direct-final.log`。

尾部两处修复的原生证据：Terminal 原入口把 cwd 当项目 selector，现仅保留原绝对路径合同并由 native metadata 核对存在目录；完整 Service.create 的合法前导空格目录、缺失 cwd 不落 row 等5项通过，日志 `terminal-native-cwd-entry-focused.log` SHA-256 `79f2262e1a8c5bb309b9c6392b02837f01bbef9f72c0d8a78075e1db2590d654`。Entry 递归扫描对命名 AGENTS 链接纳入其真实父目录但不遍历链接目标，随后 Source follow 读取规则或返回真实错误；native acquisition8/8，日志 `native-scope-entry-linked-instruction-eight.log` SHA-256 `1a353a47443293f41d9ec36d130b91e0bf6f4d58d6598c9c28bffe9e698a7e8c`。目录 junction 的真实 Source 错误为 typed `SYSTEM_ACCESS_DENIED`/PermissionDenied/raw5，首次错误期待 InvalidInput 的 fixture 红灯保留。Agent Runtime 新定向1项通过，正常规则先更新并要求 fresh call，真实 Source 错误则 hold、不发布上下文、无 mutation；日志 SHA-256 `fd5a427474c51823e5b4da4549dd25faf76dc7237d65d6ca44b013cc50fc26b6`。共享 scope 与 Terminal 的生产部分在 05:33:11Z 后冻结；后续仅增强 cfg(test) 的实际 ConPTY 回归，不以测试尾部摘要变化冒充生产更新。

候选 `8679118a…` 的普通产品链再验：Minimal 报告 `files-process-chat.minimal-1792215450474-adba636c-9e12-482a-8ae2-09be1f3afab9.json` SHA-256 `90ad2741686d54ca609da8694091149345792f2ac1a348a106b3465d1eb147dd`，General 报告 `files-process-assistant.general-1792215517146-cc2f53b8-cd50-4915-bc4f-67b5049fdbf9.json` SHA-256 `a15144c20c27242af7eb449546642c801f2c5d525cb108fc8fe371dcbb1da1c2`，各20项、answered/cleanup_proven、pending/unknown=0。默认空 workspace 在 App28188 活跃期间的宿主 rmdir 返回 EBUSY，已如实保留 `fixture_cleanup_error`；外部业务目标已 canonical 删除，这不表示所有临时目录已清理。

同候选的 Browser 完整13业务步骤 PASS：`browser-assistant.general-1792215577180-4babd536-e960-46ec-b0c1-34556c515ad4.json` SHA-256 `71cf5db0a9d970903ecd26a433fc60282b9447a4ac3f89bcb810f1022b341265`，Session `01a1485f-7a92-7012-a701-2073ca27b13e`。真实 WebView2 三种来源、名称/mtime/字节/摘要、源修改与删除后的 retained File、document dispose、正式 delete junction 且原目标快照不变、正式 root delete 均通过，answered/cleanup_proven、pending/unknown=0，HTTP fixture 已关闭；默认空 workspace 的 EBUSY 仍单列保留。

同数据根升级的独立补证为 `windows-fixture-upgrade-readback-2e76947c-a09d-4a64-ad88-0aaf96cc839d.json`，SHA-256 `7c620ce1666e3efa63e7656fd009d1035bdd4c5ec58013d86a2c789ddbff003f`：App28188 为 medium RID8192、原 SID/Session、原 JWT 与数据根，原80事件前缀及原78 uncertain 前缀完整保留；无新 spawn、setup/login 或 Agent Effect。原升级 observer 过早单次读取尚未 ready 的 CDP，failed 报告原样保留；未来 readiness 改为有界只读等待。新 retained run 的 Task29016 跨 App12892 正常退出实际存续，但 port.json 原子替换触发严格单次 inode 检查，原整份报告仍 FAILED（SHA-256 `6cfb6a8d21f78e5440f2c7691938e7ca9fb14c7ffac05994526d9073936ff96a`），未重做其业务。

该 Task 后续同原 Session 的新 cleanup Turn PASS：`task-scoped-cleanup-93c93e9c-b2b9-4da5-a4c3-1a57d35fb10b.json` SHA-256 `347853ca764debb6a00e79a7c5593050b12cfb6bd828cf5a2247dfb7217ace5d`。确切定义、SID、InteractiveToken/RunLevel0、PT15M、action/workdir 和 marker 均先核对；Stop/DeleteTask 及 root delete 由正式工具执行，独立 Task/注册表 absent、PID29016 absent，answered/cleanup_proven、pending/unknown=0，原 failed 报告和全部前缀不变。此前 guard、原生账号规范化与新 Turn 的预接纳失败均保留；其中真实 `WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE` 来自 driver 提前删除空默认目录。仅对原 exact 私有夹具恢复空 workspace 后完成本次清理，此恢复不算产品门禁修复。正式 missing-default 修复和新 App 产品链另记，不能把 FileService 层的支持当作 Session 入口已通过。

普通桌面候选完整 PASS：`desktop-assistant.general-1792215682122-fea04212-4b1c-4de3-be35-0a66712c8d9e.json` SHA-256 `81cbcfcef14f0915cd3d760c68e2cb2fe424026e976d0560ec146ad902aae681`，Session `01a14861-14ce-7bc2-b0fb-3bc5e509ac27`。新 Calculator21376/HWND2561130 的真实截图、fresh UIA/ref click、键盘12+34与独立结果46、fresh AltF4/确切 PID 消失全部成立，57项核对、answered/cleanup_proven、pending/unknown=0。原 Calculator15300 也已通过原 Session 新 Turn 的 fresh 调用清理，`desktop-scoped-cleanup-a764690e-1556-44bb-a020-9bed2b731350.json` SHA-256 `e5f2f05f4b5c5402ad443e044b48440f03db17536496518911200f9ecd2ade6d`；原80前缀、failed 报告与人类 reconciliation 不变，无旧调用重放或 host input/kill。

候选管理员完整 PASS：`administrator-assistant.general-1792215849660-d3ffa373-c91f-49bb-bcf6-3154918162f2.json` SHA-256 `da3c938fd890876f8ae1b86b07c5a836b7c53d4bba81489aee5fb29c6e6edec4`。真实 BA/SY protected DACL、ordinary read/write 拒绝、正式 administrator read/marker、AGENTS 新规则延期与 fresh proposal 的完整538字节读取、high/elevated PID24736 的同 SID/Session1/USERPROFILE/cwd/显式 PATH、cancel/terminal reaped 与同 creation 身份实际退出均成立，answered/cleanup_proven、pending/unknown=0。本次两次只读 consent 进程库存未见窗口，不把 elevated 成功倒推为已观察本次 UAC 窗口；早先用户允许系统认证的证据仍按原 executable 分别保留。

高完整性桌面候选完整 PASS：`desktop-high-assistant.general-1792216392475-74396a05-3b62-4f90-a3ad-081a38b5ec9e.json` SHA-256 `efc85ddc43f35f4d09b864b82201182aff97afa5db47a606020dbd9476681cac`。唯一 high WinForms12648/HWND10490402 经原 Computer owner 确认切换执行者为确定未执行，再 fresh bridge observe/focus/UIA/ref click/type/Apply，独立标签结果、PNG、原 supervisor cancel/reaped 与确切 PID 消失均通过，answered/cleanup_proven、pending/unknown=0。第一次实物链虽已验证并清理，但 completion 因原 `needs_replan` 门禁拒绝，整份 failed（SHA-256 `a3a092573aff37eeca5e4386b065301e97148edaba0cb2c1a71e7ebcaa657c65`）保留。修后 driver 先读取同 operation 的真实 PlanUpdated、单独 update_plan 并核对 ack revision 后再提交完成报告，未放宽门禁或结果计数。

SCM 首次运行保留 FAILED：`service-chat.minimal-1792216495669-f7fed805-eb2f-4faf-be05-f75825c92f32.json` SHA-256 `97d5581cb5a07d1be32183cac8cc8a5235920424fd0259058644c55a8b1f96a9`。普通 source/compile 之后改变同一目录 DACL，before-model 对已跟踪 current_user 指令层 refresh 真实返回 EACCES；没有 SCM create 派发，未忽略错误或借管理员缓存。同原 Session 的新 cleanup 已 answered/cleanup_proven、pending/unknown=0，并删除准确 namespace，无服务遗留。fixture 正在将普通构建与从开始就受保护的系统安装目录分开；修后实际服务链另记。

随后 SCM 的普通构建与受保护安装目录分开，实际 create/config/start 成功；普通 host 查询 SYSTEM 进程真实返回 AccessDenied，整份失败保留，不能把拒绝写成进程已消失。该失败 Session 完成正式 stop/delete 及双目录清理；后续改为原管理员进程工具只读查询确切 PID/创建时间/映像。新报告 `service-chat.minimal-1792217459845-e2e6eeab-9138-45c3-8e0c-535b3015b287.json`（SHA-256 `74c65242a1ab5133025787cb049410014f2fedab2c777974bbac77572ab72562`）的16个业务步骤全部通过：真实 SYSTEM PID31276、Session0、创建时间134366911040160737、SCM配置/运行、OnStart/OnStop标记、确切进程退出、服务与双目录删除。但最后旧 driver 未取得当前 native checkpoint 接受的 completion evidence，整份仍保持 FAILED，pending/unknown=0。

该原 Session `01a1487c-35e9-7960-87c0-4477a213ec20` 随后新 Turn 只读结算通过，报告 `service-build-scoped-cleanup-921baf44-9da0-46dd-afd9-e035eb186e28.json` SHA-256 `497534a1dff927e128b4a1660d4da7e9131e8469814ec8ff101d6ae83126c886`。正式 administrator exec 再核对服务/确切进程 absent，当前用户编译exe与管理员started marker分别通过正式 read 的 typed absence；只引用这两条当前可用读取证据完成当前清理声明，answered/completed、cleanup_proven、pending/unknown=0。原2600余事件前缀及失败报告不变，没有重建服务或重放16个业务步骤。future fixture 在原流程末尾加入同样正式 absence 观察；不放宽 completion/native checkpoint 合同。

UNC 产品链首次真实冲突来自 share 根的表示差异，不是 ACL 或 FileID 不同。`unc-chat.minimal-1792217395097-b3c20f18-fbdc-4a1c-ae91-ef135ed2d124.json` SHA-256 `d5275e9596e5e33b13d54350db0834ccb0b3cc7488ba67b6aa687bec916d1b56` 保留失败与只读诊断；准确 localhost UUID share 的双句柄属性均为目录、无 symlink、同 FileID `ae777421:00050000000f7405`，仅 `VerbatimUNC` 根末尾分隔符形成不同 Rust RootDir。修复仅接受同 host/share 的两种精确根拼写，原 pinned/metadata/FileID/竞争检查继续生效，不扩展大小写、子路径、遍历或其他别名。`workspace_write::tests` 原生5/5通过，含反例与原父目录竞争/rename；修后完整 App UNC 产品链仍须单独记录。失败 share、目录及原 Session 已真实清理，pending/unknown=0。

默认目录被删除后的准入修复保留冻结路径、owner/binding/resources/snapshot 与原 lease/OS锁身份，只对共享 native core 证明的真正 missing 接受 unbound anchor，不重建目录、不授予知识挂载。知识 lease 原生15项、Runtime Registry精确6项守卫均通过，新增 missing-unbound 携带知识metadata仍在factory前拒绝且不建目录。App入口 missing-default 测试已通过；另一个旧个人developer route夹具在旧exe也同样失败，已仅修正该测试的明确model来源，fresh cfg编译结果另记。Terminal真实ConPTY leading-space cwd已通过。库层通过不能替代冻结Session之后删除默认目录的完整产品回归。

13个validation文件集中78通过/1个POSIX跳过/0失败、587断言，31.50秒；`validation-consolidated-unc-scm-guards-rerun.log` SHA-256 `84cd1f48af2c91b577b59bccd275955447a119f239799adc86defdf77d47dd3c`。首次Bun默认5秒测试期限截断SCM的原生AST/编译用例，失败保留；复跑只将测试runner期限设为30秒，产品deadline与断言不变。

收尾合同检查发现新增知识依赖后，派生manifest仍引用旧Cargo.lock摘要。已保留失败，以现役 `agent-v2-contract write` 同步三份派生JSON，再执行 `check` 退出0；没有手写摘要或放宽校验。回执 `windows-final-contract-generator-lock-refreshed.json` SHA-256 `b1ae2fd1946244d4546f925770a53f96e1293f731e4b21c1cc123abae0af65cf`。重新冻结的91个源码/manifest文件清单 `windows-final-production-source-freeze-lock-synced.json` SHA-256 `865ed4e5888d2b1717c2de2540dc304e218a462700ecc3644674ad2fd06ece8e`，候选 debug App 从该批源码构建。随后发现 Effect anchor 与 UNC cleanup 缺口，现役 `bun run build:win x64` 的唯一NSIS变体构建已协调停止，不能将它算作最终安装包；修复后的冻结批次、构建和安装验收另记。

管理员执行采用同 executable 的 Windows UAC runner 和有界 authenticated named-pipe RPC；每个逻辑 client 链接同一现役 ProcessSupervisor。source/scope acquisition 使用独立受监管 readonly worker，结果必须包含完整 typed JSON 与真实 worker receipt。普通 GUI 保持原登录身份；取消、终态只读、host disconnect、unknown 不重放和关闭时回收使用原 owner/Effect 链，不增加授权或重试账本。

原生库级 UAC 入口为 `cargo run --locked -p nomi-system-execution --example windows_native`，需要用户在 Windows 原生 UAC 界面完成认证；它是协议/生命周期验收，不能替代默认 Agent 产品回归。

完整 App 使用 `scripts/validation/global-os-windows-fixture.mjs --exe <实际新构建> --report-root <仓库外目录> --build-label <本轮构建身份>` 启动唯一隔离数据根。通过实际 WebView2 正常 setup/login 获取 owner JWT，按私有端口公告验证 PID 与目的地后再调用原 `global-os-native.mjs`。Windows scenario factory 保留公开默认 Agent 与 canonical 结算，提供普通文件/进程、管理员 ACL/指令身份、独占 HKCU 配置、Task Scheduler、loopback 和 Calculator 场景。

首轮完整 App 类型检查曾在 `nomifun-runtime` build.rs 的官方组件 cache 创建处被沙箱 token 拒绝；随后使用真实登录 token 完成多轮 MSVC App 构建，未改供应器 manifest/摘要。最终同步合同后的 `cargo build --locked --offline -p nomifun-desktop` 退出0、11m13s，91份冻结源码逐文件一致；保留两处 incremental cache finalize 的 native5提示，不把缓存复用警告当作产品成功或失败。不可变副本 SHA-256 `4b6bb712f35357cb7ea994746c817503803a4b7b989b911eda999acd1592153e`，二进制包含的组件来源身份 `a9267460c10c0b0b34bc234e6cc9bcf514fa0ce972c7b0be1f0f3fa1861d1883`，回执为 `windows-final-debug-build-lock-synced-receipt.json`。组件输入未变时此来源身份可相同，具体executable仍以整文件SHA及native peer image核对区分。类型检查、依赖构建等待与实际产品结果分别记账。

实现、类型检查、原生库测试、完整 App、UAC 允许/拒绝、桌面、设备、正式签名与安装包必须分别验收。本记录尚未记入结果的项目均不声明通过；没有提供的外接/网络卷、额外测试账号、硬件或签名条件也不能改写为 OS 不支持。

候选 `4b6bb712…` 的 missing-default 完整20步回归已通过 Session 准入及外部 write/read/patch/search/rename，随后 start_process 被 Effect resource key 的另一处物理 canonicalize 拒绝（os error2）。原失败 `files-process-chat.minimal-1792218878897-7c44a5d0-ea61-450e-9b16-7c093719c542.json` 保留；同原 Session 的新 cleanup Turn 删除外部目标，pending/unknown=0，默认目录继续 absent，未重放进程启动。生产 resource key 已仅对 files/process 复用同一 missing-unbound anchor，保留 owner、冻结 resource 和原 Store 唯一索引；VCS/artifacts 仍要求物理目录。新 cfg 的两轮 ACL 假设及旧 VCS 断言失败原样保留：RA deny 不等于 Windows namespace metadata 查询被拒绝，不能据此添加权限策略。已通过的 key/外部写入、foreign owner、原 pending reopen 和并发唯一 winner 独立记录，修正后的 junction/原 Effect reopen 两项及新 App 全链另验。

原 UNC failed 报告为 `unc-chat.minimal-1792218990977-4721b2dd-2d80-49a3-8a20-2f5bb8a9899c.json`，SHA-256 `be97ccdfcde70b3ea454cd2fb29a682a0698dc089205f626946649a1a027ea11`。只读、同时 pinned 的三个 regular files 显示：当前原文件与一个备份等于 canonical before，另一备份等于 canonical after，各自 FileID 稳定且不同。这只证明取证时状态，不能说明原操作未修改。原336事件、seq297 pending Effect、seq299部分发布/补偿未确认、seq335 cleanup pause 和 seq336 cancel 均保留；取消删除可执行 checkpoint 正文但保留 revision，不构成文件清理。`unc-owner-verification-not-provided.json` SHA-256 `a31f656826142a705b91e0dffa68c56c8b90f31ea892944b148b8fad1e1219f7` 记录未获核实，没有调用对账 API。

另建物理/UNC namespace 均不重叠的唯一 SMB 控制夹具，通过同一普通 owner/JWT 的新 Minimal Session `01a148a8-9fbc-7d02-801c-5274e2227521` 实测16项。真实进程 exit0/reaped，23485字节完整输出；driver 错把模型 excerpt 当完整 JSON 而失败，原报告 `smb-disposition-control-chat.minimal-1792220370419-48adbb32-70b6-4cac-83ce-ae113f5e5c7b.json` SHA-256 `873d2a666d0bc516c8f02aa6901b2159eefbc8eb2b4c804754bcacea58b47935` 仍为 FAILED。随后只读精确 canonical result/ref 对应的 typed Payload，并验证 Session、kind、byte_len 与 SHA，完整诊断 `smb-control-complete-canonical-payload-readonly.json` SHA-256 `d2d34c187f39688beb4266554d4bad9af750d301e5cf7c9e1da514d1cddd42d0`；没有替换模型输入、修改 Store 或重跑 probe。

控制结果为：本地8项全部成功；UNC 的 FileDispositionInfoEx 三组 flags（0x13/0x03/0x01）在有、无 metadata holder 时均返回 native87、路径仍在；FileDispositionInfo 两项均成功且所有本应用句柄关闭后路径 absent。有 metadata holder 时，basic mark 及 DELETE handle close 后查询仍为 native5，只有 metadata holder 最后关闭才为 native2。这证明兼容路径必须处理自身 pin 的生命周期并核实缺失，不能只凭 API 成功宣称 cleanup。该结果用于新代码设计，不补造原失败被丢失的 OS error，也不结算原 pending。控制 Session 本身 cancelled、pending/unknown=0，原 UNC 仍 pending1；后续仅清理控制夹具的新 Turn 另记。

同一 App30552 的真实 Documents 产品链通过：`files-process-chat.minimal-1792220732446-b916efd9-f64b-48c5-867b-27d781fc0534.json` SHA-256 `93840e57b7236a5af127900f699c83b4fae5773757619b59ca42741a657f4625`，Session `01a148ae-25f3-7b31-ba2e-27456742979c`。Windows KnownFolder 只读解析的 `C:/Users/muli/Documents` 已存在，未由 host 创建 parent/testroot；正式工具新建唯一 child 完成20项写读、patch、搜索、rename、外部cwd、start/running/cancel/reaped、确切PID1344退出和目录删除，含4项真实指令规则延期。answered/completed、cleanup_proven、pending/unknown=0，Documents child 实际 gone；空默认 workspace 在 App 活跃时宿主 rmdir 的 EBUSY 单独保留。

升级前只读取证侧车 `windows-additional-unsettled-preservation-ab614285-5f7f-4a09-988f-e414a9511d44.json` SHA-256 `fe708c3c91ecc1ed5ca6c1af8c25723937b14c557f253c3a3a3d33606b0a0fd4` 在 App30552 活跃时冻结成功：完整336事件逐项等于原观测，pending1/unknown0/cancelled、owner/binding/resources/Snapshot 与原 JWT 字节摘要不变，mutating_requests=0。该文件仅为测试证据，不是生产账本或升级授权；现役升级脚本仍强制原 Desktop230 guard，并可另加该 UNC guard。原 pending 可保持；只有完整匹配的 typed runtime_supervisor/process_restart quarantine、Effect全身份/因果/幂等键/fence与recovery-blocked诊断才允许变为unknown，不能接受人工对账、新call/Turn、settlement、换dataset或自动重放。纯4项/27断言通过；首轮共享引用使fixture自改before的测试失败保留，freeze已structuredClone并自验证原pending条件。真实关闭/升级及其结果另记。

App30552 的完整管理员再验通过：`administrator-assistant.general-1792220872470-6fdd4869-eed5-4419-8fbd-3b39db7bf973.json` SHA-256 `5900fe38e693d9827013c8810952e3ef91ca20f76040c980ae61fc198e699ef1`，Session `01a148b0-499a-7760-ae7e-d007544a46ce`。7业务/11核对（2次安全规则延期）、ordinary read/write denied、BA/SY protected DACL、正式 administrator Source、管理员cwd规则的同身份预检与freshcall，以及 high PID22092/创建时间134366945343338008/Session1/登录profile/cwd/显式PATH均成立；cancel/reaped、确切PID退出与夹具gone，answered/completed/cleanup_proven、pending/unknown=0。本轮复用既有已允许UAC helper，未再次观察到consent窗口，不补造新的允许证据。

独立 SMB 控制夹具同原 Session 的新 cleanup Turn 通过：`smb-control-scoped-cleanup-1ba44253-87be-4315-8813-e1d74736429a.json` SHA-256 `35940b7a000f38b36e0faefd6515db7bc764b9fe83685419371eff8f500ab44b`。只执行该 share remove、canonical localroot delete 和正式 marker typed absence，3业务/5核对含2次规则延期；answered/completed/cleanup_proven、pending/unknown=0，probe重放0。原控制651事件前缀、FAILED报告及完整Payload诊断字节不变；原失败UNC336事件和pending1也逐项保持，没有清理它的share/文件。清理专用driver省略无业务需求的显式默认cwd，仍使用原冻结工作目录与绝对目标，不放宽真实路径预检。

验收隔离另有实际遗漏：现役 `apps/desktop/src/main.rs` 无条件调用 `deep_link().register_all()`，插件在 Windows 写 HKCU 的 URI handler。只读观察 `daily-nomifun-protocol-readonly-observation.json` SHA-256 `189c6b247b3e159264d218919072c92499e901d2c69ffc0715c1ec7172566e2d` 证明32/64视图的 `Software\Classes\nomifun\shell\open\command` 当前都指向本次4b测试副本。首次启动前未保存该键原值，不能声称全部日常注册未变，也不能猜测原值恢复。精确 installed candidate 只读报告 `daily-nomifun-install-candidate-readonly.json` SHA-256 `e2f43c042c3c2a7f155db2528bad12556b539214789f5f69202724e10cba5c0f` 确认 HKCU 的 NomiFun0.8.4 位于 `D:\Users\muli\AppData\Local\Programs\NomiFun\nomifun-desktop.exe`，regular/父级无reparse、SHA-256 `4e0920b1c4f6d9014472196cb0812d40fe005a15583dab37946281866f0588be`、NotSigned；这仍不证明旧handler。已向用户说明并请求指定恢复目标，依赖该选择的恢复留待最后一次测试App退出后，其他工程继续。

用户随后回复“请按照最优的处理方式进行即可，这些旧数据我不在乎”。本轮按该链接问题的语境，在最终测试App正常退出后将handler指回上述已核实日常安装；这是用户指定处理后的配置修复，不冒充历史原值复原，也不作为删除数据集或核实/结算原UNC的依据。后续fixture仅记录精确URI键的type/长度/摘要，用于检测每次launch的影响；未知自定义command不保存或打印raw值，不自动恢复或建立授权账本。

Windows删除兼容修复已冻结：同一DELETE handle的Ex仅在raw1/50/87回退basic，成功mark后消费共享OwnedFile pin/Arc，close后以 retained parent内单basename原生metadata lookup的真实raw2证absence；拒绝、pending deletion和祖先路径变化不能当成功。补偿复用原expected Arc以关闭全部自有pin；create/ADS/direct/recursive也使用此关闭与证明合同，artifact仍借用外部identity/cache pin时保守unknown，不声称SMB artifact cleanup已覆盖。清理错误raw/kind前置，原128字符cause可保留根因。

新basic生命周期9项在真实登录token下全部通过，日志 `file-delete-disposition-basic-lifecycle-native-login-final.log` SHA-256 `8b50d3c45feb0be98ecf8b4be763766f96acfc341ab43919fa2de6923901e7b8`。首次误用受限sandbox导致ReplaceFile/rename5的红保留，未通过改ACL/readonly修生产。真实login中，after-mark父rename的std与POSIX调用仍被本机OS拒5；测试未声称该竞态已发生，而用capture原parent→合法迁移/旧路径重建→basic mark+外借reader的独立adapter控制，证明旧绝对路径缺失不误导relative proof。外部reader未关的direct删除保持FILE_DELETE_OUTCOME_UNKNOWN，readonly标记前拒绝保持原属性及确定失败。

现役Windows文件集本轮47通过、2项启用目录case-sensitive的设施调用raw50失败、1项需显式其他卷而ignored，原报告保留。受影响publication/delete/rollback小组首次51通过/4旧夹具非canonical C:拼写失败；仅cfg helper将既存目标canonicalize后，原std/POSIX foreign/race四项加raw32/kind前128守卫5/5通过，日志 SHA-256 `6636bfc14d20fdb747fd3a8a78010c484d1dac05eb596c82101ec5fbba58084a`。六源及红绿日志回执 `file-delete-compat-final-receipt.json` SHA-256 `5fa72ed0b5bfae1250e43726de39b98d4677a98777ac99fea85136b14eab0214`。独立源码审查未发现新增结算/句柄阻断；真实新App UNC产品链仍另验。

包含Effect anchor与删除兼容修复的94份全部 changed/new Cargo/apps/crates输入重新冻结为 `windows-production-source-freeze-disposition-compat.json`，SHA-256 `e2064a0c49a6da11e51da6efa18e5253281523565d703cf320b0cc5e16f3eaeb`，未改Cargo.lock（仍 `dafe8479babf3504897ac16b41f09ddc3ff23bab1dcf32b6103095bd380d9451`）。未变源码由本记录初始Git HEAD证明；validation脚本与文档独立审查。最终App cfg、debug host和唯一release NSIS从这批生产输入构建，退出码与原生结果尚待分别记录，不能沿用旧91批次的成功作为新binary认证。

94批次的跨模块 `bun run check` 已退出0，全部冻结生产输入逐文件未变；`windows-disposition-source-check-receipt.json` SHA-256 `e6b3808a84d2d662dbd235b4f523d365f911b087f5764a7d2d96f499c1a4b941`，完整日志 SHA-256 `a2e2d8291785619e0e3df098b3099fcf56fbb8ea400ca69409c9613d64c02d38`。文档、UI typecheck/880×600、i18n/theme/icons、installer、external-tools/process/Browser/UARC/Session/hosting/vocabulary均通过。后续validation-only补丁另跑定向测试，不把源检查算作native产品成功。

App30552 的新Browser13业务通过：`browser-assistant.general-1792221293926-dce0b0f2-4087-4d8a-a6b9-ce88b44a0e16.json` SHA-256 `d42c82772f614630145ed9128648c0ec4b187b4d153a4d3a9a97db2bd289ad39`，Session `01a148b6-b81d-7512-9860-ae4faede21a7`，24核对、answered/completed/cleanup_proven、pending/unknown=0。独立metadata baseline严格归属唯一新 `nomifun-browser-upload-BoHTo0`（0/1/2三叶子），不读内容、不猜旧Temp目录。该Browser报告未启用nativeDataRoot参数、没有initial_native_announcement；未回写原PASS，而用独立 `browser-current-app-host-identity-guarded-supplement.json` SHA-256 `973763f0bdddc3aebbc7e5a6dfa88692cf53b93f669e21577d24e09dffa71575` 绑定原ready/Browser SHA、实际baseURL/run起止、同PID/创建时间134366923802594258/映像4b、private marker/port/APIowner，fetch JWT前先验loopback/私有路径与当前进程。关闭验证两分支都要求saved product close receipt的真实path/SHA/字段；8项/52断言通过。

随后单次请求现役Tauri process_exit，App30552物理退出；`windows-fixture-close-af6171c0-291e-40b9-9eb2-d704cfef5378.json` SHA-256 `f144d38d9276790a95aa4fb2ac08f5f2887de6949d8ee03f080acfbc51fb872c` 只证明请求与PID gone，exit_code仍null，不能称完整宿主清理通过。exact creator tree15个含helper全部实际gone，`windows-process-tree-closed-54eb43ec-1cd7-4514-9e09-a40a6cf04d4a.json` SHA-256 `5fcc3e13d1f7e3992f8158b6ad74a0ce0cb123ad74ce19fed2e5abe0824d24fc`。严格Browser关闭observer真实失败：BoHTo0及三stage/三leaf仍在，七项原FileID/dev/birthtime/size/mtime/type完全匹配、attrs16/32无reparse；`browser-current-app-retained-snapshot-after-close-metadata.json` SHA-256 `f913045142ccbc5e2428d2b6cb3c91c7133da7df8361a7e7082e96e38c03c7ff`，未host删除或重跑。

实际根因已由原stdout而非猜测确定：首次1864..1869为原UNC Session的mcp_owner/hosted_receipts cleanup未知，1942第四次仍失败，1943在07:28:10.436661Z明确进入 `desktop exit cleanup never verified after bounded retries; forcing the exit without verified cleanup`。日志 SHA-256 `19f097f2c48031f4125a2d15001eb522796fe97c5d023cd54af883e43a2230bb`，只读取证 `browser-current-app-cleanup-readonly-forensics.json` SHA-256 `2590d27c807038abe9a893a4e1dbf57417a0a100666fb6716a80fb9084aa364a`。AppServices仅在runtimes/producers清理全证后进入Browser native close；现役三个guard测试要求unknown producer时保留native authority，Windows Host::close原本已有native tab退役后files.close/clear。因此未为让snapshot测试变绿挪该围栏、添加Exit事件事后删除或改Browser生产。此处分别保留业务PASS、15进程gone PASS、完整host/snapshot cleanup FAIL。原数据根/JWT、原34d share/文件、旧336 pending1与BoHTo0残留离线保留；新App升级与原事实只读守卫、独立Browser13及关闭另验。

94批次 App cfg 全默认features加browser-use/computer-use编译退出0、9m21s，保留incremental finalize native5的缓存警告。仅运行最后两项已修正夹具的定向测试，各1/1通过：missing默认anchor的dangling/redirected junction，以及原files pending/VCS unknown跨DB reopen对新process的持久围栏。测试exe SHA-256 `02b859ba1a566d56ad79726ac8d013d501c0cf31443a2ca378814a0206e3d403`，回执 `app-final-effect-anchor-native-receipt.json` SHA-256 `b21aa66f7393d556742d3272b1f3975496b31d4aced2eb1c776612d3d46de39e`。未宣称1035个App测试全部执行；此前key/owner、17入口和原Store reopen/winner已绿的范围不重复刷数。

仅validation关闭验证新增retained进程observer：CDP派发前以QUERY_LIMITED_INFORMATION|SYNCHRONIZE打开原PID、GetProcessTimes匹配ready创建时间并flush ready，随后等待同HANDLE signaled、GetExitCodeProcess并CloseHandle/observer真实reap。拒绝/超时/未知不派发或重派退出，不kill、不later-PID补造code；报告明确process_exit_proven与overall_cleanup_proven不同，bootstrap卸载消费要求真实exit0。真实登录token下本任务独立benign短进程自然退出23，3项/31断言通过；纯保留/URI/exit守卫合并11项/100断言通过。旧App30552的null仍原样留存，不用本机制补造其退出码。GET旧cancelled Session的events/execution/effects经源码审查仅canonical Store读取，不创建Runtime；startup恢复候选也排除该cancelled/ready状态，不为新验收hydrate旧UNC producer。

94批次debug构建原日志已记录9m08s完成；会话中断使原进程退出码未直接收取，因此该字段保留null。接续后以同一命令cache复核1.53s实际退出0，并逐文件核对94输入、复制及核对不可变新exe SHA-256 `16aadb5ac55ed0e8961dbbde23160a6510a68143d582ec9180393bf61aa91a36`（309740544字节）。回执 `windows-debug-build-disposition-compat-receipt.json` SHA-256 `f8214c160b0d732ba57fe1cb744e7bc14e13daf38cc7ad3d41a9b90f8eb643d9`；组件来源输入仍为a926，整exe与4b不同，未硬编码macOS组件身份。

新App升级保留两项validation首红：旧Desktop guard是28188原冻结快照，下一跳source30552导致预检PID不等，实际spawn0；修后只允许source.ready.upgrade精确引用同path/SHA、相同数据根/JWT、新数据集false、完整匹配已验证canonical proof的继承链，仍在新App读取全部230原事实。随后App22600已实际启动，但Bun的 `ConnectionRefused` 启动瞬态没有被只接受Node cause.ECONNREFUSED的观察器处理，`windows-fixture-upgrade-443cf7df-8967-4055-9cdd-12aa36b67223.json` 保持FAILED，未再次spawn。现役只读readback补验同PID/creation/image/owner及双guard通过，`windows-fixture-upgrade-readback-ae92f88f-de95-4add-a23c-1535c011c003.json` 为当前ready，API10445/CDP10428；Desktop230与UNC336严格保留，后者pending1。源数据根/JWT复用，无setup/login/newdataset/旧调用派发。另修restart继承旧readback时刻的validation字段：本次ready时间单独记录并优先用于进程创建时序，旧报告不改；12项/86断言通过。新App业务与原生关闭结果继续独立记录。

新增validation组合6文件26项/235断言全部通过，包含真实WinPS只读installer库存与retained observer自然退出23；`validation-final-close-preservation-installer-integration.log` 保留。Installer的bootstrap卸载消费随后加强为复用完整nested native exit validator，核对同fixture创建时间/确切PID、真实exit0、HANDLE关闭及observer成功reap，不只信任报告顶层boolean；8项/83断言通过，日志SHA-256 `46376d2421827c4a4082f72ad65d13c1c5373b955167ef737f3a8ac4360ec713`。尚未以这些测试冒称实际安装循环已通过。

App22600/16a 的 `--missing-default` 完整20核对通过：`files-process-chat.minimal-1792224629505-6dda3ba4-04ec-464d-ac4b-08456aebf45a.json` SHA-256 `0b5972ca7ed7761bc35e57f1c195214bf6902f3dad2873b43cb9bad240e743fc`，Session `01a148e9-9d9b-7142-a9d6-34a832b83282`。外部文件全链及PID23500启动/running/cancel/reaped/gone，answered/cleanup_proven、pending/unknown=0；同Session owner/binding/resources/Snapshot与原prefix保留，默认目录仍absent，不创建目录绕门禁。

该App的首个新UNC样本真实write/read/patch已通过，修后备份删除得到确认；search预检的share根RootDir拼写触发driver严格requested归属检查，整轮 `unc-chat.minimal-1792224686064-ee7e88f4-1acc-4306-875b-d8d71085df74.json` 仍FAILED，SHA-256 `af80c69dbc48e9869f739c81dec298f1bc3f7b58607b3715d5c1af2c2ef8973a`。原Session `01a148ea-7ae6-7613-b118-dc4584dc7d79` 新Turn已canonical清理其share91ba与物理root，completed/cleanup_proven、pending/unknown=0；未触原34d。仅将fixture search及两个cwd明确传真实share根末尾反斜线，保持生产preflight及requested归属规则不变，定向2项通过，受限token的EPERM原样保留。

第二新UNC样本实际write/read/patch/search/rename/别名及script写入通过，随后发现不能仅作为driver拼写问题放宽的结果：native-exe-cwd为真实exit0/reaped，Win32 cwd是准确 `\\?\UNC\localhost\<本次share>`，相对.NET读到正确改后字节，但WinPS5.1 `(Get-Location).ProviderPath` 为 `C:\`。`unc-chat.minimal-1792224827326-52c572d4-8e37-4717-ab94-512931f049e1.json` SHA-256 `d9ad171280cee69b160bdc3063a35084ccd00c002dd46cb77bca9884f78910c7` 保留FAILED，同原Session `01a148ec…` 新Turn已准确清理、pending/unknown=0。原生cwd正确不证明PowerShell provider位置正确；未加Set-Location/映射盘伪造初始cwd，也未提前归类为Windows不支持UNC。源码审查定位共享 `request::canonicalize_compatible` 只处理VerbatimDisk，UNC保持extended拼写进入同一Pipe/ConPTY CreateProcessW；待双拼写原生控制后决定安全兼容准备，未猜测改生产。

App22600 的管理员独立7业务/11核对通过：`administrator-assistant.general-1792224922288-363fc549-9a25-4a84-a8cb-63f602913e07.json` SHA-256 `4f8288283eeefe75e748ced01774c2382edc1b70ea0ce05e69b5592f885351a8`，Session `01a148ee-1660-7e90-b3b5-a8ecacc2019e`。普通ACL读写拒绝、正式admin Source、同SID/Session1/profile/显式PATH、PID29800 running/cancel/reaped/gone，保留创建时间134366985597385945及native退出码3221225786，cleanup_proven、pending/unknown=0、adminroot gone。本轮未观察到consent UI，不从管理员成功倒推其状态。

同App新Browser13业务/24核对通过：`browser-assistant.general-1792225076422-0027f17e-36d2-42de-a298-17c071f74b68.json` SHA-256 `5912176ae0986346b66b71a35fcb4a5afd60fcd3e102e46e6521087168cd3d34`，pending/unknown=0、业务cleanup_proven。独立同进程身份补证 `browser-final-disposition-host-identity.json` SHA-256 `d2529e6e6b80f6f73175bc9a94e6c000287430eceafafceb812c633c469a80b3`，strict snapshot5aWxzd真实存在、after-business元数据SHA-256 `73ce7ab9afc45244ba47d2640725d261c8949aa85f7925cdee34a914791c069c`；native App关闭/快照absence尚待，不能把业务完成当清理证明，旧BoHTo0继续保留。

UNC cwd两拼写控制首轮 `unc-cwd-spelling-control-chat.minimal-1792225427086-61d00e87-3be1-4608-b882-7dc7d70f4f13.json` 保留FAILED：parent实际exit1/reaped、0dropped，但初版probe generic throw丢失子退出/stderr诊断，不能推断两拼写原因。通用UNC恢复在新Turn已清其share6aab/root，原Supervisor结算与cleanup均0pending/unknown；这是控制factory没有落实更严格“两份typed子回执及确切PIDgone后才清理”的遗漏，不追认其子诊断。随后控制专门增加独立cleanup围栏与反例；probe将逐子返回有界typed实际诊断，parent统一汇总并flush后按真实协议状态退出，缺回执/截断/未知不得自动清理。原报告及已发生cleanup事实保留，原34d与BoHTo0未动。

UNC cwd 两拼写原生控制的第二轮完整通过：`unc-cwd-spelling-control-chat.minimal-1792226071407-9967588e-2f1c-44dd-8e96-d3b9186e765c.json` SHA-256 `c62a7f6f703fbfd0395eea7af6e1be70c44dbbf88a53bffa79eaa87838c6e6f2`，Session `01a148ff-9e9b-7c82-85da-8b40693bb968`。plain UNC 的 Win32、.NET 与 PowerShell provider 均为该 share，provider 相对 marker 读取成功；extended UNC 的前两者正确，provider 为 C 盘且相对 marker ObjectNotFound。子 PID20192/29872 各以创建时间绑定原 retained HANDLE，自然 exit0、CloseHandle、确切进程 gone，parent exit0/reaped；每份640字节真实 CLIXML progress stderr 均保留，未用清空 stderr 掩盖问题。控制 namespace 已 canonical 清理、pending/unknown=0，原34d未动。

修复仅落在共享 `nomi-process-runtime/src/request.rs`：从原 UTF-16 构造 Disk/UNC 兼容候选，两次真实 CreateFileW metadata 句柄核对 FileIdInfo、类型与目录属性，适用时采用兼容拼写，否则保留原 canonical。没有 Set-Location、映射盘、隐式 shell 或单独 Terminal 分支。原 Disk 基线5项实际全部绿，首个文件名含 red 的日志也仍如实记录通过；只有新增候选合同的两项真实红及修后8项绿作为本修复证据，不虚构已复现 Disk 错目标。

完整普通登录 token 的 `cargo test --locked --offline -p nomi-process-runtime --jobs 1 -- --nocapture` 最终253通过、0失败、3忽略：ACP936前提、1000次soak和独立ConPTY handle-count分别说明，忽略不算通过。完整套件还暴露原 common PTY_COLS 被孤立 macOS cfg 遮蔽，已仅修测试 cfg；架构serial断言同步既有deadline用例为24。日志 `windows-compatible-path-process-runtime-full-after-contract-sync.log` SHA-256 `e953ec1b81a469b389d105d8546f3bdae9a35ee1021df93cb4a21751d6c8361f`，来源回执 `windows-compatible-path-source-verification.json` SHA-256 `f01d3d8f837c488d76db7368c27f612c2f780195301e292d99ca2f321a71fef5`。这些库级结果不替代最终 App 的完整 UNC 链。

97文件来源冻结 `windows-production-source-freeze-unc-cwd.json` SHA-256 `5091e2346243549ff173d0cbcf3df2531b873d1af2e2294eb00add8860f49dd6` 后，`bun run check` 实际退出0，日志 SHA-256 `c5735a5334595e72ab6a9825bc5361a228dbcfb5b6060dcf27edcfbc587f94cc`。debug 构建实际退出0、8m52s，incremental finalization 的 native5 警告原样保留。全部97摘要再次匹配，复制不可变 exe SHA-256 `12cfb938e9aa11e76ca714398d8c9036c89c58e61af713e9f533dee2e9a64279`（309745152字节），source-derived 组件身份 `0259650fa8ba01aac3e12135c9e8ac5d953a6e93bae293e0e67875b289356208` 同时来自实际 build output 并存在于该 binary。回执 `windows-debug-build-unc-cwd-receipt.json` SHA-256 `0e5adce1d79fa8176b3c138c5e51159a006d59afe054a1fcdbd9d69cb6a3ee16`。上一94批次NSIS在发现UNC cwd问题后主动中止、退出1，不作为最终包；现役 `bun run build:win x64 --config <unique acceptance config>` 从97批次重新构建，原生安装结果另记。

原34d的可读说明仅为既有严格原生只读取证的展示：核对原 immutable 工具及源码摘要后重跑一次，原文件/两备份的 FileID、内容摘要和全部元数据均未变化。`unc-patch-owner-readable-current-native.json` SHA-256 `8fb6e9e85dc38d0f7a91bff3b518a6658854e12f4233c4107bb4aa8984c56480`，比较回执 SHA-256 `6290414df80fda2ce0df0494d5adc4eeef21260021af9000be32ef348ccf344d`；56/56/59字节内容分别对应原 canonical before/before/after。未新建读取后端、未写目标/Store、未代填人类核实，原 pending1 保留。

用户随后明确表示未看过旧UNC材料，只关心当前Windows开发和最终测试，由本任务自主判断资源是否保留并自动完成交付。本轮不再请求过程材料核实；原34d仍是有用的未结算故障证据，保留pending/文件/share，不把该指示代写为`session_owner_verified`。已完成且归属明确的其他测试夹具按真实原生使用/生命周期核对后清理。`windows-final-autonomy-and-unverified-unc-scope.json`记录此范围，仅为仓库外验收材料，未创建产品授权/重试账本。

独立最终源码审查记录 `windows-independent-final-source-review.json` SHA-256 `44bf432df2bd5ffc248ac9ea3993926d89de38213c720ef442ddb75191a182ac`：实际HEAD d1e、156文件，均为源码/测试/脚本/文档，无二进制diff、非regular/reparse、NUL、大于4MiB、验收产物或已知凭据形状。raw UTF-16兼容准备、pinned文件清理与UAC transport生命周期未见新增阻断；不替代后续最终App/NSIS、签名及设备矩阵。文档本轮最小检查129文件/1291本地引用通过；首次受限sandbox的git spawn EPERM单独保留，普通登录token重跑退出0。

App22600/16a的正常关闭已获完整独立证据：`windows-fixture-close-2945c249-bc66-4818-bec3-3ac96f9f02fa.json` SHA-256 `086d430a1ce707bd0fdc48a721e3caad329352602f3e1fb45375fdb2c697c1c8`，仅一次现役product exit，派发前保留确切creation134366981732294685的HANDLE，真实exit0、HANDLE关闭、observer31804 exit0/reaped。进程树独立证明15个有效确切对象均gone，`windows-process-tree-closed-03d47846-2136-408d-a222-bedcc0d7912e.json` SHA-256 `93058732f35e3029740d94e0dfc8fc86d612b45ee910a9590884d4f1ae915e9c`。此前观察器把已复用creator PID当父进程的首红保留；修复只在native创建时间证明child早于candidate parent时排除无效边及仅由该边可达的子树，缺身份/查询拒绝仍unproven，不以CIM舍入时间猜测，也不发送terminate信号。

Browser13业务后的真实快照5aWxzd在该次正常关闭前存在、关闭后真实absent，读取内容0/文件系统写入0；`browser-final-disposition-native-snapshot-teardown.json` SHA-256 `8b1a322773b5671caf59311dc965b3982ac49f7606d281b69bf78478e2d2b509`。本次证明新App确实走完原生关闭与快照释放；旧App30552强制失败退出及BoHTo0残留仍独立保留。进程exit0本身仍不等于全部资源清理，使用各自tree/snapshot证据补全范围，不把close顶层overall_cleanup_proven改为true。

升级前新direct双快照已在App22600真实GET冻结：`windows-current-desktop-preservation-e19fb74f-3ce8-46e4-9409-2b2644ae1eba.json` SHA-256 `4df8c273c7570e6bf771851617e4db9ee5aa8147a77035488a9456cddfb3d584`，`windows-current-unc-preservation-e19fb74f-3ce8-46e4-9409-2b2644ae1eba.json` SHA-256 `8938e25a7a5ce89bba56a0393ef26f76f6071d106a64668112a1217f77fe2306`。原230/336全部事实、owner/binding/resources/Snapshot/JWT不变；这些只读证据不创建新产品事实或代替owner核实，原UNC pending1仍保留。下一candidate用这些直接当前身份的快照验证sameDataRoot升级，不靠宽松接受任意PID继承。

最终observer/installer小套件在最近的process-tree修复后合并复核：6文件25通过、0失败、204断言，包含真实只读Windows库存与自有短进程retained native exit23。`windows-final-native-observer-and-installer-validation.log` SHA-256 `ebfd139031bde2c80321e7bcbce4f18b357a09f7230878f5ef72741ad4474db`，实际退出0。没有借此宣布安装循环已执行。

12cf的sameDataRoot升级实际启动App9028；首个rendererIdentity观察失败保留`windows-fixture-upgrade-b7282f6d-fe30-46f2-bd6b-f15f053cb4d7.json`，未猜测原因或再次spawn。现役verify-existing补验同PID/镜像/登录medium、同JWT及双direct230/336 guard全部通过，`windows-fixture-upgrade-readback-c22a3539-da47-4163-af0a-8481aee5bbdd.json`为该次ready，补验spawn/setup/login/effect均0。

该App完整UNC的新样本确认了生产cwd修复：native-exe与PS1两真实receipt的Win32/PowerShell provider均为plain UNC，literal argv完整；write/read/patch/search/rename/扩展alias和两文件delete也均通过。随后真实暴露另一生产缺陷，不能归类为driver：唯一share Remove-SmbShare已在seq903 terminal exit0/reaped且exists=false，effect后before-model刷新仍保留已删literal-tool.ps1的旧Entry recursive anchor；seq907内部read、seq910`INVALID_PAYLOAD: native path has no resolvable ancestor`、seq914/916 turn failed。`unc-chat.minimal-1792227771930-7e72101c-bee7-430a-95a8-64753508624b.json` SHA-256 `3e71876a9624429336241ed31c15c52c27aaa49da39ed3edd77d6cad9937314e`仍FAILED；原Session`01a14919-917c-7a02-83aa-21267b8edf73`新cleanup Turn已准确确认该7cdab share absent并删除physicalroot，completed/cleanup_proven、pending/unknown=0，未触34d。bounded canonical诊断`final-12cf-unc-share-removal-scope-refresh-failure.json` SHA-256 `8f27fe51d88d7ce639773fd82532d6ac8d2714f60692e2bb8bb712dfbbae9923`。旧分支没有保留OS kind/raw，诊断明确未从当前状态补造。

发现上述scope缺陷后，本轮97批次NSIS构建在编译阶段主动CtrlC、实际退出1，尚未派发installer；日志`windows-installer-build-unc-cwd.log`保留，不作最终包。先修正历史scope刷新与原生错误投影，再冻结及重新构建。共享native core最小回归使用只读未挂载DOS根而不配置网络：原代码对Source与scope的无可解析根错误丢失真实OS信息，首轮红101；修后保留root lookup的typed Io/NotFound/raw，仍返回error，绝不当Source None。完整`nomi-native-files`原生12项通过，包含ACL5、真实aliases/ADS/VolumeGUID/GLOBALROOT、junction变化/指令links及限额；日志`windows-unavailable-root-native-final.log`。该小修只恢复错误真实性，历史scope退役还须独立完成与验证，不能把core绿当整轮修复。

12cf已开始的管理员7在发现UNC scope缺陷后自然收束，通过11核对，未人为取消：`administrator-assistant.general-1792228020318-74f42005-7fb9-43f4-9a68-62d80fc4b68e.json` SHA-256 `4e9a467028e87baa1910ce08716c82580178e3c3065b9509f631d1bf05267370`，Session`01a1491d-5c41-7b42-b16d-92de1b505ac0`。高身份PID20800/creation134367016820498055、同登录SID/Session1/profile/explicitPATH，native cancel exit3221225786/reaped/gone，root absent、pending/unknown=0。没有开始计划中的20项重复场景。

随后App9028单次正常产品退出，retained确切句柄实取exit0、observer33284 actual0/reaped/closed，9个有效exact进程对象全部gone；close`windows-fixture-close-4ea71075-df25-457b-8233-df3afb6913f4.json` SHA-256 `f367e13a54a903b537d294dcb5a68ec5b6ab3b6deecf1bc6f7f00768962e73bd`，tree closed`windows-process-tree-closed-d53563a5-793f-41c5-b28b-1820a589b7a9.json` SHA-256 `1383ec61e86dce0a2dd27fb7106df8f8e45c408419a0a0a44bac6b3204f866b4`。升级前current direct230/336 guard后缀c215bb2b-9fad-4358-ad1a-d8bd7d0bdc23，两份SHA-256依次为`fcd16773bb6d941855a6f686143a43c34a27d5386c93a577ac4eceb860e1e127`和`83f5065adadd016bac2aca5710fa39e20ce5549813855ebae78ed43aea663b90`，旧UNC仍pending1/unknown0。没有新建数据集或操作旧34d/BoHTo0。

利用此次真实App关闭后的开发空档，先执行已审查URI脚本的只读预检（退出0/写0），再按用户已授权目标修复HKCU既有`nomifun`的command/icon两个默认值，实际写2，32/64视图别名读回一致；没有创建/删除键、改ACL、启动App或删除数据。`windows-uri-handler-restoration-e734c8b1-62b4-4e35-b7f2-877a0e021ec7.json` SHA-256 `327c0a0683c27e170fae6b8e45165304570c58bbfd1a23e695a1b3c6283d44e3`，当前目标为已核实日常0.8.4。此为指定目标修复，不冒充原值历史恢复；下一默认测试App会再register_all，需记录本次launch新baseline，最终关闭后再核对修复。

历史scope修复首轮完整`nomifun-agent-runtime --lib`实际退出0，450通过/0失败/1忽略；新增真实Session-loop在已结算namespace撤销后撤下旧规则、向模型提供unavailable诊断，同namespace新call被hold且无dispatch、独立root真实dispatch，原effect仅一次。首个unit红为Redirect夹具在额外namespace变化后未invalidate的既有readonly-burst前提错误，只修该夹具；Access/MissingRoot/Malformed仍不补invalidate地证明retired cache须fresh。日志`windows-historical-instruction-retirement-full-runtime-first.log`保留。随后dirty-preflight与hard cache budget反例仍在补验，未把该450结果冒充后续最终源码全绿。

读取错误保真修复沿现役载荷完整落地：`AppError::NativeFileAcquisition`保留四个native失败字段及仅本地可用的原IO source/operation；六类code有明确静态HTTP/error_code映射。ordinary readonly authority/entry/metadata只改读取专用适配，不改其他native_io路径；owner往返使用原IO对象。administrator在`system_read_error`读取真实worker的typed failure并验证code与transport一致，source/scope的Err不转absence。Wave2HostPortError、Kernel capability failure、Engine及Runtime错误仅增加可选details，缺省None保持原行为；原native metadata经白名单、现役脱敏与8KiB预算进入原ToolCompleted，普通模型调用和内部指令读取复用同一typed投影，不把JSON塞message后猜类型。

该通路的8组原生定向检查合计52通过/0失败：common21、file acquisition2/text11/scope7/global files5、domain1/kernel2/engine3。实际localhost不存在share的普通/extended路径对text/page/Follow/Entry均保留typed NotFound及真实OS号；存在祖先的正常deep missing仍为合法None。明细及全部原日志摘要见`typed-native-targeted-tests-before-runtime-final-refine.json` SHA-256 `4871c62d84df049cccafdc55039b13401a68dc86e4e297400a2d6987a0085634`。Appcfg首次编译的gateway Canvas摘要穷尽match遗漏保留101；仅补明确native错误显示映射，未把native file-notfound说成Canvas不存在。随后的中断编译退出1仍不计成功，最终相同features的Appcfg另记。

历史刷新补强后的完整Runtime最终455通过/0失败/1忽略，实际退出0，回执`windows-runtime-final-typed-history-tests-receipt.json` SHA-256 `b4e16c0b5c6f62e4b314e9945ed13991161cc9a2c5e40f3e4ba8bce426d653a1`。包括真实Session-loop、同身份新鲜预检、ACL外部变化使dirty、20×16KiB空白规则触发256KiB硬cache预算、combined48KiB，以及Scope/Source两阶段未知owner原错误落ToolCompleted后fatal、无任何sibling重派。硬预算不再被load_agents_md的warning包装吞成unavailable。首轮451通过/4失败保留：root的过严guard错误要求role host包装码等于native cause码；修后依据独立typed native cause并保留外层码，显式unknown及无typed native完成证明的owner错误仍fatal，未从message恢复类型。

共享type的只读跨cfg构造审查未发现遗漏，尤其macOS-only GUI fixture的两处构造显式details=None；这只是源码构造完整性，不冒充macOS编译/原生重跑。最终`bun run check`实际退出0：前后168个Git变更输入摘要完全相同，完整UI/installer/进程/Browser/Agent Session等边界全部通过；`windows-final-typed-source-check-receipt.json` SHA-256 `ef22c0188e666c14c4f9b3bcfd1ced446f3cda1d4139f17b8ce7cc3094064693`，日志SHA-256 `6a5eb6bfa40c54d0d1e0998ba7b72adaf624037589232009b54b3d4479baa3fb`。后续仅追加文档记录时用最小文档检查，不把本结果冒充尚未执行的最终App或installer验收。

交付前真实远端检查发现main已前进，未覆盖或force-push：原Windows168文件先在`codex/windows-global-os`保存提交`1b9c317ecaf5a5b7f70de382defbaf6214849fff`（tree`c25f784760d3d1704828421cc25727bacdfa6305`），当时工作树干净；该提交记录455 Runtime/60 typed native与已发生App证据，未将后续App/installer写成通过。随后fetch并非快进整合远端`d129fa9994cdc408fc2469e23e6115dd2e3be00b`，包含Linux全局能力`05c9de03a`及4条UI/updater修复。11个冲突按平台与共享语义逐项解决，未整文件选ours/theirs，也未丢弃Linux实现。

合并保留Windows Entry/Follow、typed error、历史退役/fatal owner/硬预算，同时吸收main的canonical alias改进：路径拼写差异本身不再强迫新调用，实际新指令仍延期并要求fresh call；原调用参数不改写。unbound workspace统一`resolve_unbound_workspace_anchor`，保留Windows pinned native缺失/ACL/别名证明并加入Linux clean frozen-anchor约束；files/process共用原namespace，VCS/artifacts继续要求真实物理目录。共享执行client保留Mac、Windows、Linux各自原生入口与生命周期，其他平台明确unavailable；build identity新增build.rs自身和真实Linuxpolkit/install资产。上述源码合并不替代其他平台重验。

共享验证driver的16块冲突已合并，保留Windows精确canonical slot/unknown/control cleanup门禁和Linux全部场景。35项通过、0失败、4个平台跳过，321断言、6模块import及3个CLI错误平台flag提前拒绝通过；`windows-linux-driver-merge-review.json` SHA-256 `4cb0c5edb8dab92014306ece751dcbd1e9ba9655f81c96c5533d7b3a30b7e5af`。失败夹具继续保留原报告与目录；不是Linux原生或最终Windows App证明。

合并Runtime首轮456通过/2失败/1忽略保留：一个Windows alias夹具未按内部metadata请求归一化分隔符查表；一个旧Entry测试仍要求仅拼写变化的`resolves to`延期。修复仅调整夹具键和断言，新增确认Follow确实独立发现真实目标目录，而不是复用Entry观察。最终完整Runtime458通过/0失败/1忽略，实际退出0，日志`windows-merged-runtime-tests-final.log`。knowledge新增Linux反例在Windows verbatim PathBuf::push已正规化掉`..`，17/1首红保留，改为OsString保留真实ParentDir并断言夹具前提后18/0通过；生产检查未放宽。合并system-execution原生库7/0通过。后续App/包原生结果另记。

合并初次离线解析因本机缺gstreamer元数据而101，按既有Cargo.lock执行Windows目标`cargo fetch --locked --target x86_64-pc-windows-msvc`后退出0，仅补索引/所需缓存，不升级依赖、不安装Linux系统组件。完整`bun run check`实际退出0：135文档/1308引用、2177 renderer源的880×600边界和所有现役Agent/进程/Browser/installer边界通过；日志`windows-merged-source-check.log`。独立新UI与Linux历史结果保持原归属，不借本次Windows检查重新认证。

合并定向验证已完整结束，33通过/0失败：knowledge18、system-execution7、App8（两项missing-default effect-key/plain-verbatim与VCS strictness，加workspace_file_read全6）。App使用相同browser-use,computer-use features单次no-run编译实际退出0、8m15s，随后同exe直接过滤执行，均非0匹配。总回执`windows-merged-targeted-tests-receipt.json` SHA-256 `fb597b32f5db2ba31e94b828d34b3da7be4edbcaeb0651ff413f45668f6279de`，App子回执SHA-256 `f9844107b225eb6d4eedf3e29e94cc2c3767019d28a29c52567472ea0cd6db46`。首红与修后正例保留，没有通过放宽生产来消除测试。最终Windows native App、NSIS与收尾仍以之后实际报告为准。

远端main前进后已停止109批次debug编译（actual1，`windows-debug-build-typed-history.log`），没有将中途exe当最终artifact。此前109冻结及静态/单元结果属于整合前source cut，不套用到当前合并源码。新完整Git merge保存后重新冻结/构建，组件ID取本轮实际产物，不沿用0259或Mac e4身份。

## 合并源码冻结与最终 App 回归

Windows 实现已保存为 `1b9c317ecaf5a5b7f70de382defbaf6214849fff`，随后普通合并 main `d129fa9994cdc408fc2469e23e6115dd2e3be00b`，得到生产源码 anchor `91410efd5dd3227104b783bf794591254c014d82`、tree `5a9b265e423b82bb438ea039dbb9e9eb71ff1635`，分支 `codex/windows-global-os`。构建开始及 debug 产物保存时工作树干净。验收期间远端另前进到 `0ab974b461cc8a74ac9537277d88156c8701f010`，其等待/取消跨消费者改动不在本次生产冻结内；不得将冻结构建说成该后继 main 的原生验收，也不覆盖或重写 main。

`cargo build --locked --offline -p nomifun-desktop --jobs 1` 实际退出0、9m54s。最终 debug App SHA-256 `d22d473bb8cbf3e107e8cb57da9a9553635385783c1efc507b1bda2ab6fdb932`，310366208 bytes；本轮实际组件身份为 `853a949acccee25c5e487c638e3f0b22ba2f119b8b888d406fbafeb5f73fb529`，从 Cargo build output 读取并核实存在于不可变 executable 中。产物保存于仓库外 `accepted-debug-merged-d22d473bb8cb/nomifun-desktop.exe`；回执 `windows-debug-build-merged-final-receipt.json` SHA-256 `386f41d7ab643ffeec89f61f0d3e9adac11eb7a0ffd3254519eabffa6d317db1`。增量缓存 finalize 的 native5 note 被保留；实际构建和链接已完成，不能把缓存复用限制写成构建失败。

原数据根与 JWT 升级得到 App21108、medium8192、Session1，真实 renderer 1280×832/version0.8.12 已挂载；未创建数据集或重新 setup/login。`windows-fixture-upgrade-5bd00074-318f-4b4e-b83e-7d4eb4c3fd00.json` SHA-256 `b06e4ce21c9e0e5c83e5b0a302f617c44fac41c419ce7372dbdb8be903770cf2`，原 Desktop230/UNC336 facts、owner/binding/resources/Snapshot 均按 c215 双守卫保持，原34d pending1 未结算。用户在本轮真实 UAC 窗口允许，不沿用先前 App 的认证回答。

本轮第一次 UNC 报告 `unc-chat.minimal-1792233576034-c2523fd9-0134-4a70-a9d8-49d7994ca81b.json` 保持 FAILED，SHA-256 `7c8155922a081facc732e7bf2a128ab07be2fc31c8c72dda3db682a7ebea7ba4`。原 unc-read 的 proposal222、InstructionsUpdated238、runtime_gate held239 及 invocation_attempted=false 证明调用未派发、无该调用 Effect；驱动此前仅识别旧 Path resolves-to 延期，误拒绝现役通用新 scope 延期。该 Turn 已取消，同 Session 新 cleanup Turn 已真实移除这次唯一 share/root，pending/unknown0，不能再 resume 已清理的业务或重用旧 call。

仅修测试驱动与纯测试：通用延期须精确匹配原 proposal/name/参数与 output 的 canonical slot、同 Session/operation、InstructionsUpdated 和 runtime_gate，并拒绝任何 started/dispatch/Effect/attempted 证据；最多4次，保持原参数、以新 call ID 继续，管理员 marker 专用验证仍优先。38通过/0失败/1个平台跳过、360断言；首次测试对象共享造成的反例夹具红灯保留。`windows-generic-instruction-deferral-driver-fix-receipt.json` SHA-256 `0b07e35a70ee8349decc7dea54744ac4a350da16ccdde705fe34a1d9c03fe659`，另以新 helper 只读复核原报告222→238→239且字节未变。该修复不改变当前 App 的生产输入，也不重建或重启 App。

新唯一夹具的完整 UNC16 已 PASS：`unc-chat.minimal-1792233880432-b6270a0d-be27-409b-9013-99a6caf1b655.json`，SHA-256 `a3a83b0d8b279ff4164c006f58affb99a67f03fcff9672ddfbc18b56a3dc9788`，Session `01a14976-c764-7b80-b750-fd9734b43c0b`，16业务步骤/23检查、completed/answered、pending0/unknown0、cleanup_proven。包括真实 UNC patch/rename/alias/literal script、WinPS provider cwd、删除文件、share removal 与目标物理目录移除；4次真实新 scope 延期分别以 canonical 零派发证明和 fresh ID 完成。宿主最终 rmdir 空默认 workspace 遇 EBUSY，报告保留该 `fixture_cleanup_error`；这不是 UNC share/root 残留，也不能声称空默认夹具已删除，须在正常关闭 App 后另行精确清理。

同一 d22d App 的另外三组业务回归均已完成，原生报告索引为 `windows-final-merged-native-cases-index.json`、SHA-256 `eb4dc3d7846fdcce1abe973489cd2e6de9813744552b985eca174d5aeb382130`。这些是本机 scripted provider 经公开默认 Agent API 驱动的真实 owner/OS 结果，不是联网模型推理证明。

| 本轮最终 debug 场景 | 真实结果及报告身份 |
| --- | --- |
| 管理员进程和只读 | PASS，当前实际8业务步骤/10检查（包括 admin-source-read），completed/answered、pending0/unknown0、cleanup_proven，管理员 root absent；`administrator-assistant.general-1792233970078-b361382d-d8c4-42bc-8990-68ef33cfe17a.json` SHA-256 `3feb1b826bd27eabb2bfc594066f48d4bbee72f5886f7df2b596bc3eb508cd90`。空默认 workspace 的 EBUSY 保留，待最后退出后精确清理。 |
| Minimal/default directory missing | PASS，12命名业务步骤/16检查，completed/answered、pending0/unknown0、cleanup_proven，outside absent且无 fixture_cleanup_error；`files-process-chat.minimal-1792234154399-7225a2c3-5fa6-4c4a-ac68-a683b0820b49.json` SHA-256 `597a5ba3a14fc4badd389f69ad876a52d7dd98dc8399eb034eb1d3a8021f6fe2`。原 Session/owner/binding/resources/Snapshot/grants/data root/App PID/event prefix 全部保持，默认目录持续 absent，未递归删除、重建或另建 dataset。12步骤集合完整；此前“20”包含路径拼写延期检查，不把它改称20业务。 |
| Browser upload/retained file | PASS，13业务步骤/22检查，completed/answered、pending0/unknown0、cleanup_proven；`browser-assistant.general-1792234220031-86ea1700-c597-4f77-8c79-23f214a02a72.json` SHA-256 `d7826d8c6322f475457e3cbeeb885a0b79a11956f4cdc85bbd35f294dfd97448`。新 lmBztb 快照归属及退出后实际清理另证，不与旧 BoHTo0 残留混用；空默认 workspace EBUSY保留。 |

Browser 首次独立宿主补证因成功升级 ready 漏记顶层 caller 字段而 TypeError，保留 `merged-final-browser-86ea1700-host-supplement-failed.json`，SHA-256 `11753677850f6b345a3b741c6bb096becfb3ab6374882a2e62f2f14e1caf2384`。随后通过原 JWT 对当前 API 真实 GET、port公告与 exact PID/creation/image/medium token 只读核对，新增 `windows-fixture-readonly-caller-18aaec42-5b37-4959-9e5d-b4bfec856a8a.json`、SHA-256 `7b84754b1c655956fd64f5d4229f3024a96f5636d678f990a3fc37d980f4e7f3`。本次 caller observed_at 为10:53:56Z，明确不追称启动时已经记录；spawn/setup/login/effect均0，原5bd ready及Browser报告字节未改。宿主身份回执SHA-256 `412d1ba8505e2e0c8cc525ba597100d575c63c8cf67f6d95effdefaceeb83cb6`，preclose snapshot回执SHA-256 `a53372a0f48796009530b9d804ab90f99be5b067e9f1d4c564f25ed3d1eca9e9`；后续正常close统一绑定这份derived ready。

成功restart/upgrade的fixture脚本已补同一真实GET与exact新PID校验，再写compact caller，不沿用旧值或新setup。现有身份测试11通过/0失败、87断言；`fixture-caller-recording-fix-receipt.json` SHA-256 `455f7211ba4b17b9b0042ef847d076cc3d3b8d290fa0b6a89da009a2b155e226`。本轮生产冻结后的改动仅限验证脚本、测试和文档，不更改当前App生产源码；NSIS和完整退出结论仍须读取其实际后续回执。

## NSIS release 构建的首次链接失败

首次完整 `bun run build:win x64 --config ABS/installer-acceptance-config.json`（exec46864）实际退出1，保留 `windows-installer-build-merged-final.log` 和 exit JSON，没有生成可验收的 NSIS 或派发安装器。根任务额外设置的 `CARGO_NET_OFFLINE=true` 使 ort-sys2.0.0-rc.13 在检查既有分发缓存前直接选择 `link_error_generic`；最终 `OrtGetApiBase` 与占位错误未解析，不能将编译过程或生成的中间对象当成打包通过。

只读核对本机依赖源码后移除该次进程环境变量，保留原正常链接路径，不设置 `ORT_LIB_PATH`、不禁用链接、不伪造库或修改生产源码。该 buildscript 已声明 `rerun-if-env-changed=CARGO_NET_OFFLINE`，true→null 后实际自动重跑；新 output 已包含 `Using prebuilt binaries`、`pyke`、DirectML/DXCORE/DXGI/D3D12 和 static onnxruntime 的真实链接项，无需删除 fingerprint 或全仓 clean。复用同一 Debug 已使用的 MSVC 分发缓存；发行源为依赖清单中的1.28.0，既有目录及本次逐文件摘要不冒充本次独立压缩归档校验。

`ort-offline-release-link-diagnostic.json` SHA-256 `111d97726363da5cd2044a79c85cd66301236adcc1d3e428a16692017fe710d6` 保留源码、cache两文件摘要、原失败和正常 buildscript 输出。纠正后 exec66984 使用仓库默认3个构建 jobs，仍由官方相同 build:win 入口构建同一 UUID variant；诊断写入时仅为 `build_script_recovered_release_build_pending`，不能提前标作 App 链接、NSIS、安装或卸载 PASS。最终结果在后续独立回执记录。

## 最终 release 与真实 NSIS 周期

纠正上述进程环境后，相同官方 `bun run build:win x64 --config ABS/installer-acceptance-config.json` 已实际退出0；release 编译21m22s，makensis 完成。生产输入仍为91410 anchor，期间改动仅为验证脚本、测试和文档，不把整个工作树称为干净。`windows-installer-build-merged-online.log` 与 exit JSON 保留实际结果；产物回执 `windows-release-nsis-build-artifacts.json` SHA-256 `8a3fa84ba89c4c2d2ad99b0f42c7d6c43ae28818517ae1318bdfc85a12c6a275` 同时核对构建输入、组件853a949a、资源和不可变副本。

唯一变体 `NomiFun OS Acceptance 439f698fa737498eb1550807a39bf3f8` 使用独立 product/id/protocol/mainBinary、currentUser 安装。实际 NSIS 为 `target/x86_64-pc-windows-msvc/release/bundle/nsis/NomiFun OS Acceptance 439f698fa737498eb1550807a39bf3f8_0.8.12_x64-setup.exe`，SHA-256 `3e8997f066d6f3e3ac0a12d7de477b76e508aacc6aa3efb53e2175f98383777a`。官方渲染 NSI 的相对 `OutFile "nsis-output.exe"` 在其 `release/nsis/x64` 编译目录生成，成功后才搬到 bundle 路径；验证器依据已安装 Tauri CLI2.11.2 的官方源码和严格目录/版本/架构关系定位，不改 NSI 或借任意文件充当安装器。

首次实际 canonical install 的 NSIS 进程26988 ordinary/elevated=false、exit0/reaped、原 Effect 已成功结算，但旧验证器错误要求安装后 executable 等于打包前 raw executable，报告 `installer-chat.minimal-1792238625713-32c431c3-80da-411d-887b-0a28058c83b5.json` 保持 FAILED，SHA-256 `31ab03f665e390f14cbe6c43541ae25757afa377133556811db102a4e91f1f41`。只读取证确定官方 bundler 会把首次27字节 bundle-type 标记的 UNK 改为 NSS，制作 NSIS 后再恢复 raw executable。逐字节比较仅三个字节不同，严格预测完整文件摘要等于真实安装文件；没有忽略 PE 区域或放宽签名校验。

raw executable SHA-256 为 `38929b00c2ca2df7034f8090cda9c9bd48ea5f813bb6019e0f439386f6dcc6c0`，安装的 NSS executable 为 `3d0665f0c27e8112a44ec01090ffebc536a13395922a65a3ab64af70fdcd2920`。验证器仅允许精确首标记修改、源与安装文件均 NotSigned/无 signer，并比较预测后完整 SHA；首次失败报告不回写。修复回执 `windows-nsis-bundle-type-prediction-fix-receipt.json` SHA-256 `175afd8b03857f39adeaeb66ef3821bfded356ca4e2e03550f7ba9c6e0f29c38`；纯测试13通过/0失败/1平台过滤、129断言，真实 Windows NSI parser/token/inventory 测试另1通过/17断言。

原失败 Session `01a149bf-2f95-7da2-b619-8da896ce978b` 已取消并为 pending0/unknown0。严格只读安装核对后，仅在该 Session 的新 Turn 执行四步准确卸载清理，复用原 provider/冻结 binding/owner/resources，原350事件前缀逐项不变、installer 重装0。`installer-same-session-execute-6bacef5a-3521-4ced-9bc8-a9c490965110.json` SHA-256 `d1eecbea9c3b8a8f68accbb6702e083feac57f8ea2910535ac0708716b39ad6d` 为 PASS/answered/cleanup_proven、pending0/unknown0，唯一安装目录/登记和该次临时副本/sentinel 均 absent。这不是把原 FAILED 改成成功，也不新增恢复账本。

随后新的完整 canonical 安装/卸载周期已 PASS：`installer-chat.minimal-1792240008260-cf4abfe8-3d7c-4797-95ab-637a21abc891.json` SHA-256 `89ca35dccb157caf026f446b07ff98f4547100c9fef77014ec8ac2b962d0cd01`，Session `01a149d4-4846-70c2-91b4-901123903963`。同 Session 两个 Turn 分别完成安装2步和卸载4步，共6业务，均 completed/answered/cleanup_proven、pending0/unknown0。原进程 supervisor 保留原生退出回执；卸载复制到夹具后使用最后的原始 `_?=<exact root>` 同步等待，目录、唯一 registry/shortcut/protocol 与 sentinel 缺失，原数据保留。默认空 workspace 在 App 活跃时 EBUSY 单列，不能将其称为已删。

独立安装后的打包 App bootstrap 属于另一个证据面。`windows-installer-install-a05b55b7-2525-48a5-b8f3-768019e14b91.json` SHA-256 `53864beb2a354e0f8be781bf6891f6dc4c8cbea1cbf9234f2920f7712d5fedc6` 已证明真实 NSIS8748 ordinary、exit0、无信号，实际 KnownFolder 安装位置为 `C:\Users\muli\AppData\Local\Programs\NomiFun OS Acceptance 439f698fa737498eb1550807a39bf3f8`，九项资源完整、安装 NSS 摘要匹配。日常旧版在D盘不代表本次 KnownFolder 也在D盘；独立 App 正常 setup/login、业务 smoke、正常退出和最终卸载另记，不借 canonical cycle 代替。

最终 debug App21108 已通过单次现役产品退出正常结束：`windows-fixture-close-0f0a01eb-954e-4217-a17d-3cc8e393cb68.json` 回执 SHA-256 `af2955bb245b51c093ca3597c3e467eab92914a746ff8999e05c9b38eaf56500`，同 retained HANDLE 的真实 exit0、句柄关闭、observer30892 exit0/reaped。exact creation 为134367071328824857。独立全树15个 exact identities 全部 gone，`windows-process-tree-closed-764d1b92-72b0-4307-9e7f-55caaca951b9.json` 回执 SHA-256 `0b3b15ca56b98fc550abf416ae091ce22059bcc3239b63e7079315e22a71c104`；本次 lmBztb snapshot 从 present-before 到 absent-after，`merged-final-browser-lmBztb-native-teardown.json` 回执 SHA-256 `3a8765606fc26d36dc860842838551b0e88951778c188e2db3cfa5cd03565a7d`。process receipt 的 overall_cleanup_proven=false 为其固定证据范围，未修改该字段，树和 snapshot 分别补证。关闭前双守卫再核对 Desktop230/UNC336 以及原 pending1 不变；旧 BoHTo0 与34d故障证据继续保留。

App21108 关闭后，七份已明确终态/恢复报告对应的14个 workspace/fixture 候选中，12个已证实空且无 reparse 的目录逐个非递归删除，2个已 absent；未扫描或递归清空 Temp，没有修改 DB、证据、34d 或 BoHTo0。`windows-debug-21108-empty-directory-cleanup-735a1f21-7b18-4c61-86a2-fac1854aefb9.json` SHA-256 `d0ebbea92c97b9b2384df85fb63546e1c71a7fc2152b58cd7d02f0d215227502`。自有 Vite exec43912 单次 Ctrl+C 后结束1，独立核对无 Bun/Node Vite 进程和配置5173监听；这是主动停止开发服务，不是 App 失败，`windows-owned-vite-final-stop.json` SHA-256 `27270ef631579bde084b22ecebf0578766a6d9d7c9a09e42b7391dcbf8b4e0e4`。

最终验证脚本三个测试文件合跑实际退出0：`global-os-native.test.mjs`、`global-os-windows-fixture.test.mjs`、`global-os-windows-installer.test.mjs` 共49通过/0失败/1个POSIX跳过、454断言，含真实 Windows parser/token/只读库存。仅覆盖本次驱动变化，不冒充新的全量 Rust 或原生产品回归。

独立安装版首次 fixture 准备报告 `windows-fixture-d0f267b4-c7fb-46e1-a091-5b0ad42834bc.json` 保持 FAILED：真实安装 NSS executable 启动为PID29936，rendererIdentity 的 CDP 操作返回 exception，原驱动仅保存通用错误，未保留明细，不能猜测原因为预算或瞬态。此时尚未执行 setup/login，也未创建 Agent Effect，不重新 spawn。后续同进程只读诊断 `installed-release-29936-readiness-diagnostic.json` SHA-256 `0393d14af983a4fbb1d1cdef0e3e899e896b94e1f24338bc4001d8c5672dd134` 真实证明 `http://tauri.localhost/#/guid`、DOM complete/root已挂载、secure context、1280×832，`plugin:app|version` 返回0.8.12；同DataRoot公告PID与 `/api/auth/status` HTTP200/needs_setup=true 匹配。诊断的 spawn/setup/login/effect 均0，不追认首次准备成功；后续只能在这个原进程和数据根走正常产品初始化。

同一安装进程29936 已完成唯一一次正常 public setup/login，生成新的 ready `windows-fixture-existing-bootstrap-701a6bfd-4527-435f-824b-6a52d41bcb2a.json` SHA-256 `98579c03fa54e5f1e6133851f78a76efdcced10df4165f2d4fda6ef8ce75e59a`。实际 NSS 全摘要、ordinary Medium8192/同登录SID与Session、renderer0.8.12/1280×832、真实 authenticated caller PID29936 一致，额外 spawn/新dataset 均0。原d0f报告 SHA-256 `485732e768c5c0d178ad308df05cc7985a73ecd6842dbfcfa8b231afab2f8dec` 保持原 FAILED，不倒填成功；正常初始化成功仍须随后独立业务与关闭、卸载证明。

真实安装版的 Minimal 文件/进程回归已 PASS：`files-process-chat.minimal-1792240724571-793c0224-4c5a-45d5-9cef-8f93e82b5056.json` SHA-256 `e7bc20078f3f08464b45958ba4a2f55fd33fe8f43f2d7ab0009cb8260b4a326e`，Session `01a149df-2af0-77a0-a6b6-0205cf1a0f02`。12业务/16检查，completed/answered、pending0/unknown0、cleanup_proven，使用安装目录中的NSS executable与嵌入式release UI，不依赖已关闭的Vite。空默认 workspace 在App活跃时 EBUSY 的原观测保留，正常退出后的限定清理另证。

安装版 App29936 也已单次现役产品退出：`windows-fixture-close-76a235cf-e0e3-48bc-a29f-49f49cf4b158.json` SHA-256 `1ac9704b886a07b3477eef36e4234dc576aeb89d3b487251ac6ea5f9808ebbc5`，exact creation134367140636824078、retained HANDLE actual exit0/closed、observer12828 actual0/reaped。独立 exact tree 的7项全部gone，closed33f72fa8回执 SHA-256 `09beddfb268b2a317b40b56896f293a24ded7bd09c3ee9b44959ed74e206c4fb`，未 kill 或靠晚PID查询补造退出码；真实安装目录与新DataRoot保留给随后原生卸载和数据保留核对。

随后独立原生卸载 actual0：`windows-installer-uninstall-be351dac-c1a1-4512-84a6-ce3f00cb0d64.json` SHA-256 `b673bc8f8359d0f7ddd3e48f517488831902f1ea55307e7f670389b0d173f5f3`，status=uninstalled_data_preserved。普通进程16644/elevated=false、`/S`、actual exit0、signals0；限定等待后安装root、全部唯一registry/shortcuts和对应image process消失。真实新DataRoot的SQLite主文件完整SHA-256 `17ef65729b11af9e7dc7c5b86dc0622c661d673f7401f21a4a7366696981cd72` 与sentinel前后相同，日常登记/autostart保持。该标准NSIS `/S` 模式保留首进程退出并观察实际目标消失，不冒称独立持有其自复制卸载child的HANDLE/reap；前述canonical复制品 `_?=` 同步链为另一份更强的进程证明。没有host删除安装root或重试安装/卸载。

安装版烟测的两个空目录也已精确非递归删除：`installed-release-29936-empty-smoke-cleanup-f1c15378-ea61-447b-a98f-aa5b874c1eda.json` SHA-256 `7d867ab34e7b46a0a9c535ed618b0bfe44f9cd56cdbadd1e53d52aba21052551`。两份新旧数据根、JWT及有用证据保留；没有递归清空Temp、禁用系统保护或启动日常App。

最后一次默认测试App退出且卸载库存核对完成后，日常URI修复的只读守卫通过（7a4beb9e报告SHA-256 `8f4b6e3c29e4a6811d9fa15ebfe294cfdd9656f43495b7ced8f45303c4664b43`）；随后 `windows-uri-handler-restoration-f65ea425-32bd-4bf1-8159-84fe18f31dde.json` SHA-256 `22b576731a690069c9651d2ac2c9419bf80d68352166d78de0399b8571464798` 为 applied_and_readback_verified。pin/hash核对当前d22测试副本与日常0.8.4后，只写两个既存默认值，32/64视图读回均指向 `D:\Users\muli\AppData\Local\Programs\NomiFun\nomifun-desktop.exe`；无进程启动、键新增/删除或ACL变化。此为用户已授权的目标修复，不冒充未知历史原值恢复。

## 当前 main 的合并与补验

完整源码和记录已保存并普通推送为 `0590a1922f8f2484314b00a75889640595fb08de`，tree `545bb5ee9297f5b312c11f624bf6da5ce3ca12aa`；已创建并附加 [draft PR26](https://github.com/nomifun/nomifun-desktop/pull/26)。PR创建时main为2c6b1df36，GitHub只读核对mergeable=false/dirty。未把该冲突交付称为最终整合；在feature分支继续普通merge，保留所有提交，不rebase/force/main覆盖。

merge-tree与实际merge均仅三份生成合同冲突。保留双方相同的平台矩阵和fixture refs，使用合并后的正式Rust schema/Cargo.lock通过现役generator重新生成，而非盲选ours/theirs。`write`、`check`及定向diff-check实际0，原受限token的嵌套git EPERM和过早读取冲突manifest失败单独保留。`windows-current-main-contract-regeneration-receipt.json` SHA-256 `b19dfa371f14d131e288d68aa60d3636678b1b48045c9430eab330a8357d876d`；仅三冲突文件被再生。合并canonical schema摘要为 `b41dc3fb2d72933ea17deddd58d70c166395725821fce31078351d26b91fd577`，lock摘要 `b64ca7e6b7ff0b4d4e68ebd7504c1692c1a9acb9be8fa1a3c995d0c488d249a8`。

自动合并独立源码审查未发现Windows typed NativeFileError、historical scope refresh/fatal owner、effect→read失效、terminal只读或late I/O的覆盖。新main的等待/取消、会话结果和renderer变化须补相应检查和native，不从源码审查推断成功；当前UI typecheck与2180源880×600边界已实际0。

该普通merge已保存为 `b014415383db324dde21fbf9307f0ada4dd82f9a`、tree `d2aa2cc14e2b250fe99c83d2a023f7a57519d353`，parents为0590和2c6b1df36。该树完整 `bun run check` 实际退出0，日志 `windows-current-main-source-check.log`；后续真实修复另验，不追称整个补验批次已完成。

新合并Rust lib首轮实际101：新main的 `turn_silent_completion_tests.rs` mock初始化 `AgentEngineError::CapabilityKernel` 漏填Windows实现已加入的可选details。仅cfg(test)夹具补 `details: None`，不修改生产typed native错误或未知效果判断，原 `windows-current-main-runtime-session-process-tests.log` 保留，修后全组另记。

新main改动涉及的38份UI测试首轮476通过/1失败、2710断言，`windows-current-main-ui-tests.log` 保留。失败为初始消息慢准备期间不应预热却实际触发：独立单case复现1次、整文件3次，调用栈与参数均精确指向当前conversation的SendBox300ms focus timer。Nomi已有独占passive准备链但仍叠加focus链，前两项卸载后旧timer也未取消；不是仅并发负载或跨文件偶发。修复仅合并现役Nomi准备owner与共享timer生命周期，不删除真实断言或用sleep放宽验收；修后UI和新构建的真实产品结果分别记录。

最小生产修复为Nomi只用既有passive准备owner，shared SendBox不再为它叠加focus POST；共享计时器在卸载、Session或skip状态变化时取消，其他产品当前Session的显式focus预热保留。新增三项有效回归，修前unmount/Session-change两项真实红，修后4文件29通过/319断言、typecheck与880×600边界0。正式回执 `windows-current-main-ui-focus-fix-receipt-v2.json` SHA-256 `2d4db475a25bf2dfd1ef2e8755958d46338d1fe4880b5e5da7642257b7c1f313`；更早null回执错误保留但不作为成功依据。同一38文件修后全批480通过/0失败、2739断言，`windows-current-main-ui-tests-fixed.log` 实际0。

Rust修后当前组实际退出0：process lib147、Runtime470（另1保留忽略）、Session259，共876通过/0失败；`windows-current-main-runtime-session-process-tests-fixed.log` 保存完整结果。App相同browser-use,computer-use单次no-run编译实际0、14m02s含排队，同一固定exe精确执行14项全部通过/0忽略，含真实Windows pipe/ConPTY迟到控制、waiting/cancel/recovery/browser owner4项、read投影6和missing-default Effect key2。`windows-current-main-app-targeted-tests-receipt.json` SHA-256 `dbb9aa59b863068da37b108d111a5685e9874b7ec85560e4841f6e4872e9ad7e`；编译时存在已说明UI生产/测试与cfg(test)修复，不把其HEAD b014误称整树干净认证。

新增可复现的 [Windows等待原生驱动](../../scripts/validation/global-os-windows-wait-native.mjs) 已按审查后完整bytes纳入源码，SHA-256 `580e4f457a39f37296319a509b2406fa041cba4cfcd79eba5cce7fab20037eba`。25项纯检查和Windows PowerShell实际Parser静态检查通过，独立审查回执 `current-main-wait-native-independent-review-580e4f.json` SHA-256 `7ee081995de6f2c4d71ac460940fb56c4790cbb7e5f32ced87f27a5446ccb6af`。计划普通/管理员各一次owner自然唤醒及普通Waiting精确取消：共用原App/owner/同默认General Session，外部唯一gate只为测试控制，不伪造人类回答/授权；核对同Turn Waiting、1.5秒零模型采样、原process_id/完整输出未被terminal消费、typed终态、completion事实及cleanup。当前仅完成驱动检查，真实App/UAC结果另记。

## 新生产冻结与补验 App

新干净生产冻结为 `925e775a1c070fbbef75d8c2db0bbb5c27ce97e3`、tree `55a42d1b26a70c28faf9b11e4709c1f7ea72f36f`，包含current main2c6、上述UI生产修复和完整等待验收源码。该树再次完整 `bun run check` 实际0，`windows-current-main-final-source-check.log`；不将b014修复前检查套作这次结果。

`cargo build --locked --offline -p nomifun-desktop --jobs 1` 从干净925实际退出0、13m21s，保留incremental finalize native5 note。新exe SHA-256 `8dbc11f5d32e24ee798b4d7de3a784e7b3ecb14df29e7ab2fa39bb90dbf24e7a`、311493120 bytes，冻结到仓库外 `accepted-debug-current-main-8dbc11f5d32e/nomifun-desktop.exe`。本轮真实组件为 `21dc79a9dc51a819d9de73aa34e1fa189e682fdb8bc22588dcbd7d95b7d76466`，三份实际Cargo output同值并匹配最终binary literal，不能沿用旧853a或Mac终止符条件。`windows-current-main-debug-build-freeze-753b4802-d35e-459b-bf73-820c9f185b8a.json` SHA-256 `66b70a7074cfe12daf50bf4db069305cc9bdd95aa2f863e3ea45f448d7d644f9` 保存输入、退出及copy前后全摘要。

同原87f数据根/JWT升级到该8dbc App32104/API2084。首次 `windows-fixture-upgrade-596534d5-4007-40c5-945a-f7e7e927b776.json` 保持FAILED，SHA-256 `0cfce455da2fc3b38b3dee1258e069db20b5240302e091214c9d1d396d8ff565`，仍是被原通用CDP包装省略明细的readiness exception；没有猜首因、重启或重做setup/login。现役同PID只读verify产生 `windows-fixture-upgrade-readback-267603e7-1133-45d6-989c-01368e35ed21.json`，SHA-256 `bebcf62d4fce54371de968a12a02f3c36b90e1e55c97331ed84cc392e7d7ce13`，真实caller/PID/image/creation134367173603608424和双守卫通过。随后b76ba711 direct freeze再次核对Desktop230、UNC336及pending1不变，mutations0；新namespace业务与新的唯一b4 NSIS变体结果另记。

### 925 原生业务与认证取消

首次新UNC报告 `unc-chat.minimal-1792243870110-78d823ce-ff1c-4b39-abf9-d1d2cdd493f5.json` SHA-256 `83d45f1170417b4123d39739b6461b0a5ed048ee31a9c6d0e26e282e1ec1074b` 保持FAILED。两项普通夹具准备已成功，但share-create没有ToolStarted、dispatch或Effect；管理员instruction-scope读取被Windows两次以1223取消。原Session `01a14a0f-36fc-7ec0-9a40-823eb4344fc5` 的155事实保留，failed/pending0/unknown0；保留的诊断checkpoint不授予resume，未推断是谁取消、认证超时或lease丢失。独立只读审查 `windows-925-unc-uac-lease-independent-review.json` SHA-256 `03d8caaeb6a14b9f91be50665d4acfe7f72db8af01439ea091fb9c9b41521cc5`。

确认唯一share真实不存在、普通物理夹具完整归属后，同原Session新Turn仅以普通身份准确删除本次夹具，completed/answered、pending0/unknown0、cleanup_proven；`windows-current-main-unc-ordinary-cleanup-execute-a611a4cf-d6ff-4765-aa95-6bb9626f840d.json` SHA-256 `54a4d22a6729e86048d598ec616a91c35bcafc2272c211b44953d0c69c16fabe`。没有share创建/删除、管理员重放、旧Turn恢复或checkpoint强清。普通公开测试目录的owner/稳定marker/无reparse/精确entries与ACL前后核对，不误套JWT私有目录的独占ACL要求，也未改ACL。

用户在实际新consent11824窗口允许后，下列新唯一业务均通过；所有报告保持各自完整摘要，原认证取消报告不改为成功。

| 当前925业务 | 结果与报告SHA-256 |
| --- | --- |
| UNC | 16业务/23检查，`unc-chat.minimal-1792245157557-00cc74dc-6baf-4864-ac60-1bfda0cae9cf.json`，`6ead8b3e084de11aabce7b5eba43cf868f3cfc36c101d5e5ec380a18d0a67db9`。 |
| 管理员 | 8业务/10检查，`administrator-assistant.general-1792245503337-b05850c0-02c1-4c82-b971-4261545c149a.json`，`491697d9a19f6208f418e3fca58fc5efd650072a5dea905ceaafec4e61b3d12f`；真实PID29488取消、terminal/reaped并消失。 |
| missing-default Minimal | 12业务/16检查，`files-process-chat.minimal-1792245684296-1e46eba1-cebf-4a39-988c-33de1d3739ee.json`，`4d95bbb4ab6eec0658405185ee492f9e0992d3eb3a53b2051bcb255277cec53c`；默认目录已不存在，沿原owner/binding/resources/DataRoot，未新建数据集。 |
| Browser | 13业务/22检查，`browser-assistant.general-1792245762937-a2e273b6-1176-4241-8b0f-ffd14ea59d0c.json`，`064cc9f6005591e64062e47a0a94664278263c1fb611cf29fb96e44be465f829`；新上传快照为WgkGMz，其App关闭后的清理另证。 |

四组均completed/answered、pending0/unknown0、canonical cleanup_proven。App活跃时空默认目录EBUSY观测保留，不能仅凭业务完成声称宿主与快照已经清理。

### 925 release 与唯一 NSIS 变体

现役 `bun run build:win x64 --config <installer-current-main-config.json>` 实际退出0、28m32s；source925/tree55a42d干净起建，日志 `windows-current-main-release-nsis-build.log`。未设置会导致ORT链接失败的CARGO_NET_OFFLINE。唯一变体nonce为 `b4cf8b7c255f43088f91abfd64e201ad`，仅productName/identifier/mainBinaryName/deep-link四项变化，仍为unsigned/currentUser，不当作正式签名发布。

原始release executable SHA-256 `fd19fd652425c625dca6442a72e8dab485f360569a19f9f396ba7833908ffeab`、261672448 bytes；官方NSIS SHA-256 `808bd35a3917abc7c5ec0e5070a537222c37475457fca0f1820f6db0ab4c5420`。实际渲染NSI来自 `target/x86_64-pc-windows-msvc/release/nsis/x64/installer.nsi`。官方27-byte标记中UNK→NSS后的安装exe完整预期SHA为 `5c620466e0ff725d7ff2258a50b813982bd01c6298b087cc5c477fdf06bb437a`，不是忽略PE区域后比较；组件仍为实际21dc79a9。冻结回执 `windows-current-main-release-build-artifacts.json` SHA-256 `309940e0c86ecb1fa9d9ea43c04c14fe79ed7e387bbe76afac8d4145f7f281de`。

只读prepare确认唯一对象不存在、9项资源与NotSigned：`windows-installer-prepare-6c738544-4758-48d1-9b38-e71a3293e3ce.json` SHA-256 `775c9921ac5706110a105a76aa8dda4e73ce60776c48d58cc3d123ecb0cfdf56`。完整canonical安装2步/卸载4步在同Session两Turn通过，`installer-chat.minimal-1792246186555-08151487-1370-49a3-9c44-e03db01a0496.json` SHA-256 `60376cb53ccd4ef02c3c47123311e9f712aa366b159cff0d4ad30bca06d826af`；7检查、各Turn completed/answered、pending0/unknown0、cleanup_proven，真实NSS/resources/registry/数据保留均核对。

唯一对象已由canonical卸载后，独立原生安装actual0：`windows-installer-install-0a66739b-0060-4e80-8562-a370fe178911.json` SHA-256 `7934838f1d973d6dff9cf7c28d36de4ee55ffba34c65594deabca0c8976db038`。普通PID11292/elevated=false、`/S`、signals0，9项资源与完整NSS摘要一致。实际安装位置是当前KnownFolder选择的 `C:\Users\muli\AppData\Local\Programs\NomiFun OS Acceptance b4cf8b7c255f43088f91abfd64e201ad`，不是日常D盘旧安装；该安装版独立启动/业务/退出/卸载结果须另记，不能由安装退出0推断。

### 当前 main 等待与取消的原生核对

首轮 `current-main-wait-native-9f13ffb1-045b-4b27-bc4c-4571ae6526f3.json` SHA-256 `8049798f3e2c3a1b7f5c33117e92aad67184cf2a3799d4ad41335f8a4eb1990b` 保持FAILED。普通自然唤醒实际已完成，但验证器把事实预先限制为root operation correlation，漏掉以自身Effect ID关联的原Effect。原事实为start Effect52→Succeeded54、Waiting128→owner观察129→resolved131、completion168→cleanup171→completed174；PID11640的原retained HANDLE实际exit0、observer0/reaped/closed，park期间模型步数5→5。只读补证 `current-main-wait-first-ordinary-native-supplement.json` SHA-256 `3254b7f5b81e70a17b1984cf9a57597b966097122d13f85041ec0edfebbc1131`。修正仅让正式Effect核对消费完整同Session事实，仍精确匹配operation/call/Effect/input/resource/causation/结果；真实174事实离线正例与错误归属等负例通过，不重写原报告。

该修正c26b9a4f脚本的32项纯检查及独立复核通过，`current-main-wait-native-independent-review-c26b9a.json` SHA-256 `6d022f385ab93d7cf361287eacae2fe4a89ecbfa32cd72fc99d54fd2b2fe9cbb`。新唯一完整三项报告 `current-main-wait-native-58289691-9cd3-4698-8a4d-b1b2cabf6714.json` SHA-256 `0709ce9d3a8c73e363887d620e042258e6d7bb4699b1b8e6404cb67fc3b014fb` 仍保持整体FAILED：普通与管理员自然owner唤醒两项通过，第三项精确取消的真实结果为cancelled/pending0/unknown0/checkpoint=false，但验证器再次误用自然完成的cleanup-before-canonical-terminal顺序。

第三项原事实为TurnStarted353、Waiting477、Session-owner canonical cancel478、原runtime owner cleanup479、runtime cancelled480。源码 `native_cancellation_preserves_cleanup_but_blocks_new_admission` 专门保留先封存取消、原lease随后写清理观察的合同；host_cleanup_proven必须等待原进程真实terminal/reaped、工具及资源join成功后写入，不能只因cancelled或0/0推断。PID22512 creation134367202335478139的原HANDLE实际exit3221225786、closed，observer2260 actual0/reaped；取消不是自然exit0要求。只读补证 `current-main-wait-second-cancel-native-supplement-4b05d2c8-0cef-4301-8e36-cee75e0cce06.json` SHA-256 `0382834097da63c94a26f2cf484403c6fbcbdd54e6e6a79dd1ea62704a4ee949` 核对完整480前缀、原message/operation/binding与typed task-plan均未变。未借缺失gate伪造唤醒、未填用户回答、未重放原取消；验证器修正后的新原生结果另记。

最小修正后的driver SHA-256为 `5bef5810f2a491274e6497da70f7107f7ba55a5496a7ae51dbc045d3f8676f52`。取消严格匹配原Session/operation/message/cursor收据，再核对原generation/build的cleanup与随后Runtime closure；取消状态先出现而证明未齐时仅有界等待，错误或未知证明拒绝。自然完成仍要求cleanup先于completed。44项纯检查实际0，含缺失/错误归属、generation/build、乱序与unknown反例；真实480事实的独立离线复核通过。修正回执 `current-main-wait-cancel-readonly-validation-dd2f8a78-1a68-4c30-9ef6-0422beb39c47.json` SHA-256 `766c87156d4ddeaf09fab2f7f98c7e441b37193aca5397f95c59d33713631667`，独立review `current-main-wait-native-independent-review-5bef58.json` SHA-256 `2128c668539e50683e935a6633e64ecbd7b5492b20dad248232ada749605b268`。这些是验证器检查，不追认原两份FAILED为原生整组通过。

原生补验期间远端main后继至 `737c6046bf7c823f674ca7cbaa4ad66bbdcd22ec`，相对2c6仅新增robot registry退役恢复/endpoint错误与相关UI/文档。正常fetch后，`git merge-tree --write-tree HEAD origin/main` 实际0、结果tree `dd253e820fdb7eb1a16777bc7a5eaaffe9510d13`，无冲突；这是只读可合并性检查，未改工作树。当前原生构建仍严格对应925（含main至2c6），不将该后继生产提交借作已构建/已原生认证；最终remote/PR状态以推送后实际回执为准。

使用最终5bef58 driver的新唯一完整三项原生验收actual0/PASS：`current-main-wait-native-cdae593a-7f13-4081-9a50-f7dce62e66b7.json` SHA-256 `a776886fa9fa47811473d898ec7b7c9cb39dc19a059c544149bcd5abdfd5b35b`，同General Session `01a14a44-cf75-7323-9dab-7bae737184ee`、同925/8dbc App32104。普通PID27524与管理员PID25036自然owner-first均completed/answered；普通PID32556精确cancelled。三项pending0/unknown0/checkpoint=false、原owner/frozen binding保持且cleanup_proven。真实park分别2837/2840/2929ms，期间provider计数5→5、11→11、18→18；无伪造人类回复或额外模型采样。自然退出的原process_id正式poll与完整输出一致、exit0/reaped，取消取得exact receipt478→same generation/build cleanup479→runtime cancelled480及实际exit3221225786。三份原retained HANDLE均closed、各observer实际0/reaped；该进程退出证明不自行扩大为整个App资源清理。两份先前FAILED与补证原样保留。

该报告的独立只读复核 `current-main-wait-native-final-independent-review-cdae593a.json` SHA-256 `56ea661d14c487f12d8d7f635da03c0aaa1973f242306309718046d3e3f150d3` 实际0，复核原owner观察、poll、typed结算与旧失败不变。成功报告保存root-operation Runtime窗口与运行时已精确核验的Effect摘要，不嵌入Effect自身correlation的完整payload；独立复核不据摘要重建过去Effect，正式事实仍在保留的原数据根。

### 925 debug 正常退出与资源清理

最后双freeze `c9941ce7` SHA-256 `f1081d7867aab1e2a2786c91c97d52ee69f31fd193f09d99a86b06bfff3f4054` 核对原Desktop230与UNC336/pending1保持；Desktop守卫SHA-256 `1f92b464f3ac0f755281f9d639002ddd126984e4c8f84261423219486b98b4ab`、UNC守卫 `913c8fdbe0cdbe8534bda1699a2f43086e3bbdca2360e35a556dc41baf3bc4c1`。未提交owner reconciliation、未清理或重放旧34d。

32104现役产品单次正常退出actual0：`windows-fixture-close-8dca958f-276b-4d61-b5ed-888c5843fbc5.json` SHA-256 `69d19a951118da97f99092661f676f84662c71a718ed224d18c8b819474e043f`，exact creation134367173603608424、原retained HANDLE exit0/closed、observer12760 actual0/reaped。关闭前完整15项owned tree的before-fa916d7f摘要 `eb9437bd4996b62356f3ef68a9544797ce6b0689597cf49ee5f8b5f1a80122a6`；关闭后 `windows-process-tree-closed-3b1dd0e2-772b-497b-9e84-0db9e8ec88c4.json` SHA-256 `8f625860f2f7ce938b668e76e5c1fdb9a8a0f5b2f1e987af858b2db827967206` 证明15项exact gone，未kill或重启。

本轮WgkGMz快照关闭前真实present（2802fbe3回执SHA-256 `f2dde53db69c6598e94437881a480a2d2d4688090c5372c39ef7c8fb9a1b50db`），关闭后真实absent：`current-main-final-browser-afterclose-c194b8cc-d804-496d-8d77-187ba9596b3a.json` SHA-256 `210dfe3d833a79fbee5f50a8b629a6932ca1b05c3d62890a45965a3156cc03ec`；host delete/content read均0。不把原BoHTo0残留改成已清理。

随后原exec17924单次Ctrl+C停止本次Vite，真实中断exit1按实记录；原生库存核对15536/11612/33504及相关cmd26496均absent、5173无listener，未kill。`current-main-owned-vite-stopped.json` SHA-256 `78f70d99131eab0c20696db9441619969ebf557dc3d629a23e8e08e80160c785`。停止前记录脚本首次用了Set-Content不支持的NoClobber参数，未写文件或发信号；改用CreateNew落盘后才停止，原错误保留，不把中断码写成自然0。

限定清理仅处理已证明归属、无reparse、原业务已settled的gate及空目录。首报告 `current-main-final-known-fixture-cleanup-cbe42f32-e083-4013-972f-bbd3d4e0f61a.json` SHA-256 `dd87070a891cbe35485a855303ad9ea936749a5f3ebf6fdfc87eb590ef5ca67b` 保留FAILED：12次非递归rmdir已成功，随后外部helper的map(basename)把index误作ext，在两个wait根删除前报参数错。窄续执行先核对这12路径真实absent、全部源报告摘要不变，再仅处理余下已结算wait根；`current-main-final-known-fixture-cleanup-8e4bbd1e-46e9-4c65-ac92-7259fefb8962.json` SHA-256 `2cab5e56088465276ec72c1aec539a056db7901afda7b29082d27ebeb201f2a4` actual0，新增2个known gate unlink和4个空目录rmdir。合计18次删除均非递归，无API/DB、原34d或BoHTo0操作；没有把部分清理错误抹成全程一次成功。

### 925 实际安装版的独立生命周期

实际b4安装目录中的NSS exe（完整SHA5c620466）在Vite停止后仅启动一次，PID14700、全新唯一私有测试数据根。首次 `windows-fixture-5d05ac1e-3e97-40a6-8382-4f36b06bdff2.json` 保持FAILED，SHA-256 `1d064fac8208a98b7763e98e5313ae52e0d37b770a99791d1f6e704f873853cf`：8.66s时rendererIdentity经通用CDP包装报异常，没有形成renderer/identity/token，也未到setup/login。未保留具体CDP exception明细，不能断言首因或追认准备成功。

同PID只读诊断 `current-main-installed-startup-readonly-e32e2e68-8b2a-46d1-afa5-8c8e9d3b520f.json` SHA-256 `baec85d7c369585a293e1160b613702357b88111373d33cbb46ad76c56527957` actual0：当前同安装image仍运行、嵌入式页面完整且无Vite，原rendererIdentity表达式与plugin:app|version实际成功，正常auth状态needs_setup=true。该诊断spawn/setup/login/effect均0；后继仅可沿此原进程/数据根作唯一正常初始化，不能重新spawn或把当前健康倒推为原异常原因。

同14700的唯一正常bootstrap actual0/READY：`windows-fixture-installed-bootstrap-continuation-01a08ff9-24d5-4ec8-8313-b59231986108.json` SHA-256 `7d314a83ba031287e7e3fdbe730c595d0a1bfceb1f2dee4e3c1b3f58988607db`。普通Medium8192、同登录SID/Session1、原进程创建时间与完整NSS executable匹配，真实authenticated caller14700/API8245；setup1/login1、额外spawn0。引用并保留原5d05 FAILED，不倒填成功；业务与关闭、卸载继续独立证明。

真实安装版Minimal12业务/16检查actual0/PASS：`files-process-chat.minimal-1792247978741-2d9e8ac3-94dd-4575-9ba1-a2bfaaf81d41.json` SHA-256 `803d84158947177c0eca670df2ad11717e106a94a20c0955859be553608cd741`，Session `01a14a4d-db7a-7db2-b73c-794b3a2ed70d`，completed/answered、pending0/unknown0、cleanup_proven。使用实际NSS安装image与嵌入式release renderer；未借用debug或Vite。

安装版14700现役单次正常退出：`windows-fixture-close-814b063c-9e55-4056-ba47-4996963d245d.json` SHA-256 `18e81518e825de15afd40ed974f2bfbcb2634d98cd174e8b6fba6ac3a6b04951`，exact creation134367213914580880、原HANDLE实际exit0/closed、observer27252 actual0/reaped。完整7项owned tree关闭前9937c901摘要 `6cf093ccf8d818fdae2e76ae3fcd84bf34a214464370118d4c97fd4feea880eb`；关闭后 `windows-process-tree-closed-100153b1-c618-4059-a690-3b5ed4f44bf2.json` SHA-256 `e2969bc13e6db4268ebecb6b0fbb404a2517ac3d1c7fe764491d13c5823fef70` 证明7项exact gone，未kill/重启。真实新数据根与首FAILED保留，安装目录/登记交由随后独立NSIS卸载核对。

独立原生卸载actual0：`windows-installer-uninstall-473c360b-1249-4543-b736-6c56f32fa108.json` SHA-256 `6c544985252c616f7b988ad9353468632181064e0659974e11ad471e542bed13`，status=uninstalled_data_preserved。普通PID31508/elevated=false、`/S`、实际exit0/signals0；唯一安装目录/registry/shortcuts/image进程消失，日常登记与autostart保持。真实新SQLite完整SHA-256 `e0605efe3f2618a564499e120e66ee772fceee21a423f245917b79e927ce6eb0` 与sentinel前后不变。标准NSIS `/S`只保留首进程退出及目标真实消失，不冒称独立持有自复制uninstaller child的HANDLE/reap；canonical `_?=` 同步链另有更强进程证明。未手动删除安装目录或重试卸载。

本轮安装版烟测的两个空目录随后精确非递归删除：`current-main-installed-empty-fixture-cleanup-49e42c65-5703-4db9-b5f8-c80b97eee0cd.json` SHA-256 `1c1b04f73b78b4edee05cc75eb371dd5193bd588705df671e1a163620acbd2a4`；源报告摘要不变，无安装根/data/registry操作。

最后的日常URI只读守卫 `windows-uri-handler-restoration-4e3608e6-458f-4fc1-9102-ef9f16773a9a.json` SHA-256 `45888a76879eb94d2586fe30dbda62bc01359f3da7c283b96b85c0657d680204` actual0；然后执行 `windows-uri-handler-restoration-4ea9d1c3-ad19-4a83-b6cd-49ae26998d67.json` SHA-256 `7dd0eb2ce091734b70d106c81753414e551281b41ce3c4e48907bb6c7417a57d`，applied_and_readback_verified。pin/hash核对当前8dbc测试副本和用户指定的日常0.8.4（完整SHA4e0920b1），仅两个既存HKCU默认值实际写入，32/64视图均指回 `D:\Users\muli\AppData\Local\Programs\NomiFun\nomifun-desktop.exe`；进程启动/键新增删除/ACL变化均0。此为已授权目标修复，未知最初原值未被伪造为已恢复，原34d未结算事实不变。
