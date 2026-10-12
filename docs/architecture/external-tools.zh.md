# 外部工具与 Office 附件预览

本文描述当前实现。组件清单是
[`nomifun-runtime/components.json`](../../crates/backend/nomifun-runtime/components.json)，
供应与准入入口是
[`nomifun_runtime::resolve_tool`](../../crates/backend/nomifun-runtime/src/managed.rs)。
构建、运行、诊断与内置 Office 技能使用同一组件身份。

## 组件归属与交付

| 组件 | 批准版本 | 来源 | 使用者 |
| --- | --- | --- | --- |
| Bun | 1.3.13 | `oven-sh/bun` 的固定 release | 显式要求 Bun 的任务 |
| Node | 22.22.0 | `nodejs.org/dist` 的固定 release | Unified Plugin Core 的真实 Node 运行环境 |
| OfficeCLI | 1.0.156 | `iOfficeAI/OfficeCLI` 的固定 release；官方 npm 身份为 `@officecli/officecli` | Office 只读预览与文档技能 |

表中版本来自组件清单。清单同时记录目标平台、架构、下载地址、上游摘要来源、
制品 SHA-256、交付方式与开发版本范围。制品摘要用于组件下载校验；解包后可执行文件的
摘要用于运行时缓存和 `VerifiedTool.digest`，两者不一定相同。

Release 构建仅内嵌 OfficeCLI；Node/Bun 由同一清单受管下载，不进入应用
载荷。Debug 构建可用 `NOMIFUN_EMBED_OFFICECLI=1` 内嵌 OfficeCLI。缺少当前
目标的批准制品时构建失败。构建仍校验批准的 Node/Bun 归档并导出可执行摘要，
仅嵌入小型身份元数据。旧内嵌交付留下的同版本、同摘要可执行缓存可直接校验并
离线复用，不信任缓存 stamp，也不要求重新下载归档。
Docker 构建镜像的标签与 digest 也在组件清单中固定。
组件版本变更应同时更新制品、探测、测试和对应的第三方说明；没有运行期间的
全局 npm 自动安装或 `@latest` 升级。

组件许可证与 notice 的版本和来源由
[`notices/manifest.json`](../../crates/backend/nomifun-runtime/notices/manifest.json)
声明，随组件交付。可选 `nfagent`、ONNX 原生库与浏览器保持原有领域供应机制，
清单中的 `domain_components` 指向各自的权威来源，不建立第二套下载账本。
用户配置的 MCP 与终端 CLI 保留用户选择，发现失败必须与成功的空配置区分。

## 供应、身份与执行环境

`resolve_tool(ToolId::Bun | Node | OfficeCli)` 返回 `VerifiedTool`，包含已验证的
绝对路径、实际版本与可执行文件摘要。OfficeCLI 从内嵌载荷本地解压；
Node/Bun 先检查受管缓存，必要时才下载批准的平台制品。两种供应均使用
`<data_dir>/runtime`、进程间锁、有界临时文件、摘要验证和原子发布。
下载组件的可执行摘要由已校验的批准制品导出，不单独信任缓存 stamp。独立库调用未初始化数据根时使用平台缓存目录。
不要猜测某台机器的路径；读取 resolver 返回值或诊断的 `executable` 字段。

准入会执行真实能力探测。Node 必须报告 Node 引擎且没有 `process.versions.bun`；
Bun 必须报告 Bun 引擎。将 Bun 命名为 `node`，或让它报告兼容的 Node 版本，
都不能通过 Node 准入。OfficeCLI 在私有临时目录创建 DOCX 并生成 HTML，确认
所需协议；单独的 `--version` 或文件存在不足以证明它可用。

成功准入在进程内复用；失败可重试。组件准入后出现尺寸或修改时间变化会拒绝
继续使用，修复后重启。应用不创建 Bun-as-Node 别名，不修改系统 PATH；保留
应用进程及其子进程对已准备 Bun 的 PATH 发现。同步 PATH 发现不触发或等待
下载；真正的 Bun 消费者异步准备并使用返回的绝对路径。Plugin 的 Node 权威保持进程内
唯一，Node 消费者使用 resolver 返回的真实 Node。
子进程启动、取消、超时和进程树回收继续由 `nomi-process-runtime` 管理。

Debug 构建允许显式绝对路径覆盖：`NOMIFUN_BUN_PATH`、`NOMIFUN_NODE_PATH`、
`NOMIFUN_OFFICECLI_PATH`。路径会 canonicalize，仍必须通过引擎、能力和清单内
开发版本范围验证。Release 忽略这些覆盖，不回退到 PATH 上的同名工具。
开发覆盖属于单次启动环境，不能用来替换正式产品的供应策略。

对于声明的 OfficeCLI 家族 Skill，`engine_skills::compile` 解析一次组件，
附加独立、不可变的 Host context resource，包含状态、绝对可执行路径、版本、
摘要、来源和包身份。编译后的正文给出确切 `read_context_resource` ID。
被冻结的源正文、源摘要与 capability 要求不变；工具元数据不授予额外的进程／
文件权限。Skill 读取该资源，不猜测缓存路径、不全局安装替代工具。

## 诊断

```bash
nomicore tool-doctor
nomicore --data-dir /absolute/data-dir tool-doctor --verify
```

此子命令属于 `nomicore`；独立的 `nomifun-web` 宿主 CLI 不接受它。可用
`cargo build --release -p nomifun-app --bin nomicore` 构建，诊断时显式指定
与被检查安装相同的数据根。

