# 创意工坊：画布领域与独立工作台重构

- 日期：2026-08-23
- 状态：产品方向已确认；分阶段实施
- 本地 checkpoint：`main`（按用户要求不推送远程）

## 1. 用户确认的产品原则

1. 创意工坊内不需要“项目”概念。
2. 画布就是画布，不是项目的子页面，也不是项目的替代品。
3. 生图工作台、视频工作台与画布彼此独立。
4. 打开独立工作台时不得要求、推断或绑定画布。
5. 只有从画布节点发起的生成任务才与该画布节点关联。
6. NomiFun 主工作台中的“项目/工作路径”属于另一个产品领域，不得借名进入创意工坊。
7. 不允许用“默认项目”“临时项目”或“隐藏画布”规避建模问题。

## 2. 当前代码的真实模型

当前没有真正的 Creative Studio Project 聚合。

`creative_studio_projects` 中的一行直接保存：

- `document_json`
- `node_count`
- `connection_count`
- revision / created_at / updated_at

`CreativeProjectDocument` 直接包含 viewport、background、nodes、connections、panel、
Canvas Agent session 与 pending task IDs。它本质上就是一张持久画布。

当前错误任务所有权为：

```text
CanvasNode          = project_id + node_id
StandaloneWorkbench = project_id + workbench_kind
TemplateStep        = template_id + template_run_id + template_step_id
```

第二个分支导致所谓“独立工作台”必须绑定一张实际为画布的 project 行，并产生这些问题：

- `/workshop/image`、`/workshop/video` 无 `?projectId=` 时整页禁用；
- 工作台加载画布列表与画布详情；
- 历史、重试、退役按 `project_id + workbench_kind` 分桶；
- 独立工作台 live task 会阻止删除无关画布；
- 生成资产 origin 被错误写入画布 `project_id`；
- “创建项目/项目中心”实际上跳转到画布库。

## 3. 目标领域模型

```text
CreativeCanvas
  canvasId
  document / nodes / connections / viewport / CAS / archive

CanvasNodeTaskOwner
  canvasId + nodeId

StandaloneWorkbenchTaskOwner
  workbenchKind
  installation-owner authentication scope

TemplateStepTaskOwner
  templateId + templateRunId + templateStepId
```

### 3.1 页面边界

- `/workshop/canvases`：画布库。
- `/workshop/canvas/:canvasId`：无限画布。
- `/workshop/director/:canvasId`：当前仍是画布的专用导演编辑模式。
- `/workshop/image`：独立生图工作台，无画布参数。
- `/workshop/video`：独立视频工作台，无画布参数。
- `/workshop/prompts`：全局提示词。
- `/workshop/assets`：全局素材。
- `/workshop/templates`：全局模板定义与运行。

旧 `/workshop/projects` 只保留兼容重定向，不能继续作为产品真相。

### 3.2 独立工作台

- 没有任何 canvas/project selector；
- 无画布时也能完整使用；
- 历史按 `workbenchKind` 保存和读取；
- 结果进入全局素材库；
- “插入画布”是生成后的显式动作，不是生成前的归属要求；
- 不自动选择最近画布。

## 4. 数据兼容策略

已发布 migration 037–046 不得修改，否则 SQLx checksum 会让已有数据库启动失败。

### 4.1 建议新增 migration 047

重建 `creation_tasks`，允许 standalone 分支只有 `workbench_kind`：

```text
CanvasNode:
  project_id not null, node_id not null, workbench_kind null

StandaloneWorkbench:
  workbench_kind not null, node/template fields null
  project_id may remain only as legacy inert provenance

TemplateStep:
  template ids not null, project/node/workbench fields null
```

新 standalone 任务写 `project_id = NULL`。旧 standalone 行可以保留历史 `project_id`，
但当前 owner equality、历史查询、退役、资产 origin 和画布删除都必须忽略它。

索引调整为：

```text
(workbench_kind, deleted_at, submitted_at DESC, creation_task_id DESC)
```

### 4.2 幂等 fingerprint

`request_fingerprint` 当前包含完整 owner 且按原始字节比较。不能在 SQL 中盲目替换。

推荐：

- 旧行保留原始 fingerprint；
- 新 wire bump API contract；
- 新请求使用无 project_id 的 standalone owner；
- direct GET / boot recovery 持续读取旧行；
- 若必须支持跨版本同 Idempotency-Key 重放，再增加 fingerprint 版本与语义规范化桥。

### 4.3 归档

旧 `.nomifun-canvas.zip` v1 reader 必须长期可读；旧 manifest 的 `project/projectId` 是历史
wire，不代表新产品仍有 Project。后续新导出可增加 canvas v2 manifest，不能破坏 v1。

## 5. 分阶段实施方案

