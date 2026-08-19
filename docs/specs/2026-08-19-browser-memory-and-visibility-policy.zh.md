# Browser Use 内存误判修复与可见性策略三态化（2026-08-19）

## 1. 背景与用户报告

用户报告：「agent 触发 browser use 使用浏览器的时候很容易触发内存问题而关闭，
使用体验非常差」。

排查结论：**不是内存泄漏，是误判 + 秒杀**。一次普通的单 Agent 浏览会话，在页面
加载完成后约一个采样周期（5 秒）内就会被强制关闭，并且被当作「用户关闭了浏览器」
上报给 Agent。

同时确定了第二项需求：静默/前台的选择不应该由用户为所有场景一次性指定，而应由
浏览器按场景判断，用户只提供一个兜底倾向。

## 2. 根因（三项缺陷叠加）

### 2.1 度量本身虚高约 1.7 倍

`sample_browser_resources` 把整棵 Chromium 进程树的 `sysinfo::Process::memory()`
累加。在 Windows 上该值是 `WorkingSetSize`（工作集），**包含与兄弟进程共享的页**：
`chrome.dll` 等共享镜像映射进每个子进程，累加等于把这些页按进程数重复计算。

本机九进程 Chrome 实测：

| 指标 | 合计 |
| --- | ---: |
| `WorkingSet64`（原实现所用） | 696.3 MiB |
| `PrivateMemorySize64`（私有提交量） | 413.1 MiB |
| 被重复计算的共享页 | 283 MiB（41%） |
| 虚高倍数 | 1.69× |

`sysinfo` 在 Windows 上已经把私有提交量暴露为 `Process::virtual_memory()`
（映射到 `PROCESS_MEMORY_COUNTERS_EX::PrivateUsage`）。私有提交量是进程独占的，
跨进程累加才是有效的。

**平台陷阱**：在 Linux/macOS 上 `virtual_memory` 是虚拟地址空间大小（VSZ），
用它会更糟；且 `sysinfo` 不暴露 PSS。因此修正必须 `#[cfg(windows)]` 限定，
其他平台保留 RSS，其树总量仍是上界。

### 2.2 预算装不下一个真实浏览器

单任务独占一个 Host 时会被归因整棵进程树，其中包含 Chromium 的固定基线
（browser + GPU + network/storage + crashpad），这部分在渲染任何页面之前就存在。
对照原来的 1 GiB，一棵**空闲**树已经量到约 700 MiB。

### 2.3 回收瞬间升档，且会拿走任务唯一的 lane

severity/confidence 加速项让 streak=1 就足以进入可回收档；而与
`freeze_idle_lane_for_pressure` 不同，回收路径**没有**最后一条 lane 的保护。
于是一个在两次工具调用之间等待模型思考的 Agent（这是大部分时间），在第一个
超预算采样就丢掉了浏览器。

### 2.4 报错甩锅给用户且禁止重试

回收抛出 `LaneClosedByUser`（"The browser lane was closed."，`retryable: false`）。

## 3. 已实施的修复

| 改动 | 位置 |
| --- | --- |
| Windows 归因改用私有提交量 | `services.rs::process_tree_attributable_bytes` |
| `AUTOMATIC_TASK_MEMORY_BYTES` 1 GiB → 2 GiB | `resource.rs` |
| `RESOURCE_SAVING_TASK_MEMORY_BYTES` 768 MiB → 1.25 GiB | `resource.rs` |
| 新增 `TASK_RECLAIM_MIN_SUSTAINED_SAMPLES = 3` 滞回下限 | `hub.rs` |
| 任务唯一 lane 仅最高档可回收 | `hub.rs::reclaim_over_budget_tasks` |
| 新增 `TaskMemoryReclaimed`（`retryable: true`，映射 429） | `error.rs`、`browser_management.rs` |

### 3.1 两条不可放宽的不变量

1. **滞回下限**：「昂贵」与「泄漏」是两件事。浏览器可以合法地长期停在高位
   （几个媒体密集标签页），而 streak 只在回落到预算以下才清零——没有滞回，
   稳定会话会与失控会话被同等惩罚。
2. **最后一条 lane 保护**：单 lane 是 Agent 任务最常见的形态，关掉它等于关掉
   用户的整个浏览器。

**特别注意**：最后一条 lane 的保护**不得**额外要求 `severely_over`。那样会让
「单 lane + 持续中度超限」永久免疫回收，是漏洞而非保护。爬到最高档本身就是保护。

