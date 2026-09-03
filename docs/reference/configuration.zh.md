# 配置参考

NomiFun 读取的每一个参数与环境变量、它们的默认值，以及各自归属的文件。
所有取值都直接来自源码——本页没有列出的设置就不存在。

NomiFun 交付的是**一个**统一的 Rust 后端（`nomifun-app`，二进制
`nomicore`），以及两个嵌入它的宿主：

- `nomifun-desktop` —— Tauri 桌面外壳。在选定的 loopback 端口上以
  `AuthPolicy::TrustLocalToken` 启动后端，并把每次启动生成的本地信任 secret
  注入自己的 WebView。
- `nomifun-web` —— 独立的 Web/服务端宿主。默认以**已鉴权**模式启动同一
  个后端，并在同一端口上提供 SPA。

两个宿主共享后端的同一组配置面；各自的 CLI 仅会覆盖它们自己拥有的那几项。

## `nomifun-web` 参数与环境变量

来源：[`apps/web/src/main.rs`](../../apps/web/src/main.rs)。

| 参数 | 环境变量 | 默认值 | 用途 |
|---|---|---|---|
| `--host` | `NOMIFUN_WEB_HOST` | `127.0.0.1` | 绑定的 IP。`0.0.0.0` 会接受 LAN/VPN/公网流量；大范围暴露前请先预置或完成首次设置。不解析主机名；非法输入将在启动阶段直接失败。 |
| `--port` | `NOMIFUN_WEB_PORT` | `8787` | TCP 端口。在同一个 socket 上提供 API、`/ws` WebSocket 与 SPA。 |
| `--data-dir` | `NOMIFUN_DATA_DIR` | 按用户的应用数据目录 | 后端数据目录（SQLite 数据库、智能体状态、日志、Bun 缓存）。默认是当前 channel 的宿主共享的按用户位置；stable 使用 `NomiFun`，dev 使用同级的 `NomiFun-dev`。可用本参数或 `NOMIFUN_DATA_DIR`（按字面值，不附加后缀）覆盖；生产环境请使用绝对路径。 |
| `--dist` | `NOMIFUN_WEB_DIST` | `../../ui/dist` | 已构建 SPA 所在目录。在仓库之外部署时务必显式指定。 |
| `--admin-user` | `NOMIFUN_ADMIN_USERNAME` | `admin` | 预置首位管理员时使用的用户名。管理员存在后将被忽略。 |
| `--admin-password` | `NOMIFUN_ADMIN_PASSWORD` | — | 在启动时预置首位管理员密码，跳过交互式设置。管理员存在后将被忽略。 |
| `--insecure-no-auth` | `NOMIFUN_WEB_INSECURE_NO_AUTH` | `false` | 危险。完全禁用鉴权（桌面式本地模式）。仅可用于 loopback 或完全受信任的私有网络。 |

布尔环境变量接受 `1`、`true`、`yes`、`on`（不区分大小写）。

## `nomicore`（后端）参数

来源：[`crates/backend/nomifun-app/src/cli.rs`](../../crates/backend/nomifun-app/src/cli.rs)。

下面是独立 `nomicore` 二进制对外暴露的参数。两个宿主会构造一个带默认值
的 `Cli`，仅覆盖各自拥有的那部分——所以单独运行后端时这些参数同样适用。

| 参数 | 默认值 | 用途 |
|---|---|---|
| `--host` | `127.0.0.1`（`DEFAULT_HOST`） | 监听的主机地址。 |
| `--port` | `25808`（`DEFAULT_PORT`） | 监听端口。 |
| `--data-dir` | 按用户的应用数据目录 | 数据库 + 文件存储根目录。通过 clap 绑定 `NOMIFUN_DATA_DIR` 环境变量（按字面值）；两者都未设置时解析 `default_data_dir()`——当前 build channel 的宿主共享的按用户位置。 |
| `--work-dir` | （无） | 会话工作区目录。回退顺序：UI 中选择并持久化在 `dir-config.json` 的工作区 → `NOMIFUN_WORK_DIR` 环境变量 → 数据目录本身。 |
| `--app-version` | crate 版本 | 报告给扩展引擎用于做兼容性检查的宿主应用版本。 |
| `--local` | `false` | 独立 `nomicore` 的无鉴权本地模式。`nomifun-web --insecure-no-auth` 映射到同一策略。桌面外壳不使用该 flag，而是使用 `TrustLocalToken`。 |
| `--log-dir` | `<data-dir>/logs` | 滚动日志的目录。 |
| `--log-level` | `info` | 日志级别过滤。支持按 target 覆盖——例如 `info,nomifun_mcp=trace`。 |

