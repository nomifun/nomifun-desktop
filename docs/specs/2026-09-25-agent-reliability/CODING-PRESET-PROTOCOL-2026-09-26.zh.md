# 编程 preset 贪吃蛇任务的协议失败排查

## 原会话事实

会话：`01a0d94b-2719-7f82-b220-b1bbc564335b`。用户输入为「帮我写一个 贪吃蛇h5小游戏」，使用 `coding.codex` revision 2、`step-3.7-flash` / OpenAI Chat。开发版 build digest 为 `cb56f0c68d96a416acec5c8acb91e844dddb8f6369645d601b0850bd99d69c12`。

从受理消息到失败约 69.4 秒，最终为 failed，head 回到 ready。事件序号 17、23、29 均为 `model_response_rejected`，前两次 continuation=true，第三次耗尽纠错额度。终态错误为 `USER_LLM_PROVIDER_INVALID_TOOL_CALL`。

本回合有 3 次模型步骤、0 次工具调用完成、0 次工具执行、0 次文件效果、0 次压缩。因此失败发生在执行之前，不是贪吃蛇代码运行失败，也不是工作目录或 Coding 权限缺失。流在收到完整 usage 之前被拒绝，不能把没有 usage 记录解释成没有模型消耗。

原数据库只读调查，目录为 `%LOCALAPPDATA%/NomiFun-dev-schema-b40e0155e59ea087`。原会话、终态及其他文件未用于验证写入。

## 真实接口对照

首先导出由应用真实 preset 编译和 provider 编码产生的构造请求：25 个编程工具、完整 schema、标准系统提示，替换用户输入为本次原始中文任务。构造请求不含 HTTP 凭据、原用户数据库内容或原会话的工具历史。

随后对授权的 Step Plan 接口执行有界格式诊断。只检查响应字段、长度、终止原因和 usage，不执行任何返回的工具，也不记录模型思考内容或完整代码正文。

| 对照 | 唯一关键变化 | 观察结果 |
| --- | --- | --- |
| 原默认提示、25 个工具、Auto | 保留错误格式示例 | 没有原生 `tool_calls`；正文中出现 `write_file` 工具标记；生成 4,096 tokens 后以 length 结束 |
| 正向提示、相同 25 个工具、Auto | 替换默认提示里的一句说明 | 收到原生 `write_file`，没有正文伪调用；生成 3,370 tokens，以 tool_calls 正常结束 |
| 单工具 Required 探针 | 设置 required，用户文本要求普通 READY 回复 | 接口 HTTP 200，但返回普通文本并以 stop 结束，没有 `tool_calls` |

对照支持将原默认提示判定为这一失败的重要触发因素。模型采样并非严格确定性，这不是成功率统计，也不能用两次生成时长相减声称固定加速比例。

第三项也纠正了此前验证的不足：一个接口接受 `tool_choice=required`，并在“请调用工具”的请求中成功调用工具，并不证明它会强制执行该参数。当前接口不能依赖这个字段提供强制纠错保证，仍须检查实际响应和真实 owner 回执。

## 框架侧问题

默认高优先级提示原本写着：

```text
Use native tool calls, never XML-shaped <tool_call> text
```

这把应用明令拒绝的竞争格式本身注入了每个模型请求。在完整编程工具表下，真实模型仍会照该形式输出正文工具调用。纠错请求继续携带同一默认提示，再加上供应商没有强制遵循 required，造成三次同类失败。

现有解码器区分 `delta.content`、推理字段及 `delta.tool_calls`，本次原样接口诊断也复现了正文格式，未发现必须靠重新解释正文来补造工具调用的证据。继续保持不执行正文伪调用、整批预检、权限和幂等保护。

## 实际修改

1. 将默认策略改为正向描述：通过已提供的函数接口执行动作，助手正文用于面向用户的说明；不再在默认系统提示中嵌入错误协议标记。
2. 更新纠错代码注释，明确 Required 是发出的协议请求，不是供应商已遵守的证明。响应守卫和真实工具回执继续决定实际发生了什么，没有增加重试次数或执行文本工具的后门。
3. 应用级构造回归检查最终发给 provider 的系统提示，确保默认政策不再注入竞争的工具语法。保留完整工具表、协议拒绝与后续原生调用流程。
4. 为既有真实贪吃蛇 smoke 增加独立 JavaScript 语法验收：检查内联 classic/module 脚本，不执行生成代码；独立文件不得依赖外部 script。语法失败会使验收失败，而不是只凭文件中出现 canvas 等关键词通过。
5. 增加检查器的有效代码、错误语法、外部脚本和 module 测试。所有凭据仍通过既有隔离 runner 注入，不传给 Cargo、构建脚本或语法检查器。

## 验证记录

| 检查 | 结果 |
| --- | --- |
| `cargo test -p nomifun-agent-runtime --lib` | 115 通过 |
| 应用级 `coding_preset_creates_nested_game_in_default_conversation_workspace` | 1 通过，覆盖最终 provider 请求中的正向提示、实际工具与文件链路 |
| `game_syntax_verifier_checks_code_without_executing_it` | 1 通过，涵盖正确/错误语法、外部脚本、module 与不执行代码 |
| 既有隔离 runner `--game-smoke` | 真实 StepFun 回归通过 |
| `git diff --check` | 通过 |

本轮按编程 preset 的修改范围选择应用检查，没有重复运行上轮的 General/browser 构造测试；没有运行全仓库测试。

真实 smoke 的结果为：`coding.codex` 官方模板、`step-3.7-flash`、真实文件写入 1 次、工具错误 0、正常最终回复、生成 `snake_game.html` **13,550 字节**。独立读取与内联 JavaScript 语法检查通过。日志汇总还有 3 个 read_file 回执；读取次数不作为玩法验收证明。runner 完成了原有凭据隔离和清理检查，最终 `live_smoke_status=pass`。

该次实际执行的 build digest 为 `a134c8c456f655428d14797c7342bc182268204555cfca9b2c8a637e2ce13433`。

真实任务使用既有 `--game-smoke` 场景，在新建隔离工作区中明确指定 `snake_game.html` 及键盘、计分、开始/重新开始要求。既有 fixture 设置 temperature=0、output_limit=4096；前面的原提示/正向提示格式对照则使用捕获请求的默认采样配置。完整 smoke 是进一步的交付链路回归，不是把原失败回合重发后改记为成功，也不是对原始简短输入做统计重复采样。

## 证据与限制

诊断结果和日志保存在忽略目录 `.tmp-agent-coding-failure-20260926/`，包括原会话只读快照、原提示/正向提示/Required 探针的字段摘要。原任务失败记录保留。

这些真实失败说明先前验证的覆盖不足。单元测试、单一小工具调用与完整任务交付是不同层次的证据。本轮按实际产物、语法检查、工具回执和任务终态验证，不将小样本通过推广为 99% 成功率或行业标准认证。JavaScript 语法检查也不代表浏览器交互、碰撞逻辑或完整游戏体验已经验收。
