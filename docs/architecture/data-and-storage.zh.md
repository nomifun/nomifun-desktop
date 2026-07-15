# 数据与存储

NomiFun 把状态保存在三个地方：一个 SQLite 数据库（一切结构化数据的真理之源）、一个按安装划分的**数据目录**（数据库文件、日志、操作系统缓存的运行时），以及按会话划分的**工作目录**（agent 读写的文件）。本页解释什么内容存在哪里、怎么命名，以及如何加以保护。

## 数据目录

| 宿主 | 默认路径 | 覆盖方式 |
| --- | --- | --- |
| 桌面（`nomifun-desktop`） | 按用户的应用数据目录：Windows 上的 `%LOCALAPPDATA%\NomiFun\Nomi`，macOS 上的 `~/Library/Application Support/NomiFun/Nomi`，Linux 上的 `$XDG_DATA_HOME/NomiFun/Nomi`（通常为 `~/.local/share/NomiFun/Nomi`）。设置了 `NOMIFUN_DATA_DIR` 时变为 `$NOMIFUN_DATA_DIR/Nomi`。位于 `<system temp>/nomifun-data/Nomi` 的旧版安装会在启动时被自动搬迁（一次性；旧目录保留作备份）。 | 环境变量 `NOMIFUN_DATA_DIR` |
| Web（`nomifun-web`）与 `nomicore` bin | 与桌面外壳**完全相同**的按用户目录 —— `%LOCALAPPDATA%\NomiFun\Nomi` / `~/Library/Application Support/NomiFun/Nomi` / `$XDG_DATA_HOME/NomiFun/Nomi`（旧的相对 `./data` 默认值已删除）。设置了 `NOMIFUN_DATA_DIR` 时取**字面值**（不追加 `/Nomi`），因此 Docker `/data`、systemd `/var/lib/nomifun` 部署不受影响。 | 命令行 `--data-dir` 或环境变量 `NOMIFUN_DATA_DIR` |

数据目录内部：

```
<data_dir>/
├── nomifun-backend.db   SQLite database (sqlx)
├── server.lock          exclusive server-lock address file (the lock lives on
│                        the open OS handle; a leftover file is harmless)
├── logs/                tracing-appender file output (rotated daily)
├── conversations/       per-conversation workspaces (see below)
└── companion/                 companion file domain (shared memory hub + per-companion profiles, see below)
```

三个宿主的缺省默认值都经由同一个共享辅助函数解析：[`nomifun_app::cli::default_data_dir()`](../../crates/backend/nomifun-app/src/cli.rs) —— `dirs::data_local_dir()/NomiFun/Nomi`（按用户的 application-data 位置），仅当操作系统报告不出用户目录时才极端回退到系统临时目录（`<system temp>/nomifun-data/Nomi`）。环境变量语义保持各宿主原状：桌面外壳对 `NOMIFUN_DATA_DIR` 追加 `"Nomi"`（见 [`apps/desktop/src/main.rs`](../../apps/desktop/src/main.rs)），而 `nomifun-web` 与 `nomicore` 取其字面值（clap `env` 绑定 —— 对 `nomicore` 是新增的，它以前不读这个变量）。位于 `<system temp>/nomifun-data/Nomi` 的既有旧版安装会在启动时被一次性搬迁到新位置（[`apps/desktop/src/relocate.rs`](../../apps/desktop/src/relocate.rs)）：数据被复制（可再生的缓存/日志留在原地），旧目录保留作备份，随后后端把数据库中存储的绝对路径（知识库根、会话工作区、终端 cwd）改写到新根。

### 一个目录，一份状态

所有宿主共用一个默认值是有意为之：开发循环（`bun run serve:web`、`dev:web`、`dev`）与已安装的桌面应用读写同一份状态，因此 provider 或伙伴配置一次、处处可测，排查问题也永远只有一个目录可看。确实需要隔离沙箱时，`NOMIFUN_DATA_DIR` 或 `--data-dir` 就是逃生舱。（dev 脚本不再传仓库相对的 `--data-dir`；旧的 `data/` 与 `.dev-data/` 目录不再被任何东西读取，其内容也**不会**被自动迁移 —— 还需要的话请手动拷进新根，或用 `NOMIFUN_DATA_DIR` 指回去。）

