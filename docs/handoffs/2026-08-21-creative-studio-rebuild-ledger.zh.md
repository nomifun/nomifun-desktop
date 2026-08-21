# Creative Studio 重建实施台账

> 用途：在长任务发生上下文压缩或人员切换时，从可验证提交继续，而不是重新审计整仓。
> 分支：`codex/infinite-canvas-rebuild`
> 最后功能锚点：`03b8d591`（`fix(creative-studio): align project center with source`）
> 参考产品锚点：`ef7303d`

## 1. 续接协议

每次继续实施先执行 `git status --short --branch` 和 `git log -1 --oneline`，确认工作树与本台账锚点。主线程一次只推进一个可提交切片；只有目录边界独立、能并行验证且能实质减少主线程负担时，才使用 1-2 个 Agent。Agent 不提交，主线程统一审阅、验证和 commit。

每个切片按以下顺序收口：

1. 明确产品状态、数据权威和文件边界。
2. 完成最小实现与定向测试。
3. 对 UI 使用相同视口和状态做源/目标并排比较，并检查动态交互、Console 与 Network。
4. 运行与风险匹配的 Rust/UI 检查。
5. 仅暂存该切片，执行 `git diff --cached --check` 后独立提交。
6. 只有阶段状态或门禁变化时更新本台账，避免把过程日志写成文档噪声。

## 2. 已完成的产品主体

| 阶段 | 当前实现 | 关键提交锚点 |
| --- | --- | --- |
| P1-P2 | 受保护运行时拆分、根主题、全屏 Focus Shell、侧栏入口与返回工作台 | `ed9c66df`…`954d6dcb` |
| P3 | `nomifun.creative-studio/v1`、UUIDv7、项目 CRUD/CAS、项目中心、ZIP 导入导出 | `d03a6a64`、`b4361084`、`44933dd3`…`2ad53b01` |
| P4 | 画布 reducer/history、视口、节点、连线、小地图、选择/分组/快捷键、Editor CAS 与离开 flush | `b2e19806`…`dc6c3c34` |
| P5 | Canonical 资产 API/库、文本/图像/视频/音频节点、素材选择与结果回填 | `57128727`、`e05b18a8`、`04f805a3`、`444db764` |
| P6 | NomiFun 精确任务模型目录、幂等任务、canonical owner、取消/恢复、pending 引用持久化 | `46545c21`、`b27d70d5`、`d5179e77`、`9897cc44` |
| P7 | 生图/视频工作台、工作流定义/运行中心、提示词与素材中心 | `2283ee74`、`1414846e`、`ebd17f3a`…`aad21d9d`、`7e45f8ad` |
| P8 | 持久化画布 Agent、原子操作、裁剪/切分/遮罩编辑与真实任务回填 | `e5368474`…`00f407c6`、`cac4bca8`…`bc01849e` |
| P9 | Director v1 domain、Three.js runtime、CAS sidecar、时间轴、截图回填、归档重映射 | `1ccbd013`、`d3a609f6`、`7b7712c0`、`dfa3c0b3`、`f25825f1` |
| P10 | 旧 Workshop UI、翻译、后端路由、旧画布存储与旧任务归属退出运行链 | `63c99d4f`…`7867fe3f` |

主体采用 React/Vite + Rust 原生集成，没有 Next.js、Go sidecar、iframe 或第二套模型配置。模型调用只走 NomiFun 的 provider/model/capability 解析和任务体系。

## 3. 已完成的 UI 证据

- Focus Shell：WebUI 与真实 Tauri 已验证入口、返回、刷新、深链、主题和原生窗口控制。
- 素材中心：1440×900 浅色源/目标对照完成；真实分页、搜索提交、上传限制、编辑、删除与合集重命名已有定向测试。
- 提示词中心：1440×900 浅色源/目标并排对照完成；7 个固定上游共同步 1592 条。搜索 Enter、分类、详情、复制、加入素材、刷新恢复及 30→60 渐进加载均真实通过；干净重载 Console 无错误。
- 提示词对照图：`C:\Users\MINISFORUM\.codex\visualizations\2026\08\19\01a01aa7-ea42-76e3-aa34-9158a1382c97\32-prompts-accepted-source-target-comparison.png`。
- 项目中心：新建/打开/返回、重命名与重载持久化、选择态、真实 ZIP 导出/导入、删除确认与取消均已验证；隔离导入副本通过精确 ID 清理。1440×900 浅色/深色与 1024×768 深色源/目标同状态对照完成，干净重载 Console 无错误。
- 项目中心对照图：`C:\Users\MINISFORUM\.codex\visualizations\2026\08\19\01a01aa7-ea42-76e3-aa34-9158a1382c97\38-projects-accepted-source-target-comparison-light-1440x900.png`、`41-projects-source-target-comparison-dark-1440x900.png`、`44-projects-source-target-comparison-dark-1024x768.png`。

`7e45f8ad` 的提交前检查：

- `cargo test -p nomifun-workshop`：70 passed。
- `cargo check -p nomifun-app --lib`：通过；输出仅有其他 crate 的既存未使用代码警告。
- 提示词 + 资产 client + canvas node factory：40 passed。
- `bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`cargo fmt --check`、`git diff --check`：通过。

`03b8d591` 的提交前检查：

- 项目中心定向测试：16 passed / 113 assertions。
- `bun run typecheck`、`bun run check:icons`、`bun run check:theme`、`bun run check:dead-css`、`git diff --check`：通过。
- UI production build：通过；仅保留仓库既存的动态/静态重复导入与大 chunk 提示。

## 4. 仍需明确保留的限制

- Standalone 工作台必须显式选择真实项目；不会借用或偷偷创建最近项目。
- v1 config node 只拥有一个 `taskId`，因此 standalone 视频批量数固定为 1。要支持 1-6 并行任务，必须先升级 canonical owner/schema/archive 契约。
- 手工素材上传当前只支持图片与视频；音频可由生成任务入库，但不伪装成已支持的拖放上传。
- Director 的 GLB/glTF 模型导入仍不可用；四/十二方位批量捕获和视频导出保持显式不可用。当前不使用参考项目的压缩 Director bundle 或来源不清模型。
- 390px 只承诺关键入口和状态不白屏；尚未宣称完整移动触控生产体验。
- 真实付费 provider 端到端冒烟尚未执行，需要可用凭证和单独的成本授权。

## 5. 下一步单线程优先级

1. 画布主链：新项目上逐项验证节点、拖拽/框选/缩放/连线、撤销重做、面板持久化、冲突和重开恢复。
2. 生图、视频与工作流：先验证无模型/失败/取消/恢复，再在得到成本授权后执行一个真实 provider 冒烟。
3. Director：验证场景、时间轴、截图回填、归档重开，并确认所有不可用入口描述准确。
4. P11：1280×720、1024×768、390×844、双主题、Tauri 慢环、UI build/桌面打包和最终能力文档。

不要因为主体代码已存在就跳过这些门禁；下一提交应从第 1 项画布主链验收开始，发现问题只修画布页及其直接契约。
