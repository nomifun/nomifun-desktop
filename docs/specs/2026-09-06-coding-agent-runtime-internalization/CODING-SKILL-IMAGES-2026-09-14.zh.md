# Coding：冻结 Skill 图片资源（未验证）

任务 CAR-06 / CAR-07。分支 `rf/agent-capability-platform-v2`，没有 commit/push。
按用户要求未运行构建、测试、评测、Provider 或外部服务调用，仅源码阅读及局部 rustfmt。
Nomi build：`host9`；Coding build：`host2-coding-loop21`。这是实现状态，不是验收证明。

## 缺口与参考

原 coding_skills 对全部资源调用 String::from_utf8，导致含图片参考的 Skill 无法加载。
本轮阅读本地 Codex `codex-rs/core/src/tools/handlers/view_image.rs`，借鉴其模型图像能力
准入、图片处理与真实多模态结果的分层，没有引入 Codex 的路径授权或执行环境系统。
本切片只处理已冻结、由用户选择的 Skill 制品资源，不是任意工作区 view_image 工具。

## 实现

- CodingContextResource 使用 Text/Image 显式内容类型。index 标明资源种类、大小和
  image_input_available，不把 base64 当作可供语言模型理解的文本。
- 宿主对 PNG/JPG/JPEG/WebP 扩展名走图片路径；文件必须属于精确制品清单，长度和
  SHA-256 都一致，不能以另一个路径或未验证字节替代。仍检查常规文件和制品根边界。
- 在已有 blocking task 内读取、验证一次，再把这些字节交给共享 prepare_image_resource；
  后者不打开路径，也不自行建立文件权限。输入格式必须与扩展名匹配。
- 复用平台图片处理的维度/像素/解码内存上限，缩放最长边至 1568，重新编码 PNG/JPEG
  并去除元数据。WebP 输入输出为标准 JPEG；图片并非原始分辨率/原始编码的保证。
- 每 Session 最多 4 张，单源文件最多 4MiB；解码后每张 base64 最多 2MiB，总共最多 8MiB。
  共享 decoder 还限制单张编码字节不超过 1500KiB、40MP/16384px 和解码分配 192MiB。
  文本仍独立限制每项 256KiB、总计 512KiB；全部资源总项数仍不超过 64。
- read_context_resource 读取图片时必须单调用、不给 offset/limit，返回来源描述和
  ChatToolResultPart::Image。文字读取维持 UTF-8 byte offset、next_offset、eof 分页协议。
- 未启用视觉权限或 exact primary model 未声明 ImageInput，图片读取返回可恢复错误，
  明确没有提供 pixels；不会导致其他合法文本资源不能使用。
- on-demand activation 成功持久化之后，host 返回新的 context_image_input；引擎更新
  资源索引，清除旧 provider parent，并在下次模型轮使用。相同 generation 改权限、
  或 activation 撤回已启用视图均为契约错误。图片选择本身不是 llm.vision 授权。

## 平台、历史与恢复边界

图像源的读取/校验/解码归宿主，Coding core 只处理不可变资源和上下文策略，未直接使用
文件系统或 provider。对社区 Engine 没有强制采用 Coding 的内部资源工具。
共享图片字节助手可由其他编译期宿主使用，但其调用者必须先完成真实资源授权。

Broker 每次请求仍根据真实图像内容复核模型特性，不能以 host 视图绕过；若模型配置后来
失效，不静默去除图片伪装为成功。原有图像 token reserve 与整个上下文大小上限保留。
真实 tool result 在本轮模型上下文中可用；持久历史只保存有界来源描述和图片省略标记，
compaction 不把图片二进制当文本摘要。索引提示模型需要内容时重新读取精确资源 ID。
不自动重放工作区工具，也不新增效果 owner 或 Session。

两个官方 build 均递增，Nomi 摘要补入共享 model_attachments.rs；Coding 已含该文件及
资源/激活/宿主代码。旧 Session 不热迁移，不能用新代码的恢复假设替换旧 exact build。
CodingContextResource 和 CodingCapabilityView 为编译期源码 API，本轮字段变化必须在
自定义 Coding host 中同步；资源构造从 text 改为 content: CodingContextContent::Text。

## 未完成

不支持非制品 Skill 库、任意工作区图片路径、PDF/音频/压缩包/其他二进制读取或脚本执行。
这些不能因为 image variant 存在就标为完成。协议/生态生命周期、安全 checkpoint、
跨启动进程证明、复杂指令作用域和语义完成核对仍需推进。

没有添加或执行本切片测试；后续需覆盖制品摘要/格式不匹配、解码炸弹、资源预算、
无视觉权限、模型不支持、成功/失败/重复激活、图片 batch/paging 拒绝、真实 Provider
多模态消费、历史省略与重读。此前文本资源或附件测试不构成本功能验收证据。