让这种共享变得安全的是**排他服务器锁**：启动时（`bootstrap::init_environment`，早于数据库打开）后端对 `{data_dir}/server.lock` 取 OS 级排他 advisory 锁（`fs2`：Unix 上 `flock`，Windows 上 `LockFileEx`）。进程退出*或崩溃*时锁由 OS 释放，因此残留的 `server.lock` 文件无害，不需要任何过期启发式。同一目录上的第二个后端会快速失败，错误信息点名持有者（pid + exe）并给出两条出路：关掉另一个实例，或让这一个指向自己的独立目录。桌面外壳现在会把后端启动失败弹成原生错误对话框并退出（以前是静默白屏）。`nomicore doctor` 与 `mcp-*` stdio 子命令不受该锁影响（`doctor` 设计上就允许与运行中的服务器并存）。

## 通过 `sqlx` 操作 SQLite

[`nomifun-db`](../../crates/backend/nomifun-db/) 是数据层。来自 [`crates/backend/nomifun-db/src/lib.rs`](../../crates/backend/nomifun-db/src/lib.rs) 的要点：

持久化实体身份遵循 [id-system.md](id-system.zh.md) 中的规范前缀 UUIDv7 契约；SQLite 行顺序和自增值都不是可迁移的实体身份。

- `Database` —— 持有 `sqlx::SqlitePool` 与迁移。通过 `nomifun-db::SqlitePool` 再导出。
- `init_database` —— 打开文件、运行内嵌迁移。
- `init_database_memory` —— 测试用的内存版本。

该 crate 暴露约 20 对仓储 **trait + Sqlite 实现**。下面是非穷尽列表（完整列表见 `lib.rs` 中的 `pub use repository::{...}` 块）：

| Trait | Sqlite 实现 | 存储 |
| --- | --- | --- |
| `IUserRepository` | `SqliteUserRepository` | 用户与密码哈希 |
| `IConversationRepository` | `SqliteConversationRepository` | 会话 + 消息，含过滤与全文搜索行 |
| `IAgentMetadataRepository` | `SqliteAgentMetadataRepository` | ACP 握手结果、可用模型、agent 二进制元数据 |
| `IAcpSessionRepository` | `SqliteAcpSessionRepository` | 持久化 ACP 会话（重启后可恢复） |
| `IMcpServerRepository` | `SqliteMcpServerRepository` | 已配置的 MCP 服务器（CRUD） |
| `IOAuthTokenRepository` | `SqliteOAuthTokenRepository` | HTTP MCP 服务器的加密 OAuth token |
| `IProviderRepository` | `SqliteProviderRepository` | LLM provider 凭证（加密） |
| `IRemoteAgentRepository` | `SqliteRemoteAgentRepository` | 远程 agent 端点 |
| `IAgentExecutionRepository` | `SqliteAgentExecutionRepository` | AgentExecution、不可变 Participant、按 revision 演进的 Step/Dependency、Attempt、Conversation Link 与 Event outbox；详见[统一模型](agent-execution.zh.md) |
| `IRequirementRepository` | `SqliteRequirementRepository` | AutoWork requirements（**有意不与 conversations 建立外键** —— 即使会话被删除，循环也要存活） |
| `ICronRepository` | `SqliteCronRepository` | 定时任务及其按时区归一化的表达式 |
| `ITerminalRepository` | `SqliteTerminalRepository` | 终端会话元数据 |
| `IPresetRepository` / `IPresetStateRepository` | `SqlitePresetRepository` / `SqlitePresetRepository` | 关系化设定与每用户选择状态 |
| `IChannelRepository` | `SqliteChannelRepository` | 外部聊天渠道插件配置（Telegram / Lark / DingTalk / WeChat） |
| `IClientPreferenceRepository` | `SqliteClientPreferenceRepository` | 按客户端的偏好 |
| `ITagSettingRepository` | `SqliteTagSettingRepository` | 基于标签的分组（被 AutoWork 使用） |
| `ISettingsRepository` | `SqliteSettingsRepository` | 杂项应用设置 |
| `IWebhookRepository` | `SqliteWebhookRepository` | 出站 webhook 目的地（飞书 Lark） |

