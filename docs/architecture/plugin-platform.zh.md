# 插件与小程序架构

Plugin 是本机应用单元：标准包、稳定实例身份、独立持久数据和可选 UI / Service 共用一套安装与运行链路。产品入口、创作与管理见[插件与小程序](../specs/2026-10-02-agent-plugin-authoring-redesign/README.zh.md)；Agent 会话事实与恢复见[Agent Session 架构](agent-session.zh.md)。本页记录现役合同，已完成的重构计划与旧设计通过 Git 历史查询。

执行合同以 [Rust 作者合同](../../crates/backend/nomifun-agent-contracts/src/plugin.rs)、[Plugin Platform](../../crates/backend/nomifun-plugin-platform/src/lib.rs) 和[应用 Router](../../crates/backend/nomifun-app/src/router/plugin.rs) 为准。支持 Tauri Desktop 与桌面级 WebUI，最低视口为 880×600。

## 身份与所有权

| 身份 | 含义 |
| --- | --- |
| `draft_id` | 受管源码工作副本 |
| `plugin_id` | 本机实例、配置、授权和数据的稳定所有者 |
| `artifact_digest` | Host 计算的内容寻址包摘要 |
| `package_id` | 包作者声明的逻辑名称，用于识别安装与更新目标 |

UI-only、Service-only 和混合包使用同一种 Plugin 模型。包内 Action 对外的稳定身份为 `plugin:<plugin_id>/<action_id>`。内容更新不改变实例身份；创建独立副本使用新的实例和本地 package identity，数据与原作品独立。

Artifact 是不可变文件内容，当前 `active_artifact_digest` 和 `data_generation` 由核心数据库决定。修订号用于 CAS 并发校验，不是用户需要维护的版本管理。作品没有上一版本指针或历史版本恢复入口；回收站恢复操作恢复同一作品与数据。

## 标准包与 Manifest

```text
nomifun.plugin.json
ui/index.html              # 可选
ui/assets/**
service/main.mjs           # 可选
migrations/*.mjs           # dataVersion 变化时需要
source/**                  # 可选，供继续编辑
```

`ui/index.html` 与 `service/main.mjs` 至少存在一个。UI-only 作品不启动 Node 进程；有 Service 的作品通过独立进程实现后台 Action。作者在外部打包依赖，提供浏览器静态文件和 ESM Service；Host 不执行 npm 生命周期脚本或通用项目构建。

混合包的 Manifest 示例：

```json
{
  "schema": "nomifun.plugin/v1",
  "id": "local.todo",
  "version": "1.0.0",
  "name": "待办事项",
  "description": "一个带 Agent 工具的待办应用",
  "hostApi": ">=1 <2",
  "entrypoints": {
    "ui": "ui/index.html",
    "service": "service/main.mjs",
    "serviceMode": "onDemand"
  },
  "actions": {
    "add_task": {
      "name": "添加待办",
      "description": "添加一条待办事项",
      "input": {
        "type": "object",
        "properties": { "title": { "type": "string" } },
        "required": ["title"]
      },
      "output": { "type": "object" },
      "effect": "write"
    }
  },
  "bindings": [{ "point": "agent.tool", "action": "add_task" }],
  "dataVersion": 0,
  "migrations": [],
  "configSchema": { "type": "object" },
  "secrets": [],
  "permissions": []
}
```

Manifest、Action、Binding 和入口字段接受明确合同，未知字段拒绝；第三方扩展放入命名空间化的 `extensions`。Action 输入与输出直接使用内联 JSON Schema，Host 在调用前后验证。Action ID 以小写字母开头，只含小写字母、数字、`_` 或 `-`，最长 96 字符；`effect` 为 `read`、`write` 或 `external`。

Binding 引用本包 Action。必需 Binding Point 在当前 Host 不受支持时拒绝启用；`optional: true` 可以声明可选绑定。Host 计算 Artifact、文件及 migration 摘要，作者无需生成内部 schema URI 或摘要。

## Action 与 Binding

现役 Binding Point：

| Binding Point | 消费者 |
| --- | --- |
| `agent.tool` | Agent 工具 |
| `agent.context` | Agent 回合上下文 |
| `agent.before_model` | 模型请求前处理 |
| `agent.before_tool` | 工具调用前处理 |
| `desktop.command` | 桌面命令 |
| `desktop.event` | 桌面事件 |
| `automation.action` | 通用 Action registry；当前未接入调度器 |

各消费者注册输入、输出、调用顺序及失败语义，Plugin Core 保持实例、Action 和包的单一身份。Agent 消费者冻结准确的 Action、Artifact 与工具映射，执行时复核当前启用状态和摘要。已删除或变化的 Action 明确返回 unavailable / artifact-changed，不静默改绑。

Plugin 间调用使用 `ctx.actions.invoke("plugin:<plugin_id>/<action_id>", input)`，继续经过授权、启用状态、Artifact、取消、超时和调用链检查。SDK 不提供另一 Plugin 的 DataRoot 或 Credential。`host.invoke` 只调用 Host 实际提供的能力；当前 `desktop.files.open` 没有生产文件打开实现，不能据此生成可用功能。

