# Windows 多 Engine 交付与跨平台交接

后续补充：用户已进一步要求 Windows 安装包及真实模型测试，并明确确认模型为
`step-3.7-flash`。新增结果以
[WINDOWS-PACKAGE-LIVE-2026-09-14.zh.md](WINDOWS-PACKAGE-LIVE-2026-09-14.zh.md)
为准。本文下面的“不调用真实模型/未构建制品”等陈述仅描述此前工程检查阶段，
不是对后续测试的禁止；不覆盖现有安装、不动真实用户数据库及不 push 的约束保持不变。

## 当前范围（用户最新要求优先）

- 本机仅完成 Windows 开发与必要工程收尾，官方 Nomi / Coding 两个 Engine 共用
  源码注册、冻结目录和 Session 精确绑定。Coding 本身承担第二种实现的接入样本，
  **不再将独立社区 Engine 示例的开发、运行或验收列为交付前提**。
- 不增加无关 Provider/MCP/Git 新功能；既有未支持项记录限制，不隐瞒为可用。
- macOS / Linux 工作交给对应机器，不作为本机 Windows 完成的阻塞项。
- 保持 Agent 工作台选择 Engine；Engine 管循环/规划/上下文，平台管 Session、
  权限和工具所有者。编译打包前注册，禁止打包后挂载或在 Session 内热换。
- 必要 Windows 编译、契约生成与定向回归属于本轮收尾；不启动用户服务、不调用
  真实模型、不在用户数据库执行迁移。不 push。

## 同步基线与保护

同步 origin/rf/agent-capability-platform-v2 至 `52857f39c`，其 15 个远程独有提交
与本地 12 个独有提交合并，产生本地 merge 提交 `f6db504e6`。没有 rebase 或强推。
原未提交 loop90 工作通过 stash 保存后恢复，备份引用
`refs/backup/engine-windows-pre-sync-20260914`（`2f0cb61d96225c510a97a379a6c0409e78162520`）
及原 stash 保留。此 merge 提交本身不是全部 Engine 未提交实现的交付快照。

## 本轮收尾工作

并发目录所有权：UI、AI Agent、Contracts、DB、Kernel/Coding/Core 各自独占；
应用组装与共享调用方由主线程集成，少量应用工具适配按单文件移交。不让并发
worker 同时改写相同文件，不并发执行多个 Cargo 构建，不自动提交全工作区。

已实施的同步收尾：

1. 上游 `enabled_capabilities` 取代 initial/on-demand 模型，官方 Engine 必须
   共用已保存权限上限，不能恢复旧的动态扩权路径。
2. 统一 Plugin Product 类型必须同时进入 Coding/Nomi 工具路径，仍保留原有效果
   凭据和 Unknown 处理；历史凭据标识不因产品改名而随意迁移。
3. 上游与本地迁移编号冲突：保留远程已发布编号，严格识别本地旧迁移记录，
   新增未发布迁移重编号，不删用户数据或伪造迁移成功。
4. 重生成契约并完成 Windows 相关编译和定向检查；消除冲突标记不等于编译成功。

已消除工作区和 Git 索引中的合并冲突，恢复同步前的未暂存开发状态，未将整个
工作区自动提交。相对本轮 fetched 远端为 ahead 13 / behind 0。

当前官方构建标识为 Nomi `0.7.6-host60`、Coding `0.7.6-host2-coding-loop91`。
新构建仍坚持 exact binding；不自动把旧构建 Session 改成新构建。旧 Coding 的
`capabilities_activated` 日志明确拒绝恢复，不伪造 generation=0。

迁移编号为：上游 095/096 保留；原 runtime-events 095 → 099；本地未发布
MCP effects/observations、hosted effects、Git effects 分别移至 100/101/102/103。
只认证原 001..095 完整成功账本及固定 SHA-384；搬号和补齐迁移同事务提交。
未知 checksum、缺项、失败记录、目标编号冲突均拒绝，历史 `miniapp` owner 编码保留。
SQL 换行按仓库 LF 规则；绕过规则产生的历史 CRLF checksum 不做静默归一化。

## Windows 工程检查

- 契约生成：`cargo run --offline -p nomifun-agent-contracts --bin agent-v2-contract -- write`；
  随后 `target/debug/agent-v2-contract.exe check`，通过。
