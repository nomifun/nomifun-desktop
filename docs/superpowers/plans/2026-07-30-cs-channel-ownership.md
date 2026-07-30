# 客服渠道绑定自闭环（渠道所有权分域）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修正设计缺陷：客服的渠道 bot 与桌面伙伴的渠道 bot 彻底分域——客服在自己页面创建/管理/绑定自己的 bot（自闭环），两域互斥，不再共享挑选池。

**Architecture:** 渠道运行时（14 个 IM 插件、QR 配对、watchdog、消息循环）保持单一基建不动；`channel_plugins` 加 `owner_domain` 列（'companion'|'customer_service'）做所有权分域，DB 触发器 + 应用层双重互斥；客服详情页内建 bot 创建/管理（复用各平台配置表单组件），伙伴侧 UI 过滤只见伙伴域。

**Tech Stack:** Rust（nomifun-db 迁移 020（与远程 016 合并重排前为 019）、nomifun-channel、nomifun-customer-service）、React/Arco。

## Global Constraints

- 构建前 PATH：`export PATH="/c/Users/MINISFORUM/.cargo/bin:/c/Program Files/CMake/bin:/c/tools/nasm-2.16.03:$PATH"`。
- 迁移号**固定 `019_channel_owner_domain.sql`**（016-018 已被占用，014/015 是远程 model-invoke 的）。
- 渠道插件运行时代码（plugins/ 目录、message_loop 收发机制）不动；只动所有权/校验/查询面。
- 每任务一 commit（conventional commits + Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>）。
- 新 i18n 键 zh-CN/en-US 双语 + `bun run gen:i18n`；提交前 `bun run check:i18n`。

---

## File Map

| 文件 | 动作 | 职责 |
|---|---|---|
| `crates/backend/nomifun-db/migrations/019_channel_owner_domain.sql` | Create | 加列 + 互斥触发器 + 存量数据归位 |
| `crates/backend/nomifun-db/src/models/channel.rs` + `repository/{channel.rs,sqlite_channel.rs}` | Modify | `owner_domain` 字段贯穿行模型/DTO/仓储 |
| `crates/backend/nomifun-db/src/id_schema_contract.rs` | Modify | 契约收录新列与触发器 |
| `crates/backend/nomifun-channel/src/routes.rs` | Modify | 创建 bot 接受 `owner_domain`（默认 companion）；`companion_id` 绑定仅限 companion 域；status DTO 透出 owner_domain |
| `crates/backend/nomifun-customer-service/src/{service.rs,routes.rs}` | Modify | replace_bindings 校验目标 bot 必须为 customer_service 域（否则 400，不再跨域"抢") |
| `ui/src/common/adapter/ipcBridge.ts` | Modify | 渠道类型加 `owner_domain`；创建参数透传 |
| `ui/src/renderer/pages/customerService/CsAgentDetailPage.tsx`（+新子组件） | Modify/Create | 绑定区改为域内闭环：本域 bot 列表 + 页内创建入口（复用平台配置表单）+ 创建即自动绑定 |
| 伙伴/设置侧渠道 UI（探索定位：`RemoteConnectSection`、SettingsModal channels、requirements NotifyPanel） | Modify | 列表过滤 `owner_domain === 'companion'` |
| `docs/superpowers/specs/2026-07-29-companion-memory-summon-service-remote-design.md` | Modify | C 节补"渠道所有权分域"决策记录（2026-07-30 修正） |

**Interfaces：**
- 列：`channel_plugins.owner_domain TEXT NOT NULL DEFAULT 'companion' CHECK (owner_domain IN ('companion','customer_service'))`（ADD COLUMN 列级 CHECK 合法）。
- 跨列互斥（ADD COLUMN 不能加表级 CHECK）用触发器：`trg_channel_plugins_owner_domain_insert/update_guard`——`owner_domain='customer_service' AND companion_id IS NOT NULL` → RAISE ABORT。
- wire：`IChannelPluginStatus.owner_domain: 'companion' | 'customer_service'`；创建请求 `owner_domain?: string`（缺省 companion）。
- cs 校验错误文案：`channel bot {id} belongs to the companion domain; create a customer-service bot instead`。

---

### Task 1: 迁移 020（原 019） + 行模型/契约（TDD）

- [ ] Step 1：先读 `017_customer_service.sql` 里 channel_plugins 的重建段与 `id_schema_contract.rs` 中该表契约的表达方式；写 019：

