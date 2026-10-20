# 数据与存储

数据库和 ID 改动还必须遵守仓库级
[数据与标识符规范](../contributing/data-and-identifier-standards.zh.md)。
本文主要描述存储行为；可执行 schema 与逻辑关联 registry 仍是实现层面的
权威来源。

NomiFun 把状态保存在三个地方：一个 SQLite 数据库（一切结构化数据的真理之源）、一个按安装划分的**数据目录**（数据库文件、日志、操作系统缓存的运行时），以及按会话划分的**工作目录**（agent 读写的文件）。本页解释什么内容存在哪里、怎么命名，以及如何加以保护。

## 数据目录

| 宿主 | 默认路径 | 覆盖方式 |
| --- | --- | --- |
| 桌面（`nomifun-desktop`） | 按用户的应用数据目录：Windows 上的 `%LOCALAPPDATA%\NomiFun`，macOS 上的 `~/Library/Application Support/NomiFun`，Linux 上的 `$XDG_DATA_HOME/NomiFun`（通常为 `~/.local/share/NomiFun`）。设置了 `NOMIFUN_DATA_DIR` 时，该值**就是**数据根（按字面值，不追加 `/Nomi`）。当前目录或旧根目录中的 pre-v3 数据集会整体退役；其中的产品行不会搬入 v3。 | 环境变量 `NOMIFUN_DATA_DIR` |
| Web（`nomifun-web`）与 `nomicore` bin | 与桌面外壳**完全相同**的按用户目录 —— `%LOCALAPPDATA%\NomiFun` / `~/Library/Application Support/NomiFun` / `$XDG_DATA_HOME/NomiFun`（旧的相对 `./data` 默认值已删除）。设置了 `NOMIFUN_DATA_DIR` 时取**字面值**（不追加 `/Nomi`），因此 Docker `/data`、systemd `/var/lib/nomifun` 部署不受影响。 | 命令行 `--data-dir` 或环境变量 `NOMIFUN_DATA_DIR` |

数据目录内部：

```
<data_dir>/
├── nomifun-backend.db   SQLite database (sqlx)
├── server.lock          exclusive server-lock address file (the lock lives on
│                        the open OS handle; a leftover file is harmless)
├── logs/                tracing-appender file output (rotated daily)
├── conversations/       per-conversation workspaces (see below)
└── companion/                 companion file domain (install-wide memory database + per-companion profiles, see below)
```

