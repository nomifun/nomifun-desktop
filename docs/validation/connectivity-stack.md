# 三产品本机连接验收

`scripts/validation/connectivity-stack.mjs` 使用真实 `nomifun-web`、`nfrelay`、`nfagent` 和 Mobile 的连接存储、协议验证及 HTTP transport，建立可重复运行的隔离闭环。唯一替代业务依赖是本机受控模型，它返回确定的 OpenAI 兼容流式文字；Desktop 的认证、AgentSession、Turn、Event、消息投影与持久化均由正式服务执行。

## 构建与运行

先从当前源码构建三个二进制。Desktop 的 API 模式不需要 UI 构建：

```powershell
cargo build -p nomifun-web
```

在 `nomifun-net-infra` 构建 Relay 和 agent。正常 Relay 构建需要该仓库的控制台静态资源；`noweb` 仅适用于本验收的 API 进程，不证明控制台界面验收：

```powershell
go build -tags noweb -o bin/nfrelay-v2-test.exe ./cmd/nfrelay
go build -o bin/nfagent-v2-test.exe ./cmd/nfagent
```

回到 Desktop 仓库运行。Windows 上自动优先使用 Git 附带的 OpenSSL；其他环境使用 PATH 中的 `openssl`，也可指定 `--openssl-bin`。

```powershell
node --test scripts/validation/connectivity-stack.test.mjs
node scripts/validation/connectivity-stack.mjs --desktop-bin target/debug/nomifun-web.exe --relay-bin ../nomifun-net-infra/bin/nfrelay-v2-test.exe --agent-bin ../nomifun-net-infra/bin/nfagent-v2-test.exe --mobile-root ../nomifun-mobile --output .tmp-connectivity-stack
```

需要 Node.js 24 或更高版本。Runner 使用 Node 原生 HTTP、HTTPS 和 WebSocket upgrade；它加载 Mobile 的实际 TypeScript 模块，只为 Metro 的相对无扩展名导入补充 `.ts` 解析，不转换或复制 Mobile 实现。

Desktop 专属中继 listener 端口 `127.0.0.1:25811` 必须空闲。Runner 只探测占用，冲突时失败，不终止端口所有者。其余监听使用空闲环回端口。三个进程的数据库、缓存、日志、工作区和代理身份都放在本次新建的临时目录。正式服务默认要求认证，未使用 `--insecure-no-auth`。

HTTP 业务路径为 Mobile transport → 真实 Relay 共享 HTTPS 网关 → 真实 QUIC 承载 → nfagent → Desktop。WebSocket 使用相同路径，客户端直接用设备凭据完成升级，没有浏览器 cookie。HTTPS 证书由本次运行生成，通过 Relay 正式证书加载入口写入本次隔离目录；客户端只显式信任该证书，未关闭证书或主机名校验。Runner 还确认不提供此信任时连接失败。nfagent 独立验证同一真实 Relay 的 SPKI pin。仅丢回执和迟到响应场景经过故障注入代理，该代理也严格验证 Relay 上游证书。

共享 TLS 业务 origin 使用 `https://localhost:<port>`，让客户端按标准发送匹配的 SNI；证书包含 `localhost` SAN。监听地址仍限制在 `127.0.0.1`。不能用 IP origin 导致的无 SNI 握手失败来放宽证书验证或开启无默认路由的 catch-all。

同时覆盖 Desktop 原生连接管理与重启恢复时，构建正式模块的 headless harness：

```powershell
cargo build -p nomifun-desktop --example connection_lifecycle_harness --no-default-features
node scripts/validation/connectivity-stack.mjs --desktop-bin target/debug/nomifun-web.exe --relay-bin ../nomifun-net-infra/bin/nfrelay-v2-test.exe --agent-bin ../nomifun-net-infra/bin/nfagent-v2-test.exe --native-harness-bin target/debug/examples/connection_lifecycle_harness.exe --native-spa-dir ui/dist --output .tmp-connectivity-stack
```

Native harness 直接包含正式 `RelayPairingManager` 模块，使用正式 DesktopServer、ManagedChildProcess、专属 listener 和持久凭据存储。它按仓库内 `nfagent-runtime.json` 下载并验证固定版本。若已有同一官方工件，可加 `--managed-agent-bin`；Runner 先核 SHA-256 与当前平台 manifest 精确匹配，再复制到正式 managed cache，并由产品再次验证。编译的开发 nfagent 不满足官方 manifest 时不能作为这个参数使用；没有路径、URL 或 SHA 环境变量覆盖。Native 部分另外使用同 Relay 的开发环回 HTTP 端口，保持 LAN 关闭。

调查 native 生命周期故障时可使用 `--native-only`，只运行真实 Relay、正式 manager 和官方固定 SHA 工件，默认输出到独立 `.tmp-connectivity-stack-native-only`。可复制已经验证的官方可执行文件，始终新建 Desktop 数据、代理身份与 Relay 数据。该报告标记 `diagnostic_subset=true`；交付验收必须再次运行完整流程。RPC 超时只报告字节、行、数字响应 ID 和匹配计数，不保存原始 stdout、本机 trust 或配对内容。

## 判定项

报告 `report.json` 记录三个仓库的源码基线与工作区修改状态、二进制路径与 SHA-256、检查耗时、状态、必要身份和证据。`sources.*.commit` 是基线提交；工作区有修改时，不能把它视为该提交原始代码的验收。应结合当前 diff 和报告内的二进制哈希核对本次交付。以下项目必须全部通过：

