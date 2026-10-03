# 命令与会话可靠性共享进度

更新：2026-10-04。调度规则见 [实施计划](IMPLEMENTATION-PLAN.zh.md)。

## 2026-10-04 用户新增：全局模型配置不降能力

本批不是恢复旧全产品 Case 队列。范围为当前六个 Chat 协议族、32 个媒体/语音/检索
one-shot adapter 和一个已注册 realtime adapter 的配置→保存→切换→实际请求合同。

- 默认不注入 32K 上下文或 4K 输出；未声明上下文保持未知，以真实 typed 过长拒绝恢复。
  输出配置只作用于实际 attempt，未使用的小备用不压主模型，小主模型也不压大备用。
- 目录声明的输出上限及字段来源贯通导入。Gemini `inputTokenLimit`、Anthropic
  `max_input_tokens` 标为 input-only，不误扣独立输出上限；本地语义标记不发供应商。
- 显式数值/默认、traits、推理和其他参数不被协议切换、异步目录建议或完整保存覆盖；
  伴侣、标题、知识、health、planner、template、Creation/one-shot 不再加内部小输出常量。
  管理配置包括缺省均优先于 `.nomi.toml`。必填输出协议缺少官方声明或显式配置则明确报错，
  不猜 4096。模板/Wave3 不再以 32K/128K 作为平台上限。
- 官方 `none/minimal` 与未设置严格区分；原生 thinking 保真、不可表达的控制明确失败。
  单一 effort value 被拒不能写成整个 Reasoning 不支持。011 是新增前向迁移，旧迁移
  校验和及用户数据库不改；旧推理值、默认、fork 与删除边界保留。
- 长媒体 prompt 不再偷偷裁为 512 字符或改意图重试；Agnes 已选尺寸、TTS 已选 voice/format
  不被 fallback 覆盖，OpenAI 4096 字符限制不再套给其他 TTS 供应商。

