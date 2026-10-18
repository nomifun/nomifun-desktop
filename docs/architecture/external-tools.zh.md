# 外部工具与 Office 附件预览

本文描述当前实现。组件清单是
[`nomifun-runtime/components.json`](../../crates/backend/nomifun-runtime/components.json)，
本地解析与显式准备 API 位于
[`nomifun-runtime/src/managed.rs`](../../crates/backend/nomifun-runtime/src/managed.rs)，
每次启动的命令与工具环境位于
[`environment.rs`](../../crates/backend/nomifun-runtime/src/environment.rs)。
构建制品、受管缓存准入、诊断与内置 Office 技能使用同一批准组件身份。

## 组件归属与交付

| 组件 | 批准版本 | 来源 | 使用者 |
| --- | --- | --- | --- |
| Bun | 1.3.13 | `oven-sh/bun` 的固定 release | 显式要求 Bun 的任务 |
| Node | 22.22.0 | `nodejs.org/dist` 的固定 release | Agent、Terminal、MCP 与组件调用；完整 Node/npm/npx 发行目录 |
| OfficeCLI | 1.0.156 | `iOfficeAI/OfficeCLI` 的固定 release；官方 npm 身份为 `@officecli/officecli` | Office 只读预览与文档技能 |

表中版本来自组件清单。清单同时记录目标平台、架构、下载地址、上游摘要来源、
制品 SHA-256、交付方式与开发版本范围。制品摘要用于组件下载校验；解包后可执行文件的
摘要用于运行时缓存和 `VerifiedTool.digest`，两者不一定相同。

Release 构建仅内嵌 OfficeCLI；可选 Node/Bun 下载使用同一清单，不进入应用
载荷，也不是启动前提。Debug 构建可用 `NOMIFUN_EMBED_OFFICECLI=1` 内嵌 OfficeCLI。缺少当前
目标的批准制品时构建失败。构建仍校验批准的 Node/Bun 归档并导出可执行摘要，
仅嵌入小型身份元数据。旧内嵌交付留下的同版本、同摘要可执行缓存可直接校验并
离线复用，不信任缓存 stamp，也不要求重新下载归档。
Docker 构建镜像的标签与 digest 也在组件清单中固定。
组件版本变更应同时更新制品、探测、测试和对应的第三方说明；没有运行期间的
全局 npm 自动安装或 `@latest` 升级。

组件许可证与 notice 的版本和来源由
[`notices/manifest.json`](../../crates/backend/nomifun-runtime/notices/manifest.json)
声明，随组件交付。可选 `nfagent`、ONNX 原生库与浏览器保持各自的领域供应机制，
清单中的 `domain_components` 指向各自的权威来源，不建立第二套下载账本。
NomiRelay 的连接、重启与恢复在需要时自动准备批准的 nfagent，保留真实进度、取消、摘要校验与原子发布。
DIY 抠图模型是已确认的产品例外：App 首次使用后，可由已认证的 App runtime
在后台静默安装，不阻塞应用启动；模型缺失或安装失败时保留快速抠图后备。
抠图模型继续使用自己的供应机制，不加入受管工具清单。其批准身份由
[`matting-model.json`](../../crates/backend/nomifun-companion/matting-model.json)
统一固定来源 revision、文件长度和 SHA-256。后端是唯一持久缓存所有者，每次读取都
校验并返回同一份字节；损坏缓存重新获取，下载按块限制长度、验证摘要并原子发布。
连接、响应头和读空闲分别有期限，持续有进展的慢下载不会因总时限被截断。

Renderer 经统一 HTTP 桥获取模型 Blob：桌面跨源使用 local-trust header，WebUI
使用同源 Cookie，凭据模式统一为 `same-origin`。请求携带批准摘要，后端在版本不匹配时
直接拒绝；响应禁用 HTTP 持久缓存。Renderer 仅复用当前进程中的 Blob，显式交给
抠图 Worker，不依赖 Cache Storage 或安全上下文中的 Web Crypto。Worker 只负责推理，
不下载模型。推理超时由主线程终止 Worker 后重新解码图片并执行快速抠图；失败可重试，
录入页区分模型准备、推理失败、推理超时和图片主体识别失败，并始终保留使用原图的入口。

真实浏览器验证入口是
[`matting-browser.mjs`](../../scripts/validation/matting-browser.mjs)：通过环境变量提供
批准模型与 Playwright，分别验证桌面跨源认证、WebUI 同源 Cookie、禁用 Cache Storage
以及实际 MODNet/WASM 推理，不访问用户应用的数据或启动用户应用。
用户配置的 MCP 与终端 CLI 保留用户选择，发现失败必须与成功的空配置区分。

## 供应、身份与执行环境

既有制品解析入口返回 `VerifiedTool`，包含已验证的绝对路径、实际版本与可执行
文件摘要。只读可用性与诊断只检查本地制品；实际启动缺少必需运行时时自动准备批准的平台制品。
OfficeCLI 从内嵌载荷本地解压。受管供应使用
`<data_dir>/runtime`、进程间锁、有界临时文件、摘要验证和原子发布。
下载组件的可执行摘要由已校验的批准制品导出，不单独信任缓存 stamp。独立库调用未初始化数据根时使用平台缓存目录。
不要猜测某台机器的路径；读取 resolver 返回值或诊断的 `executable` 字段。