三个宿主的缺省默认值都经由同一个共享辅助函数解析：[`nomifun_app::cli::default_data_dir()`](../../crates/backend/nomifun-app/src/cli.rs) —— `dirs::data_local_dir()/NomiFun<channel-suffix>`（按用户的 application-data 位置）。stable 使用 `NomiFun`，非 stable channel 使用 `NomiFun-dev`、`NomiFun-beta` 等**同级目录**——channel 目录永远不嵌套在 stable 根之内；仅当操作系统报告不出用户目录时才极端回退到系统临时目录（`<system temp>/nomifun-data<channel-suffix>`，如 dev 为 `nomifun-data-dev`）。环境变量语义在所有宿主上统一：包括桌面外壳在内（见 [`apps/desktop/src/main.rs`](../../apps/desktop/src/main.rs)），`NOMIFUN_DATA_DIR` 都按**字面值**作为最终数据根（外壳不再追加 `"Nomi"`；clap `env` 绑定与 `nomicore` 共享——它以前不读这个变量）。历史自导出路径仅归一到当前 channel 默认根，不导入退役数据集。设置中的当前数据集迁移使用独立的离线复制、校验和原子位置切换流程。Windows 上展示或持久化的路径不再带 `\\?\` 扩展长度前缀。v3 不会把 `<system temp>/nomifun-data` 或其他旧根目录中的产品数据复制到 active dataset；检测到历史受管数据集时，reset 状态机会将其完整移动到 retired/quarantine 位置，然后创建全新的 v3 数据集，不改写历史数据库路径。

### 更换存储位置

设置中的更换目录会在重启时迁移整个当前数据集，保留历史会话、附件、配置、
加密密钥、浏览器资料及托管工作区。旧版单独 work-dir 中的 `conversations/`
也会迁入新根；外部项目和外部知识库的路径保持原样。数据集 generation 与业务
身份不变，这个操作不再创建空数据集。

`/api/system/storage-relocation/{prepare,start,status,cancel}` 负责预检、安排、
查询和取消一个持久操作。运行中的 Turn、未完成的 Effect、原生 checkpoint 和
未解决的 Patch 恢复会阻止安排。安排后关闭 HTTP 写入入口；所有宿主在打开
运行时缓存、日志及原生浏览器资料前，持有源数据根和工作根的排他锁完成离线迁移。
文件先复制到目标同级暂存目录，逐一核验大小和校验值，再检查当前数据库 schema、
逻辑关联及 SQLite 完整性。

仅有明确托管归属的当前路径会更新。当前 Session 资源通过规范的
`session/agent-binding-changed` 事件及仅改变位置的证明转换，历史 Event、Payload、
Snapshot 保持不可变。默认工作区采用 `managed-session` 逻辑引用，由宿主解析
实际位置。`.nomi/knowledge` 挂载从类型化绑定重新生成；其他链接或重解析点会
阻止预检，不跟随它们指向的目录。

历史消息中的托管附件绝对路径通过同一 Session 的迁移证明链解析到当前位置，
解析结果仅属于读取投影，历史原文不改写。每次更换目录都会记录位置变化，即使
资源已经使用逻辑引用；新消息中的工作区内附件使用相对路径，外部文件仍保留原路径。

根目录外固定控制目录保存计划和唯一原子位置指针。旧 CLI/环境变量路径经根描述符
定位到已提交的新根。成功发布后旧根封存且原数据保留；中断则依据该操作的精确回执
恢复。发布前失败时原根仍可使用，界面显示可重试或取消的失败。目标必须为空，
既有数据集不覆盖、不合并。此流程与备份、恢复、破坏性启动重建相互独立，不获得
转换旧 schema 或清空数据的权限。

### 每个 channel 一个目录、一份状态

同一 build channel 的宿主共用一个默认值是有意为之。已安装桌面应用与生产形态的 `bun run serve:web` 使用 stable 的 `NomiFun`；`bun run dev`、`dev:web` 与 `build:fast` 使用同级的 `NomiFun-dev`。这样既保留每个 channel 内的一份状态，也避免关闭鉴权或实验性的开发循环触碰已安装应用的状态。需要显式选择目录时使用 `NOMIFUN_DATA_DIR` 或 `--data-dir`；开发启动不会从 stable 数据集导入历史 Agent 状态。

让这种共享变得安全的是**排他服务器锁**：启动时（`bootstrap::init_environment`，早于数据库打开）后端对 `{data_dir}/server.lock` 取 OS 级排他 advisory 锁（`fs2`：Unix 上 `flock`，Windows 上 `LockFileEx`）。进程退出*或崩溃*时锁由 OS 释放，因此残留的 `server.lock` 文件无害，不需要任何过期启发式。同一目录上的第二个后端会快速失败，错误信息点名持有者（pid + exe）并给出两条出路：关掉另一个实例，或让这一个指向自己的独立目录。桌面外壳现在会把后端启动失败弹成原生错误对话框并退出（以前是静默白屏）。`nomicore doctor` 与 `mcp-*` stdio 子命令不受该锁影响（`doctor` 设计上就允许与运行中的服务器并存）。

## 通过 `sqlx` 操作 SQLite

[`nomifun-db`](../../crates/backend/nomifun-db/) 是数据层。来自 [`crates/backend/nomifun-db/src/lib.rs`](../../crates/backend/nomifun-db/src/lib.rs) 的要点：

持久化身份遵循 [id-system.md](id-system.zh.md) 中的 v3 分层契约：

- 旧产品表使用 `id INTEGER PRIMARY KEY AUTOINCREMENT`，canonical Agent Store 与 MCP OAuth authority 使用具名 TEXT/复合主键；
- 需要跨数据集的稳定实体增加具名、裸标准 UUIDv7 字段；
- 仅内部关系、单例、缓存和事件行在当前数据集内部使用表 `id`；
  需要产品定位的实体使用具名 UUIDv7 业务字段；
- 旧产品关系使用带索引的逻辑关联；canonical Agent Store 与 MCP OAuth 所有权使用已登记的物理外键。

本地 `id` 是表主键，但不是可移植的业务身份，绝不以技术行主键的身份跨越 API、
事件、受管文件或备份图。需要产品定位的实体使用具名 UUIDv7；产品 wire 契约
不引入整数业务 ID。

具体地说，Agent Execution Participant、Step、Attempt、Template Participant
分别使用 `participant_id`、`step_id`、`attempt_id`、
`template_participant_id`；Channel Plugin、User、Session 分别使用
`channel_plugin_id`、`channel_user_id`、`channel_session_id`。这七类字段都是
裸标准 UUIDv7 业务 ID，不是本地整数身份。同一规则也适用于 MCP Server、
Webhook、Creation Task、Conversation Artifact 和
IDMM Intervention。

- `Database` —— 持有 `sqlx::SqlitePool` 与 v3 baseline schema 状态。通过
  `nomifun-db::SqlitePool` 再导出。
- `init_database` —— 打开或初始化 v3 baseline 数据库。
- `init_database_memory` —— 测试用的内存版本。

该 crate 暴露仓储 **trait + Sqlite 实现** 对。下面是非穷尽列表（完整列表见 `lib.rs` 中的 `pub use repository::{...}` 块）：

| Trait | Sqlite 实现 | 存储 |
| --- | --- | --- |
| `IUserRepository` | `SqliteUserRepository` | 用户与密码哈希 |
| `IConversationRepository` | `SqliteConversationRepository` | 会话 + 消息，含过滤与全文搜索行 |
| `IAgentMetadataRepository` | `SqliteAgentMetadataRepository` | 可用模型与 agent 元数据 |
| `IMcpServerRepository` | `SqliteMcpServerRepository` | 已配置的 MCP 服务器（CRUD） |
| `IMcpOAuthRepository` | `SqliteMcpOAuthRepository` | MCP 客户端注册和当前服务授权；secret 由 authority 加密 |
| `IProviderRepository` | `SqliteProviderRepository` | LLM provider 凭证（加密） |
| `IAgentExecutionRepository` | `SqliteAgentExecutionRepository` | AgentExecution、不可变 Participant、按 revision 演进的 Step/Dependency、Attempt、Conversation Link 与 Event outbox；详见[统一模型](agent-execution.zh.md) |
| `IRequirementRepository` | `SqliteRequirementRepository` | AutoWork requirements；所有 owner 关联都遵循 Repository/Service 管理的逻辑关联策略 |
| `ICronRepository` | `SqliteCronRepository` | 定时任务及其按时区归一化的表达式 |
| `ITerminalRepository` | `SqliteTerminalRepository` | 终端会话元数据 |
| `ISshHostRepository` | `SqliteSshHostRepository` | 加密的 owner 作用域 SSH 主机簿 |
| `IChannelRepository` | `SqliteChannelRepository` | 外部聊天渠道插件配置（Telegram / Lark / DingTalk / WeChat） |
| `IClientPreferenceRepository` | `SqliteClientPreferenceRepository` | 按客户端的偏好 |
| `ITagSettingRepository` | `SqliteTagSettingRepository` | 基于标签的分组（被 AutoWork 使用） |
| `ISettingsRepository` | `SqliteSettingsRepository` | 杂项应用设置 |
| `IWebhookRepository` | `SqliteWebhookRepository` | 出站 webhook 目的地（飞书 Lark） |

伴随其行的若干参数类型包括 `ConversationFilters`、`ConversationRowUpdate`、`MessageRowUpdate`、`MessageSearchRow`、`UpdateCronJobParams`、`McpOAuthClientParams`、`McpOAuthGrantParams`、`McpOAuthGrantRefreshParams`、`CreateProviderParams`、`CreateAgentExecutionParams`、`ReconcileAgentExecutionPlanParams` 和 `SettleAgentExecutionAttemptParams` 等。Repository trait 是功能域契约；领域服务通过它们访问数据，只有范围明确的 bootstrap/schema 维护是直接使用 pool 的已记录例外。

### v3 baseline 与数据集 reset

内嵌 SQL 定义的是全新、空数据库的 v3 baseline。`init_database` 可以记录并
校验该 baseline，但不会把 pre-v3 产品行转换为 v3 行。

打开 SQLite 之前，bootstrap 会检查数据集契约和 generation。不存在数据集时
初始化 v3；检测到历史或不兼容数据集时整体退役并创建新的空 v3 数据集。

本次破坏性版本的启动先只读检查 SQLx lineage，不能仅凭 `users`、
`installation_identity` 等身份表就把历史数据库当成现役数据集。可读但
不兼容的 lineage 通过独立重建原因进入 dataset reset，不受已消耗的旧
一次性 retirement 标记阻断；旧数据库、WAL/SHM 与托管旁路存储一起隔离，
包括 `browser-v4`。新 reset plan 使用冻结的 v3 清单，仍能恢复旧 v1/v2
计划。数据库读取失败及现役 schema/data 损坏保留原数据并失败；迁移条数不能
推断应用版本的新旧。备份恢复不执行此启动重建策略。
不提供逐表历史迁移、兼容读取、ID 规范化或降级路径。

运行时 baseline 校验至少确认：

- 旧产品表使用 `id INTEGER PRIMARY KEY AUTOINCREMENT`；canonical Agent Store 与 MCP OAuth 表使用已登记的 TEXT/复合主键；
- 稳定业务 ID 是裸标准 UUIDv7；
- canonical Agent Store 与 MCP OAuth authority 例外之外的 schema 没有物理 `FOREIGN KEY`、`REFERENCES`、数据库
  cascade 或 `*_row_id`；唯一的 trigger 是基线已注册的 guard trigger；
- 每个逻辑关联都有必需索引和 registry 登记。

### 定时任务所有权

`cron_jobs.user_id` 是定时任务聚合的非空、不可变所有者，不是请求时从
Conversation 临时推导的提示字段。新任务必须显式接收已认证的 canonical 用户
ID；可选的 Conversation 绑定必须已经属于同一个 owner。目标缺失、存在多个
反向 owner，或正反向 owner 不一致时都会拒绝写入，不会猜测或静默修复。

HTTP、Gateway、服务和 Repository 的公开读写都必须携带 `user_id`，越权访问统一表现为不存在。调度器按裸 UUIDv7 `cron_job_id` 读取任务，并在执行前与持久化行重新核对所有者，以阻断删除后同 ID 重建等竞态。Repository 事务负责校验可选 Conversation 关联以及任务生成的 Conversation Artifact 的 owner 一致性。任务所有者不能原地迁移，运行时不存在安装所有者兜底。定时任务只有一个执行目标——Agent，因此 v3 领域模型、API 与基线 schema 不表达 target type，也不包含旧的 terminal 专用字段。

### 安装级执行权限

`installation_identity.owner_user_id` 所引用的 canonical 用户是安装所有者。
只有该 owner 可以启动主机 runtime，
并使用文件、终端、Skill、AgentPreset、知识库挂载、Office 预览和平台 Gateway 等能力。
其他已认证主体只保留普通 Nomi Conversation 和定时任务中的模型调用；用户身份、
role 文本或开放 JSON 都不能扩大权限。

v3 baseline 直接创建上述权限模型，不保留或规范化旧 schema generation 中的历史行。
本地 loopback capability 的签名根和可续期 lease 只存在于进程内，绝不持久化。

### 逻辑关联策略

默认策略是逻辑关联：Agent Store 例外之外的产品表
都没有物理外键。稳定父实体关联（例如
`cron_run_reservations.conversation_id`、`cron_run_reservations.cron_job_id`）保存父实体的裸 UUIDv7
业务 ID。Repository 层在事务
中校验目标，并执行登记的 `RESTRICT`、应用层 `CASCADE`、`SET_NULL` 或
`KEEP_HISTORY` 删除策略。应用层 `CASCADE` 是 Service/Repository 行为，不是
SQLite cascade 或 trigger。例外表使用基线中冻结的物理 `FOREIGN KEY` 与
`ON DELETE` 行为；唯一允许的 trigger 是基线已注册的 guard trigger。
数据库和受管 side-store 都执行 orphan audit。

`requirements.owner_conversation_id` 是采用 `SET_NULL` 删除策略的逻辑关联；
删除会话时由应用事务清空绑定，因此持久化 AutoWork runner 仍可继续运行，
不依赖数据库级 cascade。

### MCP OAuth 配置

`mcp_servers` 继续作为连接配置的唯一来源。`mcp_oauth_clients` 按 owner、issuer 与
client profile 保存不可变注册身份；新注册不会覆盖仍被原 grant 引用的客户端。
`mcp_oauth_grants` 为每个 owner/server 保存一条当前授权，包含确切连接引用、issuer、
resource、client 引用、scopes、有效期与加密的 access/refresh token envelope。
重新登录替换 grant ID；刷新只通过 CAS 推进该 grant 的 revision 和凭据字段。
外键校验 client/owner/issuer 一致性，server soft delete 在同一事务清除 grant。
Attempt、PKCE verifier、授权码与发现缓存不建立持久化表。

前向迁移 `010_mcp_oauth_authority.sql` 清除缺少授权身份归属证明的旧 URL-only OAuth 凭据，
不猜测导入新授权，因此升级后需要重新登录；MCP 配置、其他非 Agent 配置和当前 Agent 事实保留。
迁移只推进已知 manifest，已发布 baseline checksum 不变。两张新表的 reset scope 均为
`Preserve`，Agent-only reset 不清除它们；这不授予备份或恢复重建不支持数据库的权限。

## 静态加密 —— AES-GCM

敏感字符串（provider API key、OAuth token、渠道 bot token 等）在写入前用 AES-256-GCM 加密，由 `nomifun_common::crypto::{encrypt_string, decrypt_string}` 与 `nomifun_app::load_or_create_data_encryption_key` 加载的数据加密密钥承担。

主密钥是每个 v3 数据集独有的 `<data_dir>/encryption_key` 文件，在新数据集
初始化时创建。密码修改或 JWT 轮换不会改变它；v3 reset 不导入历史密文或旧
密钥。丢失 active dataset 的 `encryption_key` 会使该数据集的加密列无法解读。

MCP OAuth client secret 和 access/refresh envelope 在 `McpAuthService` 写仓库前加密。
包含这两类数据的备份必须同时保存原数据集密钥。手工 HTTP headers 与导入 raw JSON 仍沿用
连接配置存储路径，不是 OAuth grant 记录。

工作区中锁定的 `aes-gcm` crate 版本是 `0.10`。

## 按会话的工作区

每个会话拥有一个 agent 可自由读写的目录：

```
{work_dir}/conversations/{workspace_id}/
```

- `work_dir` —— 运行时工作目录；未显式设置时回退至数据目录。来源依次为：`--work-dir` flag → `dir-config.json` 中持久化的工作根绑定 → 环境变量 `NOMIFUN_WORK_DIR` → `<data_dir>`。继承到的 `NOMIFUN_WORK_DIR` 若指向默认数据根位置或已不存在的目录会被忽略（防止自动更新重启时残留的自导出值）。设置迁移会在暂存副本中把新数据根写为工作根绑定，不再创建独立的空工作区数据集。
- `workspace_id` —— canonical AgentSession 的裸小写 UUIDv7；临时工作区投影也把
  同一值放入 `extra.temp_workspace_id`。它固定 36 字符，目录名不包含类型前缀、
  标题 slug 或 `temp` 标记。

未选择自定义工作区时，canonical AgentSession 创建完成后立即物化该目录；已有
Session 在首次运行或浏览时也会幂等补建。会话删除 owner 在提交删除墓碑前移除
该目录。用户选择的自定义工作区不属于这一生命周期，永远不会随会话删除。其内的
文件操作处于沙箱中并被监视：

- [`nomifun-file::path_safety`](../../crates/backend/nomifun-file/src/path_safety.rs) 拒绝逃出工作区的路径（如 `..` 或绝对根）。
- [`nomifun-file::watch_service`](../../crates/backend/nomifun-file/src/watch_service.rs) 借助 `notify` 把文件系统变更通过 WS 反馈给 SPA。
- [`nomifun-file::snapshot_service`](../../crates/backend/nomifun-file/src/snapshot_service/) 记录工具编辑前后的快照以便审计。

仓库通过 `nomifun_common::error::workspace_path_has_edge_whitespace_segment` 强制额外约束：工作区路径的任何目录名不得以空白字符开头或结尾（或整段全为空白）——这类名称会破坏 Win32 路径往返，且在任何 UI 中都无法分辨。目录名内部含空格则完全支持：macOS 默认的用户级数据目录（`~/Library/Application Support/NomiFun`）本身就含空格，而所有子进程管道（`Command::current_dir`、PTY cwd）均以独立参数传递工作区路径，对空格安全。

### 知识库挂载（`.nomi/knowledge/`）

会话、终端会话或伙伴绑定把知识库带入某个工作区时，库会挂载到 `{workspace}/.nomi/knowledge/` 之下——与项目技能同属 `.nomi/` 域——以 junction/symlink 建链、复制兜底，并内置 `.gitignore` 使挂载永不进版本控制。平台托管的 `README.md`（检索协议、各库梗概 + TOC、回写规则）在每次启动时重写。旧位置 `{workspace}/.nomifun/knowledge/` 的遗留挂载会在下次同步时被自动清理。

## 伙伴数据（`companion/` 文件域）

数字伙伴的数据位于主产品数据库表之外，是一个可整体导出、由受管数据重置流程清理的文件域
（详见[伙伴指南](../guides/companions.zh.md)）。它作为受管数据集的一部分参与
v3 reset/restore，不通过历史逐行迁移导入。多伙伴布局如下：

```
<data_dir>/companion/
├── shared/                      整机唯一的一份文件（不是「共享记忆」）
│   ├── config.json              SharedCompanionConfig：采集开关、事件保留/容量策略、会话归档、default_companion_id
│                                （learn/evolve 已于 2026-08 迁到每个伙伴的 profile 上）
│   ├── events/YYYYMMDD.jsonl    采集链路的原始事件（自动按期限/硬容量清理；隐私敏感，导出需显式勾选）
│   └── memory.db                独立 SQLite（PRAGMA user_version 版本阶梯）：
│                                整机一个记忆数据库，但每一行记忆都归属于
│                                唯一一个伙伴（companion_memories.companion_id），
│                                也只有该伙伴读得到；+ 每宠运行态
│                                （companion_runtime_state：XP、mood，以及每个伙伴在
│                                events/ 里的 learn_cursor_ts / evolve_cursor_ts 游标）
└── companions/
    └── {companion_id}/                companion_id 为裸标准 UUIDv7，目录即真相
        └── config.json          CompanionProfileConfig：名称/形象/人格/每宠聊天模型/
                                 该伙伴自己的 learn + evolve 设置/桌宠开关、位置与休眠时段