## 4. 可见性策略三态化

### 4.1 一个决定性约束

可见性切换**不是窗口开关**，而是替换 Chromium Host 进程：
`set_lane_visibility_for_user` → `set_lane_visibility_and_maybe_focus_once`
→ `transition_primary_visibility_locked` → Host 重启。
`HOST_RESTART_ATTEMPT_TIMEOUT` 为 75 秒，且由于 Primary 各 lane 共享一个规范
Host，替换会在新 epoch 下重绑**所有**存活的 Primary lane。

因此：**开 lane 时决定是免费的，运行中再决定不是。**

### 4.2 四层设计

**第 1 层 · 用户偏好（兜底与硬边界）**

| 取值 | 语义 |
| --- | --- |
| `headless` | 永远静默，即使遇到需要用户介入的时刻 |
| `auto` | **新默认**：宿主按 lane 裁决 |
| `external` | 永远以真实窗口启动 Primary |

**第 2 层 · 模型意图（建议，非权威）**

工具接受 `presentation`：`unattended`（默认）/ `attended`。模型只表达**意图**，
不得指定机制；传 `headless`/`headful`/`external`/`visible` 等机制词会被**拒绝**
并给出改用意图的提示，而不是静默降级为例行。这与
`MODEL_IDENTITY_INPUT_FIELDS` 拒绝模型指定身份/档案是同一条纪律。

两个上报点：开 lane 时，以及在运行中 lane 上派发操作前。后者才是主场景——真实
流程是「导航 → 撞上登录墙 → 此时才需要用户」。

**第 3 层 · 宿主裁决（权威）**

`resolve_lane_visibility(policy, intent, identity_mode)` 与
`may_escalate_lane_to_headful(policy, intent, identity_mode, current, used)`
为纯函数，策略以真值表形式可审可测。

**第 4 层 · 单向升级**

- 仅 `auto` 会升级；`headless` 已向用户承诺不弹窗，`external` 本就可见。
- 仅 Primary：只有它承载用户真实登录态。Anonymous 撞登录墙应报
  `NeedsPrimaryIdentity`，而不是在无法登录的档案上弹窗。
- **只朝可见方向**：让用户能看见并接手是安全方向；把用户正在监督的工作藏起来
  是透明性倒退，因此**故意不提供**降级路径。
- 每 lane 上限 `MAX_LANE_VISIBILITY_ESCALATIONS = 2`，因为每次都是一次进程替换。

### 4.3 实施中修掉的两个真实缺陷

1. **迁移会让窗口复活**。v2 的注释写明：无版本标记的 `external` 可能是从已废弃的
   `silent=false` **推断**出来的，v2 特意阻止了这类状态弹窗。第一版迁移无条件保留
   `external`，会让这些用户重新被弹窗。已改为按世代区分：仅 **v2 标记**证明是明确
   选择才保留；无版本一律迁到 `auto`。仓库既有测试
   `..._unversioned_external_to_headless` 抓到了这个回归。
2. **升级会形成重启循环**。第一版从 `config.headful` 读取当前可见性，但按 lane 的
   切换**故意不改**安装级默认值，于是已可见的 Host 被判为静默，每次上报都再升级
   一次直到用尽额度。已改为读取 Host slot 的实际状态。新增测试断言 epoch 稳定。

## 5. 契约变更

`display_mode` 与机制在只有两个取值时是同构的，因此 `GET /api/browser/display-mode`
原先从实时 Host 可见性反推 `display_mode`。加入 `auto` 后这条推断失效：`auto` 与
`headless` 都表现为 headless，策略无法从机制还原。

```
GET /api/browser/display-mode
{
  "display_mode": "auto",              // 用户策略，取自持久化存储
  "effective_visibility": "headless"   // 当前机制，只读
}

PUT { "display_mode": "auto" }
```

`PUT` 使用独立请求类型，因此 `deny_unknown_fields` 会**拒绝**客户端伪造的
`effective_visibility`，而不是静默忽略。

**`ui-api-contract-version.txt`：19 → 20。**

偏好世代标记 `BROWSER_DISPLAY_MODE_POLICY_VERSION`：`2` → `3`；
`agent.browserUse.displayModeVersion` 类型放宽为 `2 | 3`，因为迁移仍需读取旧标记
来判定 `external` 是否为明确选择。

