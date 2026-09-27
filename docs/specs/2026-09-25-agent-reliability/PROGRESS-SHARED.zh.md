# 跨平台共享 Case 处理进度

更新：2026-09-27。调度规则见 [实施计划](IMPLEMENTATION-PLAN.zh.md)。
本表管理 675 个 Both Case 的公共根因；Windows/macOS 产品验收分别记在各自文档。
本轮 S-P0-01～05 已完成共享子断言的走查、修复与回归；不等于 675 条完整 Case 全部通过。

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

## 全领域公共队列

下表 Case 集合 = 实施计划对应领域中平台为 Both 的所有 ID；675 条各有且只有一个主领域。

| 队列 | 共享 Case 数 | 测试 → 排查 → 修复任务 | 后续门槛 / 状态 |
| --- | ---: | --- | --- |
| S-D01 | 85 | 工具/模型协议、注册/激活与版本；先找 Schema/admission 断点，再做生产 owner 修复 | P0 本轮；完整传输/故障矩阵待走查 |
| S-D02 | 70 | 控制状态、完成证据、事件和真实体验；保留首次失败，核对 canonical/UI 一致性 | S-D02-01 当前证据约束已补；Windows GEN 真实完成报告仍 FAIL，未关闭 |
| S-D03 | 45 | 文件/Artifact 合同、原子边界、source digest、负向隔离 | S-D03-01～03 原子 write、删除、名称、批次准备与错误归因在 Windows 定向回归；macOS/完整入口待验 |
| S-D04 | 120 | 公共 command/args/cmd Schema、进程 owner 与清理协议 | P0 边界本轮；原生实现转 W01/M01 |
| S-D05 | 27 | Git/SSH 授权与副作用核对；独立 remote/host 夹具 | 条件资源准备后走查，禁止共享生产 remote |
| S-D06 | 67 | Skill/MCP/Plugin/Browser/Computer 的发现、冻结与生命周期 | Skill 本轮；其余条件资源待走查 |
| S-D07 | 53 | 领域 owner/cardinality、跨实例与精确目标绑定 | Canvas/PAL 入口本轮；S-D07-01 修复画布名称上下文，其他平台/完整集合待验 |
| S-D08 | 103 | 五类 Agent 的产品入口与目标能力；逐角色验证，不互相代替 | 首批 Windows 四条路径已有结果；完整矩阵待走查 |
| S-D09 | 75 | 恢复 fence、取消、并发、压缩、预算与长稳；按状态边界注入故障 | P1 故障验证后安排 LONG/soak |
| S-D10 | 15 | PORT-001～015 内部端口、outbox、generation 与外部 grant 隔离 | P0 映射后逐端口确定性回归 |
| S-D11 | 15 | 权限/资源/旧快照/撤权/secret 负向，验证拒绝前无副作用 | 本轮只验关联静态与资源断言；竞态仍待走查 |
| **合计** | **675** | 只统计共享 Case 定义 | 不增加 4,740 个平台结果槽 |

## 历史修复与未关闭项

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