1. Desktop 业务端点拒绝匿名请求。
2. Relay 同一 bootstrap attempt 可恢复原结果，不重复创建隧道；不同 attempt 被拒绝。
3. 真实网络代理使业务 HTTP 可达，HTTPS 验证严格生效。
4. 两张短时配对码分别兑换为独立设备，消费后的重复兑换失败，状态为 `used`。
5. Mobile 正式协议、系统世代验证和两条认证 WSS 连接成功。
6. Mobile 提交 canonical Session 与文字 Turn，真实 WSS 收到模型流，重新读取正式历史得到完整回复。
7. 撤销设备 A 后，其 HTTP、续期和重连失败，已经建立的 WSS 关闭；设备 B 的 HTTP 与 WSS 仍可用。
8. 丢弃已提交 Turn 的 HTTP 回执，Mobile 不自动重发：代理请求数、模型调用数和持久用户消息均只增加一次。
9. 终止本次拥有的 nfagent 后，业务入口不可用，WSS 关闭，Relay 不宣称传输就绪；同一代理身份重新启动后 HTTP、WSS 和历史恢复。
10. 第二个真实隔离 Desktop 有独立身份和存储世代，Mobile 缓存 namespace 区分世代；切换时迟到的旧目标响应被拒绝，新目标不出现旧 Session。
11. 用本连接的窄权限令牌撤销中继连接后，对应隧道被移除。
12. 本次拥有的进程全部结束，默认临时目录全部清除。清除前检查绝对路径位于创建时的临时父目录内，并验证本次随机 ownership marker。

密码接入也必须生成独立设备凭据；该设备不能管理其他设备或签发配对码，自行撤销后其他手机仍保持可用。配对还通过 Mobile 正式链接解析器执行，不兼容协议请求不能消费一次性票据。

正常路径要求 Relay 的正式状态 API 确认 `L0/QUIC`。UDP 故障场景使用本次拥有的丢包 sink，确认发生过 QUIC 尝试，再要求同一代理身份通过实际共享 TCP listener 回退为 `L2/TLS:443`、`supports_datagram=false`。该路径需要重新完成设备 HTTP、WSS 与 canonical 文字 Turn，不能只凭代理进程存活判断回退成功。

过期恢复场景独立启动短时 Relay：只有同时声明开发环回模式时允许 `-desktop-dev-enrol-ttl 2s`，生产默认期限不变。实际入网凭据到期后，同 attempt bootstrap 返回 `410 enrol_expired`；错误 attempt 无权取消，原 attempt 可通过窄权限取消接口删除资源。重启真实 Relay 后重复取消仍须成功。

提供 native harness 时，额外验证 SHA 固定运行工件、只开中继且 LAN 关闭、消费后的二维码隐藏、显式刷新得到新票据、暂停后重启进程仍保持暂停、恢复时复用原隧道、解除接入时删除远端隧道与本地配对/代理身份。配对码只在显式请求时生成；连接或重启中继不会生成配对码。还实际丢弃成功 bootstrap 回执，等待入网凭据到期，然后重启 Harness；正式 manager 必须保留不确定接入意图、用原 attempt 恢复而不重复创建代理或隧道，并能解除该不确定接入。控制面不可达时，关闭必须在 15 秒内完成，解除接入可被抢占；未确认的撤销保留本机状态，仅在本机忘记不接触中继且把中继端连接留给 Relay Console 撤销。受保护的中继状态不可读时不阻塞启动，状态报告类型化错误并可仅在本机忘记；操作失败原因以 `actionErrorCode` 与状态级 `errorCode` 分开报告。接入失败且没有托管网络代理端运行时，专属 listener 随即关闭；已有中继连接时再次提交接入配对串会被拒绝（`RELAY_ALREADY_PAIRED`），且不改变连接状态。Harness 的本机 trust 和二维码只在私有 stdio RPC 中使用，不保存到报告或 stdout 日志，也不解密或读取 DPAPI 暂存凭据。

Runner 从不自动重试变更请求。GET 就绪探测与显式同 attempt bootstrap 恢复有自己的用途；它们不能解释成不确定业务 POST 可重放。报告与进程日志不包含 JWT、配对码、入网凭据或管理员密码。不要把临时目录中的数据库、证书私钥和网络代理身份当作公开报告附件。

## 证据边界

这是同电脑、真实协议和正式持久化路径的集成验收。它能证明跨网中继的代码路径可通，并不衡量外部路由、NAT、防火墙、公网 UDP、带宽、容量或 HA。TLS 证据覆盖严格客户端验证与 HTTPS/WSS 转发，生产 ACME 签发、续期和域名部署需其自身证据。

默认使用 Web host 承载 Desktop 的共享后端。带 native harness 时额外覆盖正式进程托管与连接持久化生命周期。两种模式都不覆盖 Tauri 窗口入口、原生相机、系统 App 深链、Android/iOS 真机或 H5 UI 操作。界面验收应与本报告并列，不能互相替代。

`--keep-fixture` 仅用于失败调查，仍然结束进程；会保留本次隔离目录及其中的合成凭据，报告明确给出目录。下一次运行总是新建目录，不复用上次状态。

## 交付报告模板

```text
源码：Desktop <commit + dirty scope> / Relay <commit + dirty scope> / Mobile <commit + dirty scope>
二进制：report.json 中的三个 SHA-256
命令：<完整 runner 命令；不包含密码或 token>
时间：<Asia/Shanghai>
结果：<pass / fail>，<通过数量>/<总数量>
真实链路：<HTTP、WSS、QUIC、认证与 canonical Turn 证据>
恢复与撤销：<断链恢复、独立设备撤销、Relay 清理证据>
目标与世代：<两 Desktop 身份、存储世代、迟到响应拒绝>
清理：processes_cleaned=<bool>，fixture_removed=<bool>
UI 与 native 的独立证据：<链接或明确未覆盖>
仍未覆盖：<公网部署、真机、ACME、容量与 HA 等实际边界>
```
