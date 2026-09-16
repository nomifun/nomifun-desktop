# UARC 并发、合并、删除与验证协议

## 1. 目标

本协议防止四类效率损失：

1. 多人同时修改共享合同造成代码合并风暴；
2. 多人同时重写同一领域造成编辑风暴；
3. 每个任务重复跑全仓检查造成测试风暴；
4. 新实现完成后旧代码仍可达造成长期维护风暴。

## 2. 任务领取

任务只有满足以下条件才能从 `planned` 进入 `active`：

- 所有 `depends_on` 已 `integrated`；
- 当前 wave barrier 已记录；
- `write_set` 与活动任务无交集；
- `delete_set` 和 `retained_set` 已填写；
- 平台范围已填写；
- UI 任务已有 `interaction_spec`；
- Integration 已分配一个 Feature 席位。

`STATUS.zh.md` 由 Integration 写入领取结果。聊天中的“我开始做”不构成正式领取。

## 3. Worktree 与分支

- 每个任务使用独立 worktree；
- 分支名：`codex/uarc-<task-id-lowercase>-<short-name>`；
- 所有同波任务从同一个 barrier commit 创建；
- Worker 不 pull/merge 其他任务分支；
- 共享合同变化只能由 Integration 在下一 barrier 发布；
- 一个任务以一个主提交交付，必要修复最多一个附加提交；
- 不 force-push 已交付给 Integration 的任务分支。

## 4. 文件所有权

### Integration-only 文件

见总计划 §4.1。Worker 发现必须修改时，提交如下请求而不是直接编辑：

```text
Integration request
  task_id
  target file
  exact API/schema field
  reason
  consumer call site
  required tests
```

Integration 将多个领域请求合并成一次共享修改，避免 N 个 Worker 分别碰同一文件。

### 临时所有权升级

若某文件意外成为两个任务的共同依赖：

1. 两个任务停止修改该文件；
2. Integration 将文件移入 integration-only；
3. 抽出稳定接口；
4. 发布新 barrier；
5. Worker 继续各自消费者实现。

不允许通过复制文件或 DTO 绕开所有权冲突。

## 5. 交付包

Worker 交付必须包含：

```text
Task: UARC-xxx
Barrier/source commit:
Commits:
Changed files:
Deleted files:
Retained legacy files + reason:
Behavior delivered:
Tests run + exact result:
Tests not run + reason:
Windows status:
macOS status:
Known limitations/blockers:
Integration requests:
```

缺少删除说明或平台状态的交付不进入 merge queue。

## 6. 合并队列

Integration 对每项任务依次执行：

1. 检查任务仍基于正确 barrier；
2. 检查实际 diff 是否超出写集；
3. 检查有没有把 shared contract、生成物或根依赖偷偷带入；
4. 复核产品行为、错误边界和 delete set；
5. 运行 per-merge tests；
6. 合并一个任务；
7. 更新状态台账；
8. 再处理下一个任务。

同一时刻只处理一个 merge。多个绿色任务排队不会并行落入 Integration branch。

### 冲突处理

- 互斥写集内出现冲突说明计划失效，不能机械选择 ours/theirs；
- 由 Integration 判断真正 owner，并把另一个任务退回新 barrier；
- 共享合同冲突由 Integration 重做，不让两个 Worker分别解决；
- 纯相邻 import/format 冲突可由 Integration 修复，随后运行双方定向测试；
- 不允许为了合并保留两条生产路径。

## 7. 删除协议

### 每项任务

任务必须把被替代的旧实现分为：

- 本任务立即删除；
- 明确由后续任务删除；
- 继续复用并迁入新边界。

第二类必须指向一个已存在的 task ID。没有 owner 的“后续删除”视为本任务未完成。

### 删除证据

- `rg` 生产可达引用为零；
- Cargo/TS import graph 无旧模块；
- 路由、命令、UI、配置和环境变量入口清零；
- 旧测试和 fixture 不再运行或被误认成产品证据；
- 文档标记历史或删除；
- dead CSS、i18n、icon、schema/generator 检查通过；
- 删除不依赖 commented code 或永久 compatibility wrapper。

### 删除窗口

领域任务删除自己完全替代的局部代码；跨领域 legacy root 由 Wave 6 的 UARC-052/053/054 统一删除。
Integration 不允许在中间波保留“新旧均可用”的未登记状态。

## 8. UI 产品协议

### 交互设计先行

UI task 开工前将以下内容写入 task 说明或专用设计段：

- entry/exit；
- primary user flow；
- state matrix；
- information hierarchy；
- destructive/external action feedback；
- keyboard/focus；
- responsive desktop constraints；
- Windows/macOS difference；
- final copy keys。

没有交互设计，不得先堆组件再让 Integration 猜产品行为。

### 视觉要求

- 复用现有设计 token 和组件；
- 模块、Action 和资源状态有明确层级；
- 推荐能力与已绑定资源清晰可见；
- 技术详情折叠，不污染主流程；
- 空状态给出下一步而不是空白；
- 错误信息说明用户能做什么；
- loading 不造成布局跳动；
- 危险操作确认与普通选择视觉区分；
- 所有交互可由键盘完成。

### 验证

- 纯逻辑 model tests；
- 关键 DOM interaction tests；
- i18n parity；
- typecheck；
- desktop UI boundary；
- Windows 实际窗口检查；
- macOS 实际窗口、Retina、焦点、输入法和原生权限检查。

快照或视觉记录只证明布局，不代替真实点击、焦点和 native surface 验证。

## 9. 测试租约

每台机器维护一个逻辑 test lease：

| Lease | 并发 | Owner |
| --- | ---: | --- |
| Cargo | 1 | 当前获批 Worker 或 Integration |
| Full UI | 1 | Integration |
| Native desktop | 1 | Platform owner |
| Packaging | 1 | Integration/Platform owner |

领取 Cargo lease 的任务可运行自己 crate tests；其他 Worker 同时只做代码、文档或不需要 Cargo 的
定向 JS tests。禁止多个 Cargo 链接任务竞争内存、磁盘和构建目录。

## 10. 测试层级与失效规则

### 结果复用

测试结果只有在以下均未变化时可复用：

- 被测源文件；
- 直接/共享合同依赖；
- feature flags；
- platform target；
- fixture/生成物。

Integration 合并影响上述任一项时，只使相关结果失效，不盲目重跑所有任务。

### 失败处理

- Worker 自己的定向测试失败：留在任务分支修复；
- Integration per-merge 失败：撤出该任务，不让后续任务叠加；
- Wave gate 失败：冻结新合并，定位到最小任务/共享修改；
- 平台专属失败：shared task 可保持 integrated，但平台状态为 blocked/pending，initiative 不完成；
- flaky 必须先复现和归因，不用连续重跑掩盖。

## 11. macOS 回合并

macOS task 只能修改 manifest 声明的 macOS/platform files。需要 shared interface 变化时提交 integration
request，由 Integration 先修改 shared contract，Windows 回归通过后再继续 Mac 实现。

Mac task 合入后：

1. 运行 shared Rust/UI tests；
2. 运行受影响 Windows compile/tests；
3. Browser/Computer/Process 变更运行 Windows native regression；
4. 更新两平台 evidence；
5. 两平台均 verified 才关闭 task。

## 12. 状态纪律

- 不用“基本完成”“应该可用”作为状态；
- `integrated` 只表示代码已合并，不表示平台验证完成；
- `verified` 必须附命令或原生证据；
- `complete` 必须满足 delete set 和全部 required platform gates；
- Windows-only 结果永远不能把 `macos_required` 任务标为 complete。