### Phase 0：连续性修复（本分支已完成）

- 记住当前应用会话内最后一个合法创意工坊完整地址；
- 主侧栏再次进入时恢复该地址；
- 非法、未知、外部或超长地址回退 `/workshop`；
- 保留显式创意工坊首页入口；
- 不自动推断最近画布。

### Phase 1：独立工作台真正解耦

1. 后端 owner union 去掉 standalone 的 `project_id`。
2. history GET / retire body 去掉 `project_id`。
3. repository 按 `workbench_kind` 分页、恢复和退役。
4. 新增 migration 047，兼容旧 standalone 行。
5. 新资产 origin 只写 `workbench_kind`。
6. 画布删除只检查 CanvasNode live task。
7. 前端删除 scope bar、画布选择器与 Unowned disabled 页面。
8. image/video route 进入即用。
9. `ui-api-contract-version.txt` 20 → 21，然后重新构建 UI。

### Phase 2：Canvas 产品 façade

1. canonical route 改为 `/workshop/canvases`；旧 `/projects` redirect。
2. 产品层引入 `CreativeCanvasDocument/Summary/Detail/Repository`。
3. frontend adapter 可暂时映射旧 `/api/creative-studio/projects` wire。
4. UI、错误、通知、按钮彻底移除 Project 术语。
5. Director 统一改为当前画布语义。

### Phase 3：外部 API / Gateway / archive v2

- 新 `/api/creative-studio/canvases`；旧 `/projects` deprecated alias。
- 新 Gateway `list_canvases/get_canvas`；旧 project 能力短期兼容。
- 新 canvas v2 archive writer；保留 v1 reader。
- Agent session / proposal receipt 产品层改用 canvas 命名。

### Phase 4：工作草稿连续性

当前 route resume 只恢复地址和后端持久状态。image/video 的 prompt、model、参数、
reference IDs、布局仍是 route-local state。

增加按 `workbenchKind` 的版本化 session draft：

- 保存 prompt、exact model identity、生成参数、reference asset IDs、布局；
- 不保存 busy/error/modal open/完整 asset 对象；
- 恢复时按 asset ID 重新 hydrate；
- key 中不得包含 projectId/canvasId。

## 6. 禁止方案

- 隐藏创建一个 project/canvas 作为 standalone owner；
- 自动绑定最近画布；
- 只隐藏 selector、后端仍按 project_id 分历史；
- 原地修改 migration 037–046；
- 全仓机械替换“项目”，误伤软件项目、Requirements 项目和历史 handoff；
- 删除旧 archive reader 或旧 task direct-read 兼容。

## 7. 关键文件

### 前端

- `ui/src/renderer/pages/creativeStudio/app/routes.ts`
- `ui/src/renderer/pages/creativeStudio/app/CreativeStudioSider.tsx`
- `ui/src/renderer/pages/creativeStudio/projects/**`
- `ui/src/renderer/pages/creativeStudio/workbenches/product/{ownership,shared}.tsx`
- `ui/src/renderer/pages/creativeStudio/workbenches/product/{Image,Video}WorkbenchProductRoute.tsx`
- `ui/src/renderer/pages/creativeStudio/tasks/{types,client,historyClient}.ts`
- `ui/src/renderer/pages/creativeStudio/workbenches/{history,runtime}/**`
- `ui/src/renderer/pages/creativeStudio/canvas/**`
- `ui/src/renderer/pages/creativeStudio/director/**`

### 后端与数据库

- `crates/backend/nomifun-creation/src/{routes,service,dto}.rs`
- `crates/backend/nomifun-db/src/repository/{creation_task,sqlite_creation_task,sqlite_workshop}.rs`
- `crates/backend/nomifun-db/migrations/047_*.sql`（待创建）
- `crates/backend/nomifun-workshop/src/{routes,service,creative_studio,archive}.rs`
- `crates/backend/nomifun-gateway/src/caps_creative_studio.rs`
- `crates/backend/nomifun-conversation/src/creative_studio_agent_session.rs`

## 8. 最终验收

1. 空数据库、零画布时，image/video 可直接使用。
2. 独立工作台请求、响应、历史 URL 和 retire body 均无 project/canvas 字段。
3. 多张旧伪 project 下的同类 standalone 历史合并且分页稳定。
4. 删除任意画布不影响 standalone live task、历史或产物。
5. CanvasNode live task 仍能阻止删除对应画布。
6. 旧 task ID 可 direct GET、boot resume；旧 archive 可导入。
7. 返回工作台再进入，恢复页面、历史与未提交草稿。
8. 全流程不创建隐藏 project/canvas。
9. 新开页面 Console 0 error / 0 warning。
10. 文档与 README 截图不再出现旧顶部导航或“返回项目”。
