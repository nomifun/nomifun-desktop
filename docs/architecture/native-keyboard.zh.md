# 语音键盘与输入入口

语音键盘是 NomiFun 的软件语音输入能力。系统快捷键、设置页操作和字幕工具栏调用同一套语音 Actor；设备连接不构成录音、识别、编辑或美化的前提。设置入口为“设置 → 语音键盘”，路由 `/settings/voice-keyboard`。第一期关闭自动串口扫描；外置四键板接入在第一期提交之后作为用户主动启用的独立输入适配器加入。

## 软件操作

默认全局快捷键为 `CommandOrControl+Shift+K` 显示/隐藏语音字幕，`CommandOrControl+Shift+Space` 按住说话、松开识别，`CommandOrControl+Shift+B` 整理当前全文，`CommandOrControl+Shift+Escape` 取消当前操作并保留已定稿正文。Windows/Linux 使用 Control，macOS 使用 Command；用户可修改四个绑定，空值关闭对应动作，总开关关闭时释放全部绑定。输入格式、重复绑定和操作系统占用分别校验；注册失败显示在语音功能状态中，主工作台仍可使用。

快捷键按下只接受一次，系统重复按下事件不能重建录音；松开结束本次按住意图。取消、关闭、禁用、锁屏、睡眠和更新撤销旧意图，恢复后需要新的按下。修改快捷键须完整注册成功，持久化失败恢复旧绑定；不能把尚未保存的显示值当成已生效配置。快捷键回调仅向 Actor 发消息，不持有麦克风、供应商连接或文稿。

录音使用所选系统麦克风。真实首个非空输入回调发布聆听，全零样本只证明采集已启动，不制造识别成功。支持流式 partial/final 与停录后提交的普通 ASR。用户选择现有模型目录的模型；不建立第二模型目录、凭据库或 AgentSession，不自动切换模型和密钥。

字幕支持 UTF-16 光标插入、选区替换与原生 IME 编辑。无光标时续接正文，不添加录音边界换行。Ctrl+C / Command+C 使用系统选区复制；用户将内容粘贴到目标应用。Esc 结束本轮采集并关闭字幕，输入法组合中的 Esc 不同时关闭文稿。当前能力不自动向其他应用注入文字。

美化只调用既有 `ModelInvokeService::transform_text`，输入当前全文和指令，单次无工具请求；只有完整成功才提交候选。录音中请求美化先真实停采并完成识别；取消不等待云端结束，不接受旧任务的迟到结果。正文绑定 document ID、revision、操作 ID 与取消代数，旧 finally 不得释放新任务资源。

## 运行与资源边界

```text
NomiFun 主进程
  ├─ 应用维护：启动准入、安装维护端点、真正退出与文件事务
  └─ 语音 Actor：文稿、OS 采音、ASR、取消与美化
       ↑ 系统快捷键 / 设置页 / 字幕工具栏
       └─ 串口输入适配器：第一期不启动，后续仅用户主动开启
```

应用维护在语音 Actor 之前独立初始化；没有配置识别模型、没有 USB 设备或语音初始化失败，不能让工作台退出。语音功能状态展示局部失败，设置页仍可排查。字幕窗口与识别预热按使用需要创建，启动工作台不打开麦克风。麦克风只有一个 owner；停止以真实回执为准，超时继续保留 owner，不能丢弃句柄假报停止。

语音输入与安装维护拥有不同职责：

| 位置 | 职责 |
| --- | --- |
| `apps/desktop/src/keyboard/` | 软件语音 Actor、系统快捷键、采音、字幕、识别及界面命令 |
| `crates/backend/nomifun-keyboard/src/document.rs` | 文稿与设置状态；包含四个可自定义快捷键 |
| `crates/backend/nomifun-keyboard/src/service.rs`、`protocol.rs` | 有界设备协议与可 join 串口适配器；运行目录由调用方明确传入 |
| `apps/desktop/src/app_maintenance.rs`、`app_maintenance/platform.rs` | 应用级生命周期端点及已发布旧服务/登录登记的单向退役 |
| `crates/backend/nomifun-desktop-maintenance/` | 启动门禁、鉴权停机、安装归属校验、文件事务和短命 helper；不依赖串口、音频、模型或窗口库 |
| `apps/desktop/nsis/app-maintenance-hooks.nsh` | 安装、更新、卸载的应用维护会话 |
| `scripts/build-app-maintenance.mjs` | 构建独立 `nomifun-app-maintenance` |
| `ui/src/renderer/pages/settings/KeyboardSettings.tsx` | “语音键盘”设置，模型、麦克风、字幕与快捷键，不以设备连接决定就绪 |