第一条只输出组件、领域供应引用与 notice 清单：不解压、不启动探测、不打开
数据库。第二条显式执行准入，可能下载 Node/Bun、写入所选数据目录的 runtime 缓存、创建私有临时探测
文件并启动子进程。JSON 中的 `observations` 包含 `ready` / `unavailable`、
版本、实际路径、摘要或具体错误；任一组件失败时退出码非零。
两种模式都不初始化 Agent 数据、数据库或日志系统，也不获得数据库重建权限。

源码开发可运行 `cargo run -p nomifun-app --bin nomicore -- tool-doctor --verify`。
未内嵌 OfficeCLI 的 debug 构建需要它的显式开发路径；Node/Bun 默认使用受管下载。`doctor` 是另一个检查用户 CLI 发现的命令，
其 PATH 发现结果不能替代受管组件准入。

## 首次准备与包体积

统一后端启动前必须准备 Node。桌面准备窗口观察同一供应任务，显示缓存检查、
下载字节进度、解压与能力验证；可重试错误在后端尚未创建时提供重试。Web
宿主通过启动诊断输出同样的阶段。准备成功后仍建立原有唯一、不可切换的 Node
权威，不引入第二套运行时选择。

Bun 仅在功能明确请求时准备，并发请求共享供应；单个观察者取消不撤销其他
消费者。批准组件身份未变时，跨应用更新复用缓存并可离线使用。全新安装首次
Node 准备需要网络；Office 转换本身不需要 Node/Bun 下载或网络访问。

`GET /api/runtime-tools/{tool}` 观察准备状态，`POST /api/runtime-tools/{tool}/prepare`
立即开始或加入同一任务，两者复用实例 owner 鉴权。只接受 `bun`、`node`、
`officecli`，不接受调用方 URL、路径、版本或全局取消。MCP 测试与终端启动在
业务 RPC 前准备 Bun；未准备的直接请求返回 `COMPONENT_PREPARATION_REQUIRED`，
不会先创建终端行或进程。

只供应当前目标架构。提高压缩率、删除源文件不能代替从发行载荷移除 Node/Bun。
体积验收须使用同一 release profile 和压缩格式实际构建 App、DMG 对比，不能把
压缩组件大小直接当成安装包增量。

## Office 请求与缓存

PPT、Word、Excel 共用一个 API，内置预览支持 `.pptx`、`.docx`、`.xlsx`。
其他格式明确返回不支持，可使用系统打开或下载。

| 操作 | 接口 | 契约 |
| --- | --- | --- |
| 启动 | `POST /api/office-preview/start` | `{request_id,file_path,workspace?,doc_type}`；`request_id` 为 UUIDv7，`doc_type` 为 `ppt` / `word` / `excel` |
| 状态 | `GET /api/office-preview/status/{request_id}` | 仅请求所属用户可读取 |
| 停止 | `POST /api/office-preview/stop` | `{request_id}`；在 start 到达前也能记录取消 |
| 产物 | `GET /api/office-preview-artifacts/{capability}/?request_id=...&nonce=...` | 活跃 capability、匹配请求及 UUIDv7 nonce；仅返回该租约的 HTML |

Start 立即创建用户作用域的请求租约。状态为 `queued`、`rendering`、`ready`、
`error`；ready 返回根 capability URL，error 返回 `{code,message,retryable}`。
前端只轮询自身尚未完成的请求。取消按请求 ID 执行，不依赖成功响应才获得的
capability；有界 tombstone 覆盖 stop 先于 start 的竞态。

后端使用稳定的有界文件快照，验证 Office ZIP 结构与资源限制，再以内容摘要、
引擎版本、引擎摘要和渲染参数组成缓存键。多个租约可共享一个渲染任务；取消
单个租约不影响其他消费者，无消费者时停止任务。渲染并发有界；租约、缓存字节、
缓存条目和进程时间也有界。已发布的活跃产物受固定保护，缓存按访问顺序淘汰。
每次新请求都执行源路径授权，缓存命中不能绕过它。

渲染执行批准引擎的 `view <文件> html` 并关闭自动 resident。进程退出、超时、
取消和输出过大均走统一回收，stderr 保持有界诊断。成功产物经校验后原子发布；
HTTP 读取有界并流式发送，避免慢客户端复制整份大型 HTML。

## 显示与更新

产物是带 capability 的只读资源，不是上游端口代理。产物路由依赖随机租约
capability，不要求 iframe 发送 SPA 会话凭据；启动、状态和停止仍要求鉴权。
反向代理必须保留路径和 `request_id` / `nonce` 查询参数，不扩展成任意文件或
任意 `/api/*` 的公共访问。读取会重查源路径可读性与租约是否活跃。

iframe 使用 `sandbox="allow-scripts"`，CSP 禁止网络连接、外部资源、表单与
嵌套 frame。首屏脚本等待字体、图片和两次 animation frame 后发送消息；renderer
校验 iframe source、opaque origin、request ID、capability 和 nonce。
HTTP ready 或 iframe load 不能单独证明首屏已显示。未确认的候选 frame 设为 inert。

已有文件事件与活动标签 metadata 轮询推进 source revision。更新期间保留旧
iframe 与租约，新首屏确认后替换并释放旧租约；失败保留上一版。PPT / Excel
使用真实内部滚动容器，Word 使用文档滚动；恢复位置消息采用同样身份验证，
Excel 工作表编号额外限制为有界整数。自动打开合并文件创建和写入事件，文件
完整性以后端快照为准。只有 `retryable` 错误提供重试；系统打开与下载一直保留。

当前实现与行为测试位于
[`nomifun-office`](../../crates/backend/nomifun-office/)、
[`OfficeArtifactViewer`](../../ui/src/renderer/pages/conversation/Preview/components/viewers/OfficeArtifactViewer.tsx)
及其相邻测试。被替代的 watch 服务、端口探测、代理和按文档类型的启动接口
不再是当前设计输入。
