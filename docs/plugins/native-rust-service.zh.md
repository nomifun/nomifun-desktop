# Rust 原生 Service 插件开发与验收

Rust 插件是同一 Plugin Product 的**原生 Service 执行后端**，不是第二套插件平台，也不把 Rust `dylib`/trait object 加载进宿主。JS 与 Rust 共用 Release、Catalog、capability 选择、Service Host、调用授权、流式背压、取消和进程树所有权。

## 信任与启用

原生插件具有宿主操作系统账户的进程权限。进程隔离可以隔离崩溃、回收进程树，**不是文件/网络/凭据安全沙箱**。宿主的 capability 授权检查仍保护宿主接口，但不能限制原生程序自行访问操作系统。

默认禁止原生执行。宿主管理员在启动 NomiFun 前显式设置：

```powershell
$env:NOMIFUN_ALLOW_NATIVE_PLUGINS = '1'
# 然后从此环境启动宿主；不要给未知插件开启此选项。
```

当前是宿主级信任开关，不是按发布者签名授信，也不是每个插件的权限沙箱。启动子进程时清除环境，仅保留 Windows 系统目录及临时目录变量；这减少意外传递 API Key，不能阻止可信原生程序读取本机其他数据。不要在普通 action payload 中传宿主密钥。

## 编写插件

SDK 位于 `crates/backend/nomifun-plugin-sdk`，可运行示例为 `examples/echo.rs`。实现 `Service::invoke`，再调用 `serve`：

```rust
use nomifun_plugin_sdk::{async_trait, CallContext, Service};
use serde_json::Value;

struct MyService;

#[async_trait]
impl Service for MyService {
    async fn invoke(&self, method: &str, payload: Value, ctx: &mut CallContext)
        -> Result<Value, String>
    {
        match method {
            "plugin.my-service.echo" => Ok(payload),
            "stream" => {
                ctx.emit(payload).await?;
                Ok(Value::Null)
            }
            _ => Err("unknown method".into()),
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    nomifun_plugin_sdk::serve(MyService).await
}
```

- stdout 专用于 NDJSON 协议，日志写 stderr。SDK 处理启动握手和精确 release/generation/runtime 身份。
- `emit(...).await` 等待宿主 ACK，每个调用最多一个未确认事件；仅流式调用可用。成功返回必须发生在事件已完成之后。
- `ctx.storage("kv", payload)`、`database_query`、`database_execute`、`database_batch` 使用已有宿主管理的存储接口；文件目录来自 `ctx.host.storage`。SDK 没有新建数据库或全局存储注册表。
- 取消会丢弃调用 future；长任务应主动让出执行。不能把调用任务私自 detach；阻塞/不合作代码最终由宿主超时和进程树终止处理。不要声称任意原生副作用可以被撤销。
- 单帧上限 1 MiB、最多 128 个在途 SDK 调用、16 帧写队列；宿主另有自身的准入和容量限制。写入由单一任务完成，取消调用不会截断其他调用的协议帧。
- 当前 SDK 的 `start`/`stop` 用于初始化/释放；受管存储 RPC 通过 invocation 的 `CallContext` 发起。不承诺启动期的同等存储便利接口。

## 构建、打包与导入

宿主只导入已编译制品，不替第三方运行 Cargo/build.rs。开发者在自己的构建环境编译。每个 Release 对应一个明确目标，不扫描 PATH 寻找 Rust、Node 或插件程序。

Windows 本机演示：

```powershell
cargo build -p nomifun-plugin-sdk --example echo
cargo run -p nomifun-plugin-platform --example package_native_echo -- `
  target/debug/examples/echo.exe .tmp-rust-echo-release
