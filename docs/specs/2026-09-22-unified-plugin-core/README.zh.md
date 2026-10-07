# Unified Plugin Core：最终架构与一次性重构合同

> 日期：2026-09-22
> 状态：产品方向已确认，等待一次性实施
> 适用目标：Tauri Desktop 与桌面级 WebUI；最小视口继续遵守仓库的 880×600 合同
> 数据策略：Plugin 子系统 clean cut；不保留 N1/M1 兼容层，不迁移实验期 Plugin 数据
> 本文地位：Plugin 重构的最终产品合同、目标架构和完成门禁。实施过程中不得重新扩回双聚合，也不得以未删除旧实现宣称完成。

## 1. 恒定产品目标

以下四项是本次重构的最高约束，优先级高于现有 N1/M1 合同、历史实施计划和过渡代码：

1. 用户能够简单创建运行在 JS Runtime 上的插件：
   - 带 UI 的可视化 App；
   - 无 UI 的系统能力插件，可增强 Agent、Desktop 及其他已注册扩展点；
   - 同一插件允许同时拥有 UI、后台逻辑与系统能力。
2. Plugin 是完整应用单元，每个本地 Plugin 实例拥有自己的持久化数据库、持久 KV、Files、临时 Cache、配置与 Credential 引用。
3. NomiFun 内可通过 Chat 一键生成、预览、保存并运行；外部作者也可打包目录或 ZIP 后直接导入。两种来源必须进入同一条安装与运行链路。
4. 架构保持简单、灵活、开放、强大：
   - 少做没有真实需求的限制；
   - 不把内部治理概念暴露给作者和普通用户；
   - 不因“未来可能扩展”预建第二套身份、状态机、测试平台或发布平台；
   - 开放性来自小而稳定的包格式、SDK、Action 与 Binding，而不是无限增加内部可替换层。

## 2. 最终决策摘要

最终系统只有一套 Unified Plugin Core：

- 一种 Plugin Package；
- 一种 Plugin 本地实例；
- 一个 Host 管理的 JS Runtime authority；
- 每个有 Service 的 Plugin 一个独立进程；
- 一套 UI/Service 语义一致的 Plugin SDK；
- 一个稳定 Plugin DataRoot；
- 一条 `Draft 或 Import -> Stage -> Validate -> Activate` 链路；
- 一套由 Active Artifact 派生的 Action/Binding 索引；
- 一套 DTO、Repository、Application Service、Router、Bridge 和 UI 模型。

有 `ui/index.html` 的 Plugin 即 App；没有 UI、只有 Service 与 Binding 的 Plugin 即无 UI 能力插件。两者不是不同产品，也不允许再出现 MiniApp、安装型 Plugin、发布型 Plugin 等平行平台。

当前 N1 安装聚合与 M1 发布聚合不继续深层合并，而是一起退出，由新的单聚合替代；退休设计与实施记录只保留在 Git 历史，不再构成仓库内可执行入口。

## 3. 架构总览

```text
                      Chat 创作
                          │
外部源码 / 外部包 ────────┼────────► Draft 工作目录
                          │                │
                          └────────────────┤ 保存 / 导入
                                           ▼
                                  Stage 临时 Artifact
                                           │
                              临时 DataRoot 中校验与启动
                                           │
                                           ▼
                                    原子激活 Plugin
                          ┌────────────────┼────────────────┐
                          ▼                ▼                ▼
                    沙箱 UI iframe     JS Service       Action Bindings
                          │                │                │
                          └──────── Unified Plugin SDK ─────┘
                                           │
                                           ▼
                             Plugin DataRoot / Host API
```

系统只有三个持久身份：

| 身份 | 含义 |
|---|---|
| `draft_id` | 可选的 Chat/源码创作草稿 |
| `plugin_id` | 本机 Plugin 实例、配置、授权和数据的稳定所有者 |
| `artifact_digest` | 内容寻址的不可变运行版本；同时作为版本事实，不再叠加 Artifact ID、Release ID、Candidate ID |

`package_id` 是包作者声明的逻辑名称与更新关联键，不是额外的本地生命周期聚合。默认同一 owner 下只安装一个相同 `package_id`；显式“创建副本”时生成新的 `plugin_id` 与新的本地 package identity。

