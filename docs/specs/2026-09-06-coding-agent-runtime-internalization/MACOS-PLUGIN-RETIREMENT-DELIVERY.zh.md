# macOS：会话页替换移除及本地包复核

接续 2026-09-15 的多 Engine / hooks 验证。历史检查仍按其源码范围解释，本记录不将旧结果
升级为当前源码的真实模型、签名或跨平台证明。
最终包内重验于 2026-09-16 解锁后完成；本地工程与正常退出通过，严格签名仍失败。

## 现行结果与范围

- 按用户决定完整移除插件替换会话页能力，U0/U1 取消；标准会话是唯一会话界面。
- 普通带 UI App、独立 Surface、常驻 Node Service、KV、工具及作者入口保留。
- 保留 before_model、已实现的 before_tool；高成本 after_tool 与专属结果结算改造取消。
- Engine 继续源码集成、随 app 编译注册；不恢复 Wrapper、不新增社区 Engine 验收。
- 既有会话精确引擎绑定与历史插件存储编码不改写，不删除日常数据。

详细移除与原生证据见[会话页替换退役记录](../../reviews/2026-09-15-agent-session-view-retirement.zh.md)。

## Git 与源码归属

统一使用 `rf/agent-capability-platform-v2`。当前构建包含 `2ff029512`、交接提交 `14d10aa39`、
此前已推送的 `8cd2cebcf` 及远程 `7d231c3ec`；远程创建功能通过普通 merge 保留。
没有 reset、强推或覆盖其他人的工作。

- 移除提交：`6393f85db`。
- 远程合并：`2648a8c12`，第一版合并包源码 `85d4620a7`。
- 原生退出修复及最终包源码：`d6a2e53865dcfac614a8a44a9a840af7868926f3`。
- 当前内置身份：Nomi `0.7.6-host66`、Coding `0.7.6-host2-coding-loop99`。
- 本次后续修改仅本地提交；未推送、上传制品、发布 Release 或更新。

## 实际检查

| 检查 | 结果及边界 |
| --- | --- |
| 合并后前端定向 | 11 文件、59 测试、247 断言通过 |
| 合并后普通 App / hooks | plugin_product_discovery 8、plugin_ui_sessions 4 通过 |
| 最终综合检查 | `bun run check` 通过；新增源文件纳入后 process-runtime boundary 通过 |
| 最终进程运行时 | lib 132、parent_death 2、pty_contract 11 通过 |
| 最终终端 | `nomifun-terminal --lib` 145 通过，包含交互式 shell 的独立 job PGID 回归 |
| 原生构建 | `bun run build:mac arm` 退出 0，release 编译 8m52s |
| DMG / 资源 | hdiutil 校验通过，主程序 arm64 且可执行；挂载主程序与构建 app 相同；388 个前端文件及 LICENSE/NOTICE 与构建输出一致 |
| release-lock | 真实文件、平台、干净源码提交一致 |
| 严格 codesign | **失败**：`code has no resources but signature indicates they must be present`；未放宽门禁 |

最终定向命令：

```sh
cargo test --locked -p nomi-process-runtime --lib --test pty_contract --test parent_death -- --test-threads=1
cargo test --locked -p nomifun-terminal --lib -- --test-threads=1
bun run check
bun scripts/check-process-runtime-boundary.mjs
bun run build:mac arm
```

所有 Cargo 工作串行；构建使用未签名本地模式，显式去除继承的 Apple / updater 签名变量。

## 首次原生退出失败与修复

第一版 DMG `85d4620a7` 已验证标准会话正文、个人 Agent 无页面替换页签、普通 App 页面正常。
但交互 `/bin/sh` 中执行 `sleep 120 & wait` 后，Command-Q 虽令 app、watchdog、shell 退出，
独立 PGID 的 sleep 仍存活；回收接口错误返回成功。旧包保留为失败证据，不能作为完整验收通过。

修复由现有 macOS process owner 完成：保留未回收 leader 的身份，按受管 session 查找普通 job，
使用内核校验的 PID generation 发信号。扫描对照枚举前的身份基线，发生 fork、消失、换代或
新 zombie 就重试；成员全部停止并确认无活成员后才完成。超时仍保留 Pending，复用原 cleanup。

