# Windows 安装包与 StepFun 真实模型测试

日期：2026-09-14。分支：`rf/agent-capability-platform-v2`；本地工作树，未 push。
本文记录上轮工程检查之后新增的安装包及真实模型测试，不把编译通过等同于真实模型通过。

## 范围与隔离

- 构建 Windows x64 NSIS release 安装包，不签名、不发布更新。
- 本机已有 NomiFun 0.7.5；不覆盖安装、不修改其用户数据或卸载注册项。
  对本轮安装包做完整性及渲染脚本检查，提取包内程序，在独立数据目录进行启动测试。
  这不等于完成覆盖安装、升级或卸载生命周期验收。
- StepFun endpoint 固定为 `https://api.stepfun.com/step_plan/v1`；不得自动切换模型、
  provider 或 endpoint。API Key 不写入仓库、配置、报告或测试进程 argv。
- 官方 Nomi/Coding 的最小真实模型测试使用临时工作区；分别验证精确 Engine binding、
  创建文件、读取修改、只读命令及多轮续接，不包含 cron/remote/社区示例。

## StepFun 实际探测结果

用户指定模型：`step-3.77-flash`。

1. 认证调用 `/models` 返回 HTTP 200。列表包含 `step-3.7-flash`，不包含 `step-3.77-flash`。
2. 对用户指定的 `step-3.77-flash` 发出最小 `/chat/completions` 请求，返回 **HTTP 404**。
3. 没有自动替换模型；用户随后明确确认使用 **`step-3.7-flash`**。
4. 确认后对 `step-3.7-flash` 发出的最小真实请求返回 HTTP 200 和 choices。
   该探测仅给 64 token，未得到最终文本，不把它记作对话任务完成；主链使用实际模型配置另测。

模型名阻塞已解除，现使用显式环境变量选择 `step-3.7-flash` 运行双 Engine 主链。
下文单独记录结果；接口模型可用性不等于 Engine 验收通过。此报告不包含凭据或原始响应正文。

## 真实双 Engine 测试入口

`scripts/validation/run-nomi-core-live-provider-smoke.mjs --engine-smoke` 选择新增的
ignored 测试 `nomi_core_official_engines_reach_live_stepfun`。非敏感环境变量
`NOMIFUN_LIVE_STEPFUN_MODEL` 固定为用户确认的 `step-3.7-flash`，默认值相同；
先前笔误 `step-3.77-flash` 已从允许列表移除，不允许回退。

runner 先在无凭据环境编译，随后从 `NOMIFUN_LIVE_STEPFUN_API_KEY` 读取凭据并删除
该环境变量，只通过 stdin 交给测试进程；模型和测试结果不允许静默 fallback。
每个 Engine 四轮，单轮 180 秒，双 Engine 链 15 分钟上限。凭据清理及审计复用既有流程。

runner self-test：通过。`cargo check --offline -p nomifun-app --test nomi_core_live_provider_smoke`
通过（3 分 35 秒）；随后 runner 的 `cargo test --no-run` 可执行测试编译通过。

首轮可执行测试已编译并运行，在 `session.create` 返回
`RESOURCE_SELECTION_REQUIRED` / HTTP 422，尚未进入模型任务。
这是 live fixture 未提供当前 API 要求的资源选择，修正只限测试配置，
不放宽产品校验。已补齐 workspace/process_session 资源选择、空 skills/MCP 选择及
Windows 规范路径比较；测试证据模块 `evidence_tests` 最终 **9 通过 / 0 失败**。

最终主链结果（`.git/windows-live-engine-6.*.log`）：

| Engine | 已进入阶段 | 结果 |
| --- | --- | --- |
| Nomi | `engine.nomi.create` | `CONFLICT`，测试 failure status=422 |
| Coding | `engine.coding.create` | `CONFLICT`，测试 failure status=422 |

两个 Engine 分别创建独立 Session/工作区测试，不因 Nomi 先失败而跳过 Coding。
两者都未取得首轮工具成功证据，因此后续 patch、exec、多轮续接不记为已通过。
`422` 为测试错误报告状态，不据此断言 StepFun 返回了 HTTP 422；直接模型探测为 200。
当前未定位该通用冲突的底层原因，不武断归因于模型服务、产品实现或测试配置。
本轮只修测试 fixture，未更改生产引擎或放松权限；真实主链验收明确 **失败，待排查**。