## 4. 统一 Plugin Package

### 4.1 目录结构

```text
nomifun.plugin.json
ui/
  index.html                  # 可选
  assets/**
service/
  main.mjs                    # 可选
migrations/
  *.mjs                       # dataVersion 变化时可选
source/**                     # 可选；供继续编辑，Runtime 不读取
```

`ui/index.html` 与 `service/main.mjs` 至少存在一个：

- 只有 UI：零 Node 进程的可视化 App；
- 只有 Service：无 UI 能力插件；
- UI + Service：完整 App，UI、后台逻辑和系统能力共享同一个 Plugin 与 DataRoot。

### 4.2 Manifest v1

最终只保留一个作者合同，建议形态如下：

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
        "properties": {
          "title": { "type": "string" }
        },
        "required": ["title"]
      },
      "output": { "type": "object" },
      "effect": "write"
    }
  },
  "bindings": [
    { "point": "agent.tool", "action": "add_task" }
  ],
  "dataVersion": 1,
  "migrations": [],
  "configSchema": { "type": "object" },
  "secrets": ["api_key"],
  "permissions": ["network"]
}
```

合同规则：

- Manifest 使用一个明确 schema 版本；核心字段未知时拒绝，第三方扩展放入命名空间化的 `extensions` 字段，避免顶层字段漂移。
- Action 的输入与输出直接使用内联 JSON Schema，不要求作者生成 canonical schema URI、schema registry 或多层 digest。
- Binding 必须引用本包 Action；必需 Binding Point 不受当前 Host 支持时禁止启用，可选 Binding 可明确标为 optional。
- Artifact digest、文件 digest 和 migration digest 全部由 Host 计算，不要求用户手工输入。
- `source/**` 可以随包携带，但不参与 Runtime；没有源码的预构建包同样是一等公民。
- 外部作者使用任意工具打包依赖，最终输出浏览器静态文件和单个 ESM `service/main.mjs`。NomiFun 不承担通用 npm registry、生命周期脚本和任意项目构建系统。
- Chat Creator 直接生成这一格式，不再先生成中间 DTO 再翻译成另一种包。

## 5. Action + Binding：唯一开放能力模型

### 5.1 Action

Action 是 Plugin 提供的稳定可调用方法，包含：

- 本地 Action ID；
- 名称和说明；
- 输入/输出 JSON Schema；
- 副作用说明；
- 由 `service/main.mjs` 实现的 handler。

Host 对外形成稳定身份 `plugin:<plugin_id>/<action_id>`。Agent 或 Desktop 配置引用稳定 Action 身份，不引用 Artifact digest；更新后由当前 Active Artifact 继续实现该 Action。若新版本删除 Action，消费端显示 unavailable，而不是静默改绑。

### 5.2 Binding

Binding 只描述 Host 在何处使用某个 Action。首个稳定集合为：

```text
agent.tool
agent.context
agent.before_model
agent.before_tool
desktop.command
desktop.event
automation.action
```

Plugin Core 不硬编码各消费者的复杂领域模型。每个 Binding Point 由其拥有者注册：

- 输入/输出合同；
- 是否允许多个 Provider；
- 调用时机与顺序；
- 用户选择或授权方式；
- 失败语义。

新增 Agent/Desktop 扩展点只增加一个 Binding adapter，不修改 Plugin 身份、表结构、生命周期或包安装状态机。

现有统一 Agent Module/Action 架构仍是 Agent 内部权威。Plugin Action/Binding 在消费者边界物化为该架构理解的 Action，不要求删除 Agent 核心仍在使用的全局 Package/Module 合同；只删除 Plugin 作者侧对 `PackageContributions`、Role、Provider、Consumer surface 等复杂内部结构的直接依赖。

### 5.3 Plugin 之间与 Host 能力调用

Service SDK 提供：

```js
ctx.host.invoke("desktop.files.open", input)
ctx.actions.invoke("plugin:<plugin-id>/<action-id>", input)
```

Host API 通过 Grant 控制；Plugin 之间的 Action 调用继续经过启用状态、Active Artifact、取消、超时和调用链检查，不能绕过 Host 直接取得别的 Plugin DataRoot 或 Credential。

## 6. 唯一 JS Runtime 与进程模型

- NomiFun 启动时确认一个 JS Runtime authority；Plugin Manifest 只声明 Host API 版本，不选择 Node 路径、Node 主版本或 Runtime 实例。
- UI-only Plugin 不启动 Node。
- 每个含 Service 的 Plugin 最多一个独立进程，不再使用 Shared Extension Host。
- `onDemand` 在首次调用时启动并可在空闲后停止。
- `continuous` 在 Plugin 启用及应用启动恢复时启动。
- 一个 Plugin 崩溃、更新或停止不重启其他 Plugin。
- Runtime generation、PID、Surface session 和健康观察都是内存运行事实，不进入长期领域状态；重启后从 `enabled + active_artifact` 重建。

统一 Service 合同：

```js
export async function activate(ctx) {
  return {
    async invoke(action, input) {
      // 实现 Manifest 中声明的 Action
    },
    async deactivate() {
      // 可选资源清理
    }
  };
}
```

`ctx` 至少包含：

```text
pluginId
artifactDigest
signal
storage
cache
config
secrets
host
actions
```

现有独立 Service process supervisor 的 NDJSON、取消、超时、队列限制、进程树清理和 storage IPC 可以保留；现有 Runtime 选择、Candidate Test Host、Shared Extension Host、Mount generation 和对应切换证明退出。

## 7. UI Surface 与统一 SDK

- UI 在 sandboxed iframe 中运行，通过版本化 MessageChannel 与 Host 通信。
- UI 与 Service 使用同一组语义合同，不再有“预览假 API / 正式另一套 API”。
- UI 可在没有 Service 的情况下直接使用 Host 管理的 KV、Database、Files、Cache、Config 和获授权 Host API。
- UI 调用 Plugin Action 时统一经过 Action dispatcher；存在 Service 时由 Service 实现。
- 一个 Plugin 只有一个 App Surface；复杂页面通过 App 内部路由实现，不建设 Slot/Shell 替换系统。

浏览器侧建议 API：

```text
window.nomi.storage.kv.*
window.nomi.storage.db.*
window.nomi.storage.files.*
window.nomi.cache.*
window.nomi.actions.invoke(...)
window.nomi.host.invoke(...)
window.nomi.config.get()
```

现有 MessageChannel 握手、防重复调用、Active Artifact fence 和 iframe sandbox 可以保留并收敛为唯一 Bridge。

## 8. Plugin DataRoot

### 8.1 数据布局

每个 Plugin 实例拥有 generation 化 DataRoot：

```text
plugin-data/<plugin-id>/
  generations/
    <generation-id>/
      data.sqlite
      files/
  staging/
```

`plugins.data_generation` 是当前权威 generation。普通写入修改当前 generation；只有 `dataVersion` 变化时才复制并切换 generation。

`data.sqlite` 同时承载：

- Plugin 自定义表；
- Host 管理的 `_nomifun_kv`；
- Host 管理的 `_nomifun_migrations`。

Cache 是非持久、可设置 TTL 的内存能力，不进入 Backup。

### 8.2 统一 Storage API

UI 和 Service 共享：

```text
storage.kv.get/set/delete/compareAndSwap
storage.db.query/execute/batch
storage.files.read/write/list/delete
cache.get/set/delete
secrets.get(slot)
```

必要边界只有：

- SQLite 与 Files 限定在本 Plugin DataRoot；
- 禁止 ATTACH 核心数据库及跨 Plugin 路径；
- `_nomifun_*` 表名保留给 Host；
- 查询参数化，保留取消、超时和结果大小保护；
- Credential 明文不进入 Config、日志、Package 或 Backup。

Plugin 在自己的数据库中可自由使用 SQLite DDL/DML，不再通过受限 JSON Migration AST 描述普通表结构。

### 8.3 配置与 Credential

- 非秘密 Config 直接存入 `plugins.config_json`，并由 Active Manifest 的 `configSchema` 验证。
- Credential 使用独立 `plugin_credential_bindings`，只保存 slot 到 Host Credential ID 的引用。
- 更新新增权限或秘密槽位时需要用户确认；普通代码更新不重复请求已有 Grant。
- Package 导出和 Backup 均不含 Credential 明文；Backup 只记录需要重新绑定的 slot。

## 9. Preview：临时数据会话，不是第二环境

系统不建立长期 Dev/Prod 双环境，只存在一个正式 DataRoot 与一个临时 Preview DataRoot：

- 新 Plugin 使用空 Preview DataRoot。
- 编辑已有 Plugin 时，从当前 data generation 创建临时副本。
- 同一次 Draft 预览会话的 UI 与 Service 共享这份临时数据，Chat 多轮修改不会因为 HTML reload 自动清空测试数据。
- Preview 使用与正式运行完全相同的 SDK、Bridge、Service Host 和 Storage adapter，只替换 DataRoot handle 与 Host grants。
- Preview 网络、Host capability 与秘密默认关闭；用户显式启用时界面准确说明边界。
- Preview 数据从不合并回正式 DataRoot；保存只发布代码和声明，并按 migration 合同转换正式数据。
- Draft 源码、Chat 消息和文件正常持久化；Preview DataRoot 在会话结束、Discard 或应用恢复清理时删除。

当前 iframe 沙箱实现可作为基础，但必须删除每份文档都创建全新内存 `Map` 的假存储实现，改用统一临时 DataRoot Bridge。

## 10. Migration 与原子激活

### 10.1 数据版本

- Manifest 声明整数 `dataVersion`。
- 从版本 `N` 到 `N+1` 的 migration 是包内 JS 模块，可通过统一 Storage API 同时迁移 SQLite 与 Files。
- Migration 文件属于 Artifact digest，Host 在 `_nomifun_migrations` 记录 ID 与 digest，重复或被改写的 migration 被拒绝。
- 初次安装从版本 0 在空 DataRoot 上执行到目标版本。

### 10.2 保存或导入算法

保存草稿与导入外部包必须调用同一个 `install_artifact` application service：

1. 冻结 Draft 或导入目录到 staging。
2. 规范化路径、校验 Manifest、Action、Binding、Schema、权限和入口文件。
3. 计算内容寻址 Artifact digest，并写入不可变 Artifact Store。
4. 若是首次安装，创建空 staging DataRoot；若 `dataVersion` 变化，从当前 generation 克隆到新 staging generation。
5. 在 staging DataRoot 执行 migration。
6. 使用 staging Artifact 与 staging DataRoot 启动同一种 JS Host，执行 module import、`activate` 和无业务副作用的 health check；UI 同时完成静态加载检查。
7. 取得 Plugin mutation mutex，停止旧 Service，撤销旧 Surface 和 Binding admission，并等待或取消有界的 in-flight 调用。
8. 在核心 SQLite 事务中同时更新：
   - `active_artifact_digest`；
   - `previous_artifact_digest`；
   - `data_generation` 与可选 `previous_data_generation`；
   - `revision`、Config/Grant 结果和 mutation journal。
9. 按原 `enabled` 状态启动新 Service、发布 Binding 索引并允许 Surface。
10. 启动失败则在旧 generation 尚保留时恢复旧指针与运行状态；成功后清理 staging 和完成 journal。

Artifact 与新 data generation 都在事务前完整生成，因此核心 DB 指针是权威提交点。崩溃恢复只需读取一个通用 `plugin_mutations` journal：事务前恢复旧指针并清理 orphan；事务后以新指针为准完成启动或执行有证据的回退。

### 10.3 Previous 与数据回退

- 同一 `dataVersion` 的更新：Previous 只切换代码，复用当前 data generation。
- `dataVersion` 变化：保留一个 previous data generation；显式完整回退会同时恢复代码与旧数据，并明确提示更新后新增数据会丢失。
- 只保留一代 Previous，不建设任意版本历史、分支或多版本升级矩阵。

## 11. 生命周期与用户动作

Plugin 长期持久状态只保留：

```text
enabled: boolean
trashed_at: timestamp | null
active_artifact_digest
previous_artifact_digest | null
data_generation
previous_data_generation | null
revision
config_json
last_error
```

运行状态从当前进程观察派生：

```text
stopped / starting / running / failed
```

最终产品动作：

```text
创建 / 导入
预览
保存
打开
启用 / 停用
配置与授权
恢复上一版本
导出 Package / 导出 Backup
移入回收站 / 永久删除
```

删除以下产品与后端概念：

```text
Build Candidate
Ready Candidate
Ready Release
Apply
Publish
Publish Authorization
Auto Apply
Auto Publish
Active Epoch
Pointer Revision
Mount Revision
Candidate Test Receipt
Service Test Receipt
Catalog Publication
```

构建、验证、测试只是一次 Save/Import 请求内部的 staging 步骤，不成为长期领域对象。首次保存默认启用；更新保持当前 enabled 值。

## 12. Chat Authoring 与外部导入

### 12.1 Chat

- Draft 是受管工作目录，不是中间生成 DTO。
- Chat Agent 直接读写标准 Manifest、UI、Service、Migration 与可选 Source。
- 每轮修改触发统一 Preview reload。
- 编辑已有 Plugin 时 Draft 关联 `plugin_id` 与打开时的 `revision`；保存使用单一 CAS 防止覆盖外部更新。
- 保存成功后 Draft 可保留为继续编辑的工作副本；Discard 只删除 Draft，不删除已安装 Plugin。

### 12.2 外部导入

只保留两种 Bundle：

1. **Plugin Package**：代码、资源和可选 Source，不含用户数据；
2. **Plugin Backup**：当前 Package、DataRoot、非秘密 Config、Grant 元数据和 Credential slot 清单，不含秘密值。

目录与 ZIP 使用同一检查器。导入时 Host 自动计算摘要并显示：

- 包身份与版本；
- UI/Service 形态；
- Action 与 Binding；
- Host 权限与网络声明；
- 是否包含可信本机 Service；
- Backup 是否包含用户数据。

删除 Share Bundle、Prebuilt Artifact Import、Whole-App Backup 三套平行状态机和要求用户输入 digest 的确认方式。

## 13. 信任与开放边界

- UI iframe 是受控 Surface，使用 CSP 和 Host Bridge。
- `service/main.mjs` 是用户选择运行的可信本机 JS 代码。当前目标不是第三方 Marketplace，不建设无法兑现的“完全不可信 Node 沙箱”。
- 外部 Service Package 初次安装及权限扩张时明确提示其本机代码能力；Chat 生成代码仍展示实际权限。
- Manifest permissions 对 Host API、Credential、UI 网络来源进行真实授权；不能把未强制执行的 Node 原生网络/文件/子进程访问伪装成沙箱保证。
- 如未来建设不可信 Marketplace，应作为独立产品决策引入真正可执行的 sandbox，不得反向污染当前本地 Plugin Core。

“少做限制”不表示允许跨 Plugin 数据访问、绕过 Credential 授权或篡改 NomiFun 核心数据库；这些是所有权边界，不是产品治理层。

## 14. 最终持久化 Schema

核心数据库只保留以下 Plugin 表：

| 表 | 作用 |
|---|---|
| `plugins` | 本地实例、package identity、Active/Previous、data generation、enabled、revision、Config、错误 |
| `plugin_artifacts` | 内容寻址的不可变 Package 元数据与文件根 |
| `plugin_drafts` | Chat、工作目录、生成状态、关联 plugin 与 CAS revision |
| `plugin_credential_bindings` | `plugin_id + slot -> credential_id` |
| `plugin_grants` | Host API、权限和用户确认结果 |
| `plugin_library_state` | 固定、集合、最近打开等纯产品组织状态 |
| `plugin_mutations` | 安装、更新、data generation 切换和永久删除的短期 crash journal |

Plugin 业务数据不进入核心数据库；它位于 Plugin DataRoot。Surface session、运行进程、Host generation、Catalog index、测试结果和 Ready 状态不持久化。

## 15. API 与 DTO 收敛

最终只保留两组资源：

```text
/api/plugin-drafts
/api/plugins
```

典型动作：

```text
POST   /api/plugin-drafts
POST   /api/plugin-drafts/{id}/generate
POST   /api/plugin-drafts/{id}/preview
POST   /api/plugin-drafts/{id}/save
DELETE /api/plugin-drafts/{id}

GET    /api/plugins
POST   /api/plugins/import
GET    /api/plugins/{id}
PUT    /api/plugins/{id}/enabled
PUT    /api/plugins/{id}/config
POST   /api/plugins/{id}/restore
POST   /api/plugins/{id}/export
POST   /api/plugins/{id}/backup
DELETE /api/plugins/{id}
```

具体 HTTP 动词可按现有 Router 习惯微调，但不得恢复 Project/Mount/Product/Runtime 分裂。前端只允许一个 `pluginPlatform` 类型入口和一个 Bridge。

## 16. 现有代码的保留、重写与删除

### 16.1 明确保留并收敛

- 内容寻址 Artifact Store、路径规范化、原子 staging 和文件扫描中的通用安全原语；
- committed JS Runtime provider，但它退出 Plugin 领域状态；
- 独立 Service process 的 IPC、取消、超时、队列边界、进程树清理；
- Surface 静态文件提供、MessageChannel 握手和 Active Artifact fence；
- SQLite authorizer、参数化查询、KV CAS、文件路径安全和备份中的通用实现；
- Chat model selection、生成请求和 Draft 文件编辑能力；
- Host Credential Store 与 Agent/Desktop 已确认的消费者合同。

### 16.2 必须重写

- Plugin Manifest、compiler、SDK 与 artifact envelope；
- Plugin repository、application service、router 和 API DTO；
- DataRoot、preview storage、migration 与 activation coordinator；
- Binding registry 与 Agent/Desktop adapter；
- Plugin Library、Creator、Detail、Run Surface 和配置 UI；
- Plugin DB canonical baseline、生成合同、边界检查与验收测试。

### 16.3 必须物理删除

- `nomifun-plugin-platform/src/application/` N1 聚合；
- `plugin_n1.rs`、`plugin-n1-contract` 及其生成物；
- M1 中的 Product/Project/Ready/Publish 聚合、repository、DTO 和测试；
- PluginMount、PluginProduct、ReadyCandidate、ReadyRelease、PublishAuthorization、AutoApply 等生产类型；
- Shared Extension Host 及其 Plugin Mount 生命周期；
- Candidate Test Host、Service Test Receipt 和对应持久化；
- 两套 TS 类型、Bridge、Router 和 UI 拼装；
- N1/M1、MiniApp、安装型/发布型 Plugin 的旧产品文案与文档；
- 只为旧状态机存在的 migrations、fixtures、checks、generated schemas 和 release gates；
- 旧 API alias、fallback、feature flag 和“暂时保留以后恢复”的空接口。

全局 Agent 核心仍实际使用的 Package/Action 合同不因名称相似而盲删；只有 Plugin 旧入口和不再有生产消费者的合同退出。

## 17. Clean cut 与无历史债规则

- 在一个重构分支和一个最终合并边界内完成；允许内部按依赖顺序施工，不允许发布或合并任何双架构中间态。
- Plugin 子系统使用新的 canonical baseline；不编写 N1/M1 数据转换器，不读取旧 Plugin 表，不保留旧 Artifact 或 DTO decoder。
- 非 Plugin 数据、Agent 数据和其他产品数据不受此次 clean cut 影响。
- 若开发数据库中存在实验 Plugin，重构前可人工导出；最终生产代码不携带迁移责任。
- 最终文档不能把旧设计继续标记为待办或备用路径；过期规格应删除或显式归档为非权威历史，不得参与当前开发路由。
- 每个替换任务必须同时提交新增、调用方改线、旧实现删除和对应测试；禁止只加 facade 后延期清理。

## 18. 一次性实施顺序

下列只是同一重构分支中的依赖顺序，不是分期交付：

1. 冻结 Unified Manifest、Artifact、Plugin、Draft、DataRoot、Action/Binding 与 SDK 合同。
2. 建立新 canonical schema、repository、DataRoot 和通用 mutation journal。
3. 收敛 Artifact Store、Service Host、Surface Bridge 和 Runtime authority 到新合同。
4. 实现统一 staging、preview、migration、activation、rollback、delete、package 与 backup。
5. 将 Chat Authoring 和外部 Import 接到同一个 `install_artifact`。
6. 实现 Binding registry，并接入保留的 Agent/Desktop 真实消费者。
7. 将前端切换到单一 Draft/Plugin DTO、Bridge 和产品流。
8. 删除 N1/M1 生产代码、DB 表、API、生成物、测试、脚本和文档。
9. 运行完整验收、边界检查、Windows 原生验证及当前 release 所要求的平台验证。
10. 完成终审：生产路径不可达旧实现，仓库不存在继续维护双架构所需的兼容代码。

任何步骤未完成，整个重构保持未完成状态。

## 19. 验收闭环

以下全部成立才允许标记完成：

1. Chat 创建 UI-only App，预览、保存、打开，重启后 DB/KV/Files 持久存在，Node 进程数始终为零。
2. Chat 创建无 UI Agent Tool，保存后真实 Agent 能发现、选择并调用。
3. 创建 UI + Service 混合 Plugin，UI 与 Service 共享同一 DataRoot。
4. 无 UI Plugin 可以绑定保留的 Agent hook 和 Desktop command，并执行真实消费者闭环。
5. 外部目录和 ZIP 与 Chat 产物通过完全相同的 Artifact 校验、安装和 Runtime。
6. 编辑已有 Plugin 时，Preview 使用临时数据副本，任何预览写入都不改变正式 DataRoot。
7. 同 dataVersion 更新原子切换代码并保留数据。
8. dataVersion migration 能同时迁移 SQLite 与 Files；失败时旧 Artifact 和正式 DataRoot 完全不变。
9. 更新提交关键点崩溃后，重启只能恢复完整旧状态或完整新状态，不出现混合指针。
10. 停用、更新、回收和删除会撤销 Surface、Binding、Service、Credential 与 Host Grant 访问。
11. Previous 代码回退与 previous data generation 完整回退符合本文语义，并明确数据损失提示。
12. Package Export 不含用户数据；Backup 包含 DataRoot 和非秘密配置，但不含 Credential 明文。
13. 权限扩张会请求确认；未变化权限不会在每次更新重复授权。
14. 所有 Action、Binding、Storage 和 Bridge 只有一套 SDK、一套 DTO 和一套合同测试。
15. Windows Desktop 与 Desktop WebUI 真实闭环通过；目标 release 要求的 macOS 验证取得目标机证据。
16. `bun run check:desktop-ui-boundary` 通过，且没有新增 880px 以下或移动端布局。

## 20. 完成审计

最终审计至少执行以下语义检查：

```text
生产代码中不存在 N1 / M1 Plugin 路径
不存在 PluginMount / PluginProduct / ReadyCandidate / ReadyRelease
不存在 Apply / Publish / AutoApply / AutoPublish 状态机
不存在两套 Plugin Bridge、DTO、Repository 或 Router
不存在旧 HTTP fallback、字段 alias 或旧 manifest decoder
不存在旧 Plugin 表的生产读写
不存在只为退休模型保留的 TODO、feature flag 或 release gate
```

同时确认：

- 保留代码均有新生产调用方；
- 删除列表与实际文件、导出、生成合同、测试和文档一致；
- Git staged/unstaged 范围没有混入用户无关改动；
- 最终报告记录所有运行命令、平台证据和确实无法在当前主机执行的项目。

## 21. 明确不做

- 不建设 Marketplace、签名运营、远程分发或灰度系统；
- 不建设多 JS Runtime 选择与热切换；
- 不建设 Shared Extension Host；
- 不建设任意历史版本列表；
- 不建设 Dev/Prod 双持久环境；
- 不恢复会话页替换、Shell 插槽或移动端 Plugin UI；
- 不将系统所有内部模块都插件化；
- 不用复杂 Capability/Role/Provider 图替代明确的 Action/Binding；
- 不以“未来可能需要”为由保留旧合同和不可达实现。

这些边界不是功能缩水，而是确保个人本地工具能够长期保持简单、开放和可维护。
