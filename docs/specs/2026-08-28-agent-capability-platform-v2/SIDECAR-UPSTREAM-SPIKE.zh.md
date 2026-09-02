# SL-S2-10 Codex official app-server upstream spike

> 执行日期：2026-09-02
>
> 任务：`SL-S2-10`
>
> 本文是本任务的新 spike 记录，不修改 `GLOBAL-CLOSURE-TODO.zh.md` 的实时状态。
> 当前结论只能是 `blocked` 或 `observed`，不能用本文宣称 C8、S2 或发布完成。

## 1. 范围与止损边界

本轮只做两件事：

1. 对官方 Codex app-server 的 pinned source 做协议事实核对；
2. 提供不读取凭据、不联网重试的静态检查、live smoke 和人工验证框架。

没有修改生产 Runtime、contracts、kernel、app、Cargo、UI 或任何既有文档。
本轮不恢复历史 vendor patch，不预先保留 `runtime/hello`、
`runtime/session/dispose` 或 `native_action/start`。

验证器的状态语义如下：

| 状态 | 含义 |
| --- | --- |
| `observed` | 从指定 pinned source 或一次实际进程观测到的事实 |
| `blocked` | 缺少必要 upstream、binary、凭据授权或人工输入 |
| `fail` | 已实际观测到协议或清理行为不符合预期 |
| `self-test-pass` | 只代表 spike harness 自身测试通过，不代表 upstream |

## 2. 本机输入核对

| 输入 | 结果 | 说明 |
| --- | --- | --- |
| 官方 checkout | `../codex` 存在 | 目录是独立的 `openai/codex` Git worktree |
| checkout 工作树 | clean | 本轮只用 `git show <pin>:<path>` 读取，不切换其 HEAD |
| requested pin | `dc2ccc6843abb09c9d297862dc10b6bd12a3935d` 存在 | Git 对象可读取 |
| pin 与 checkout HEAD | 不同 | 当前 HEAD 为 `4ee04c0aa5833ac39b1763f6ea44c7bc777c83dd`；pin 是其祖先，静态审查严格使用 pin tree |
| vendor source lock | `vendored_upstream_source=false` | `vendor/codex-runtime/source-lock.json` 只有来源元数据，不是 upstream source |
| 已安装 CLI | `codex-cli 0.147.0` | 只证明本机有可执行 CLI，不证明它来自 `dc2ccc...` |

旧 vendor patch series 中列出的自定义 RPC 和 ACK 机制属于历史输入，不能反向约束当前一期协议。它们与 05 的“先做 upstream spike”要求冲突时，以真实 pinned upstream 和 05 为准。

## 3. pinned source 的静态协议事实

静态读取的主要文件：

- `../codex/codex-rs/app-server/README.md`
- `../codex/codex-rs/app-server-protocol/schema/json/ClientRequest.json`
- `../codex/codex-rs/app-server-protocol/schema/json/ClientNotification.json`
- `../codex/codex-rs/app-server-protocol/schema/json/ServerNotification.json`
- `../codex/codex-rs/app-server-protocol/schema/json/ServerRequest.json`
- `../codex/codex-rs/app-server-transport/src/transport/stdio.rs`

### 3.1 传输与握手

- 官方 app-server 支持 stdio newline-delimited JSON（JSONL）。
- 线上 JSON 对象省略 `"jsonrpc": "2.0"` 字段；不能擅自改成另一套 framing。
- 每条连接先发送一次 `initialize` request，收到 response 后发送
  `initialized` notification，再发送其他 request。
- `initialize` response 提供 `userAgent`、`codexHome`、`platformFamily`、
  `platformOs` 等身份信息。
- pinned schema 没有独立的 `version` RPC。版本身份应由 initialize response
  与精确 binary 的 `--version`/构建来源共同记录，不能假设一个不存在的
  `version` 方法。
- websocket 不是本任务的最小依赖；本地 Sidecar 候选只使用 stdio。

