# 创意工坊 Canvas 领域重构跨机器交接

- 日期：2026-08-23
- 仓库：`nomifun/nomifun-desktop`
- 分支：本机 `main`（仅本地 checkpoint，不推送远程）
- 基线：`e573ed08`（已同步 `origin/main`）
- 设计规格：[`../specs/2026-08-23-creative-studio-canvas-domain-redesign.zh.md`](../specs/2026-08-23-creative-studio-canvas-domain-redesign.zh.md)

## 1. 需求背景

用户先要求统一创意工坊与设置的应用壳层，随后发现两个连续性/领域问题：

1. 返回工作台后再次进入，固定回到初始首页，产生“一切从头开始”的中断感。
2. 创意工坊错误引入 Project 概念：所谓 Project 实际就是一张 Canvas；独立生图/视频
   还被要求绑定该伪 Project/Canvas。

用户最终确认：创意工坊没有 Project；Canvas、Image Workbench、Video Workbench 都是
独立产品对象。主工作台的项目/工作路径是另一个领域。

## 2. 本阶段保留的代码进展

本分支只保留已完成、可验证的连续性修复：

- `resumeLocation.ts`
  - session-scoped last Creative Studio location；
  - exact route validation；
  - search/hash 保留；
  - 非法/外部/未知/超长值回退 `/workshop`。
- `Sider/index.tsx`
  - 进入创意工坊时使用 last location，不再固定 root。
- `resumeLocation.test.ts`
  - 深链、动态 canvas/director、异常 storage、fail-closed 测试。
- 中英文 guide 与 frontend architecture 同步连续性合同。

真实 UI 已验证：

- prompts → 返回工作台 → 再进入，恢复 `/workshop/prompts`；
- 完整 query/hash 能恢复；
- 新开验证页面 Console 无错误。

## 3. 明确没有推送的半成品

曾在本地探索性修改 standalone owner、history、migration 047 与 image/video route。
该实现尚未完成全链测试，已从可执行代码中撤回，避免远程分支处于无法编译状态。

不要从 Git 历史寻找该半成品；其设计结论、风险与精确文件清单已完整写入设计规格。

## 4. 为什么必须跨层改

仅删除 UI selector 会留下隐藏错误：

- owner 仍是 `projectId + workbenchKind`；
- history/retire 仍按 project 分桶；
- live standalone task 仍阻止画布删除；
- asset origin 仍虚构画布归属。

Phase 1 必须在同一合同变更中协调 UI、Rust service/DTO/routes、DB repository 和 append-only
migration，并 bump UI/API contract。

## 5. 推荐下一步顺序

1. 阅读设计规格全文和本 handoff。
2. 同一台机器上的下一个 Agent 直接在本地 `main` checkpoint 继续，先确认工作树干净；
   不需要切分支，也不需要应用 patch。
3. 其他机器才以 `e573ed08` 为基线应用交付的 format-patch；不要假设远程存在本次
   checkpoint 分支。
4. 先写 migration 047 与 DB migration test，锁定 legacy/new owner 兼容。
5. 改 Rust owner/list/retire/asset origin，跑 creation + db 测试。
6. 改 TS owner/history/runtime，跑 tasks/workbench 测试。
7. 删除 standalone scope bar 和 disabled unowned UI。
8. 做空数据库真实 UI 验证。
9. 再进入 Canvas façade 与 `/workshop/canvases` 阶段。
10. 最后做 session draft、README 截图和全量文档。

## 6. 数据兼容高风险

- migration 037–046 已发布，不得修改。
- `request_fingerprint` 是原始字节精确比较，不能 SQL 字符串替换。
- 旧 standalone row 的 project_id 可以保留为 inert provenance，但不能继续参与 owner equality。
- 旧 `.nomifun-canvas.zip` v1 必须可读。
- Canvas Agent session / proposal receipt 的删除策略存在旧缺口，不能顺手猜测修复。
- `ui-api-contract-version.txt` 当前为 20；wire 变更需要 21 并重新 build UI。

## 7. 当前文档与截图债务

现行 README、STATUS、architecture/API docs 仍多处描述 project-owned standalone tasks。
具体路径见设计规格。

这两张截图已过期：

- `docs/images/readme/zh/creative-workshop.png`
- `docs/images/readme/en/creative-workshop.png`

它们仍展示旧顶部导航，中文版还显示“返回项目”。最终产品验证后重拍，不要现在用 mock
或静态改图替换。

## 8. 验证命令

阶段一最低验证：

```powershell
bun run typecheck
bun test --cwd ui src/renderer/pages/creativeStudio/tasks `
  src/renderer/pages/creativeStudio/workbenches `
  src/renderer/pages/creativeStudio/app

cargo test -p nomifun-creation
cargo test -p nomifun-db --test creative_studio_task_owner_migration
cargo test -p nomifun-db --test id_schema_contract
cargo test -p nomifun-workshop
cargo check -p nomifun-app

bun run check
bun run build:ui
```

最终还需真实 UI：零画布直接进入 image/video、创建/恢复/retire 历史、删除画布隔离、
返回后草稿连续性、窄屏与 Console。

## 9. Git 边界

- 使用本机现有 Git 身份，不加 AI trailer。
- 本阶段按用户要求在本地 `main` 创建 checkpoint，但不推送任何远程。
- 同机 Agent 继续使用当前本地 `main`；不要为形式上的分支隔离搬动 checkpoint。
- 跨机器继续时建议从 `e573ed08` 新建 `codex/creative-studio-canvas-domain`，再应用
  `creative-studio-canvas-domain-checkpoint.patch`。
- 不 force-push，不改写已发布 migration。
- 未获得新的明确授权前，不得把本地 `main` 或后续开发分支推送到 origin。
- 提交前 stage 精确文件并跑 `git diff --cached --check`。
- 推送前 fetch，再确认远程没有新分叉。

## 10. 本机非仓库审查资产

旧机器上有临时 UX 审查截图与报告，位于 `.codex/visualizations/.../creative-studio-continuity-audit/`。
它们不是继续开发的依赖；跨机器以本规格与实际运行页面为准。
