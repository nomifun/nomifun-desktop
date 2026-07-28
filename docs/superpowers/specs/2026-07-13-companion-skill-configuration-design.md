# 桌面伙伴 Skill 配置设计

## 目标

让用户能为每个桌面伙伴单独配置全局 Skill，同时保留伙伴通过挖掘、反思和示范教学获得的自进化专精。配置在应用重启后保留，并在下一条伙伴消息生效。

P0 验收范围：

1. 可为单个伙伴启用 opt-in Skill。
2. 可为单个伙伴关闭 auto-inject Skill。
3. 不同伙伴的配置互不影响。
4. 保存后同步已有伙伴会话，无需重建伙伴。
5. Skill 卸载后保留配置，重新安装后自动恢复。
6. 伙伴专精的审核、编辑、赠送和示范教学链路不变。
7. Skill 配置只授予能力可见性，不扩大工具、文件、浏览器或远程渠道权限。

## 方案比较

### 方案 A：profile 保存授权意图，会话保存派生快照（采用）

profile 只保存用户显式启用的 opt-in Skill 和显式关闭的 auto-inject Skill。后端计算有效集合，将它写入会话 `extra.skills`，并在伙伴固定 workspace 中建立受管 Skill 链接或副本。

优点：保持全局 Skill 为唯一事实源；每个伙伴只保存差异；可复用 Nomi 原生 `SkillTool`；Skill 卸载和重装能自然恢复。代价是配置变更时需要同时协调 profile、会话快照、workspace 和 agent recycle。

### 方案 B：把 Skill 文件复制到伙伴目录

每次配置时复制完整 Skill 文件。

优点是运行时独立；缺点是产生多份事实源，更新、卸载、安全修复和导入导出容易漂移，因此不采用。

### 方案 C：把伙伴专精合并进全局 Skill 库

把自进化 Skill 与用户安装的通用 Skill 统一存储和调用。

优点是表面模型简单；缺点是混淆“用户授予”和“伙伴学习”两种语义，也会破坏现有 `companion_skill` 审核与归属链路，因此不采用。

## 产品模型

伙伴「技能」页分成两个区块：

- 已配置能力：展示全局 Skill，区分“默认能力”和“额外能力”，支持搜索和开关。
- 伙伴专精：保留现有草稿、启用、归档、编辑、赠送和示范教学功能。

auto-inject Skill 默认开启，用户可按伙伴关闭；opt-in Skill 默认关闭，用户可按伙伴启用。已配置但已卸载的 Skill 显示“未安装”，名称继续保存在 profile 中。

保存成功后提示“将在下一条消息生效”。首版不包含依赖诊断、同名冲突处理、配置模板或导入导出。

## 数据模型

后端：

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompanionSkillConfig {
    pub enabled: Vec<String>,
    pub disabled_auto: Vec<String>,
}
```

`CompanionProfileConfig` 增加 `skills: CompanionSkillConfig`。旧 profile 通过 `serde(default)` 得到空配置。

前端增加对应的 `ICompanionSkillConfig`。profile 只保存用户意图，不保存 Skill 文件副本。有效集合由后端计算：

```text
effective = (auto_inject - disabled_auto) ∪ enabled
```

结果去空、去重并按名称排序。未安装的 `enabled` 名称保留在 profile，但不进入当前有效集合。

## 后端数据流

### 新建或恢复伙伴会话

伙伴线程服务读取 profile，计算有效集合，并用瞬态字段创建会话：

```json
{
  "preset_enabled_skills": ["mermaid"],
  "exclude_auto_inject_skills": ["cron"],
  "workspace": ".../companion/workspaces/..."
}
```

`ConversationService` 生成规范化的 `extra.skills`，且不持久化这些瞬态字段。伙伴线程随后同步固定 workspace，补齐链接并校验会话快照。

### 修改配置

`PATCH /api/companion/companions/{id}` 保存 profile 后：

1. 找到该伙伴的现有单会话。
2. 重新计算有效 Skill。
3. 同步 `{workspace}/.nomi/skills`。
4. 通过内部方法替换 `conversation.extra.skills`。
5. 仅当快照变化时终止缓存 agent，使下一条消息重新 bootstrap。

公共会话 PATCH 继续禁止直接修改 `extra.skills`。

## workspace 受管语义

同步器只管理自己创建的 Skill 项，不能删除用户手工放入 `.nomi/skills` 的内容。

manifest 保存 Skill 名称、源路径和创建形态。清理旧项之前必须确认当前目标仍与 manifest 对应：

- 符号链接或 junction 必须仍指向记录的源路径。
- copy fallback 必须带有可验证的受管标记；如果无法证明归属，就保留目标并只移除 manifest 记录。
- 同名用户项已存在时不覆盖，也不登记为受管。
- manifest 中的非法名称或越界路径必须忽略。

同步失败采用 best-effort：记录告警、保留 profile 意图；后续打开线程或再次修改配置时自动重试。会话快照只包含当前已解析的 Skill，避免模型看到不可调用能力。

## 前端行为

前端并行读取全局 Skill 列表和 auto-inject 列表，根据 profile 差异计算开关状态：

- auto-inject：`checked = !disabled_auto.includes(name)`。
- opt-in：`checked = enabled.includes(name)`。

开关保存期间禁用其他开关，避免并发 PATCH 造成丢失更新。缺失的已配置 Skill 作为只存在于 profile 的条目展示，可关闭；重新安装后恢复正常条目和有效状态。

## 错误处理与兼容

- profile PATCH 无效类型时返回 `BadRequest`。
- 会话不存在时只保存 profile，新建会话时应用配置。
- workspace 链接失败不回滚 profile；下次 reconcile 重试。
- 会话 `extra` 损坏时内部更新方法以空对象修复，不能 panic。
- agent recycle 失败记录告警，但已保存的 profile 和快照保留。
- 默认空配置继续启用所有 auto-inject Skill，旧伙伴行为不倒退。

## 测试策略

后端覆盖：

- 旧 profile 默认值与持久化往返。
- 有效集合的排除、启用、去重和缺失 Skill 行为。
- 受管项清理不会删除被用户替换的同名目录。
- 会话快照变化时更新并 recycle；相同集合不更新、不 recycle。
- 非对象或损坏的 `extra` 能被安全修复。

前端覆盖：

- 已配置能力与伙伴专精两个区块存在。
- auto-inject 和 opt-in 开关写入正确字段。
- 缺失 Skill 可见且可关闭。
- TypeScript、i18n key 和生产构建通过。
