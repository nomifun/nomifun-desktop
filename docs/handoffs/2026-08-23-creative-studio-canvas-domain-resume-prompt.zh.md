# 跨机器继续开发启动 Prompt

```text
请继续开发 nomifun/nomifun-desktop 的创意工坊 Canvas 领域重构。

本次 checkpoint 已提交在当前机器的本地 main，但没有推送远程。
原始基线：e573ed08。
跨机器备用补丁：creative-studio-canvas-domain-checkpoint.patch

开始前：
1. 如果仍在原机器：进入 nomifun-desktop，保持当前本地 main，不要应用 patch。
2. 如果在其他机器：从 e573ed08 创建开发分支并应用随交接复制的 patch：
   git switch -c codex/creative-studio-canvas-domain e573ed08
   git am creative-studio-canvas-domain-checkpoint.patch
3. 完整阅读：
   - docs/specs/2026-08-23-creative-studio-canvas-domain-redesign.zh.md
   - docs/handoffs/2026-08-23-creative-studio-canvas-domain-handoff.zh.md
   - 仓库 AGENTS.md
4. 检查 git status、当前分支、origin、HEAD 与 Git 身份，保留用户无关改动。

用户确认的不可违背产品原则：
- 创意工坊内没有 Project 概念，Canvas 就是 Canvas。
- Image Workbench、Video Workbench 与 Canvas 彼此独立。
- 独立工作台不得要求、推断或绑定 Canvas。
- 只有 Canvas 节点发起的任务才归属 canvasId + nodeId。
- 不得创建默认项目、临时项目或隐藏画布。
- 主工作台“项目/工作路径”是另一领域，不得混入创意工坊。

当前已完成：
- 返回工作台后再次进入会恢复当前应用会话中最后一个合法创意工坊地址。
- 非法/未知/外部地址 fail-closed 到 /workshop。
- 该连续性修复已有 TS 测试和中英文文档。

当前尚未实施：
- standalone owner/history/API/DB 去 Canvas 绑定；
- /workshop/canvases 与 Canvas 产品 façade；
- image/video 未提交草稿恢复；
- README 新截图和全量 Project 术语收口。

请从规格的 Phase 1 开始：
1. 新增 append-only migration 047，不修改 037-046。
2. 目标 owner：
   CanvasNode { canvasId, nodeId }
   StandaloneWorkbench { workbenchKind }
   TemplateStep 保持现状。
3. 新 standalone 写 NULL legacy project_id；旧 project_id 只作 inert provenance，历史按 kind 合并。
4. GET history 与 retire body 去掉 project_id。
5. 新 standalone asset origin 只带 workbench_kind。
6. 画布删除只受 CanvasNode live task 限制。
7. 协调前端删 scope bar/selector/Unowned disabled 页面，零画布时 image/video 也必须可用。
8. wire 变化将 ui-api-contract-version.txt 从 20 bump 到 21，并 build UI。

不要只做文案替换或隐藏 selector。每完成一个协调层就跑相应定向测试；Phase 1 全绿后再开始
Canvas façade/路由重命名。不要在没有真实验证时声称数据迁移、桌面窗口或生成链路已通过。
未经用户新的明确授权，不要推送远程。
```