### 3.2 Thread、Turn、取消和事件

| NomiFun 需要的语义 | 官方 upstream surface | 结论 |
| --- | --- | --- |
| 创建/恢复执行绑定 | `thread/start`、`thread/resume`、`thread/fork` | 可直接复用 |
| 启动一轮输入 | `turn/start` | 可直接复用 |
| 向活动 turn 增量输入 | `turn/steer` | 可直接复用 |
| 取消活动 turn | `turn/interrupt` | 不新增 `turn/cancel`；等待 `turn/completed` |
| 生命周期事件 | `thread/started`、`turn/started`、`turn/completed`、`item/started`、`item/completed` | 可作为 Host event ingress |
| thread 数据删除 | `thread/delete` | 这是 thread 数据生命周期，不是 Runtime dispose ACK |

官方 README 明确说明：`turn/interrupt` 成功后，最终会发出
`turn/completed`，其状态应为 `interrupted`。Host 不应把 interrupt response
当作 turn 已经完成的唯一事实。

### 3.3 Host-managed Tool seam

官方 pinned source 提供实验性的 dynamic tool seam：

1. 在 `thread/start` 注册 `dynamicTools`；
2. turn 运行期间由 upstream 发出 `item/tool/call` server request；
3. Host 根据 `threadId`、`turnId`、`callId`、namespace、tool 和 arguments
   做本地路由；
4. Host 返回 `contentItems` 与 `success`；
5. upstream 继续发出 `item/completed`，作为该 Tool 调用的最终状态。

这已经是可观察的 Host-managed Tool callback。当前没有静态证据要求再增加
`native_action/start` 这种第二次“调用前通知” RPC。动态 Tool 是 experimental
API，live 验证仍必须由用户明确提供可丢弃的测试模型/登录环境。

### 3.4 关闭与资源清理

- stdio transport 在 stdin EOF 后报告 connection closed；
- 官方测试 helper 使用关闭 stdin 的 graceful shutdown；
- Host 应先关闭协议输入，再在 bounded deadline 内等待并回收整个 descendant
  process tree；
- `thread/delete` 可删除 thread 数据，但不等同于一个私有
  `runtime/session/dispose` ACK；
- upstream pinned source 没有 `runtime/hello`、`runtime/session/dispose`、
  `native_action/start`。

因此当前最小 Sidecar 方向是：

```text
initialize -> initialized
-> thread/start / thread/resume / thread/fork
-> turn/start / turn/steer / turn/interrupt
-> upstream events + item/tool/call callback
-> close stdin -> bounded wait -> Host process-tree cleanup
```

## 4. Patch 结论

### 静态结论

`no-patch-indicated-by-static-review`。

理由：

- 必需的 initialize、thread、turn、interrupt、event surface 都在 pinned
  schema 中；
- Host-managed Tool 有 `item/tool/call` callback；
- 取消后的最终状态有 `turn/completed`；
- 进程关闭有 stdio EOF/connection close 语义；
- 三个历史自定义 RPC 在 pinned upstream source 中不存在。

### 最终结论

`blocked-until-live-smoke`。

静态 source 不能证明：

- 用于交付的 binary 确实由 `dc2ccc...` 构建；
- live initialize/thread/turn/interrupt/event 在目标进程上按预期工作；
- dynamic Tool callback 的实际 request/response 顺序；
- Windows packaged process-tree cleanup 没有遗留进程。

在这些事实被真实观测前，不提出 patch，也不把现有生产 Runtime 的旧协议适配器当作 upstream 证据。

## 5. 已执行验证

### 已完成

- 核对官方 checkout、Git pin 对象、pin 与当前 HEAD 的关系；
- 在 pin tree 上读取并解析四份官方协议 schema；
- 读取 pinned README 与 stdio transport source；
- 执行 `codex app-server --help`，确认本机 CLI 暴露 app-server 入口；
- 新建 spike harness 的 `--self-test`、静态检查和 fake-transport 测试框架。

