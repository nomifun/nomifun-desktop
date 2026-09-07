# main 产品修复合入 Agent Capability Platform v2

日期：2026-09-08。目标分支：`rf/agent-capability-platform-v2`。

- 合并前重构分支：`7ed336ad6ef146a65af896c64f52dc26a3c76329`。
- 本次 main 起点：`34a7695f3d0028713889cf29959adc7060d82a61`。
- 公共祖先：`2c58eb6574ddb03f65eeb802ac1528bae8510e80`。
- main 未合入历史共 22 个提交，采用保留两个父节点的 merge；产品架构继续以重构分支为准。

## 带入的产品修复

- 创意素材删除保留稳定身份、生成历史和终态引用；删除内容有待清理状态、不可复活约束，新生成/模板运行拒绝已删除素材。
- 画布中文输入法组合输入、光标和提及引用处理；画布节点连接、指针捕获、拖动及可见性。
- 素材操作菜单、统一封面、图片预览、共享视频播放器和视频节点交互。
- 模型选择器、宽高比/分辨率分开选择、生成面板布局及主题提示气泡。
- 主分支 v0.7.6 版本元数据、UI API contract 23、更新器和联系方式文档。此合并本身不构成新安装包发布。

## 冲突与架构处理

1. `Cargo.lock` 的旧 MiniApp/Preset 包冲突保留 `nomifun-miniapp-platform` 与 `nomifun-plugin-platform`，不恢复已退役的 `nomifun-miniapp` / `nomifun-preset` 包。锁文件仅有 76 个工作区包从 0.7.4 升至 0.7.6，第三方锁定版本不变。
2. 数据库生命周期断言继续使用重构分支的统一迁移头常量，并更新到 75。
3. Windows 文件身份读取继续使用重构分支的 `CreateFileW`、`FILE_READ_ATTRIBUTES`、路径处理与句柄关闭方式，补入 main 的 `FILE_FLAG_OPEN_REPARSE_POINT`，保留“不跟随重解析点”的保护。
4. 保留 NomiCore、AgentPreset/Revision/Snapshot、Plugin N1、MiniApp M1、逻辑引用和既有三项内置能力修复；合并没有改回旧执行/存储聚合根。

## 两条已发布迁移链的收敛

main 与重构分支在 058 后分别使用了 059/060。简单接受 Git 自动合并会出现重复迁移版本；直接改写旧 SQL 会破坏已安装数据库的校验和。

- 重构分支原有 001–072 文件原样保留；M1 Host KV 原本未发布的 073 顺延为 075。
- main 的 `059_workshop_asset_content_deletion.sql` 与 `060_workshop_asset_deletion_guards.sql` 以原始 SQL 字节接入 073/074。
- main 已发布 059/060 数据库：只接受完整 001–058 精确共同前缀，加上已成功执行且校验和完全匹配的 main 059、可选 060。
- 只在可写迁移阶段、一个外层事务内，把这两条 ledger 记录的版本号映射到 073/074，然后补齐重构分支 059–072 与 M1 的 075；原校验和、描述和安装时间保留。升级失败时连同版本号移动一起回滚。正常启动后即为唯一的 001–075 正式链，不保留第二套运行时 schema。
- 只读启动探测只报告 `UpgradeRequired`；不改 ledger、不退役目录。未知版本、失败记录和校验和篡改继续拒绝。
- 已验证 main 059、main 060、refactor 072/075、重复打开、失败回滚、篡改拒绝，以及素材历史/待删除内容保留。

SQL 原文字节核对（与远程 main blob 相同）：

| 正式版本 | main 原版本 | SHA-384 |
|---|---|---|
| 073 | 059 | `ae3e1cbb9d66050fc6c5c631b3f578cb15c12a4d7aaf6be284974765b050c91ab9d08ccdae271702b4cee36f9d536f65` |
| 074 | 060 | `cffa5536115edc38b2f23357931a351130f16f63f2cec440508c362805393c4d551dd7d96bfc2318e4cf920de2952aad` |

main 原测试中 060 的固定摘要与该提交的实际 blob 不一致；这里按已核实的原始 blob 修正测试摘要，未修改 SQL 内容。

