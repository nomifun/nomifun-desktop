# Coding 工作区文本分页与搜索（2026-09-14，未验证）

任务：CAR-05 / CAR-06。分支 `rf/agent-capability-platform-v2`；未 commit/push。
本切片已写入源码，未运行测试、构建、E2E 或真实服务；不代表 CAR 整体完成。

## 原缺口与实现

此前 `read_file` 经 Wave2 返回整份文本，底层通用文件读取允许较大文件，工具日志／上下文
投影又有独立截断预算。模型可能只看到不完整 JSON，无法可靠继续读取；仓库指令加载也仅
提取 `content`，没有分页完整性合同。

现在 `fs.read` 发布严格 schema，平台 FileService 提供 `read_text_page_for_agent_session`：

- 只通过已授权 AgentSession workspace 的 read 操作解析相对路径；引擎不直接打开文件。
- 单个 UTF-8 源文件最多 8 MiB，元数据预检查后实际读取仍限量，拒绝非普通文件和非 UTF-8；
  原 UI／通用文件 API 不变，不做二进制有损转换。
- `offset`／`limit` 是字节而非行；默认页为 16 KiB，limit 范围 4..16384。
  起止必须处在 UTF-8 边界，返回原文本（不加行号、不转换换行符）、`start_line` 和
  一基 `start_column_bytes`。
- 每页返回完整读取内容的 `sha256`、`total_bytes`、`offset`、`next_offset` 和 `eof`。
  offset > 0 必须提供前次 `expected_sha256`，内容不一致时返回 `FILE_CONTENT_CHANGED`，
  不返回新版本页面；应丢弃原分页序列、从 0 开始重新读取。
- 24 KiB 上限覆盖整份序列化页面的元数据与 JSON 转义，作为工具 text part 再次转义后
  另限 24 KiB，为宿主 call ID 的最坏转义情况／32 KiB 日志结果封套留 8 KiB；必要时缩短内容页，并同步游标，
  非 EOF 页保证前进。不是先返回大结果再从 JSON 中途切断。
- 读取前后检查描述符长度／修改时间，并重新核对路径解析；异常变化不返回页面。
  **摘要仅标识本次读到的字节，不是 OS 原子快照或写入租约**；仍受现有 path owner 的并发
  rename／链接竞态边界约束。每次续读重新读入、计算摘要，不缓存任意文件或新增磁盘副本。
- 不存在的目标只有在路径权限核对后才转为 not-found，悬空链接或权限失败不能冒充文件缺失。

## 搜索接线

相邻 `fs.search` 此前依赖 UI 文件列表缓存并逐份读取大文件，搜索片段固定截取行首，
长行后部的实际命中可能根本不在返回片段里。现已改为 FileService 的新鲜、有界扫描：

- 每次重新扫描指定文件／目录，不使用 UI inventory cache；沿用目录 hidden／ignore 规则，
  不跟随扫描条目的 symlink，不读取父目录／全局 Git ignore 或 Git exclude 配置。
- 最多 20,000 个遍历条目、2,048 次文件尝试、64 层目录、64 MiB 实际源字节读取；
  每文件最多 8 MiB。与分页共享授权读取及摘要代码，读失败也累计已消费字节。
- 文件间检查 5 秒扫描时间预算；这是协作式截止，不承诺中断单次 OS IO／walker 内部操作。
- 单行 literal query 最多 1024 字符／4096 字节，保留有意义的前后空格；最多 200 个匹配行，
  一行返回首个匹配。整份 JSON 结果最多 24 KiB，嵌入工具 text part 后另限 24 KiB，
  超预算前移除放不下的条目。
- 片段围绕真实匹配位置，保留准确行号、字节列和 `byte_offset`、整份来源 `sha256`。
  模型可据此通过 `read_file` 继续读取；长行中部的命中不再只返回无关行首。
- 返回 scanned／skipped 数量、实际源读取量、`incomplete_reasons`；不可读、变化、非 UTF-8、
  超限、symlink 等跳过情形不冒充“文件内无匹配”。所有不完整原因都会令 truncated=true。
  即便没有触及预算，结果也只对应应用了过滤规则的本次扫描，不是全工作区不存在的证明，
  更不是一致性 filesystem snapshot。

## Coding 指令读取

根目录和 scoped `AGENTS.md`／`AGENTS.override.md` 均使用同一 Kernel 工具：

1. 按精确游标／摘要续读，逐页保留既有工具事件和派发记录。
2. 要求所有页的总长度和摘要一致；拒绝错误游标、空的非 EOF 页、文件消失和超限。
3. 单文件仍最多 16 KiB；最多 16 页，超限整体失败，不能将第一页或已拼接前缀当成完整规则。
4. 现有祖先缓存、总指令预算、按需能力授权和效果后刷新规则不变。

源码集成的 Coding host 若自行实现 `read_file`，需返回上述分页字段；不再接受只有
`content` 的模糊完整性返回。此调整是源码接口／工具合同演进，不是运行时动态插件 ABI。

## Codex 借鉴范围

查阅本地 Codex `codex-rs/core/src/tools/handlers/unified_exec.rs` 及
`tools/handlers/unified_exec/exec_command.rs` 的 `max_output_tokens`、`truncation_policy`、
`original_token_count`／`output_omitted_bytes` 处理，借鉴“明确限制并披露输出不完整”的原则。
当前参考仓库没有 `handlers/read_file.rs`，本切片的 UTF-8 游标与 SHA-256 续读合同是针对
Nomifun owner／Kernel／日志边界设计的实现，不宣称从 Codex 移植了现成文件分页工具。

## Skill Library 边界调查与后续

现有 `nomifun-skill-library::materialize_skills_for_agent` 返回可变 `source_path`，沿用旧
CLI symlink 接入；它并未创建 `MaterializedSkill`／贡献锁。当前 materialized Skill 需要
package、target artifact、version、body digest 等确切来源。因此没有把旧目录直接传给引擎，
也没有用按名称搜索目录代替 Agent revision 锁。
非制品 Skill 仍需补齐平台侧导入／冻结／版本绑定，再进入引擎共享的已选贡献路径。
引擎本身仍只能源码集成、随应用构建注册；Skill 数据接入不等于允许打包后挂载引擎。

当前 Coding build：`host2-coding-loop24`；Nomi：`host10`，共同覆盖 Wave2 schema／owner
变化。摘要纳入文本分页／搜索模块及其 path authority／workspace binding 源码。

待用户恢复验证后应覆盖：空文件、中文／emoji 边界、CRLF／BOM、超长行／控制字符转义、
内容变化续读、越界游标、8 MiB 限制、悬空链接／越权路径、完整指令拼接／超限失败、真实
Kernel schema 与工具日志投影；搜索缓存失效、长行中段命中、跳过文件披露、目录过滤、
总读取／结果预算与匹配偏移续读。旧 `content`-only instruction fixtures 也需按新合同更新。
工作区图片／其他二进制、多协议 MCP、生态持久效果、安全 checkpoint 续跑及任务语义核对
仍未因此完成，历史通过记录不覆盖本切片。
