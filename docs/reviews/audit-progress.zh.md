# 全局代码审计进度台账

最后更新：2026-09-12。此文件是续接入口，不能把测试通过或目录扫描等同于模块审计完成。

## 续接位置

- 当前批次：R82/R83/R87已验证并独立提交，源码截至 `31fa8aa0c`；上一已核实远端 `793ab368e`。R84 broker继续，R88 terminal已放写，R89接续需求页剩余范围；Execution engine已补读至文件末尾，未闭环问题沿用R73-03。Browser Use暂跳过。
- 后续集成：当前分支已包含 `dd35e01de` 的 MiniApp 分支合并；设计文档已跟踪。本批仅更新遗留路由测试，未修改 MiniApp 产品实现，模块仍待深审。
- 全局覆盖：111 个模块边界中，7 个已验证、55 个部分完成、47 个待审、2 个整模块审计中；218 个唯一问题/任务。完整阅读与跨模块问题闭环分开登记，Browser Use仍暂跳过。
- R1 证据：[首轮记录](2026-09-12-code-quality.zh.md)。R1–R39 各报告中的“未提交/dirty”描述是当时快照；当前提交状态以下方验收记录为准。
- R2 证据：[运行时与测试边界记录](2026-09-12-runtime-and-test-boundaries.zh.md)，包含改动、失败尝试、验证及覆盖限制。
- R3 证据：[生命周期与状态记录](2026-09-12-lifecycle-and-state.zh.md)，记录四个 Host 旧实现失败、Context falsy 初值及消息重放缺陷；保留待办，勿重复修复。
- R4 证据：[历史分页与数据库边界记录](2026-09-12-history-and-pagination.zh.md)，UI 最终 3304/0；数据库分页最终定向 14/0，需求/素材仓储 46/0；区分提取前后验证时间点。
- R5 证据：[Mount 生命周期记录](2026-09-12-mount-lifecycle.zh.md)，12 项红→绿证据，最终跨层 77/0、生命周期复验 23/0、进程边界检查通过。
- R6 证据：[IPC 背压记录](2026-09-12-ipc-backpressure.zh.md)，4 项红→绿，新增 13 回归；最终跨层 90/0、IPC 复验 7/0、进程边界通过。
- R7 证据：[启动与诊断记录](2026-09-12-startup-and-diagnostics.zh.md)，4 项红→绿、9 项新增回归；最终跨层 99/0。
- R8 证据：[运行时协议记录](2026-09-12-runtime-protocol.zh.md)，4 项红→绿、新增 6 回归，最终跨层 105/0。
- R9 证据：[在途容量记录](2026-09-12-inflight-capacity.zh.md)，3 项红→绿、新增 5 回归，最终跨层 110/0。
- R10 证据：[Host 文件发布记录](2026-09-12-host-file-publication.zh.md)，1 项红→绿、新增 5 回归，最终跨层 115/0。
- R11 证据：[清理证明记录](2026-09-12-cleanup-proof.zh.md)，2 项红→绿、底层观察回归；最终跨层 117/0、底层边界 23/0。
- R12 证据：[Runtime 绑定记录](2026-09-12-runtime-binding.zh.md)，2 项红→绿；App 定向 2/0，跨层 118/0。
- R13 证据：[Mount 提交查询记录](2026-09-12-mount-commit-query.zh.md)，2 项红→绿；App 3/0，跨层 121/0。
- R14 证据：[公共模式脱敏记录](2026-09-12-pattern-redaction.zh.md)，6 项红→绿；模块 16/0，浏览器调用方 20/0。
- R15 证据：[网络出站记录](2026-09-12-network-egress.zh.md)，4 项红→绿；net 43/0，知识库抓取 21/0。
- R16 证据：[代理解析记录](2026-09-12-proxy-parsing.zh.md)，2 项红→绿；代理 26/0，net 47/0。
- R17 证据：[代理进程记录](2026-09-12-proxy-process.zh.md)，后代持管道红→绿；net 50/0、1 个子进程夹具 ignored，进程边界通过。
- R18 证据：[代理缓存记录](2026-09-12-proxy-cache.zh.md)，2 项红→绿；net 53/0、1 子进程夹具 ignored。
- R19 证据：[精确脱敏记录](2026-09-12-exact-redaction.zh.md)，4 项红→绿；net 58/0、provider 19/0。
- R20 证据：[URL 诊断记录](2026-09-12-url-diagnostics.zh.md)，3 项红→绿；net 61/0、provider 19/0、模型 15/0、Agent 36/0；全量模型测试未完成单列 R20-02。
- R21 证据：[凭据编码记录](2026-09-12-credential-encoding.zh.md)，3 项红→绿；net 65/0、provider 19/0、模型响应 4/0。
- R22 证据：[网络模块收尾](2026-09-12-network-completion.zh.md)，网络模块已验证；net 66/0、1 子进程夹具 ignored，进程边界通过。
- R23 证据：[Agent 分词记录](2026-09-12-agent-error-tokenization.zh.md)，Bearer 红→绿，send_error 38/0；模型全量 4 线程 396/0，默认高并发热点仍待定位。
- R24 证据：[SFTP 发布记录](2026-09-12-sftp-publication.zh.md)，4 项红→绿、新增 16 项协议回归，nomi-ssh 27/0、后端 sink 4/0；真实 sshd 未运行，取消后临时文件/发布不确定性已说明。
- R25 证据：[SSH Glob 记录](2026-09-12-ssh-glob.zh.md)，四项真实 shell 红→绿，后端 sink 10/0；移除 ls，保留通配/字面字符和 shell 状态隔离。
- R26 证据：[持久 shell 记录](2026-09-12-ssh-shell.zh.md)。本批 24 个新测试、15 项行为红→绿；nomi-ssh 56/0、后端单元 21/0。
- R27 证据：[SFTP 流边界记录](2026-09-12-sftp-stream-boundaries.zh.md)，两项红→绿、共五项回归；nomi-ssh 40/0、后端 sink 10/0。
- R28–R30 证据：[资产/压缩/协议记录](2026-09-12-assets-compact-protocol.zh.md)，assets 10/0、App 2/0；compact 54/0、Agent 8/0；protocol 49/0，调用方 CLI 关闭/更新问题单列 R30-02。
- R31–R32 证据：[共享类型/配置入口记录](2026-09-12-shared-types-config.zh.md)，types 70/0、provider 定向 2/0；config 定向 66/0，剩余范围 R32-03。
- R32–R33 证据：[配置/Hook/schema 记录](2026-09-12-config-hooks-schema.zh.md)，config 最终 184/0、provider 7/0、Agent hook 2/0。
- R34 证据：[CLI 生命周期记录](2026-09-12-cli-command-lifecycle.zh.md)，CLI 12/0；Stop/EOF/配置队列及共享清理已修复，输出失败等仍属 R30-02。
- R35 证据：[记忆存储记录](2026-09-12-memory-storage.zh.md)，九项红→绿，memory 150/0、Agent 17/0；路径命名/多文件写入等 R35-07 保留。
- 下一检查点：R30-02 的 MCP 连接取消/bootstrap 归属、R38-03 的 brace/loader/MCP 容量、R42-04 的认证并发/密码持久化/代理信任仍未解决；ProtocolSink 输出失败和 inline shell 副作用已在 R47/R48 修复，勿重复做。R45/R49/R50 的剩余边界见新问题行；R36-04、R32-03、R35-07、SSH/PluginService 旧待办保留。
- 本地钩子收尾（2026-09-12）：用户明确授权检查后，确认 `.githooks/` 的 4 个未跟踪脚本仅是标准 Git LFS 钩子；仓库没有 LFS 跟踪规则，`git lfs ls-files` 为空。已删除脚本及空目录，并移除本地 `core.hooksPath=.githooks`；未修改全局 Git 配置或贡献者身份。默认 `.git/hooks` 只有未启用的 sample，另两个注册临时 worktree 无自定义钩子。以后启用 LFS 时可用 `git lfs install --local` 重新生成标准钩子。
- 钩子收尾验证（历史批次）：当时清单为 111 边界/90 唯一问题；仅钩子和记录变动，未重复代码测试。R40–R43 的最新结果见下方并行批次验收。
- 完成定义：列清子模块→核对生产入口及跨模块调用→检查并发、错误、权限和关闭路径→记录问题及证据→修改→对应回归通过。没有证据不能标记完成。

## R61 / R62 / R64 / R66 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R61 Wave4 | `cfcf38db2` | cargo test --locked -p nomifun-agent-domain-wave4 --lib --tests：20/0 |
| R62 Wave5 | `995ad0dbe` | 同上Wave5：11/0；主线程补删全仓无调用的批量binding helper |
| R64 Login | `a8efb4d6e` | Login/Auth/theme定向34/0；冻结后全UI3381/0（616文件）、typecheck通过 |
| R66 System→DB settings | `5f60efb88` | System settings/client_pref23/0、HTTP settings_routes18/0、DB settings_repository6/0、ai-agent language3/0 |

Rust均4线程。R66真实SQLite并发修改语言和通知开关，旧实现把语言覆盖回en-US；原子COALESCE字段UPSERT/RETURNING后通过，并覆盖实际缺行插入、部分/空更新和false值。删除旧先读合并及二次查询，无新锁/队列/依赖；四个重复DB内联测试由公开trait集成用例保留，标量偏好测试合并，两处mem::forget移除。

R64仅页面本地生命周期：同渲染重复提交、卸载后回调/计时器、延迟重定向、不可用storage处理；删除已注释DOM对应的CSS动画/未用ref。AuthProvider的请求代际和会话归属未改；首次管理员与remember-me既有策略不自动重设计。全UI通过同时复验了R56和会话翻译断言，不再沿用之前失败结果。R61/R62无旧红→绿记录；App owner仅只读追踪，当前未跑App集成、实际Robot/Channel/Remote和其他OS。

四批合计生产净减152行（R61 -57、R62 -12、R64 -57、R66 -26），测试净增263行（-20、+25、+311、-53），总净增111行。不把测试增长算成总代码量下降。

## R67 / R70 / R71 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R67 Wave2 | `5e74f9f3f` | cargo test --locked -p nomifun-agent-domain-wave2 --lib --tests：13/0，明确跳过2个Browser测试 |
| R70 Channel policy | `fd3d00c43` | App nomi_core_wave4/nomi_core_robot 定向17/0 |
| R71 AuthProvider | `206808376` | Auth lifecycle/contract/Login定向32/0；typecheck通过 |

R67完整阅读非Browser声明与测试：32能力/21actions；对已有三个严格workspace schema补Kernel入口校验，invalid输入不得调用owner，九个非法输入和三个有效对照；清理重复资源常量/测试fixture。没有改变Browser实现；typed dispatcher和Computer Context等剩余项见R67-03。无旧实现运行证据。

R70沿用R61-03：Channel在现有policy fence内读取DB，避免等待writer期间读到旧值。首次回归误用不写group policy的通用update_plugin，旧/新均失败，不能计为缺陷证据；改为实际专用事务并确认落库后，旧实现返回allowlist而非disabled失败，修复后通过。Robot prepare/activate顺序尚未解决，不把此项整体关闭。

R71沿用R64-03：复用已有AbortController，两个await后检查取消，避免旧refresh发布状态/派生下一请求；catch安全处理null等非Error拒绝。实际AuthProvider注入忽略abort的响应，四项生命周期回归旧失败；非Error回归旧7通过/1失败，最终32/0。未引入请求代际框架；login/setup/logout相互竞态、QR提前返回、持久密码策略/服务端cookie响应归属仍未解决。当前未重跑全UI；等待R69冻结后集成验证。

三批生产净增13行（R67 +5、R70 +5、R71 +3），测试净增68行（-14、+15、+67），总净增81行；无新依赖/框架。Rust均4线程，未跑全workspace或真实设备/其他OS。

## R68 / R69 / R72 / R73 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R68 Cron | `5212c24f6` | cargo test --locked -p nomifun-cron：260/0（189lib、3prompt、62service、6skill_file），1线程 |
| R69 MCP页面 | `65d0dfe7d` | 定向15/0、关联27/0；与R71一起全UI3398/0（617文件）、typecheck通过 |
| R72 System version/sysinfo | `cdc36483a` | 单元27/0，HTTP system_info_routes19/0，均4线程 |
| R73 Execution首批 | `20e8ef24a` | cargo test --locked -p nomifun-agent-execution --lib：88/0，4线程 |

R68完整读完16源文件+3集成测试、基线18737行；只读核对App装配/Session/shutdown与DB reservation/finalize/advance。修调度溢出、busy原子permit及取消释放、关闭后拒绝timer安装、技能建议持久化成功后才缓存hash；主线程补删全仓无生产调用的set_processing，确保permit是唯一写入途径。复用既有runtime fixture，删重复mock/自测构造和冗余正文副本。没有旧行为运行证据；取消不代表底层runtime已终止或撤销已执行副作用，跨文件/DB和detached检测任务仍待办。

R69完整读三文件，基线生产358/测试34行；只读追踪市场面板/CRUD/catalog/connection/OAuth/表单及后端导入。原生产实现跑同一套15项回归4通过/11失败，修复后通过。页面revision/ref阻止旧解析覆盖、重复确认与迟到导航，初次目录加载失败不开放编辑；切离市场卸载预览。共享CRUD迟到报错、跨重建防重、卸载后save Promise和后端批量非事务仍未修，不能把页面取消说成撤销导入。

R72完整读version/sysinfo/system_info_routes并追踪App/UI/发布调用：SemVer按precedence比较、预发布双标记过滤、平台关键词边界和签名排除；主线程补现有安装包扩展名/universal及release-lock排除。非法repo/当前版本先校验，分页按Link指示且上限5页，整次30秒deadline，不回显上游错误正文。新增安装包回归先两次夹具类型编译失败（不计行为证据），修正后确实因推荐为空失败；其余agent改动无旧运行证据。正文容量、重定向、实际签名/其他OS未验证；sysinfo目录与generation策略未改。

R73仅首批，不声明执行层全审。完整lib/manifest、production、lifecycle、event_publisher、两路由；scheduler前半至租约heartbeat/shutdown及现有harness、engine装配和取消/重规划/恢复等调用段已读。lead projection返回成功或错误却只在Ok(false)安排重试，复用现有worker并移除无效bool/ensure封装；两种cancel封装完全同义，删除无效step集合。真实SQLite+注入一次失败的会话效果端口回归先因空plan夹具失败，修正后旧实现明确未安排重试失败，最终88/0；验证相同operation重试、落delivered标记和不重复投递。未测真实模型/会话投影。

四批生产净增2行（R68 -28、R69 +35、R72 +36、R73 -41），测试净增288行（-234、+239、+129、+54），总净增290行。R68删除的cfg(test)辅助函数归入测试，不算生产删减。无新依赖/框架；回归增长和生产增减分别统计。

## 历史并行边界（R73 / R74 / R75 / R76）

