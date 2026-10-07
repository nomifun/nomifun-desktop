# R31–R32：共享类型与配置入口

## R31-01：nomi-types 完整模块范围

已阅读 lib、agent、message、tool、llm、skill_types、compact、file_state，
Cargo 清单和 plan_mode_transition_test；核对四类 provider 工具定义构造、
Agent 的委派请求 validate / context modifier 消费及 provider round cursor 清理入口。
类型层没有自己的进程/后台任务，生命周期实现仍归调用方模块。

- 工具描述文档写明最多 200 字符，但旧代码按字节设限，150 个 é 也会截成 100 个加省略号。
  用 char_indices().nth(200) 取得真正字符边界，删除二次边界修补遍历。
- 同时识别 CRLF 空段落，保留既有 LF 行为；2 项回归旧实现失败，修改后通过。
- 删除 skill_types 中与现有公开集成测试重叠的 4 项构造/Debug/相等性测试；保留集成测试中的所有对应断言。
- ToolArtifact 别名和 with_artifacts 保留：它们有明确跨 provider/历史 wire 兼容目的，不能因形状相同就删除。
- Agent 状态/权限、消息序列化及缓存状态字段未机械改写；约束是否在每个业务入口被调用，仍须在对应执行/工具模块核对。

验证：nomi-types 65 单元 + 5 集成 = 70/0；nomi-providers 的 deferred 定向 2/0。
Provider 的四处生产调用均已阅读；定向过滤只命中 OpenAI/Anthropic 两项测试，不冒充四类 provider 的完整联调。

## R32-01：存在的配置文件不能当作“不存在”

原 load_config_file 对读取失败一律返回默认值，对 TOML/字段错误也只 warn 后默认。
例如 bash_sandbox 的布尔值误写成字符串，会悄悄清空整份配置并回到未启用限制的默认值。

改为只有 NotFound 使用默认值；不可读、非法语法/字段值错误通过已有 Result 返回调用方。
去掉重复日志和无错误结果；读取失败不修改文件。旧迁移失败关闭契约保留。
新增非法配置/目录被当文件两项旧实现失败，修复后通过；正常缺失仍可选。

兼容变化：以前被静默忽略的坏配置现在会阻止配置解析，需要用户纠正；这是本次有意修正。

## R32-02：profile 的两处优先级丢失

- profile.model 原只写 default.model，随后 resolve 却先选 provider.model，导致选择的 profile 模型不生效。
  在现有 profile 应用位置同步覆盖当前 provider 的 model，保留 CLI 的最终优先级。
- 子 profile 的 compat 原整对象替换父 profile；改为使用现有 ProviderCompat::merge 合并，
  只覆盖显式字段（包括显式 false），其余继承。

两项旧实现失败、修复后通过。没有新增配置框架或更改 provider API。

验证：cargo test -p nomi-config --lib config:: -- --test-threads=4，66/0（含四个新回归）。
这个命令不等于 config 全模块测试，更不等于全模块审计完成。

## R32-03：配置层合并与持久化剩余范围

已阅读 config.rs 生产加载/迁移/合并/profile/init 路径，以及 compact、plan、file_cache 全文件；
compat.rs 只读到 ProviderCompat 默认值/合并/accessor 和 schema sanitize 入口。
config.rs 现有测试只阅读与本批变更相关部分，模板后半与其他测试未逐行覆盖。

待继续：

- merge_config_files 用“值是否等于默认值”猜测字段是否提供，无法可靠表达显式恢复默认；
  compact 甚至只检查 context_window/enabled，单独设置 compaction/toon/其他预算可能丢失。
- Session 分支中同一 directory 条件重复且整对象替换会丢其他全局设置；
  plan/file_cache 的整块替换、browser 的 OR 规则需结合约定统一判断，不先添加一套覆盖框架。
- persist_config_migration 的比较和最终替换之间仍有竞争窗口；init_config 的 exists + write 同样待核对并发发布。
- profile 继承深度、额外配置 schema 投影、logging/hooks/shell 的关闭/错误路径待审。
- 现有少数 Config::resolve 测试读取实际全局配置，后续隔离此类测试环境，避免依赖个人设置；本批新回归仅使用临时目录或纯内存配置。

nomi-config 因此保持“部分完成”。没有将这些已发现的剩余点隐去或宣称全部解决。

## 本组收尾

R28–R32 累计修改的生产段（按文件首个 cfg(test) 前计算）净减少约 99 行；
另增行为回归、删除重复测试，不用新增测试总量来冒充生产代码精简。
未自动提交，未重跑无改动的 UI；Windows 本机验证，不代表其他操作系统/真实外部服务。