```sql
ALTER TABLE channel_plugins ADD COLUMN owner_domain TEXT NOT NULL DEFAULT 'companion'
    CHECK (owner_domain IN ('companion','customer_service'));
-- 存量归位：已被客服绑定且未被伙伴占用的 bot 归客服域
UPDATE channel_plugins SET owner_domain = 'customer_service'
 WHERE companion_id IS NULL
   AND channel_plugin_id IN (SELECT channel_plugin_id FROM cs_channel_bindings);
-- 错误状态清理：同时挂两头的，保伙伴、清客服绑定
DELETE FROM cs_channel_bindings
 WHERE channel_plugin_id IN (
    SELECT channel_plugin_id FROM channel_plugins WHERE companion_id IS NOT NULL);
-- 互斥触发器（insert/update 两枚）：customer_service 域 bot 不得携带 companion_id
CREATE TRIGGER trg_channel_plugins_owner_domain_insert_guard
BEFORE INSERT ON channel_plugins
WHEN NEW.owner_domain = 'customer_service' AND NEW.companion_id IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'customer-service channel bots cannot carry a companion binding'); END;
CREATE TRIGGER trg_channel_plugins_owner_domain_update_guard
BEFORE UPDATE OF owner_domain, companion_id ON channel_plugins
WHEN NEW.owner_domain = 'customer_service' AND NEW.companion_id IS NOT NULL
BEGIN SELECT RAISE(ABORT, 'customer-service channel bots cannot carry a companion binding'); END;
```

- [ ] Step 2：行模型/DTO/仓储贯穿 `owner_domain`（编译错误驱动逐点补全）；契约测试收录列与触发器；仓储加 `list_plugins_by_owner_domain`（或现有 list 带过滤参数，择现状更贴合者）。
- [ ] Step 3：迁移测试：预置"被客服绑定的无主 bot"与"双挂 bot"走 019，断言归位与清理；触发器测试：给 cs 域 bot 设 companion_id 被 ABORT。`cargo test -p nomifun-db` 全绿 → commit。

### Task 2: 后端互斥校验（渠道 + 客服两侧，TDD）

- [ ] Step 1：`nomifun-channel/routes.rs` 创建路径接受 `owner_domain`（校验枚举值）；`companion_id` 绑定写入处（:283 一带）校验目标 bot `owner_domain=='companion'`，否则 400。status DTO 透出 owner_domain。
- [ ] Step 2：`nomifun-customer-service` 的 replace_bindings 路由层校验：每个 plugin id 必须存在且 `owner_domain=='customer_service'`，否则 400（文案见 Interfaces）；service doc 注释中删除"同 bot 重绑替换（steal）"的跨域含义（同域内 re-bind 仍允许）。
- [ ] Step 3：集成测试：companion 域 bot 绑到客服 → 400；cs 域 bot 设 companion_id → 400/ABORT；cs 域 bot 正常绑定/换绑成功。两 crate 测试全绿 → commit。

### Task 3: 前端——客服渠道自闭环

- [ ] Step 1：探索复用面：读 `ui/src/renderer/components/channels/PlatformConfigBody.tsx` 与 SettingsModal contents/channels 各平台表单的 props 契约，确认能否独立嵌入（目标：客服页内弹窗创建 bot）。若耦合过深，退而求其次：客服页"新建渠道 bot"按钮跳转/打开既有渠道创建面，但创建参数带 `owner_domain='customer_service'` 并在成功回调里自动绑定——探索后择优，写进 commit message。
- [ ] Step 2：`CsAgentDetailPage` 绑定区改造：列表只显示 `owner_domain==='customer_service'` 的 bot（含绑定态：绑本客服/绑其他客服/未绑定）；"新建渠道 bot"入口（Step 1 选型）；创建成功自动 `replaceBindings` 纳入本客服。空态文案引导（"客服使用自己的渠道机器人，与桌面伙伴的渠道相互独立"）。
- [ ] Step 3：伙伴/设置侧过滤：定位所有消费 `getPluginStatus`/渠道列表的伙伴域 UI（RemoteConnectSection、SettingsModal channels 列表、requirements NotifyPanel 等），加 `owner_domain==='companion'` 过滤（后端字段已透出）。i18n 双语 + gen:i18n。
- [ ] Step 4：结构/纯函数测试按既有 bun test 风格；`bun test` + `check:i18n` + `bun run typecheck` 全绿 → commit。

### Task 4: spec 决策记录 + 回归

- [ ] Step 1：spec C 节补一段（2026-07-30 修正）：渠道所有权分域设计与动因（客服自闭环、互斥、不迁移共享池语义）。
- [ ] Step 2：`cargo test -p nomifun-db -p nomifun-channel -p nomifun-customer-service` + `bun test` + `cargo check --workspace --exclude nomifun-desktop` 全绿 → commit。