伴随其行的若干参数类型包括 `UpdateAgentHandshakeParams`、`ConversationFilters`、`ConversationRowUpdate`、`MessageRowUpdate`、`MessageSearchRow`、`UpdateCronJobParams`、`UpsertOAuthTokenParams`、`CreateProviderParams`、`UpdateRemoteAgentParams`、`CreateAgentExecutionParams`、`ReconcileAgentExecutionPlanParams` 和 `SettleAgentExecutionAttemptParams` 等。Repository trait 是功能域契约；领域服务通过它们访问数据，只有范围明确的 bootstrap/schema 维护是直接使用 pool 的已记录例外。

### 迁移

迁移是用 `sqlx::migrate!` 内嵌的 SQL 文件。它们在每次启动 `init_database` 时运行。Schema 只向前演进；不支持降级。

### 定时任务所有权

`cron_jobs.user_id` 是定时任务聚合的非空、不可变所有者，不是请求时从
Conversation 临时推导的提示字段。新任务必须显式接收已认证的 canonical 用户
ID；可选的 Conversation 绑定必须已经属于同一个 owner。目标缺失、存在多个
反向 owner，或正反向 owner 不一致时都会拒绝写入，不会猜测或静默修复。

HTTP、Gateway、服务和 Repository 的公开读写都必须携带 `user_id`，越权访问统一表现为不存在。调度器是唯一允许按全局任务 id 读取的内部入口，但定时器会同时捕获所有者，并在执行前与持久化行重新核对，以阻断删除后同 id 重建等竞态。任务与 Conversation 的双向绑定以及任务生成的 Conversation Artifact 都由数据库触发器强制同属一个用户；Artifact 的状态更新也在 SQL 写入前按所有者过滤。任务所有者不能原地迁移，运行时不存在安装所有者兜底。定时任务只有一个执行目标——Agent，因此 ID-v2 的领域模型、API 与基线 schema 都不表达 target type，也不包含旧的 terminal 专用字段。

### 安装级执行权限

`installation_identity.owner_user_id` 所引用的 canonical 用户是安装所有者。
只有该 owner 可以启动主机 runtime，
并使用文件、终端、Skill、Preset、知识库挂载、Office 预览和平台 Gateway 等能力。
其他已认证主体只保留普通 Nomi Conversation 和定时任务中的模型调用；用户身份、
role 文本或开放 JSON 都不能扩大权限。

Migration 041 一次性完成硬切：规范化次级用户保留的 Conversation 与定时任务模型
选择，禁用没有可用模型的任务，墓碑化可恢复执行图，删除次级模板与终端，并安装
所有权和 model-only trigger。因为 SQLite migration 不能删除文件，启动 reconciliation
还会清除次级用户或孤儿定时任务的 Skill 目录。本地 loopback capability 的签名根和
可续期 lease 只存在于进程内，绝不持久化。

### 按会话的外键说明

`requirements`（AutoWork 队列）有意**不**为 `conversation_id` 建立外键。AutoWork 持久执行器（`nomifun-requirement`）是后端权威的，并能在会话被删除后存活 —— 外键会把它的生命周期与会话耦合在一起，破坏 boot-resume 的设计。

## 静态加密 —— AES-GCM

敏感字符串（provider API key、OAuth token、渠道 bot token 等）在写入前用 AES-256-GCM 加密，由 `nomifun_common::crypto::{encrypt_string, decrypt_string}` 与 `nomifun_app::load_or_create_data_encryption_key` 加载的数据加密密钥承担。

