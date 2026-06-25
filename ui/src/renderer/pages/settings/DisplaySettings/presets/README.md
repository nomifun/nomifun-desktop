# 预设 CSS 主题契约 / Preset CSS Theme Contract

每个预设主题是一段完整的 CSS，由 `Layout.tsx` 注入为 `<head>` 末尾的 `<style id="user-defined-custom-css">`。
注入前 `processCustomCss` 会给**所有声明自动追加 `!important`**（`@keyframes` 块内除外）。

## 为什么必须覆盖到 `body` 层

Arco Design 的全部 design token（`--color-fill-*`、`--color-text-*`、`--color-bg-*`、
`--color-border-*`、`--primary-1..10` 等）定义在 `body { ... }` 与 `body[arco-theme='dark'] { ... }` 上。
CSS 自定义属性按**最近祖先**继承：只在 `:root` 覆盖这些变量，对 body 内的所有元素**完全无效**
（这正是历史主题"侧边栏选中态/下拉选项颜色不跟随"的根因——选中态是 `bg-fill-3` →
`var(--color-fill-3)`，被 Arco 的 body 定义短路）。

因此每个主题必须用下面的双块结构，把变量同时打在 `:root` 和 `body` 上：

```css
/* ===== Light ===== */
:root,
body {
  /* A. App 变量 + B. Arco token（亮色全量） */
}

/* ===== Dark（必须出现在 Light 块之后） ===== */
[data-theme='dark'],
[data-theme='dark'] body {
  /* A + B（暗色全量，变量集合必须与亮色块完全一致） */
}
```

**暗色块与亮色块的变量集合必须对称**：主题的亮色块以 `!important` 打在 `body` 上后，
Arco 自带的 `body[arco-theme='dark']` 暗色切换就被压制了——暗色值只能来自主题自己的暗色块。
漏写任何一个变量，暗色模式就会漏出亮色的值。

亮/暗切换机制：`<html data-theme="light|dark">` + `<body arco-theme="light|dark">`（useTheme 同步设置）。

## A. App 变量（亮/暗各一份）

| 组 | 变量 |
|---|---|
| 主色 | `--color-primary` `--primary` `--color-primary-light-1..3` `--color-primary-dark-1` `--primary-rgb`(r,g,b 三元组) |
| 品牌 | `--brand` `--brand-light` `--brand-hover` `--color-brand-fill` `--color-brand-bg` |
| 品牌色阶 | `--aou-1..10`（亮模式 1 最浅→10 最深；暗模式反转） |
| 背景 | `--bg-base` `--bg-1` `--bg-2` `--bg-3` `--bg-4` `--bg-5` `--bg-6` `--bg-8` `--bg-9` `--bg-10` `--color-bg-1..4`(与 bg-1..4 同值) `--bg-hover` `--bg-active` |
| 填充 | `--fill` `--color-fill` `--fill-0` `--fill-white-to-black` `--dialog-fill-0` `--inverse` |
| 文字 | `--text-primary` `--text-secondary` `--text-disabled` `--text-0` `--text-white` |
| 边框 | `--border-base` `--border-light` `--border-special` |
| 语义 | `--success` `--warning` `--danger` `--info` |
| 组件 | `--message-user-bg`(用户气泡) `--message-tips-bg` `--workspace-btn-bg` `--color-guid-agent-bar`(首页 Agent 条) `--sider-section-title-color`(侧栏分组标题，仅暗色需要) |
| 终端 | `--terminal-surface-bg`(保持深色，xterm 画布恒深) `--terminal-border` |

## B. Arco token（亮/暗各一份，必须在含 `body` 的选择器组里）

| 组 | 变量 |
|---|---|
| 背景 | `--color-bg-1..5` `--color-bg-popup`(下拉/弹层) `--color-bg-white` |
| 文字 | `--color-text-1..4` |
| 填充 | `--color-fill-1..4`（**fill-3 = 侧边栏选中态**，fill-2 = 下拉选项 hover/选中） |
| 边框 | `--color-border` `--color-border-1..4` |
| 主色衍生 | `--color-primary-light-1..4` |
| 主色阶 | `--primary-1..7`（**RGB 三元组**，如 `22, 93, 255`；Arco 按钮/链接经 `rgb(var(--primary-6))` 取色） |
| 次级按钮 | `--color-secondary` `--color-secondary-hover` `--color-secondary-active` `--color-secondary-disabled` |
| 杂项 | `--color-tooltip-bg` `--color-mask-bg` `--color-spin-layer-bg` `--color-menu-light-bg` `--color-menu-dark-bg` |

