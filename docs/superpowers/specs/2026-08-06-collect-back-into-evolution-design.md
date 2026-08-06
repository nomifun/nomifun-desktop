# 数据采集归位「进化」标签，并停止采集模型回复

- 日期：2026-08-06
- 状态：设计已确认，待实施。分支 `feat/collect-back-into-evolution-tab`。
- 影响面：`ui/src/renderer/pages/nomi/workspace/tabs/EvolutionTab/**`、`ui/src/renderer/pages/settings/**`（删除）、`ui/src/renderer/components/layout/Router.tsx`、i18n 两语、`crates/backend/nomifun-companion/src/{collector,prompt,learner,config}.rs`、`docs/guides/companions.zh.md`

## 1. 问题陈述

本次修正两个独立的问题，共用一次改动窗口。

### 1.1 采集配置被放到了应用设置里

2026-08-04 的伙伴工作区重构（[spec](2026-08-04-companion-workspace-redesign-design.md) §4.5 / §5.2）把 `tabs/CollectTab.tsx` 判为"应用级隐私设置"并迁出伙伴页。commit `03ce6bc9` 删除了它，`e2699069` 在 `/settings/privacy` 重建了这些控件。

这个判断是错的。数据采集是**为伙伴而做的能力**——它存在的唯一目的是给伙伴的定时学习提供素材。把它归到应用设置，产生三个后果：

1. **配置路径变长。** 用户在「进化」里打开定时学习后，要设定学习素材，必须离开伙伴页、进入设置、找到「数据采集」。原设计里这两件事在同一屏（`git show 03ce6bc9^:ui/src/renderer/pages/nomi/index.tsx:271-272`，`collect` 标签渲染的是 `<LearnTab collectionSection={<CollectTab />} />`）。
2. **能力范围被无意扩大。** 归入设置后它被表述为"这台设备记录什么"的隐私设置，暗示了一个比实际更宽的适用面。它并不是通用隐私控制，只是伙伴学习的素材开关。
3. **留下了一颗维护钉子。** `shell.structure.test.ts:172-177` 立了一条守卫，禁止 `EvolutionTab` 出现 `patchSharedConfig`/`getSharedConfig`，以强制执行"采集归设置页"这个决定。

### 1.2 采集了模型回复，成本不成比例

`companion_dialogues` 一个开关产出两种事件：

- `companion.user_message` —— 主人对伙伴说的话（`collector.rs:667`）
- `companion.reply` —— 伙伴自己的回复，从流式分片缓冲、在 `turn.completed` 时落盘（`collector.rs:816`）

回复的体量远大于提问，却只被当作学习器的上下文使用——`LEARN_SYSTEM` 第 7 条（`prompt.rs:61`）明确禁止把它当作主人的事实、意愿或承诺。为一份不能直接产出记忆的数据付出大头的磁盘与算力，不成比例。

工作会话的模型回复本来就不采集（`collector.rs` 注释："Work-session model replies are not a collection source"），`chat_assistant_replies` 源已在 `29f43f7f` 删除。所以本次只需处理 `companion.reply`。

## 2. 目标与非目标

**目标**

- 采集控件回到「进化」标签，与学习配置同屏；`设置 › 数据采集` 入口消灭。
- 停止采集 `companion.reply`；已在盘上的残留回复不得被学习器误读。
- 后端采集配置的存储层级、字段、路由**一行不动**。

**非目标**

- 不把 `CollectConfig` 改成按伙伴。它仍是装机级的一份共享配置；本次只改它在哪儿编辑，不改它属于谁。
- 不新增伙伴工作区标签。标签条保持 7 个。
- 不新增"每个伙伴从哪些已记录源学习"的选择器。该字段在 `CompanionProfileConfig` 上仍不存在，本次不引入。
- 不重命名 `companion_dialogues` 字段。改名会触发 `SharedCompanionConfig::load_migrating` 的 JSON 迁移，代价与收益不成比例；字段语义收窄由文案承担。

## 3. 采集 UI 归位

### 3.1 落点：并入「进化」，不新增标签

采集控件插在「进化」标签的 `学习配置` 与 `技能生成配置` 之间，还原重构前"学习与素材同屏"的形态。