主密钥是每个安装独有的 `<data_dir>/encryption_key` 文件。旧安装没有该文件时，新版本首次启动会用当前解析到的 JWT secret 派生并写入一次，以保证既有密文可读；之后修改密码或轮换 JWT 不会再改变数据密钥。丢失 `encryption_key` 会使所有加密列无法解读。

工作区中锁定的 `aes-gcm` crate 版本是 `0.10`。

## 按会话的工作区

每个会话拥有一个 agent 可自由读写的目录：

```
{work_dir}/conversations/{label}-temp-{workspace_token}/
```

- `work_dir` —— 运行时工作目录；未显式设置时回退至数据目录。来源依次为：`--work-dir` flag → 环境变量 `NOMIFUN_WORK_DIR` → `<data_dir>`。
- `label` —— 由会话标题派生的短 slug。
- `temp` —— 字面字符串；表明这些目录是用户也可以投放文件的可写暂存空间。
- `workspace_token` —— 后端签发并存入 `extra.temp_workspace_id` 的 `ws_…`
  token；它标识这一个托管工作区，不复用 canonical Conversation 实体 ID。

未选择自定义工作区时，Conversation 行创建完成后立即物化该目录。会话被删除时该目录被移除（`nomifun_common::hooks` 中的 `OnConversationDelete` 钩子）。其内的文件操作处于沙箱中并被监视：

- [`nomifun-file::path_safety`](../../crates/backend/nomifun-file/src/path_safety.rs) 拒绝逃出工作区的路径（如 `..` 或绝对根）。
- [`nomifun-file::watch_service`](../../crates/backend/nomifun-file/src/watch_service.rs) 借助 `notify` 把文件系统变更通过 WS 反馈给 SPA。
- [`nomifun-file::snapshot_service`](../../crates/backend/nomifun-file/src/snapshot_service/) 记录工具编辑前后的快照以便审计。

仓库通过 `nomifun_common::error::workspace_path_has_edge_whitespace_segment` 强制额外约束：工作区路径的任何目录名不得以空白字符开头或结尾（或整段全为空白）——这类名称会破坏 Win32 路径往返，且在任何 UI 中都无法分辨。目录名内部含空格则完全支持：macOS 默认的用户级数据目录（`~/Library/Application Support/NomiFun/Nomi`）本身就含空格，而所有子进程管道（`Command::current_dir`、PTY cwd、ACP 会话 JSON）均以独立参数传递工作区路径，对空格安全。

### 知识库挂载（`.nomi/knowledge/`）

会话、终端会话或伙伴绑定把知识库带入某个工作区时，库会挂载到 `{workspace}/.nomi/knowledge/` 之下——与项目技能同属 `.nomi/` 域——以 junction/symlink 建链、复制兜底，并内置 `.gitignore` 使挂载永不进版本控制。平台托管的 `README.md`（检索协议、各库梗概 + TOC、回写规则）在每次启动时重写。旧位置 `{workspace}/.nomifun/knowledge/` 的遗留挂载会在下次同步时被自动清理。

## 伙伴数据（`companion/` 文件域）

数字伙伴的数据刻意**不进主库迁移体系**，而是一个可整体导出/清空的文件域（详见[伙伴指南](../guides/companions.zh.md)）。多伙伴布局如下：

```
<data_dir>/companion/
├── shared/                      共享记忆中枢（全体伙伴一份）
│   ├── config.json              SharedCompanionConfig：采集开关、学习间隔与学习模型、default_companion_id
│   ├── events/YYYYMMDD.jsonl    采集链路的原始事件（隐私敏感，导出需显式勾选）
│   └── memory.db                独立 SQLite（PRAGMA user_version 版本阶梯）：
│                                共享记忆/建议/学习历史 + 每宠运行态（companion_runtime_state：XP 等）
└── companions/
    └── {companion_id}/                companion_{uuid_v7}，目录即真相
        └── config.json          CompanionProfileConfig：名称/形象/人格/每宠模型/桌宠开关与位置
```

旧版单宠布局 `companion/nomi/` 在首次启动时被自动迁移为 `shared/` + 第一只伙伴 "Nomi"，原目录写入 `.migrated` 标记后保留（一个版本周期后清理）。