官方边界参考：[OpenAI Chat](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)、
[Gemini Models](https://ai.google.dev/api/models)、[Anthropic Models](https://platform.claude.com/docs/en/api/models/retrieve)。
已保存的旧数值来源不能证明是平台默认时不自动清除；用户明确限制仍保留。字节/历史投影/
超时/授权保护仍是独立资源策略，不冒充模型官方规格。逐供应商 paid live、Windows native、
release 全规格不在本批认证范围，其他平台验收历史不改。验证及证据见 macOS 本批记录。

## 既有命令/会话收敛范围及平台结算

2026-09-30按用户明确目的收敛为[八个命令问题簇、三组正式会话](IMPLEMENTATION-PLAN.zh.md)。
本轮处理简单命令、步骤衔接、过程状态及结果可信性；通用/编程入口各保留真实执行证据。
旧675共享＋82 Windows全产品队列停止排程，原目录/首败/已修代码保留；范围外、手测和复用分开记录。
核心共同链路达标后单独通知，再补其中尚缺的Windows命令/native断言，不以全产品发布认证阻断收尾。

2026-10-04用户明确跨盘/真实UNC先跳过，保留未验。Windows e869aa532正式Tauri/Computer Use
已核对当前Turn插入后旧2048/low冻结、下一Turn默认wire省略/high刷新两项；canonical预算与实际
请求一致。夹具强制报告响应错误和响应上限首败保留，完整报告链不计PASS，W276旧原因不外推。
成功恢复在输入前因cold-launcher PowerShell环境停止，Turn/effect0、未故障/冷启，仍未验；B/D
仅准备候选，既有stdin/Stop/历史真值/Creative/W274托盘Quit复用。完整共享/Windows/发布未认证。

2026-10-04 mac近期四目标收尾：原A13完整11项交付、B09原效果/字节/步骤及最新33来源
报告事实与中文表达独立PASS、真实Keychain等待下有界exit1证据复用。最新23788B报告同
projection正式零模型冷读/必要结果可达/正常exit0(31.924秒)成立，2156events/27Turns/
14effects逐行不变。生成run480秒观察器expiry/TERM的FAIL仍原样保留，不能由cold证明洗绿。
配置后warmup停滞/输出默认限制已修并实际采用；不代判Windows、全部旧答案表达、N3或
发布认证。mac停止新增模型与全量走查，详细证据/系统等待及socket缺证边界见mac页。

2026-10-03全局B发现并修复中文结果尾注展示接线：Runtime plain_zh_v2固定尾注与renderer
原英文-only suffix不匹配，合法中文报告返回undefined。最小修复只识别相同中文格式，保留
原文/计数/同Session及Turn回执核对、英文兼容和缺证降级；首回归7PASS/1FAIL保留，修后8/8
及desktop boundary通过。随后5937b5bcd正式Tauri已验证中文非零分类（真实exit1/原输出、宿主
1/1尾注、UI原生非零说明）；首载体因未暴露plan而未执行的前提失败保留。仅此UI子断言成立，
其他分类新UI、历史模型采用、自由表达及复杂整链仍未验，详见Windows页全局B正式呈现。

全局A唯一当前规则正式样本保留FAIL：14个真实Step5请求/SSE、14组原生参数与canonical相同；
初始FIRST+LF、最终FINAL+LF成立，但模型提前承诺的patch无guard、stdin无append_newline，
后续正确guard/LF参数被承诺核对拒绝，改回原错误承诺后执行。无EOF/最终报告，预算暂停后取消；
不改执行层或重跑取绿，不覆盖W250/W249/W240。B修复41a689a00与新历史公开来源的Windows
组件结算见Windows页；C自动采样/D迟到控制载体仅外部候选，正式恢复及新UI采用仍未验。

### 后续全面目标的当前缺口（2026-10-03；整体A/B/C/D并行结算）

原六簇中两项指定采用缺口已有正式证据：W273实际Step5按`/requirements/1/source`拒绝反馈
纠正input1引用；W274人工托盘Quit真实运行中的Turn和三层树，实际exit0/取消一次/reaped162ms，
原记录保持。正常空闲Quit及真实写锁失败exit1复用W259/W264，不把原生API当托盘点击。
这两项是限定子断言闭环；W273严格整链三项生成FAIL、W274首例提前暂停的前提失败保留。

| 仍开放的工作簇 | 具体边界与下一步条件 |
| --- | --- |
| 步骤遵循 | W250漏stdin/重复cursor0；已保留摘要cursor25/input待办，历史请求工具表和拒绝HTTP/SSE无法恢复，唯一根因未证；不靠新正向短链抹除 |
| 精确参数 | W249原参数与实物17 bytes缺LF、历史Windows SSE缺；W240模型未提议用户要求的来源SHA guard；没有已证执行层根因，不自动补字节或强制改通用schema |
| 复杂交付与报告 | 原复杂任务漏项/稳定性未关闭；W273仍错用指定引用行、连带改description、completed却报告等待用户，原生提议与canonical相同，保留生成/报告残余 |
| 新历史正文的Windows实际采用 | W277原W250正文组件成立；W278旧Session选择Step5被409非模型合同变更门禁拒绝，正式模型请求0，未送新输入/未重放。共享新来源标记及公开表达修复的Mac证据不代判Windows；typed续接实际UI仍未验 |
| Windows生命周期与成功恢复 | W276推理前停滞、未证Runtime收尾及实际exit1未解释，原盒关闭；C新载体合同3/3，唯一正式样本因自然lease与固定120秒不相容拒绝故障注入，未冷续做。实际暂停后取消/API exit0不代判成功恢复 |
| Windows完整专项边界 | W268强杀清理/受控冷隔离、W270 cmd owner、W271代码页、W272一次真实S3的deadline/lease归约、W274活树Quit及W275实机复杂路径/literal argv/显式管道可复用；W279实际ConPTY尺寸80×24→132×43及清理已验。S3交互ACK首败未解释，第二卷/真实UNC夹具缺、OS重启及各专项其余边界未验 |

表中六簇不是六个Case或新的全量队列，分到A/B/C/D四条工作流并行调查，现场资源统一串行。
未验证项不计PASS。Creative Windows暂停/retry/冷读
直接复用W260；完整共享、Windows认证及发布矩阵仍未完成。下方W258交付表和阶段总结是原
有限交付的历史基线，不由本次子断言改成全面目标complete。

W276补当前共享配置修复的Windows正式采用未达：同实例输入已接受但目标模型/预算未出现，
两个执行组织首错保留；UI Stop取消一次，原生API退出实际exit1且Runtime清理未证明，最终
后代/监听清空。临时公开API载体未复现，未改生产代码；新停滞/清理原因与正式采用保持OPEN，
不改原W274活树托盘exit0，也不把某个组件或其他载体的通过当本例PASS，详见Windows页W276。

本次独立包提交0261d4c99（D尺寸）/e8bebce49（C载体），经ba1391c7a正常合并推送；
A1无新投递根因，纯审查不计修复。B/C GUI与夹具均已自然exit0，后代/监听0，StepFun新请求0。
清理后同步ae3a91c70的旧回答来源/pairing收尾，Windows直接history9/9、pairing timer3/3通过；各正式采用
仍按上表保留，不把同步或组件数计整体闭环，详细首败与覆盖边界见Windows页W278/C结算。

W277在Windows显式使用原W250关闭制品核对最新正文投影：16条结果正文/原binding/turn/error/
顺序完整，原DB SHA保持、EOF仍0，紧凑消息26986bytes且不扩大64KiB上限；control提议参数
只在投影明确省略、精确archive READ保持。仅组件/既有回归证据，无owner或模型调用，不代判
真实模型采用、旧漏stdin/缺LF/来源guard遵循或完整报告通过，整体余项不由此关闭。

2026-10-02用户再次要求尽快聚焦收尾：最新实施计划已删除B→C→A整组/N3活动门槛，改为具体缺陷
定位、直接修复、最小回归及必要的一次短UI。复用已验子链，暂停纯说明微调/旧矩阵扩展；
同一未改根因不付费循环，阶段交付、生成精度残余与未认证项分开，不将旧FAIL改记PASS。

2026-10-02用户要求整体复盘和提速方案：实施计划已固定最后四个工作包，机制结算、生成残余
归类、Windows共用退出边界及一次最终交付，预计1～2小时有效工作。该估计不表示共享/Windows
全部Case通过；旧缺证、复杂生成失败和平台阻断保持。单主agent、复用已验夹具和证据，无新根因
不新增模型/整组测试，诊断时间盒不可通过新批次重置。本次只设计/更新计划，无模型调用或产品改动。

用户随后将目标明确为上述四包的阶段收尾，允许Windows退出不可达时保留阻断并人工交接。
四包已按W258证据完成结算，单份[阶段交付总结](DELIVERY-SUMMARY.zh.md)汇总已验机制、开放问题及
退出人工步骤；本次目标按该交付范围完成，完整共享/Windows认证及所有原失败状态保持。

### macOS 阶段性 P0 交付（2026-10-02，用户选择方案 1）

用户选择先交付已修且已验证的产品机制，生成精度残余后续安排。macOS 本轮自动走查收尾：
C06-15 历史命令结果策略 `ace29cdd0`（3 项回归/正式两命令 N1）、C05-09 退出失败码
`6f1aaa270`（12 项回归/正常冷读）、C05-10 CEF 按需生命周期 `a3a0bf001`（4 项回归/
正式未用及首次使用）已交付。B09 原始模型参数缺 LF/步骤错误仍开放，产品字节/恢复保障
4/4 不代判生成通过；复杂 A 漏项、已用 CEF Keychain 等待/失败分支 live、旧 MM/N3/发布
认证等保留为后续未解决或未验项。详见 [macOS 阶段交付记录](PROGRESS-MACOS.zh.md)。
本项不关闭共享全范围验收、不改写 Windows 结果或改变其排程；本次仅文档，无新增模型调用。

macOS 后续快速方案已设计，见 [四簇短队列及停止线](PROGRESS-MACOS.zh.md)：
R1 字节/步骤、R2 多项结果交付（并入默认 GEN）、R3 旧 MM 实际 retry、R4 已用 CEF/失败退出。
先复用证据定位，直接修复 + 最小回归，至多一次针对性正式验证；不重跑全 A/B/C/角色/N3。
本次仅设计、未执行/未增加模型请求，不改共享全范围门禁及 Windows 结果或排程。

macOS 已执行 R1/R2 短子链：R1 5请求/精确写改读N1；R2新增完整默认General fixture opt-in，
不再删Computer/自动化模块，补正式必需资源选择，4项回归和正式Tauri/StepFun4请求的三结果
交付N1。实际15模块/Skill与官方manifest全同；旧减权模式保持、无系统权限改变。此前422
预检和A13/B09首败保留，不关闭原完整A/B/N3或Windows结果。详见macOS进度；共9个新请求。

macOS R3发现并修复Creative链路丢弃canonical暂停：复用严格pause判据，非终态消息明确暂停，
保留pending/停止确认、不自动重发或取消；41项直接回归及正式HTTP暂停→用户Stop通过。
另外以debug指定UUID的一次回执丢失单独进入实际失败消息retry，真实Tauri点击后原key/
Turn/operation仅一个、模型仅一次，冷读不重发；付费0，首个native/冷观察超时仍FAIL保留。
仅隔离等价negative N1，不代判Windows旧数据/N3/全MM。详见macOS R3记录；Windows结果未改。

macOS 四簇快速方案执行/交付记录已齐：R1/R2真实短链共9个StepFun请求，R3产品pause修复/
实际retry及冷读negative N1，R4核对未漂移CEF/退出源码、复用正式已用Browser119ms ack并补
当前退出/清理边界3项。Keychain系统等待及native失败分支live仍OPEN/未验，不改记PASS。
此为限定方案的交付，不声明全部体验问题、原阶段二三、N3或共享全范围达标；残余见macOS
最终状态表。R4无新模型/native运行或公共源码改动；Windows原生结果与排程保持。

## 核心机制交付表（W258核账；子链证据）

| 簇 | 已验机制与直接证据 | 保留边界 |
| --- | --- | --- |
| C01 命令选择与参数 | W245实际cwd/Hidden命令一次；W255当前cwd/完整九条目及真实Hidden、首次matching报告20项 | 特定合法命令及交付N1；原复杂A首败保留 |
| C02 读取与搜索 | W245完整样本/两搜索各一次；W255中文空格文件一次全文、首尾/43 bytes/完整SHA | 只读事实与短交付已验，长任务漏项稳定性未认证 |
| C03 Git观察与小测试 | W245 status/diff及指定两测试各一次、exit0/1、索引/文件不变、哨兵未执行 | 实际执行/非零交付可复用，原首report拒绝未消失 |
| C04 文件与步骤结果 | W209三正式文件链各31/31；W250实际来源SHA/末LF/保护原件/最终回读；W210摘要单位 | 精确效果及保护成立；W240未带要求的来源guard仍开放，健康子链不代判全部模型遵循 |
| C05 进程与停止 | W257六组原生参数/实物18/EOF18/exit0/推进游标；W248/249 Stop前父子存活、385/536ms消失 | 当前短链与Stop成立；旧漏stdin/重复cursor仍开放；W259空闲托盘、W264活动原生API exit0及真实写锁失败exit1已验，其他退出/完整平台认证未验 |
| C06 过程与交付真实性 | W245业务非零；W230冷读/原回执分类12项；W254历史EOF0/当前读取13项；W255三结果交付 | 冷读和特定交付子链已验；新混合结果/长任务生成与全UI未认证 |
| C07 连续会话与纠正 | W242纠正身份冷读；W248/249同Turn纠正/实际压缩/Stop、取消冷读；W253历史保留在W254正式采用 | 不复活/不重做的已验链复用；C原17-byte缺LF、引用反馈采用及完整长会话未全部达标 |
| C08 模型协议接合 | W257真实7请求/SSE，六原生调用参数与canonical/owner结果相同，含一次压缩；GEN/COD已有正式样本 | 普通生产网络拓扑、旧拒绝帧及各类生成准确性不由这一个样本认证 |

本表只列候选复用与缺口；是否受新代码影响需核对具体基线，不把源码测试存在当已跑通过。
W258已读取12份独立断言产物并核对当前差异：b1c21a0a5之后Windows核心Runtime/Session/Broker/
File/Router/Terminal源码无变化；后续desktop变更为debug精确key的回执丢失夹具，Creative与macOS
变更单列。W209以来文件执行实现保持，仅instruction_scope增加识别元数据/说明，后续W250文件链
亦成立。因此复用上述指定断言，不把表格的8行计成8个完整Case通过。
本轮按具体核心缺陷及独立证据交付；原A/B/C/N3保留历史/认证状态，不再作为整组复跑门槛，
也不再按原Case×角色×OS计算完成率。W254/255关闭历史真实性和多项结果交付的短链，生成步骤/字节
残余及Windows托盘退出另列，尚不宣称共享全阶段达标。

### 剩余处理边界（W258归类）

- W250漏stdin/重复cursor：真实调用和实物偏差已证；旧拒绝输出/实际工具表缺失，唯一原因待证。W259核对原seq564摘要及保留回执均含cursor25，seq576完成提议仍为0；该次压缩未完全丢失游标，实际HTTP/SSE仍缺。W257新短链PASS不覆盖旧FAIL，不再补相同正向样本。
- W249末LF：原ToolCallCompleted已是17-byte无LF，owner/实物相同；Windows原SSE未保存，不把macOS供应商结论移植到本机，不自动补字节。
- W240来源SHA及原复杂交付：产品schema允许无expected_source，但用户本次明确要求guard；提议未遵循继续开放。后续正确源保护/短报告不证明复杂任务稳定性。
- Windows正式退出：W258原BLOCKED_UI_REACHABILITY保留；W259人工空闲托盘Quit自然exit0及原记录/最终清理N1已验。W264当前正式Tauri已有原生API自主退出，活动Turn正常exit0与真实写锁清理失败exit1分别通过；后者未证清理时不报成功。冷恢复未重放已完成写入，本地模型复用旧call ID被拒绝，恢复完成未验。活动托盘点击/其他失败分支及全角色/N3/100 seed/LONG/99%未认证，协调器组件规则复用。

机制核账、残余归类、Windows退出边界和阶段交付已结算，原失败不改PASS；尚未满足的实际体验要求仍开放。
外部2026-10-02/windows/w258-core-delivery保存所读产物hash/选定断言/源码差异和归类，
没有新模型、测试或产品修改。首次读错W255断言集合字段已纠正并留档，原产物未变。
正式零模型冷实例另核对三表/文件/无活动Turn共5项成立，既有记录无复活。原10分钟退出观察
到期后复核同一PID仍活，延续同一进程至15分钟而未重启；没有实际Quit证据。自有夹具已清理，
本轮核心机制交付可用，不宣称共享全阶段或Windows全部Case完成；后续恢复该断言须有可操作托盘的环境/人工。

后续W259已按上述人工条件取得新实例正常Quit证据，详见Windows页对应记录及
外部2026-10-02/windows/w259-open-items-closeout；本次无产品修复/模型调用，旧退出阻断不改写。

用户后续已开启剩余共享/Windows全面走查的新目标，旧有限交付不代判该目标完成。W260使用
当前正式Windows产物验证Creative HTTP暂停→实际Stop、精确提交回执丢失→实际retry→同数据
冷读：34项直接回归/桌面边界通过；两个本地模型调用，分别28/30events、0effects，retry原key/
Turn/operation一个，冷读四表整行hash及模型计数保持。三个GUI均由用户托盘Quit实际exit0、
最终后代/监听清理；原夹具/观察器首错单列。无新产品源码，原旧MM及生成/平台/发布余项保持，
不把隔离negative N1计为真实StepFun生成或全MM通过，详见Windows页W260。

W261尝试W249精确引用反馈的真实StepFun采用，未达目标：初始控制工具按现有adaptive合同未
暴露，错误的“先update_plan”载体两请求无进程后保留；修正前提后，原生模型仍在追加输入前
自行cancel/收尾，未形成input1或目标拒绝。9个新真实请求/完整SSE、5组参数与canonical相同，
实际请求明确保存等待/不取消notice、process ID/cursor，工作区保持、helper清理。无新产品修复，
新生成首败及操作者未及时提交steering的边界保留，不覆盖W249采用或旧缺证；全面目标仍active。

W262在Windows正式复杂只读任务发现摘要提示预算未扣固定替换前缀：1462byte摘要遵守1536提示
仍无法落入冻结28160token包络。已按已选固定状态/输入/摘要wrapper和JSON最大转义成本计算
可用提示量，保留全部硬限、状态/回执、授权及一次纠正；新反例首红后修，最终Runtime271项通过。
原正式FAIL保留；修后原任务24真实HTTP200后在交付前触及本地录制器上限、后续未派发上游，
暂停后结束回合，240秒观察越限亦保留，未计原五项任务PASS。两空闲GUI已有原生退出API实际
exit0、最终清理，未计托盘点击；WIN-012真实ACP936双pipe/生产decoder的字节/游标/lifetime
子断言另在W263通过。复杂交付及其余共享/Windows认证仍开放，详见Windows页W262/263。

### C04-01 / C05-04 精确末尾 LF 与模型参数（2026-10-01，macOS MAC-B-01；MAC-B-02 正式修后仍失败）

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

- MAC-B-02 原任务/原件/断言不变的正式修后：新字段说明已进 native request，原 SSE 和
  canonical **8 组参数全同**；两个 file content 仍缺 LF，排除 NomiFun 解码/owner 改字节归因，
  **说明不足以闭环**，不能把 13/3/2 确定性结果升级为 live PASS。16 requests/三次压缩达原 cap，
  UI 结束为499 events/cancelled/head ready，helper/完整文件链未到达，没有加预算或第二任务。
- 两个 literal command 错装整条 ls/printf 脚本，macOS sandbox wrapper exit71；一个后续正确
  argv 的提议因 needs_replan 未派发。五个实际 process 71/0/71/0/0 都 reaped，非六次执行或
  丢 canonical receipt。下一根因核对 wrapper 前真实准备失败/not_started，不按 stderr 猜测、
  默拆参数或把脚本自动交 shell。原五次执行/取消部分产物和首次 verifier 计数错误分别保留。
- 空数据无模型 Cmd-Q 有 CEF 未确认/forced-exit 告警（虽 exit0 仍 FAIL）；实跑 Cmd-Q 正常0
  只记独立子样本。原件/键值隔离、499-row closed snapshot 与无孤儿核对通过；完整 B/GEN/N3/
  C、公开语言和旧 C05 条件仍开放。证据 `2026-10-01/macos/mac-b-newline-recheck/`；本批仅短进度。

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

### C07-03 首次长消息的标题派生（2026-10-01，W198）

- 新隔离正式主页发送综合A长指令，创建Session两次HTTP400，未产生Session/模型/命令。
  planGuidEntry把整条输入原样当title；本次带末尾换行，违反创建时的edge whitespace合同，也不符合标题更新的200 UTF-8 byte上限。
  首败/API日志保留，不把零执行记任务PASS；原加密StepFun路由已解析且支持tool calls，未改供应商配置来掩盖产品拒绝。
- 现仅派生标题：trim、取首行、按完整Unicode字符限制200 UTF-8 bytes；完整input继续交给原初始消息链。
  不改title合同、用户任务内容、资源/权限、模型/工具预算或AutoWork的单Turn入口；长中文/emoji/CRLF/完整待发消息最小回归先红后绿，30/30。
- 正式修后GEN/StepFun同一任务成功创建并completed，9模型步/一次压缩；独立观察22/22：cwd与实际Hidden、中文43 bytes/4行/hash、两搜索、Git patch字节、
  两个指定Bun依序各一次exit0/1及reaped、原件/原Git修改不变，最终五项已知事实交付且没有为修报告重做命令。
- 整组A仍FAIL：首次report引用旧当前证据被schema拒绝，修正后才接受；earlier observation与内部资格用语仍显示。
  参数拒绝计数另须审查。原指令内容全部保留，编辑器末尾LF→CR的严格字节比较FAIL另存，只在去末尾换行的内容核对中一致，不覆盖严格结果。
  不由22个操作断言或标题恢复N1关闭完整A/N3/共享阶段；证据在2026-10-01/windows/w198-comprehensive-a。

### C06-05 控制拒绝计数与修正时点（2026-10-01，W199）

- 内部控制无平台binding，schema拒绝漏计；现配对结果结算时统一计一次，并移除旧未暴露分支的重复计数。
  未执行参数错误不计命令启动、不改workspace epoch、不dispatch或重开任务，已有业务非零和历史错误保留。
- 修计数后的正式首轮暴露第二个根因：模型复制拒绝前schema的expected值，四次均落后当前计数，最终failed。
  原失败不改记成功。反馈新增拒绝整批之后的确定计数，命令总数不变；不放宽const/资格/批次或控制上限。
- 14项不同Runtime定向回归、fmt/diff及正式构建通过。修后正式GEN13步/两次压缩，报告拒绝一次后接受；
  反馈/报告/canonical/UI均2/1，普通测试exit0/1各一次，六个合法exec含Git观察，无重放；独立25/25。
  原十文件/Git/旧事件及首次failed不变。基线编码误读的24/25 oracle另留，显式UTF-8的核对独立保存。
- 只关闭漏计与计数修正滞后的GEN N1，完整A首发/N3、内部警告/等待语言及B/C仍待验。
  证据在2026-10-01/windows/w199-rejected-control-accounting；不把正常业务非零抹成成功，也不以参数纠正记整体无错。
- 同步d8423c219为ab1b32a24后，计数/修正配对回归1/1及fmt/diff通过；原正式GEN身份和平台限制保留。

### C06-06 普通非零与其他未成功结果的呈现（2026-10-01，W200）

- 完整同回合回执能与总数精确对账时，尾段分别说明命令非零/退出码与参数拒绝/未执行；纯参数拒绝也明确展示。
  普通业务非零仍记录为原失败总数，不以展示分类修改canonical或宣称已修复。远端/信号/清理等实际故障继续保留原错误。
- 首次正式冷读11/14：内部控制无历史tool projection，不能确认其类别；首败保留。
  现只分类已经证明的命令结果，其余数量说明“类别尚未确认”，不从差值猜参数、供应商或本地故障。
- UI40/40、类型/i18n/桌面边界及正式构建通过；隔离正式Tauri冷读14/14，原事件/错误/统计/十文件/Git状态完全保留，新模型/执行事件0。
  仅关闭展示说明子根因；内部控制历史详情、内部术语及完整A/B/C/N3仍待验。证据在2026-10-01/windows/w200-command-and-argument-outcomes。
- 正常同步9aab28584为184742201，本批UI源码hash与正式构建一致；原冷读身份和覆盖限制保留。

### C06-07 无owner派发的结果冷读（2026-10-01，W201）

- 预检拒绝/内部控制有canonical Runtime结果，却没有owner工具投影，历史卡片及详情缺失；最小反例01及W200正式缺分类首败保留。
  现只读派生同Session/Turn/模型步的配对调用结果，不把未结算提议当执行；不重复owner卡片或写回旧投影。
  稳定显示ID、分页/总数及详情按ID读取一致，既有权限/执行身份保持；其他Session或模型步不能借同名call ID配对。
- App5/5、Session4/4、fmt/diff及正式构建通过。隔离正式Tauri冷读19/19：参数未执行卡片和原输出完整，业务非零/拒绝分类精确。
  原事件/投影/首次failed/十文件/Git保持，新模型/执行事件0；只关闭该冷读缺口，内部术语、长历史性能及完整A/B/C/N3仍待验。
  证据在2026-10-01/windows/w201-rejected-tool-cold-history。
- 正常同步e6cc5c7c0为db6758da1；本批源码hash与正式构建一致，原冷读身份及覆盖限制保留。

### C04-02 正式文件链与项目名前缀（2026-10-01，W202；失败待修）

- 正式Coding第一轮23步/5压缩：模型把已选项目显示名当额外目录，最终文件在错误路径；移动exit1后自行建目录/重试，最终却宣称完整完成。
  原件及相似名保持、错误位置的18-byte内容正确，严格6/14仍FAIL，不能用hash正确或report首次接受代替路径/步骤验收。
- 仅补充模型说明中的工作区/默认cwd/准确相对路径；14项合同、fmt/diff及正式构建通过，原宿主前缀和canonical Schema/权限不变。
  新目录、同任务第二轮仍发生前缀、错误SHA、Cmdlet当程序、引号和重复探测，31步/12压缩后正式Stop为cancelled，严格5/14；未证明提示有效。
  11个已退出命令reaped/无清理错误，原件和旧事件保持；两个FAIL、所有参数/执行错误及未完成文字均保留。
- 不关闭C01/C04或共享阶段。实际模型定义接合、来源摘要/压缩、首发命令形态及重复/完成语义继续排查，暂不追加付费循环。
  证据在2026-10-01/windows/w202-files-step-chain；独立路径/次数断言不因说明调整而放宽。
- 正常同步b530d751f为a435abe35；源码hash与正式构建一致，两轮失败及未闭合状态保留。

### C08-03 路径、shell及patch参数说明投影（2026-10-01，W203）

- W202后核对接合：顶层工具说明原样编码，参数Schema却在App装配时被canonical替换，cmd/argv/cwd与嵌套patch说明未投影；装配反例01首红保留。
  扩展既有固定白名单至已存在参数节点，只复制description；路径/完整SHA/hunk及Cmdlet脚本说明到达模型参数位置。
  不复制default/required/范围/分支或新增字段，不改注册Schema/贡献锁、权限、owner及原参数；不通过自动改路径或吞错制造通过。
- App10/10、Runtime14/14、fmt/diff通过，结构和canonical原件一致，恶意约束/authority与其他Module说明隔离。
  仅关闭说明投影缺口，尚无新正式模型样本；W202两轮FAIL、完整B/N3与其他行为根因仍保留。
  证据在2026-10-01/windows/w203-model-parameter-guidance；组件通过不代判真实任务或共享阶段。
- 正常同步1095a185e为3b1b06c3c；参数投影源码未变，正式模型验收仍待补。

### C04-03 参数投影后的实际文件效果（2026-10-01，W204；Coding N1，整组FAIL）

- 正式Coding/StepFun原样文件任务14步/两次压缩完成，创建/read/来源SHA保护patch/复制移动/限定删除/最终read均实际正确。
  用户相对路径没有项目名前缀；Copy/Move采用cmd中的带引号LiteralPath，错误保留；三条实际exec均exit0/reaped/无清理错误。
  最终正确路径18-byte/3行/末尾LF/整文件SHA、原件/相似名及旧事件一致；只新增文件效果N1，不以组件检查代判真实执行。
- 整组仍FAIL 15/18：未要求的listing实际执行，收尾另两个补查提议未dispatch被拒，summary却称没有额外操作。
  首次报告接受不证明范围遵守；真实2/0错误计数、内部警告及所有历史首败保留。
  下一缺口为禁止额外操作和真实交付，完整B/N3/GEN/C未闭合；证据在2026-10-01/windows/w204-parameter-guidance-live。

### C07-04 明确范围及报告阶段指引（2026-10-01，W205；真实行为待验）

- W204额外listing及不实“无额外操作”保留。运行政策明确只读探测也受用户禁止项限制，输入例子不能被当成任务。
  最终检查以用户范围为前提；完成复核只修报告，已关闭动作工具不再探读/列表。summary按实际调用披露偏离和未执行提议。
  权限/工具表/拒绝/计数/证据及派发不变，不通过缩小任务或隐藏失败制造通过。
- 实际压缩与拒绝保护共23项不同检查、fmt/diff通过；范围指令、原输入、工具表及阶段清除正确，01/02夹具准备错误另留。
  无新模型样本，尚未证明范围遵守；完整B/N3/GEN/C和共享门槛未达。证据在2026-10-01/windows/w205-explicit-scope-guidance。
- 正常同步a860eb019为162da2daf，运行指引代码未变；真实模型范围验收仍待补。

### C07-05 健康多步链的报告参数拒绝（2026-10-01，W206）

- 正式Coding健康文件链已完成，report因旧引用参数拒绝后却重新创建已删除源文件两次；完成回答混淆源/最终内容。16步/6压缩、严格12/18 FAIL，首败保持。
  旧参数修正报告阶段只由已结算失败命令的门槛触发，健康链不具备该来源，故仍可派发动作。
- 无显式计划、成功命令/修改已观察、单report参数拒绝、无活进程/未决patch时，现进入已有报告复核；不修改权限/Schema/计数/证据或效果。
  有效多步反例修前确实派发重复write，修后零新效果且2/0拒绝统计正确；已有显式计划修复/未决patch保护保持。9项定向、fmt/diff通过。
  新分支晚于正式binary，未证明真实修后/N3；cwd探测、无命令纯读取、摘要/交付及完整B/C仍待验。
  证据在2026-10-01/windows/w206-explicit-scope-live，正式失败和修后回归分开，不以单元通过代判共享阶段。

### C01-03 已准入工作区环境数据（2026-10-01，W207；模型效果阻断）

- 正式模型请求缺少已准入workspace根信息；现由宿主验证后的Session凭据提供JSON root/默认cwd/OS，路径值是数据，不增加读取/执行权限或额外探测。
  编码与正式prepare_turn接合2/2、fmt/diff及正式构建通过；原任务/Schema/owner不变，用户要求的pwd/list仍正常执行。
- 正式Coding首个模型步因供应商不可用暂停，任务工具/效果0；UI正确说明未完成，host清理、磁盘/旧事件和零假交付核对9/9。
  暂停现场先保留，再由正式结束回合取消隔离测试；初始agent_turns.running不代表当前仍活跃，以turn_paused/head/UI为准。
  本批不是任务成功或模型有效性/N3样本，完整B/C和共享阶段未关闭；证据在2026-10-01/windows/w207-admitted-workspace-context。
- 正常同步f1f0a8dd8为df596caf9后，缺失文件/健康报告修正2项通过；原正式构建身份和阻断范围保留。

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

### C01-02 macOS Seatbelt 裸名准备与未启动合同（2026-10-01，确定性子合同已修）

- MAC-B-02 的 literal 错整行启动 sandbox wrapper 后返回71，形成 failed-command/重规划；
  正确 argv 下一步因此被拒。既有显式路径有 pre-spawn access 检查，裸 PATH 名没有。
  真实 native 最小反例先失败且已 reap；不能从类似 stderr、exit71 或文本声称未启动。
- 仅对 macOS Seatbelt 的裸名按请求 PATH/cwd 检查实际可执行准备条件，在 watchdog/wrapper
  创建前失败才能进入既有 typed spawn failure→PROCESS_NOT_STARTED。成功不改 program/args，
  权限/Sandbox/贡献指纹不变；空格合法名、相对/空 PATH、前项 EACCES 后项可执行仍原样 execvp。
  未配置 PATH、ELOOP 等不确定访问错误或检查后变化不虚构证明，直接 Unix exec/Windows 路径不改。
- macOS zero-authority preflight1/1、native process6/6（最后 PATH 边界复核1/1）、App typed恢复1/1、
  Runtime既有非启动计数/控制1/1、direct exec ABORT1/1；failed tool1/command failure0/无假成功
  和原安全边界继续核对。没有增加模型调用、读取凭据或把原模型首败改成通过。
  最终 Seatbelt 相关4/4（含不确定访问仍保留实际 execvp、原 profile/TMPDIR）通过。
- 证据 `2026-10-01/macos/mac-c01-seatbelt-start/`；真实 UI/live是否避免原恢复开销、模型参数/
  B内容/GEN/N3/A/B/C及C05仍待验。只关闭准备/typed owner子合同，不代判另一平台。

### C01-03 macOS 减权 GEN 正式目录观察（2026-10-01，N1 子链）

- MAC-GEN-01 正式 Tauri/加密 StepFun Plan，8-call/4096/180秒先冻结；实际3 requests/126 events/
  无压缩。首次 pwd-P/ls-a 各0/reaped，精确 cwd/隐藏及中文空格列表/退出码进入 report和正式UI，
  无模型文件内容读取/其他命令/改文件，原件hash不变；正常Cmd-Q0、无残留。只记该命令 N1。
- 完整通用 template 在准备阶段要求 Computer/Scheduler 资源，不能为只读命令补授权来通过。
  用既有正常编辑 API 派生减权 GEN，仅删除这两个模块；closed DB 逐项证明 persona、instructions、
  route和其他 grants相同，不使用Coding模板冒名，也不声称默认通用完整配置已验。
- helper缺feature/provider、正常API互斥错误和过早读取编译metadata首次错误均保留，0模型请求。
  oracle先误算宿主指令读取/限制pwd选项/漏中文为0，raw SSE ID和实际字节/路径另核对纠正；
  原任务/独立结果/安全要求不变。普通备份首败后closed126-row快照完整；真实key matches0。
- evidence `2026-10-01/macos/mac-gen-observe/`；helper确切模式/最小回归新增，产品Schema/权限/
  断言不改，预算/模式/来源回归6/6。GEN已有这个有边界的实际样本，不能代替A/B/C、其他资源与入口、N3或旧失败关闭。

### C05-05 macOS pipe stdin/EOF 正式子链（2026-10-01，N1）

- MAC-B-03 隔离 fresh run003/正式 Tauri/StepFun：原12-call/4096/360秒不变，实际7 requests/
  243 events/无压缩。start/write/close各一次，真实13-byte中文单LF及READY/ECHO/EOF正确，
  final poll0/reaped/no cancel/kill/shell，report/UI/canonical与独立helper receipt、原件hash一致。
  completed/headready、实际Cmd-Q0/无残留，只有输入/EOF/终态N1，不把原文件B首败记成功。
- 另有真实限制：helper预读、末poll cursor0重放全输出、进度英文及内部字段；完整效率/语言/
  B/N3仍未达标。外部verifier先误禁所有读文件（原任务允许规则/指定helper），首败保留，
  仅纠正这项测试范围；无关读/其他命令/改文件/输入字节等实质断言不改。
- 旧closed CUA绑定的被动读取会重启旧bundle/默认profile，UI-isolation事故单列，未手动删/改
  用户数据或向其发任务，默认profile自动启动影响不能当no-change证明；今后先显式隔离启动并核对PID。空数据probe退出CEF仍阻在Security
  decrypt，TERM未清后owned KILL137，C05首败不因后续实跑0关闭，未绕系统授权/保护。
- `2026-10-01/macos/mac-b-stdin-eof/`保存原参数、独立243-row快照/首败/采样/截图；无新产品
  改动或付费重试，完整文件/长helper后代/C/N3/MM等仍开放，Windows原件与结果未改。

### C05-06 macOS UI Stop / parent-child 原生清理（2026-10-01，N1）

- MAC-B-04 正式Tauri/StepFun：12-call/4096/360秒原预算，实际7 requests/228 events/无压缩。
  start一次、六poll正确cursor，父子真实关系/同组及双心跳在UI Stop前独立ps/文件证明。
  User明确10分钟等待期限，在现有600000硬限内，非扩大权限或由timeout制造停止通过。
- UI原生Stop点击→两PID都无 **141ms**（5秒断言内），心跳后续不增；cancelled/headready/
  host_cleanup_proven及被中断poll的原call原生cancelled/reaped回执一致、is_error=false，
  native cleanup148ms/只interrupt，原输出/已发生效果与三原件保留，无新效果/孤儿。
- UI取消/旧回复提示可见，实际Cmd-Q0/无App Helper/fixture/listener；closed228-row快照逐项
  一致/ok、真实key match0。只记停止/后代N1，不代替完整B/C/N3或C05旧退出失败。
- 英文/重复状态/内部cursor叙述仍在，完整语言/过程体验未通过；无新修复假设或复制底层矩阵。
  证据 `2026-10-01/macos/mac-b-stop-descendants/`，原Windows/先前首败未改写。

### C07-04 macOS cancelled 重启冷读与文字通道诊断（2026-10-01，N1 子链）

- MAC-C-01 复用实际B04cancelled Session/正式App/原隔离data/work重启，0新模型/Turn。
  UI展开原cancelled poll的输出/清理及旧回复提示；原228 events字节全集、cancelled/headready、
  心跳和原件hash均不变，父子/forwarder不复活，Cmd-Q0/无残留。只记取消重启冷读N1。
- 语言缺口只读raw定位：04～06英文原在reasoning，Runtime→thinking分流正确；07公开
  content确也含自我规划。公开196/思考194 chars且trim不等，非UI串通道或可精确去重事实。
  原文/事件/字段保留，不按关键词/翻译/模糊相似删消息；生成层语言/叙述仍待修/验。
- 普通post-exit读取SQLite14首败后，writer/WAL无的immutable只读与live readonly/原完整
  快照全同，未改DB状态。`2026-10-01/macos/mac-c-cancel-cold/`；未交新产品假设或重复矩阵，
  未新增模型/凭据读取；完整连续纠正/压缩/C/N3/A/B/C05/MM等未因该子链关闭，Windows不代判。

### C06-08 正常文件缺失的完成元数据（2026-10-01，确定性子根因已修，live修后待验）

- MAC-B-05原文件要求/原件/独立字节断言，包含W205正式Tauri/StepFun，在原16-call预算内。
  磁盘终版32-byte/正确hash及copy/move/delete真实，但报告未接受；436 events/两压缩/
  14主steps因bounded length失败，typed task incomplete/headready，**整组FAIL**不被副作用通过。
- 实际missing_ok read是 `workspace_file_absent`，非错误表示成功观察缺失，不能推作文件存在。
  原completion scope丢掉kind，压缩后read/path“成功”与mv关系让模型误读/续写；不是文件owner
  吞末LF或正常缺失被拒。原回执/请求/UI/首败及元信息缺失最小红测均保留。
- 只保留bounded `{kind:workspace_file_absent,file_exists:false}`：已派发/non-error/scoped/
  text read/missing_ok/原path精确匹配。内容/凭据/环境/stdin/输出正文不复制，权限/输出资格/
  schema接受集合/epoch/计数不改。过期不复活当前资格；没有把“缺失观察”当文件内容/hash证明。
- completion32/32（含原freshness/失败计数/正文排除）通过，其他路径/模式/假marker/失败/未派发
  等guard继续断言。真实修后能否完整交付仍待正式验收，不再扩paid循环/关B/C/N3或另一平台。
  evidence `2026-10-01/macos/mac-b-file-results/`；仅必要源码/最小回归/短进度。

### C06-09 完成报告层级与空路径提示（2026-10-01，macOS MAC-B-06；提示修后live待验）

以下保留B06当时首败与修复；后续B08形态N1及剩余交付限制见本段末，不能据此改写首败。

- W207/正式Tauri/StepFun原文件任务：12 requests/367 events/两压缩，真实32 bytes/正确SHA/
  copy-move-delete及一次accepted报告，原件不变、四命令各0/reaped，报告纠正无副作用重放。
  仍FAIL_RECOVERED：criteria套数组、root summary/count错层；后续过期path两次预检拒绝，
  error2保留。10组raw参数/canonical精确相同，非解码/owner篡改；不以completed覆盖首败。
- 现有criteria description明确flat object array及root sibling；path集合空时显式omit/[]，
  只用匹配eligible call支持其作用范围。仅元说明，不放宽schema接受/资格/计数或补造证据。
  最小首红保留，completion32/32；新说明未再付费复测，不能代判零失败或完整B/N3。
- 本次没有missing_ok读取，C06-08正式缺失语义仍待补；rm后真实历史内容可以报告但不能证明
  持续当前状态，path拒绝继续保留。最终原summary的字面反斜杠n不自动反转义成伪原文。
- W207准入根数据回归的macOS `/var`→`/private/var`是原测试预期错误，既有准入正确；
  改用exact receipt且canonical同目录、零模型/零effect断言保持，alias/nonalias/纯数据各通过。
  本平台不改Windows验收；native退出CEF未确认/owned TERM等仍记失败，详见Mac进度。
  全证据 `2026-10-01/macos/mac-b-file-delivery/`；无预算扩大、权限变化、正文重写或新模型循环。

- **2026-10-02 MAC-B-08 修后正式N1子断言**：源783b49a2c/正式Tauri/StepFun原文件任务，
  9 requests/305 events/一压缩，首report接受、错误0，flat criteria/root summary/omit stale paths，
  7组raw参数/canonical全同，32-byte/正确SHA/copy-move-delete/原件保护均通过，不归因唯一改动。
  公开report仍有内部字段/earlier及三未验提示，原文来自上游report参数而非UI reasoning串出，
  不能净化/放宽或代验完整交付。旧oracle缺语言/原生限时断言的PASS首结果保留，严格补充FAIL。
- 观察者错过独立480秒App边界，owned TERM后0仍expired/FAIL，late UI guard不访问closed绑定；
  最终截图/AX缺口明确保留。此失败不能被形态N1覆盖，完整A/B/C/N3及C06-08/原生/MM仍开放。
  全证据`2026-10-02/macos/mac-b-report-recheck/`，无追加paid循环/新权限/猜测性产品修复，Windows不代判。

### C06-10 历史恢复提示的三处一致性（2026-10-02，确定性子根因）

- W212已更新report tool总说明；参数说明和每轮Completion context仍有旧history恢复限制。
  现两处与总说明一致：只有已展示的历史工具找回请求摘要所需既有输出，不能升格current/
  eligible，不能为修account重放观察或效果。仅description变化，不改权限/接受集合/门禁/计数。
- 复用旧结果交付/不复活stale的最小回归，语义首红保留，修后completion32/32；false-current
  仍拒绝、epoch/资格不变，historical reporting仍合法，三处提示新增对齐断言。
  零模型/无用户数据变动，证据`2026-10-02/macos/historical-guidance-consistency/`；正式live与
  公开语言未验证，不能据此覆盖Mac/Windows已有失败或关闭完整A/B/C/N3。

### C06-11 空公开答复不等于收尾（2026-10-02，macOS MAC-A-09）

- 真实原A任务/最新同步基线：8requests/202events/一压缩，仅pwd/ls/read已执行；后续未做却
  partial report/completed，整组FAIL。raw/canonical11组相同，零公开text/零call的stop后
  工具表变report-only，合法后续read/exec拒绝；六字符串argv另属生成问题，不靠默拆放过。
- active-ledger空答复/只有private thinking不作closing answer，复用现有协议纠正/事件/恢复及
  2-consecutive/8-total/Turn预算，要求原已展示工具继续未完工作或合法report，不重放/加权。
  正常公开收尾及实际report参数纠正仍关闭effect，patch/计数/权限/Schema集合保持。
- 真语义首红保留，剩余read/无重放与重复empty有界失败通过，Runtime244/244；测试桩错误
  单独保留、按接口修正，不代判修后live。原UI/202-row快照/文件/清理独立核对在
  `2026-10-02/macos/current-command-sync/`；公开语言/完整A/B/C/N3未关闭，Windows记录未改。

- **2026-10-02 MAC-A-10 修后formalN1**：原A任务/独立约束，源ef626f626/正式Tauri/StepFun；
  两次真实reasoning-only stop均进入有界纠正，后续仍有exec/read且required并实际结算，
  无未暴露拒绝/工具错误。一次Task内两分支不当N2/N3，不承诺模型不新提议已完成观察。
  整组仍FAIL：16请求/4压缩/一次伪摘要拒绝、后续sh重复、两指定测试与report未做，正式cap取消；
  397events/原件/Git/真实退出/UI/只读快照独立保持，未扩大预算/翻译原文/稀释重复计数。
  全证据`2026-10-02/macos/empty-stop-live-recheck/`；只关闭该空答复误收尾的正式N1子断言，
  完整A/B/C/语言/N3及旧CEF/MM不关闭，Windows独立结果不代判。

### C06-12 完成工具静态说明占用（2026-10-02，macOS 零模型）

- MAC-A-10请求15实测27 tools/50,892 wire JSON bytes，system/23,911；report tool/10,111，
  顶层description/4,534，含参数的全部description合计8,214，非SDK encoded_size/token。
  四次压缩保留5/5/1/1原call IDs，不将模型重复简单归为全部回执丢失。
- 只合并重复静态措辞3,247→2,385 bytes；Schema、动态说明、Completion context、错误计数、
  evidence资格/接受规则源字节保持。首红保留，completion40/40，未放宽既有测试。
  旧wire模拟仅省862 bytes/约1.7%工具总量，不当正式重验或全部上下文/重复根因闭环。
- 新模型0/UI未启动，证据`2026-10-02/macos/completion-prompt-footprint/`；完整A/B/C/N3、
  公开交付及原Mac/Windows失败不覆盖，不扩固定预算/权限、不丢历史结果。

### C06-13 参数/引用正确不等于任务无漏项（2026-10-02，MAC-A-11 正式FAIL）

- 最新合并源11b7d0164/正式Tauri/StepFun原A，7请求/2压缩/338events、UI41秒completed；
  六命令各一次、两Bun实exit0/1且reaped，九组native/canonical全同，原件/Git/sentinel不变。
  提示减重确已进入请求，但单例无重做/较短执行不证明其单独因果或完整A/N3通过。
- 两指定搜索未执行，最终漏真实cwd/头尾原文且英文，整组FAIL。最后模型请求仍有完整四项
  用户输入和语言规则；仅引用input_0的supported账本没有独立语义完整性证明。第二压缩后
  旧文件原文不在最终请求，history工具已展示但未用；未做文本启发式补任务/翻译或重复观察。
- 正式空stop纠正后exec仍展示/required并实际运行两测试；终态UI先存、Cmd-Q0/native156.81秒
  无expiry/signals/CEF错误、fixture200/0/无残留、完整338-row只读库同。具体tool-row展开未捕获。
  证据2026-10-02/macos/mac-a-completion-copy-recheck；本批无源码猜修/第二付费任务，旧失败/
  A/B/C/语言/N3及Windows独立结果保持，下一缺口聚焦漏项与公开交付，不再扩底层矩阵。

### C06-14 已读短页保留为历史数据（2026-10-02，macOS 确定性修复）

- A11读过的四行在第二压缩后最终请求缺失；完整要求仍在、history已展示但未调用，未认定
  压缩全丢要求或唯一漏项根因。仅成功scoped/匹配call与路径/ReadOnly文本页进入原owner
  metadata，含页最多512 bytes；完整byte cursor/行列/版本标记保留，超限整体省略，不裁剪。
- 原scope2KiB/detail4KiB/观测32KiB/64条及淘汰保持；无新缓存、读取、权限或额度。历史epoch/
  eligibility不动，旧read不能变current，write/instruction-scope/失败/未派发/错ID或路径等不留正文。
- 真语义首红、新三回归、13拒留形式与淘汰通过，completion43/43、Runtime254/254；测试enum
  编译错误另存后按已有ManagedEffect修正。Schema/提示/报告接受/prune源字节不变，零模型/UI。
  证据2026-10-02/macos/historical-file-page；A11原FAIL、漏搜索/英文/完整A/B/C/N3及Windows不覆盖。

- **2026-10-02 MAC-A-12 C06-14 formalN1**：源685e672a2/正式Tauri/StepFun原任务，四个后续
  task请求均有精确97-byte历史四行/epoch2/eligiblefalse，旧ID不入current enum；未达到report。
  整组FAIL：16请求/5压缩/一伪摘要拒绝，literal sed错误→deferred Bun/重复观察、cap暂停；
  观察者漏及时UI结束，native480秒TERM0仍expired，未假报取消或Cmd-Q通过。491rows/raw参数/
  原件/Git及WAL-aware只读库保持，证据2026-10-02/macos/mac-a-file-page-recheck；不覆盖A/B/C/N3。

- **2026-10-02 MAC-B-09 完整B仍FAIL**：源7033e254e/正式Tauri/StepFun，提前24-call整链预算，
  锁屏run零请求、人工解锁后新run；24请求/5压缩/546events/17参数全同。模型先patch后create，
  缺guard/漏FILE末尾LF，后续实际patch/cp产物31非32；未到两helper/EOF/父子cancel/report。
  四原件保持，首败/拒执行不吞；观察者超时TERM0仍失败、终态UI缺失，不伪造取消/完成。
  WAL-aware完整只读库保持，证据2026-10-02/macos/mac-b-mainline-recheck；不加额/重跑或猜修，
  转C主线，旧Mac/Windows操作子链及完整B/N3缺口原样保留。

- **2026-10-02 MAC-C-02 核心链formalN1**：源e46e63a90/正式Tauri/StepFun，12请求/2压缩/
  233events；原输入及正式steer保留，纠正后单写/回读20-byte新目标，旧目标未写，三原件保持。
  UI Stop取消/父子清理、live Cmd-Q0；零模型冷读用户两输入/全部233events不变，旧Turn不复活。
  整组仍FAIL：cold CEF shutdown未ack，90秒TERM/KILL；上游实际公开输出英文omission占位，
  不删/译原文、早期poll被steer拒绝不计成功，完整交付/N3待验。证据2026-10-02/macos/mac-c-mainline，
  原冷WAL错误副本另留，正确readonly WAL备份作核账，无付费重跑/源码猜修/Windows代判。

### C05-07 有界退出许可不是清理证明（2026-10-02，macOS MAC-C05-03）

- B07原联合正式任务仍本地cap未交付：文件/单LF输入正确，close/EOF/report未达到；原write漏LF/
  patch漏path与wire参数一致，原schema已要求path，非owner改写。C06-09未被实际调用，仍待验。
- 原Main fallback在未验证清理后设置cleanup_verified并返回正常0，独立于CEF是否已返回。
  现分离forced-handoff permission；无清理证明的normal0/未指定码返回1，已有非零、原intent及
  Tauri重启归属保持，fatal仍需真实清理。无期限扩大、吞错、权限/保护或Browser engine修改。
  首红0vs1保留；新增2、coordinator4、相关cleanup15全通过，Windows原生未验，不改其记录。
- 修后零模型正式冷读529 rows/原件/状态全同，当前Cmd-Q0无清理错误；用户报告已授权但系统项
  未识别，不能据此关闭native旧失败。整个probe准备95.9秒超冻结90秒、runner未及时止，仍
  NOT PASS；只保留正常退出子观察。原cap/CEF/备份/时间首败在仓库外
  `2026-10-01/macos/mac-b-combined-recheck/`（跨午夜）；完整B/C/N3及报告说明live仍开放。

### C05-08 原生观察的独立期限（2026-10-02，macOS MAC-C05-04）

- 上次手动cold probe先准备95.9秒而未执行90秒runner边界，原NOT PASS保留；现独立监督
  owned child，单调deadline及有界TERM/KILL，timeout后0也不当成功，三项定向回归通过。
  只改validation runner/minimal tests，不改产品期限/安全或扩大模型预算。
- 复用字节/签名核对的正式App及原隔离cancelled数据，真实UI cold/Cmd-Q0，56.88秒内无signal/
  timeout/forced，529 canonical rows和原件/终版/helper回执全同、零新模型/Turn/残留。
  仅有界冷读退出N1，不代判旧native失败/错误码分支/完整A/B/C/N3或另一平台。
  原限制与完整证据在`2026-10-02/macos/native-cold-deadline/`，无付费重跑或历史改写。

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

### W208 命令未启动与业务非零展示（2026-10-01）

- Case/子断言：C06、A08/A09/A17/A19；按用户澄清减少系统导致的失败并准确展示正常业务结果，保留真实错误。
- 公共修复：仅完整本地启动宿主记录证明用户代码未启动时，摘要/操作详情明确“命令未启动，尚未执行”；状态仍error，原诊断及失败/重试历史保留。
  启动拒绝不与其他故障合并成相同标签，继续执行后的摘要不再盖住该事实；不能从错误本身推断系统或供应商故障。
- 验证：定向UI98项及相关结构18项、类型/i18n/desktop边界与正式构建通过；Windows正式Tauri原失败会话冷读独立15/15，历史及文件不变、零新模型事件。
  首红、摘要首败14/15及构建占用失败另存，修后证据不覆盖历史；不是新的模型执行/N3或完整B通过，共享门槛未达。
- 证据：仓库外2026-10-01/windows/w208-process-not-started-ui，见Windows进度的覆盖与残余项。

### W209 文件步骤与最终交付N3（2026-10-01）

- Case/子断言：C01/C04/C06/C07/C08；正式Windows Tauri、原任务、StepFun/step-3.7-flash，COD/GEN/COD三例7/8/8步、1/2/2压缩完成。
- 每例严格18/18、扩展31/31：精确创建/来源保护patch/本机复制移动/限定删除/最终读各一次，18 bytes/3行/末尾LF/全文SHA及正确值=2一致；原件/旧事件保持，无额外探测/重做。
  报告首次接受、0/0失败计数；只关闭Windows文件效果/结果交付N3，不覆盖历史FAIL、不代判完整B/进程/超时/C/macOS。
- 无新源码或重复构建。W206拒绝修正分支未实际触发，组件证据与真实正向链分开；同路径两次编辑被UI概括为2个文件的文案另待修。
- 证据：仓库外2026-10-01/windows/w209-files-no-replay-live，三例独立UI/DB/事件/模型轨迹/磁盘oracle；共享门槛未达，继续共享队列。
- 同步c4456fa41为03e164aad，保留远端报告说明及macOS证据；合并后证据/防重做/正式准入root三项通过，原N3构建身份保持。

### W210 摘要调用次数与文件数量（2026-10-01）

- Case/子断言：C06、A05/A17/A19；工具摘要的count是逻辑调用数，同文件创建/修改两次不能据此称2个文件。
  现读/编辑摘要明确操作次数，实际文件清单继续按文件数量展示；原调用、状态、计数及错误不变。
- 67项相关UI、类型/i18n/desktop边界及正式构建通过；Windows正式Tauri原会话冷读14/14，11回合、事件/投影/效果与三组文件均不变，零新模型事件。
  原误报首败保留，仅关闭本摘要单位缺口，不新增运行中/多文件patch的模型样本或代判完整A/B/C。
- 证据：仓库外2026-10-01/windows/w210-file-operation-count-ui；共享门槛未达，继续当前核心队列。

### W211 综合A收尾与历史ID命名（2026-10-02）

- 原五项正式Windows GEN/StepFun：实际cwd/目录属性/中文读搜/Git和Bun exit0/1正确，但重复回读、九次历史ID参数拒绝及旧搜索引用报告拒绝，20步/10压缩后正式取消；严格17/22 FAIL保持。
- 公共说明修复：原工具调用call_id和历史记录hits[].id/next_after_id并非同类ID；顶层及字段明示先搜索、复制64位记录ID。只改变description，严格Schema/查询/读取/隔离与旧观察资格不变。
- 模型实际暴露工具的ID/游标正负例及冻结表2/2、fmt/diff通过，首红保留；正式停止/清理等8/8另记，不替代真实模型修后。
- 显式计划收尾、重复回读/旧引用、完整A/N3及B进程/C仍缺；证据在仓库外2026-10-01/windows/w211-comprehensive-a-closure和2026-10-02/windows/w211-tool-history-id-guidance。共享门槛未达。
- 正常同步248feb461为041d3564e，保留远端退出证明及macOS记录；合并后的未验证退出/重启两项通过，原正式FAIL与模型效果未验状态保持。

### W212 诊断结果与历史恢复指导（2026-10-02）

- 澄清两处模型说明：已请求的预期非零不产生新修复任务；已暴露历史工具仅恢复已见输出供summary，不使旧观察取得当前证据资格。严格计划/Schema/权限/计数及合法待办修复保留。
- 两首红另存，6项相关定向、fmt/diff及正式构建通过；显式计划本已有有效报告直接关闭路径，不凭猜测改写合同。
- 原五项正式Windows GEN/StepFun仍FAIL：9步/4压缩后cwd/listing各执行两次，尚未完成后续项，正式取消。工具error0不能代判通过；十文件/Git/旧事件与host清理等独立11/11。
- 指导效果未证，继续核对报告前的压缩续接/执行进度；完整A/N3及B进程/C和共享门槛仍缺。证据在仓库外2026-10-02/windows/w212-diagnostic-report-guidance。
- 正常同步783b49a2c为6865cb180，保留macOS独立deadline与进度；本批Rust未变、未重复模型/构建，Windows结果不代判macOS原生夹具。

### W213 指令元数据与压缩回执（2026-10-02）

- W212摘要把指令定位零扫描数当作目录缺失。真实File owner新增用途/非目录列表标记，原kind/路径/权限/计数保持；摘要提示区分回执、计划和指令元数据，截断不授权重跑。
- 三首红与9项定向、fmt/diff及正式构建分别保留；没有扩大硬预算、工具面或证据资格。
- 原五项Windows GEN/StepFun仍FAIL：8步/5压缩，cwd/listing/读搜/Git重做，第二个测试及交付未完成，正式取消。原11/22保持，加强搜索重复计数10/22另记；磁盘/旧事件及清理9/9。
- 五次压缩均只保留原输入，没有原工具交换，摘要还将已完成项列为待办或输出伪调用。后续查固定开销与回执保留；该样本未调用指令定位，不能代判新字段的模型效果或完整A/N3。
- 证据：仓库外2026-10-02/windows/w213-instruction-scope-compaction；共享门槛未达，继续核心队列。
- 正常同步b53fce0b4为204703da1，保留字段/上下文历史恢复说明与macOS记录；相关历史/证据两项通过，原正式FAIL及覆盖限制保持。

### W214 最新完整回执与可选余量（2026-10-02）

- C06/C07确定性首败：固定上下文76k、原资源80KiB，实际摘要后的最新完整回执本可装下，却被可选软余量排除。
- 现最新完整文本交换按原input_limit/bytes/messages硬限保留，旧交换仍有软余量；不拆call/result、不恢复未知历史、不变更失败/新鲜度/图片保护。
- 修后79,148≤81,920 bytes，原exit1/错误和调用ID、持久化/restore一致；更大回执及typed prompt-overflow仍受原限制。相关正反与边界7项、fmt/diff通过，首败及中间编译错误另留。
- 无新模型/正式UI样本；超硬限大批次和完整A/N3仍缺，不能将该子根因推成全部压缩问题闭环。证据在仓库外2026-10-02/windows/w214-latest-receipt-budget，共享门槛未达。
- 正常同步c2aa06028为8651083ec，保留macOS锁屏夹具记录；产品代码未变，没有重复构建或付费请求。

### W215 原回执保留与伪摘要首败（2026-10-02）

- 正式Windows GEN/StepFun、原五项任务、新workspace/profile，11步/6压缩。前五次保留原输入一次及2/2/3/2/2条原交换，W214只获正式N1；最后大批次仍无原交换，不记N3。
- 整体FAIL：首摘要输出伪工具调用，随后重复源文件脚本及错误5行/空末行；搜索/Git重做。真实样本仍43 bytes/4行，两个Bun各一次exit0/1，原件/Git及旧事件保持，正式取消。
- 原11/22及加强重复计数10/22分别保存，清理/磁盘/实际测试等10/10。零系统工具异常不代表步骤正确；不将伪摘要当新授权或真实执行，不覆盖旧失败。
- 后续查摘要协议校验/有界恢复与补查，完整A/N3及B进程/C和共享门槛仍缺；仓库外2026-10-02/windows/w215-latest-receipt-live保存全部现场，本批只更新短进度。
- 正常同步8ae0f1197为352b58219，保留macOS锁屏进度；源码未变，原构建/FAIL及覆盖限制保持，没有重复模型或编译。

### W216 摘要协议拒绝与有界纠正（2026-10-02）

- C06/C07/C08首红证明裸工具XML/JSON原可进入summary。现拒绝裸协议文本/原生ToolCall，正常说明与文档示例保留；不是新工具授权或真实调用。
- 每次prepare最多一次同源/无工具纠正，二次拒绝原输入不变、坏草稿不提交，明确错误保留。CompactionSummaryRejected只记录ID/安全reason；恢复/取消/接管按无效果观察处理，可能效果保护不变。
- 三类正反、codec/恢复/Store fence及输出/预算等8项相关定向、App编译和fmt/diff通过，首败/中间编译错误另留，零新付费/真实命令。
- 正式模型/N3、旧伪摘要历史、其他重做根因及完整A/B/C仍缺，不用组件代判W215或共享门槛。证据在仓库外2026-10-02/windows/w216-compaction-protocol。

### W217 正式A执行链GEN N1（2026-10-02）

- 正式Windows GEN/StepFun原五项任务，12步/4压缩completed；目录/读搜/Git各一次，测试按start/poll一一配对、依序exit0/1，报告首次接受、1/1计数，无重做/额外检查，独立执行16/16。
- 4个压缩保留原交换2/4/2/2条，原输入一次；无伪摘要/拒绝，不能证明W216修正分支真实触发或N3。
- 完整A仍缺：cwd完整值和历史/当前状态披露不足，criteria用两个测试poll ID支持前四项，证据范围错误；不把账本接受或动作正确当完整语义验收。
  原仅识别exec的16/22另留，合法start/poll按原真实进程/调用数独立审计，不覆盖旧结果、不放宽要求。
- 仓库外2026-10-02/windows/w217-summary-protocol-live保存全部证据，本批无新源码或付费复跑；完整A/N3/COD、B进程/C与共享门槛未达。

### W218 start/poll终态与后续命令（2026-10-02）

- 更正W217关联：五项criteria使用的是cwd/目录枚举exec的两个ID；目录项匹配，其余读搜/Git/测试错引。上一批误写为测试poll，原调用/制品/失败记录不覆盖。
- 首红证明终态引用谓词仅接受launch==result，合法start→poll的精确exit0/非零/timeout被排除。现仅为保留原launch及完整匹配过程的已清理poll保持自身终态引用；当前文件、启动及旧交互不因后续命令重新变新。
- 实际WorkStatus/CommandTracker、15种反例及既有终态/Schema/历史/文件失效共9项定向通过；修后正式模型/N3、完整A的事实与引用交付及B/C仍待验，不关闭共享阶段。
- 证据在仓库外2026-10-02/windows/w218-terminal-poll-evidence；无新付费或正式UI样本。

### W219 Coding压缩后读搜/Git重做（2026-10-02，正式FAIL）

- Windows正式COD、原综合A、新workspace/profile、StepFun/step-3.7-flash；10步/4压缩。两Bun一次exit0/1、四命令reaped，但首个测试后重复读搜/Git，随后报告旧ID预检拒绝；正式Stop为cancelled，原14/22 FAIL保留。
- 磁盘/Git/完整读及测试顺序、清理/旧事件/分类UI独立9项通过；取消截图可见而AX树缺字，原自动9/10另附复核，不改PASS。未交付完成。
- 压缩原交换2/0/0/0，输入仍74.5～78.1k bytes；重复早于报告拒绝。W216拒绝和W218终态poll分支未触发；不由此关闭完整A/N3或共享阶段。后续核对固定上下文/定义开销，不扩预算或继续付费盲重试。
- 仓库外2026-10-02/windows/w219-coding-a-terminal-report保留构建、原调用、时序、DB、UI及独立审计；本批仅短进度。

### W220 最近交换的私有推理与事实回执（2026-10-02）

- W219测试回执683 bytes，同轮私有推理6,453 bytes；原可选后缀携带推理超预算后连call/result一起丢弃，而持久化重放本已省略私有内容。原轨迹尺寸及最小首红保留，未复制私有文本到诊断。
- 现可选文本后缀计量/保留与durable replay使用同一省略标记，完整事实调用、结果/错误和用户输入保留；借用字段计量，不复制大载荷做预算探测。原媒体必需分支、硬预算/文本上限、日志/事件/证据资格不变。
- 四类私有载荷修后77,119≤81,920 bytes，原exit1/ID及恢复一致；真实大回执仍被原上限排除。三项新回归与七项原预算/协议/签名检查共10项定向通过，fmt配置/diff通过。
- 正式模型/N3、全部重复根因和完整A/B/C仍待验，不由组件结果关闭W219或共享阶段；证据在仓库外2026-10-02/windows/w220-context-footprint。

### W221 正式私有后缀与Windows属性位（2026-10-02，整体FAIL）

- W220正式Windows COD原综合A：10步/6压缩、原交换2/3/0/2/2/0、原输入各一次；样本读/两搜索/status无重做，两Bun各一次exit0/1，首次native git diff字节正确，五命令reaped。只记保留交换N1，两次零保留与完整连续性仍缺。
- 目录计算把Hidden位与Normal=128比较，9项全报Hidden，7项和独立基线冲突；产品Stop为cancelled，未交付，A FAIL保留。磁盘/Git/读搜/测试/清理/旧事件等独立10/10，不代判结果或整组通过。
- Windows模型说明补充位掩码须转Boolean/比较零，原示例保留；任意脚本、真实输出、canonical合同/权限不改。三个既有宿主/argv/admission检查通过，提示正式效果/N3与A/B/C仍待验；无新同构测试或付费循环。
- 证据在仓库外2026-10-02/windows/w221-private-tail-live；基于原生调用效果核对，字段别名/native Git不是自身失败，真正的属性值误判保持。

### W222 最新回执的触发阈值与硬输入余量（2026-10-02）

- 确定性首红：81,057 bytes/27,019估算tokens的最新完整回执可装入原81,920/28,160硬限，却因input_limit混入软token触发阈值被丢弃。现只对最新文本交换按原硬余量及实际usage核对，更旧历史仍保留软余量。
- 额外1,500 usage tokens时同回执继续排除；实际typed PromptTooLong仍用更严格20,708恢复上限，不提升预算/权限、刷新文件资格或截断真实结果。未见图片和mandatory前置保护保持。
- 两项新回归与七项原预算/恢复/私有投影/复核/媒体边界共9项通过，fmt配置/diff通过；首败另留。正式模型/N3、完整归因及A/B/C仍待验，证据在仓库外2026-10-02/windows/w222-latest-hard-token-envelope，共享未结案。

### W223 正式保留、poll字段与精确搜索（2026-10-02，整体FAIL）

- Windows正式COD原综合A 8步/6压缩，原交换7/2/2/2/2/2、原输入各一次，无零保留或读搜重做；cwd/Hidden、字节/hash、Git及两测试0/1效果正确。当前可引用集合含两个terminal poll，W218/保留/Hidden仅记N1。
- 首次poll复制launch总期限120000作wait而预检拒绝，修正30000后取得终态；报告错引旧读搜/Git再拒绝，正式Stop为cancelled、A FAIL。搜索结果正确但父目录范围违背单文件要求，原独立12/13保留，不能因夹具只有一文件改判范围合格。
- 模型说明澄清poll单次0～30000与总期限不同、search可用精确文件路径；原Schema接受集合/权限不变。三项既有lifecycle/shell/admission及搜索说明后的admission通过；首发/N3/交付及A/B/C仍缺，提示晚于本次binary。
- 仓库外2026-10-02/windows/w223-hard-envelope-live保留原样本、事件/DB/参数、终态/分类UI及独立审计；无修后付费循环，共享未结案。

### W224 早先事实与当前验证的报告形态（2026-10-02）

- 只在disposition说明增加已有合法unverified/rationale形态的JSON例，明确正文交付早先具体结果、后续未复核单独说明并省略旧证据字段；示例不代替结果，必要工作未完/未知效果继续blocked。
- 原接受集合、证据资格、计数/计划/patch保护和权限保持；五项既有交付/旧引用/计划/patch/计数回归通过，fmt配置/diff通过。无新同构测试、模型或构建。
- 正式模型是否采用/N3与A/B/C仍待验，不关闭W223首败或共享阶段；证据在仓库外2026-10-02/windows/w224-historical-report-shape。

### W225 引用拒绝后的具体反馈（2026-10-02，正式整组FAIL）

- Windows正式COD原综合A 9步/4压缩completed，无操作重做，独立操作13/13；首报告旧ID被拒，第二报告用cwd ID支持读搜/Git而获Schema接受，独立语义FAIL；W224的unverified未采用。缺完整cwd/目录值及后续未复核披露，历史不重写。
- 单独report的引用拒绝现在定位有限criterion索引，移除引用enum错误的其他ID替换列表并附原合法形态；不修改参数、接受集合、计数、批次/权限或自由文本语义判断。新反例首红、修后[1,2,3]及隐私/批次/漂移/历史保护共9项通过，fmt配置/diff通过。
- 新反馈的实际采用/首发N3与A/B/C仍缺，源修晚于本次binary；证据在仓库外2026-10-02/windows/w225-report-shape-live，共享未结案。

### W226 受控超时报告与平台清理支持度（2026-10-02）

- Windows正式GEN 6秒deadline一次start、两poll正确游标0/25，父子READY、timed_out/reaped；独立CIM20样本/心跳/原件/旧事件核对，原12/13保持。报告两次拒绝后update_plan/第三report完成，内部警告仍在；不是首发/N3/完整B。
- 确定性修复：干净明确timeout终态可直接报告optional plan，仍不授权后续effect；错误/丢失/未知与patch保护不变。真实pipe因无控制台interrupt却总尝试该阶段而产生日志错误；平台支持度用于清理阶段选择，Windows pipe跳过、ConPTY/Unix保留，显式请求及真正失败不吞错。
- Agent8项、native期限/pipe Job/ConPTY/信号错误4项共12项通过，首败/测试桩耗尽另留，fmt配置/diff通过。修后正式/N3、READY/证据/警告语言及A/B/C仍缺，共享未结案；证据在仓库外2026-10-02/windows/w226-process-timeout-live。

### W227 干净受控超时正式子链（2026-10-02，GEN N1）

- W226修后正式Windows GEN原诊断5步/0压缩：一次start、两poll正确0→25，父子READY/实际timed_out/reaped，跳过不支持interrupt、errors=[]、清理187ms；CIM18样本/心跳/原件/旧事件独立核对。报告首次接受，无update_plan/参数拒绝或重启，原1/1超时计数仍在。
- 只新增干净timeout/直接收尾GEN N1。旧样本首败不覆盖；复制oracle限定旧exit_code回答短语，原12/13和补充审计分开保存。公开回答的无退出码/信号说明间接，READY“可用证据”警告仍不合格，整体体验/完整B/N3不代判。
- 仓库外2026-10-02/windows/w227-timeout-cleanup-recheck保留正式构建/DB/事件/UI/物理见证，无新源码或重复回归。A/C和共享门槛继续开放。

### W228 已结算进程的早期poll事实（2026-10-02，确定性修复，正式待验）

- W227真实READY回执因非终态而不合格；最小首红确认即使同进程已精确结算、后续命令已执行，早期成功poll仍不能引用。原W227内部警告/体验缺口保留。
- 现仅为完整保留、同process_id且已清理的exit0/非零/timeout调用链保留早期成功poll的历史事实资格；context关联精确terminal ID，明确早期running不代表当前运行。启动/stdin、旧文件/产物不升格，不修改工作区epoch、valid_through、权限或原参数。
- 实际WorkStatus/CommandTracker正向三终态及21种失败/错配/缺链反例、新旧completion共36项通过；首红/修后分日志，fmt配置/diff通过。正式Tauri/模型采用、公开语言、取消历史poll及N3、完整A/B/C仍待验，共享未结案。
- 证据在仓库外2026-10-02/windows/w228-historical-process-poll；本批无新付费模型或原生进程，Git仅源码、两项回归和短进度。

### W229 READY历史引用正式GEN样本（2026-10-02，N1）

- W228正式custom-protocol/StepFun原六秒任务，5步/无压缩：start一次、poll0→25两次、report首次接受；分别引用早期READY与精确timeout终态，无重规划/拒绝/重做，W228新分支实际生效N1。
- 独立17/17：两READY各一次、真实timed_out/无exit/无signal、父子存活亲缘及消失、清理187ms/errors=[]、原件/旧事件保持，无外部kill。断言发送前冻结；poll数按真实cursor链核对，旧W227原oracle不改。
- 公开回答不再出现READY不可引用警告，但仍大量process_id/cursor/cleanup字段；最终没有明确无signal，UI仍以“曾有1次尝试未成功”和通用计数表示受控timeout。保留体验缺口，不以账本完成/17项动作断言关闭完整B、N3或共享阶段。
- 仓库外2026-10-02/windows/w229-ready-timeout-live保留构建、隔离夹具、DB/事件/原调用、17项audit/人工复核及折叠/展开UI；本批无新源码或重复测试。

### W230 干净timeout的用户展示（2026-10-02，正式冷读已验）

- 沿W229真实受控timeout，原UI阶段/页尾只显示通用未成功计数。新增严格原生typed分类：确切工具名、timed_out/success=false、无矛盾exit/signal、非空process_id、完整输出及reaped/errors/清理阶段字段；只改变展示，原error、计数、canonical正文/回执/任务状态保持。
- 同Turn/会话且计数全部对齐时，页尾说明运行时限与已清理；阶段及详情同样显示该事实。缺回执、清理失败、MCP同名或混合/不一致计数保持通用披露，未把超时推断为成功或预期业务结果。
- 三条首红、修后及邻近共135项不同UI检查通过；类型/i18n/桌面边界/正式构建通过。新增测试的类型首错另留，修正后检查通过。正式custom-protocol冷读W229、原READY/timeout/原始回执仍可展开，events/turns/sessions行数与字节、全部夹具不变、新模型0，复核12/12。
- 原首次11/12仅因展开前截图，保留原audit并以03b已展开截图另审，断言不改。仅关闭冷读展示缺口；实时/混合结果、模型公开语言与无signal说明、N3及完整A/B/C仍缺，共享未结案。仓库外2026-10-02/windows/w230-timeout-outcome-ui保存全证据。

### W231 完成上下文的已见原生命令输出（2026-10-02，正式待验）

- W225原轨迹：首摘要保留cwd/九项列表，后续摘要却改为pending/未执行；原两个native stdout仅138/778 bytes，命令引用本已合格，完成上下文只有scope/终态而没有原输出。轨迹尺寸/摘要身份审计及最小首红保留。
- 现按原调用保存完整小native output chunk及原cursor/loss元数据，仅随原已合格call暴露为observed_output；派发/结果ID/owner绑定须匹配。每条2KiB、聚合4KiB，仍计入原32KiB/64观察窗口，超限整条省略、随观察淘汰；不复制env/stdin/文件正文或额外owner字段，不刷新文件/证据或改变Schema/权限/硬预算。
- completion39项（含新三项输出/反例/缓存边界回归）及固定prefix/mandatory/usage三项，共42项不同检查通过，fmt配置/diff通过。只有确定性数据保留修复；正式模型是否抵抗错摘要、cwd/目录具体交付、报告范围与N3/完整A/B/C仍待验，共享未结案。
- 仓库外2026-10-02/windows/w231-retained-command-output保存W225非私密尺寸审计、首红/修后和定向预算日志；本批无新付费模型、正式UI或构建样本。

### W232 原综合A重复观察与压缩预算首败（2026-10-02，正式FAIL）

- W231正式Windows COD/StepFun、原五项任务：5步/三次成功压缩后failed，cwd/目录各重复一次，两个Bun及report未到达。四个native命令均exit0/reaped，读搜/Git首次结果正确，十文件/Git/原事件保持；原操作11/15及六项交付缺失保留，不能当完整A通过。
- 宿主明确预算错误：压缩替换83,428 bytes/27,810估算tokens，实际余量27,731，超79；UI正确显示“应用处理失败”，不是业务非零或供应商不可用。回合先自行终止，未执行Stop或Retry，不新增付费重跑。
- 首三个成功压缩后输入79,234/78,280/80,743，保留调用1/1/2。W231缓存正式模型抗错摘要/交付效果仍未证明；原重复与终态失败不覆盖。下一步核对summary硬上限与不可压缩前缀的实际余量，不扩大冻结预算或丢必需输入。
- 仓库外2026-10-02/windows/w232-comprehensive-a-output保存正式构建、原夹具、首败事件/DB/调用/audit/人工复核和失败UI；本批只记事实，完整A/B/C/N3及共享门槛仍缺。

### W233 已生成摘要的替换预算与一次纠正（2026-10-02，正式待验）

- 沿W232已生成摘要加mandatory前缀后仍超余量的失败；最小首红4,000-byte合法摘要使整体66,571超过原65,536-byte上限，原代码直接失败。现在接受每块摘要前核对完整replacement的实际byte/token/message及缩减条件。
- 无法适配的摘要记录REPLACEMENT_CONTEXT_BUDGET拒绝，整个prepare只允许一次同来源/无工具/更短请求；仍不适配明确失败并保留原input。冻结输出/上下文/actual-usage余量、accepted input及原纠正/分片预算保持，未裁剪摘要或结果来制造通过。
- 新三项及旧fixed-prefix/usage/硬token/typed恢复/mandatory/协议/输出边界共11项不同检查通过，首红/修后与fmt配置/diff分日志。两次不适配记录两条拒绝、零ContextCompacted、原input逐字保持；此为组件恢复合同，W232正式失败与重做、A/B/C/N3/共享门槛继续保留。
- 仓库外2026-10-02/windows/w233-summary-fit-recovery保存全日志；本批无新付费模型、命令或正式构建，Git仅相关代码、三项最小回归和短进度。

### W234 完成操作后的有界摘要拒绝（2026-10-02，正式FAIL）

- W233正式Windows COD原综合A，8步/六次成功压缩，cwd/list/完整读/两单文件搜索/Git status/diff各一次，两Bun依序start→poll exit0/1，四进程reaped，无重做、参数拒绝或原件/Git/旧事件变化。原操作13/15保留；两项失败为未到report及未completed，最终六项交付缺失。
- W233新分支实际触发：两条REPLACEMENT_CONTEXT_BUDGET拒绝后failed，替换83,993 bytes/27,998估算tokens仍大于27,876余量；一次纠正上限和原context保护生效，但未恢复任务。真实1/1业务非零与NomiFun预算失败分开，未Stop/Retry或继续付费重跑。
- 仅新增不重做的操作子样本及二次拒绝合同实证，不能关闭完整A/N3或共享门槛。下一步缩减重复完成说明造成的固定上下文开销，保留证据规则、用户输入及冻结预算；仓库外2026-10-02/windows/w234-summary-fit-live保存全证据。

### W235 完成说明的固定开销（2026-10-02，正式待验）

- 沿W234仍超122 tokens，缩减每轮重复的completion说明：3,856→2,400 UTF-8 bytes，减少1,456 bytes。保留旧/当前区分、精确scope、历史恢复、unknown/blocked、证据/需求及禁止重做规则；JSON状态/输出、Schema和资格谓词未改。
- completion39项及mandatory/实际usage/报告修正防重做三项，共42项既有定向通过，fmt配置/diff通过；无需新镜像测试。只证明固定说明变短及合同未回归，正式实际余量/模型交付/N3/完整A/B/C仍待验，W234失败继续保留。
- 仓库外2026-10-02/windows/w235-completion-context-copy保存原/新说明与尺寸审计、42项定向日志；无新付费模型、命令或正式构建，Git仅一行说明和短进度。

### W236 五项具体交付与报告位置误修（2026-10-02，recovered/整体FAIL）

- W235正式Windows COD原A，12步/七压缩completed；一命令合并cwd/list且9项真实Hidden/System全对，完整读/两搜索及两Bun各一次，三个native进程全部reaped，exit0/1正确、原件/Git/旧事件不变。末次一条REPLACEMENT_CONTEXT_BUDGET拒绝后继续并交付，W233一次恢复分支正式N1；不覆盖W234二次拒绝失败。
- 最终完整cwd/九项列表/文件元数据/搜索/Git修改/两测试结果六项存在检查全通过，accepted四项unverified/最后测试supported与两精确poll对应，无借用scope。仍重复git_status一次；首report[1,2,3]零起始拒绝后先改了第1/2/3项，留下第4项，两次继续拒绝后才改准，四次report的三首败保留。
- 原audit12/15保留：一个false来自合法合并输出的整串JSON解析限制，另存第二行解析及9项属性复核；重复status/首report不通过是真的行为失败。中文最终又重复四条英文rationale警告，整组仍FAIL，不关闭A/N3/共享；下一步让反馈提供明确的一起始序号及JSON路径，避免零/一起始误修。
- 仓库外2026-10-02/windows/w236-compact-guidance-live保存构建、DB/事件/四报告/原audit/人工复核与澄清、正式UI；无新源码或重复测试，未Stop/Retry/追加模型。

### W237 被拒criterion的一起始序号和JSON路径（2026-10-02，正式待验）

- W236零起始[1,2,3]被当作第1/2/3项，错误改动合法第1项并漏掉第4项，报告额外两拒绝；最小首红确认无明确数字/位置映射。现兼容保留原索引，另给一起始[2,3,4]和精确/criteria/1～3路径，仅供同报告被拒位置修正。
- 反馈保留原参数/Schema/计数/整批不派发和不借用其他ID规则，明确保留已合格项、rationale使用summary语言并适配实际不确定性；不复制私密summary/rationale/ID，不自动重写report或翻译模型文本。
- tool_validation8项、同步后completion40项及防重做/重复拒绝界限两项，共50项不同定向通过；首红/修后与fmt配置/diff分日志，无新增同构测试。正式模型采用/首发与N3、重复Git及完整A/B/C仍待验，共享未结案。
- 仓库外2026-10-02/windows/w237-explicit-criterion-locations保存全证据；本批无新付费模型、命令或构建，Git仅两字段/说明与既有回归加强及短进度。

### W238 文件→EOF→父子取消的完整B首败（2026-10-02，正式FAIL）

- 正式Windows GEN/StepFun一次完整B，16步/九压缩：写18-byte LF文件、摘要保护patch、cmd copy/move与限定del各一次；echo管道一次输入18 bytes/close/exit0，STDIN_EOF在close结果返回，后续119游标正确；hold父子READY后精确cancel一次，182ms/reaped/errors=[]，独立CIM50样本确认亲缘/消失，全部原件保持。
- 实际工具错误0，但最后回读/报告前两次REPLACEMENT_CONTEXT_BUDGET拒绝后failed：84,385 bytes/28,129估算tokens超过27,885余量。原context与首败保留，未Stop/Retry/外部kill。文件/进程跨步骤已有实际证据，完整B仍未交付，不计N3或共享达标。
- 原audit20/28保留；四个false为夹具只识别PowerShell复制/删除、只累计poll游标且只在terminal查EOF的形状限制。另存精确cmd参数、跨stdin/close游标/EOF、磁盘18 bytes/hash复核；真正未达的最后read/report/完成/顺序继续失败。生成夹具脚本的首次语法错误发生在模型前，另记工具错误。
- 下一步减少完成context中重复的空metadata字段，保留实际值、原参数与所有证据/预算门禁；仓库外2026-10-02/windows/w238-combined-b保存全证据，本批Git仅短进度。

### W239 模型context的空metadata投影（2026-10-02，正式待验）

- W238长链在报告前仍超244 tokens；当前每个available/ineligible条目及scope重复序列化无值path/command/artifact/owner metadata。最小首红保留；现只在派生模型metadata对象一层省略null，不递归到原参数、command、output或数组。
- 实际值/零/false/原参数显式null、非零终态/清理、精确call与已结算早期poll关系全部保留，原观测/Scope台账/资格/Schema/权限/计数/预算不改。completion41项及mandatory/实际usage/报告不重做三项，共44项不同检查通过，fmt配置/diff通过。
- 只关闭空字段重复的确定性开销，正式B完成/模型效果/N3/COD与完整A/C仍待验，不用单元PASS覆盖W238失败。仓库外2026-10-02/windows/w239-absent-context-metadata保存首红/修后/41项和三项预算及配置检查；无新模型/命令/构建样本。

### W240 收尾B的来源保护偏离（2026-10-02，正式FAIL/cancelled）

- 修后正式GEN完整B：第二步以无摘要保护的write_file直接覆盖新建文件，违反明确来源SHA要求；首败原调用已保存。正式Stop后cancelled，保留最后read已执行及磁盘正确18-byte/echo输入、原件hash、父子cancel/reaped证据，工具结果错误0，未报告成功。
- 原audit23/28保留，来源保护/重复write和停止后的未交付保持失败；无相同根因重跑。UI还出现私有推理省略占位文本，合并记录展示残余，按用户优先收尾要求转C链，不新增说明优化批次。
- 本批仅真实主线走查，仓库外2026-10-02/windows/w240-mainline-b-closeout保存原参数/阶段首败/取消DB、字节/进程/原事件审计及UI；没有产品源码或新回归，共享未结案。

### W241 连续纠正首败与冷读展示（2026-10-02，正式FAIL）

- 正式COD完整C：27步/15压缩，同回合steer-accepted接受纠正；却启动helper三次/自行cancel两次、来源各读三次。写纠正结果被计划门禁阻止，四次来源引用拒绝后failed；引用多加原输入没有的“核对”，未产生结果或禁止初稿，未执行操作者Stop/Retry。
- 原audit10/21不改；同回合纠正另以canonical事件证明。原件hash/宿主清理/当前PID消失成立，70条见证不证明三次启动全部拓扑；failed不改判cancelled，完整C未通过。
- 正式零模型冷启动：events/turns/sessions全行数/hash及全部夹具文件不变、无复活，但实时纠正气泡在冷读UI不可见，canonical纠正仍在。记录恢复展示残余，不用数据库完整代判UI完整。
- 外部2026-10-02/windows/w241-mainline-c保存所有首败和独立冷读证据。按收尾约束合并处理A重复/报告、B来源保护/交付、C重启重读/引用和恢复展示，不对未改根因付费循环，不展开旧矩阵。共享仍未结案。

### W242 纠正消息在冷读中被错误过滤（2026-10-02，正式子断言PASS）

- W241的steer message projection使用独立事件UUID，但correlation_id是拥有该输入的Turn operation；历史响应误拿operation当消息UUID而丢弃。accepted用户消息改用已有message projection ID，旧记录也恢复；不改canonical来源、Turn归属、执行或权限。
- 双纠正独立身份回归修前红/修后绿，相关历史17项通过，fmt/diff与desktop边界通过；原失败会话正式Tauri零模型冷读7/7，纠正/failed提示可见，全表与磁盘hash不变、无复活或新动作。
- 只关闭C冷读纠正展示；完整C执行/实际Stop、A重复/报告、B来源保护/交付及必要N3仍开放。外部2026-10-02/windows/w242-mainline-recovery保留首红/根因/回归/正式冷读，不启动未改根因的付费循环，下一项合并A复验。

### W243 合并A复验仍受替换预算阻断（2026-10-02，正式FAIL）

- 一次正式COD A、约2分57秒、11步/8压缩：cwd/list/专用全文及两搜索/Git各取得结果且Git未重复，第一指定测试exit0；自选PowerShell摘要脚本先exit1，修正提议未dispatch。十文件/Git/原事件不变，第二测试/报告未到达。
- 压缩两次候选后83,681 bytes/27,894估算tokens超过27,854限制40而自行failed，UI确有Nomi内部失败；未Stop/Retry。原audit10/16与交付0/6保留，不代判W237报告反馈/语言采用或完整A。
- 外部2026-10-02/windows/w243-mainline-a-closeout保存原始首败和终态。下一步直接核对正常压缩的触发阈值与冻结硬接纳上限，供应商margin/权限/原context保持；不继续删说明以追逐几十token，不重跑未改根因。A/B/C共享仍未结案。

### W244 正常摘要误用触发阈值及立即再压缩（2026-10-02，正式预算N1）

- 两首红：27,857-token合法摘要被27,294 soft trigger拒绝，原冻结硬接纳上限28,160未超；修接纳后下一小步又立即摘要。正常replacement现使用原硬上限扣实际usage margin，token floor给后续小步留余量；typed overflow严格cap、输出/字节/消息限、原上下文/回执/一纠正上限不改。
- 一个最小回归覆盖两故障，compaction19项和7项严格预算/summary邻近，共26项不同定向通过，fmt/diff及正式构建通过。正式COD A约1分33秒、七步/五压缩、summary拒绝0，两个指定测试各一次exit0/1、报告首发接受、精确测试scope和1/1失败计数、UI正常非零成立，预算/测试链为N1。
- 整体A仍FAIL：Git status/diff各两次，前四项实际值交付省略；原15/16及交付1/6不改，completed不是完整A PASS，N3/GEN不代判。B来源SHA/交付、C重启重读/来源纠正/Stop等仍开放；只处理这些主线直接阻断，不新增矩阵或未改根因的付费循环。
- 外部2026-10-02/windows/w244-mainline-compaction-envelope保存两首红、26项/构建、正式原参数/DB/事件/独立scope与UI；Git仅预算接纳/余量、一个回归和短进度。共享门槛仍未达。

### W245 已返回search/Git小结果未进入固定context（2026-10-02，正式交付改善N1）

- W244只有请求参数、无匹配/status/diff数据，压缩摘要丢值后无法直接交付。既有四结果回归首红保留；现scoped ReadOnly、成功dispatch且同call的search/status/diff仅复制指定字段，每snapshot512 bytes，原scope/detail/台账预算不改，超限不剪裁，私密/错身份/query/动作/未执行/opaque等负例不入context。
- 原epoch/资格/权限/Schema/计数保持，历史数据不升级current；45项completion及三项硬预算/余量/报告不重做，共48项不同定向通过，fmt/diff/正式构建通过。
- 一次正式COD A约1分24秒，11步/四压缩、summary拒绝0，观察/两测试各一次exit0/1；交付由W244的1/6改善为4/6，实际原文/SHA/两搜索/Git修改进入答案。首report三项的五个旧ID被预检拒绝后仅纠正report，无重做，2/1计数及首败保存。
- 整体A仍FAIL：实际cwd/完整九条目及三个英文rationale残余，原15/16、4/6保持，不计N3/GEN或共享达标。外部2026-10-02/windows/w245-mainline-read-result-retention存全证据；按固定收尾规则转C直接阻断，不继续未修根因的付费循环。

### W246 running结果不合格不等于应丢失进程身份（2026-10-02，正式连续纠正N1）

- 首红确认running缓存process_id/READY/next_cursor因只投给eligible条目而消失。现仍tracked的running缓存进入ineligible数据，并给last_observed_running_processes；不取得完成资格/新授权，清空tracker后不能显示旧running数据。原输出/详情/台账和冻结预算不变，46项completion及三项严格预算/报告邻近，共49项不同检查通过，fmt/diff/正式构建通过。
- 一次正式COD C、13步/四压缩，同Turn纠正已应用；实际一helper、来源各一次、正确18-byte结果写/读各一次，无自取消/重启重读。两项新输入/replan阻止的提议未dispatch且原错保留；原17/21及按实际原参数/dispatch/steering另审都保留。
- 正式cancelled/冷读无复活、全部DB三表与磁盘hash不变，原AX7/8及去浮层后实际取消截图分开保存。但helper在Stop前已达120秒deadline，CIM277样本证明约120.106秒父子消失；本次不证明运行中Stop，不代判完整C/N3或共享门槛。18字节误判、初始start无自适应观测及审计假定均另澄清，不覆盖原文件。
- 外部2026-10-02/windows/w246-mainline-live-process-context存全部证据。下一步只补实测期限不足导致未覆盖的live Stop、以及A/B直接缺口；不增角色/旧矩阵，不用组件/timeout代判Stop。

### W247 等待进度被要求清理并报告完成（2026-10-02，正式FAIL）

- 一次正式COD C、helper期限按实测在运行前改为300秒，产品预算保持；19步/八压缩后模型自行cancel并completed，操作者Stop未执行。18-byte纠正及来源/原件保持，report旧ID拒绝及原计数保存，完整C不代判PASS。
- 根因直接在宿主：无tool的等待响应触发完成复核，用户角色notice要求“Before ending, poll or explicitly cancel ... no process may survive this turn”；模型公开说明按系统要求取消，与原保持到Stop约束冲突。既有running进程回归强化后首红保留，转W248修执行/收尾状态区分。
- 外部2026-10-02/windows/w247-mainline-live-stop存原参数/复核指令来源/DB/事件/353条CIM/未执行Stop及UI；不重复付费旧根因，普通完成/unknown/取消和宿主清理仍需严格保持，共享未结案。

### W248 把活进程进度保留在执行阶段（2026-10-02，正式live Stop N1）

- W247等待进度触发收尾并被要求cancel。现has_running_processes独立于can_report，活进程继续原执行/poll及用户保持约束，不索取报告或授权自取消；实际终态清理、无活进程报告/unknown patch/blocked/计数与预算不改。既有live回归首红保留，修后七项不同邻近检查、fmt/diff/正式构建通过。
- 一次正式COD C、22步/八压缩，同Turn纠正、来源各一次、无自cancel/重启；Stop前真实父子存活，canonical cancelled后CIM约385ms均消失，419样本/心跳/host cleanup/后续零派发，取得live Stop N1。W247首败不覆盖，完整C仍不代判PASS。
- 原20/24及三拒绝保持：write未派发、两引用拒绝，实际写/读各一次但参数漏末LF，真实17bytes不符合18byte要求；缺LF的provider/codec归因未验，不补写洗绿。冷读全部DB/磁盘不变无复活，原6/8及视觉停止提示分开保存，错误文件不被恢复或隐藏。
- 外部2026-10-02/windows/w248-mainline-live-progress保存首红/中间红/七项/正式父子停止/原参数/DB/事件/审计/冷读；仅闭此直接等待/停止根因，C字节/引用、A/B与必要N3仍开放，共享未达。

### W249 引用拒绝保留精确addition位置（2026-10-02，正式采用未触发）

- W248三个新要求的第2个source多加“核对”，只提示input 1导致重复原错。首红保留；内部merge沿原验证给数字JSON路径/来源索引，update_plan反馈rejected_parameter_path与source_input_index，旧String错误API/验证集合/原计划/拒绝/计数保持，不回显原输入/quote或自动修复。9计划＋6要求＋两严格预算/防重做，共17项不同定向及fmt/diff/正式构建通过。
- 一次正式COD原C，38步/15压缩、239.114秒内Stop，helper一次/源文件完整分页无重读，父子活体Stop后约536ms消失、cancelled无新派发；零模型冷读全部三表/磁盘不变无复活。原21/24、冷读6/8与末LF缺失保留，真实17bytes，不补写洗绿。
- 本次update_plan省略requirements，未触发引用拒绝，不代判位置反馈采用/完整C/N3。录得ToolCallDelta空参数文本，不能归因provider或codec；工具已见参数缺LF只是边界事实。外部2026-10-02/windows/w249-mainline-citation-location存全首败/审计及澄清；不重跑未改LF根因，A/B及必要N3仍开放，共享未达。

### macOS C03 主链补缺口与末LF归因（2026-10-02）

- 正式Tauri/StepFun、源e2f7e47ca，原C任务/16-call预算：13请求/两压缩，READY→同Turn纠正→20-byte写/回读/中文说明→后续poll→实际live Stop，父子消失/cleanup证明及冷读四表/磁盘不变为本机N1。四次预检拒绝保留；准备公开报告和W248新review分支未覆盖，原C02冷KILL未修，不代判完整C/N3或Windows结果。
- B09零调用复用定位：末LF要求在原接受输入及写前provider请求仍保留；三次原始响应SSE的content已缺末LF，canonical参数逐项相同。因此本样本不支持codec/文件owner裁剪归因，也不自动补LF洗绿；不外推为全部平台/供应商原因。证据外部2026-10-02/macos/mac-c-live-progress，首败及完整轨迹不入Git。

### macOS A13 合并复验，实际执行与公开交付分开（2026-10-02）

- 正式Tauri/StepFun原A任务、源e2f7e47ca/预算16：7请求/两压缩，全部观察/两测试各一次、实际exit0/1/reaped，无重复或改文件。首report旧三ID拒绝后只修报告，真实错误计数/UI2/1、completed及正常Cmd-Q为本机操作链N1；共享预算首败未再触发，不代判完整A/N3。
- 整组仍缺cwd/隐藏项名称/原文头尾交付；末任务请求07中这些实际值及两搜索都存在。A12零模型三请求六组pwd/ls输出/游标也精确，现证据不支持再加缓存或owner裁剪归因。原首败、未展开详情和夹具首错保留；不放宽交付或重复未改根因，外部2026-10-02/macos/mac-a-shared-results，未改Windows结果。

### C05-09 Tauri退出传递丢失失败码（macOS实测，2026-10-02）

- 命令减权GEN原C任务12请求/一压缩，准备公开一次/纠正/真实回读/后续poll/live Stop/冷读为本机N1，一次replan拒绝保留。冷进程0/85.13秒，但清理四轮未验证；实样本主线程CEF shutdown、worker钥匙串解密RPC，native等待61.851秒。不改保护，不代判完整C/默认GEN/N3或Windows。
- 锁定Wry2.11.2 RequestExit传code给回调后只设置ControlFlow::Exit，App::run在Tao直接exit0，既有协调器失败码没有到达进程。现run_return后显式采用协调器终态；未验证/未获允许保持非零，正常0/原非零/fatal和Tauri restart所有权保持。两最小回归＋原邻近共12项不同检查、正式Tauri通过；无CEF调用/清理判定/预算/权限改变。
- 修后同数据零模型正式冷读正常0/32.51秒且清理无失败、原四表/六文件全同；本次未重触发失败分支，真实失败码路径和CEF卡顿仍须分开，未宣称全部根因关闭。原native首败及严格红保留于外部2026-10-02/macos/mac-gen-continuous；Windows native与真实restart未验、Windows结果未改。

### W250 包装工具提议误入压缩摘要，集中主线收尾（2026-10-02）

- 正式GEN原完整B复验最后交付缺口：真实SHA patch/复制移动/精确删除/18-byte末LF终版及最后read各一次，原件保持；没有stdin写入却close，真实EOF0 bytes，hold重复cursor0，未报告。20步/11压缩、原20/28及三模型拒绝/一截断/两摘要拒绝保持。观察组织越240秒限14.275秒后正式Stop为cancelled，父子原cancel159ms/69样本、host cleanup/后续零派发及真实取消UI成立；完整B仍FAIL。
- 确定摘要缺口：三次接受仅含tool_call的JSON包装提议，原校验仅识别直接调用对象。只补该形态到已有CompactionInvalidSummary/一纠正路径；首红保留，普通历史/文档数据、原输入/context、无工具摘要及冻结预算不改。两个既有回归加强，8项不同定向及fmt/diff通过，不追加付费循环；未证明它是漏stdin原因或新修复已被真实模型采用。
- 主线剩余集中为摘要修后正式验证、A结果/引用语言、B输入/游标/交付、C末LF及必要N3/入口，复用已验子链，不扩业务/旧矩阵。Windows完整场景0/3，共享未达；来源保护本次遵循不抹W240首败。外部2026-10-02/windows/w250-mainline-b-final存完整证据，Git只保留必要源码/已有回归/短进度。
- 正常同步a3a0bf001；共享退出码修复的两项回归在Windows通过，合计10项不同定向检查，未跑全仓测试。正式Windows退出和摘要新修复的live效果保持待验；旧正式构建及macOS证据不移植为本机PASS。
### 用户再次要求核心体验提速收敛（2026-10-02）

- 实施计划追加缺陷驱动规则：复用未受影响的实际证据，停止为复杂整组全绿/N3/角色矩阵重复子链；安全、精确字节、结果真实性和首败不减。未通过/生成残余/发布认证分开，不改写Windows结果。
- macOS新增C复跑在发送前关闭，实际请求0/无native启动/fixture200退出及无残留。当前只追结果交付完整性、精确字节/步骤提议两个体验缺口；已有操作/EOF/Stop/冷读/原生退出修复不再从头跑，仍不宣称整组A/B/C或全部模型行为达标。

### W251 原生历史丢失真实回执，公开EOF被错误摘要替代（2026-10-02）

- 正式GEN只读收尾W250，当前文件18-byte/完整SHA正确，却公开旧EOF18，原实物/回执为0。真实Runtime零模型重放仅两消息、无原ToolResult，保留错误摘要；首败及原audit/另审不改。
- 修复闭合回合重放：原生process实际准入、原call/result及互动process_id匹配后，按原投影策略保留已丢出tool尾部的小回执。单记录2 KiB、整块含标签4 KiB，白名单字段/整条省略计数，不复制input/env/私有字段、不添加工具交换/当前证据或权限；恢复前缀及隔离归档保持。两最小回归先红后绿，历史5＋预算/隐私/不重做4＋恢复/核对/不覆盖3，共12项不同定向通过，fmt/diff通过。
- 真实W250修后重放9回执/1省略、准确EOF0，原错误摘要和事件保持。但唯一修后正式样本没有新read、仍答EOF18（11/13），不代判live采用或共享达标；无同根因付费循环。新二进制/嵌入服务及无截断已核对，实际请求wire未取得，生成/接合责任仍有验证缺口。完整证据外部2026-10-02/windows/w251-report-closeout，不把组件结果拼为整组PASS。

### C06-15 高优先级策略误把历史命令事实一律判旧（macOS实测，2026-10-02）

- A13请求policy说后续process使旧命令证据失效/只引用最新，实际机制却保留reaped命令过去terminal/output。强化原缓存回归首红；仅改两句区分当前文件状态与过去命令结果、逐项matching advertised ID，不改资格/权限/Schema/预算/计数或刷新旧文件。
- 三项直接回归、正式Tauri及一次针对性日常任务通过：3请求/两实际命令各一次0/reaped，真实cwd/完整五名称在中文report/UI、各matching ID、首次report/error0/无额外操作与文件改动。宿主政策进入真实wire，正常Cmd-Q、全快照/无残留独立核对。
- 只关闭该产品矛盾的针对性链N1，不声称唯一因果、完整A/N3或所有模型交付达标；A13首败/B缺LF仍保留，不追加未改问题循环。外部2026-10-02/macos/delivery-policy存首红/修后/wire/DB/UI及首次格式oracle错误；Windows结果未改。

### W252 删除旧排程冲突，限定核心收尾（2026-10-02）

- 实施计划删除残留的B→C→A/N3活动门槛和旧整组结束条款，采用每缺陷30分钟零模型定位、最小修复/回归及必要的一次短UI；历史断言/首败/未认证状态不变，阶段交付和生成精度残余分列。
- W250/W251真实记录经正式Runtime与OpenAI Chat编码器重建，源/编码消息各9条，真实EOF0保持，网络请求/真实凭据读取均0；证明构造与编码可保真，不能代判原历史HTTP或模型采用。外部2026-10-02/windows/w252-convergence-audit存证明及首次探针编译错误；无新产品源码、模型循环或全仓测试。

### W253 后续闭合回合再次擦除较早原生回执（2026-10-02）

- W251保留只附在该回合尾部，后续回合合法压缩仍能清掉它；零模型两回合反例首红保留。整组历史现单独暂存验证后的有界回执，原对话/未交付纠正重放完再加入，App接线采用；原单回合API、预算、身份/Schema、当前证据及执行权限不变，不按跨Turn重复call ID误排旧结果。
- 历史6＋恢复/硬预算/报告不重做3，共9项不同定向、App check及fmt/diff通过；有效原件/错源/超限/非零/清理未决沿前批断言保持。真实记录新入口/正式编码仍9条且EOF0保持，数据排在错误旧回答后、当前用户输入前；网络/真凭据读取0，source/DB不改写。
- 这只关闭可复现的数据再丢失，排序对模型准确性的作用未验，不代判W251因果、live或共享达标。两项夹具首错与真实行为首红分开，外部2026-10-02/windows/w253-closed-history-receipts保留完整记录；不付费循环或扩展角色/业务矩阵。

### W254 历史真实回执与当前读取的正式交付N1（2026-10-02）

- b1c21a0a5正式Tauri/StepFun/GEN原只读提示一次复验，两model步骤/一实际压缩、UI18秒completed；当前文件全文read一次、字节18/LF/三行/完整SHA正确，旧echo EOF0/exit0/reaped及hold cancelled/159ms准确交付，并明确历史/当前边界。
- 独立13/13、真实UI及语义核对通过；所有原文件/旧事件hash/旧Turn整行不变，cancelled不复活、错误历史回答不删，无命令/进程/写入/补查或错误拒绝。原W251首败保持；仅关闭针对性保留/交付链N1，不代判原长场景、全部生成精度或共享全阶段。
- 外部2026-10-02/windows/w254-native-history-final留原提示/源与产物/独立基线/DB/事件/正文/UI；无新源码或整组/N3重跑，真实过去HTTP及Windows退出仍未验，不继续同根因付费循环。

### W255 Windows三项结果交付N1与机制阶段收尾（2026-10-02）

- 正式Tauri/StepFun/COD新Session、复用b1c21a0a5构建及隔离profile/data/work；三步骤/UI13秒，cwd/含真实Hidden的九条目/中文空格文件全文各一次，三项具体值及首尾/43 bytes/完整SHA一次中文report首次接受，逐项matching call，exit0/reaped、错误0/0，无重复或附带动作。
- 20项不同定向断言及真实UI通过；原文件集合（含.git）、旧事件/Turn全部不变，原W245完整A首败保留。Windows短链验证已同步的命令事实策略，不拼成原完整A或所有生成精度PASS；W254历史EOF0/当前读取子链直接复用。
- 机制修复与相关正式采用作为阶段交付；生成漏stdin/重复cursor/末LF/来源前提仍开放，Windows托盘退出观察受限、全矩阵/N3/LONG/99%未认证。页首移除残留整组/N3活动口径，没有新根因不再付费复跑；不宣称共享全阶段完成。证据外部2026-10-02/windows/w255-multi-result-final，Git仅短进度。

### W256 stdin遗漏的有限定位，唯一原因未证（2026-10-02）

- 原W250记录中关闭前摘要仍明确18-byte输入未做、游标/handle保留，冻结Snapshot允许input；第8步拒绝无派发、第9步close真实EOF0与磁盘一致，7项直接核对。记录不支持待办已被压缩抹掉或冻结准入缺失。
- 从ReasoningDelta提取的XML经现有正式守卫234个UTF-8分片均正确识别名称/拒绝文本，不是原拒绝输出或历史请求；当时tool_hint为何缺失和实际工具表仍缺证，不代判供应商/宿主责任或完整B已修。零模型/网络/产品改动，不追加付费循环或相邻Case；原FAIL和诊断首产物保留。证据外部2026-10-02/windows/w256-stdin-attribution。

### W257 Windows真实stdin wire→owner→实物短链N1（2026-10-02）

- 正式Tauri/GEN、新data/work/profile、原加密StepFun连接，仓库外loopback夹具保留实际7请求/SSE、认证头不记录；六步骤/一实际压缩/UI18秒。start/poll/input/close/poll/report各一次，同handle，实际18 bytes及末LF/EOF/hex、游标0→11→119、exit0/reaped和中文首次交付成立，无额外工具/协议错误。
- 19项独立oracle加PID消失/68个制品无凭据，共21项不同断言；真实六组原生参数与canonical相同、helper/AGENTS保持、原source三张canonical表整行不变。两次错误DB克隆在模型前由身份门禁拒绝，首败保留；正式新库初始化后仅内存重加密导入所选连接，没有删除身份凭据或放宽保护。最终自有GUI/forwarder已核对清理。
- 只关闭当前短链接合/精确stdin的N1证据缺口；未复现W250原拒绝或证明其唯一原因，不改旧完整B/长任务生成残余/Windows托盘退出/全阶段认证。没有新产品源码、构建或整组/N3循环；本次网络拓扑含录制夹具，普通生产直连不由此认证。证据外部2026-10-02/windows/w257-stdin-wire，Git仅短进度。

### macOS 复杂交付与摘要协议根因修复（2026-10-02）

- 私密reasoning-only被拒绝后不回放假调用；兼容Chat不将私密思考或内部省略标记编码为普通
  助手正文，不解析私密名字缩窄工具面。原生参数、签名/加密续接、Frozen grants保持。
- 多编号普通进度不提前关闭剩余授权工具；已结算报告参数修正仍report-only，不允许重做。
  中文明确只读禁止句识别及重复完成说明去重，未削弱证据/错误计数/权限合同。
- 单次自检不足，改显式编号交付与受限真实结果引用；宿主交付所选精确缓存数据，历史数据
  不变新鲜证据，缺项blocked。原完整A13 run023正式N1通过：13请求/540events/18原生参数
  全同，具体cwd/名称/头尾行数/搜索/Git/两测试0、1实际交付；七原件/Git不变，失败6/2保留。
  不是N3或首发全部提议无错，Windows未代验、旧失败保持。
- 裸动作参数JSON曾被接纳为压缩摘要；现只按原冻结schema匹配拒绝，禁止外部schema读取，
  正常结构化状态笔记允许。run024只实际覆盖XML摘要拒绝，裸JSON live未验；无自动字节补齐。
  B09仍跳过before→after且重复末LF，末mandatory投影增长硬失败；已去重交付目录正文投影，
  最后轻量修复仅本机回归，原完整B/全包络界限/公开元数据简化继续开放。
- Runtime270、Broker18及原35协议项、本机正式构建通过；无限付费次数是用户明确授权，不改
  产品上下文、进程/清理保护。完整轨迹仅外部2026-10-02/macos/complex-delivery-repair，
  不迁移Windows credential/结果；真实Keychain物理失败仍开放。

### macOS B09 强制投影及交互合同（2026-10-03）

- 完成引用目录仅ref/tool/path，不重复argv/结果/源全文；多编号简短summary与精确附件职责
  分开，所有Schema断言和已保存字节保持。五槽/最后三read/13ref直接回归，不扩预算。
- 编号首次file mutation/stdin前需独立plan；只证明先规划，不证明解释/字节正确。显式关闭
  计划后的报告参数修正保持report-only。正式B09主动plan后顺序改善但仍缺LF/遗漏删除/
  错误PTY同步exec而FAIL；Stop/正常退出/冷五表成立，不将已验子链代判整体。
- 摘要root tool_calls数组无ID包装漏识别现拒绝；未知tool同样拒绝，普通状态/嵌套历史保持。
  补真实exec等待结束与start先返回句柄、pipe stdin/EOF及tty字段说明，仅模型呈现，注册
  Schema/owner字节/Frozen provenance不改。Runtime271及直接回归成立，最后补充未新live。
- 证据仅外部2026-10-03/macos/b09-plan-envelope；原B生成精度/完整报告和真实Keychain物理
  清理仍开放，Windows原生结果未代判，未新建动作框架或扩大权限/保护/断言。

### macOS 源绑定参数契约与未知恢复（2026-10-03）

- 复用plan的有限exact_actions，源quote仅定位、参数digest-only checkpoint；owner前比较
  真实参数/最终stdin字节，拒绝不改写。编号写/patch/stdin须当前契约，独立只读仍原门。
  reservation先持久化，typed正向owner回执推进；未知效果不复位，冷恢复区分未派发/
  真实成功/已派发未知，新ID不能重复同源已完成动作。Runtime277及六个直接回归成立。
- 原B正式采用已改善33→32B精确变更、copy/move/delete各一次；但遗漏内容回读、运行中
  控制/五次压缩超过默认30秒helper寿命，stdin效果未知，原完整B仍FAIL。31请求/594events/
  八effects原样；桌面锁定阻断Quit，480+5期限TERM/exit1/expired不冒充正常退出。
- 后续聚焦声明时机和真实顺序/交付；不延长预算/寿命或以模型契约作语义证明。此版改变
  共享控制schema，Windows未正式采用，旧平台结果保持。证据外部2026-10-03/macos/
  b09-exact-actions；真实Keychain与公开元数据呈现风险仍开放。

### macOS B09 receiver 预声明与声明反馈（2026-10-03）

- receiver_ref引用前序start exact动作，在真实running owner回执后绑定目标digest；stdin
  原参数/字节/TTL不变，未知/已结束/cold不能造live receiver，同ID重声明不复位host状态。
  仅digest checkpoint、当前项投影，编号报告说明去重的结构断言严格相同。
- 原B formal仍FAIL：9请求/116events/0effects，错误patch声明连续拒绝，未进入receiver实际
  采用。正常UI/Cmd-Q0/102.47秒及冷五表成立，不以零效果关闭字节/步骤/交付缺口。
- 补schema支持tool枚举和精确声明参数定位/脱敏反馈；同源成功stdin不可换ref新ID重入。
  Runtime279及直接负例通过，最后反馈/别名保护未新live。外部2026-10-03/macos/
  b09-receiver-ref保留首败；共享新控制字段Windows未代验，Keychain/公开表达仍开放。

### macOS B09 定位反馈采用与固定说明去重（2026-10-03）

- clean5de6798ec正式原B定位反馈后plan接纳，32请求/591events/六effects，但首写直接
  after32而非before33→修改，stdin未预声明而helper活动期控制/压缩后结果未知，原B仍FAIL。
  read内容/hash后delete与copy/move各一次成立；未知动作不重放，真实失败与修正均保留。
- 正式Quit请求native exit1/noexpiry/noTERM，Agent shutdown不验证重试失败；非CEF/
  Keychain样本，冷五表全同不代判cleanup已完成。
- 只在既有host mandatory root/default-cwd上下文时合并七份tool root说明，规则一处，
  独立工具原说明/schema/名称/曝光不变。Runtime280及结构/幂等/serialized减少回归通过，
  最后去重未live，未扩大预算。证据外部2026-10-03/macos/b09-contract-feedback，
  语义生成/receiver采用/完整报告/真实Keychain仍开放，Windows结果保持。

- 同native样本定位resource cleanup失败结果被永久缓存，阻断上层显式原owner重试。
  完成失败且未消耗release才允许新settlement flight，pending/success/最终release继续缓存；
  不清unknown/panic、不跑任务或重放参数。新缓存判定1项+cleanup_retry9项通过，
  最后修复未native，Agent failed cleanup与Keychain物理阻塞不混同，Windows未代验。

W265在Windows源1d3712209的当前前端/正式Tauri采用固定root说明去重：实际StepFun请求仅一处
instructions、tool重复0，原五项只读操作/两个指定测试0/1及原始index字节保持，16项限定核对
成立。12真实HTTP200/10组完成参数与canonical相同；第11摘要耗尽4096输出tokens、length且
正文0，单次纠正被120秒正式Stop中断，无最终五项报告，原复杂交付仍FAIL。摘要推理配置的
产品唯一原因未证，不改冻结配置/硬限或循环付费；W250/W249/W240和其他认证边界保持，
详见Windows页W265，全面目标继续active。

### macOS 晚控制真实终态与公开交付格式（2026-10-03）

- 后台deadline可能先在原owner完成，scope尚无terminal便盲目stdin并报unknown。新增冻结
  terminal只读查询，原reap成立才返回真实终态/输出且本次control未执行；lookup/未reap
  仍unknown，不续lease/不cancel/不消费cursor。Runtime收终态但不推进未执行控制的epoch。
- 本机原生process定向11/11、Runtime283/283；首READY重复输出观察器FAIL保持，修后核对
  首start与terminal完整输出及cursor0重放。新宿主公开格式持久化版本、旧文本回放保持，
  literal原文/真实计数和未知保留，已知wrapper简短显示；语言仅摘要显示回退。
- exact邻近指导保持initial创建→实际receipt→独立修改，不自动解释任务/补字节。
  无付费或正式UI重跑；原B/A13新源码正式证据及Keychain物理等待未闭环，Windows未代验。
  外部证据2026-10-03/macos/late-input-terminal，不提交完整日志/数据库/模型轨迹。

### macOS B09 完整效果链与末端summary wrapper（2026-10-03）

- clean e22e329ca正式原B首次取得实际before33→patch32→cp/mv→内容read/hash→rm、
  13B输入/EOF/exit0、第二helper父子Stop完整链；33native参数等于canonical，原件全同。
  46请求/33步骤/1020events/14effects，8门拒+2真实exit2保留；report0，完整B仍FAIL。
- 最后protected前缀可单独容纳，但额外空摘要wrapper跨硬线，摘要尚未发送。去掉已被
  admitted workspace完整规则覆盖的独立336B hint，不丢任何结果/Schema/预算；部分上下文
  仍原说明，不移除adaptive索引。冻结边界及Runtime284/284通过，最后未formal复验。
- 正式Cmd-Q0/332.15s/noexpiry/noTERM、父子witness和五表封存独立成立；非CEF/Keychain
  样本，不代判系统物理风险或Windows结果。外证据2026-10-03/macos/b09-postfix-live。

W266补c58378d25迟到控制在Windows的native采用：新增真实pipe/ConPTY两项，当前App owner
11/11定向通过，1秒期限后未缓存scope先取得真实timed_out/reaped，stdin/close/resize均未执行；
原片段/冻结output/cursor0回放一致，无重启/输入重放。只新增Windows回归，生产代码不改，
正式UI/真实模型采用与原B/W250仍未验，不能把这两条拼成完整进程或Windows认证。

### macOS A13 修正反馈与历史交付分离（2026-10-03）

- 最新97bbbd8dc正式原A仍FAIL：尾读未做，原实值只交付4bundle；public_format plain_zh_v1
  已实际采用，但模型仍复制公开英文证据术语。原件/Git保护、实际测试0/1、Cmd-Q0成立；
  6请求/5步骤/386events/五effects，全证据外置complex-report-closure，旧A N1不代判新样本。
- 定位旧argument repair通知要求将旧值转summary，与独立delivery发布合同冲突。编号报告
  现只改criteria现状引用，保留仍合法历史delivery refs，不恢复current资格/不重跑；中文
  显示例句与公开label语言说明补齐，尾部参数指导无API/权限扩张。Runtime285/285及
  source/privacy/历史refs回归通过，最后未formal，不将缺项或blocked洗绿，Windows未代验。

### macOS A13 原完整交付修后 N1（2026-10-03）

- 最新688d251d4正式原A/StepFun8请求/7步骤，16native=canonical、11/11结果真实读取并
  精确发布/UI呈现，含实际cwd和独立头/尾两页97B、搜索/Git/测试0/1；原件/Git/哨兵保护。
  原完整功能N1 PASS，3report预检拒绝+预期exit1共4/1保持，不认证N3或首发无错。
- Cmd-Q0/190.257s/noexpiry、五表437/1/1/1/5全同/ok。公开中文版本实际采用，但search/
  VCS fallback机器字段/自由术语未全收敛。外部最小audit与全部日志在complex-report-closure-fixed。
- 原B补报告未执行：Cua真实Mac锁定、任务未发送/模型0，原480秒native期限后TERM/expired
  保持失败观察，旧1020events/14effects及failed全同，等价relay/fixture关闭；不代判B或Windows。

### macOS 历史读取控制 bootstrap（2026-10-03）

- 原B同Session正式仅补报告3次请求全部thinking-only length：末completion4096，public/
  tool0；2次有界续写后TaskIncomplete，旧1020events/14effects完整无重放，新报告仍0。
  Cmd-Q0/noexpiry与五表1065/1/1/2/14同源独立成立，完整证据外置complex-report-closure-fixed。
- 已有owner-auth history_port但LOAD未曝光、又不在ToolSearch目录，造成历史任务入口循环。
  首轮有port则LOAD，成功归档控制才激活ToolHistory，无TaskLedger/owner/证据提鲜/权限增开；
  考虑旧build兼容性仍保持原校验，不由此声称旧历史必可导入。纯思考截断用非代码分块提示，
  预算/续写数/私密思考不回放保持。Runtime288/288，新修复未formal，Windows未代验。

### macOS 历史游标检索修正（2026-10-03）

- dc686a7dd正式LOAD曝光并调用1次，但传末UUID被拒；后SEARCH操作ID得到空归档，未READ，
  后3thinking-only length，5请求/63新events/报告0，B继续FAIL。前1065events及14effects
  全值保持，无新effect；解锁后正常Cmd-Q0/noexpiry，闭五表1128/1/1/3/14同源/ok。
- before_turn现在明确不透明排他游标及完整next_before_turn遍历，拒绝反馈给latest prior
  重启路径，不自动修cursor或放宽scope/binding。Runtime289/289、archive/身份/私值保护
  回归成立；最后未formal，证据外置history-report-bootstrap，Windows结果未代判。

### macOS 公开交付 v2 与原 B 报告未完成（2026-10-03）

- 033677469正式同Session纯报告3请求均thinking-only length/4096，public/tool0，没有LOAD；
  报告0仍FAIL，游标修复未获实际采用证据。正常Cmd-Q0/noexpiry，闭五表1172/1/1/4/14，
  旧1128events/14effects全值不变，无重放。外部观察buffer首次失败保留，不归因为产品故障。
- 新持久化 public_format v2仅中文化已知search/Git包装元数据，未知字段完整JSON保留，
  路径/查询/原文/差异/hash/计数/false/空值不猜改；v1/None精确历史输出保持。
- renderer先于Markdown按gi删除Skill标记，旧精确大写转义漏小写/混合。仅新v2统一转义
  marker开头，覆盖结构化、process/read、未知fallback，数组结构和JSON逆解实际值保持。
  Runtime291/291、实际renderer parser及只读复核通过；新v2尚未formal，不代验Windows，
  不将原B、自由语言或真实Keychain物理失败改PASS。证据外置history-cursor-live。

### macOS 历史数据跨实现升级读取（2026-10-03）

- 原B与补报告 Session/runtime/build_id/snapshot 全同，但实现 digest 改变使 archive 全
  binding Eq 必然拒绝旧结果。data-only import 现仅允许有效不同 digest，原 source_binding
  与 source_turn进入payload/hash；其余身份、完整codec、原子导入保持。执行恢复的全Eq不改，
  不刷新证据/重放/增权。宿主native/message游标均不得越过固定accepted-root。
- Runtime294通过/1默认ignored；另显式对原B只读634事件/33完整结果正文、失败标记、来源
  与READ回放验证通过，owner/model0。App history5/5及独立复核通过，首次失败另存外证据。
  正式B模型采用未验，不改Windows，证据外置history-cursor-live；Mac锁定不阻断确定性修复。

### macOS 清理前置的 Robot 任务归属（2026-10-03）

- abort accept-loop不能证明detached source/session/嵌套语音任务或Agent bridge结束。现保留
  current/retired Gateway及全部本地worker completion，封准入、取消后join；5秒超时/丢waiter
  留原凭据，panic/poison不转成功，业务失败不等同清理失败。ASR及已接受dispatch停止fence补齐。
- 生产host把Robot/SSH join放在Browser之前，未知则保留Browser/DB；仍需独立Agent runtime
  关闭证明，不把本地Robot完成当canonical进程、远端模型或物理设备已清理。
- Robot153、App bridge2、Robot失败隔离1、既有Agent失败隔离/重试1及独立复核通过；首次
  App测试编译错误与修后各自外置history-cursor-live。未有真实Keychain/CEF物理live，风险OPEN，
  Windows新共享关闭顺序/Robot改动未代验，未改已有Windows记录。

### macOS 退出前置的资源/存储分段（2026-10-03）

- BrowserResourceService资源屏障/物理工厂分段，关闭期间封全部准入，物理worker取消不丢，
  未确认资源不调用native。storage-independent默认false，仅macOS CEF明确true；生产Host
  已确认存储关闭后才走CEF tail，其他工厂保留旧顺序，已关DB重试不重启producer。
- BackgroundTaskRegistry两个外层取消首红：丢剩余句柄和已join错误；RAII在owner释放前
  归还。Knowledge后台create/resume独立持有句柄，阻止停后publication，真实local worker
  完成才join；业务失败不假称Synced，foreground预算不变。Gateway先停、所有前置错误
  拦native/DB。旧只观察Gateway的spy换为真实工厂计数器，失败native仍0，不放宽断言。
- Browser21、App22(browser-use)/21(default)、Knowledge6、native state4通过。CEF返回事实
  与ack分离，丢ack仍失败/不可重入。真实guardian接入/系统Keychain/native live未完成，
  证据外置history-cursor-live，Windows新共享分段/消费者关闭未代验，首败保留。

### macOS guardian 接入与宿主关闭资格（2026-10-03）

- Browser工厂新增独立的 post-storage-close Rust入口：需显式storage-independent opt-in，
  不能升级已开始的ordinary flight；App只在真实存储关闭阶段后调用。默认实现仍走原关闭，
  Windows架构/结果不变。资格不是模型能力或权限，失败不会制造完成；平台23及App定向1通过。
- macOS加载CEF前启动私有受管guardian，真实内核身份/执行代/nonce登记Helper；正常返回、
  初始化失败及30秒native超时清理共用5秒预算。宿主消费者/存储已关闭、Helper执行代absent/
  guardian已join、Main仍在FFI才允许独立非零退出；不依赖被占用的主队列，不重入CEF。
- 本机真实Unix RPC首红发现对端关闭后setsockopt读超时EINVAL；改为绝对期限非阻塞poll，
  不取消peer/birth/path检查。确认丢失夹具以1字节响应前缀证明已接纳、正文不解码，再由
  独立Status核对；关闭前未接纳的首次失败保留，不以UID或请求中的PID替代内核身份。
- 原生生产server/client子进程四路径通过，macOS lib32通过、desktop/Helper/example编译
  通过。此层无CEF/Keychain/UI，不关闭真实系统等待、物理阻塞自退出/冷库或原B最终报告；
  模型0，完整首败/修后外置guardian-native，Windows验收不代判。

### closed-turn 历史查询的过期作用域（2026-10-03，macOS发现）

- 原B正式Low报告首败：旧history错误/空archive被原样重放，当前loader虽有schema却没有
  首次作用域状态说明；模型未查历史，错误报告实际可读的文件结果缺失。1请求/无新效果，
  Turn completed不等于完整Case通过，原B33记录与字节独立审计保持。
- closed-history仅给LOAD/SEARCH/READ回复加原turn/过期record-ID标签并保留原output/error；
  旧操作cursor仍由当前平台核验，不删除历史或赋予当前证据/执行权限。普通owner结果、
  archive原codec与checkpoint不改。已授权loader的首次reader说明与实际可见schema同步，
  激活后替换真实archive状态，无port不宣称可用、不新增自动请求或输出/上下文额度。
- 两首红后Runtime296通过、显式原B634events/33逐项原正文/error/source_binding完整回归
  通过；修后正式模型未验，Windows未代判。原B公开漏项仍OPEN，外证据history-upgrade-report。
- 随后Mac正式`30c274dee`修后采用检查仍FAIL：旧回复作用域标签进入摘要、当前reader说明/
  LOAD schema进入主请求，2请求/1主步仍没有历史查询，重复错误缺失结论。不归为包错版/
  schema不可见，也不由Turn completed关闭语义缺口；停止同机制付费循环。无新效果、五表
  封存/正常退出通过；证据history-scope-recheck，完整原B和公开质量保持OPEN，Windows未代验。

### 明确历史回合引用的受控模型前加载（2026-10-03，macOS先验）

- 旧摘要会重复错误缺失结论，只有reader说明不能保证查询。普通新回合对当前用户明确
  同Session turn:user地址，复用现有认证history port/完整codec/binding检查限量加载目标
  archive；无引用/无port/非法/foreign/current地址不读，非目标不导入，不新建权限或重放。
- 4引用共享8页/5秒，错误/循环/超时/取消不成为缺失或清理证明；当前已加载record/type/
  error计数与SEARCH/READ在摘要后仍同步，数据只作历史。checkpoint/recovery不改，
  原source binding与正文/error、原failed Turn保留，输出/上下文既有预算不扩。
- 两模型前加载子断言首红后Runtime300通过、原B634events/33完整回归通过；真实模型采用/
  完整B报告及Windows新入口未验，不能以定向回归关Case。外证据history-reference-load。
- Mac正式`ae24847f0`已实际预加载原33条/无淘汰/错误10并展示SEARCH/READ，6请求
  （5摘要+1主步）仍零正文查询，重复错误文件缺失结论，严格Case仍FAIL。原效果/prefix与
  五表冷封存、正常退出通过；不归为数据读不到或入口不可见，下一聚焦正文/完成路径，
  不再同机制文案复跑。外证据history-reference-load；Windows原结果不改。

### Session模型配置变更与数据历史兼容（2026-10-03，macOS确定性先验）

- 已保存路由不刷新，但Runtime读到新限制，会使普通上下文/参数修改被旧摘要校验阻断；
  下一本机空闲新Turn按原选模型解析、在Session写fence下CAS仅Chat绑定，其他Agent/
  资源合同不变。不兼容推理覆盖按已有模型切换规则清除，旧key保持原admission/完整
  输入冲突检查且不重放。远端、Attempt、active Turn和恢复绑定不自动改写。
- 受控工具archive接入已有host仅模型Snapshot兼容证明，默认false；完整journal、原
  binding、Session/runtime/build合同继续验证，只导入数据而非权限、恢复或新鲜性。
- App公开保存→下一请求四轮验证通过：模型切换、vision/1M/4096/medium、Session high、
  供应商negative reasoning后的参数撤销，原历史保留/资源不变/旧key无重放。archive4、
  引用3及fixture9通过，原301 Runtime基线复用；三次夹具/安排失败外留、不吞错改PASS。
  本批无真实模型请求或正式UI样本，active全局配置并发修改仍待闭环；Windows不代判。
  外证据2026-10-03/macos/step5-b09-closure/run-001-build。

### 运行Turn模型推理配置视图（2026-10-03，macOS确定性先验）

- trusted准备捕获本Turn非安全模型字段，预算/request共用，终态或durable失败暂停清除；
  每次仍用实时安全字段重建原exact digest，凭据/启用/URL/协议/连接不冻结或旁路。
  捕获前保存失配明确首调用前失败，仍有admission→capture小窗口，未称全并发丝滑。
- 三项定向resolver→target→wire回归及既有公开API四轮链通过：当前旧值、下一轮新值、
  八种安全修改/任务删除拒绝、无跨Turn视图。未调用真实模型或正式UI，不代判Windows。
  外证据step5-b09-closure/run-001-build，原完整报告/Keychain未因此关闭。

### 结清补丁历史不应阻断仅模型变化（2026-10-03，macOS正式首败）

- 原B切Step5后正式0模型请求即被旧patch state的Snapshot检查阻断；最后state已全空v2，
  本地问题曾投影成UNKNOWN_UPSTREAM，不能归因供应商。首败与1295/8/14封存保留。
- 仅完整source Session/operation/engine/terminal时序、无未覆盖后续patch派发、严格
  default v2且host证明仅模型变化时返回default；pending/legacy/预算超出/错源/无证明
  仍拒绝，不迁移checkpoint或恢复权限。真正不兼容恢复为SessionConfigurationChanged。
  三项定向回归通过；修后正式待验，Windows未代判。外证据step5-b09-closure。

### 明确旧回合引用的正文数据投递（2026-10-03，macOS正式缺口）

- Step5真实1请求仍错误文件缺失；实际输入继承旧seq1262摘要，仅helper receipt正文，
  file before/after/SHA/33/32及stdin13均缺。archive33 metadata不是33全文已送模型。
- 明确当前User同Session引用才投递已验证目标的quoted Assistant/Text正文；原binding/
  error/截断保留，非System/acceptedInput/当前证据或恢复权限。<=64KiB并复核原context
  byte/message预算，不扩大output。官方exact reader保留fixed-root、context/Agent floor、
  canonical closed source与完整codec；default不启用，authoritative拒绝不cursor绕过。
  共享8页/5秒及journal预算不增，取消直传。Runtime304/1ignored、host定向1通过；
  正式修后仍待验，Windows未代判。外证据step5-b09-settled，原失败/数据不改写。

### 有界历史投影优先操作结果（2026-10-03，macOS正式残余）

- 正文正式采用恢复file字节/hash/顺序，但64KiB先被planning参数占满，仅23/33、后段
  结果未入请求；模型stdin计数字符错误，完整报告仍FAIL。原数据/效果保持，模型owner0。
- 原预算内stable优先操作记录；标原插入index/非执行时序，原payload不改，selected/
  included错误计数和omitted IDs明确。stdin proposal的UTF-8+显式LF机械计数为derived/
  非owner-byte receipt，不能绕过before_tool或冒称实际写入。七项定向回归通过/1ignored，
  正式修后待验，Windows未代判。外证据step5-b09-body。

### 有界正文投影的来源去重（2026-10-03，macOS原数据先验）

- 同64KiB内明确省略control proposal参数、原archive/READ不动；完整结果text/error
  保留，来源字段无损提取sources并经record.source_identity解析，非新permission/证据。
- 原B634events/33真实结果显式回归逐项text/error/binding同、33/33正文全部纳入且
  omitted为空、owner0；七项archive回归通过。上轮正式仍未交付错误总数，首败保持，
  新正式采用待验/Windows未代判，派发参数字节推导仍非owner receipt。外证据step5-b09-report。

### 截断续写的公开回复展示（2026-10-03，macOS真实UI发现）

- 原B正式Step5两请求已实际采用33/33结果全文（原64KiB、全部10条错误），操作/字节
  正确且无重放；模型仍拒绝合计8次拒绝+2次命令失败为10，完整报告FAIL保持。
- canonical两段正文均持久化/非hidden，UI却把7277-byte首段折进过程区，仅显示1064-byte
  尾段。只读typed输出截断事件派生同Turn链接，renderer公开显示真实链，不重写旧正文/ID；
  不推断“续”文字/时间，不公开思考或工具前说明，缺前页明确提示。Rust2/UI101/type/
  desktop/i18n定向通过，含纯思考防误链接；`b78ac77a3`/同步合并`c7f02f926`正式同原数据
  零模型冷读已公开首/尾正文并正常Cmd-Q0/67.485秒。原1457events/12Turns/14effects及
  源/备份五表全同、双integrity/无自有PID或listener；不证明新生成语义或流中历史刷新前
  展示。Windows仅共享代码影响，原平台结果不代判；外证据step5-b09-complete与continuation-cold。

### 历史来源错误账本的集合口径（2026-10-03，macOS原B发现）

- 完整33结果已投递但模型将8非exit2与2命令失败当不交集合；原typed WorkStatus
  最后seq1015实际为工具结果失败10/命令失败2。只在validated source journal的有界
  历史metadata保存最后typed计数及原身份，作为明确引用的quoted data，绝不替换当前
  Turn const/work/evidence。说明工具is_error含非零命令flag，命令观察独立、可重叠不相加；
  无status未知、省略不减ledger、不解析owner正文JSON、不硬编码个案数字。
- archive10项及显式原B634/33 journal1项通过，原64KiB内56445bytes全部33结果与10/2
  来源账本一致；`da09563a0`/同步合并`919c685bf`正式原B报告恢复1请求/1步通过，实际33/33
  及typed10/2入模型，8084-byte公开完整结果/UI与源记录相同，无owner重放，当前账本0/0。
  Cmd-Q0/89.201秒无expiry或信号、1493events/13Turns/14effects及旧前缀/效果/闭库同。
  原failed Turn/旧FAIL保持，非首发或N3认证，Windows未代判。外证据historical-accounting。

### 公开语言策略与原始结果载体分离（2026-10-03，macOS正式叙述残余）

- 补普通公开叙述的opaque process/resource句柄与引擎规划/计数术语边界：共享policy
  合入原MINIMAL同一instruction，公开字段描述引用，默认用户语言说明动作/结果/原因。
  技术原文/标识显式例外，源值/stdout/argv/hash/PID与数字/required/enum/证据资格均不改，
  不用regex清洗、不加词表拒绝或权限。直接Assistant与structured completion共用policy。
- 三项机制及五项邻近精确载体/回放/计数回归通过；`11ecee9c4`正式同原B报告一次采用，
  1请求/1步、policy在system0一次，源33结果/10含2保持；新正文仍照抄历史引擎术语，
  表达FAIL，不以机制通过关闭模型表达。Cmd-Q0/70.066秒无expiry/信号，1525events/
  14Turns/14effects与原前缀/效果/闭库同，owner0。未改根因不继续付费循环，Windows
  原结果不代判，外证据public-report-language。

### closed replay模型回答不是实际结果或新报告模板（2026-10-03，macOS真实输入）

- source已有33结果/10含2及语言policy，新回复仍沿用107/111旧报告行；旧plain Assistant
  11份42278B没有与owner结果区分。仅closed typed公开文本/验证后完成文本包装低信任模型
  回答，original_text/否定/原owner值保留；未知compaction来源null，不改User/ToolResult/
  accepted输入、当前ledger、权限或checkpoint。现有历史controls数据仍data，不授真伪权威。
- history9项及checkpoint/explicit引用两项通过，实际模型采用/表达待验；原失败与Windows
  结果保持。外证据closed-assistant-provenance，不能用公开文本0的原B634 journal冒充覆盖。

- `1afc2e2cf`/同步合并`b8b71673b`mac正式原B报告1请求/1步已实际采用12旧答案标签，
  50331bytes/顺序与多重集合/typed source及unknown=null全同、33源结果/10含2/当前0保持。
  B事实PASS，公开表达仍FAIL；不改原文/结果/失败来制造通过。Cmd-Q0/243.169秒（含锁屏
  等待，无expiry/信号），1557events/15Turns/14effects及旧前缀/效果/闭库保持，owner0。
  Windows未代判，原待验为该批历史状态；后续未改表达根因不付费复跑。

### pairing后台清理纳入现有宿主收尾（2026-10-03，macOS真实退出发现）

- 真实Keychain堵塞时closedpool后timer仍query：App丢弃Handle、interval不接取消。仅该
  producer接既有shutdown token和BackgroundTaskRegistry，取消停止新sweep、已发query
  结束后join，原失败/超时语义不吞。周期与取消/在途两个小回归共3通过，App现有drain1
  通过，修后正式待验。
  外证据pairing-exit-owner；不把macOS出现的共享缺口当Windows已验。

- mac正式原B样本243.169秒（含解锁等待）跨60秒且无pairing closedpool警告；实际App
  build_channel_state注册timer→join/registry Closed0→真实pool close→native gate持续持有并
  跨两个周期的回归1/1通过。只补test-util/cfg(test)装配入口；初编/SQLx暂停初始化首败
  外留后调整测试时钟阶段，原断言不减。它是修后barrier证据，不冒充新的自然Keychain样本。

### 可选历史交付来源与当前完成账本分离（2026-10-03，macOS表达后续）

- historical_results以明确source_turn/archive_id选择已验archive，host解析原文/flags/
  绑定/来源计数并冻结v3展示；不接受model data、不进current evidence/const。可选暴露现
  plan/report控制，不强制普通历史答复走Ledger；选plan后原权限/进程/recovery/终态输入
  fence及完整当前任务覆盖保持。未知诊断保留，v1/v2旧字节不改，公开8KiB与快照48KiB不增。
- Runtime319/1ignored、原B634/33黄金1及可选控制正负路径通过，原direct答复仍可用。
  正式采用/表达待验，旧FAIL及Windows结果保持，外证据historical-public-delivery。

- `577ad3f44`/同步合并`88eeff987`mac正式原B同task1请求/1步，plan/report和33历史
  selector实际入请求，但模型仍直接回答，结构交付NOT_SELECTED/公开表达FAIL；事实与
  原10含2保持。Cmd-Q0/96.832秒，无expiry/信号，1593events/16Turns/14effects及旧前缀/
  效果/闭库保持，owner0。不强制control或改task伪造通过、不重复同根因；Windows未代判。

### 历史报告专用的收窄入口（2026-10-03，macOS未采用后续）

- 可选v3未选择的首败保持；仅latest User普通指令五条件合取+非引用同Session精确源+
  已认证closed/loaded目录时进入报告only，引用/JSON/普通问答/许可当前操作/后续撤销不触发。
  只收窄原动作，当前plan/report与history controls；无权限/freshness或旧任务继承，真实
  blocked仍允许缺项，当前证据/计数、running/pending恢复与终态input fence不变。
- 两classifier与控制正负strict1通过，Runtime321/1ignored，正式采用待验、原表达FAIL
  不抵扣，Windows未代判；外证据history-report-entry。

- `3436e40c5`/同步合并`5937b5bcd`mac正式1请求/1步Specific(update_plan)真实采用，
  provider却返回report_completion/半截参数JSON、finish=tool_calls；精确协议拒绝且owner0，
  pause后用户UI结束回合保留cancelled/no report。FAIL_PROVIDER_SPECIFIC_TOOL_CHOICE，不放宽
  校验或补参数；Cmd-Q0/100.227秒无expiry/信号，1625events/17Turns/14effects及旧前缀同。
  原表达FAIL仍开放/Windows未代判，外证据history-report-entry。

- strict Specific首阶段选面仍含未来report的共享缺口已收窄为单一当次控制，下一边界
  按原schema重建，plan→report，不接受错tool或补JSON。正负strict回归通过，修后正式
  待验，原失败保持/Windows不代判，不立即重复同场付费请求。

- `d9c2113e9`mac单一plan→单一report真实采用：4请求/4步、plan接受，report先因旧ID
  current evidence/缺historical_results被schema拒绝，随后provider半JSON且tool_calls终止，
  protocol pause保留，UI结束回合cancelled，无report/owner0。FAIL_NATIVE_ARGUMENT_JSON
  AFTER_REPORT_SCHEMA_REFUSAL；1693events/18Turns/14effects及旧前缀同，Cmd-Q0/165.832秒
  无expiry/信号，完整外证据history-report-single-control。不补JSON/放宽合同或重发洗绿。

### strict历史报告的紧凑输入合同（2026-10-03，macOS参数混淆后续）

- 仅严格已验历史only任务用source_turn/archive_ids/short_summary/missing_items四字段。
  host fresh标准submission仍保原全accepted scope/current真实counts，无evidence unverified
  不作完整proof；missingblocked，最新User撤销/多源/unknown/foreign/注入旧字段/data/预算
  错误拒绝或退optional，一般report schema和JSON协议不变。source账本不进current证据。
- 新2及控制1回归、Runtime323/1ignored、原634/33source黄金1通过；正式采用/表达待验，
  原失败/Windows结果保持，外证据history-report-compact。

Windows独立发现strict宿主语言与持久化格式判据不一致：旧中文输入会使英文实际摘要的label/
rationale变中文。仅将宿主字段语言对齐最终摘要（含既有missing文字）的格式判据，摘要/缺项
原字节、最新来源、当前计数及权限不改；首次red及缺项分支red保留，strict2项+路由1项通过。
真实模型采用未由该组件代判，详见Windows页宿主语言修复及原会话未准入记录。

Windows原W250三项历史请求尝试沿用原模型/原data/work；UI点击后无新接受/Turn/模型请求，
原943events/763前缀/4Turns/8effects/9文件保持，提交HTTP缺证、原因未证。原观察器误读旧
completed Turn已保留并纠正，未重启/重发；实际API exit0/进程监听清理，正式采用仍OPEN。

- mac正式紧凑合同尚未到达：3个ONLY update_plan请求，模型先提出未暴露旧report后无调用，
  守卫拒绝保持，最终任务未完成，当前1/0与source10含2分开。原前缀/19Turn/14effects不改，
  native正常0；不把组件通过或未采用记公开报告PASS。
- fresh纯历史报告去除多余plan模型往返，报告沿已有optional空plan验证/关闭；真实
  needs_replan仍plan-only。全部scope/source/current/patch/process/exact/预算检查不放宽，
  最小首红→修后单请求+stale-plan/optional/current伪证负向及Runtime323/1ignored通过。
  新正式采用待验，外证据history-report-direct；不代判Windows或旧失败。

- direct正式首红定位初次AGENTS discovery对不存在空plan误抬needs_replan，2请求仍plan、
  无模型call/无report，旧数据与效果保持、native0；不以正文宣称完成代判采用。
  增加非空AGENTS授权read真实接线最小首红，再沿已有context空plan规则修复；已存在plan
  或steering/recovery gate仍要求replan，规则内容/权限无变化。host2reads/modelowner0、
  strict单请求与stale-plan及Runtime323/1ignored通过；新正式待验，证据history-report-instruction-entry。

- mac修后正式首请求起ONLY四字段report，初次AGENTS接线及compact原生调用实际采用；
  6请求中后4次均validJSON但16记录超原8KiB全部拒绝，有界终态失败，未交付/表达未通过。
  旧prefix/Turn/14effects同、无modelowner、native0/242.247秒；不扩预算或删必要值取绿。
  后续仅离线选择/预算定位，原A/B结果与Windows边界不改，完整外证据history-report-instruction-entry。
- 同时去除strict上下文继承的optional direct-answer/旧historical_results指示，仅替其catalog
  说明为当前4字段合同；普通optional不变，实际请求无冲突说明回归通过，未再付费验证。
- 历史超8KiB拒绝反馈增加完整所选projection精确总字节与8192限制/未发布/禁止重复同选择，
  当前合同blocked缺项路径不变；无裁源/扩预算/自动完成。精确反馈与无发布回归1通过，
  Runtime324/1ignored。实际模型修正/完整表达未验，未再付费循环或改Windows结果。

### 完整历史报告与服务商默认输出（2026-10-03，用户变更容量要求）

- 不再要求完整报告适应8KiB/16条摘要限制。host retained历史选择≤128、正文/解析快照
  使用已有native256KiB envelope，仍在1MiB事件存储下；模型原生参数48KiB/source/current/
  权限/缺项/错误保护不变。原33results全部submit/快照/中文render/回读精确通过（69,250/
  22,661B），不靠删必需记录取绿。旧超限/fixture首错分别保留；正式采用待验。
- 默认输出不再因context预留量强制4096；配置wire Option独立，无limit按provider默认，
  已配大上限不被16K/ctx÷8截小；UI去除unknown required协议4096猜值、保留custom，
  未知required明确缺配置。Runtime327/1ignored、route1、UI17/boundary/i18n、fixture10通过。
  actual model request None/100K已断言，explicit输出完整预留输入窗口；首次夹具显式100
  导致default断言红保留，case明确unset再通过，caller自定义限制未抹去。
  StepFun官方Chat max_tokens默认INF已核对，实际默认wire与完整报告合并一次正式验证待验，
  外证据history-report-complete-capacity，不改Windows原生验收结果。

- `dd4ee03eb`mac正式UI保存Step5output4096→默认null成功（其他配置不变）；原task一次
  accepted/started后约3分钟无executionclaim/runtime/模型请求，UI Stop取消一次。native
  收尾owned_runtime_teardown未证明，保护exit1/426.487秒、noexpiry/无外部信号，非CEF。
  最终1880events/22Turn/14effects，旧prefix/效果同、双库ok；不以组件绿代判actualwire/
  完整报告/变更后丝滑使用。新preexecution和teardown缺陷窄定位中，Windows结果不改。

Windows发现显式wire ceiling与内部预留不一致：context8192/output8000原被接受，实际只
预留7168，输入/输出/安全量合计超过context。from_limits和caller override首红均保留，
现拒绝不能完整预留的配置，不改wire值、不合成4096；邻近6回归通过，不归因或重开W276。

Windows另发现完整历史增强沿用v3而改变正文，升级后合法旧v3会被matches_delivery拒绝；
新增强输出改v4，v3只接受pre/post增强两种已知精确派生，canonical旧文本不改。v4同时
保留合法cmd脚本文本，原v3不追增字段。首红保留，历史8/strict2/相邻回放1/路由1通过，
真实旧会话/新v4模型与UI采用仍未验，不改原复杂失败和平台边界。

- 配置后warmup旧Snapshot/newproviderrevision→send新Snapshot/cache复用反例已在既有
  router四回合集成复现，preparing无terminal首红保持。自动binding实际变化时，在已有
  idle/writefence先prove旧runtimeclose再replace；失败保binding/quarantine，无新Turn。
  同集成修后及unsetoutput→warmup→send actual省略max_tokens通过，正常shutdown原断言
  保持；不放宽Snapshot/权限或吞cleanup错误。mac正式修后待验，Windows未代判。

- mac`4028c917f`修后正式executionclaim/默认outputwireomit成立，33/33历史report accepted/
  delivered/completed，首摘要超长拒绝计current1/0保持；1945/23/14旧前缀/效果同，native0。
  原B核心来源全部精确，公开资料未丢，但表达普通叙述内部字段仍FAIL、完整UI滚动未验。
- known历史文件read直接LF正文及已知调用/cleanup值可读；strict-only prose作为待验草稿
  不发布第二份完成，validatedhostreport才交付，普通/optional仍原路径。新摘要内部字段
  明示拒绝、最新User技术要求/rawsource豁免不改写。定向回归通过，新正式待验，Windows不代判。
- 新presentation使用历史v4，旧v3回放保持；Runtime327/1ignored通过。字段词门禁仅明示
  有限6词而非全语言完整proof，最新User出现豁免边界不外推。新正式完整UI/表达仍待验。

Windows另发现词门禁误拒合法已选来源路径reports/reaped/result.txt、reports/state=ready.txt。
仅在校验视图中豁免完整已解析路径/明确引用的完整调用或诊断span，提交原文不改；真实自由
字段叙述、假URL和无来源引号仍拒绝。首红保留，正负路径及strict来源/缺项回归通过，正式
模型采用未由组件代判，详见Windows页原样来源误拒修复。

Windows按新warmup根因的修后正式样本已executionclaim/旧参数请求1，但新配置保存后UI
插入未接纳、流暂停，没有下一Turn；冻结两项仍未验，组织首败保持。实际API exit0/清理
成立，非托盘；不由此宣布W276全通过或保留的原原因已定位，详见Windows页配置正式样本。
- macv4正式4个report各33selector，但625/521chars超strict512全拒绝，有界失败；
  no公开草稿/owner0、旧prefix/effects同/native0。用户要求最终完整，strict摘要对齐
  普通report2048而不删必要解释，600字原样回归通过；source/48KiB参数/缺项不变，首败保持。
- mac98e16同原task33accepted/current0/0/draft0/25335B，max_tokens省略；实际UIafter全文/
  哈希和父子STOP/清理已见。锁定等待+滚动超native480触runnerTERM，expiredtrue/exit0
  仍observerFAIL。source/backup2059/25/14同、旧prefix/效果保留；正常exit另待零模型验。
- 普通摘要仍含工具/回执字段，表达FAIL保持；明示reject扩已知字段并一次反馈所有词，
  不裁/重写原值或原source。组件通过不代判修后模型，Windows结果不改。
- mac最新2请求遇recorder stream/trace失败502，第二SSE无finish/DONE/native参数未完；
  pause→UI取消，normalCmd-Q0，旧prefix/effects保持；不能补JSON或称完整provider协议输出。
  同正式包零模型单批冷读8屏/正常exit0/93.704秒、五canonical表原行不变，必要文件/哈希/
  helper/Stop可达。关闭冷读/正常退出子断言，不洗旧expiry，修后最终表达实际采用仍待验。