```
进化 标签
┌ 学习配置 ──────────────── 开启定时学习 / 学习周期 / 学习模型
├ 采集来源 ──────────────── 5 个源开关 + 敏感度 + 今日/当前保留计数   ← 搬回
├ 保留策略 ──────────────── 目标保留期 / 容量上限 / 当前占用 + 应用   ← 搬回
├ 技能生成配置 ──────────── 开启 / 保守·激进
├ 休眠时段 ──────────────── 起止
└ 全部停止 ──────────────── 一键全关                                ← 搬回
```

不新增标签有一个额外好处：`shell.structure.test.ts:77-84` 把 `'collect'` 钉为**已退役的标签 key**，新增标签会撞上该断言；并入「进化」自然避开。

`全部停止` 排在最后：它同时关掉采集与学习/技能生成，是整个标签的总闸，放在所有被它影响的分区之后才读得通。

### 3.2 搬迁清单

三个分区已经使用与工作区标签相同的 `NomiSettingLayout` 原语（`NomiSettingSection` / `NomiSettingList` / `NomiSettingRow`），因此按原样搬迁，不重写内容：

| 从 | 到 |
|---|---|
| `pages/settings/privacy/CollectionSourcesSection.tsx` | `pages/nomi/workspace/tabs/EvolutionTab/CollectionSourcesSection.tsx` |
| `pages/settings/privacy/RetentionSection.tsx` | `.../EvolutionTab/RetentionSection.tsx` |
| `pages/settings/privacy/StopAllSection.tsx` | `.../EvolutionTab/StopAllSection.tsx` |
| `pages/settings/privacy/useCollectSettings.ts` | `.../EvolutionTab/useCollectSettings.ts` |

保留现行版本而非恢复旧 `CollectTab`：现行版本严格更完善——5 个源开关（旧版 4 个，`companion_dialogues` 从未有过开关）、完整的保留策略与实时占用、降低策略时的危险确认。

搬迁时必须重写的文件头注释：三个分区与 hook 的注释目前都断言"`设置 › 数据采集` 是唯一编辑者"（`useCollectSettings.ts:8-14`），搬迁后这句话不再成立。

### 3.3 删除清单

- `pages/settings/PrivacySettings.tsx` —— 外壳（`SettingsPageWrapper` + `h1` 页头）是设置页专用，不随分区搬迁。其加载态与重试态与 `EvolutionTab/index.tsx:52-66` 已有的实现逐字重复，由后者接管。
- `pages/settings/privacy/` 目录。
- `Router.tsx` 的 lazy import（:18）与 `/settings/privacy` 路由（:224）。不留重定向 shim：全仓仅 9 处引用该路径，无外部深链、无托盘菜单、无 onboarding 引导，`/settings` 裸路径本就重定向到 `/settings/system`。
- `SettingsSider.tsx`：`BUILTIN_TAB_IDS` 中的 `'privacy'`（:27）、导航项（:81-86）、随之失去引用的 `DataLock` 图标 import。
- `SettingsPageWrapper.tsx`：移动端 pill 导航的 `privacy` 项（:40-45）及其图标 import。
- `EvolutionTab/LearningSection.tsx`：`CollectionLink` 组件（:34-57）与 `学习素材来自哪里` 行（:107-114）。分区已在同屏，跳转链接失去意义。
- `shell.structure.test.ts:172-177`：禁止 `EvolutionTab` 触碰共享配置的守卫。这是 §1.1 所述的那颗钉子，随本次决定一并拔掉，而不是绕过。

### 3.4 Rules of Hooks 约束

`rulesOfHooks.test.ts` 静态禁止组件级早返回之后出现 hook 调用，而每个标签都以 `if (!profile) return <Spin/>` 开头。因此 `useCollectSettings()` 必须声明在该守卫**之上**。`EvolutionTab/index.tsx` 现有的三态渲染（加载 / 读取失败重试 / 正常）需要合并两个数据源的状态：进化配置与采集配置各自独立失败，任一失败都要显示可重试的错误态而非静默的空白分区。

### 3.5 i18n

43 个 `settings.privacy.*` 叶子键改名到 `nomi.collect.*`（该命名空间目前不存在），两个语言各 43 个。留在 `settings.*` 命名空间下会与实际归属不符。

