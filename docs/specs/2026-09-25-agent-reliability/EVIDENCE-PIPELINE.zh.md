# 独立验收与实测证据流水线

状态：2026-09-25 源码实现；本轮未执行 freeze/record/aggregate、grader、测试或真实模型调用。

## 文件与阶段

- `scripts/validation/agent-reliability-collect.mjs`：冻结清单、运行固定的独立断言程序、签名保存单次结果、聚合。
- `scripts/validation/agent-reliability-report.mjs`：已有统计门禁。采集器直接复用，不另造“成功率”算法。
- `crates/backend/nomifun-app/tests/common/native_reliability_capture.rs`：供产品 fixture 调用的导出辅助函数，无测试函数，也不会自动调用模型。

流程为：先冻结计划 → 在真实产品路径执行预定任务 → 导出 canonical capture → 在 Agent 工作区外运行独立 grader → 保留签名回执 → 聚合全量计划 → 统计报告。
capture 辅助函数必须由测试 harness 明确传入 `LiveProvider` 或 `ScriptedProvider`；后者不能进入 live 统计。
不要把 fixture 的编译、脚本 provider 或模型自己的 report_completion 当成独立实测成功。

## 冻结计划

计划使用现有 report 输入中的 `schema_version:1`、`suite_id`、`runtime_build_digest`、`model`、`strata`、`scheduled_trials`。
每份 suite 必须固定一个精确模型标识，分别统计不同模型，不能混合模型以取得一个总通过率。最初实现只接受 `step-3.7-flash`；2026-09-25 接手验证中按用户最新要求扩展为逐模型验收，原有置信界、失败分母及构建隔离不变。在开始任务前记录实际安装的 runtime build digest，不用 Git commit 代替 runtime binding digest。

每个 stratum 另外声明：

- `max_model_steps`：正数，不超过 4096。
- `max_compaction_requests`：0..2048。
- `max_resume_authorizations`：0..64。
- `allowed_terminal_kinds`：默认仅 `turn/completed`。暂停是非终态，不自动成为成功；预定取消用例可以明确允许 `turn/cancelled`。
- `allow_owner_reconciliation`：默认 false。需要人工核对的运行不能混进未经说明的全自动稳定性结论。
- 可为每个 scheduled trial 提供 `input_sha256`，其口径为实际接受的主输入文本的 UTF-8 SHA-256。

计划的 `grader` 包含：

- `program`：独立 verifier 的绝对可执行文件路径，不允许 cmd/powershell/bash 等 shell 作为入口。
- `cwd`：Agent 工作区之外的 verifier 目录。
- `args`：固定参数数组。仅完整参数 `{workspace}`、`{capture}`、`{trial}` 会被替换，不拼 shell 字符串。
- `files`：需要固定哈希的 grader 代码/依赖清单；程序文件本身也会固定哈希。
- `timeout_ms`：1..600000；`independent_assertions:true`。

```text
node scripts/validation/agent-reliability-collect.mjs freeze --plan suite-plan.json --output frozen-suite.json
```

该命令输出 manifest SHA-256。把这个 pin 放在 Agent 不可修改的 harness 配置中。冻结后变更测试定义必须创建新 suite，不能用更新 manifest 掩盖失败。

## capture 契约

包含 `schema_version:1`、`source:live_product`、suite/trial/session、runtime build digest、model、`observed_models`、`duration_ms`、工作区绝对路径、可选输入哈希和按 seq 连续的 canonical `events`。
`observed_models` 来自测试 harness 的真实路由观察；不是模型自述。无实际模型调用时为空，不能伪装已经观察到模型。
每个事件保留 session/seq/event_id/kind/correlation_id，并提供 `resolved_payload` 或原始 inline payload。runtime metadata 必须可解析。
辅助函数只导出身份、生命周期、计数和哈希，移除原始提示、文件正文及 provider reasoning；独立断言检查实际工作区产物。

在 fixture 仍持有目录时调用 `common::native_reliability_capture::capture_native_turn`，或者使用已有 runner 的 fixture 保留功能，再启动独立 verifier。
不要在 TempDir 已经删除后改用别的工作区来冒充原产物；采集器要求实际工作区与 capture 路径一致。
一个试验对应一个新的 Session、一个逻辑 Turn。暂停/恢复仍属于同一 Turn，不可拆成独立成功样本。

## 独立断言与签名回执

grader stdout 必须只有 JSON：`{"checks":{"tools":{...},"execution":{...},"quality":{...}}}`；键与冻结 stratum 完全一致，值只能是 true/false/null。
必须检查真实产物、禁止的改动、任务完整范围和实际终态，不能只检查 exit code 或把模型报告转抄为 true。

harness 通过 `NOMIFUN_RELIABILITY_EVIDENCE_KEY` 提供独立生成的 64 位十六进制签名密钥（256 bit）。
不要复用模型 API key；不要放进 argv、清单、工作区或代码。这个环境变量只能给 collector，不给 Agent/app，也不会传给 grader 子进程。

```text
node scripts/validation/agent-reliability-collect.mjs record --manifest frozen-suite.json --manifest-sha256 <pin> --trial <trial-id> --capture capture.json --workspace <actual-workspace> --output receipts/<trial-id>.json
```

执行前后核对 grader 哈希；超时、异常或无效 JSON 留为 null/unverified。终态、人工介入及执行额度不符合冻结条件，会使执行检查失败。
回执只保存 checks、元数据、capture/输出摘要和 HMAC，不复制 grader 原始 stdout/stderr。已有结果文件不覆盖。
失败后修复代码应另建构建/suite或按预定试验安排记录，不能覆盖原失败回执。

## 聚合

```text
node scripts/validation/agent-reliability-collect.mjs aggregate --manifest frozen-suite.json --manifest-sha256 <pin> --receipts receipts --output evidence.json --report report.json
```

聚合核对 HMAC、清单身份和所有样本，复用现有精确置信界。缺失的预定运行仍留在分母。重复 Session/trial、不同构建、不同模型、无效签名不能通过。
状态为 not_proven 时退出 1；无效/缺失配置退出 2。没有真实样本不生成“99% 已达到”。

## 安全与证明边界

- 哈希/HMAC保护采集链完整性，不自动证明 capture 来源真实、任务分布代表性或 grader 逻辑正确；仍需单独审计。
- 使用独立 OS 用户/容器、只读 grader 挂载和 Agent 无权写入的 evidence 目录。单靠目录不同不能隔离同用户下的任意 shell。
- collector 不证明 grader 子进程树已全部清理；超时/信号后由 fixture owner 回收。不要把父进程退出当成完整清理证明。
- 脚本不会自动执行 live runner，也不会读取 StepFun key。真实授权恢复、任务驱动、capture 导出和阶段验收由下一台机器按矩阵进行。

## 待测试点

冻结定义漂移、错误 pin、grader 源码在运行中修改、错误/缺失签名、重复 Session、缺失/失败样本、mock capture、不同模型/build、事件序列缺口、工作区替换、超时与遗留子进程、stdout 非 JSON、check 缺项、人工核对未声明、超预算、文件覆盖拒绝，以及与 report 既有门禁的兼容。