## 个性化点缀（可选，克制）

双块之后可以追加少量签名样式（毛玻璃、霓虹辉光、渐变按钮等），约束：

1. **只动外观**：颜色/背景/边框/圆角/阴影/字体/backdrop-filter/transition。
   禁止 `display`/`position`/`overflow`/`z-index`/`width`/`height`/`margin`/`padding` 这类布局属性
   （所有声明会被加 `!important`，布局属性会直接打碎页面）。
2. **选择器要窄**：点缀只打在具体目标上（如 `.layout-sider`、`.arco-btn-primary`、
   `.arco-modal`、`.arco-select-popup`），不要用 `*`、`div`、`[class*=...]` 这类宽选择器。
3. **禁止网络依赖**：不允许 `@import`/`@font-face` 外联字体；`font-family` 调整必须带完整本地回退栈。
4. **`@keyframes` 可用**（processor 已对其跳过 !important），但动画要轻（呼吸/微光级别），避免大面积重绘。
5. **变量不要写进 `@media`**：设置页预览缩略图靠静态解析 `:root`/`[data-theme='dark']` 块取色，
   嵌套块解析不到。

## 内容容器可读性红线

主题可以有玻璃、辉光、渐变和半透明质感，但承载正文或表单控件的容器必须先保证清晰可读。
这类问题通常不会破坏布局，却会让用户看不清弹窗内容或觉得消息列表脏乱，因此属于主题层硬约束。

1. **按钮弹窗 / 下拉 / Popover 背景不能过透明**：`.arco-popover-content`、`.arco-dropdown-menu`、
   `.arco-select-popup` 等内容弹层如果自定义 `background` / `background-color`，背景色必须足够实。
   半透明玻璃建议 alpha ≥ `0.86`；如果使用渐变，每个作为主体底色的 color stop 也应 ≥ `0.86`。
   可继续使用 `backdrop-filter`，但不能依赖模糊来弥补过低透明度。弹层里有标题、说明、表单、列表时，
   读感应接近实体卡片，而不是透到底层页面发白。
2. **不要给每条消息外层套主题背景**：禁止在预设主题里给 `.message-item` 添加
   `background` / `background-color` / `backdrop-filter` / 玻璃边框等大容器样式。
   `.message-item` 是消息列表每条消息的外层排版容器，不是真正的消息气泡。给它加背景会形成
   “每条消息一块浅白底/大卡片”的突兀感。消息气泡颜色应通过 `--message-user-bg`、
   `--message-tips-bg` 或具体消息组件自身样式控制。
3. **主题点缀不要扩大到结构容器**：如果需要装饰工作区按钮、侧栏或面板，选择器必须指向真实目标
   （例如 `.workspace-btn`、`.layout-sider`、`.arco-modal`），不要把同一组样式同时打到
   `.message-item` 这类通用排版节点上。

## 预览缩略图

无 `cover` 的主题卡片由 `CssThemeSettings` 静态解析主题 CSS 生成布局缩略图，
取色键：`bg-1`/`bg-2`/`bg-3`/`color-primary`/`color-text-3`/`color-fill-2`/`color-primary-light-3`。
保证这些变量在双块中有字面量或可解析的 `var()` 链即可。

## 自检清单

- [ ] 双块结构正确，Dark 块在 Light 块之后
- [ ] A + B 全量变量，亮/暗集合对称
- [ ] `--primary-1..7` 与 `--primary-rgb` 是 RGB 三元组（不是 #hex）
- [ ] 亮暗两种模式下正文对比度 ≥ 4.5:1
- [ ] 没有布局属性、宽选择器、外联资源
- [ ] Popover / Dropdown / Select 这类内容弹层背景不低于可读性红线，不能因为过透明而发白
- [ ] 没有给 `.message-item` 这类消息排版外层套大背景、玻璃边框或毛玻璃