`nomi.evolution.collectionScopeTitle` / `collectionScopeDesc` / `openCollectionSettings` 三键随 §3.3 的行删除而删除。

两语言必须同步：`check:i18n` 会在任一单边键上失败。改完须跑 `bun run gen:i18n` 重新生成 `i18n-keys.d.ts`（参考语言是 en-US）。

## 4. 停止采集模型回复

### 4.1 后端改动

- `collector.rs`：删除 `companion.reply` 的落盘（:816）与其所在的缓冲区排空循环。随之删除流式回复缓冲机制（`reply_buffers` 及 `MAX_REPLY_CHARS`），前提是确认 `tool_calls` 路径不依赖该缓冲区——`message.stream` 分支同时服务两者，需逐一核对后再删。伙伴 XP 的发放（`add_companion_xp`）独立于缓冲区，不受影响。
- `config.rs`：`companion_dialogues` 的字段注释目前写"owner messages + companion replies"，收窄为仅主人消息。
- `prompt.rs`：重写 `LEARN_SYSTEM` 第 7 条（:61）——不再存在 `companion.reply` 事件，该条现有表述失效。同步更新 :283-284 两个断言。

### 4.2 残留数据不得被误读

盘上已有的日文件里存着历史 `companion.reply` 事件，按保留策略自然老化前会一直被学习器读到。

若只删掉 `LEARN_SYSTEM` 第 7 条，就**没有任何规则再禁止模型把伙伴自己的话当成主人的意愿**——那正是第 7 条在防的误读，删掉它反而打开了这个口子。

因此学习器读取事件时必须过滤掉 `companion.reply`，让残留数据既不进 prompt 也不产出记忆，随保留策略自然消失。这是本次唯一新增的防御逻辑，且是删除动作的必要配套，不是可选项。

### 4.3 文案

两语言的 `companion_dialogues` 源描述目前写"你和桌面伙伴的对话原文：你说的话和伙伴的回复（超长会截断）"，改为只描述主人一侧。

### 4.4 已知代价

学习器失去对话上下文。当主人说"对，就这样做"时，学习器不再能解析"这样"指向什么，这类指代型内容的提炼质量会下降。

这是明知的取舍：回复占据采集体量的大头，而它按设计从来不能直接产出记忆。接受质量下降换取磁盘与算力。若日后判断代价过高，替代方案是保留高度截短的回复摘要而非完整原文——那需要一份新设计，不是恢复本次删除的一部分。

## 5. 测试

**基线先行。** main 上本就有约 13–14 个既有失败（见 `preexisting-test-failures-linux`）。实施前记录基线，收尾时逐一比对，既有失败不得计入本次改动，本次引入的失败不得混入既有失败。

需要更新的现有测试：

- `shell.structure.test.ts`：删除 §3.3 所述守卫。其全语料不变式（`@license` 文件头、禁 arco 图标、禁 `border-border-N` 等）对搬入 `pages/nomi` 的四个文件自动生效——它们按运行时目录遍历自动纳入，搬进来即受约束。
- `collector.rs` 中断言产出两条事件的测试（`companion_dialogues_collects_companion_dialogue_by_default` 等），改为只断言 `companion.user_message`。
- `prompt.rs:283-284` 的两个断言。

新增覆盖：

- 采集分区确实渲染在「进化」标签内（结构断言，与既有 tab 结构测试同风格）。
- 学习器过滤 `companion.reply`：喂入含历史回复事件的日文件，断言它既不进 prompt 也不产出记忆。这是 §4.2 的守护，不能只靠人工核对。

验证命令：

```
bun run check          # 含 typecheck / check:i18n / check:theme / check:icons / check:dead-css
bun test ui/src/renderer/pages/nomi/workspace/
bun test --cwd ui
bun run test:crate     # companion crate
```

## 6. 文档

- `docs/guides/companions.zh.md`：更新采集配置的位置描述。
- `2026-08-04-companion-workspace-redesign-design.md`：§4.5 与 §5.2 断言采集迁往应用设置。不改写历史结论，追加一条指向本文的修正说明——该 spec 的 §4.2/§4.3 已有同样的"实施时作废"先例，沿用其体例。