子命令（供智能体 CLI 桥与诊断使用）：

| 子命令 | 用途 |
|---|---|
| `mcp-requirement-stdio` | AutoWork requirement 声明工具的 MCP stdio server。 |
| `mcp-knowledge-stdio` | 每会话 knowledge search 的 MCP stdio server。 |
| `mcp-gateway-stdio` | 平台 Gateway 工具的内部 stdio 传输；只接受宿主签发、带作用域、有效期和签名的能力声明。 |
| `mcp-open-stdio` | 暴露可靠 OS `open` 工具的 MCP stdio server。 |
| `terminal-hook --event <kind>` | 一次性 terminal 生命周期 hook relay。 |
| `doctor` | 自检：填充智能体注册表，逐个探测 `$PATH` 上的每个 CLI，并打印一张按智能体维度的可用性表格。 |
| `remote open <binding_id>` | 打开 canonical Remote AgentSession。 |
| `remote turn <agent_session_id> <json-input>` | 在显式 Remote AgentSession 上启动一轮。 |
| `remote observe <agent_session_id>` | 从 `--after-seq` 后读取 Remote Session Event/消息。 |
| `remote cancel <agent_session_id>` | 取消显式 Session 的活动 Turn。 |

## 共享环境变量

下列变量由后端读取，不论被哪个宿主嵌入。