开发中另记录并修正：shell history expansion 导致的测试输入失败、API 不接受 signal 0 的
`EINVAL`，以及固定大数组超过 512 KiB poller 栈导致的 SIGABRT。最终改用自身 SIGCONT 预检和
有界匿名 mmap 缓冲；没有修改线程栈或用失败结果冒充成功。完整最终回归通过。

## 最终本地制品

| 制品 | SHA-256 |
| --- | --- |
| `dist/desktop/NomiFun_0.7.6_aarch64.dmg`，88,753,853 字节 | `ce3ccb28d70f4dd6d6e0e21609f673768a6ffb07e9f0393fe535200e2f763bd4` |
| app 主程序 | `2468e368bbb8faf4fb40e9a4158c5496e8612093c520da712c61fb1bcda39fca` |
| release-lock | `87ab5407f64705d4d8fd64e66ff2fd0634d9e5f7d2d2d406694fd4b1dda247a3` |

### 最终包内启动、交互及退出

已从只读挂载 DMG 中直接启动主程序；lsof 确认 backend DB 在
`.git/hook-product-validation/before-tool-native-p1z81Z/data` 隔离目录，健康检查 200，
三个既有会话的精确绑定不变。首次观察时 Mac 再次锁定；用户于 2026-09-16 回复“已解锁”后，
先重新核对实际进程与隔离 DB，再完成以下原生步骤：

- 标准会话正常显示 `Reply BASELINE.` / `BASELINE`，个人 Agent 无页面替换页签。
- 普通“敏感文件检查”独立 App 页面打开并显示内容；没有执行新的模型任务。
- 通过原生“新建终端”启动 `/bin/sh`，输入 `sleep 120 & wait`。
  退出前确认 app 10209、watchdog 10659、shell 10660，以及 **PID/PGID 均为 10663** 的 sleep
  全部存活，后台任务确实处于与 shell 不同的进程组。
- Command-Q 后应用退出 **0**；上述四个 PID 全部消失，没有人工补杀。
  app 的 65077/65078/65079/65080/65081/65083/65084 七个监听端口全部关闭。
- 确认库已静止、无 WAL 后只读比对：三个旧会话绑定完全不变，terminal_sessions、
  terminal_scrollback、terminal_turn_admissions 行数均为 0；更新清单 SHA-256 不变。
- 退出后未调用窗口观察，避免工具自动重启；DMG 已卸载，测试进程均已清理。

证据：`package-startup.json`、`package-active-processes.json`、`package-cleanup.json`，
以及 `01-standard-conversation.png` 至 `04-active-terminal.png`。构建后仅增加本交接文档，
制品源码归属仍为 `d6a2e53865dcfac614a8a44a9a840af7868926f3`。

## 限制与后续平台验证

- 这是未签名本地测试包；Developer ID 签名、公证、Gatekeeper 安装/升级/卸载未执行。
- 最终合并源码没有重新进行真实模型验收。此前 before_tool 的固定 StepFun 原生放行/拒绝闭环
  与严格末尾 LF smoke 失败分别记录于[hooks 实施记录](../../reviews/2026-09-15-agent-tool-hooks-implementation.zh.md)，
  不把历史成功覆盖当前源码，也不把合成 LF 失败改写为通过。
- 旧 macOS 缺少精确 generation 信号 API 时明确拒绝新 PTY，避免 app 加载时缺符号；旧系统实机未验。
- 异常宿主骤亡后的独立 job 回收尚未补完整证明；刻意 setsid 脱离原 session 不在本次扩展范围。
- Windows 仍需原生复核公共改动和普通 App/Service；Linux 继续 TODO。
- 更早 H1a 验证中曾因读取已退出 app 的窗口而触发默认开发目录启动，已在 hooks 记录中披露。
  本次包测试均先明确启动、核验实际隔离 DB，再操作窗口；退出后只查进程和端口，不读窗口触发重启。

本机证据在 `.git/macos-final-delivery/`，第一版失败证据另存 `initial-package-85d4620a7/`。
这些日志、隔离数据与 dist 制品不随 Git 提交上传。