伙伴绑定的知识库不在 `companion/` 域内：绑定关系存主库 `knowledge_bindings('companion', companion_id)`，知识库内容在知识库自己的托管目录（URL 源知识库抓取的 markdown 快照存于其 `snapshots/` 子目录）。

## 内置 bun 运行时

NomiFun 自带其 `bun` 运行时（1.3.13），使 MCP 服务器与工具子进程不需要系统级 Node.js 安装：

| 步骤 | 发生了什么 |
| --- | --- |
| 编译期 | 目标 OS/arch 的 bun 二进制经过 **zstd 压缩** 并通过 `include_dir!` 内嵌进 `nomifun-runtime`。 |
| 首次运行 | `nomifun_runtime::init(&data_dir)` 把二进制解压到 **`<data_dir>/runtime/`** 子树（详见下文运行时缓存说明）。 |
| 启动 | `enhance_process_path()` 把 bun 的 bin 目录前置到进程 `PATH`，**且早于任何 tokio 线程被构建**（顺序在两个宿主的 `main.rs` 中都得到强制）。 |
| 派生 | `nomi_process_runtime::ChildProcessBuilder` 继承启动期合并后的 `PATH`，使 `npx`、`bun` 与其他 JS 工具能正确解析。 |
| 清理 | `nomi_process_runtime::ProcessSupervisor` 或 `kill_process_tree` 统一持有并回收 Agent / MCP 子进程树。 |

运行时缓存锚定在后端的 `data_dir` 上：[`nomifun_runtime::init(&data_dir)`](../../crates/backend/nomifun-runtime/src/cache.rs) 把 `<data_dir>/runtime` 记为缓存根，因此在桌面上 bun 二进制会解压到 `<data_dir>/runtime/bun-<version>-<sha12>/` —— 即 Windows 上默认的 `%LOCALAPPDATA%\NomiFun\Nomi\runtime\bun-…\`（macOS/Linux 为对应的按用户 app-data 位置），或设置了 env var 时的 `$NOMIFUN_DATA_DIR/Nomi/runtime/bun-…/`。当 `init` 未被调用时（`mcp-*` 子命令、单元测试、`build.rs`），缓存通过 `dirs::cache_dir()` 回退到平台缓存目录：Windows 上的 `%LOCALAPPDATA%\nomifun\runtime\`、macOS 上的 `~/Library/Caches/nomifun/runtime/`、Linux 上的 `$XDG_CACHE_HOME/nomifun/runtime/`（或 `~/.cache/nomifun/runtime/`）。

## 日志

日志通过 `tracing-appender` 进入 `<data_dir>/logs/`。默认级别是 `info`；用 `--log-level`（如 `--log-level info,nomifun_mcp=trace`）或环境变量 `RUST_LOG` 覆盖。在 debug 构建中桌面外壳额外保留控制台（release 构建设置 `windows_subsystem = "windows"`）。

日志配置类型 —— `ResolvedLogging`、`create_file_layer` —— 位于 `nomi_config::logging`（agent 层的配置 crate）。后端通过接缝访问它们：`nomifun_ai_agent::nomi_config::logging::*`。

## 首次运行状态

全新安装的启动顺序如下：

```
1. nomifun-runtime::init           extract bun into OS cache
2. enhance_process_path             prepend cache bin dir to PATH
3. bootstrap::init_environment      resolve work_dir / log_dir, init tracing,
                                    take the exclusive {data_dir}/server.lock