| 环境变量 | 读取方 | 作用 |
|---|---|---|
| `NOMIFUN_DATA_DIR` | 所有宿主 | 当宿主选择遵循该值时，作为后端数据目录的真值来源。在**每个**宿主上——桌面外壳、独立 Web 宿主与 `nomicore` 二进制——都按**字面值**作为最终数据根（桌面外壳不再附加 `/Nomi`）。未设置时目录为按用户的应用数据默认值（见[下文](#数据目录与工作目录的语义)）。 |
| `NOMIFUN_NFAGENT_PATH` | 桌面外壳 | 开发/离线调试覆盖：指定一个已存在的 `nfagent` **绝对路径**，跳过 NomiRelay 按需下载。显式文件由设置者负责信任与版本管理；普通用户应使用受 SHA-256 校验的托管运行时。 |
| `NOMIFUN_WORK_DIR` | `nomicore` | `--work-dir`（按会话区分的工作区根）的回退值。优先级低于 UI 中选择并持久化在 `dir-config.json` 的工作区；若继承到的值指向默认数据根位置或已不存在的目录，会被忽略（防止自动更新重启时残留的自导出值）。 |
| `JWT_SECRET` | `nomifun-app` | 用于签发会话 JWT 的密钥。解析顺序见 [鉴权密钥解析](#鉴权密钥解析)。 |
| `NOMIFUN_HTTPS` | `nomifun-auth::CookieConfig` | 取真值时，会话与 CSRF cookie 会带上 `Secure` 标记和 `SameSite=Strict`。当应用通过 HTTPS 暴露（TLS 反向代理等）时请打开。默认 `false` → 不带 `Secure` 标记，`SameSite=Lax`。 |
| `SHELL` | 智能体引擎（Linux/macOS） | 智能体引擎派生子进程时使用的 shell。在 systemd 下的 Linux 服务器上请显式设置（系统账户通常没有 `$SHELL`）。 |
| `NOMIFUN_URL` | `nomicore remote` | 使用 canonical Remote 操作时的运行中实例 base URL。 |
| `NOMIFUN_ACCESS_TOKEN` | Fresh-v4 host、`nomicore remote` | `/mcp` 与 `/api/remote/*` 使用的安装级令牌；启动值会播种/轮换持久 verifier，且不绑定伙伴。 |

代码库不集成 `SENTRY_DSN`：这个环境变量并未被读取。

## 后端常量

来源：[`crates/backend/nomifun-common/src/constants.rs`](../../crates/backend/nomifun-common/src/constants.rs)。
这些是编译期值，不是环境变量——列在这里只是为了让运维方了解相关上限。

| 常量 | 取值 | 用途 |
|---|---|---|
| `DEFAULT_HOST` | `127.0.0.1` | `nomicore` 的默认 `--host`。 |
| `DEFAULT_PORT` | `25808` | `nomicore` 的默认 `--port`。（Web 宿主将其覆写为 `8787`。） |
| `BODY_LIMIT` | `10 MiB` | 应用于每条路由的默认请求体大小限制。需要更大的路由（例如 `/api/fs/upload`）会安装自己的更大限制。 |
| `UPLOAD_MAX_SIZE` | `30 MiB` | 文件上传路由（`/api/fs/upload`）的上限。 |
| `MAX_REMOTE_IMAGE_SIZE` | `5 MiB` | 下载聊天中引用的远程图片时的上限。 |
| `COOKIE_NAME` | `nomifun-session` | 会话 cookie。 |
| `CSRF_COOKIE_NAME` | `nomifun-csrf-token` | CSRF cookie（不是 HttpOnly——JavaScript 需要读取它）。 |
| `CSRF_HEADER_NAME` | `x-csrf-token` | 与 CSRF cookie 值对应的请求头（Double Submit Cookie 模式）。 |
| `COOKIE_MAX_AGE_DAYS` | `30` | Cookie 的 `Max-Age`。 |
| `SESSION_MAX_AGE_SECONDS` | `30d` | JWT 有效期，与浏览器会话 Cookie 生命周期保持一致。 |
| `HEARTBEAT_INTERVAL` / `HEARTBEAT_TIMEOUT` | `30s` / `60s` | WebSocket 的心跳 ping/pong。 |

## 数据目录与工作目录的语义

- `data-dir` 存放 SQLite 数据库（`nomifun-backend.db*`）、各智能体状态、
  Bun 缓存、日志文件，以及任何嵌入式扩展数据。把它当成普通数据库来
  对待——做好备份、限制权限。两个同时运行的后端共享它的情况已被机制
  性地阻止（见下面的服务器锁）。
- 三个宿主（`nomifun-desktop`、`nomifun-web`、独立的 `nomicore`
  二进制）都通过 `nomifun_app::cli::default_data_dir()` 解析默认目录。
  同一 build channel 的宿主共享它：stable 使用按用户的 `NomiFun` 目录
  （Windows 上的 `%LOCALAPPDATA%\NomiFun`、macOS 上的
  `~/Library/Application Support/NomiFun`、Linux 上的
  `$XDG_DATA_HOME/NomiFun`），非 stable channel 使用 `NomiFun-dev`、
  `NomiFun-beta` 等**同级目录**——channel 目录永远不嵌套在 stable 根
  之内。根脚本中的 `dev`、
  `dev:web`、`build:fast` 选择 dev；已安装应用、`serve:web` 与 release
  构建保持 stable。需要把 stable 快照复制到开发目录时运行
  `bun run seed:dev`；需要显式位置时则使用 `NOMIFUN_DATA_DIR` 或
  `--data-dir`。
- 后端启动时（早于打开数据库）会对 `{data_dir}/server.lock` 取一把
  OS 级**排他锁**。同一数据目录上的第二个后端进程会快速失败，错误
  信息会指出持有者（pid + 可执行文件名）并给出两条出路：关掉另一个
  实例，或用 `NOMIFUN_DATA_DIR` / `--data-dir` 给这一个指一个独立
  目录。锁是 advisory 的（经 `fs2` 走 `flock` / `LockFileEx`），进程
  退出或崩溃时由 OS 自动释放——残留的 `server.lock` 文件无害。
  `nomicore doctor` 与 `mcp-*` stdio 子命令不取这把锁（doctor 设计上
  就允许与运行中的服务器并存）。
- `work-dir` 存放按会话区分的工作区。未设置时按以下顺序解析：
  `--work-dir` → UI 中选择并持久化在 `dir-config.json` 的工作区 →
  非空的 `NOMIFUN_WORK_DIR` 环境变量 → 数据目录本身。继承到的
  `NOMIFUN_WORK_DIR` 若指向默认数据根位置或已不存在的目录会被忽略——
  以防自动更新重启时残留的自导出值。
  会话会在 `<work-dir>/conversations/` 下创建子目录；删除会话同时
  删除其工作区。
- 所有宿主——包括桌面外壳——都把 `NOMIFUN_DATA_DIR` 当作**最终
  数据根**，按字面值生效、不附加 `/Nomi` 后缀，因此 Docker
  （`/data`）与 systemd（`/var/lib/nomifun`）部署不受影响。环境变量
  与 `--data-dir` 都未设置时，回退到上述共享的按用户默认目录；旧的
  相对 `data` 默认值已不复存在。旧版构建使用
  `NomiFun/Nomi<suffix>`（更早为 `<system temp>/nomifun-data/Nomi`）；
  升级后首次启动时，既有的遗留数据集会被自动迁移到
  `NomiFun<suffix>`（一次性、抗崩溃、中断后下次启动续跑；若旧应用
  实例仍在运行则推迟到下次启动）。数据库中持久化的绝对路径——
  知识库根目录、终端 cwd、自定义工作区——会在搬迁后一次性改写。

## 鉴权密钥解析

`JwtService` 由单一密钥构造；`AppServices::from_config` 按以下顺序解析它：

1. 若已设置，使用 `JWT_SECRET` 环境变量。
2. 否则，使用 `installation_identity.owner_user_id` 所指向的安装所有者用户行中
   持久化的值。
3. 否则，生成一个全新的强随机密钥，并**持久化到数据库**供后续启动使用。

修改密码流会顺带轮换 JWT 密钥，使所有现有会话失效。

静态加密使用独立的持久密钥，存放在 `<data-dir>/encryption_key`。旧安装若还没有该文件，启动时会用当前解析到的 JWT 密钥派生并写入一次，以保证既有加密字段仍可读取；之后修改密码或轮换 JWT 密钥不会再改变数据加密密钥。

## TLS / HTTPS Cookie 处理

NomiFun 自身不做 TLS 终止——请在前面放一个负责 TLS 终止的反向代理
（Caddy、nginx 等）。届时：

- 设置 `NOMIFUN_HTTPS=true`，使 cookie 带上 `Secure` 标记和
  `SameSite=Strict`。否则浏览器会在 HTTPS 响应上拒收 `Secure` cookie，
  登录看似会无声失败。
- `/ws` 上的 WebSocket 升级无需额外的请求头即可穿过任何符合标准的
  代理；Caddy 开箱即用。

可参考 [`guides/web-server-deployment.md`](../guides/web-server-deployment.md)
中完整的 Caddy + Docker 示例。

## 日志

- 所有日志同时写入 stdout（让 `journalctl`/`docker logs` 能捕捉到）以及
  `<log-dir>/nomicore.log` 上的按日滚动文件。
- `--log-level` 接受完整的 [`tracing` `EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
  指令：一个全局级别，或一组以逗号分隔的按 target 覆盖项。

  示例：

  - `info` —— 全局 info。
  - `debug` —— 全局 debug。较啰嗦；适合短时复现。
  - `info,nomifun_mcp=trace` —— 默认 info，MCP 模块为 trace。
  - `warn,nomifun_conversation=info,nomifun_terminal=debug` —— 整体
    更安静；会话引擎为 normal/info；终端为 debug。

不存在另一套 `RUST_LOG` 通路——`--log-level`（或宿主中等价的环境驱动
开关）是唯一的总开关。

## 另见

- [Web 服务部署](../guides/web-server-deployment.md) —— 用 Docker、
  systemd、Caddy 运行 `nomifun-web`。
- [作为桌面应用运行 Nomi](../guides/desktop-app.md) —— 桌面端专属配置。
- [API 概览](./api-overview.zh.md) —— 配置完成并启动后，后端对外暴露
  了什么。
- [疑难排查](./troubleshooting.zh.md) —— 配置在运行时出错时的症状与
  修复方法。