保留现有 `keyboard-settings.json` 中的模型引用与软件偏好；新增 `shortcuts` 使用明确默认值。设置调整串行自动保存，文本输入合并且避开 IME 中间态；已修改值、原生确认与持久化成功分别处理。快捷键组通过明确的应用动作提交，避免输入半个组合时不断注册系统绑定。

本地麦克风检测不依赖 ASR 或外置键盘，不上传、写入或保留音频，只返回首回调、累计样本和最近窗口电平，最多约 5 秒。离页、换设备、关闭和更新按本次 testId 停止；晚到应答不得恢复旧检测。云端测试使用所选供应商，只有属于本次操作的新结果可显示为成功。

## 安装、升级与旧资源退役

Windows 安装器先完整解压 payload（含卸载器），由独立 `nomifun-app-maintenance` 保持应用维护租约，通过鉴权端点确认真实停采和退出后进行可恢复文件替换。helper 不初始化数据库、语音 Actor、串口或模型。应用版本升级不先卸载旧版；备份和替换只覆盖本次 payload 的应用文件，用户数据不被枚举或复制。

`maintenance.lock` 负责启动准入，`desktop.lock` 负责应用生命期。Windows 应用锁释放不证明系统进程及 exe 映像占用已经结束；helper 关停前保留精确安装路径的原生进程句柄，`apply` / `remove` 前最多等待 30 秒真正退出，再重查准备期间的新启动。文件发布和回滚使用同目录已校验暂存文件，对错误 5、32、33 最多重试 5 秒；持续失败保留原始错误并验证恢复。安装器每阶段最多等待 90 秒，回滚完整性不明时不启动混合版本。

已发布版本使用 `NomiFun/keyboard-service` 作为维护端点和锁的约定目录。应用维护继续使用该 rendezvous 身份以协调旧安装，仅保留同一目标安装的鉴权关停、登录项移除及旧卸载命令入口，不恢复或启动设备守护进程。此目录名称不表示当前存在键盘后台服务。旧资源退役失败限制语音输入，并提供诊断；不能在无法证明旧资源停止时创建第二个输入 owner。

Windows `release:win` 和签名 RC 流程在上传/接纳产物前运行 `run-windows-app-maintenance-gate.mjs`，对本次构建的 helper 执行真实进程/文件/IPC 回归与生产 NSIS 模板的隔离安装、恢复、卸载。夹具包与真实发布包的运行证据分别记录，不以夹具证明正式 Tauri GUI 已验收。

macOS/Linux 只清理归属于当前安装的旧登记；AppImage 跨临时挂载不能证明进程归属时，先手动退出旧实例。Linux 窗口继续使用已验证的 XWayland 路径，纯 Wayland 无 XWayland 未取得支持证据。随包维护/卸载脚本说明见[打包说明](../../apps/desktop/keyboard-packaging/README.md)。

## 验证

- 语音/设备核心：`cargo test --locked -p nomifun-keyboard --lib`。
- 独立维护：`cargo test --locked -p nomifun-desktop-maintenance --lib --bins`。
- 原生 Windows 维护：`node scripts/validation/run-windows-app-maintenance-gate.mjs --helper <本次构建的绝对路径>`。
- UI：语音键盘设置、就绪、自动保存、字幕组件的定向测试，`bun run typecheck`、`bun run check:desktop-ui-boundary` 和 i18n 检查。
- 目标平台的系统快捷键按下/松开、组合键占用、锁屏恢复和实际麦克风/供应商调用需要单列原生证据。编译、合成 PCM 或模拟 UI 不替代这些证据。

当前自动化、原生夹具和未执行范围见[验证记录](../validation/native-keyboard.zh.md)。
