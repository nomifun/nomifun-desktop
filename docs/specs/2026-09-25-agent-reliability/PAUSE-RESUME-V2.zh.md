# 暂停、授权续期与结果核对实现说明

日期：2026-09-25。本轮按用户要求不执行测试、编译验收或真实模型评测；这里记录实现约束和待验收点。

## 数据与权限模型

- 暂停不是 completed/failed/cancelled。canonical Turn 保持非终态 `running`，单独保存暂停版本/原因/清理证明；Session head 投影为 `paused`，保留 active Turn。
- `turn/paused` 提升 fence、停止 lease，旧 producer 不可再次写入。普通发送不能越过仍然存在的 active Turn。
- `POST execution/pause` 请求在下一个已结算边界暂停；立即中止仍使用 cancel，cancel 不会变成可恢复暂停。
- `POST execution/resume` 仅接受认证 Session owner。请求绑定 operation、pause revision、checkpoint revision/digest、idempotency key 和显式预算增量。
- 模型工具没有调用这些授权接口的能力。恢复不改 Snapshot、能力代次、原输入或任务范围。旧版已写 terminal 的 failed/cancelled/completed Turn 不重新打开。
- 预算有每次增量和累计硬上限；用户授权才增加额度。不会通过清零累计步骤/日志计数来伪装新预算。

## 核对后续跑

- 先读取 canonical checkpoint 后的完整尾部和真实 owner effect receipts；冻结 head seq，提交时 CAS，期间任何新输入/核对/取消都会使旧准备结果失效。
- 已有 returned/rejected 回执直接作为历史事实；不会再执行相同工具。
- 未决效果不能从模型文字或超时推断成功/失败。会话所有者可通过独立核对 API，针对 exact effect/input digest 提交明确、可审计的人工核对结论和证据摘要。
- 人工结论标为 owner attestation，不伪装自动探测或原始执行回执；每个 effect 仍由原有 effect 生命周期约束。
- 对工具已准入但结果未进入 Runtime 记录的情况，生成的是“核对观察”，不是再次 ToolStarted/执行。
- 未准入的提案不执行；不完整的未执行模型批次显式丢弃；已执行效果所在批次不得整体丢弃。
- 已 applied 的用户补充指令从尾部纳入 checkpoint，保持 receipt 顺序；accepted-but-unapplied 的输入仍由原 inbox 恢复排队。
- 计划标记需要重新确认，旧 completion/command evidence 失效；实际工作区/规则在恢复后重新观察。

## 待测试验收点

1. 暂停无 terminal；普通发送被阻挡；cancel 可终结暂停；重启不会未经授权自动恢复暂停。
2. pause/resume 重复请求幂等；错误 owner、旧版本、旧 digest、权限代次改变、已终结 Turn 均拒绝。
3. 相同授权只增加一次预算；额度不足/累计硬限/未显式确认 stalled retry 不开始模型请求。
4. SDK cleanup → canonical pause → Finish(paused) 顺序正确；老 relay 不能终结新 generation。
5. 已执行效果且无 Runtime result 的尾部，用真回执补观察；一个副作用仍只执行一次。
6. 未准入/截断提案、交错批次、缺失调用/未知效果/未证明清理必须阻止不安全恢复。
7. 人工核对要求 exact identity + digest + 明确风险确认，保留证据摘要和认证主体；不能由模型声明替代。
8. applied steering 尾部保持顺序、附件与来源；并发取消/新输入使 prepared resume CAS 失败。
9. 日志和 payload 授权增量与 reader 上限一致；累计用量不归零；极大单批次仍有预留收尾空间。
10. 新旧迁移、默认未分段调用、已有纯文本/工具/协作流程兼容；旧终态不被悄悄复活。

本文件不是测试通过报告。实现期间若边界有调整，以实际代码及最终交接补充为准。

## API 使用顺序

路径前缀均为 `/api/agent-sessions/{agent_session_id}`，沿用认证 Session owner；这是 API-first 交付，没有新建图形管理面板。

1. `GET /execution`：读取 operation、pause、checkpoint、model_progress、budget 和未决数量。state=paused 是执行投影，底层 Turn 仍为非终态 running。
2. `POST /execution/pause`：提交 operation_id、idempotency_key、reason；通常在下一静止边界暂停，立即停止用原 cancel。
3. `GET /execution/effects?operation_id=...`：读取未决 effects 和缺失 Runtime 观察的 invocation 摘要。owner_receipt_available 不代表执行许可。
4. 核对实际结果后 `POST /execution/reconcile`，绝不能从模型自述填写 verified=true。

```json
{
  "operation_id": "从 execution 读取",
  "expected_pause_revision": 1,
  "idempotency_key": "核对请求唯一键",
  "effect_id": "候选 effect_id",
  "expected_input_digest": "候选 input_digest",
  "outcome": "confirmed_succeeded",
  "evidence": {
    "verified": true,
    "evidence_digest": "真实核对记录的64位十六进制摘要",
    "reference": "回执或核对记录ID，不放凭据"
  }
}
```

失败用 confirmed_failed，这不是安全重试授权。没有 effect row 时可用 call_id 替代 effect_id（后者省略），摘要取 unresolved_invocations.expected_input_digest。
invocation 核对不会清除已有 unknown effect rows；关联效果仍需结清。

5. `POST /execution/resume`：

```json
{
  "operation_id": "从 execution 读取",
  "idempotency_key": "本次授权唯一键",
  "expected_pause_revision": 1,
  "expected_checkpoint_revision": 8,
  "expected_checkpoint_digest": "读取到的精确摘要",
  "budget": {
    "additional_segments": 0,
    "additional_journal_mib": 0,
    "additional_payload_mib": 0,
    "retry_stall_guards": false
  }
}
```

示例值必须替换为当前状态；已有额度足够时增量可以为 0。cleanup_proven=false 时另需 cleanup_attestation，形状同 evidence，证明相关资源确实已关闭。
每次最多增加 16 段、16 MiB journal、32 MiB payload；累计最多 32 段/4096 步、64 MiB journal、128 MiB payload，不是无限续期。
相同幂等键必须对应相同请求；重新暂停后用新键和新 pause revision。

## 收尾语义与附加验收

- 旧 canonical completed/failed/cancelled 不重开。只有 canonical 仍非终态时，未提交成功的 Runtime 私有 terminal proposal 才可被 owner reconciliation marker 取代。
- 模型/传输中断、恢复准备的附件/资源失败可保持暂停。没有可用 checkpoint 的初始错误不伪装可恢复。
- 清理失败记录 cleanup_proven=false，宿主 transport 仍 quarantine；某些旧资源 owner 需要核对后重启，不能只改标志声称清理成功。
- 旧 tool/Kernel scope 只在 cleanup 确认且 generation 改变后重开；模型/压缩编号、总用量不归零。
- pending 人工核对先记录有审计前驱的 uncertainty，再 reconciled，不声称原始首次成功；实际 owner 晚到回执与人工核对的竞争需验收。
- 普通候选历史仍为 32 MiB，有授权的大 Turn 才扩大，reader 绝对上限不等于默认加载量。

追加待验收：channel/robot 不误报成功、UI 队列保持暂停、重复暂停的消息 ID、准备失败再继续、私有 terminal commit 丢失、实际回执/人工核对竞争、取消后结清但不复活、无 checkpoint 拒绝、版本/digest/预算/owner 伪造。
