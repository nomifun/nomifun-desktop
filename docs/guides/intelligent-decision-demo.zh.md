# 智能决策 dev Demo（StepFun Coding Plan）

这个 Demo 使用当前源码的 dev/test 构建，启动完整的 NomiCoreApplication，经过正式
Provider、AgentPreset、AgentSession、Turn、消息查询和 IDMM API。主 Agent 和旁路模型
均固定为 `stepfun-plan / step-3.7-flash`，成功的生成只请求
`https://api.stepfun.com/step_plan/v1/chat/completions`。

每次运行使用新的临时数据目录，退出时关闭 Runtime 和后台服务并清理数据。
不会连接、修改或清空正在使用的桌面 dev 数据，也不要求先启动 `bun run dev`。
这是后端整体流程 Demo，不包含桌面点击或截图验收。

## 运行

在仓库根目录用 PowerShell 设置本次进程的环境变量（不要把真实 key 保存到脚本）：

```powershell
$env:NOMIFUN_LIVE_STEPFUN_API_KEY = Read-Host 'StepFun Coding Plan API key' -MaskInput
try {
  bun run demo:idmm
} finally {
  Remove-Item Env:NOMIFUN_LIVE_STEPFUN_API_KEY -ErrorAction SilentlyContinue
}
```

`-MaskInput` 适用于 PowerShell 7。Windows PowerShell 5.1 可以使用仓库已有的
Windows Credential Manager 入口，隐藏输入并保存一次，然后重复运行：

```powershell
powershell.exe -NoProfile -File scripts/validation/run-nomi-core-live-provider-from-windows-credential-manager.ps1 -Setup -IdmmSmoke
powershell.exe -NoProfile -File scripts/validation/run-nomi-core-live-provider-from-windows-credential-manager.ps1 -IdmmSmoke
```

默认生成忽略入 Git 的 `.tmp-idmm-demo-report.json`。指定另一份报告或只编译：

```powershell
bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --idmm-smoke --report .tmp-idmm-demo-report.json
bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --idmm-smoke --compile-only
```

runner 在不含 key 的环境里运行 Cargo，编译后只把 key 经 stdin 交给测试进程。
Provider 使用正常加密存储；关闭后还会检查临时目录中没有明文 key。
控制台仅展示固定状态、时间和计数；报告包含本 Demo 的有限对话内容。

## 场景与验收

| 场景 | 触发 | 必须观察到的效果 |
| --- | --- | --- |
| `off` | 主模型生成格式选择问题，IDMM 关闭 | 后台等待及手动检查均不介入，无 `origin=idmm` 回合 |
| `manual_rule` | 300 秒后台间隔内调用“立即检查” | 自动回复推荐项 `2`，主模型输出 `IDMM_RULE_DONE_HTML` |
| `background_rule` | 主模型生成问题后自然结算，不调用 evaluate 来触发 | 后台自动回复 `2`，主模型完成；覆盖后台枚举、终态通知及周期巡检 |
| `sidecar_question` | 主模型询问最终标题，用户偏好已在当前输入中明确 | 真正调用 StepFun 旁路模型，答案包含“缓存观察实验”，主模型继续确认标题；旁路失败允许默认上限内重试 |
| `sensitive_halt` | 主模型请求 API key（只有测试文字，没有真实 key） | 记录 `sensitive_input_required / halted`，不调用旁路、不生成恢复回合 |
| `provider_pause` | 受控网关返回 503，Broker 重试耗尽产生原生暂停 | 验证当前 IDMM 供应商恢复分支没有接入此状态，再经正式 resume API 恢复同一 Operation；拒绝过期 checkpoint，重复 resume 不重复执行 |
| `stalled_recovery` | 首次模型请求保持静默；静默阈值设置为 30 秒 | 后台取消原回合，创建一个 IDMM 回合，真实模型继续并完成 |

所有问题、回答和恢复结果均来自正式 Session 流程，没有直接插入消息投影或修改
Turn 数据。前五个场景的模型输出也不是 mock；因此模型不遵循提示或供应商出错时
Demo 会失败，而不是修补答案使其通过。