runner 对模型、exact Engine binding、工具参数/结果、最终标记和多轮历史严格断言；
安全输出仅包含固定阶段及类型化错误。测试失败后仍执行宿主关闭、凭据未持久化审计和
临时目录清理。不能将这次测试写成“真实模型双 Engine 通过”。

## Windows 构建

命令：`bun scripts/run-win-build.mjs x64`，加载已有 Windows 工具链环境。
前端生产构建通过；frontend build ID：`dee3c70b-9208-4129-8677-59483f85c5e8`。
Rust release 构建通过（33 分 20 秒），NSIS 打包成功，runner exit=0。

本轮新制品：`dist/desktop/NomiFun_0.7.6_x64-setup.exe`，2026-09-14 19:38 本地生成，
69,942,941 字节（约 66.7 MiB）。SHA-256：

```text
71F8D9330BDF82A75AB0589E941011EC3FA46707EC89778A5257ED79CE0B9199
```

Authenticode：`NotSigned`；这是未签名本地测试包，不代表正式发布签名验收。

已通过的制品检查：

- 7-Zip NSIS 压缩完整性检查，退出码 0；提取包内真实程序及资源成功。
- `LICENSE`、`NOTICE`、`webui-dist/nomifun-build.json` 与本轮源资源 SHA-256 一致。
- 完整核对全部 401 个前端文件，路径集合一致，SHA-256 内容差异为 0。
- 包内 exe 长度 249,497,088 字节，SHA-256
  `E69979D5DEE1BF15D1639F345DA97F6EC199AB8F53C9CBE6977013B6FBBA6C37`。
  与 release 原 exe 逐字节比较，仅 3 字节不同：Tauri 打包时的
  `__TAURI_BUNDLE_TYPE_VAR_UNK` → `__TAURI_BUNDLE_TYPE_VAR_NSS`，与构建日志的 NSIS patch 对应。
- 实际生成的 NSIS 脚本契约检查通过；去除 NOTICE 的负向 fixture 正确被拒绝。
  checker 已按当前 bundle 配置检查 webui-dist、LICENSE、NOTICE 的实际文件/目录指令，
  不再要求已退役的 infinite-canvas 源文件；未改变生产打包配置或删除 NOTICE。

现存构建提示：后端 28 项未使用入口/声明 warning；Vite 大 chunk 提示。
不为清理无关 warning 扩大本轮工作。

## 包内程序隔离启动结果

- 启动的是本轮安装包提取出的 exe，不是旧安装或开发模式程序。
- 独立数据、workspace、WebView2 profile；backend `/health` 返回 200。
  WebView2 浏览器/renderer 子进程启动，观察到 renderer 的本地 API 请求 200。
- 首版脚本把 backend `/` 当作桌面 SPA 入口并要求 200；源码确认该 API 根路径
  返回 404 属于桌面静态资源与 API 分离，已纠正探测假设。
- 当前管理员提升权限会话中，WebView2 调试端口未监听，CDP 页面探测超时；
  非提升 linked-token 启动尝试报“指定的登录会话不存在，可能已被终止”。
  未取得页面截图、UI 内双引擎目录断言或正常退出证据，不宣称完整桌面烟测通过。
- 已清理测试启动的 payload/WebView2 进程并复核无残留。失败运行使用强制清理，
  不等于正常关闭验收。
- 应用启动会自动注册 `nomifun://`，首轮探测曾把该关联指向提取目录。
  测试结束后只在值仍指向该 payload 的前提下，将 DefaultIcon 和 open command
  恢复到本机 `AppData/Local/Programs/NomiFun/nomifun-desktop.exe`（0.7.5）。
  已安装程序及卸载注册版本仍为 0.7.5，未执行覆盖安装、升级或卸载。

后续验收应在普通交互 Windows 会话完成 UI/正常退出，在独立 Windows 测试环境
完成安装/升级/卸载生命周期；不需要新增社区 Engine 示例。

## 本轮结论

**新 Windows 安装包已生成，制品检查通过；完整交付验收未通过。**
真实双引擎首轮冲突和桌面 UI/退出未验证项须继续处理，不能据打包成功宣布可完整交付。

本机日志位于 `.git/windows-package.*.log`，启动证据位于
`.git/windows-package-smoke/`；均不随源码提交。