```

`shared/` 的含义是「整机一份」，不是「伙伴之间共用」。`memory.db` 把所有伙伴的记忆行放在同一张表里，每行用**唯一一列可空的** `companion_id` 标明主人（`companion_skills` 同理）；所有面向伙伴的读取都按主人过滤，因此一个伙伴永远看不到另一个伙伴的记忆，记忆也不能在主人之间转移。唯一的无主状态是 `companion_id IS NULL`：升级前的旧行、以及名册为空时导入的行就是这个样子。这一列保持可空（零伙伴的安装是受支持的状态），这类行会由一次幂等的启动迁移落户到唯一主人名下（若显式 `default_companion_id` 仍在名册里就是它，否则是最早创建的伙伴）——一条 `UPDATE`，绝不按伙伴复制，因此 `memory_id` 保持稳定，同一件事也不会分裂成每个伙伴各一份。

2026-08 之前，两张表用两列编码同一件事：`scope_kind TEXT NOT NULL`（`'user'` / `'companion'`）+ 可空的 `scope_companion_id`，再用一条表级 CHECK 把它们钉死成 `('user', NULL)` 与 `('companion', id)` 两种合法组合。判别列完全由「有没有主人」决定，因此它只可能与主人列打架，不可能提供额外信息；启动迁移把两张表重建为这一列（SQLite 不允许 DROP 一个被表级 CHECK 引用的列）。重建逐行保留原有 `id`：`companion_memories_fts` 是 external-content FTS5，索引以 `content_rowid='id'` 为锚。记忆行与技能行的主人都以列本名 `companion_id` 上线 —— 技能 wire 曾靠一个 `serde(rename)` 多沿用了一个版本的历史名，而这正是 wire 与列悄悄对不上的典型途径；旧包里的 `scope_kind` 在导入时被接受并丢弃，两种行上的 `scope_companion_id` 都被翻译成 `companion_id` —— 导出包是主人磁盘上的长期文件，不是能让他们重新发一次的请求。

历史单宠布局 `companion/nomi/` 不迁移到 v3；检测到它时随整个旧数据集退役。

伙伴绑定的知识库不在 `companion/` 域内：绑定关系存主库 `knowledge_bindings('companion', companion_id)`，知识库内容在知识库自己的托管目录（URL 源知识库抓取的 markdown 快照存于其 `snapshots/` 子目录）。

## 受管执行组件

Release 二进制仅内嵌 OfficeCLI；后端启动不准备或下载 Node/Bun。用户命令优先使用
已有有效工具链，既有受管缓存可复用；实际启动缺少必需运行时时自动准备批准制品。
`nomifun_runtime::init(&data_dir)` 把受管缓存根锚定到 `<data_dir>/runtime`；
普通解析只准入本地可用制品，可本地解压内嵌 OfficeCLI。
消费者使用经验证的绝对路径，不猜测缓存目录名称。只读发现与诊断不下载缺失工具，
也不改系统 PATH。Agent、Terminal、MCP 与托管应用进程复用每次启动的用户工具环境，
完整 Node/npm/npx 发行目录与持久用户 CLI 安装位置分开；运行时升级与 Agent 数据清理不删除用户工具。进程树仍由
`nomi-process-runtime` 持有并回收。

未初始化数据根的独立库消费者通过 `dirs::cache_dir()` 使用平台缓存目录。
runtime 二进制与 Office HTML 产物均为可丢弃缓存。供应、身份、开发覆盖、
第三方说明、诊断和有界预览缓存生命周期详见[外部工具](external-tools.zh.md)。

## 日志

日志通过 `tracing-appender` 进入 `<data_dir>/logs/`。默认级别是 `info`；用 `--log-level`（如 `--log-level info,nomifun_mcp=trace`）或环境变量 `RUST_LOG` 覆盖。在 debug 构建中桌面外壳额外保留控制台（release 构建设置 `windows_subsystem = "windows"`）。

日志配置类型 —— `ResolvedLogging`、`create_file_layer` —— 位于 `nomi_config::logging`（agent 层的配置 crate）。后端通过接缝访问它们：`nomifun_ai_agent::nomi_config::logging::*`。

## 首次运行状态

全新安装的启动顺序如下：

```
0. bootstrap::prepare_startup_storage_root
                                    读取固定位置指针；完成已确认的离线迁移