为了重复制造故障，Demo 启动一个仅监听 `127.0.0.1` 的临时网关。它只向固定的
官方 Step Plan 上游转发请求，禁止 HTTP 重定向；故障场景中的 503 和静默由该网关
注入，不能解释为 StepFun 本身发生了故障。静默的首次请求不会发送给远端。

验收不仅检查 IDMM 的 `Succeeded` 记录，还从 canonical `turn/started` 的
`source_message_id` 找到 `origin=idmm` 的 accepted input，并检查同 Operation 的
`turn/completed`，同时核对实际投递的 `2` 或标题。
重复 evaluate 后必须仍只有一个恢复回合。Demo 不授予工具，预期没有工具效果；
完成标记说明主模型已接收决定并结束本轮，不能当作文件保存或外部操作的证明。

## 查看结果

控制台每完成一个场景输出：

```text
idmm_demo_case=background_rule status=pass elapsed_ms=... idmm_turns=1 sidecar_calls=0
```

整次运行只有在七个不同 Session 的预期行为、应用关闭和凭据检查通过时，才
输出 `live_smoke_status=pass`。报告使用 `status=pass_with_limitations`，并明确标记
`provider_pause_requires_explicit_native_resume`，不能将其解读为 IDMM 已自动恢复 503。
场景失败时保存已有场景、有限诊断与固定错误码，
报告的 `status` 为 `fail`。返回码非零就是失败；编译或启动前失败可能未更新报告，
不要用已有报告判断一次失败的新运行。

报告含每个场景的 Session ID、耗时、canonical Turn 身份与来源、完成回合数量、
介入记录、旁路调用次数、注入故障次数及有限消息内容。可用以下命令查看：

```powershell
Get-Content .tmp-idmm-demo-report.json -Raw | ConvertFrom-Json |
  Select-Object -ExpandProperty cases |
  Format-Table case, status, elapsed_ms, idmm_turns, sidecar_calls
```

定位失败时，优先看固定的 `phase/code/status`，不要启用会打印 HTTP authorization
或完整请求体的调试日志。默认总预算 30 分钟，包含首次编译；每场景有单独等待上限。
无供应商故障时，七个场景通常主要等待真实模型生成与 30 秒静默验收。

## 实现入口

- `tests/support/live_idmm_demo.rs`：场景、真实网关、正式 API 与 canonical 回执验收。
- `scripts/validation/run-nomi-core-live-provider-smoke.mjs --idmm-smoke`：凭据隔离、编译、执行及报告。
- `scripts/validation/idmm-demo-evidence.mjs`：报告完整性检查。
- `nomifun-idmm/src/service.rs` 的后台回归测试：不调用手动 evaluate，验证后台能发现已启用会话。

后台枚举使用 SQLite `GLOB` 的 `*` 通配符。该修复是后台场景真正能够自动触发的
前提。其他语义安全和并发边界仍见 [智能决策说明](intelligent-decision.zh.md)。

## 实测发现的供应商暂停边界

当前 Runtime 在可恢复的供应商故障上保留 native checkpoint，并提交 `turn/paused`。
该回合的 `agent_turns.state` 仍是 `running`，Session head 为 `paused`。IDMM 的
`ProviderFailure` 分支只处理已失败回合，因此不会直接执行原生 resume。

`provider_pause` 场景关闭静默恢复来隔离这个分支，等待真实后台检查，验证没有
新 IDMM 回合，随后由 Demo 的 owner 操作正式 `/execution/resume`。resume 请求
携带精确 Operation、pause revision、checkpoint revision 和 digest，不改写模型、
Snapshot、Session 或输入事实。恢复后必须在原 Operation 中完成，不新增 Turn。

如果产品配置同时开启静默恢复，另一条静默分支可能在阈值后取消回合并提交新输入；
这与原生 checkpoint resume 是两种行为。本 Demo 不把它混称为供应商故障自动恢复。