- Windows 后端、桌面宿主及 Coding 集成用例编译：
  `cargo check --offline -p nomifun-desktop -p nomifun-app --lib --bins --test coding_runtime_production`，通过。
  这是编译检查，不是安装包、真实模型或桌面交互验收；当前开发构建未启用静态 WebUI。
- 新迁移安全回归：`cargo test --offline -p nomifun-db --lib database::displaced_conversation_runtime_migration::tests:: -- --nocapture --test-threads=1`，
  **5 通过 / 0 失败**，全部使用内存 SQLite，不访问用户数据库。
- 既有迁移兼容回归：`cargo test --offline -p nomifun-db --test displaced_agent_preset_migration --test agent_snapshot_capability_retirement_migration -- --test-threads=1`，
  **4 通过 / 0 失败**。
- 共享 Engine Core：`cargo test --offline -p nomifun-engine-core --lib`，
  **14 通过 / 0 失败**，包括静态权限集合、Windows 子进程取消/超时及回收。
- UI 定向回归 **80 通过 / 0 失败**；`check:desktop-ui-boundary` 通过。
- `bun scripts/check-windows-installer-contract.mjs` 和
  `bun scripts/check-process-runtime-boundary.mjs`，通过。
- 本地缺失 `bun-types` 已通过 `bun install --frozen-lockfile --ignore-scripts` 恢复，
  没有升级依赖或修改锁文件。完整 `bun run typecheck` 已通过。
- Coding：`cargo test --offline -p nomifun-coding-engine --lib`，最终
  **36 通过 / 0 失败**。首轮 7 项失败来自旧 mock 返回协议、预算、流程及阻塞位置，
  仅修测试 fixture，未改生产取消逻辑。新增 mandatory 上下文超预算时不调用模型、
  明确报错并释放单轮准入的回归；模型打开阶段的取消是已有回归。未忽略失败用例，
  未放宽权限或上下文必需信息保护。

本轮定向回归合计 **139 通过 / 0 失败**（UI 80、Coding 36、Core 14、迁移 9）。
Windows 开发集成及上述工程收尾已完成；这里不宣称完成安装包或真实模型验收。

UI 定向回归复跑命令（仓库根目录，20 文件、279 次断言）：

```powershell
bun test --cwd ui src/common/types/agentPlatform src/renderer/pages/agentSettings src/renderer/hooks/agent/agentResourceSelection.test.ts src/renderer/components/agent/AgentResourcePicker.interaction.test.tsx src/renderer/pages/conversation/platforms/nomi/steerDraftSurvival.test.ts src/renderer/pages/conversation/utils/agentSwitch.test.ts
```

现存 warnings：后端有未使用入口/声明提示；桌面开发构建提示未启用 static WebUI；
UI 测试有 React/Arco ref、act 和模拟 DOM 高度提示。没有把 warning 记为零，亦未
为清理无关历史 warning 扩大本轮改动。

日志保留在本机 `.git/engine-win-*.log`；这些日志不会随源码提交。
本轮没有构建或运行独立社区示例、调用真实模型、启动用户服务、执行用户库迁移、
签名或安装 Windows 制品。开发检查不能替代上述尚未执行的产品验收。

## 跨平台 TODO（交接给对应机器）

- [ ] macOS arm64：同步本机最终交付版本及工作区内容，安装依赖，执行构建/安装/
  启动；检查旧外部 Runtime 打包残留、权限及子进程取消/回收。
- [ ] Linux x64：同步相同版本，确认 WebKit/系统依赖，执行构建/安装/启动；检查
  进程树、路径/文件权限与流取消行为。
- [ ] 两平台复用 Windows 的 Nomi/Coding 主链用例；重点覆盖工作台保存→Session、
  文件/命令、多轮续接、取消、恢复、压缩与 Fork 精确绑定，不另建社区示例门禁。
- [ ] 记录平台特有补丁、工具链版本和失败项；不得用 Windows 结果代替平台结果。
- [ ] 发布签名、公证和安装包制品验收由对应机器完成，不在本机伪造通过记录。

跨平台接手时保留同一重构分支，不恢复已退役 Wrapper，不更换 Session owner。