### 首个 live 阻塞

曾对本机已安装 CLI 做过一次有界 live 探测，但在预设窗口内没有产生可用
协议输出，外层命令超时。随后确认没有留下 app-server 进程；未继续盲目重试。
这次探测不能形成 initialize PASS，也不能证明当前 CLI 与 pinned commit 相同。

阻塞分类：

1. 已安装 `codex-cli 0.147.0` 没有可复查的 `dc2ccc...` binary provenance；
2. 当前没有经过本任务确认的、与 pin 精确对应的已构建 app-server binary；
3. 未获得本任务专用的 live model credential 授权，因此不运行 turn/cancel 或
   dynamic Tool live E2E；
4. Windows 子进程 live harness 的首次关闭行为不可靠，后续只保留 bounded
   cleanup，不重复无限等待。

## 6. Self-test 与人工验证

### 6.1 只验证 harness

```powershell
bun scripts/validation/codex-app-server-spike.mjs --self-test
bun test scripts/validation/codex-app-server-spike.test.mjs
```

`self-test-pass` 仅表示 JSONL parser、schema method extractor、redaction 和
fake transport harness 自身工作，不表示 upstream 已通过。

### 6.2 静态 pinned review

```powershell
bun scripts/validation/codex-app-server-spike.mjs `
  --upstream-dir ..\codex `
  --pinned-commit dc2ccc6843abb09c9d297862dc10b6bd12a3935d `
  --report build.noindex\codex-app-server-spike.static.json
```

期望看到的是 `status: "blocked"` 或 `status: "observed"` 加上明确 blocker；
不能把命令退出码或某个 `observed` check 改写成 PASS。

### 6.3 用户提供 exact binary 后的安全 live smoke

先在官方 checkout 的 exact pin 上构建 app-server，并保留 binary 来源记录。
然后运行：

```powershell
bun scripts/validation/codex-app-server-spike.mjs `
  --upstream-dir ..\codex `
  --pinned-commit dc2ccc6843abb09c9d297862dc10b6bd12a3935d `
  --run-live `
  --binary C:\path\to\codex.exe `
  --report build.noindex\codex-app-server-spike.live.json
```

这一步只做 initialize、initialized 和 ephemeral `thread/start`，不读取 token
文件，也不自动联网获取 credential。

### 6.4 明确授权后才运行 turn/cancel

用户准备一个隔离的 `CODEX_HOME`，由 Codex 自己管理登录状态，不把 token
写入命令、日志或报告，再显式运行：

```powershell
bun scripts/validation/codex-app-server-spike.mjs `
  --run-live `
  --run-turn-cancel `
  --allow-live-model `
  --codex-home C:\path\to\isolated-codex-home `
  --binary C:\path\to\exact-pin\codex.exe `
  --report build.noindex\codex-app-server-spike.turn-cancel.json
```

人工核查：

- initialize response 的身份字段存在；
- `thread/start` response 与 `thread/started` 都出现；
- `turn/start` 返回 turn id；
- `turn/interrupt` 返回后仍等待 `turn/completed(status=interrupted)`；
- 若模型调用了 `item/tool/call`，Host 返回只读 `contentItems`，并观察
  `item/started -> item/tool/call -> response -> item/completed`；
- 关闭 stdin 后进程在 deadline 内退出，Windows descendant 为空；
- 报告没有 credential 内容，也没有自定义历史 RPC。

## 7. 本任务交付边界

本轮只新增：

- `SIDECAR-UPSTREAM-SPIKE.zh.md`
- `scripts/validation/codex-app-server-spike.mjs`
- `scripts/validation/codex-app-server-spike.test.mjs`

没有提交，没有修改 `GLOBAL-CLOSURE-TODO.zh.md`，没有修改任何生产代码。
完成 `SL-S2-10` 仍需要在精确 pinned binary 上完成 live smoke，并由主机集成
阶段根据真实 trace 决定是否关闭 TODO。