## Service Runtime

NomiFun 管理唯一 JS Runtime authority。Manifest 声明 Host API 兼容范围，不选择 Node 路径或 Runtime 实例。每个有 Service 的 Plugin 最多一个独立进程；一个作品停止、崩溃或更新不重启其他作品。

`onDemand` 在首次调用时启动，`continuous` 在启用和应用启动恢复时启动。进程 PID、Runtime generation 和健康状态是运行观察；重启从持久化的 enabled、Active Artifact 与 DataRoot 重建。

Service 导出：

```js
export async function activate(ctx) {
  return {
    async invoke(action, input) {
      // 实现 Manifest 中声明的 Action，并返回符合 output Schema 的结果。
    },
    async deactivate() {
      // 可选清理。
    }
  };
}
```

`ctx` 包含 `pluginId`、`artifactDigest`、`signal`、`storage`、`cache`、`config`、`secrets`、`host` 和 `actions`。Host 通过 IPC 提供受限操作，保留真实取消、调用超时、队列限制和进程树退出证明。

Service 是用户选择运行的本机 JS 代码。Manifest permissions 控制 Host Bridge 与 UI 网络能力，不构成 Node 原生文件、网络或子进程沙箱保证。

## UI Surface 与 SDK

UI 在仅允许 scripts 的 sandboxed iframe 内运行。Host 自动注入 `window.nomi`，经版本化 MessageChannel 连接同一 Bridge；作者不添加 SDK script。Surface session、generation、Artifact digest 和调用身份共同校验每次操作，关闭或替换后迟到响应不能写入新页面。

```text
window.nomi.storage.kv.get/set/delete/compareAndSwap
window.nomi.storage.db.query/execute/batch
window.nomi.storage.files.read/write/list/delete
window.nomi.cache.get/set/delete
window.nomi.actions.invoke
window.nomi.host.invoke
window.nomi.config.get
```

UI 与 Service 共用 Storage、Cache 和 Action 的语义，UI-only 作品可直接使用 Host 管理的存储。`storage.kv.get` 返回保存的 JSON 值，缺失时返回 `null`；`set` 返回 `{revision}`，`compareAndSwap` 返回 `{applied, revision}`。Service 的 `secrets.get(slot)` 按已绑定的 Host Credential 引用取值，UI 不提供秘密读取 API。

CSP 禁止任意外部脚本、worker 和表单提交；声明并授予 `network` 时才允许 UI 的网络连接。iframe 的 localStorage、sessionStorage、indexedDB 和 cookies 不可用，alert / confirm / prompt 被阻止。作者使用实际控件事件及 awaited SDK 操作，读写失败显示真实错误；初始化和共享状态修改保持串行，持久化成功后再提交 UI 状态。

## DataRoot、配置与 Credential

```text
plugin-data/<plugin-id>/
  generations/<generation-id>/
    data.sqlite
    files/
  staging/
```

`plugins.data_generation` 指向当前数据。`data.sqlite` 同时承载作品自定义表、Host KV 表 `_nomifun_kv` 和迁移记录 `_nomifun_migrations`。KV、数据库与文件均持久化；Cache 是可设置 TTL 的内存数据，不进入备份。

SQLite authorizer 与路径检查把操作限制在本作品 DataRoot，禁止 ATTACH 核心数据库、跨作品路径及修改 Host 保留的 `_nomifun_*` 表。作品可在自有表中使用 SQLite DDL / DML，查询使用参数，调用与结果大小受 Host 限制。

Config 存入 `plugins.config_json`，使用当前 Manifest 的 `configSchema` 校验。Credential 绑定只保存 `slot -> Host Credential ID` 引用；Manifest 声明秘密槽位，引用必须来自可用的已有 Provider 或 Connection。Host 的 Credential 管理响应只返回引用和元数据，Package 与 Backup 不导出 Host Credential 明文；作者应通过 `secrets.get` 使用秘密，避免将其写入普通配置、文件、日志或 Action 输出。

安装直接采用 Manifest 声明的权限。更新保留用户对已有权限的显式开关，新声明权限默认授予；设置界面可以收窄权限。实际使用秘密槽位时绑定可用的 Credential 引用，缺少必需配置时向用户取得信息。安装与权限扩张没有独立审批对象或待审批阶段。

## Preview 与创作验证

新作品使用空 Preview DataRoot，编辑已有作品时克隆当前数据。Preview 使用正式运行的 SDK、Bridge、Service Host 和 Storage adapter，换成临时 DataRoot 与当前预览配置；预览写入不合并回正式数据。

省略预览 `access` 时使用 Manifest 声明的权限，已有作品继承仍被 Manifest `secrets` 声明的 Credential 引用；新作品没有默认 Credential 绑定。显式预览权限必须是声明权限的子集。权限仍受真实 Host 能力与引用可用性检查，声明能力不代表 Host 已提供实现。

同一草稿预览可复用临时数据以支持重新加载。自动 `test_ui` 的每个 case 重新建立空数据或正式数据副本，防止前一用例污染后一用例；它也重置 Service 临时存储。`test_action` 按调用顺序共享当前预览数据，因此混合包先执行 Service 用例，再执行 UI 用例。持久化验证通过真实 Service restart 或 UI reopen 后的结果断言完成。

