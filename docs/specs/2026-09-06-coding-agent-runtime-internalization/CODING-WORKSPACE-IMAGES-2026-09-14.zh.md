# 工作区图片读取与待观察像素保留（2026-09-14，未验证）

任务 CAR-05 / CAR-06 / CAR-07。分支 `rf/agent-capability-platform-v2`，未 commit/push。
本切片只有实现及源码检查，按用户要求未执行构建、测试、E2E、真实 Provider 或外部服务。
不以此前历史通过记录证明本切片可用；整体目标仍未完成。

## 平台读取与共享引擎端口

`fs.read` 新增可选 `format`：省略或 text 时仍走 UTF-8 分页；image 时只接受相对 path
及可选 expected_sha256，不允许 offset／limit（包括 null）。Coding 的模型工具仍叫
read_file，不新增可绕开 Snapshot 的内置原生读盘工具。

平台 `workspace_file_read` 在既有 FileService 工作区 read 授权下读取源字节：

- 支持 PNG／JPG／JPEG／WebP，源文件最多 4 MiB；共用普通文件检查、路径 confinement、
  限量读取、读后元数据／路径检查和源 SHA-256，不从引擎直接读取任意绝对路径。
- 可提供 expected_sha256；源摘要不符时不返回图片，不能把历史图片假称为当前文件。
- 复用共享图片预处理：扩展名与格式匹配、最多 40MP／16384 边长、192 MiB decoder
  allocation 限制、最长模型边 1568、重新编码 PNG／JPEG 移除元数据；输出编码前最多
  1500 KiB，base64 最多 2 MiB。
- 工作区图片读取／解码通路共有 2 个并发 permit；解码时 permit 跟随 blocking task，
  调用 future 被丢弃也不能提前释放正在解码的配额。此配额不宣称覆盖其他所有图片入口。
- 该读法不是 OS 原子快照或写入租约；仍保留平台 path owner 的并发 rename／链接竞态边界。

`EngineKernelSession::install_tools` 在 Kernel invoker 与 EngineToolHost 之间安装
共享 `WorkspaceMediaTools`，官方 Coding 和使用该公共宿主接口的源码引擎可共用：

1. 只有 exact Snapshot 中的 PlatformBuiltin fs.read.invoke、image 请求会进入转换通路；
   不根据任意 Plugin 返回的 kind 字段识别图片，也不打开返回结果里描述的其他路径。
2. 派发前核对当前 active-set generation、active llm.vision，以及 Agent immutable revision
   的 exact primary model route 支持 ImageInput；返回时再核对 generation。
3. Kernel 仍独立核验 Session／principal／action／schema／资源授权后才调用文件 owner。
4. 核对返回 path、源摘要、尺寸预算和媒体格式，再投影为真实 ChatToolResultPart::Image。
5. 转换在 EngineToolHost 日志处理之前发生：持久历史仅保存来源文字及图片省略说明，不把
   base64 作为文本工具结果落盘或交给模型猜图。Model Broker 继续按实际请求核对 ImageInput。

源码自定义宿主若绕过 EngineKernelSession 自行组装低层 Kernel adapter，需自行提供经过
授权的媒体投影／日志策略，不能把内部图片 JSON 当成稳定扩展协议。上述通路不开放运行时
安装、动态库挂载或 Session 热切换引擎。

## Coding 策略与压缩

Coding 在已有 context_image_input 状态基础上，让工作区图片读取只允许单调用 batch；
条件不满足时整批不执行。按需激活仍走平台持久激活合同，不隐式启用 llm.vision，也不会
偷偷更换模型路线。工具描述告知缩放／重新编码和压缩后重读限制。

本次还修复了与 Skill 图片共用的一处压缩缺口：原压缩会将所有工具图片换成文字描述，
可能让刚读取的图片在下一轮模型请求前就丢失。现在识别最新 assistant 工具 batch 中尚无
后续 assistant 回复的图片结果，压缩时保留整个 call/result 配对及像素；已接受 steering
输入仍通过原 retained_inputs 保留。不会只保留图片而制造孤立工具结果。

发起 summary 请求前预检“必留输入／指令／工具 + 待观察图片 batch”预算；如果这些本身
已经放不下，明确失败而不是花费 summary 调用后静默丢图。替换后仍核对整体预算和配对。
此前已经经过后续 assistant 响应的图片仍可在压缩／历史重放时省略，需时重新读取。
这只保证输入保留，不证明 Provider 已成功消费、模型理解正确或图片内容可作为任务成功证据。

## Codex 参考与差异

已查阅本地 `multi/codex/codex-rs/core/src/tools/handlers/view_image.rs`：它在调用开始检查
InputModality::Image，通过 environment filesystem／sandbox 读取文件，并交由集中图片处理
路径组织模型输入。Nomifun 借鉴这些职责分离，不照搬 Codex 的环境选择器或 original-detail
能力。当前仍不提供原分辨率、裁剪、缩放区域、SVG／PDF／GIF 或图片编辑／生成。

## 构建身份和待验证项

Coding `host2-coding-loop25`；Nomi `host11`（共享 fs.read schema／owner 变化）。构建摘要
包含新 owner 和共享 Engine 媒体适配器；不复用旧 exact build 身份。

待验证：授权越界／链接、超大／损坏／伪扩展名图片、摘要不一致、纯文本模型或未激活 vision、
激活后调用、混合 batch 拒绝、真实 Provider 图片工具结果、无 base64 历史、解码取消配额、
压缩前保留新图片配对与预算失败、旧图片压缩后重读。
非制品 Skill 冻结贡献接入、多协议 MCP／生态效果生命周期、安全 checkpoint 续跑、
跨启动进程树证明和任务语义核对等仍需继续实施，不因本切片改变整体状态。