```

输出目录必须尚不存在。示例打包器打印可导入的 `bundle/release` 路径和 artifact digest；在现有 Plugin 的预编译制品导入入口使用它，然后 Test、Publish、Enable，在 Agent capability 选择中选择 `plugin.rust-echo`。它以独立 ID 发布，不覆盖系统内置 ID。这个示例是最小 Tool capability；按目标的组件合同修改其 capability 声明，不能仅改 ID 就成为模型/调度器实现。

打包器源码是 `nomifun-plugin-platform/examples/package_native_echo.rs`。它调用 `build_plugin_native_bundle`、现有 Release Store 和 Share Bundle 导出；可复用此流程打包自定义 Service。示例锁/依赖摘要仅为示例标记，正式发布者应绑定真实 Cargo.lock、依赖清单和构建配方，不把示例摘要当供应链证明。可选 UI 仍通过同一 Release 的 `ui/**` 交付。

Service Manifest 的新增部分为：

```json
{
  "execution": { "kind": "native", "target": "x86_64-pc-windows-msvc" },
  "entrypoint": "service/plugin.exe"
}
```

这是完整 Service descriptor 的片段，摘要、生命周期及协议字段由打包器填充。旧 JS descriptor 不序列化 `execution`，读取时默认 Node，所以旧 Release 的 canonical digest 不因新增字段而改变。

目标枚举包含 Windows MSVC、Linux GNU、macOS 的 x64/arm64。Windows 固定入口为 `service/plugin.exe`，其他为 `service/plugin`；宿主目标不匹配时拒绝，不模拟兼容。**目标枚举不是跨平台运行验收**，各发布目标还需本机验证；Linux 的 libc/动态库兼容性也不能仅由 target triple 保证。本批实测 Windows x64。不要把动态依赖放进未声明的 `service/**` 附带文件：当前制品只接受一个原生可执行入口及可选 UI，优先自包含构建。

沿用制品默认大小限制：单文件 64 MiB、Release 总计 256 MiB。原生装载另有 256 MiB 的防御性上限，不会绕过导入的更小限制。

Test Host 目前验证启动，不自动生成任意 action 的业务输入。声明了 contributions 的插件可能显示 `NeedsTestInput`；发布时要显式确认该警告。业务验收必须实际调用 capability，不能把启动成功当成全部动作正确。

## 可复现验证

两个真实进程测试默认标注 ignored，以免假设已存在某个编译产物；使用以下命令必须显式执行，缺失路径会失败，不会静默跳过：

```powershell
cargo build -p nomifun-plugin-sdk --example echo
$env:NOMIFUN_NATIVE_TEST_EXECUTABLE = (Resolve-Path target/debug/examples/echo.exe).Path
cargo test -p nomifun-plugin-platform --test native_service --test service_application -- --include-ignored
cargo test -p nomifun-plugin-platform --test service_process --test service_storage_ipc
cargo test -p nomifun-agent-contracts --lib native
```

Linux/macOS 调整 executable 路径（去掉 `.exe`）及环境变量设置方式。测试用运行时显式允许原生执行，同时注入“一旦访问 Node 就 panic”的 authority，验证原生链路没有 Node 隐式依赖。测试覆盖当前进程和 Product 链路，不代表所有 Agent 组件已开放。

## 明确的剩余范围

本后端能执行已有组件合同下发布的 Rust Service capability；它不会自动产生尚未存在的模型连接授权、会话资源消费、整套 UI Shell 或调度器替换接缝。按 2026-09-15 上线收敛决定，这些缺口转入长期需求池，不作为本期必须补齐的工作；native 默认关闭，仅作为可信开发者实验能力保留。不另外新增“Rust 专属 Catalog/Compiler/凭据库”来绕过缺口，也不把任意 `Arc<T>`/宿主内存跨进程传递。

当前发布范围、跨 OS TODO 和可复制的目标机验证 prompt 统一见[发布就绪台账](../reviews/2026-09-15-plugin-release-readiness.zh.md)。插件 Agent 页面与 native 执行分别准入：前者需宿主设置 `NOMIFUN_ALLOW_EXPERIMENTAL_AGENT_UI=1`，后者需 `NOMIFUN_ALLOW_NATIVE_PLUGINS=1`，两者都不写入默认安装配置。开关变更后重启宿主；关闭页面实验不删除已保存的页面偏好，也不停止正常内置 Agent 会话。
