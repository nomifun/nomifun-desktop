# R28–R30：资产、输出压缩与协议模块

## 范围与原则

按用户要求转向全局覆盖与局部简化，不再扩展 SSH 依赖层设计。本批完整阅读
nomifun-assets、nomi-compact、nomi-protocol 的生产文件、现有测试与 Cargo 清单；
核对 App 资产挂载、UI 固定 logo URL、Agent 工具结果压缩/提示注入及 CLI 协议消费入口。
调用方仅覆盖这些行为，不代表 App、Agent 或 CLI 整个模块已完成。

## R28-01：固定 URL 缓存与无状态包装

- 固定 logo URL 原先声明一年 immutable；改为 public, no-cache，允许存储但使用前需 ETag 验证。
- GET 条件请求按弱比较识别 W/ 标签，合并判断全部 If-None-Match 请求头。
- 合并 200/304 公共响应头；删除只有 Arc<AssetService> 的 state.rs，路由直接调用无状态服务。
- 调用方、测试和导出同步修改；全仓无 AssetRouterState 剩余引用。
- 不新增资产缓存或版本框架，保留现有散列和 MIME 实现。路径穿越、缺失文件、公有访问仍测试覆盖。
- 已发送给旧客户端的一年缓存无法由新响应追溯撤销；本次修复约束后续获取的缓存策略。

验证：基线 8/0；新增缓存策略和弱/重复 ETag 两项回归旧实现失败，修复后单元 10/0。
App content_e2e_suite 的 assets_e2e 过滤测试 2/0（依赖编译约三分钟，测试约四秒）。
SVG 中脚本、javascript:、onload/onerror 和 foreignObject 搜索未命中；这不是全部 SVG 内容的逐元素审计。

## R29：压缩不得无意破坏数据

- R29-01：CRLF 曾把每行内容变成空串，leading newline 也会丢失。
  CRLF 与进度回车分开处理；合并行尾空白/空行清理，删除只被该链调用的 trim_trailing_whitespace。
- R29-02：长对象的键未转义，生成非法 JSON；改用 serde 字符串编码。
  Full 模式先识别 JSON，再处理普通日志前缀，不将已识别 JSON 字段折叠；后缀原样保留，避免第二个数据块被折叠。
  保留短对象内联与两空格缩进，只有更短时替换 JSON 本体。
- R29-03：TOON 将空串、true/null/数字字符串输出成其他类型；键中的逗号破坏列数，反斜杠/控制字符未正确转义。
  字段名和值共用小型引用函数，复用 serde 转义；删除手写括号计数，使用 JSON 流解析器定位块末尾，并保留外围文本。
- R29-04：相似行比较以字符数除以字节数，导致中文完全相同也不折叠；统一为字符计数。

验证：旧实现 47 个既有测试通过，新增 7 项中 6 项明确失败，修复后 54/0。
JSON Full 首个测试的键相似度不足，原实现通过；加强测试后排队任务取消，未记录为红→绿。
最终该测试及第二 JSON 块保护通过。Agent output_compaction_test 8/0（真实工具执行与模拟 provider，无外部 LLM）。

范围限制：Full 行折叠本来就是有损、启发式功能；JSON/TOON 只尝试第一个候选块，
不是任意日志的语法分析器。Safe 保留现有有限 ANSI 清理范围，不承诺完整终端仿真。
本批没有改变等级配置、外部协议或增加依赖。

## R30-01：协议 I/O 边界与重复缓冲

- stdin 原来无界读行并向无界队列发送。改用现有 Tokio 有界队列（8 条）及逐行 16 MiB 上限（包含换行）；
  超限关闭输入，给历史导入保留空间但不继续累积异常输入。消费者速度直接形成背压。
- 接收方关闭时 async 读取循环退出；同时保留 Tokio stdin 的 OS 阻塞读不可取消这一限制，不声称它已被回收。
- JSON 解析错误只记录分类/行列，不再输出可能包含用户字段值的 serde 文本诊断。
- 删除 ProtocolWriter 外层 Mutex/BufWriter，持有标准 stdout 共享锁直至整帧及 flush 完成，避免重复缓冲与锁。
  保留同步 emitter 和显式 I/O Result，不引入发送任务或队列。
- 命令/事件结构及未知字段兼容规则不变；CLI 仅 recv 调用，无显式 UnboundedReceiver 类型依赖。

验证：队列不背压、接收方 Drop 后读循环不退出、超长行仍继续接受下一命令，三项旧实现失败。
共增加六项读取回归，覆盖顺序/空行/非法命令/CRLF/无终止换行、无 EOF 的持续输入和精确上限；最终协议模块 49/0。
消费方 cargo check -p nomi-cli 通过（R30 完成时）；后续 R31/R32 的验证另见对应报告。
第一次 writer 简化编译残留 MutexGuard 解引用，已修正为 StdoutLock 直接借用后重跑通过。
初版超限测试只检查有限输入读完，不能证明拒绝；已替换成下一命令不可继续处理的回归，并重跑取得旧实现失败证据。

## R30-02：CLI 调用方续审，尚未修改

已阅读 main.rs 的 run_json_stream_mode：

- pre-message 阶段 Stop 直接 return，跳过末尾统一 engine/MCP shutdown。
- 活跃请求中 select 使用 Some(sub_cmd) 模式；输入 EOF 后该分支失效，执行不会因 EOF 取消。
- PendingConfig 用整包替换，连续部分更新可能丢前一包字段，需核对约定并补回归。
- 多处 emitter 错误被忽略，需与 OutputSink 错误契约一起核对。

这些是后续 CLI/Agent 调用方工作，不计入协议库已验证范围。尚未运行真实 CLI 进程退出/故障输出实验。

## 交付与未执行

所有改动保留 dirty，未提交。state.rs 的删除可从 Git 恢复；未触碰 .githooks。
未重复无改动的 UI 全量测试，未跑全 Rust workspace、桌面打包或原生 Linux/macOS。
后续从总台账接续，不能将本批三个模块完成当作全局审计完成。