4. bootstrap::init_data_layer       open database, run migrations
5. AppServices::from_config         instantiate every service
6. ensure_admin_credentials (web)   pre-seed admin if NOMIFUN_ADMIN_PASSWORD is set
7. create_router → axum::serve      bind and start serving
```

第 3 步就是第二个后端在已被占用的数据目录上快速失败的地方（见上文「一个目录，一份状态」）。

桌面外壳跳过第 6 步的管理员预置，但并不是旧式全局 `--local`：它使用 `TrustLocalToken`，只信任自己 WebView 呈递的本次启动 secret。在 Web 宿主中，如果不存在管理员且未设置 `NOMIFUN_ADMIN_PASSWORD`，安装将进入**首次运行的交互式初始化**：下一位访问浏览器的访客通过 `POST /api/auth/setup` 选择用户名与密码。如果首次运行初始化暴露在非 loopback 绑定地址上，会记录一条警告。

## 备份与重装

- **数据库** —— 使用 SQLite Backup API 或在数据库连接上执行
  `VACUUM INTO` 创建一致快照。不要直接复制
  `nomifun-backend.db`：WAL 数据可能仍在
  `nomifun-backend.db-wal` 中，裸复制可能得到不完整数据库。
- **备份清单** —— 记录 schema version、storage-generation/dataset ID、
  创建时间以及每个文件的校验和。Restore 保留实体 ID；Merge 导入遇到同
  ID 不同内容必须失败。
- **加密密钥** —— 离线 bundle 会在文件存在时纳入
  `<data_dir>/encryption_key`。缺少该文件时，provider API key、OAuth token、
  渠道 bot token 等加密列将无法解密。
- **工作区** —— bundle 只递归纳入后端托管的
  `<work_dir>/conversations/`。磁盘其他位置由用户选择的自定义工作区属于外部
  用户项目，绝不会被隐式复制。
- **伙伴数据** —— bundle 递归纳入 `<data_dir>/companion/`（共享记忆中枢 +
  每宠配置，见[伙伴指南](../guides/companions.zh.md)）。
- **bun 运行时缓存** —— 可丢弃；下次启动时会重新解压。

`nomicore` 提供离线 CLI 命令：

```text
nomicore --data-dir <源数据目录> backup --output <备份目录>
nomicore restore --bundle <备份目录> --destination-data-dir <新数据目录>
```

`backup` 在打开 SQLite 前获取数据目录的 `server.lock`，因此不会与运行中的
后端竞态；它使用与服务启动相同的 CLI / 持久化配置 / 环境变量规则解析
`work_dir`。输出目录必须不存在，并且必须位于两个源根目录之外。备份以不运行
migration、恢复或 quarantine 的方式打开现有 ID-v2 数据库；无效源会直接失败。
完整 bundle 包含 WAL 安全的数据库快照、
存在时的持久加密密钥、伙伴文件域和托管会话工作区。日志、`server.lock`、
数据库 WAL/SHM sidecar、runtime/Bun 缓存、浏览器 profile、进程/会话临时
文件和自定义外部工作区均排除。

清单为每个 payload 文件记录可移植相对路径、字节数和 SHA-256，并以目录条目
保留空的伙伴/工作区目录。备份与恢复都会拒绝 symlink、Windows junction /
reparse point、路径穿越、特殊文件、未声明的 payload 文件/目录，以及单文件
超过 8 GiB、总计超过 64 GiB、文件超过 200,000 个或目录超过 200,000 个的
bundle；JSON 清单本身上限为 64 MiB。`restore` 在写入前验证整个 bundle，只接受不
存在或空的目标目录，在目标旁边 staging 并验证所有文件，最后通过一次 rename
安装完整数据目录；失败不会暴露部分目标。所有实体 ID 保持不变，但会写入新的
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

这些命令实现完整的离线 Backup/Restore。ID 契约中的逻辑 Merge/Clone 规则
尚未接入真实 SQLite CLI 操作，文档和产品界面不得宣称 CLI 已提供 Merge 或
Clone。

干净卸载因此是删除数据目录、（如果单独设置过）工作目录与 OS 缓存目录。

## 交叉参考

- 仓储 trait 及其消费者列在 [`backend-crates.md`](backend-crates.zh.md) 中。
- 命中各仓储的 HTTP 路由，以及镜像状态变化的 WS 主题，汇总在 [`communication.md`](communication.zh.md)。
- agent 侧的数据（TOML 配置、技能、文件缓存）见 [`agent-engine.md`](agent-engine.zh.md)。