| 批次 / 负责人 | 独占写集 | 任务 |
| --- | --- | --- |
| R76 / Mencius | nomifun-system/src/provider.rs及专用provider测试 | 接续provider配置/入口；R72验证结束已放写，其他System文件只读 |
| R73 / 主线程 | nomifun-agent-execution/src/**、crate tests | 先读装配/HTTP/lifecycle/event_publisher，跟踪调度和engine的cleanup/lead报告；完整engine/scheduler仍待续读 |
| R74 / Carver | ui/src/renderer/pages/cron/** | R69冻结后顺接Cron页面；全UI3398/0及typecheck后放行写入；共享hooks和后端只读 |
| R75 / Hypatia | nomifun-file/src/**、crate tests | R68冻结后顺接文件模块；R73验证结束已放写，其他crate只读 |

R68/R69/R72已冻结由主线程接管，worker不得回改；共享接口、App/DB/Auth、台账/Git均由主线程负责。R74在3398/0完整UI后放写，R75在88/0执行层后放写，Rust集中排队。

## R74 / R75 / R76 / R77 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R74 Cron弹窗 | `0ede75ac5` | Cron页面58/0（12文件），完整UI typecheck通过 |
| R75 File | `c392d49cb` | cargo test --locked -p nomifun-file：348/0，4线程 |
| R76 Provider | `e528829` | System provider单元5/0、provider_routes14/0，4线程 |
| R77 Execution控制步骤 | `1732186a2` | cargo test --locked -p nomifun-agent-execution --lib：93/0，4线程 |

R74完整阅读Cron页面32文件、基线4491行（生产3660/测试831）；只读跟踪共享adapter/会话调用，未改共享接口。未修改调度时省略schedule补丁，保留at/every/时区；复杂cron不再降成预设；同ID实时更新不覆盖草稿，提交锁/会话归属拦重复与迟到提示。真实组件同一夹具旧5通过/11失败，修后16/0；主线程Cron58/0及完整typecheck。首次主线程Bun目录参数遗漏未找到目标，不计验证；修正--cwd ui后通过。列表/详情/表达式编辑/时区repair等未闭环见R74-05。

R75完整阅读20个Rust文件、基线11901行。现有目标解析也检查最终symlink；复制先验证最近存在父路径，创建目录复用单组件校验；snapshot删除检查相对路径和父目录；patch临时文件仅create_new成功后才有清理权。远程图片分块遵守已有体积限制并逐跳校验URL，ZIP分块读取/取消、同ID拒绝覆盖与按flag身份清理，watch stop与start共用锁；删除重复事件构造、无效collect和只断言常量的测试。无旧实现运行红→绿；Windows348/0，三个Unix链接回归未运行。取消不等于blocking结束；输出路径排他/原子ZIP、hardlink、snapshot pathspec和路径TOCTOU仍未闭环，不宣称文件模块全完成。

R76完整阅读provider.rs及provider_routes；相邻DB/协议/App/UI仅调用追踪。create从校验到存储统一trim平台；Bedrock更新保存已校验的规范值，等价配置不清health/增revision；删除单层URL转发函数。三种认证方式的真实SQLite回归覆盖等价/省略/真实region变化；无旧运行证据，未访问真实AWS/凭据。创建后display-name写入、删除前软引用清理跨事务等R76-04保留。

R77完整补读control_steps、plan_materializer、domain_mapper、participant_router、attempt_runner、delivery、conversation_effect；participant_resolver读至prepend_frozen_snapshot，artifact_contract仅零散段不计读完；engine/scheduler仍按R73未读范围继续。三个定向旧失败：usize::MAX选票索引溢出、Borda无分数仍成功、bypass误判PASS；局部修复后通过。候选1024硬限同时用于计划入口、选票解析和持久状态执行；不把页数或JSON解析当正文容量界限。另修循环计数与quiet_rounds溢出，新增极值回归但无旧运行证据。93/0在File冻结后运行；首次旧行为测试曾编译尚未完成交接的File，不作为File验证证据。

四批生产净增143行（R74 +36、R75 +99、R76 -3、R77 +11），测试净增536行（+142、+181、+154、+59），总净增679行。无新依赖、通用框架或新后台服务；本批不是总代码量下降。只为已确认错误补局部检查和回归；未运行全Rust workspace/真实服务/其他OS。

## R78 / R79 / R80 / R81 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R78 Cron列表 | `938d93132` | 主线程Cron目录97/0（13文件）、完整UI typecheck通过 |
| R79 Provider model | `616495a97` | cargo test --locked -p nomifun-system --lib --test provider_model_routes --test provider_routes：166+7+14=187/0，4线程 |
| R80 Snapshot单文件 | `e9e525561` | cargo test --locked -p nomifun-file --test snapshot：50/0，4线程 |
| R81 Execution回执/解析 | `98149318b` | cargo test --locked -p nomifun-agent-execution --lib：95/0，4线程 |

R78完整读原hook521行；三个列表和run history以当前请求所有权管理loading/错误/结果，GET期间按ID合并事件与本地成功操作，修复前后检查时区修复归属。39项新增行为回归中首34项旧2通过/32失败，后5项无旧运行证据。未撤销已发PUT、未解决服务端事件/写响应全局排序；共享时区修复、详情和SessionList仍在R74-05。

R79完整读provider_model及专属routes，调用追踪DB/DTO/UI/Workshop/ModelInvoke不算相邻模块全审。Ark型号校验与协议平台匹配统一忽略大小写，WebSocket根URL复用已有解析并保留query禁令；真实SQLite验证拒绝不落库/有效型号能解析。无旧运行证据；保存/删除长度不一致登记R79-03，其他跨事务问题沿用R76-04。

R80字面单文件stage/unstage/discard/reset不再扩大到glob邻居；直接恢复HEAD index条目保留mode和邻居冲突，checkout禁pathspec且拒绝缺失/目录，reset不再吞index/HEAD错误，stage区分NotFound与其他metadata错误。主线程snapshot两次49通过/1失败：get_path和find_prefix均不能以不同大小写取得HEAD条目；核对libgit2源码后按core.ignorecase字面回退，最终50/0。Unix特殊文件名和新增链接用例未运行；没有把本批失败称为全部旧实现红绿。未重跑整个File，R75-08和取消/路径并发风险保留。

R81合并立即/轮询完成回执，产物验证失败携带明确错误和retryable=false，保留真实provider失败；共享借用JSON扫描器替代会删除字符串内代码围栏的重复扫描；删除无效标签参数和单层转发，快照替换后参与者连续排序。新增两个生产映射/解析测试并增强已有排序/能力回归，无旧运行证据；未模拟真实会话投递及外部副作用。补读planner、resolver、artifact_contract全文件，scheduler生产1146–3028已续读；engine本轮补读243–804，其余与既有R73范围合并后继续，不声明全Execution完成。

四批生产净增48行（R78 +66、R79 -5、R80 +48、R81 -61），测试净增579行（+275、+74、+186、+44），总净增627行。无新依赖/公共框架；未跑全Rust workspace、真实模型/外网、其他OS；仅相关UI范围完整typecheck。

## R85 / R86 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R85 模型删除跨入口 | `76b8b9021` | System provider_model_routes7/0；Workshop --lib model_cleanup3/0，均cargo test --locked、4线程 |
| R86 Execution效果ack去重 | `0903461ab` | cargo test --locked -p nomifun-agent-execution --lib：96/0，4线程 |

R85沿用R79-03，不重复编号。DTO/DB允许长自然键，System和Workshop删除却独有512字符限制；只删除两处上限，保留trim/非空。增强已有CRUD roundtrip为长键并保留原断言，Workshop沿用实际cleanup planner；旧实现分别HTTP400和BadRequest失败，修复后通过。App删除协调器只读确认原样转发，未跑App运行时或真实模型。

R86 StopTurn/Steer交付后返回operation/effect，再共享状态编码、repository ack和事件发布；DecisionInput按原settlement返回。无新公共抽象。沿用SQLite scheduler harness与注入端口，真实启动attempt后测试首次stop失败不ack、同ID重试、stop/steer事件及pending清空。非缺陷修复不声称旧实现红绿；未模拟外部副作用、decision-input或lease故障。

两批生产净减25行（R85 0、R86 -25），测试净增77行（+15、+62），总净增52行。无新依赖/框架。engine本轮续读243–1207、1568–2081，之前1208–1567已读不重复；2082以后和部分scheduler测试仍需继续。全项目仍未完成。

## R82 / R83 / R87 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R82 v4-root | `39515819b` | cargo test --locked -p nomifun-v4-root --lib：14/0，4线程 |
| R83 Requirements UI | `31fa8aa0c` | 需求页目录59/0（15文件）、完整UI typecheck通过 |
| R87 Execution历史隔离 | `7161d651f` | cargo test --locked -p nomifun-agent-execution --lib：98/0，4线程 |

R82完整阅读8个Rust文件、原3132行；恢复先拒绝被换成链接的root，目录迭代错误不再当非空；缺失package直接报错，runtime capability/skill同时检查SQL归属列和JSON归属。真实SQLite包含有效runtime重启对照及满足FK的错误归属，Windows14/0；无旧实现运行证据，Unix链接回归未运行。App先bootstrap后取server锁、DB取消/关闭、marker残片/热journal、build digest升级契约和路径TOCTOU仍保留；尚未跑App调用方测试，避免混入R84/R88活动写集。

R83原38文件全部阅读（生产3969/测试483行）。修列表/标签请求代际与卸载归属、看板遵守服务端200上限并按has_more翻页、附件并发合并及上传计数、状态菜单和拖放复用同一合法流转判断。主线程只读核对后端set_status和分页契约；agent报告27项新增回归旧失败后通过，主线程冻结快照59/0和完整typecheck。没有将offset分页称为一致性快照，抽屉/通知/AutoWork/批量动作与后端部分写入仍待后续。

R87确认adjust直接接收含历史的detail；pick_lead跳过retired，调整prompt跳过superseded步骤和依赖，保留有效记录。两项旧实现实际失败，修后98/0；复用现有active_participants和actor_event，未新增框架。engine2082至末尾、scheduler3018–3656和4089至末尾已补读；结合之前范围，未将已读问题标为闭环。规划timeout提前返回尚未发done事件列入R73-03，需结合消费方验证，不盲改。

三批生产净增106行（R82 +17、R83 +99、R87 -10），测试净增534行（+215、+263、+56），总净增640行。测试/生产分开，不宣称总体减量；没有新增依赖/服务或通用框架，未跑全Rust workspace或真实服务。

## 当前并行边界（R84 / R88 / R89）

| 批次 / 负责人 | 独占写集 | 任务 |
| --- | --- | --- |
| R84 / Hypatia | nomifun-chat-model-broker/src/**及tests/** | 保持原独占任务，完成即冻结交接 |
| R88 / Mencius | nomifun-terminal/src/**及crate tests | 完整审计终端生命周期/进程与权限边界；R87结束后放写 |
| R89 / Carver | ui/src/renderer/pages/requirements/**剩余范围 | R83提交/typecheck后接续抽屉/通知/AutoWork，不重复已闭环行为 |
| 主线程 | 未分配模块、共享边界、台账/Git | 验收后逐批推送，Execution剩余跨边界问题保留；Rust集中串行 |

R82/R83/R87已冻结提交。Terminal在Execution/App依赖链，R88冻结前不编译该链；root/broker不依赖terminal。Browser Use仍跳过。

## 历史并行边界（R82 / R83 / R84）

| 批次 / 负责人 | 独占写集 | 任务 |
| --- | --- | --- |
| R82 / Mencius | nomifun-v4-root/src/**及crate tests | 根目录初始化/恢复/持久状态，先完整阅读再局部修复；不在当前Rust验证依赖链，已放写 |
| R83 / Carver | ui/src/renderer/pages/requirements/** | 需求页面入口/状态/CRUD/错误生命周期；R78 typecheck后放写 |
| R84 / Hypatia | nomifun-chat-model-broker/src/**及tests/** | 模型代理协议/请求/响应生命周期，只读追踪App/平台；不在当前验证依赖链，已放写 |
| R86及后续 / 主线程 | nomifun-agent-execution/src/**及crate tests；共享边界/台账/Git | R85已关闭R79-03，R86合并持久效果ack；继续engine剩余范围，Rust串行验收，不修改R82/R83/R84写集 |

R78–R81均已冻结提交，worker不回改冻结范围，不修改共享依赖/台账/Git；Browser Use暂跳过。

## 历史并行边界（R78 / R79 / R80）

| 批次 / 负责人 | 独占写集 | 任务 |
| --- | --- | --- |
| R78 / Carver | ui/src/renderer/pages/cron/useCronJobs.ts及同目录专属测试 | 身份/重叠GET/事件快照竞态和loading；R74复验后放写 |
| R79 / Mencius | nomifun-system/src/provider_model.rs及tests/provider_model_routes.rs | 模型CRUD/输入/冗余，R76验证后放写；旧provider只读 |
| R80 / Hypatia | nomifun-file/src/snapshot_service/helpers.rs及tests/snapshot.rs | 单文件pathspec/错误归属；R75及Execution/System验证后放写 |
| 主线程 | Execution未交给worker的src/tests、共享接口、台账/Git | 继续回执/产物错误归属与剩余范围；Rust集中验证 |

R74/R75/R76/R77已提交；worker不回改旧冻结文件，R80只接管明确的snapshot两文件。其他目录只读，跨边界修改先交主线程。Browser Use暂跳过。

## R56 / R63 / R65 阶段验收

| 批次 | 已提交源码 | 验证与边界 |
| --- | --- | --- |
| R56 UI hooks/config | `2c92be0f9` | config/hooks/i18n/语音配置定向68/0；Login新测试加入前typecheck通过；本检查点config/hooks+会话适配器72/0 |
| 会话恢复测试翻译断言 | `04d31d7a9` | 保留完整事件序列，期望文案遵守i18n初始化状态；单文件15/0，未改产品代码 |
| R63 导入目录归属 | `a124c1a03` | Knowledge export13/0、Companion export26/0；16并发导入旧实现丢文件先失败，取消回归保留目录至blocking完成 |
| R65 Shell命令路径 | `c38aa2019` | cargo test --locked -p nomifun-shell -- --test-threads=4：96/0；真实隐藏cmd特殊字符cwd通过，未开GUI窗口 |

R56新增回归先有14项失败；主线程另复现离线启动后reload空配置仍保留旧主题，修复replaceCache对已订阅缺省值的就绪通知后通过。全量UI第一次3363通过/7失败/1错误：6项来自尚在编辑的Login夹具，另1项来自翻译状态断言；该次不记通过。Login冻结后须重跑完整UI/typecheck。

R63以已有tempfile独占目录+Arc持有替代PID/毫秒共享目录和手动删除，空.import-tmp根保留以避免与其他导入创建竞态；不保证取消后业务DB/文件事务回滚、不声称提前中止blocking或崩溃清理。R65仅扩展已有opener的cwd参数；hand_off生命周期未修。

本检查点生产净减5行（R56 -3、R63 -3、R65 +1），测试净增579行（484+57+32+翻译断言6），总净增574行；无新包/框架。测试增加如实单列，不冒充总代码量减少。

## 历史并行边界（R61 / R62 / R64）

| 批次 / 负责人 | 独占写集 | 任务 |
| --- | --- | --- |
| R61 / Mencius | nomifun-agent-domain-wave4/src/**、crate tests | identity/channel/device声明与handler/资源/生命周期 |
| R62 / Hypatia | nomifun-agent-domain-wave5/src/**、crate tests | automation/supervision/Remote声明与权限/生命周期 |
| R64 / Carver | ui/src/renderer/pages/login/** | Login表单提交/生命周期/重定向及专用测试 |

R56已冻结转主线程；上述worker不得回改旧写集。共享接口、台账、Git均由主线程负责；Rust验证统一排队。

## R57 / R58 / R59 / R60 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R57 Wave1 | `0eae3feba` | cargo test --locked -p nomifun-agent-domain-wave1 --lib --tests：8/0 |
| R58 Wave3 | `679024b0e` | cargo test --locked -p nomifun-agent-domain-wave3：10/0 |
| R59 ZIP budget | `3fcbface2` | Common/Skills --lib zip_safe::：11/0+5/0；Companion/Knowledge --lib export::：26/0+9/0；Workshop --lib archive::：12/0 |
| R60 Knowledge export | `e310c3acd` | cargo test --locked -p nomifun-knowledge --lib export::：11/0 |

以上均4线程；最后有效结果合计83项（R60的11项替代R59知识库9项，不重复累计）。R59两处实际ZIP在旧实现写出超过4KiB后失败，修复后通过；R60固定临时文件被覆盖/源目录遍历失败却发布两回归旧失败后通过。R57/R58无修复前行为证据。App调用方已只读追踪，但本检查点未重跑App测试；另两个Wave正在编辑，避免把未冻结代码混入验证。未跑全Rust workspace/UI、其他OS或真实生成/外部服务。

四批合计+301/-160，净增141行：生产+106/-125（净减19），测试+195/-35（净增160）。分别为R57生产-37/测试+62、R58 -8/+6、R59 +33/+68、R60 -7/+24。R60将已有tempfile从dev-dependency移至生产依赖；未新增包、版本、锁文件或框架。只修改已确认的问题，不因缺少产品owner而自动扩建能力。

## 历史并行边界（R56 / R61 / R62）

| 批次 / 负责人 | 独占写集 | 任务 |
| --- | --- | --- |
| R56 / Carver | useTheme/useColorScheme/useFontScale及专用测试 | 继续验证回滚/重载/DOM；离线后reload空对象的初始化状态通知需主线程协调configService |
| R61 / Mencius | nomifun-agent-domain-wave4/src/**、crate tests | 身份/channel/device声明与资源/handler/生命周期 |
| R62 / Hypatia | nomifun-agent-domain-wave5/src/**、crate tests | automation/supervision/Remote的声明与权限/生命周期 |

R57/R58写集冻结转交主线程；agent不得回改。跨模块接口、configService、台账和提交统一由主线程处理；其余目录只读。R59/R60无未提交源码残留。Browser Use仍跳过。

## R51 / R53 / R54 / R55 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R51 UI config | `c5566cb3d` | configService/i18n/textToSpeechConfig 定向 Bun 44/0；`bun run typecheck` 通过 |
| R53 Realtime | `368658fb7` | `cargo test --locked -p nomifun-realtime --lib --tests -- --test-threads=4`：102/0 |
| R54 Skill Library | `03f2408b9` | `cargo test --locked -p nomifun-skill-library -- --test-threads=4`：160/0、2 真实外网用例 ignored |
| R55 Runtime resolver | `aaf16d70e` | Runtime 全包 38/0（cargo test --locked，4 线程） |

R51 新测试前 30 项对原逻辑加请求注入缝运行：4 通过、26 失败；修复后通过，再补 3 项边界，主线程合并调用方定向 44/0。R55 隔离子进程先失败于目录查询未从首次失败恢复，统一成功缓存后通过；不修改测试进程全局环境。R53/R54 仅有修复后回归证据。没有运行 UI 全套、全 Rust workspace、真实发布包或其他操作系统。

本检查点源码/测试净增 652 行：生产 R51 +46、R53 +24、R54 +66、R55 -8，合计 +128；测试分别 +323、+137、+34、+30，合计 +524。不把测试增长算作生产简化；不新增依赖、框架或通用后台任务。R54 清理全局失败覆盖、serial/环境变更和临时目录泄漏测试；复用真实文件树验证保护行为。

模块完整阅读不等于跨模块问题闭环：R51 服务端并发写顺序、R53 App shutdown/认证、R54 TOCTOU/原子导入及调用契约、R49-05 构建和平台遗留仍按索引保留。

## 历史并行边界（R56–R58）

| 批次 / 负责人 | 独占写集 | 任务 |
| --- | --- | --- |
| R56 / Carver | hooks/system/useTheme.ts、hooks/ui/useColorScheme.ts、useFontScale.ts 及同目录专用测试 | 配置回滚/重载与 DOM 一致性；configService 冻结只读 |
| R57 / Mencius | nomifun-agent-domain-wave1/src/**、crate tests | 注册、handler/context/resource 的实际入口与生命周期 |
| R58 / Hypatia | nomifun-agent-domain-wave3/src/**、crate tests | 创作/多模态/Office/MiniApp 声明、输入和资源授权 |

原 R51/R53/R54 写集已冻结由主线程接管。Contracts/Kernel/App 和跨模块接口仅主线程可修改；各 worker 不回改旧目录，不改依赖/锁/台账、不提交。Rust 验证主线程排队，UI 可独立运行定向验证；Browser Use 暂跳过。

## R45 / R49 / R50 / R52 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R45 Public | `0da0f03` | `cargo test --locked -p nomifun-public --lib -- --test-threads=4`：29/0 |
| R49 Runtime | `2e42d8fa8` | `cargo test --locked -p nomifun-runtime -p nomifun-shell -- --test-threads=4` 中 Runtime 37/0 |
| R50 Shell | `f3ceb978e` | 同上 Shell 94/0 |
| R52 Webhook/DB | `fc4299cf3` | `-p nomifun-db --test webhook_repo` 6/0；`-p nomifun-webhook` 25/0（均 cargo test --locked、4 线程） |

本检查点 191 项定向 Rust 测试通过。R52 两项真实数据库并发回归先失败（名称/标签描述被覆盖），原子 SQL 后通过；随后全 Webhook 编译发现 notifier 测试尚有一处旧 upsert 调用，迁移后全包重新通过。未以这次调用迁移编译错误充当行为红→绿证据。其他 R45/R49/R50 新回归未在修复前执行。

四批源码/测试共 +928/-669，净增 259 行：生产 +300/-304（净减 4），测试 +628/-365（净增 263），不含台账。Runtime 自身净减 106 行，R52 生产净减 20 行；Public 新增 9 项回归是本检查点测试增长的主要来源。没有新增依赖/框架/后台服务。R52 仅两个明确的字段补丁参数替代整行写接口，无数据库迁移、全局锁或重试框架。

R49 删除的 `tests/extract_integration.rs` 只测试 zstd/tempfile、未调用 runtime；真实 extract_into 内容/校验/别名/失败恢复回归保留。另清理环境写入/泄漏和无断言测试，Git 历史可恢复删除文件。人工 shell 回归仅检查参数/合成 PATH，不替代真实登录 rc；Windows 上未执行 Unix 链接/macOS 探针及 VS Code fallback，未验证真实发布包/30 秒超时/外部 MCP/原生应用启动。

剩余 Public R45-04、Runtime R49-05、Shell R50-04 和 Webhook R46-05 都保留，模块不因读完或测试通过而标记全完成。本次按明确白名单提交，R51/R53/R54 未完成写集不混入；Browser Use 未改。

## 历史并行边界（R51–R54）

| 批次 / 负责人 | 独占写集 | 任务 |
| --- | --- | --- |
| R51 / 原 Public agent | `ui/src/common/config/**` | 配置初始化/重载及写入竞态、订阅失败隔离；其他 UI 和后端只读 |
| R52 / 主线程 | Webhook service/tests；DB webhook/tag_setting models、repository 和相应测试 | 接续 R46-05：数据库原子字段更新与调用迁移；无 schema/依赖变更 |
| R53 / 原 Runtime agent | `crates/backend/nomifun-realtime/src/**`、tests | WebSocket 身份、广播背压、任务及关闭 |
| R54 / 原 Shell agent | `crates/backend/nomifun-skill-library/src/**`、tests | 导入导出/路径、加载、并发写入和错误归属 |

原 R45/R49/R50 写集已冻结转交主线程；worker 不回改原目录。共享接口由主线程统一处理，其他 agent 只读；Rust 测试集中排队，UI agent 只跑定向测试。此表不代表审计完成。

## 历史并行边界（R44–R47）

| 批次 / 负责人 | 独占写集 | 任务 |
| --- | --- | --- |
| R44 / Auth agent | `crates/backend/nomifun-auth/src/**`、本 crate tests | 接续 R42-04，审剩余入口/路由/信任/令牌；不改已验证契约 |
| R45 / Public agent | `crates/backend/nomifun-public/src/**`、本 crate tests | Remote MCP 入口/权限/会话生命周期 |
| R46 / Webhook agent | `crates/backend/nomifun-webhook/src/**`、本 crate tests | 发送/CRUD/通知完整模块审计 |
| R47 / 主线程 | `crates/agent/nomi-agent/src/skill_tool.rs`、必要 Agent 回归；必要时 nomi-skills 执行契约 | 接续 R38-03，inline shell 与完成证据副作用边界 |

依赖清单、共享接口和台账只由主线程编辑；worker 不跑 Cargo、不提交/推送。Auth/Public 仅共享只读契约，发现需要改变契约时先停止该项写入并交主线程，不并发改逻辑边界。

R47 定向 147/0 后，主线程顺接 R48（R30-02）：独占 `nomi-cli/src/main.rs`、`command_audit_tests.rs` 和 `nomi-agent/src/output/protocol_sink.rs`，统一 CLI 输出失败入口；其余 worker 无写集交叉。

R44/R46 写入冻结交主线程后，原 worker 顺接 R49（`nomifun-runtime/src/**`、tests、build.rs/build_support.rs）和 R50（`nomifun-shell/src/**`、tests）；不再编辑 Auth/Webhook。R45 worker 仅补完同写集内令牌补充间隔丢失，主线程暂不编辑该 crate。

## R44 / R46 / R47 / R48 阶段验收

| 批次 | 已提交源码 | 最终验证 |
| --- | --- | --- |
| R44 Auth | `31330e1c1` | `cargo test --locked -p nomifun-auth -p nomifun-webhook -- --test-threads=4` 中 Auth 251/0 |
| R46 Webhook | `072ccfda6` | 同上 Webhook 23/0；`-p nomifun-requirement --test notify` 3/0 |
| R47 Skills→Agent | `5f816fb51` | `cargo test --locked -p nomi-skills -p nomi-agent --lib -- skill_tool shell:: completion_evidence_tests --test-threads=4`：Agent 94/0、Skills 53/0 |
| R48 CLI 输出 | `7642f4ac8` | `cargo test --locked -p nomi-cli --bin nomi -- --test-threads=4`：17/0 |

本检查点 441 项定向 Rust 测试通过；未改 UI，不重复 UI 全量/构建。R47 两项回归先失败后通过；R48 Ready 回归先修正夹具类型推断错误后，确认为启动 reader 断言失败，再修复通过。其他新回归没有修复前运行证据。R48 复用同一失败 emitter、容量 1 的错误通知及既有 shutdown，不改 OutputSink trait；实测流式 TextDelta、Pong、空闲 ConfigChanged、同次轮询完成的 Info 和 Ready 失败，均检查清理/禁止虚假成功。

四批源码/测试共 +647/-130，净增 517 行；生产代码 +155/-53（净增 102），测试 +492/-77（净增 415），不含台账。增长来自已确认错误的局部处理和回归，没有新增依赖或后台框架；QR 只是接入既有清理入口。没有以减少行数为理由删除功能，也不宣称本检查点实现全项目净减。

未验证真实飞书/Slack、30 秒超时到期、OS stdout 管道断开、真实远程 MCP 或其他操作系统。R45/R49/R50 独占写集仍在审计/集成，本次提交只包含上表文件，不混入其未验证改动。当前无 Browser Use 改动。

## 本轮并行边界（R40–R43）

| 批次 / 负责人 | 独占写入范围 | 本轮目标 |
| --- | --- | --- |
| R40 / Skills agent | `crates/agent/nomi-skills/src/**` | 接续 R38-03，保持对外 API 不变 |
| R41 / CLI agent | `crates/agent/nomi-cli/src/**` | 接续 R30-02，只修 crate 内可闭环问题 |
| R42 / Auth agent | auth 的 cookie/csrf/jwt/password/rate_limit 五个源文件与 `tests/core_audit.rs` | 核对认证核心，其他入口只读 |
| R43 / 主线程 | `ui/src/platform/**`、`ui/src/common/update/updateTypes.ts`、`ui/src/common/adapter/ipcBridge.ts`、`ui/src/renderer/components/settings/UpdateModal.tsx` 及必要调用测试 | 平台桥接及失效手动更新分支清理 |
| R43 / 测试 agent | `GuidAgentPresetLaunch.structure.test.ts`、`miniAppsNav.structure.test.ts` | 只修确认与 HEAD 产品逻辑脱节的断言；生产代码只读 |

共享接口、依赖清单、锁文件和本台账仅由主线程处理；worker 不提交、不推送、不运行 Cargo，Rust 验证集中排队。发现跨写集问题先交主线程判断，不抢改。此表是任务分配，不代表完成审计。

## R40–R43 并行批次验收（2026-09-12）

- 基线 `2e5834c6e1`，20 个变更文件（19 个源码/测试 + 本台账）；源码已提交为 `2594cb595`（Skills）、`dea4e445f`（CLI）、`e58b04442`（Auth）、`c8b935071`（UI），本记录随后提交并统一推送。三个 Rust agent 与一个独立测试 agent 已结束；主线程逐份复核补丁和跨模块调用，依赖清单/锁文件未变，无并发编辑同一文件。
- R40：补完 frontmatter/hooks/prompt/integration 的剩余测试阅读，修复 YAML 回退和嵌套 brace；删去重复和自我实现逻辑的测试。参数替换生成 shell 是既有行为，真实临时文件测试确认 MCP 不执行它；并未改变执行/审批策略。
- R41：接续同一 R30-02，而非新增重复问题编号。session 初始化失败、CLI-owned 协议写失败、结束输出失败统一进入既有 shutdown；优先保留原错误并附加清理失败上下文。空 MCP manager 也显式清理，失败时保留到最后重试。没有修改共享 OutputSink 或完成证据框架。
- R42：吊销令牌保留到验证宽限期结束；限流使用单调时钟，合并窗口重置，避免溢出及后台强引用；密码生成拒绝采样并去掉 shuffle 临时数组；CSRF 复用相同 Bearer 提取规则。认证剩余路由/信任/兼容策略记为 R42-04，未将其标成安全审计完成。
- R43：平台 invoke 同步发送失败解绑一次性响应监听；移除无调用 subscribe 和颜色 token；沿更新生产调用链删除失效手动下载实现、状态和专用类型，保留 native updater 与手动外链。这里只删除 UI 死路径，后端 version.rs 的 release asset API 仍有独立生产用途，未一并删除。
- 集成修复：第一次 UI 全量为 3302/2；两项失败对应 HEAD 中早已合入的模型引用与 MiniApp 页面路由变化。独立测试 agent 只改两份结构测试，核对 commit 祖先和生产逻辑后更新精准断言；未修改 MiniApp 产品实现。

| 最终验证 | 结果 |
| --- | --- |
| `cargo test --locked -p nomi-skills -p nomifun-auth -- --test-threads=4` | Skills 384/0；Auth 161 单元 + 85 集成 = 246/0 |
| `cargo test --locked -p nomi-cli --bin nomi -- --test-threads=4` | 15/0 |
| `cargo test --locked -p nomi-agent skill_tool -- --test-threads=4` | 59 单元 + 1 plan-mode 集成 = 60/0 |
| `bun test --cwd ui`（全部 UI 修改后） | 3304/0，611 文件 |
| `bun run check`、`bun run build:ui`（更新死路径删除后） | 通过；既有大 chunk 警告仍列 R2-04 |
| 清单与差异检查 | 111 模块/100 唯一问题；`git diff --check` 通过 |

桥接新回归有修复前失败证据，其余本批 Rust 回归只记录修改后通过，不倒推红→绿。未运行全 Rust workspace、真实 broken stdout/外部 MCP、原生其他操作系统或桌面发布/真实升级；测试不等于这些集成已验证。

本批源码/测试（含新增 bridge.test.ts，不含文档）为 +844/-692，净增 152 行：生产代码 +385/-481，净减 96 行；测试 +459/-211，净增 248 行。Rust 行内测试按测试区段归类，CLI 大 diff 包含命令循环缩进，不作为新增功能计数。Skills 净减 133 行，UI 源码与测试合计净减 56 行；新增量主要是 CLI 异常清理及认证回归，没有新增依赖/通用框架。全项目仍未完成 code review，不承诺“无冗余/无缺陷”。

## 阶段性提交验收（2026-09-12）

- 用户已授权按模块提交。本轮只做迁移确认、现有变更复验、分组提交和记录收尾；没有继续扩大重构，也没有推送。基线为 `41bfea6723cec3d8c5e7a1ad278edd4909959051`，分支为 `rf/agent-capability-platform-v2`。
- MiniApp 产品实现已隔离到 `C:/Users/rika0/code/nomifun/miniapps-product`，分支 `codex/miniapps-product`；该处仍在开发，本轮未修改。当前工作区无对应实现差异；`MiniAppSurfacePanel.tsx` 的规范化内容与 HEAD 相同，未纳入提交。
- 本地 `docs/specs/2026-09-12-miniapps-product-redesign-review.zh.md` 与另一 worktree 的同名文档内容不同；保留为未跟踪文件，不删除、不混入审计提交。
- 使用贡献者已配置的 Git 身份。每次只暂存明确文件白名单，检查暂存差异后提交；以单次命令的 `core.hooksPath` 指向新建空临时目录，未改变持久配置，未执行既有 `.githooks/`。
- MiniApp 迁移后，锁文件遗留 App 对 `zip 2.4.2` 的引用，首次 `--locked` 因需更新锁文件停止。改用 `--offline` 同步后，相对本轮开始仅删除该条失效引用，没有升级依赖；其他既有锁变化按对应模块提交。

### 提交明细

| 提交 | 范围 |
| --- | --- |
| `914008ff06` | 进程清理凭据 |
| `8d31fe5bfe` | 共享类型与工具输出压缩 |
| `ee0f226a69` | 配置合并与 Hook |
| `30c9767e31` | MCP HTTP/SSE |
| `54b420debb` | 记忆存储 |
| `2d3860d574` | 技能遍历与参数替换 |
| `bd19cfb482` | CLI 协议与生命周期 |
| `a4a3f8b1a8` | 网络出站与脱敏 |
| `bc122ed8db` | SSH/SFTP 与持久 shell |
| `b4ccc373f1` | 数据库分页 |
| `f55133e512` | UI 旧实现清理与状态归属 |
| `b0ee370437` | 未配置能力处理器 |
| `8e83adfa6a` | Host/Mount 运行时生命周期 |
| `620cccaada` | 静态资产与缓存 |

以上 14 批覆盖 189 个源码、测试、依赖文件，合计 +10434/-6676，净增 3758 行，不含本记录及清单脚本。其中 UI 净减 2132 行，skills 净减 583 行，domain-support 净减 100 行；Host/SSH 回归及边界实现增加了总量，不能宣称全项目代码净减少。共删除 25 个旧文件，已提交到 Git，可恢复。现有 33 份审计 Markdown 和清单脚本单独作为第 15 批记录提交，不另建本轮报告。

### 本轮重新执行的验证

- `bun test --cwd ui`：3304 通过、0 失败，609 文件；`bun run check`、`bun run build:ui` 均通过。构建仍有既有大 chunk 警告，R2-04 保留。
- 下列包级回归共 1362 通过、0 失败、1 忽略；忽略项为既有网络子进程夹具：

```text
cargo test --offline -p nomi-types -p nomi-compact -p nomi-config -p nomi-protocol -p nomi-skills -p nomi-memory -p nomi-mcp -p nomi-cli -p nomi-redact -p nomifun-net -p nomi-ssh -p nomifun-agent-domain-support -p nomifun-agent-session -p nomifun-agent-kernel -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-assets -- --test-threads=4
```

锁文件同步后，以下定向回归均使用 `cargo test --locked`，末尾均带 `--test-threads=4`：

| 包与目标/过滤参数（接在命令前缀后） | 通过 / 失败 |
| --- | --- |
| `-p nomi-process-runtime --test child_process_builder --test architecture_contract --` | 23 / 0 |
| `-p nomifun-db --test conversation_repository --` | 91 / 0 |
| `-p nomifun-db --lib -- repository::sqlite_conversation::tests repository::sqlite_requirement::tests repository::sqlite_workshop::tests` | 112 / 0 |
| `-p nomifun-ssh --lib --` | 30 / 0 |
| `-p nomifun-app --lib plugin_runtime_host --` | 3 / 0 |
| `-p nomifun-app --test content_e2e_suite assets_e2e --` | 2 / 0 |
| `-p nomifun-ai-agent -p nomifun-model-invoke --lib -- send_error error::tests` | 49 / 0 |
| `-p nomi-agent --lib -- skill_tool memory` | 69 / 0 |
| `-p nomi-agent --test memory_context_integration --` | 7 / 0 |
| `-p nomi-agent --test tool_execution_test hook --` | 2 / 0 |
| `-p nomi-providers --lib -- schema deferred` | 8 / 0 |

- Rust 本轮合计 1758 通过、0 失败、1 忽略。资产最初误用 `--test assets_e2e`，Cargo 报目标未注册；查明它属于 `content_e2e_suite` 后按上表通过。这是命令选择错误，不是测试断言或编译失败。
- `bun scripts/check-review-inventory.mjs`：111 个模块边界、90 个唯一问题编号，无缺项或重复；`git diff --check`、代码提交范围的 `git diff --check <基线> HEAD`、各组 `git diff --cached --check` 均通过。检查只覆盖本轮文件，不检查既有 `.githooks/` 内容。
- 未跑全 Rust workspace、桌面发布包、原生 macOS/Linux、真实 sshd/外部 MCP/模型服务；相关平台与集成限制仍按各历史报告保留。本轮未重跑 model-invoke 全量，R20-02 默认高并发热点仍待审。
- 交付边界：本轮审计源码与记录分组提交；上述 MiniApp 设计文档及既有排除项留在原处。提交不会改变模块覆盖状态，也不代表已消除所有设计缺陷或冗余。

## 状态与防遗漏规则

- 2026-09-12 用户补充：以全局覆盖和简单、局部的问题解决为优先；不得为未复现的推测引入新框架、通用抽象或后台任务。模块没有明确问题时记录已核对范围，不为了产生改动而重构。

- `待审`：只枚举或扫描过（包括仅运行过测试）。
- `审计中`：正在检查，续接位置必须具体到文件/行为。
- `部分完成`：已审子范围写在行内；其余仍待审。
- `已审计`：模块内约定范围已检查，尚有验证未完成；`已验证`：该范围修复和验证均完成。
- 每个问题只有一个编号；跨模块问题登记一次，在相关模块引用同一编号。每次修改后记录验证命令/结果；不沿用修改前的通过结果。
- 大模块须在审到时细分文件/行为；目录一行是覆盖索引，不代表只检查一个文件就完成整个模块。
- 开始新批次先读本文件和上一批记录，检查 Git 状态，运行 `bun scripts/check-review-inventory.mjs`；只接续待办，不重做已有证据的任务。
- 清单脚本按实际 Cargo crate、UI 公共目录、renderer 公共目录和页面目录核对登记；缺项、重复、过期目录均报错。它不评估代码正确性，不意味着逐行覆盖。
- 清单脚本同时检查问题编号重复；跨模块引用写在说明列，不新增同编号的问题行。新增问题使用新编号，后续复现沿用已有编号。

## 问题与任务索引

| 编号 | 状态 | 唯一问题/任务 | 证据或后续动作 |
| --- | --- | --- | --- |
| R1-01 | 已验证 | Kernel 批量回收首错中断 | registry_resource_tests；4 个 Rust crate 共 104 测试通过 |
| R1-02 | 已验证 | Kernel 非法句柄拒绝时泄漏、direct/Role 重复校验 | 同上；共用保留与回收实现 |
| R1-03 | 已验证 | Session observation 多版本快照 | 并发回归修复前失败、修复后通过 |
| R1-04 | 已验证 | 模型串行写入队列被回调拒绝污染 | 2 个回归修复前失败、修复后通过 |
| R1-05 | 已验证 | Canvas 保存中撤销导致离页保护提前关闭 | CAS 回归修复前失败、修复后通过 |
| R1-06 | 已验证 | Bun 隔离依赖下 React 类型不可见 | 根依赖修复，check/build 通过 |
| R1-07 | 已验证 | 确认无用的旧 UI 实现与包装 | 删除 20 文件；入口/引用/生产构建确认，见首轮记录 |
| R1-08 | 已验证 | 草稿退役分支与多余泛型 | 真实 React hook 回归通过 |
| R1-09 | 已验证 | 模型健康测试重写产品规则 | 提取并直接测试生产规则 |
| R2-01a | 已验证 | 同一 Mount 并发首次 demand 触发身份冲突 | 真实 Node 回归修复前失败；Actor 合并同配置初始化，成功/拒绝/超时向所有等待者交付；不同配置不合并；Host/adapter/Kernel 54 测试通过 |
| R2-01b | 已验证 | Mount 卸载与资源交付/回收的竞态 | R5 已复现回收遗漏、同代句柄复用、双重释放、在途服务/获取；JS 生命周期 + Rust 交付屏障、独立租约已实现；跨层 77/0、定向复验 23/0；仓内仍无 unload_mount 生产调用 |
| R2-01c | 已验证 | Host 服务异步任务与停止屏障 | R3 四个真实 Node 回归先失败后通过，另测关闭后拒绝新服务；Actor JoinSet 归属/取消/超时/panic；Host/adapter/Kernel 共 59 项通过。仓内尚无实际业务服务接入 |
| R2-02 | 已验证 | conversation 测试里独立实现业务逻辑 | 只读测试改挂真实 useNomiMessage，steering 改测生产 steerOrQueue；全 UI 3287/0；后者不覆盖完整 SendBox DOM |
| R2-03 | 待审 | 超大浏览器/知识库/会话/Canvas 模块 | 逐子模块深审，不用机械拆文件充当质量提升 |
| R2-04 | 待审 | 生产 bundle 大块 | 先测量依赖与加载路径，不调高阈值掩盖告警 |
| R2-05 | 已验证 | 公共 createContext 初值与无效同步分支 | 明确内部 state + initialValue；删除死 effect/JSON 克隆/重复类型，工厂按实例初始化。false 旧实现失败；6 个回归覆盖 falsy、批更新、重渲染和实例隔离；全 UI 3294 项通过 |
| R2-06 | 已验证 | 测试框架导入与手写 API 声明重复/缺失 | 14 处 Vitest 导入统一 bun:test，删除 Vitest shim；Bun shim 改为官方 test API + 原 equality 契约。只引入测试类型，不污染浏览器全局；check/test/build 通过 |
| R3-01 | 已验证 | activate 阶段的 SDK 服务调用与 Mount 驻留时序 | R5 真实 Node 复现未知 handle 导致整代失败；精确绑定 pending MountLoad、失败激活等待 SDK 收尾、关闭旧 SDK 已实现；含未知 handle 拒绝、失败重试及悬挂取消；跨层 77/0 |
| R3-02 | 已验证 | 消息批处理污染旧快照的索引、空回调栈 | React updater 重放回归旧实现把 2 行变成 3 行；删除跨快照可变 WeakMap 和从未写入的队列，保留每批索引；19 个定向测试和全 UI 3294 项通过 |
| R3-03 | 已验证 | 历史分页的陈旧响应与 loading 状态归属 | R4 五个真实 Hook 回归旧实现失败；统一已提交 scope/revision 与请求归属、分页复用时间合并；删除无生产调用的全量模式及 3 个重复结构断言，13 个行为测试覆盖失败、StrictMode、卸载和事件；UI 3304/0、check/build 通过 |
| R4-01 | 已验证 | SQLite 分页参数加法/乘法溢出 | 会话 7 个、需求/素材各 1 个旧实现回归实际 panic；五处 limit 先升位加法，三类仓储共用安全 offset，删除搜索结果整页克隆；最终分页定向 14/0，保留各接口默认值和上限；详见 R4 |
| R5-01 | 已验证 | JS 资源适配的拒绝清理和回调归属 | 3 个真实 Node 回归先失败后通过；非法/重复获取结果先释放本次资源、保留原方法接收者；清理失败统一关闭代际，不依赖 Rust Kernel 校验兜底 |
| R6-01 | 已验证 | Actor IPC 背压阻塞监督与关闭，出站缺少限额 | 4 个真实 Node 回归先失败后通过；统一 Actor 有界出站队列、受限序列化、取消安全的分段写入和原截止时间；新增 13 项回归，最终跨层 90/0、IPC 复验 7/0，详见 R6 |
| R7-01 | 已验证 | Host 启动 stderr 背压、错误暴露与清理收尾 | 4 个真实 Node 缺陷已红→绿，原取消回收通过；合并有界 cleanup、诊断任务归属及安全错误，9 项定向通过；最终跨层 99/0，详见 R7 |
| R8-01 | 已验证 | Host 运行时协议绑定、重复服务与失败交付 | 4 个回归红→绿；角色绑定、响应写完后释放关联、验证前保留 pending、安全公开错误；最终跨层 105/0，详见 R8 |
| R9-01 | 已验证 | 有界队列之外的在途请求/服务累积 | 3 项红→绿；独立工作/取消/服务配额，合并等待者计数，新增 5 回归；跨层 110/0，详见 R9 |
| R10-01 | 已验证 | Host digest 文件并发发布与文件边界 | 并发竞争红→绿；完整后非覆盖发布、有界内容校验；新增 5 回归，跨层 115/0，详见 R10 |
| R11-01 | 已验证 | 清理证明未完成时的重启与提交屏障 | 复用底层观察凭据，2 项红→绿；跨层 117/0、底层边界 23/0；app 绑定层另续 R12 |
| R11-02 | 已验证 | 无调用的 Host 旧错误/响应入口 | 删除 HelloTimeout、host_failure_response 及专用常量；全仓引用检查及跨层验证 |
| R12-01 | 已验证 | Runtime 绑定停止的取消窗口和替换屏障 | 2 项红→绿；绑定保留至 proof 完成，删除 take/回填及 process_count 判断；App 2/0、跨层 118/0 |
| R13-01 | 已验证 | 在途 Mount 被提交查询误判为空 | Actor 统一 pending/resident 查询；真实激活红→绿及队列/超时回归；跨层 121/0 |
| R13-02 | 已验证 | 自动应用嵌套 Runtime 读租约等待环 | 真实 Authority 排队写租约红→绿；复用已有租约并合并重复分支；App 3/0 |
| R14-01 | 已验证 | 公共模式脱敏的格式漏识别和重复复制 | 6 项红→绿，现代 key/Bearer/引号赋值/PEM 和 Cow 已修复；16/0、浏览器定向 20/0 |
| R15-01 | 已验证 | API 页面响应因 UTF-8 截断及标准 XHTML 类型漏识别 | 2 项红→绿，直接比较 ASCII marker 并删除解码/小写分配；定向 7/0 |
| R15-02 | 已验证 | 出站跳转重置超时及特殊 IPv6 地址放行 | 2 项红→绿；单次总超时、DNS 上限、地址策略；net 43/0，知识库 21/0 |
| R16-01 | 已验证 | 代理环境读取被无关非 Unicode 变量触发 panic | 独立子进程注入红→绿；vars_os 保持 Unicode/空值规则；net 47/0 |
| R16-02 | 已验证 | 代理端点解析与跨平台重复装配 | 端点红→绿；统一端口验证和四平台装配；代理 26/0、net 47/0 |
| R17-01 | 已验证 | 代理辅助进程 stdout/退出/清理不共用边界 | 后代持管道红→绿；共用受管所有者、有界读取/清理；net 50/0，详见 R17 |
| R18-01 | 已验证 | 代理缓存并发重复探测和过期起点 | 2 项红→绿；同一探测/发布归属、完成后 TTL；net 53/0 |
| R19-01 | 已验证 | 精确凭据重叠匹配/截断泄漏及替换重扫描 | 4 项红→绿、穷举区间 oracle；net 58/0、provider 19/0 |
| R20-01 | 已验证 | URL 诊断脱敏规则分散、括号查询与重复脱敏 | 3 项红→绿；删除模型层双实现、Agent 包装；net 61/0、provider 19/0、模型 15/0、Agent 36/0 |
| R21-01 | 已验证 | 精确凭据百分号混合大小写及二次编码 | 3 项红→绿，等长规范匹配替代大小写枚举；net 65/0、provider 19/0、模型响应 4/0 |
| R22-01 | 已验证 | 共享客户端失败回退丢失配置及代理键重复 | 删除默认客户端回退、明确既有 panic 契约，保留有错返回入口；net 66/0 |
| R23-01 | 已验证 | Agent 分词后 Bearer 空格判断永不命中 | 真实入口红→绿，覆盖引号/括号/HTML/大小写/行边界；send_error 38/0 |
| R24-01 | 已验证 | SFTP 破坏性覆盖、临时碰撞及忽略权限/关闭错误 | 4 项协议红→绿；独占创建、句柄权限确认、原子扩展、禁先删后改名；nomi-ssh 27/0 |
| R24-02 | 已验证 | SFTP 目录上限在全量累积之后才检查 | raw 逐批有界累计/早停，缺 size 文件读取仍受限；协议回归通过 |
| R24-03 | 已验证 | SFTP 取消后的句柄归属/恢复与分散截止时间 | 单槽成功复用、错误取消关闭并重建；服务器 EOF/恢复、单次预算协议回归通过 |
| R25-01 | 已验证 | SSH glob 重定向、分词、选项注入及引用规则冲突 | 4 项可靠红→绿，sink 10/0；安全转义与内置列表替代 ls，见 R25 |
| R26-01 | 已验证 | 持久 shell UTF-8 按包解码破坏跨包字符 | 3 项红→绿，流式解码/终结上限、4681 小流 oracle；nomi-ssh 33/0 |
| R26-02 | 已验证 | shell 初始化目录状态与参数/环境归属 | cd 非零状态和真实 sh 目录语义共四项红→绿，nomi-ssh 44/0；原生 OpenSSH 待验 |
| R27-01 | 已验证 | SFTP 依赖读包未使用 max_packet_len，分配可越过输出上限 | 超限包头红→绿；256 KiB 帧边界及原字节保真，见 R27 |
| R27-02 | 已验证 | SFTP 依赖关闭帧无法越过阻塞 write_all | Pending 写入红→绿，取消中断写/关闭并观察底层流 Drop；nomi-ssh 40/0 |
| R26-03 | 已验证 | shell 等待/写入/关闭总预算及退役任务未统一归属 | 八项预算/Drop 红→绿；合并状态槽、无脱管任务、显式收证，nomi-ssh 56/0；transport/channel-open 未取得句柄边界另续 |
| R25-02 | 已验证 | SSH grep/列表超时及命令失败被转成功字符串 | 四项红→绿；仅无匹配归一、按可用性选引擎、共用结果校验；后端单元 30/0，见 R25 续审 |
| R25-03 | 待审 | SSH glob 匹配带控制字符的名称不能由行协议准确表达 | 输入校验不约束通配匹配到的名字；需核对路径/PTY 输出契约后拒绝或重设计 |
| R28-01 | 已验证 | 固定 logo URL 长期 immutable 与条件请求语义；无状态路由包装冗余 | 两项红→绿，10/0+App 2/0，删除 state.rs，见 R28–R30 |
| R29-01 | 已验证 | CRLF 和空行导致内容丢失、行尾空白重复遍历 | 红→绿，合并清理 pass，compact 54/0 |
| R29-02 | 已验证 | JSON 键未转义和结构化输出被当日志折叠 | 键转义红→绿，结构保护回归通过；54/0+Agent 8/0 |
| R29-03 | 已验证 | TOON 类型/字段引用歧义、转义及括号边界错误 | 三项红→绿，使用 serde、保留外围文本，见 R29 |
| R29-04 | 已验证 | 相似行字符/字节单位混用导致 Unicode 不折叠 | 中文相同行红→绿，统一字符数 |
| R30-01 | 已验证 | stdin 行/队列无界及接收方关闭；stdout 重复锁/缓冲 | 三项红→绿、六项读取回归，protocol 49/0；OS stdin 限制见报告 |
| R30-02 | 部分完成 | CLI 生命周期及全部 JSON 输出入口的失败清理已修复 | R48 17/0；CLI/sink/engine 共用现有 emitter 的有界失败通知，Ready 失败不启动 reader，回调失败退出在途轮次且不发成功 StreamEnd。connect_all 内 Stop/EOF 取消、bootstrap 失败资源归属仍待跨 crate 审计；未验真实 OS broken stdout |
| R31-01 | 已验证 | 工具描述按字节截断及 CRLF 段落遗漏；共享类型测试重复 | 两项红→绿，types 70/0、provider 定向 2/0，见 R31 |
| R32-01 | 已验证 | 存在的非法/不可读配置静默退回默认 | 两项红→绿，仅 NotFound 可选；config 定向 66/0 |
| R32-02 | 已验证 | profile 模型优先级失效、继承 compat 字段丢失 | 两项红→绿，复用字段合并；config 定向 66/0 |
| R32-03 | 部分完成 | 配置字段存在性/合并、初始化、profile 深链已修复；硬迁移并发窗口待办 | 四项合并红→绿；最终 config 184/0，剩余边界见 R32–R33 |
| R33-01 | 已验证 | Hook 将工具输入拼成 shell 源码及诊断泄漏 | 真实 shell 注入红→绿；复用环境通道，Agent hook 2/0 |
| R33-02 | 已验证 | schema 清理误删关键字同名参数并改写实例数据 | 两项红→绿，合并遍历；provider 定向 7/0 |
| R33-03 | 已验证 | 无生产调用的旧 shell 构造器、日志无效配置副作用与全局测试状态 | 删除旧路径，迁移到受管 shell 测试；config 最终 184/0 |
| R35-01 | 已验证 | 记忆索引 UTF-8 截断 panic 及全行 Vec 冗余 | Unicode 红→绿，memory 150/0 |
| R35-02 | 已验证 | frontmatter 分隔符偏移/无效 YAML 丢原文 | 两项红→绿，简化完整行解析 |
| R35-03 | 已验证 | 索引追加丢失并发条目及覆盖非 UTF-8 原文 | 两项红→绿，改持锁直接追加 |
| R35-04 | 已验证 | 引用计数溢出/并发丢失及回写损坏正文元数据 | 四项红→绿；symlink TOCTOU 等限制见 R35-07 |
| R35-05 | 已验证 | memory 无使用的错误分支与依赖包装 | 删除 error.rs 和 thiserror/rstest 直接依赖，Agent 17/0 |
| R35-06 | 已验证 | 后端蒸馏 name 字段漏脱敏 | 补用现有脱敏函数；ai-agent/companion check 通过，无真实模型回归 |
| R35-07 | 待审 | 记忆路径命名碰撞、索引/文件写入契约及非协作文件边界 | 已发现静态证据，需兼顾已有数据归属；完整范围见 R35 报告 |
| R36-01 | 已验证 | MCP HTTP 通知忽略错误状态、响应错配及诊断回显凭据/正文 | 前五项回归修复前均失败；共享 headers/status、校验 id/version、移除敏感回显，最终 MCP 122/0 |
| R36-02 | 已验证 | MCP SSE 分块破坏 UTF-8、重复解析、监听器脱离传输生命周期 | 按完整字节帧解码、共享解析、Drop 中止监听并标记关闭；含 Unicode/Drop/空白回归，无旧失败证据 |
| R36-03 | 已验证 | MCP 自定义凭据可随跨源 endpoint/redirect 外发 | URL 标准解析、endpoint 与重定向仅同源；两项回归通过。跨源服务需显式配置，不再自动转发 |
| R36-04 | 待审 | MCP 剩余传输、管理、代理及协议边界 | stdio 未完整读；manager 只读到请求超时/连接段，tool_proxy/协议测试未读完。body/SSE 缓冲无界、混合换行、JSON-RPC 完整校验、session DELETE、关闭等待及请求门等待超时仍待审；未验真实外部服务 |
| R37-01 | 已验证 | domain-support 无执行实现却返回 accepted 成功 | App v4 装配直接注册 model-media 占位工具；改用现有 CapabilityExecution 错误，合并三类未配置处理器。8 项元数据回归及 App check 通过；返回错误分支经静态核对，无完整调用回归 |
| R37-02 | 已验证 | domain-support 构造重复及无调用资源辅助函数 | 共享私有 const 默认构造，删除全仓无引用的 typed_resource_bindings_for；不改 wave 私有实现，不改 C7 元数据；模块净减 100 行 |
| R38-01 | 已验证 | skills shell 非零退出因有输出而被报告成功；重复包装错误 | 现有测试改正断言后旧实现失败；按退出状态失败并保留诊断，删去嵌套错误文本。skills 407/0、Agent skill_tool 59/0 |
| R38-02 | 已验证 | skills 空引号参数丢失/位置错位、换行不分词；解析及测试冗余 | 空参数红→绿，保留 token 起始标记并用 mem::take；简化 frontmatter 行偏移与 YAML→JSON，删除 paths 8 项/substitution 15 项重复测试、修正恒真断言；407/0 |
| R38-03 | 部分完成 | skills 剩余解析容量和 MCP/加载边界 | R47 已修 inline shell 副作用标记（见 R47-01），审批策略未改。R40 已读完剩余测试，YAML 回退/嵌套语义已修。brace 输出/递归深度、多行 YAML 集合/anchor 回退、MCP 分页/命名碰撞/软预算、loader 深度/大小/SKILL.md 文件链接仍待审 |
| R39-01 | 已验证 | skills 目录环重复加载，两套遍历及重复 metadata 查询 | 真实 Windows junction 旧实现加载同一技能 64 次；统一遍历、祖先 canonical 路径防环、排序稳定优先级；正常目录链接/旧命令格式均通过，未运行原生 Unix symlink 分支 |
| R39-02 | 已验证 | 相邻占位符漏替换、参数/环境值被重扫、全文参数误匹配及错误追加 | 三项新测试旧失败；单次扫描原文，保留任意非数字命名参数及边界语义；按消费标记决定 fallback，替换值不再当模板解析 |
| R39-03 | 已验证 | 旧平铺命令基目录不存在；基目录说明被当模板解释 | 两项既有测试补断言后旧失败；根目录取真实文件父目录，说明在替换及 shell 执行后添加；没有改变正文参数→shell 的现有契约 |
| R39-04 | 已验证 | skills 测试重复及权限补充文件独立重复夹具 | 删除 loader 10 项、executor 6 项重复/无有效断言用例；权限 3 组独有断言合并入原用例后删除补充文件；复用加载写文件辅助函数，无新增依赖/框架 |
| R20-02 | 待审 | model-invoke 默认并发全量测试持续高 CPU 未完成 | 单例 1/0（1.27s）、4 线程全量 396/0（37.04s），尚未定位默认高并发资源热点；未把降低并发当根因修复 |
| R40-01 | 已验证 | YAML 回退为问题字段补引号时破坏其他合法集合/块标量 | 仅补引号给无法独立反序列化的顶层行；数组、hooks map、block scalar 组合回归通过；未声称多行集合/anchor 已覆盖 |
| R40-02 | 已验证 | brace 嵌套匹配与顶层逗号拆分错误、重复字符串分配 | 按深度匹配括号并借用切片拆分；嵌套/后缀/Unicode/空分支/未闭合回归通过，上限问题留 R38-03 |
| R40-03 | 已验证 | Skills 重复/弱断言及只验证自写去重的假集成测试 | 合并同义用例、改精确输出/饱和上限断言；删除不调用生产去重逻辑的用例，保留 loader 原有回归；Skills 净减 133 行 |
| R42-01 | 已验证 | JWT 黑名单在 exp 清除会在时钟宽限期重新接受已吊销令牌 | 保留至 exp + 默认 validation leeway（含边界），维持验证策略；过期宽限内/外回归通过，未新增调度器 |
| R42-02 | 已验证 | 限流墙钟跳变、计数溢出/零配额与清理任务持有对象不释放 | 用 Instant、共享窗口重置、饱和计数和 Weak 清理所有权；并发配额及 Drop 等六项回归通过 |
| R42-03 | 已验证 | 密码抽样有模偏差且 shuffle 下标限于 256；CSRF Bearer 解析重复 | 拒绝采样现有随机数，保留长度/类别约束，去掉 shuffle 临时 Vec；CSRF 复用与原规则等价的 extractor；认证包 246/0 |
| R42-04 | 待审 | 认证跨路由、代理信任及令牌/密码兼容策略 | R44 已读完剩余 11 源文件及指定集成测试；128 字节密码 vs bcrypt72、私网 peer 转发信任、失败后计费并发穿透、同秒 JWT、blacklist 清理生产入口仍未解决。另有密码/secret 多步持久化及跨请求续期-登出竞态，需事务/部署契约核对 |
| R43-01 | 已验证 | 平台 invoke 同步发送抛错后保留响应监听器 | 新回归旧实现 1 失败，改 once + catch 中解绑后定向 3/0；同步 falsy 响应与重复响应边界一并覆盖 |
| R43-02 | 已验证 | UI 旧手动更新分支无生产实现及平台无调用导出 | update.download 恒为错误 stub、progress 为 noop、recommendedAsset 无提供方；删除专用状态/类型/监听及无调用 subscribe/颜色 token，保留 Tauri 原生下载/安装与外链；未删后端独立 release API 的资产类型 |
| R43-03 | 已验证 | 已合入产品变更与 Guid/MiniApp 结构测试断言脱节 | git show HEAD 确认窄模型引用/稳定 preset 和四条路由均已存在；仅更新两测试，保留禁止 plain-Nomi、完整 fallback 和不传配置凭据的约束；定向 10/0、最终 UI 3304/0 |
| R44-01 | 已验证 | 下游 logout/吊销/轮换后滑动续期仍追加会话 cookie | 显式 session cookie 优先、发布前重验 token；真实 logout/显式/无关 cookie/吊销/secret 轮换五场景通过；不宣称跨请求竞态消失 |
| R44-02 | 已验证 | QR TTL 使用墙钟且清理未接入、任务强引用泄漏 | 单调 TTL、精确过期边界、auth router 接已有清理任务并用 Weak；单次并发消费/清理/Drop 回归通过 |
| R44-03 | 已验证 | 修改用户名的仓储冲突被包装为 500 | 两个 handler 删除冗余错误包装，复用已有转换返回 409；真实 SQLite 保留原用户名回归通过 |
| R44-04 | 已验证 | 多 CSP policy 合并字段导致 frame-ancestors 替换丢其他限制 | 对逗号分隔的每条 policy 独立替换；现有多 CSP 回归增加 script-src 保留断言，白名单策略不变 |
| R46-01 | 已验证 | 飞书无效/截断响应假成功与响应/等待无界 | 要求显式整数成功码、30 秒请求上限、64 KiB 响应累积上限；回环 HTTP 覆盖非法/截断/已知长度和 chunked 超限 |
| R46-02 | 已验证 | Webhook 网络/远端错误回显端点凭据和响应正文 | 使用错误类别、HTTP 状态、数值飞书错误码；不回显 URL/正文，合成凭据回归通过 |
| R46-03 | 已验证 | 通知使用 SQLite 行号且待审核被报告完成 | 改为稳定 requirement_id、needs_review 明确状态；Unicode 截断只扫描前缀；删除两处测试 Box::leak |
| R46-04 | 已验证 | notify_events 元素带逗号经存储往返变成多个事件 | 校验既有 done/failed/needs_review 集合，保留空/省略语义；非法更新不落库回归通过 |
| R46-05 | 部分完成 | Webhook 与仓储/需求通知的跨模块契约 | R52 两项真实并发丢更新回归红→绿；以原子字段 SQL/RETURNING 取代先读再整行写，父绑定同事务校验。Requirement detached 通知、cancelled 触发集合不一致、owner 任意端点/重定向策略仍待处理 |
| R47-01 | 已验证 | inline 技能实际执行 shell 却不使旧完成证据失效 | 同一替换及 shell regex 判定，MCP/纯文本保持非副作用；成功/写后失败实文件与分类两回归红→绿，已有证据测试补成功 opaque 分支；不改审批类别/执行语义 |
| R45-01 | 已验证 | Public 预检拒绝合法 JSON 转义方法且读取错误误报超限 | Cow 接受转义方法且保留原始 body；400/413/408 分流，不暴露读取诊断；29/0 中包含两项新预检回归 |
| R45-02 | 已验证 | Public 创建在 map 锁等待时取消遗留占位、跨 manager 释放占位 | 先取 map 锁，再无 await 地预留并发布；释放复用归属检查；取消/重复释放/流 body 配额/关闭回归通过 |
| R45-03 | 已验证 | 初始化 token refill 丢失不足整间隔的时间 | 未满桶保留小数间隔；满桶丢弃剩余信用；3 项时间边界回归通过，无新计时器 |
| R45-04 | 待审 | Remote rmcp 处理生命周期与 Host/session 跨模块资源边界 | HTTP body 配额不约束 detached handler/product operation，context.ct 未消费；Host shutdown 未接 transport cancellation；session store observation 即使取 1 条仍加载全量投影 |
| R49-01 | 已验证 | Runtime 重复 init 告警误用 OnceLock::set 的被拒新值 | 比较实际首次保存路径；first-init-wins 不变，37/0；未捕获 tracing 告警作运行断言 |
| R49-02 | 已验证 | 解压缺别名误命中新鲜缓存、失败遗留 staging 与锁外无效删目录重试 | freshness 检查 bun/bunx/node；stamp 最后发布，Unix 同级链接；锁内清 staging，删除相同 blob 重试和递归清缓存；Windows 回归通过 |
| R49-03 | 已验证 | embed→stub 构建仍包含旧 blob、ZIP 后缀匹配误取文件 | stub 使用空字节常量；ZIP 精确末级文件名；移除空 write；stub 元数据回归通过，未运行真实发布下载 |
| R49-04 | 已验证 | Runtime 测试依赖宿主环境、全局环境写入及无业务测试 | 显式人工 override/home/PATH fixture，去除泄漏/环境写入；删仅测 zstd/tempfile 的 extract_integration.rs，真实 extract_into 及失效/修复回归保留；37/0 |
| R49-05 | 部分完成 | Runtime 构建缓存发布、跨平台探针及解析策略仍未闭环 | R55 删除 BUN_DIR 失败永久缓存，目录查询复用成功解析；隔离子进程回归红→绿，38/0。仍待：build.rs 存在即复用/直接写最终 exe/blob、非 Linux Unix reader join 被后代拖住（LNX-04）、nvm/fnm 逆字典序、musl/arm64 baseline 真实包核对 |
| R50-01 | 已验证 | macOS VS Code fallback 检测命中但仍执行裸 code | 检测返回具体程序并由 opener 启动；记录器检查 argv/未安装不执行；Windows 通过，macOS 条件分支未运行 |
| R50-02 | 已验证 | STT 未落实音频 30 MiB 上限且 multipart 超限变成 400 | 总请求 31 MiB、音频 30 MiB 分层校验，保留 413/400；大于 10 MiB、恰好 30 MiB、音频/流式总量超限及畸形 body 回归通过 |
| R50-03 | 已验证 | Shell 路由测试误字段、无断言及语音夹具不必要泄漏 | snake_case+具体业务错误；TTS 合并无上游配置的 Unicode 边界；删除两处 mem::forget 和无断言 VS Code 测试；整包 94/0 |
| R50-04 | 部分完成 | Windows terminal cmd 解释及 opener hand-off 生命周期 | R65目录改为原生cwd、不进入命令文本，两层cmd禁用AutoRun，真实隐藏命令及记录器通过，Shell96/0；未验实际终端GUI/UNC。hand_off仍等退出/收stderr且builder保留kill_on_drop，请求占用/取消误杀/无界stderr待统一进程契约 |

| R51-01 | 已验证 | 配置旧 GET 跨 reset/reload 覆盖新状态、在途本地写/删除丢失 | 请求身份及修改 key 集合隔离；保护待 PUT key，同步失败后仍可重试；33 新测试及调用方共 44/0 |
| R51-02 | 已验证 | 配置批次半发布、订阅抛错阻断写入、旧 unsubscribe 解绑新订阅 | 整批 cache 后通知，单个同步订阅错误隔离，解绑捕获原集合；合并 set/remove/setBatch 写入口 |
| R51-03 | 部分完成 | 配置服务端写顺序、reset 与 hook/多窗口状态边界 | R56已修hook回滚/reload/DOM和局部native顺序；并发PUT未做服务端排序/CAS，reset不取消已发PUT、请求无超时；多个hook实例/真实多窗口和已发native调用取消未闭环 |
| R53-01 | 已验证 | WebSocket 注册与取消信号获取有窗口、关闭后写任务脱离请求 | 原子返回 ID/信号，RAII 注销，收发 future 同一请求归属，Close 后停止发送并有界确认；真实回环关闭/4409/背压回归通过 |
| R53-02 | 已验证 | 心跳任务不随 manager 释放、连接计数测试靠固定 sleep | Drop 取消 heartbeat 与 client 信号；有界状态等待取代固定 sleep，删无效 accessor/无断言用例；Realtime 102/0 |
| R53-03 | 待审 | Realtime 认证、全局 shutdown 与拥塞关闭/容量剩余契约 | WS vs HTTP 用户有效状态校验不一致；App 未显式持有 manager/upgrade shutdown；阻塞 sender.send 延迟 policy Close；heartbeat panic/runtime teardown 后 AtomicBool 不复位且 start_heartbeat 可多开；队列按条数非字节、无总连接上限 |
| R54-01 | 已验证 | 技能导入导出先删目标可删除重叠源、同步非法目标先 prune | 规范化并检查源/目标重叠和全部 workspace 目标后才操作；缺源失败保留目标；替换末级链接不跟随；真实树回归通过 |
| R54-02 | 已验证 | link AlreadyExists 回退覆盖并发赢家、ZIP 写完才校验额度 | R54 冲突直接返回；R59 将8KiB写前额度合并到common，修复Knowledge/Companion同类缺陷及u64饱和绕限；两真实ZIP先红后绿，Common11+Skills5+Companion26+Knowledge9+Workshop12通过 |
| R54-03 | 已验证 | 外部路径设置持久化失败但已修改内存 | 锁内克隆候选，save 成功再发布；add/update/remove 失败均保持原状态 |
| R54-04 | 已验证 | 技能 location 误用 frontmatter 名称且 IO/目录遍历错误被吞 | 使用实际目录，缺 name 从 manifest 父目录回退；保留非 NotFound IO、传播遍历错误，完整 fence 行支持 CRLF；160/0 |
| R54-05 | 部分完成 | 技能文件发布/链接安全与跨模块命名及 ZIP 契约 | R59 已修Knowledge/Companion ZIP预算并统一复制。仍待TOCTOU、内部链接递归复制/环、delete-then-copy无回滚；JSON非crash/cancellation/cross-manager原子；startup两rename读窗口。保留名companion/shared/_drafts、frontmatter vs resolver、复杂YAML、无调用UI bridge query/JSON与scan shape错配仍待核对 |

| R57-01 | 已验证 | Wave1按trim后长度校验却向owner传原文，空白绕过maxLength | 改为仅用trim检查空值、原文计字符；跨10种operation边界和真实Kernel INVALID_PAYLOAD回归，8/0 |
| R57-02 | 已验证 | Wave1无调用批量绑定包装及重复构造 | 删除typed_resource_bindings_for、三个canonical binding复用已有构造；全仓引用核对，生产净减37行 |
| R57-03 | 待审 | Wave1声明、宿主装配及资源预算剩余范围 | knowledge.search声明Gateway但无action handler；六action/六context/两resource及source.sync/skill.hooks未接实际owner，先确认支持契约不自动补功能。Project items空数组校验与schema minItems不一致；items/Skill arguments总量嵌套及owner截止时间/取消后持久化未闭环 |
| R58-01 | 已验证 | Wave3非对象输入错误归属不一致且公开转换可接收Serde位置数组 | 对象校验收敛到operation_from_input，统一WAVE3_INVALID_REQUEST；18action表驱动回归，10/0；非Creation其余字段仍由业务入口校验 |
| R58-02 | 已验证 | Wave3重复空schema及恒成立/重复成功测试 | 合并两个完全相同schema函数不变digest，现有注册测试改核对每项effect/presentation；删重复转换测试，生产-8/测试+6 |
| R58-03 | 待审 | Wave3输入深度与Creation/模板运行资源契约 | Creation仅要求generation_provider，Canvas/template由输入+安装owner业务校验，是否应冻结到snapshot须统一；template.run只声明canvas:write，实际provider/model来自模板；非Creation/输出对象之外依赖owner，Office未准入Nomi；不自动加新资源规则 |
| R60-01 | 已验证 | 知识库导出固定dest.tmp覆盖无关文件、并发共享临时输出 | tempfile独占创建并持句柄写入、sync后persist，移除手动错误清理；既有同名tmp内容保留回归红→绿 |
| R60-02 | 已验证 | 知识库导出吞WalkDir错误发布残缺包、每个文档整块读内存 | 遍历错误显式传播保留旧包，流复制取代整文件Vec；缺源旧回归失败，修复后export11/0 |
| R60-03 | 部分完成 | 导入临时目录及知识库导出其余文件/事务边界 | R63独占TempDir及blocking持有Arc修复Knowledge/Companion碰撞与解压取消后过早清理，16并发回归红→绿，39/0；后续DB/文件修改取消事务未解。Knowledge导出lossy文件名/Unix反斜杠、非快照、目录fsync，导入metadata容错/重复entry/并发名称保留 |

| R56-01 | 已验证 | 主题/配色未响应reload删除，失败回滚与缓存及DOM不同步 | 配置订阅驱动视觉与缓存提示，revision抑制旧广播/回滚；仅保留唯一支持配色，删除冗余状态 |
| R56-02 | 已验证 | 离线启动后成功读取空配置未通知缺省就绪，保留旧主题提示 | 主线程实际回归旧失败；replaceCache复用订阅通知、无新事件框架；config/hooks调用方68/0 |
| R56-03 | 已验证 | 缩放过期native结果覆盖新值，失败写后保留乐观状态 | hook局部promise顺序及revision守卫，失败reload；adapter返回因子保真。跨实例/native已发请求/服务端顺序留R51-03 |

| R61-01 | 已验证 | Wave4非对象action输入的错误归属与公开转换不一致 | 统一转换入口，十action拒绝非对象为WAVE4_INVALID_REQUEST，20/0 |
| R61-02 | 已验证 | Wave4重复binding/空schema构造、Remote fixture依赖及重复测试辅助 | 复用现有构造，移除无调用批量helper及重复断言夹具；生产-57/测试-20 |
| R61-03 | 部分完成 | Wave4宿主激活顺序、策略读锁及资源/Context剩余契约 | R70已修Channel policy先读DB后等锁，正确专用事务回归红→绿、App17/0。Robot link/audio按需路径先取lease后activate但owner要求先active仍未修；Context预算、destination/message绑定策略及支持范围未闭环 |
| R62-01 | 已验证 | Wave5字符串错误丢canonical code/泄漏Display诊断、非对象边界及伪unknown binding | 复用typed failure，真实owner mismatch才附binding；对象输入/输出校验，11/0 |
| R62-02 | 已验证 | Remote exact descriptor漏端口版本和command/receipt schema，构造及测试冗余 | 六类变异拒绝且顺序无关；空对象不解析fixture，主线程删无调用批量binding helper |
| R62-03 | 待审 | Wave5宿主未接线、D026 exact标志及Remote运行时保证 | App action host仍unconfigured不自动接功能；D026前两outcome explicit-session需契约确认；期限/关闭/exact-zero属App/Runtime，声明测试不证明实际运行 |
| R64-01 | 已验证 | Login同渲染重复提交、卸载后回调和延迟重定向覆盖后续导航 | 提交ref及finally/卸载守卫；导航交现有router，Login20/0、全UI3381/0 |
| R64-02 | 已验证 | storage抛错阻断页面/提交，以及无引用装饰CSS/ref | storage失败隔离、focus跟随认证探针；删注释背景及62行CSS，不改正常remember-me契约 |
| R64-03 | 部分完成 | 共享Auth请求代际、持久凭据和真实WebUI端到端边界 | R71已修被取消refresh覆盖状态和非Error二次异常，五项旧失败、Auth/Login32/0。login/setup/logout跨操作竞态、QR提前返回、cookie归属与可逆密码localStorage策略仍待审；未跑真实浏览器登录 |
| R66-01 | 已验证 | 系统设置部分更新先读后写整行导致不同字段并发丢失 | 真实SQLite旧失败；现有仓储传Option字段，单条原子UPSERT/RETURNING，System/DB/routes/调用方50项通过 |
| R66-02 | 已验证 | 设置测试重复、测试数据库handle无必要泄漏及单例id注释错误 | 删除重复DB内联用例并保留公开trait断言，标量偏好合并、移除两mem::forget；生产-26/测试-53 |
| R67-01 | 已验证 | Wave2三个严格workspace输入只在Nomi包装执行校验，普通Kernel入口可绕过 | handler复用既有schema校验；9非法/3有效输入验证owner调用边界，非Browser13/0 |
| R67-02 | 已验证 | Wave2资源常量别名、polling和host request测试夹具重复 | 复用现有常量/fixture，生产+5测试-14，净减9行；未删Browser专用覆盖 |
| R67-03 | 待审 | Wave2 typed dispatcher/Computer schema及宿主缓存预算剩余边界 | direct typed入口仍绕过shape校验；Computer Context声明空object但owner返回generation/result；200ms debounce清理不硬限1024，owner caches淘汰未确认；未配置SSH/connector/resource provider不自动补功能 |
| R68-01 | 已验证 | Cron Every下一次时间及At延迟整数溢出 | checked_add/saturating_sub，校验和最终计算均拒绝溢出；创建/更新无部分写入，全包260/0 |
| R68-02 | 已验证 | Cron busy先查后设非原子且取消跳过释放 | DashMap占用+作用域permit；真实execute_prepared的重复发送/完成/abort回归；删除无调用set_processing |
| R68-03 | 已验证 | Cron关闭后已运行任务仍可重新安装timer | 终态shutdown标志和既有mutation gate内准入；保留init/resume临时cancel_all语义 |
| R68-04 | 已验证 | 技能建议持久化失败被提前缓存hash抑制重试 | 传播错误，成功后写hash；实际SQLite失败后补conversation重试成功且无孤立job事件 |
| R68-05 | 已验证 | Cron重复序列化/忙测试桩/自测构造与未读正文副本 | 复用已有fixture与schedule转换；生产-28测试-234，260/0；删除代码可从Git恢复 |
| R68-06 | 待审 | Cron跨文件事务、技能路径、detached检测和embedded receipt取消边界 | save/delete/job启动与持旧快照文件生成无统一并发约束；symlink/junction/TOCTOU；检测spawn未登记；embedded owner abort/panic后sender残留无结果；崩溃不确定保守恢复未改 |
| R69-01 | 已验证 | MCP市场旧解析覆盖、重复导入和离页迟到预览/导航 | 组件revision/ref与tab卸载；原15项4/11、现15/0；取消不撤销后台写 |
| R69-02 | 已验证 | 初始MCP目录GET未完成或失败仍开放安装编辑 | 页面loading/error门控；错误态需重新进入页面；全UI3398/0/typecheck |
| R69-03 | 待审 | MCP共享CRUD/catalog/连接/市场和后端导入剩余契约 | 迟到hook报错、跨重建防重、save Promise卸载可能不settle；批导入非事务/同名upsert；表单吞错/自动测试、connection/OAuth归属、市场刷新/storage异常和provenance回退未改 |
| R72-01 | 已验证 | 版本SemVer build比较与预发布/跨平台资产误选 | precedence、双预发布标记和关键词边界；排除sig/release-lock，补实际安装包扩展名/universal；单元27/0，HTTP19/0 |
| R72-02 | 已验证 | 更新检查先联网后校验、错误正文泄漏和短页过早结束 | repo本地校验、30秒总deadline、URL/上游错误正文隔离，Link指示续页最多5页；HTTP19/0，未实际等待30秒到期 |
| R72-03 | 已验证 | sysinfo重复目录逻辑及把自定义路径误当默认路径的测试 | 保持目录优先级/generation；共用目录选择与平台映射，版本查询不再解析目录；单元与HTTP通过 |
| R72-04 | 待审 | 版本响应容量/重定向、资产优先级和sysinfo嵌入契约 | 五页非正文硬限；缓存/ETag/并发合并未加；client决定重定向；未覆盖所有资产及签名安装；env目录/nonUnicode/uninitialized嵌入边界未改，UI native updater独立 |
| R73-01 | 已验证 | Execution结果汇报错误绕过已有重试worker | 无效Ok(false)分支改为错误安排重试并返回原错；真实SQLite+失败一次效果端口红→绿、相同operation与delivered验证，执行层88/0 |
| R73-02 | 已验证 | Execution取消封装和未消费step集合、冗余ensure/bool契约 | 直接复用真实cleanup/lead reconciliation；生产-41，无新重试框架 |
| R73-03 | 部分完成 | Execution剩余范围与已读未闭环错误/关闭边界 | R81已修产物校验失败缺少非重试错误、计划字符串代码围栏丢失；历史消息回退、同步文件验证/分页容量，loop历史体积、router long-context、任务准入/非取消DB await/副作用、坏outbox头事件、generation取消仍待核对；不以95/0代替全审 |
| R74-01 | 已验证 | 普通Cron任务编辑重写at/every和已存时区 | 未改调度省略schedule，显式cron修改保留时区；真实弹窗回归 |
| R74-02 | 已验证 | 复杂表达式误识别预设且实时更新覆盖草稿 | 非精确预设保持custom，表单按可见性/任务ID初始化；旧失败后通过 |
| R74-03 | 已验证 | 异步校验前重复提交与旧弹窗结果污染 | 同步ref锁及弹窗session检查，未撤销已发后端写入；58/0及typecheck |
| R74-04 | 已验证 | Cron页面完整静态覆盖与回归核对 | 32文件全部已读，12测试文件58/0；相邻模块只读跟踪不算全审 |
| R74-05 | 部分完成 | Cron列表/详情请求归属、表达式方言、时区repair与会话映射 | R78已修列表及runs请求归属/事件覆盖；详情删除/重连/轮询导航，Builder截补六字段/年份与显示，旧快照补时区需DB契约；SessionList active ref/映射清理未修 |
| R75-01 | 已验证 | File写入/复制最终链接越界与目录名单组件校验不一致 | 现有写目标resolve、最近存在复制父路径检查、统一单组件校验；Windows348/0，Unix链接测试未跑 |
| R75-02 | 已验证 | snapshot新文件删除可传绝对路径/父遍历 | 相对路径检查先于index变更；删除验证父路径且只删末端link；glob等仍见R75-08 |
| R75-03 | 已验证 | patch临时名冲突时误删他人文件 | create_new成功后才进入清理域，真实碰撞回归保留临时文件及原目标 |
| R75-04 | 已验证 | 远程图片先collect才限额且redirect绕过URL校验 | 分块执行既有大小上限及逐跳URL校验；未做DNS重绑定/真实外网验证 |
| R75-05 | 已验证 | ZIP整文件读取、取消登记泄漏/覆盖和输入即输出 | 64KiB分块、已打开文件长度上限、Drop取消与身份清理、同ID冲突；不代表输出路径排他或副作用回滚 |
| R75-06 | 已验证 | File watch停止与开始竞态及重复实现 | stop共用watcher锁；复用事件发送、删除无效collect及无行为常量测试 |
| R75-07 | 已验证 | File完整静态覆盖与平台验证记录 | 20 Rust文件11901基线行，348/0；三个Unix链接用例未运行，无修复前行为证据 |
| R75-08 | 部分完成 | File路径TOCTOU/ZIP发布与snapshot生命周期 | R80已修单文件glob/错误吞掉；取消Drop早于blocking结束、同输出路径互删截断、不同路径hardlink别名未隔离；snapshot共享临时目录/清理与并发index更新、路径TOCTOU/DNS重绑定/其他OS和真实I/O失效仍待闭环 |
| R76-01 | 已验证 | Provider创建校验原始平台但保存trim平台 | 全程同一trim值，Ark展示名空白绕过与Bedrock空白拒绝修复；routes14/0 |
| R76-02 | 已验证 | Bedrock更新校验规范值却保存原配置 | 保存next_bedrock，省略不写；三认证方式等价/region变更health-revision真实SQLite回归 |
| R76-03 | 已验证 | Provider无调用的URL单层转发封装 | 全仓确认后直接用已有provider_model校验，生产净减3行；provider单元5/0 |
| R76-04 | 部分完成 | Provider创建/删除跨事务和列表/历史配置边界 | display_name在图事务后、软引用先删DB后删；列表分次查询/单损坏行使全列表失败，历史Bedrock配置未自动修；Ark大小写校验R79已闭环 |
| R77-01 | 已验证 | 模型选票极大索引/候选数直接控制数组分配 | 1024候选本地硬限覆盖入口/解析/执行；usize::MAX旧溢出后通过 |
| R77-02 | 已验证 | Borda全空票或未打分候选可胜出 | 仅有有效分数的候选参与赢家比较；旧无分数成功回归失败后修复 |
| R77-03 | 已验证 | 验证标记后缀误将bypass/notpass当PASS | 明确末尾独立标记，保留JSON与冒号文本；旧bypass回归失败后通过 |
| R77-04 | 已验证 | Loop iteration及quiet_rounds极值溢出 | iteration饱和终止、长度先比较后计算窗口；新极值回归，最终Execution93/0 |

| R78-01 | 已验证 | Cron列表/runs旧请求或空身份污染数据/loading | 当前请求所有权、切换清数据/空身份idle；实际hooks回归，Cron97/0 |
| R78-02 | 已验证 | Cron GET快照覆盖期间事件和本地成功操作 | 当前GET的按ID变更合并，保留其他任务及运行时间；时区修复前后归属检查 |
| R79-01 | 已验证 | 大小写平台可绕过Ark无效型号校验 | eq_ignore_ascii_case与协议一致；真实SQLite及运行解析回归 |
| R79-02 | 已验证 | WebSocket根URL重复通用解析校验 | 复用parse_realtime_url、保留专属query限制，增强已有回归 |
| R79-03 | 已验证 | Provider模型保存允许超过删除入口的512字符限制 | R85同时移除System/Workshop删除独有的上限，保留trim/非空及既有保存契约；两条旧实现均失败，HTTP7/0、Workshop model_cleanup3/0；App转发只读确认 |
| R80-01 | 已验证 | Git单文件pathspec扩大范围且大小写索引恢复丢条目 | 字面index/checkout、保留mode和邻居冲突、缺失/目录拒绝；Windows50/0 |
| R80-02 | 已验证 | reset吞index失败继续改工作树，stage将metadata失败当删除 | 错误先返回，symlink_metadata保留dangling link；Windows锁/HEAD回归通过，Unix链接回归未运行 |
| R81-01 | 已验证 | 已完成回执产物校验失败被当无标记超时重试 | 明确agent_artifact_verification_failed且非重试；成功/原provider错误映射回归，Execution95/0 |
| R81-02 | 已验证 | planner全局删除代码围栏破坏步骤文本且重复JSON扫描 | 复用control_steps借用扫描器，普通/调整计划保留字符串内容 |
| R81-03 | 已验证 | resolver无效标签参数/分配及快照替换排序间隙 | 全仓无有效tag调用，保留description能力推导；连续排序回归，删除产物单层转发 |
| R86-01 | 已验证 | scheduler StopTurn/Steer重复持久状态编码和ack/事件发布 | 合并共同尾部，DecisionInput保留独立settlement路径；真实SQLite/启动attempt+注入端口验证失败不ack、相同ID重试与两种事件，Execution96/0 |
| R82-01 | 已验证 | root恢复路径未先拒绝被替换的链接根 | 恢复探测/打开SQLite前检查root；Windows14/0，新增Unix真实链接回归未运行 |
| R82-02 | 已验证 | root物化遗漏缺失package及SQL owner漂移 | package显式缺失报错，runtime capability/skill校验SQL package/version；真实SQLite有效/错误对照及skill单元，14/0 |
| R82-03 | 已验证 | 空目录探测吞掉目录条目迭代错误 | transpose传播错误，不再当非空；同批14/0但无可注入OS迭代错误回归 |
| R82-04 | 部分完成 | root完整阅读后跨进程/崩溃/路径持久化边界 | App bootstrap早于server lock；部分DB错误/取消未等close；marker残片/hot journal、build digest升级、TOCTOU/hardlink/祖先保护与其他平台仍待核对 |
| R83-01 | 已验证 | 需求列表/标签旧请求覆盖与卸载后回调 | 请求归属和重连刷新，过期refresh拒绝；59/0、完整typecheck |
| R83-02 | 已验证 | 看板请求500被后端限为200造成静默截断 | page_size200按has_more加载完才发布，错误/加载可见；跨页非一致快照仍保留 |
| R83-03 | 已验证 | 并发附件上传丢失/恢复已删附件且loading过早结束 | 同步最新受控值合并、上传计数和卸载保护；实际组件回归 |
| R83-04 | 已验证 | 状态菜单/拖放发出非法手动流转及残留拖动ID | 复用同一合法判断，按后端set_status核对；取消拖动清理ID |
| R83-05 | 部分完成 | 需求页面完整阅读后未闭环保存/通知/AutoWork/批量边界 | 抽屉保存与后续GET/重复提交、通知并发和特殊键、AutoWork加载归属、Workspace删除分页/选择/键盘错误，后端部分写入删除保留，R89接续 |
| R87-01 | 已验证 | 调整计划选择retired模型且把superseded图当当前计划 | 沿用revision字段过滤，两个旧回归失败后通过；Execution98/0；复用现有参与者和事件构造 |

## 模块覆盖索引

以下目录各登记一次。Rust 76 个，UI 35 个，共 111 个自动核对边界。

| 模块目录 | 状态 | 已审范围 / 续接范围 |
| --- | --- | --- |
| `apps/desktop/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `apps/web/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/agent/nomi-a11y/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/agent/nomi-agent/` | 部分完成 | R47 SkillTool 的 inline shell、副作用标记与完成证据消费链已核对；R48 ProtocolSink/CLI 共同输出入口已验证。其余 engine/builder/session/工具编排未完成全模块审计 |
| `crates/agent/nomi-browser-engine/` | 待审 | 用户要求 Browser Use 暂跳过；不计作完成 |
| `crates/agent/nomi-browser/` | 待审 | 用户要求 Browser Use 暂跳过；不计作完成 |
| `crates/agent/nomi-cli/` | 部分完成 | R34/R41/R48 已审 CLI 生产文件和命令回归，JSON 所有输出入口失败清理 17/0；MCP connect_all 的取消及 bootstrap 跨 crate 归属等见 R30-02 |
| `crates/agent/nomi-compact/` | 已验证 | R29 全文件及工具输出调用链；CRLF/JSON/TOON/Unicode 修复，54/0+Agent 8/0；Full 有损及首候选块限制见报告 |
| `crates/agent/nomi-computer/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/agent/nomi-config/` | 部分完成 | R32–R33 全生产文件已读；合并/hook/schema/旧 shell 清理验证 184/0；仅余硬迁移并发窗口、历史测试设计剩余核对 |
| `crates/agent/nomi-mcp/` | 部分完成 | R36 HTTP/SSE 生产代码及定向测试已审；122/0、Agent/CLI check 通过。stdio、manager 后半部、tool_proxy 未完整审计；其他限制见 R36-04 |
| `crates/agent/nomi-memory/` | 部分完成 | R35 全生产文件/现有测试及调用已读；150/0+Agent 17/0；路径碰撞/多文件写入和非协作边界 R35-07 保留 |
| `crates/agent/nomi-protocol/` | 已验证 | R30 命令/事件/读写全文件及测试，49/0；stdin 有界、标准 stdout 整帧锁；OS stdin 阻塞与调用方 R30-02 不冒充已解决 |
| `crates/agent/nomi-providers/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/agent/nomi-skills/` | 部分完成 | R38–R40 全生产与既有测试已读；R47 inline shell 副作用判断调用链已修（两项红→绿），本批 Agent 94/0 + Skills shell 53/0。剩余容量/加载/MCP 等见 R38-03 |
| `crates/agent/nomi-tools/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/agent/nomi-types/` | 已验证 | R31 全模块及测试/四类 provider 调用已读；描述字符与 CRLF 修复、重复测试删除，70/0；provider 定向 2/0 |
| `crates/backend/nomifun-agent-contracts/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-agent-control-plane/` | 待审 | R1 仅回归通过，未独立深审 |
| `crates/backend/nomifun-agent-domain-support/` | 已验证 | R37 全生产文件、C7 表、8 项现有测试及 App 两套装配已审；Kernel 按 role/mount/capability 校验绑定，不重复造校验层。假成功/重复构造/无调用辅助函数已清理；8/0、App check 通过，未新增完整调用夹具 |
| `crates/backend/nomifun-agent-domain-wave1/` | 部分完成 | R57 唯一 lib.rs 全部生产/测试、6 包/25 capability/14 operation 和 App/Kernel 调用已读；8/0，生产净减37行；Gateway声明/宿主未装配/预算等见 R57-03 |
| `crates/backend/nomifun-agent-domain-wave2/` | 部分完成 | R67非Browser声明/handler/测试完整已读，32能力/21action；严格workspace Kernel输入修复、13/0（跳过2 Browser）；typed dispatcher/Computer Context/缓存及宿主待办R67-03 |
| `crates/backend/nomifun-agent-domain-wave3/` | 部分完成 | R58 全3147行lib.rs及五个App适配器已读，4包/18action；10/0，统一对象输入和错误码，schema/helper及弱测试去重；输入/目标/provider权限契约 R58-03 保留 |
| `crates/backend/nomifun-agent-domain-wave4/` | 部分完成 | R61完整已读、20/0；输入/重复构造生产-57。R70修App Channel policy锁顺序，App17/0；Robot激活/其余资源Context契约R61-03仍待办 |
| `crates/backend/nomifun-agent-domain-wave5/` | 部分完成 | R62唯一文件和全测试、五包21capabilities十action已读，11/0；typed error/对象边界/Remote descriptor已修，生产-12；宿主未接线与运行时关闭保证R62-03保留 |
| `crates/backend/nomifun-agent-execution/` | 部分完成 | R73/R77/R81/R86/R87既有覆盖，engine本轮补读2082至末尾，scheduler3018–3656和4089至末尾；98/0。历史计划隔离已修，R73-03跨边界/timeout等剩余问题不关闭 |
| `crates/backend/nomifun-agent-kernel/` | 部分完成 | R1-01/02：registry 释放与句柄校验已修复验证；获取/关闭并发、其余注册/解析待审 |
| `crates/backend/nomifun-agent-platform/` | 待审 | R1 仅回归通过；本轮将追踪 shutdown 与资源回收 |
| `crates/backend/nomifun-agent-session/` | 部分完成 | R1-03：store 观察快照已修复验证；其余写入/迁移/压缩待审 |
| `crates/backend/nomifun-ai-agent/` | 部分完成 | R20 URL 公共边界、R23 Bearer 分词已验证，send_error 38/0；其他业务路径待审 |
| `crates/backend/nomifun-api-types/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-app/` | 部分完成 | R12 停止取消/物理清理确认、R13 自动应用嵌套租约已验证（App 3/0）；其余入口/路由/业务待审 |
| `crates/backend/nomifun-assets/` | 已验证 | R28 全部生产文件/测试及 App/URL 调用核对；删除 state 包装、修复缓存，10/0+App 2/0；静态 SVG 仅危险标记扫描，见报告 |
| `crates/backend/nomifun-auth/` | 部分完成 | R42/R44 全 16 个生产文件及大部分测试已读；续期/QR/Conflict/CSP 修复后 251/0。认证并发、密码事务和信任策略等 R42-04 尚未闭环 |
| `crates/backend/nomifun-browser-platform/` | 待审 | 用户要求 Browser Use 暂跳过；不计作完成 |
| `crates/backend/nomifun-channel/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-chat-model-broker/` | 审计中 | R84 Hypatia独占src/tests，主线程验收共享调用边界 |
| `crates/backend/nomifun-codex-runtime/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-common/` | 部分完成 | R59 完整 zip_safe.rs及预算所有调用已读；合并三导入器写前预算，11/0，调用方回归52/0；其余common文件仍待审 |
| `crates/backend/nomifun-companion/` | 部分完成 | R59/R63 export.rs解压预算及临时目录所有权已修，定向26/0；其余export/全模块未深审，后续业务取消事务见R60-03 |
| `crates/backend/nomifun-conversation/` | 部分完成 | R4 已核对 list_messages 的 owner 校验、游标解析和 keyset 排序契约；其余 service、运行时/发送/权限路径待审 |
| `crates/backend/nomifun-creation/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-cron/` | 部分完成 | R68完整19文件/18737行已读；溢出、busy原子取消释放、终态shutdown、skill重试修复，全包260/0；生产-28测试-234，DB/文件/检测任务/embedded receipt剩余R68-06 |
| `crates/backend/nomifun-customer-service/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-db/` | 部分完成 | R4分页、R52Webhook/tag_setting原子更新已验证；R66 settings trait/repo/tests完整已审并改原子patch/RETURNING，6/0；其余仓储/事务/配额/查询计划待审 |
| `crates/backend/nomifun-file/` | 部分完成 | R75完整20文件11901行、348/0；R80单文件字面路径/错误传播已修，snapshot50/0。R75-08未闭环；Unix链接回归未跑 |
| `crates/backend/nomifun-gateway/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-idmm/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-js-authoring/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-js-host/` | 部分完成 | R2–R13 已记录子范围验证，最新跨层 121/0；在途 Mount 查询已修复；提交后持续准入及 JS 其余入口待审 |
| `crates/backend/nomifun-js-kernel-adapter/` | 部分完成 | Host 句柄实例/代际及 opaque lease 传递、release 已核对；删除构造后立即丢弃的 identity；R5 跨层 77/0；注册映射其余待审 |
| `crates/backend/nomifun-js-runtime/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-knowledge/` | 部分完成 | R59/R60/R63 export.rs全文件及相关入口已读，ZIP/安全导出/流复制/导入临时归属修复，export13/0；其余service待审，未决R60-03 |
| `crates/backend/nomifun-mcp/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-miniapp-platform/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-model-invoke/` | 部分完成 | R20 URL 去重/错误响应已验证；4 线程全量 396/0，默认并发热点 R20-02 未定位；调用/适配其余待审 |
| `crates/backend/nomifun-office/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-plugin-platform/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-plugin-service/` | 待审 | 已追踪 commit_fence/auto_apply 调用；提交→注册发布→Kernel dispatch 全程准入仍待深审 |
| `crates/backend/nomifun-public/` | 部分完成 | R45 全 5 个源文件/内联测试及相关调用已读；预检/取消占位/归属/refill 修复后 29/0；rmcp 在途处理容量、Host shutdown 与 observation 全量投影见 R45-04 |
| `crates/backend/nomifun-realtime/` | 部分完成 | R53 全 9 Rust 文件及调用已读；socket/heartbeat 生命周期修复，102/0。认证用户有效状态、App shutdown、阻塞写入时 policy Close 和容量边界见 R53-03 |
| `crates/backend/nomifun-requirement/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-robot/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/backend/nomifun-runtime/` | 部分完成 | R49 全 11 原始 Rust 文件及调用已读；R55 目录/解析重复缓存红→绿，38/0；删除无业务测试。构建缓存、非 Linux Unix 探针、版本排序和资产映射仍见 R49-05 |
| `crates/backend/nomifun-shell/` | 部分完成 | R50全源文件及集成/调用已读；R65 Windows路径移出cmd命令文本，整包96/0；GUI/UNC未验、hand-off归属R50-04保留 |
| `crates/backend/nomifun-skill-library/` | 部分完成 | R54 全 17 Rust 文件/7463 行及外部调用已读；传输重叠/预检/链接冲突/ZIP预算/配置发布修复，160/0、2 外网 ignored；资产未逐个审，跨模块/原子发布限制 R54-05 保留 |
| `crates/backend/nomifun-ssh/` | 部分完成 | R25 glob 与搜索/列表错误语义已验证，后端单元 30/0；R25-03 名称协议、pool/service/routes 等仍待审 |
| `crates/backend/nomifun-system/` | 部分完成 | R66/R72/R76/R79已读范围；R85关闭R79-03长模型删除，HTTP7/0、Workshop3/0；R79 System187/0为上批证据。R72-04/R76-04及其他源码仍待审 |
| `crates/backend/nomifun-terminal/` | 审计中 | R88 Mencius独占src/tests，Execution/App在其冻结前不编译；root/broker可独立验证 |
| `crates/backend/nomifun-v4-root/` | 部分完成 | R82全8文件读完，修恢复根/物化归属/目录错误，14/0；Unix链接未运行，App调用方和R82-04剩余风险保留 |
| `crates/backend/nomifun-webhook/` | 部分完成 | R46 全 7 源文件与 2 集成测试/调用边界已读；R52 原子部分更新与父项检查红→绿，Webhook 25/0 + DB webhook_repo 6/0；通知归属与取消/出站策略仍见 R46-05 |
| `crates/backend/nomifun-workshop/` | 待审 | 未深审；按入口→状态归属→调用方→错误/关闭路径检查 |
| `crates/shared/nomi-process-runtime/` | 部分完成 | R11 只读清理凭据 accessor 和关联 Drop/shutdown 边界已核对，23 项边界验证；平台/会话/输出等其余仍待深审 |
| `crates/shared/nomi-redact/` | 已验证 | R14 完整公共模式脱敏模块；6 项红→绿，模块 16/0、浏览器调用方 20/0；阈值与 best-effort 限制见报告 |
| `crates/shared/nomi-ssh/` | 部分完成 | R24/R27 文件发布与流边界、R26 解码/目录/预算/已取得通道归属已验证，56/0；connection 认证/transport/channel-open 取消及测试支持仍待审；真实 sshd 未验 |
| `crates/shared/nomifun-net/` | 已验证 | R15–R22 完整模块审计，net 66/0、1 子进程夹具 ignored；原生 macOS/Linux 未运行及支持范围限制见报告 |
| `ui/src/common/adapter/` | 部分完成 | R43 只追踪桥接/dialog 及 ipcBridge 更新接口，删除失效下载 stub/noop；原生更新 helper 回归包含于 UI 3304/0。其余适配入口、请求取消/超时仍待审 |
| `ui/src/common/browser/` | 待审 | 未深审 |
| `ui/src/common/chat/` | 待审 | R2-06 仅统一测试导入并回归；业务未深审 |
| `ui/src/common/config/` | 部分完成 | R51全8文件及调用已读；R56补离线→空配置就绪通知，hook/config调用方定向68/0；服务端PUT顺序/reset与真实多窗口R51-03保留 |
| `ui/src/common/protocolBindings/` | 待审 | 生成代码：审计生成源与契约，不手删生成产物 |
| `ui/src/common/types/` | 部分完成 | R1-07 弃用类型删除；R2-06 测试类型整合已验证；其余类型/契约未深审 |
| `ui/src/common/update/` | 部分完成 | R43 已读唯一 updateTypes.ts 及更新 UI/adapter 调用，删无生产来源的手动下载类型。includePrerelease/repo 请求字段与 native provider 的实际支持范围、非 Tauri 路径仍待核对，不冒充整体更新策略已验证 |
| `ui/src/common/utils/` | 部分完成 | R1-07：部分弃用类型/工具删除验证；其他实现待审 |
| `ui/src/platform/` | 部分完成 | R43 三个生产文件及桥接调用已读；同步发送抛错监听器泄漏红→绿，定向 3/0；删无调用 subscribe/颜色 token。异步 transport 未响应、adapter 替换及并发 dialog 所有权仍待跨层审计 |
| `ui/src/renderer/assets/` | 待审 | 未深审 |
| `ui/src/renderer/components/` | 部分完成 | R1-07/R2-05 旧组件与 Provider 已核对；R43 删除 UpdateModal 的失效手动下载/完成状态，保留 Tauri 下载/安装和外链。侧栏仅修旧路由断言，不计 MiniApp 深审；其余组件仍待审 |
| `ui/src/renderer/hooks/` | 部分完成 | R1-08 useSendBoxDraft；R56主题/色彩/字号完整已读，68/0；R71 AuthProvider已读并修取消refresh/非Error，Auth/Login32/0；跨操作竞态R64-03和其余hooks保留 |
| `ui/src/renderer/services/` | 部分完成 | R1-07 旧 TtsService 删除；R2-06 matting 仅统一测试导入；其他服务待审 |
| `ui/src/renderer/styles/` | 待审 | 未深审 |
| `ui/src/renderer/utils/` | 部分完成 | R2-05 createContext 已审并验证，核对 HOC 装配但未改写 HOC；其他工具模块待审 |
| `ui/src/renderer/pages/agentSession/` | 待审 | 未深审 |
| `ui/src/renderer/pages/agentSettings/` | 待审 | 未深审 |
| `ui/src/renderer/pages/browser/` | 待审 | 用户要求 Browser Use 暂跳过；不计作完成 |
| `ui/src/renderer/pages/companion/` | 待审 | R2-06 仅统一测试导入并回归；业务未深审 |
| `ui/src/renderer/pages/conversation/` | 部分完成 | R1-07 旧订阅/fence 删除；R2-02 只读 hook/steering、R2-05 Provider、R3-02 批处理、R3-03 历史分页已验证；发送/流/其余状态待审 |
| `ui/src/renderer/pages/creativeStudio/` | 部分完成 | R1-05/07 CAS 撤销保存修复、旧 Projects 删除；R2-06 部分测试导入统一；Canvas 路由/资产/执行待审 |
| `ui/src/renderer/pages/cron/` | 部分完成 | R74完整32文件4491行；R78列表/runs归属及事件合并已修，Cron97/0及typecheck；R74-05其余未闭环 |
| `ui/src/renderer/pages/customerService/` | 待审 | 未深审 |
| `ui/src/renderer/pages/guid/` | 待审 | R43 仅对照 HEAD 核对官方 Agent 发起链并修旧结构断言，不计完整模块审计 |
| `ui/src/renderer/pages/knowledge/` | 待审 | 未深审 |
| `ui/src/renderer/pages/login/` | 部分完成 | R64页面/CSS/测试完整已读；提交/卸载/重定向/存储失败修复，UI3381/0；R71同步非Error网络错误断言，Auth/Login32/0/typecheck；R64-03仍保留 |
| `ui/src/renderer/pages/mcp/` | 部分完成 | R69三文件完整已读；页面解析/确认/卸载/初次加载修复；旧4/11、现15/0，全UI3398/0/typecheck；共享hook/市场/后端事务等R69-03未修 |
| `ui/src/renderer/pages/miniApps/` | 待审 | 未深审 |
| `ui/src/renderer/pages/modelHub/` | 部分完成 | R1-04/09：串行队列恢复、健康规则合并；目录/模型刷新/偏好待审 |
| `ui/src/renderer/pages/nomi/` | 待审 | 未深审 |
| `ui/src/renderer/pages/openCapabilities/` | 待审 | 未深审 |
| `ui/src/renderer/pages/plugins/` | 待审 | 未深审 |
| `ui/src/renderer/pages/requirements/` | 部分完成 | R83原38文件全读，59/0及typecheck；R89接续R83-05，尚未完成全部错误生命周期闭环 |
| `ui/src/renderer/pages/settings/` | 待审 | 未深审 |
| `ui/src/renderer/pages/terminal/` | 待审 | 未深审 |

## 跨切面与非模块目录

下表补充目录清单之外的入口，不与上表重复计数。

| 范围 | 状态 | 续接范围 |
| --- | --- | --- |
| 根 package.json / bun.lock | 部分完成 | R1-06 已验证；其他依赖、脚本入口待审 |
| 根 Cargo.toml / Cargo.lock / .cargo | 待审 | feature、版本、构建配置与平台差异 |
| ui 根配置、src/renderer 及 pages/common 根文件 | 部分完成 | R1 检查路由/动态图标引用；其余初始化/错误边界待审 |
| ui/package.json / 测试运行时类型 | 部分完成 | R2-06 已验证；非测试依赖与配置待审 |
| ui/test、ui/public、全局样式与静态资产 | 待审 | 测试装配、动态加载、资产引用；不按 TS 图直接删除 |
| scripts / packaging / Docker 配置 | 部分完成 | 仅本轮 check-review-inventory 的正常/缺失/重复/过期清单已验证；原构建/打包/进程管理脚本待审 |
| docs / 仓库治理配置 | 部分完成 | 审计记录已整理；架构文档已观察到过期计数，其他待核实 |
| vendor | 不修改 | 第三方源码；仅审计自有调用边界，避免手工改 vendored 依赖 |
| target / build.noindex / dist / node_modules / 日志 | 排除 | 生成/缓存/运行产物不作为自有源码；不做无关清理 |

## 验证流水

- R1：`bun run check`、`bun test --cwd ui`（3288/0）、`bun run build:ui`、4 个 Rust crate（104/0）、`git diff --check` 均通过；详见首轮记录。未跑全 Rust workspace、桌面包、macOS/Linux、真实外部服务。
- R2 清单：`bun scripts/check-review-inventory.mjs` 通过（111）；内存注入缺项、重复项、过期项均返回失败，不修改台账做负例。
- R2-01a：新增并发首次需求回归修复前失败，错误为 Mount 身份冲突；修复后 `cargo test -p nomifun-js-host -p nomifun-js-kernel-adapter -p nomifun-agent-kernel` 54/0，通过。
- R2-01a：三个新增并发用例额外连续执行三轮，每轮 3/0。
- R2-02/06：真实只读 hook 定向测试 4/0；最终 `bun test --cwd ui` 3287/0（608 文件），`bun run check` 和 `bun run build:ui` 通过。失败夹具与类型方案取舍见 R2 记录。
- R2 最终差异：`git diff --check` 与模块清单检查通过；累计净减少 1958 行源码/测试/依赖清单（含清单脚本，不含文档）。
- R3：四个服务生命周期回归在旧实现失败，修复后通过；最终 Host/adapter/Kernel 59/0，另五个多线程服务测试定向复验 5/0（含 panic payload 不进入 public state）。
- R3：Context 初值与批处理重放回归旧实现失败，修复后定向 19/0；最终 UI 3294/0（609 文件）、check、build 全通过。
- R3：清单核对 111 模块 / 20 唯一问题编号；内存注入重复编号、模块缺失/重复/过期均被拒绝，未写入伪造台账。git diff --check 通过。
- R1–R3 累计：71 个源码/测试/依赖清单/脚本文件，+1308 / -2907，净减少 1599 行（不含文档）；删除 21 个旧文件。未提交，既有 .githooks/ 不变。
- R4：UI 3304/0（609 文件）、check/build 通过；会话仓储集成 91/0、单元 66/0；共享 offset 提取后分页重跑 14/0、需求/素材仓储 46/0。未跑全部 Rust workspace、桌面包或其他操作系统。
- R4：清单仍为 111 个模块边界 / 21 个唯一问题；git diff --check 通过。R1–R4 累计 82 个源码/测试/依赖清单/脚本文件，+1924 / -3138，净减少 1214 行；删除 22 个旧文件，不含文档，未提交。
- R5：新增 18 项真实 Node 回归，其中 12 项有修复前失败证据；最终 Host/adapter/Kernel 77/0、生命周期复验 23/0；三处 Node 语法与进程边界检查通过；未改 UI，未重复全 UI。
- R5：清单核对 111 模块 / 22 个问题、git diff --check 通过。R1–R5 累计 84 个源码/测试/依赖清单/脚本文件，+2706 / -3241，净减少 535 行；删除 22 个旧文件，不含文档，未提交。
- R6：4 个真实 Node 回归先失败后通过，最终新增 13 测试；Host/adapter/Kernel 90/0、IPC 复验 7/0、进程边界及 Node 语法检查通过；未重复无改动的 UI 全量。
- R6：清单核对 111 模块 / 23 个唯一问题，git diff --check 通过。R1–R6 累计 91 个源码/测试/依赖清单/脚本文件，+3316 / -3282，净增加 34 行（新增回归与有界传输导致总量上升），累计删除旧文件仍为 22；不含文档，未提交。
- R7：新增 9 项（4 个红→绿，取消原实现即通过）；定向 9/0，首次跨层暴露测试准备等待过短，修正同步后最终 99/0；进程边界与 Node 语法通过。未跑 UI/全 Rust workspace/其他平台，详见 R7。
- R8：新增 6 项（4 个红→绿），最终 Host/adapter/Kernel 105/0；进程边界、Node 语法、差异检查通过；范围限制和关联 ID 非持久防重放语义见 R8。
- R9：新增 5 项（3 个红→绿），最终跨层 110/0；进程边界重跑、Node 语法、差异检查通过，详见 R9。
- R10：新增 5 项（1 个红→绿），文件定向 5/0、最终跨层 115/0；清单与差异检查通过，详见 R10。
- R11：新增 3 项（2 个红→绿）并扩展启动取消后重试；跨层 117/0、底层 child/架构边界 23/0、进程边界通过，详见 R11。
- R12：App 新增 2 项红→绿、Host 新增 1 项并扩展 2 项证明入口回归；App 2/0，跨层 118/0，详见 R12。
- R13：2 项红→绿、2 项查询排队/超时回归；App 3/0、跨层 121/0，详见 R13。
- R14：6 项红→绿，nomi-redact 16/0、浏览器脱敏调用方 20/0，详见 R14。
- R15：新增 5 项（4 项红→绿），nomifun-net 43/0、知识库抓取 21/0，详见 R15。
- R16：2 项红→绿，另启用 2 项跨平台纯解析回归；代理 26/0、net 47/0、进程边界通过，详见 R16。
- R17：后代持管道红→绿，新增 3 项行为测试；net 50/0、1 子进程夹具 ignored，进程边界通过，详见 R17。
- R18：缓存并发/完成后 TTL 两项红→绿；net 53/0、1 子进程夹具 ignored；清单和差异检查通过，详见 R18。
- R19：精确匹配/截断 4 项红→绿，1 项穷举 oracle；net 58/0、provider 19/0，详见 R19。
- R20：URL 公共边界 3 项红→绿；net 61/0、provider 19/0、模型定向 15/0、Agent 36/0；模型全量未完成单列待办，详见 R20。
- R21：编码/截断 3 项红→绿，另增保真回归；net 65/0、provider 19/0、模型响应 4/0，详见 R21。
- R22：网络模块收尾，net 66/0、1 子进程夹具 ignored，进程边界/清单/差异通过，详见 R22。
- R23：Bearer 真实入口红→绿，send_error 38/0；另验证 model-invoke 4 线程全量 396/0，默认高并发根因待审，详见 R23。
- R24–R30：SSH、assets、compact、protocol 的命令/结果和未运行项见各批报告；R28 assets 10/0+App 2/0，R29 compact 54/0+Agent 8/0，R30 protocol 49/0+CLI check 通过（R31/R32 前）。
- R31–R32：types 70/0、provider deferred 2/0；config 定向 66/0（含四项红→绿）。未重跑无关 UI，配置模块尚未全审。
- R32–R33：合并四项、hook 一项、schema 两项红→绿；config 最终 184/0，provider schema 7/0、Agent hook 2/0。CWD 测试假设修正及未运行项见报告。
- R34：CLI 最终 12/0，新增四项配置/真实 hook/活跃本地请求生命周期回归；没有旧红→绿，未验证 OS stdin/broken stdout/MCP 清理故障，见报告。
- R35：九项红→绿；memory 150/0，Agent 单元 10/0、context 集成 7/0；ai-agent/companion check 通过，平台/文件系统限制见报告。
- R36：MCP 最终 `cargo test -p nomi-mcp -- --test-threads=4` 122/0，`cargo check -p nomi-cli -p nomi-agent` 通过；前五项旧失败，后续 UTF-8/生命周期/重定向仅记修复后回归。未运行外部 MCP/原生 POSIX。
- R37：`cargo test -p nomifun-agent-domain-support -- --test-threads=4` 8/0，`cargo check -p nomifun-app` 通过；现有测试验证声明/元数据，未新增完整执行夹具。
- R38：两项复用测试旧失败；修复后 430/0，再删除 23 项重复用例后最终 skills 407/0；`cargo test -p nomi-agent --lib skill_tool -- --test-threads=4` 59/0。本轮未跑 UI/全 Rust workspace/其他操作系统。
- R37–R38 行数：六个修改文件合计 +81/-484，源码与测试净减 403 行；MCP 收尾测试另增 1 行。无新框架、新依赖或新测试文件；保留未提交状态。清单检查 111 边界/86 唯一问题，差异空白检查通过。
- R39：六项回归先失败后通过（四项新测试、两项扩展原测试）；随后增加正常目录链接保真验证。`cargo test -p nomi-skills -- --test-threads=4` 最终 392/0；`cargo test -p nomi-agent --lib skill_tool -- --test-threads=4` 59/0。调用回归后仅调整技能测试，无生产改动。未跑全 UI/全 Rust workspace/macOS/Linux。
- R39 行数：相对本批开始的 8 个源码/测试文件净减 280 行，其中生产段净减 87 行、测试及测试注册净减 193 行；无新测试文件，删除 permissions_supplemental_tests.rs（断言已合并，可从 Git 恢复）。保留 R1–R38 和并行 MiniApp 改动，未提交。 清单核对 111 边界/90 唯一问题，git diff --check 通过；无关并行文件仅有 CRLF 提示。