1. nomifun-runtime::init           将受管 runtime 缓存绑定到当前数据根
2. enhance_process_path             发现用户命令路径
3. bootstrap::init_environment      解析 work_dir，获取数据根与工作根排他锁，
                                    随后创建 log_dir 并初始化 tracing
4. bootstrap::prepare_v3_dataset    check generation; hard reset/quarantine as a whole
5. bootstrap::init_data_layer       initialize/open the v3 database baseline
6. AppServices::from_config         initialize services and managed side stores
7. bootstrap::finalize_data_layer   write and finalize the dataset reset receipt
8. ensure_admin_credentials (web)   pre-seed admin if NOMIFUN_ADMIN_PASSWORD is set
9. create_router → axum::serve      bind and start serving
```

第 3 步就是第二个后端在已被占用的数据目录上快速失败的地方（见上文「一个目录，一份状态」）。

桌面外壳跳过第 8 步的管理员预置，但并不是旧式全局 `--local`：它使用 `TrustLocalToken`，只信任自己 WebView 呈递的本次启动 secret。在 Web 宿主中，如果不存在管理员且未设置 `NOMIFUN_ADMIN_PASSWORD`，安装将进入**首次运行的交互式初始化**：下一位访问浏览器的访客通过 `POST /api/auth/setup` 选择用户名与密码。如果首次运行初始化暴露在非 loopback 绑定地址上，会记录一条警告。

## 备份与重装

- **数据库** —— 使用 SQLite Backup API 或在数据库连接上执行
  `VACUUM INTO` 创建一致快照。不要直接复制
  `nomifun-backend.db`：WAL 数据可能仍在
  `nomifun-backend.db-wal` 中，裸复制可能得到不完整数据库。
- **备份清单** —— 记录当前 format version 3、generation-5 的备份合同、源
  storage-generation、创建时间及每个文件的校验和。Restore 保留完整 SQLite
  身份图，包括业务 UUIDv7、技术 `id` 与 canonical TEXT/复合主键；仅目标的
  `storage-generation` 轮换。
- **加密密钥** —— 离线 bundle 会在文件存在时纳入
  `<data_dir>/encryption_key`。缺少该文件时，provider API key、OAuth token、
  渠道 bot token 等加密列将无法解密。
- **工作区** —— bundle 只递归纳入后端托管的
  `<work_dir>/conversations/`。磁盘其他位置由用户选择的自定义工作区属于外部
  用户项目，绝不会被隐式复制。
- **伙伴数据** —— bundle 递归纳入 `<data_dir>/companion/`（整机一份的记忆数据库 +
  每宠配置，见[伙伴指南](../guides/companions.zh.md)）。
- **受管 runtime 与 Office 预览缓存** —— 可丢弃；内嵌制品下次需要时重新解压，预览重新生成，可选下载仍由用户选择。

`nomicore` 提供离线 CLI 命令：

```text
nomicore --data-dir <源数据目录> backup --output <备份目录>
nomicore restore --bundle <备份目录> --destination-data-dir <新数据目录>
```

`backup` 在打开 SQLite 前获取数据目录的 `server.lock`，因此不会与运行中的
后端竞态；它使用与服务启动相同的 CLI / 持久化配置 / 环境变量规则解析
`work_dir`。输出目录必须不存在，并且必须位于两个源根目录之外。备份只接受
v3 数据集；不会迁移、恢复或 quarantine 历史数据集，历史数据源直接失败。
完整 bundle 包含 WAL 安全的数据库快照、
存在时的持久加密密钥、伙伴文件域和托管会话工作区。日志、`server.lock`、
数据库 WAL/SHM sidecar、runtime/受管 runtime 缓存、浏览器 profile、进程/会话临时
文件和自定义外部工作区均排除。

清单为每个 payload 文件记录可移植相对路径、字节数和 SHA-256，并以目录条目
保留空的伙伴/工作区目录。备份与恢复都会拒绝 symlink、Windows junction /
reparse point、路径穿越、特殊文件、未声明的 payload 文件/目录，以及单文件
超过 8 GiB、总计超过 64 GiB、文件超过 200,000 个或目录超过 200,000 个的
bundle；JSON 清单本身上限为 64 MiB。`restore` 在写入前验证整个 bundle，只接受不
存在或空的目标目录，在目标旁边 staging 并验证所有文件，最后通过一次 rename
安装完整数据目录；失败不会暴露部分目标。恢复整体复制已验证的 WAL 安全 SQLite
快照，保留精确身份及关联图，不重排技术行或重建事实，并验证 schema/data 完整性及
已登记引用。恢复会写入新的
`storage-generation`，避免源数据集浏览器缓存污染恢复后的数据。来自源自定义
work-dir 的托管工作区统一落到
`<destination-data-dir>/conversations`；自定义外部工作区必须由其所有者另行
备份。

bundle 含数据库加密密钥和加密凭据，整个目录都必须按敏感数据保护，并使用与
在线数据目录相同的访问控制来存储和传输。数据库存在加密行但持久密钥缺失，或
密钥文件无效时，`backup` 会拒绝生成无法恢复的 bundle。

恢复命令没有目标 `--work-dir` 选项：它会有意把托管工作区创建在新数据目录
下。若要改用独立 work root，应先移动恢复出的托管目录，并在服务首次启动前
设置常规 work-dir override；绝不能把 restore 目标指向已有外部项目。

这些命令实现当前 generation-5 离线 Backup/Restore，不迁移历史数据或合并
数据集，也不获得启动协调器的重建权限。lineage 不受支持、schema/data 损坏和
I/O 失败均 fail closed。完整 SQLite 快照保留技术 `id`，不使这些整数成为
可移植的产品身份。

干净卸载因此是删除数据目录、（如果单独设置过）工作目录与 OS 缓存目录。

## 交叉参考

- 仓储 trait 及其消费者列在 [`backend-crates.md`](backend-crates.zh.md) 中。
- 命中各仓储的 HTTP 路由，以及镜像状态变化的 WS 主题，汇总在 [`communication.md`](communication.zh.md)。
- agent 侧的数据（TOML 配置、技能、文件缓存）见 [`agent-engine.md`](agent-engine.zh.md)。