准入会执行真实能力探测。Node 必须报告 Node 引擎且没有 `process.versions.bun`；
Bun 必须报告 Bun 引擎。将 Bun 命名为 `node`，或让它报告兼容的 Node 版本，
都不能通过 Node 准入。OfficeCLI 在私有临时目录创建 DOCX 并生成 HTML，确认
所需协议；单独的 `--version` 或文件存在不足以证明它可用。

成功准入在进程内复用，失败可重试。准入后可执行文件尺寸或修改时间变化会拒绝继续使用。
Node 使用完整官方发行目录，包含 npm、npx、模块和启动器；不创建 Bun-as-Node 别名，也不修改系统 PATH。

`nomifun-runtime::prepare_process_launch` 是 Agent 原生进程、Terminal 与 MCP stdio 共用的
命令准备和每次启动环境入口。明确 Shell 文本保留 Shell 语义并取得同一环境，原生程序保留 literal argv。
每次启动组合调用方环境、当前持久用户 CLI 目录与已验证运行时，不改写多线程宿主的全局环境。
显式 PATH 与已有可用工具链优先，保留真实 HOME/USERPROFILE、npm 配置、代理与证书配置。
受管 npm 缺省安装到独立于可替换运行时缓存的持久用户工具目录，用户原有 prefix 仍优先。
运行时升级与 Agent 数据清理不拥有这些已安装工具。托管应用按已识别项目元数据或内建静态预设备齐必需 Node/Bun，自定义 Node 优先，准备等待接入原操作取消；任意 Shell 文本不被改写。

Terminal 保留现有 Windows PowerShell shim 适配。Windows Agent/MCP 的通用 `.cmd`/`.bat`
启动和参数保真仍待平台实现与原生验证，不能将本次 macOS 共享改造当成该项通过。
启动、取消、超时和进程树回收继续由 `nomi-process-runtime` 管理。

Debug 构建允许显式绝对路径覆盖：`NOMIFUN_BUN_PATH`、`NOMIFUN_NODE_PATH`、
`NOMIFUN_OFFICECLI_PATH`。路径会 canonicalize，仍必须通过引擎、能力和清单内
开发版本范围验证。Release 的受管组件准入忽略这些开发覆盖；用户命令发现和
应用托管仍保留各自的系统／自定义路径选择。
开发覆盖属于单次启动环境，不能用来替换正式产品的供应策略。

Agent 的 `workspace.process/exec` 与 `start` 使用显式 `component` 目标调用批准组件，
与普通 `command`、Shell `cmd` 三选一。例如 `component=officecli` 与 literal `args`；
宿主在本次调用中解析、验证程序并固定 `OFFICECLI_NO_AUTO_RESIDENT=1`，拒绝冲突的环境值。
普通程序、Shell 与 Bun 命令发现保持原语义，不把同名全局工具静默改路由。
Skill 编译不再探测组件、保存 ready 状态或让模型搬运可执行路径；已授权工具的短 Host
说明直接展示组件入口。源正文和摘要仍由原 Snapshot 冻结，说明不增加进程或文件权限。
组件缺失、验证失败与准备取消沿原未启动回执准确报告；只有新调用才准备组件，已有 Effect
回执不被当前环境覆盖，未知效果不重放。每次实际启动之前再次核对已准备文件的身份。

## 诊断

```bash
nomicore tool-doctor
nomicore --data-dir /absolute/data-dir tool-doctor --verify
```

此子命令属于 `nomicore`；独立的 `nomifun-web` 宿主 CLI 不接受它。可用
`cargo build --release -p nomifun-app --bin nomicore` 构建，诊断时显式指定
与被检查安装相同的数据根。

第一条只输出组件、领域供应引用与 notice 清单：不解压、不启动探测、不打开
数据库。第二条校验本地可用组件，可能把内嵌 OfficeCLI 解压到所选数据目录的
runtime 缓存、创建私有临时探测文件并启动子进程，不下载缺失的 Node/Bun。
JSON 中的 `observations` 包含 `ready` / `unavailable`、
版本、实际路径、摘要或具体错误；任一组件失败时退出码非零。
两种模式都不初始化 Agent 数据、数据库或日志系统，也不获得数据库重建权限。

源码开发可运行 `cargo run -p nomifun-app --bin nomicore -- tool-doctor --verify`。
未内嵌 OfficeCLI 的 debug 构建需要它的显式开发路径；缺失的受管 Node/Bun
报告为不可用。`doctor` 是另一个检查用户 CLI 发现的命令，
其 PATH 发现结果不能替代受管组件准入。

## 首次准备与包体积

桌面和 Web 后端启动不准备或下载 Node。全新安装无需受管 Node/Bun 即可启动。
终端和 MCP 界面只提交一次执行请求，后端按实际启动目录、命令和子进程 PATH 优先解析用户工具，
缺少必需运行时时在原调用内自动准备，程序启动前完成验证。界面只读观察进度，不增加下载确认，
也不在准备后重放可能已经产生效果的启动请求。真实准备失败保留原因并允许重试。
Office 转换使用内嵌 OfficeCLI，不需要 Node/Bun 下载或网络访问。

`GET /api/runtime-tools/{tool}/availability` 只做本地诊断，不触发下载；
`GET /api/runtime-tools/{tool}` 只读观察准备状态；`POST /api/runtime-tools/{tool}/prepare`
开始或加入同一准备任务，不代表另一层权限决策，接口复用实例 owner 鉴权。
只接受 bun、node、officecli，不接受调用方 URL、路径、版本或全局取消。
共用准备保留下载完整性、临时解包和原子发布；单个观察者取消不撤销其他消费者的准备。

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