## 6. 迁移矩阵

| 存量状态 | 迁移结果 | 理由 |
| --- | --- | --- |
| v3 + 合法值 | 原样保留 | 权威 |
| v2 + `external` | `external`（重新盖 v3 标记） | v2 标记证明是明确选择，不能悄悄收回 |
| v2 + `headless` | `auto` | v2 对**所有**安装都持久化 `headless`，它反映旧默认而非决定 |
| 无版本 + `external` | `auto` | 可能是从 `silent=false` 推断而来，**不得**让窗口复活 |
| 无版本 / 更旧 / legacy-silent | `auto` | 失败关闭方向，且 `auto` 仍静默启动 |
| v3 + 非法值 | `auto` | 修复 |

前端 `migrateBrowserDisplayMode` 与后端 `resolve_browser_display_mode` 实现同一套
规则，避免两侧漂移。

## 7. 验证

| 检查 | 结果 |
| --- | --- |
| `nomifun-browser-platform --lib` | 272 passed / 0 failed |
| `nomi-browser --lib` | 236 passed / 0 failed / 6 ignored |
| `nomifun-app --features browser-use --lib` | 462 passed / 0 failed |
| `bun run test:ui` | 2212 passed / 1 failed（见 §8 上游既有） |
| `check:i18n` / `theme` / `icons` / `dead-css` / 两个边界检查 / `agent-vocabulary` | 全部通过 |
| `cargo fmt`（改动包）、`git diff --check` | 通过 |

未执行：真实 Chrome 的 `integration_managed_host --ignored` 验收集。
`cargo fmt --all` 在本机因 Windows 路径过长报 os error 206，改用 `-p` 逐包检查。

## 8. 上游既有破损（非本次引入）

均在 `origin/main` 的 `180cabe0` 上、不带本次任何改动复现过：

- `bun run typecheck` 报 4 个错误，位于 `AboutModalContent.tsx` 与
  `FeedbackReportModal.tsx`，源自 `9876b2f1 chore: scrub public contact pii`
  删除了 `email`/`emailHref`/`trailingFallback` 但调用点仍在引用。这会让
  `bun run check` 在 typecheck 阶段提前退出，因此后续各项检查需单独执行。
- `CreateStudio form visual design > keeps the dialog and configuration cards
  compact...` 失败，源自 `1c5f214c style(ui): unify modal visual contract`。

本次未修复上述两项，它们属于引入它们的上游提交。

另有一项环境性间歇：`nomifun-app` 全量跑偶发**一个**失败，且每次换一个不同测试
（`oversized_body_...`、`active_owner_bindings_...`），失败点是 loopback
`.send().await.unwrap()` 传输错误而非断言，隔离重跑 8/8 与 3/3 通过；与
`docs/handoffs/2026-08-04-browser-use-task-resource-hardening.md` §4.5 记录的
特征一致。

## 9. 必须诚实表达的产品边界

共享 Chromium Host 上**无法**做到精确的按任务物理内存归因——
`shared_rss_estimate_bytes` 本质是估算。因此估算值只应用于限流与降级，
**不应**作为强杀用户前台工作的唯一依据。若确实需要硬性物理隔离，唯一正确做法是
每任务独立 Host + Job Object/cgroup，并接受基线进程与内存的增加。

同理，per-lane 的可见性升级是一次进程替换，不是窗口属性切换；任何把它描述为
「切换窗口显示」的文案都会误导用户对其代价的预期。

## 10. 关键代码位置

- 归因度量：`crates/backend/nomifun-app/src/services.rs`
- 资源策略常量：`crates/backend/nomifun-browser-platform/src/resource.rs`
- 回收与升级：`crates/backend/nomifun-browser-platform/src/hub.rs`
- 决策内核（纯函数 + 真值表）：`crates/backend/nomifun-browser-platform/src/model.rs`
- 错误码：`crates/backend/nomifun-browser-platform/src/error.rs`
- 意图解析与转发：`crates/agent/nomi-browser/src/managed.rs`
- 工具 schema：`crates/agent/nomi-browser/src/tool.rs`
- 管理 API 与策略持久化：`crates/backend/nomifun-app/src/router/browser_management.rs`
- 前端迁移与设置：`ui/src/common/browser/browserSettings.ts`、
  `ui/src/renderer/components/settings/SettingsModal/contents/BrowserUseSettingsContent.tsx`
