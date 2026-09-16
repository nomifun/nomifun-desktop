# macOS 多 Engine 继续验收（2026-09-15）

> 09:10 最终接续：Mac 已解锁，开发窗口及当前 DMG 内的 Nomi/Coding 原生发送、
> 官方模板保存、个人 Agent 改引擎后旧 Session 不改绑、Command-Q 活动终端回收均已补验。
> 本轮本地 macOS 开发与交付基线收尾完成，仍有下述明确失败和未执行项；不是正式发布验收全绿。
> 以下锁定和待验描述保留为当时记录，以文末“解锁后的最终原生验收”为当前结果。


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
- 原包 `973bbe206` 的架构、资源和退出证明作为历史证据保留；当前制品已更新为下述源码。
  未执行 Developer ID 签名、公证或发布。

本机日志、一次性数据及辅助脚本位于 `.git/macos-engine-20260914/`，不随源码提交。
Windows 需回归共享工具声明、预算和缓存行为；Linux 原生运行继续 TODO。

## 继续打包与非交互验收

源码提交：`2ede9aaf73eff787abd2315130a04b7dd68a4265`，构建时工作区干净；`bun run build:mac arm` 退出 0，
Release 编译 8m25s。日志 `package-continuation-build.log`。只构建 arm64。

- app：`target/aarch64-apple-darwin/release/bundle/macos/NomiFun.app`
- DMG：`dist/desktop/NomiFun_0.7.6_aarch64.dmg`（87,529,766 字节）
- release-lock v2 对应上述源码，文件摘要验证通过；DMG 的 `hdiutil verify` 通过。
- 从 DMG 只读挂载目录比对主程序与构建 app：一致；架构 arm64、执行权限存在。
- 401 个前端文件的路径和 SHA-256 一致，LICENSE/NOTICE 一致，没有旧 Runtime/hello 制品。

| 当前制品 | SHA-256 |
| --- | --- |
| DMG | `0f4c16cd7ca5123872cefd9d123198ba43fbbc33fc953fecbf958aa816fb1d11` |
| 主程序 | `8567e3e188e22b0410a243b014c6c6431dfe95f02fd3be27a94621d1cf4c65f3` |
| release-lock | `ba7d669a708f873c475b580d341f028774de3e0ea45ebfda0bb0f656a8bfe0cd` |

Mac 锁定时只执行了非交互验证：从 DMG 包内主程序启动，使用新的隔离数据目录，
health 200；发送 SIGTERM 后有正常插件清理日志，退出码 0。见
`package-startup-2ede9aaf7.json`、`package-startup-2ede9aaf7.log`。
这不补足当前制品的原生 UI 点击发送验证；不能把上版 UI 或退出操作直接写成当前包已重复执行。

`check-macos-arm64-native.mjs` 总结果仍为 fail：严格 codesign 检查退出 1，
`code has no resources but signature indicates they must be present`。主程序仅有 linker ad-hoc 签名，
没有完整 bundle 签名或公证；未修改门禁。其他架构、资源、发布锁和 DMG 完整性检查通过。

当前仍待用户解锁 Mac，完成原生 UI 发送、新会话 binding 以及修改 Agent 后旧会话不改绑的现场验证。
目标未标记完成。本轮构建与本地提交均未推送，未发布 Release 或更新。

## 锁定阻塞审计与暂停

连续三个目标回合实测 Mac 仍锁定，CUA 自动解锁失败；原生 UI 发送、新 Session 精确绑定及
旧 Session 不改绑的最后现场验证无法继续。目标按阻塞规则暂停，不标记完成。

已经由 SIGINT 请求开发 runner 正常清理并结束（退出码 130），停止 loopback 模拟服务
（SIGTERM，退出码 143），复核 runner、开发 app 和模拟服务均无残留；开发日志包含正常插件清理。
数据与应用配置完整保留，恢复信息位于 `.git/macos-engine-20260914/paused-state.json`。
解锁后先检查 61365 端口是否可用，再以 `python3 .git/macos-engine-20260914/ui-model-stub.py 61365`
恢复模拟服务；不要为了占用端口而杀掉其他进程。随后按前述 Cargo runner 命令启动隔离 dev，
接续名为 `macOS 原生会话绑定` 的个人 Agent 测试。若端口已被其他服务使用，更新测试连接配置。

当前制品及源码验证结果保持不变；没有推送或发布。

## 解锁后的最终原生验收

09:00–09:10（Asia/Shanghai），从保留的隔离测试根恢复，主代理串行执行，没有新增实现修改。
bun run dev --no-watch 使用前述一次性 Cargo runner 的字节相同原生 app 身份启动，编译 13.59s；
本次窗口直接正常显示，无需 Cmd-R。开发 app 使用 localhost:5173，包内应用使用
tauri://localhost。真实用户数据和已安装应用均未使用。

### 个人 Agent 与精确绑定