## UI 测试入口

ReactDOM/Arco 会在模块加载时探测 DOM。新增 `ui/bunfig.toml`，统一预加载现有 `ui/test/setup-dom.ts`，避免不同文件加载顺序导致菜单/IME 用例虚假失败。素材菜单的外部鼠标点击测试补齐 mouseup，避免 React 的全局选择状态残留到后续光标用例。产品 IME 修复本身原样带入。

三份既有 HTTP wire 测试改为按路径、查询参数与载荷断言，同时接受桌面绝对 URL 和 WebUI 相对 URL；origin 选择仍由 httpBridge 的独立环境测试覆盖。

## 验证记录

- 数据库生命周期：31 passed。
- ID/schema 合同：20 passed。
- main 059/060 事务升级与失败回滚/篡改拒绝：4 passed。
- 素材删除增量迁移与不可复活保护：3 passed。
- 创意工坊、Agent 工作台、Guid、提示气泡 UI：1057 passed（207 个测试文件）。
- 类型检查、i18n、主题、图标、dead CSS、Windows installer contract、Creative Studio retirement：通过。
- Browser platform、automation Session dependency、Agent vocabulary：通过。
- 完整 UI 测试：3351 passed（611 个测试文件）。
- UI production build：通过；最终类型检查再次通过。
- 创建任务领域：72 passed；知识库：315 passed；创意工坊：138 passed，合计 525 passed。
- Nomi 能力投影：15 passed，保留已有内置能力与动态插件边界。
- 应用启动与数据根保护：21 passed；素材桥接：21 passed。
- 回滚用例改为在 067 制造冲突后再次通过：验证 059–066 已执行的 DDL、迁移编号移动全部回滚，原有资产与冲突表保留。
- Rust 回归覆盖共 640 个不同测试；完整 UI 覆盖 3351 个测试。没有把重复运行的用例重复计入数量。

仓库总检查的两项既有失败（相关脚本/代码相对合并前 HEAD 没有变化）：

- `check:process-runtime-boundary`：`nomifun-js-authoring/src/build.rs` 的 Windows 测试清理代码引用 `taskkill.exe`，被静态边界规则报告。
- `help --check`：`gate:plugin-n1` 已存在于 package.json，但未登记在 scripts/scripts.json。

本次不把上述基线失败写成全部仓库检查通过。未执行签名安装包发布、三平台原生验收或真实外部模型/硬件调用。

## 合入提交

| Commit | 内容 |
|---|---|
| `f30d1dd4e` | chore(release): v0.7.5 |
| `d41a5de65` | chore(release): add macOS updater entries for v0.7.5 |
| `b69287e71` | docs(release): note macOS v0.7.5 supplement |
| `ebe3fe45c` | docs: update WeChat group QR code |
| `e18a4b470` | docs: update WeCom group QR code |
| `89670cf67` | Merge branch 'main' of github.com:nomifun/nomifun-desktop |
| `58222e69d` | feat: 更新企业微信群联系方式 |
| `797ffede0` | fix: 统一提示气泡背景色 |
| `0acfd32c5` | fix: 优化画布模型选择器展示 |
| `3de7336ac` | fix: 移除画布生成配置展示 |
| `c553b607a` | style: 优化画布节点与操作面板 |
| `cfe96d466` | feat: 拆分创意工坊尺寸选择 |
| `a640fa55f` | feat: 优化创意工坊节点连接体验 |
| `28247c862` | feat: 添加画布图片预览并优化弹窗适配 |
| `57ef8befa` | style: 优化模型选择列表展示 |
| `5c0a183a8` | fix: preserve history when deleting creative assets |
| `3e3bbfd4f` | fix: 优化创意工坊视频节点体验 |
| `b1e2174e7` | fix: 统一创意素材封面预览 |
| `7f921c870` | fix: 优化创意工坊素材与视频 UI |
| `a6493a1ae` | style: optimize creative studio UI alignment |
| `52aa9dbb4` | fix: preserve canvas prompt IME composition |
| `34a7695f3` | chore(release): v0.7.6 |
