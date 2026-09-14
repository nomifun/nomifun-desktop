# macOS 多 Engine 继续验收（2026-09-15）

本记录接续 `MACOS-DELIVERY-2026-09-14.zh.md`，不覆盖原失败记录。所有工作仍在
`rf/agent-capability-platform-v2`；未 push、未发布 Release/更新。此前源码基线为
`3e5bca1e1`，包含要求的 `2ff029512`。本轮目标尚未全部完成。

## 已取得的新证据

- `live-engines-compaction-fixed-18.log`：StepFun Coding Plan / `step-3.7-flash` 的完整双引擎
  **八阶段通过**，包括每种引擎的创建文件、读取修改、只读命令及多轮续接。该轮没有恢复提示。
- `live-compaction-21.log`：独立真实压缩用例通过，**1 次摘要请求、1 次上下文替换**。
  在一次性会话的标准 messages 表内构造明确标记的合成历史，超过默认消息数量上限；
  压缩后检查原始历史行全部保留、精确引擎绑定不变，并完成新的文本响应。
  这些合成行是测试输入，不当作真实模型或工具执行结果。
- `native-engine-cancel-fixed.log`：两个官方引擎分别实际启动原生 shell 和 sleep 子进程；
  先通过 PID identity 证明它们存在，再调用会话 cancel API，同时等到 finished 和两代进程消失。
  宿主关闭成功。使用 loopback 模型，非真实 Provider 取消证明。
- 本轮所有真实调用继续仅使用指定的 StepFun endpoint 与模型，凭据仍由隔离 runner 经 stdin
  交给测试进程，不进入 Cargo、工具子进程、源码、报告或命令 argv。

## 本轮修复

### 命令参数声明

原 `process.exec` JSON Schema 只在 `oneOf` 分支中声明字段，顶层没有 `properties`。
StepFun 的工具文档按顶层 properties 描述参数：
https://platform.stepfun.com/docs/zh/api-reference/tool-call

补充顶层字段描述以便模型发现参数，**原 oneOf 分支、必填项、禁止额外字段和各数值上限全部保留**。
用正反例验证补充前后接受集合一致，包括 exec/start、各进程控制操作、null/额外字段和越界值。
修改后真实 Coding 命令阶段通过；不以文档说明单独推断厂商对 oneOf 的实现。

### 压缩生成预算

真实续接失败被细分为 `CODING_COMPACTION_OUTPUT_LIMIT`。原代码用摘要字节上限 / 4
再次限制总生成 tokens，这不能覆盖推理输出。压缩现在使用本回合**已冻结且受调用者限制**的
生成预算；上下文预留原本已经按这个预算计算。独立摘要字节上限、历史消息上限和压缩次数上限不变。

先补回归并观察旧实现返回 2048、期望冻结预算 3000 的失败，再修复；同时验证私有推理不进入
派生摘要，超过摘要字节上限仍拒绝。Coding 单测最终 **39 通过**。

### 错误和烟测证据

- Coding 错误转换保留 Broker 的非敏感 HTTP 状态数值，不改变路由或重试策略。
- 增加固定分类及编译内源码位置诊断，不输出任意 Provider 文本、参数或凭据。
- `--engine-smoke --engine-family=coding` 是明确标记的单引擎诊断；默认 `--engine-smoke`
  仍必须取得两个引擎的全部八阶段证据。重复/未知选择或脱离 engine-smoke 的选择被拒绝。
- `--compaction-smoke` 为独立压缩验收，要求实际摘要和上下文替换计数均大于零。
- 测试中的“首个工具错误立即停止”与 Coding 的计划纠正循环冲突。现在只允许有证明的执行前
  拒绝继续交给引擎处理：引擎本地计划/报告、明确的计划执行前门、以及不能通过相同 canonical
  process Schema 的调用。必须存在后续成功观察才可从外部效果序列中排除，并单独报告次数。
  Schema 合法的 owner 错误、真实文件失败、未知结果或未纠正的拒绝仍失败，不能成为成功证据。
- 完成报告提示只引用当前仍可用的观察，不能用修改前的读取证明修改后的工作区。

正式构建标识更新为 Nomi `0.7.6-host62`、Coding `0.7.6-host2-coding-loop95`。
源码摘要覆盖变更；没有自动改写旧 Session binding 或历史存储编码。

### 原生工作台到新会话的缓存

在真实开发窗口中保存个人 Agent 后点击“使用 Agent”，首页实际回落到默认模板。
根因是工作台只刷新自身列表并发出 SWR revalidation；未挂载的首页缓存没有得到新列表。

改为通过当前 SWR context 的 mutate 发布刚从服务器取得的权威列表，再开放导航。
回归新增“先有旧缓存、创建保存、再挂载首页选择器”的链路：修复前 Nomi/Coding 两项都失败，
修复后通过。原生 UI 重验后，“使用 Agent”已正确保持 `macOS 原生会话绑定`。
另纠正供应商页面“自定义模型仅可供 Nomi 使用”的过时提示。

## 开发窗口与当前人工阻塞

为解决裸开发可执行文件没有可供 CUA 识别的 app 身份，使用仅位于 `.git` 的 Cargo runner：
给调试程序补临时 `.app` 元数据，不改程序字节，不加载任何额外 Engine。
`dev-app-runner-proof.json` 证明副本与 `target/debug/nomifun-desktop` SHA-256 一致。

实际命令：

```sh
CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER="$PWD/.git/macos-engine-20260914/dev-app-runner.py" \
NOMIFUN_DATA_DIR="$PWD/.git/macos-engine-20260914/dev-gui-data" \
bun run dev --no-watch
```

原生窗口地址是 `localhost:5173/#/guid`，使用 Tauri dev + Vite。首次 CUA 观察为空白，
Command-R 后可正常渲染操作；不能把这个首次观察写成无条件通过。
已在原生 UI 配置标记为“本地UI模拟（不调用云端）”的 loopback 服务、选择
`step-3.7-flash`、创建个人 Agent，并独立核对默认路由无 failover、URL 仅为 loopback。
模拟凭据是测试占位值，不是真实用户 key。

**准备点击输入框发送新会话时，Mac 锁定，CUA 自动解锁失败。已请求用户手动解锁。**
“原生 UI 点击发送 → 新 Session 精确绑定 → 修改 Agent 后旧 Session 不改绑”的最终现场验证
仍待完成，不能用已通过的回归代替。测试 app 与 loopback stub 状态由后续接手时实时确认。

## 检查与历史失败

- Wave 2 全部单测：初次 12 pass / 4 fail，原因是旧状态捕获 fixture 使用空 fs.read；
  只补合法路径，保留前置 Schema 校验；复跑 **16 pass / 0 fail**。
- 应用 fixture / 证据测试：**15 pass / 0 fail**（另有真实调用的 ignored 入口）。
- UI 定向：缓存修复后 **73 pass / 0 fail**；i18n、TypeScript、桌面边界通过。
- `agent-v2-contract check`：通过；未改写生成物。
- 综合 `bun run check` 仍在旧术语门禁失败，不宣称全绿；原 109 处存储/历史/实现引用未全局替换。
- 本轮真实诊断还出现过 ProviderUnavailable，保留每次日志，没有替换 Provider/模型。
- 原包 `973bbe206` 的架构、资源和退出证明保留，但本轮代码变化后需要重新构建最终 app/DMG。
  未执行 Developer ID 签名、公证或发布。

本机日志、一次性数据及辅助脚本位于 `.git/macos-engine-20260914/`，不随源码提交。
Windows 需回归共享工具声明、预算和缓存行为；Linux 原生运行继续 TODO。