创作使用 canonical PluginAuthoring Session 和原 Nomi Runtime。计划记录交付产物、功能与准确用例，修复不能弱化已接受的计划。全部计划 case 与需要的持久化证据通过后才安装准确修订；诊断探针不增加交付要求。安装关闭预览并检查正式作品，安装后不再调用预览测试工具。

当前会话内使用新工具的要求通过 `current_conversation_case` 保存准确 Action、输入和预期结果。结算从同一 canonical Turn 的真实调用和 owner settlement 证明使用，不从模型自述、预览结果或 Message projection 推断。

## 安装、迁移与恢复

草稿保存、目录、ZIP 与 Backup 导入共用 `PluginInstallService` 的安装链路。修改按 Plugin mutex 串行，保存与更新复核确切 revision：

1. 冻结标准包，验证路径、Manifest、Schema、入口与 Binding，计算并保存不可变 Artifact。
2. 为新作品建立空 staging 数据；更新克隆当前数据；Backup 导入自己的数据快照。
3. 执行所需 migration，并在 staging 数据上通过同一种 Service Host 验证启动；UI 文件完成静态检查。
4. 停止旧 Service 并撤销旧运行准入，在核心数据库事务中提交 Active Artifact、数据指针、Config、Credential 引用、权限与 CAS revision。
5. 按作品 enabled 状态恢复运行和 Binding；激活失败恢复实际变更前的状态，mutation journal 为崩溃后的恢复提供证据。

同 `dataVersion` 的代码更新在临时副本上验证，正式运行继续使用当前数据 generation；验证副本的写入不进入正式数据。数据版本升级在副本上迁移并提交完整新 generation，失败保留原数据。

Manifest migration 为 `{id, from, to, path}`，版本链逐级覆盖 `N -> N+1`，模块导出 `migrate(ctx)`，可迁移 SQLite 与 Files。Host 在 `_nomifun_migrations` 记录准确 ID、版本与文件摘要；已记录的 migration 被改写时拒绝。首次安装从版本 0 执行到目标版本，Backup 保留已有数据迁移记录。

失败回滚及 journal 中的旧状态只服务于原子操作恢复，不提供历史版本产品。停用、关闭页面和正常重启保留作品数据；移入回收站撤销运行访问但保留作品，恢复重新使用同一作品。永久删除要求作品已在回收站，并删除其数据与关联配置。

## 导入与导出

| 格式 | 内容 |
| --- | --- |
| Plugin Package | 当前标准包、资源和可选 source，不含用户数据 |
| Plugin Backup | 当前 Package、当前 DataRoot、非秘密 Config、权限元数据与需重新绑定的秘密槽位 |

目录和 ZIP 进入同一内容与路径校验；拒绝越界路径、符号链接、重复或 Windows 冲突路径以及超限内容。导入现有包默认更新同身份作品并保留未显式覆盖的配置与 Credential 绑定；创建副本保持独立身份和数据。

Backup 始终恢复为新的本地 Plugin，不覆盖已有作品；其 Config 与数据为备份中的实际内容。导入后为秘密槽位选择当前可用的 Host Credential 引用，不导入明文或旧账号权限。导出排除 Preview、staging、Cache 和其他数据 generation。

## 持久化与接口入口

| 表 | 用途 |
| --- | --- |
| `plugins` | 本地实例、当前 Artifact / 数据指针、配置、启用与回收状态 |
| `plugin_artifacts` | 不可变包元数据与文件根 |
| `plugin_drafts` | 工作目录、作品关联、canonical 来源与真实验证报告 |
| `plugin_credential_bindings` | 秘密槽位到 Host Credential 引用 |
| `plugin_grants` | 声明权限的实际开关 |
| `plugin_library_state` | 常用、分类、别名和最近打开等作品组织数据 |
| `plugin_mutations` | 安装、更新和永久删除的短期 crash journal |

作品业务数据位于各自 DataRoot。进程、Surface 和 Binding 索引从当前持久记录重建；测试与交付证据保存在已有草稿报告，不增加独立任务或发布账本。

作品与草稿接口位于 `/api/plugins`、`/api/plugin-drafts`，工作台 Session 入口为 `/api/plugins/authoring/sessions`。自然语言创作通过 canonical `/api/agent-sessions/{id}/turns`；源码修改通过草稿 `/files`，预览与保存使用 `/preview`、`/save`。作品的 `/restore` 只恢复回收站内容，导出源码与备份分别使用 `/export`、`/backup`。具体 DTO 与方法以 [Router](../../crates/backend/nomifun-app/src/router/plugin.rs) 和 [API 类型](../../crates/backend/nomifun-api-types/src/plugin_platform.rs) 为准。

修改时运行受影响的 SDK、存储、安装、消费者或 UI 测试及 `bun run check:unified-plugin-boundary`。涉及 canonical Session 内容时同时遵循 Agent Session 边界；修改 renderer 时运行 Desktop UI Boundary。
