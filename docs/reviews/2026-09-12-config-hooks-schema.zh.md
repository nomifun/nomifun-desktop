# R32–R33：配置合并、Hook 与 schema 清理

承接 R32-03。已补读 nomi-config 全部生产文件、模板、compat/hook/shell/logging 测试及三份集成测试；config.rs 的历史测试按涉及行为核对，不将全量测试通过等同于逐条测试设计审计。

## R32-03：分层配置精简

- 合并输入改为保留显式字段的 TOML table；每份文件仍先独立进行类型校验，合并后再填默认值。删除“值等于默认值就视为未填写”的判断。
- 修复显式恢复默认 provider/截图等标量、独立 compact 参数丢失、session/plan/file_cache 局部覆盖误清全局值；四项新回归在旧实现失败。
- 保留既定特例：hooks/skills deny/LSP 追加；浏览器/电脑开关和 bash_sandbox 按 OR；session.enabled 按 AND；空 write_root/allowlist 不解除全局约束；同名 profile/MCP、Bedrock/Vertex 和 provider extra_body 整值覆盖。普通配置字段可以显式恢复默认值。
- 删除不再被生产调用的 ProjectInstructionsConfigFile::merge 与 LoggingConfig::merge；旧测试改走真实配置输入，不另留第二套合并实现。
- init_config 使用已有 tempfile 的完整写入 + persist_noclobber，避免 exists/write 覆盖竞争；新增已有内容保留和八线程初始化测试。没有声称这两项在旧实现稳定失败。
- profile 继承改为循环与 HashSet 去重，不再按链长递归；4096 层链回归通过，未在旧实现主动触发进程级栈溢出。
- 配置测试不再读取个人全局配置；模板 hook 示例改为双引号环境变量引用，日志路径示例不再暗示支持未实现的 ~ 展开。

仍待处理：硬迁移 persist_config_migration 的“内容比较→原子替换”不是针对任意外部编辑者的 CAS；比较后仍存在并发覆盖窗口。本批没有引入跨进程锁协议或宣称该窗口已解决，R32-03 仍保留这一项。

## R33-01：Hook 输入被当作 shell 源码

旧 interpolate_command 把工具参数直接插进命令。例如 echo 内的文件名 $(echo HOOK_INJECTION) 实际执行了子命令；真实受管 PowerShell 回归已复现。

- 值只经已有进程环境传递；Windows 仅将变量引用转换为环境变量语法，POSIX 直接使用 shell 原生展开。
- 删除含展开值的命令 debug 日志，避免工具输入随诊断泄漏；去掉两处只遍历一次的临时 Vec。
- 同一个回归覆盖命令替换、引号/空格/分号和包含另一变量名的数据，修复后均原样交付。

兼容变化：引用变量需使用双引号；单引号内不展开，变量内容也不会再次作为脚本执行。Hook 命令本身仍是受信任可执行配置，本次不将任意 shell 命令包装成安全沙箱。

## R33-02：schema 清理误改参数名和实例数据

旧两次全 JSON 遍历会删除名为 additionalProperties 的真实工具参数，并改写 const/examples 内形似 schema 的普通数据。两项旧实现失败。

复用既有 schema 关键字分类，合并为一次仅遍历 schema 位置的清理；参数名和 const/enum/examples 保持不动。原运行时 schema 不修改，provider 投影仍沿用既有有损兼容策略，不扩展为通用 JSON Schema 框架。

## R33-03：过时 shell 构造入口与日志初始化

- 全仓调用核对：ShellInfo/shell_info、shell_command_args/builder、重复 PowerShell payload 和 SupervisedShell::supervisor 无生产调用；删除。保留实际被 hooks/skills 使用的 SupervisedShell。
- 旧构造器测试迁移到真实受管 shell，验证环境/CWD、PowerShell 语法和非零退出码；不再测试无人使用的备用执行路径。
- 日志过滤规则先验证再创建目录/worker；日志集成测试改用局部 subscriber，避免设置进程全局 subscriber。

## 验证与限制

- cargo test -p nomi-config -- --test-threads=4：最终 172 单元 + 12 集成 = 184/0。
- cargo test -p nomi-providers --lib schema -- --test-threads=4：7/0（部分过滤命中分类器测试，不冒充所有 provider 端到端覆盖）。
- cargo test -p nomi-agent --test tool_execution_test hook -- --test-threads=4：2/0。
- 中间一次整模块运行的 CWD 测试错误地期待绝对路径，而生产按相对 .nomi.toml 工作；修正测试假设后重跑通过。一次补丁因瞬时写入失败未生效，已核对文件完整后重试成功。
- 只格式化本轮新增测试文件；未整库格式化、未提交、未运行无改动 UI。仅 Windows 本机执行，POSIX 分支/真实外部 provider 未运行。
- nomi-config 保持部分完成，仅剩明确登记的硬迁移并发窗口和测试设计剩余核对；不回头重复已验证的合并/hook/schema/旧 shell 清理。