- 原生开发窗口使用“macOS 原生会话绑定”发送 Nomi 消息，显示 native-ui-binding-ok；
  会话 01a0a294-9142-71a0-a0e0-0c32c17c06fa 正常 finished，绑定
  nomifun.nomi / 0.7.6-host62 / default，digest
  96d8a2d2eff35ba5f90d0f39f373e32f457367567058ec9a398126e719c59ad0。
- 在同一个人 Agent 基本设置中明确选择 Coding 并保存为 revision 2，新会话绑定
  nomifun.coding / 0.7.6-host2-coding-loop95 / coding，digest
  d1f2edb8f4be0c1eb91088962e0b54fe043862ec914439cf78ffd3023d939953。
- 配置正确的模型特性后，新 Coding 会话
  01a0a298-b208-79d2-9f6a-a951732ca37f 返回 native-ui-binding-ok 并 finished。
  修改模型后创建流程使用新的冻结 preset 快照；不将该派生快照误报为原个人 Agent revision 3。
- 对比首次 Nomi 会话保存前后及包内运行后的数据库事实，原 Nomi binding 完全不变。
  证据：native-ui-nomi-before.json、native-ui-binding-final.json。
- 开发窗口 Command-Q 正常退出，runner 退出 0，无开发 app/runner 残留；
  日志 dev-gui-resume.log 有正常插件及终端清理。

### 当前 DMG 的官方模板、发送与退出

当前包仍为干净源码 2ede9aaf73eff787abd2315130a04b7dd68a4265 构建，摘要保持前表不变。
从 DMG 只读挂载的 NomiFun.app 启动，使用原有隔离测试根；没有使用 Vite。

- 官方“最简问答”模板的基本设置可发现 Nomi host62 与 Coding loop95，选择 Coding 后
  保存为 DMG Coding Template Verification；“使用 Agent”保留该选择与本地模拟模型。
- 包内 Coding 会话 01a0a29b-2b18-7871-9b7e-e72ec27c7f68 与默认 Nomi 会话
  01a0a29b-567a-7b42-88ba-5625a96e6444 均通过原生点击发送，显示模拟回复并 finished；
  两者数据库 binding 分别匹配上面的精确 build/digest/profile。
- 从原生“新建终端”启动精确命令 /bin/sh -c 'sleep 120 & wait'。
  退出前实测 app 81268、watchdog 81562、shell 81563、sleep 81564 在运行；
  Command-Q 后全部 PID 消失、终端行数 0、日志 deleted=2（含一次错误输入已退出的终端），
  应用退出码 0。
- 证据：package-native-ui-final.json、package-final-active-processes.txt、
  package-ui-original-root.log。模拟服务已停止、DMG 已卸载；隔离数据和日志保留。
- 再次计算 DMG 和 release-lock SHA-256 与前表一致；已有 updater manifest 摘要未变化。

本次 UI 模型服务是 127.0.0.1 的确定性 fixture，不连接云端，不冒充真实模型验收。
真实 StepFun step-3.7-flash 八阶段与独立压缩的成功证据仍为前述真实测试日志。

### 新发现及操作失败，如实保留

1. 模拟模型初始 traits 为空，Coding 在请求发送前报 UnsupportedFeature。
   第一次特性编辑误点弹窗外而未保存，复验仍失败；实际保存 streaming/function_calling/reasoning
   后数据库已确认。没有修改路由或权限门禁。
2. 旧 Coding 会话在模型配置变更后续发明确报 RouteRevisionMismatch；
   使用新会话后成功。这是冻结路由版本校验，不静默采用新配置。
3. **Coding 错误消息上的“重试”当前走 edit-resubmit/rewind，返回 400：
   The selected runtime does not support rewind。** 此入口限制未修复，本轮没有增加 Coding rewind。
   现有失败会话保留，已成功验证的正常新建/续接与真实执行链不等同于重试支持。
4. 直接复制已创建数据集到另一个根进行包内测试，因归属关系报
   reserved as the external work root of another NomiFun dataset。
   保留拒绝日志 package-ui-final.log，退出该实例后在原隔离根启动成功；
   没有改写 dataset 身份、历史存储编码或绕过归属检查。
5. 首次终端输入误拼接 $SHELL 且引号被输入方式改变，终端已退出；
   用原生 setValue 明确设置完整命令后才取得 shell/sleep 在运行的退出证据，不计前次为通过。

### 收尾边界

本轮 macOS 本地基线所需原生交互与包内退出已补齐。严格 codesign 仍失败，未执行
Developer ID 签名、公证及 Gatekeeper 安装/升级/卸载；bun run check 仍有 109 处旧术语引用，
不能宣称工程门禁或正式发布验收全绿。真实模型调用中取消、异常退出后的旧数据根恢复、
真实用户库迁移仍未验证；本地模拟活动进程取消和正常退出有独立证据。
Windows 当前公共修改仍需原生回归，Linux 继续 TODO，社区 Engine 示例不在验收范围。

最终 fetch 只更新远程引用；指定分支仍包含 2ff029512，未 reset、强推或覆盖他人工作。
本轮新增的收尾内容仅为文档及本地证据，不需要重打包。没有 push、Release 或更新发布。
