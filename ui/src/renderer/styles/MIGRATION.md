# Theme Color Migration Guide

> 门禁 / Gate: `bun run check:dead-css`（`scripts/check-dead-css-utilities.mjs`）
> 拦住本文「⚠️ 四个死写法」一节里的**任何一处**使用。存量已清零，棘轮基线已删除，
> 现在是一刀切禁令。
> The sweep is done and the ratchet baseline is gone: these forms are now simply
> banned, anywhere under `ui/src`.

## ⚠️ 四个死写法 / Four dead forms (measured, not guessed)

下面四种写法在本仓库**编译不出 CSS，或编译成错的属性**。都用真实 UnoCSS
生成器（`ui/uno.config.ts`）实测过。写出来不会报错、tsc 抓不到、页面也不会崩——
元素只是静默保留继承来的颜色，所以特别容易蔓延。

### 1. `{text,bg,border}-[rgb(var(--RAMP-N))]` —— 整条声明被浏览器丢弃

`RAMP` ∈ `primary` | `danger` | `success` | `warning` | `link`。

UnoCSS 把中括号里的值识别成颜色，于是注入 slash-alpha：

```css
/* text-[rgb(var(--danger-6))] 实际产出 */
.text-\[rgb\(var\(--danger-6\)\)\] {
  --un-text-opacity: 1;
  color: rgb(var(--danger-6) / var(--un-text-opacity));
}
```

但这些 ramp 变量是**逗号分隔的三元组**（Arco 是 `--red-6: 245,63,63`，预设主题写
`--primary-6: 232, 23, 74;`），展开后是 `rgb(245,63,63 / 1)` —— 语法非法，浏览器
把**整条 `color` 声明**丢掉。

```tsx
// ❌ 编译出来的声明被浏览器丢弃，元素继承父级颜色
<span className='text-[rgb(var(--danger-6))]'>删除</span>
<div className='bg-[rgb(var(--primary-6))]' />
<div className='focus-visible:border-[rgb(var(--primary-6))]' />

// ✅ 用项目自带规则（uno.config.ts 的 (bg|text|border)-(primary|success|warning|danger)-[1-9]）
<span className='text-danger-6'>删除</span>   // → color: rgb(var(--danger-6))
<div className='bg-primary-6' />
<div className='focus-visible:border-primary-6' />
```

**例外（合法，不要改）**：显式 `rgba(...)` 自带 alpha，UnoCSS 不再注入，产出原样保留：

```tsx
// ✅ 合法：background-color: rgba(var(--primary-6),0.12)
<div className='bg-[rgba(var(--primary-6),0.12)] text-danger-6' />
// ✅ 合法：非颜色属性（阴影）里的 rgb()/rgba() 不走颜色管道
<div className='shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.22)]' />
```

`link` 色阶**没有**对应的项目规则（规则只覆盖 primary/success/warning/danger），且
Arco 只提供三元组 `--link-6`（没有 `--color-link-6`），所以要 link 色就写
`text-[rgba(var(--link-6),1)]` —— 自带 alpha，不会被注入。

同理，Arco 的其它色板（`--arcoblue-*` / `--purple-*` / `--orange-*` / `--cyan-*` /
`--gray-*` …）也都是逗号三元组、也都没有项目规则，写法一样：

```tsx
// ❌ 同一个 bug，只是色板名不同
<Tag className='!bg-[rgba(var(--arcoblue-6),0.1)] !text-[rgb(var(--arcoblue-6))]' />
// ✅ 自带 alpha
<Tag className='!bg-[rgba(var(--arcoblue-6),0.1)] !text-[rgba(var(--arcoblue-6),1)]' />
```

**前缀也不止 `text` / `bg` / `border`。** 任何走 UnoCSS 颜色管道的工具类都会被注入
slash-alpha —— `ring-` / `outline-` / `border-t-` / `border-b-` / `divide-` …，而项目规则
只覆盖无方向的 `text|bg|border`，所以这些前缀**没有**规则类可用，只能写自带 alpha 的形式：

```tsx
// ❌ --un-ring-color: rgb(var(--primary-6) / var(--un-ring-opacity))
<div className='ring-2 ring-[rgb(var(--primary-6))]' />
<div className='border-t-[rgb(var(--primary-6))] focus-visible:outline-[rgb(var(--primary-6))]' />

// ✅ 自带 alpha
<div className='ring-2 ring-[rgba(var(--primary-6),1)]' />
<div className='border-t-[rgba(var(--primary-6),1)] focus-visible:outline-[rgba(var(--primary-6),1)]' />
```

（`border-t-primary-6` 是**没有** CSS 的：项目规则不带方向，theme 里也没有 `primary-6` 键。）

门禁的 ramp 规则按 `<任意前缀>-[rgb(var(--<任意色板名>-N))]` 匹配，覆盖全部色板与全部前缀。

### 2. `border-border-*` —— 产出 0 条 CSS

theme 里没有名为 `border` 的颜色，所以 `border-border-` 后面**跟什么都不生成规则**：
数字后缀（`border-border-1/2/3`）与命名后缀（`border-border-base`）一样死。门禁最初
只按 `border-border-\d` 匹配，于是 `HTMLViewer.tsx` 里 3 处 `border-border-base`
连棘轮带禁令一起漏过去了；现在规则是 `border-border-[a-z0-9]+`。

```tsx
// ❌ 一条 CSS 都没有（数字后缀与命名后缀都一样）
<div className='border border-border-2' />
<div className='border-b border-border-base' />

// ✅ 三种可选替代
<div className='border border-solid border-arco-2' />  // → var(--color-border-2)（Arco 边框 token，1-4）
<div className='border border-3' />                   // → var(--bg-3)（项目背景/分隔色阶）
<div className='border border-[var(--border-base)]' /> // → var(--border-base)（基础边框变量）
```

### 3. `-b-` 方向陷阱：`border-b-base` / `border-b-light` 打不到基础边框

UnoCSS 解析 `border-b-*` 时**先吃掉 `-b-` 当成 bottom 方向**，再拿剩下的键去
`theme.colors` 里查，所以：

| 写法             | 实际产出                                | 本意                              |
| ---------------- | --------------------------------------- | --------------------------------- |
| `border-b-base`  | `border-bottom-color: var(--bg-base)`   | 四边基础边框 `var(--border-base)` |
| `border-b-light` | `border-bottom-color: rgb(246 246 246)` | 浅色边框 `var(--border-light)`    |

`uno.config.ts` 里曾有一个 `borderColors = { 'b-base', 'b-light', 'b-1', 'b-2', 'b-3' }`
配置块想支撑这套写法，**它永远不可达**（方向解析先发生，且 `b-1/2/3` 会被
`backgroundColors` 的数字键抢先命中），已删除；删除前后整站产出 CSS 逐字节相同。

```tsx
// ❌ 落到 --bg-* 上，不是基础边框色
<div className='border border-b-base' />

// ✅ 四边基础边框（注意必须带 border-solid，本仓库没有全局 border-style reset）
<div className='border border-solid border-[var(--border-base)]' />
// ✅ 只要下边框（宽度/样式/颜色分开写）
<div className='border-b border-b-solid border-b-[var(--border-base)]' />
```

同一个陷阱还有一个变体：**数字键**也会被 theme 颜色劫持。`border-b-2` 不是
「下边框 2px」，而是 `border-bottom-color: var(--bg-2)`（`backgroundColors` 的 `2` 键
命中）。要下边框宽度就写 `border-b-2px`。

### 4. `bg-primary-6/12` —— 项目规则被斜杠透明度打断，产出 0 条 CSS

`uno.config.ts` 的 ramp 规则是 `^(bg|text|border)-(primary|success|warning|danger)-([1-9])$`，
**以 `$` 结尾锚定**。末尾挂一个 `/12` 就没有任何规则能匹配整个类名了：

```tsx
// ❌ 一条 CSS 都没有（规则锚定 $，/12 让它匹配不上）
<div className='bg-success-6/12' />
// ✅ 要透明度就写自带 alpha 的中括号值
<div className='bg-[rgba(var(--success-6),0.12)]' />
```

这是把写法 1 **机械 sed** 成项目规则类时最容易踩的坑：原来的
`bg-[rgb(var(--success-6))]/12` 里的 `/12` 会被留在后面，于是一个死写法换成了另一个
死写法，而且换完之后连 grep 都找不到它了。门禁把这条也列为禁令。

## ✅ 存量清理已完成 / Sweep completed

存量当初故意没有跟门禁同一次改动一起清：一次性替换会同时改变数百处渲染颜色（原本
静默继承父级颜色的元素开始显示本意颜色），需要逐站点判断 + 明暗双主题目测。

**这项清理已经做完了**，`scripts/check-dead-css-utilities.mjs` 的 `BASELINE` 随之删除，
门禁从棘轮变成一刀切禁令。清理时实际处理的量：

| 写法                                                | 文件数 | 出现次数 |
| --------------------------------------------------- | ------ | -------- |
| `{text,bg,border}-[rgb(var(--RAMP-N))]`（含 `/NN`） | 79     | 228      |
| `border-border-N`                                   | 17     | 40       |
| `border-b-base` / `border-b-light`                  | 4      | 8        |
| **原基线去重合计**                                  | **95** | **276**  |
| 另外补掉的非语义色板（arcoblue/purple/orange/cyan/gray） | 5      | 8        |
| 另外补掉的非 text/bg/border 前缀（ring/outline/border-t/border-b） | 9      | 9        |

清理时定下的几条判断，后续改动请沿用：

- **ramp → 项目规则类**：`text-[rgb(var(--danger-6))]` → `text-danger-6`。
  非语义色板没有项目规则，改成自带 alpha 的 `text-[rgba(var(--arcoblue-6),1)]`。
- **中括号 + `/NN` 一律改 `rgba()`**，绝不能 sed 成 `bg-success-6/12`（见上文第 4 节）。
- **`border-border-N` → `border-arco-N`**，只换颜色，不动同级的
  `border` / `border-solid` / `border-dashed` / `border-b` 宽度与样式类。
  ⚠️ 注意：`border` 只产出 `border-width`，本仓库没有把 `border-style` 设成 `solid` 的
  全局 reset，所以只写 `border border-arco-2` 的元素**仍然不画边框**。要画就得同时有
  `border-solid`。这是与颜色无关的另一个坑。
- **`border-b-base`**：按现场意图拆——四边用
  `border border-solid border-[var(--border-base)]`，只要下边框用
  `border-b border-b-solid border-b-[var(--border-base)]`。
- **反向断言的测试要一起改**：`knowledgeCreateCtaContrast.test.ts`、
  `scheduledTaskLayout.test.ts`、`KnowledgeControl.utils.test.ts` 曾用字符串断言把破写法
  钉死。它们已改成**用真实 UnoCSS 生成器编译该工具类、断言产出的声明是可解析的颜色**
  ——断意图而不是断字面量，这样再也钉不住一个坏值。新增同类断言请照这个写法。

改完跑 `bun run check:dead-css` + `bun test --cwd ui` + `bun run build:ui`，并在明暗两套
主题下目测：这类修复会让**原本不显示的颜色开始显示**，要留意红字落在红底上之类的新问题。

## 🚧 尚未清理：第五个死写法 `bg-bg-N` / `text-text` / `border-border`（重复前缀）

上面四条是门禁**当前覆盖**的写法。集成核对时又量到**同一类错误的第五个变体**，它还没有
清理、也**还没进门禁**（进门禁就会让 `bun run check` 立刻变红）：

theme 里没有名为 `bg`、`text`、`border` 的颜色，所以「**前缀写两遍**」的类名索引到空：

| 写法            | 实测产出                       | 本意                       |
| --------------- | ------------------------------ | -------------------------- |
| `bg-bg-1/2/3/4` | **0 条 CSS**（元素完全透明）   | `background: var(--bg-N)`  |
| `text-text`     | **0 条 CSS**                   | 正文色                     |
| `border-border` | **0 条 CSS**                   | 基础边框色                 |

```tsx
// ❌ 背景根本不画（浏览器里实测 background-color: rgba(0, 0, 0, 0)，明暗主题都一样）
<div className='bg-bg-2' />
// ✅ 去掉重复前缀即可，语义完全对应
<div className='bg-2' />        // → var(--bg-2)
<div className='bg-base' />     // → var(--bg-base)
<div className='text-t-primary' />
<div className='border-[var(--border-base)]' />
```

实测证据（三重）：真实生成器对 `bg-bg-1/2/3` 产出 ZERO CSS；`ui/dist` 全部 CSS 产物里
`.bg-bg-` 与 `.border-border` 规则数为 **0**；headless Firefox 里注入 `bg-bg-1/2/3` 量到
`rgba(0, 0, 0, 0)`，而 `bg-1/2/3` 量到 `rgb(250,250,250)/(242,242,243)/(228,228,230)`（亮）
与 `rgb(18,18,18)/(31,31,31)/(42,42,42)`（暗）。

存量：**88 处 / 32 文件**（`bg-bg-3` 32、`bg-bg-1` 30、`bg-bg-2` 23、`bg-bg-4` 2、`bg-bg-0` 1），
集中在 `pages/conversation/Preview/**`、`pages/browser/**` 与几个 settings 面板。

根因已经铲掉：`ui/src/renderer/styles/colors.ts` 的头注**曾把 `bg-bg-0` / `text-text` /
`border-border` 当作推荐写法**（连 `var(--color-bg-0)` 这个变量也不存在），这就是 88 处的
来源，现已改为正确写法并注明为什么那四个是死的。

清理配方（机械，但**会让 88 处背景色从"透明"变成"有色"**，属于真实视觉改动，需要在明暗
两套主题下过一遍 Preview / browser 两个区域，故独立成一次改动）：

1. `bg-bg-N` → `bg-N`（`bg-bg-0` 要单独看：`bg-0` 也是死的，theme 没有 `0` 键，应为 `bg-base`）；
2. 给 `scripts/check-dead-css-utilities.mjs` 加第五条禁令
   `/\b(?:bg-bg|text-text|border-border)(?:-[a-z0-9]+)?\b/`（`border-border-*` 已被第 2 条覆盖）；
3. 补 `--self-test` 用例，跑 `bun run check` + `bun test --cwd ui` + `bun run build:ui`。

## 🎨 使用方式

### 1. UnoCSS 原子类（推荐）✨

```tsx
// ✅ 背景色 - 简洁直观
<div className="bg-base">     // 主背景 (白色/黑色)
<div className="bg-1">        // 次级背景 (#F7F8FA)
<div className="bg-2">        // 三级背景 (#F2F3F5)
<div className="bg-brand">    // 品牌色背景 (#7583B2)

// ✅ 文本色 - 语义化
<div className="text-t-primary">    // 主要文字 (#1D2129)
<div className="text-t-secondary">  // 次要文字 (#86909C)
<div className="text-brand">        // 品牌色文字

// ✅ 边框色（⚠️ 不要写 border-b-base，见上文「-b- 方向陷阱」）
<div className="border-[var(--border-base)]">  // 基础边框 (#E5E6EB)
<div className="border-arco-2">                // Arco 边框 token
<div className="border-3">                     // --bg-3 分隔色

// ✅ 状态色阶（走项目规则，1-9）
<div className="text-danger-6 bg-primary-1 border-success-5">

// ✅ 品牌色系列
<div className="bg-aou-1">           // AOU 色板 1-10
<div className="hover:bg-brand-hover"> // 品牌色悬停
```

### 2. 内联样式（CSS 变量）

```tsx
<div style={{ backgroundColor: 'var(--bg-base)' }}>
<div style={{ color: 'var(--text-primary)' }}>
<div style={{ borderColor: 'var(--border-base)' }}>
<div style={{ backgroundColor: 'var(--brand)' }}>
```

## 📋 常见颜色映射表

| 旧值 (Hex) | UnoCSS 类                               | CSS 变量                | 说明            |
| ---------- | --------------------------------------- | ----------------------- | --------------- |
| `#FFFFFF`  | `bg-base`                               | `var(--bg-base)`        | 主背景          |
| `#F7F8FA`  | `bg-1`                                  | `var(--bg-1)`           | 次级背景/填充色 |
| `#F2F3F5`  | `bg-2`                                  | `var(--bg-2)`           | 三级背景        |
| `#E5E6EB`  | `bg-3` 或 `border-[var(--border-base)]` | `var(--border-base)`    | 边框/分隔线     |
| `#7583B2`  | `bg-brand` / `text-brand`               | `var(--brand)`          | 品牌色          |
| `#EFF0F6`  | `bg-aou-1` / `bg-brand-light`           | `var(--aou-1)`          | 品牌浅色背景    |
| `#E5E7F0`  | `bg-aou-2`                              | `var(--aou-2)`          | AOU 色板 2      |
| `#1D2129`  | `text-t-primary`                        | `var(--text-primary)`   | 主要文字        |
| `#86909C`  | `text-t-secondary` / `bg-6`             | `var(--text-secondary)` | 次要文字        |
| `#165DFF`  | `bg-primary` / `text-primary`           | `var(--primary)`        | 主色调          |

## 🔄 迁移步骤

1. **搜索**硬编码颜色：`bg-#`, `text-#`, `color-#`, `border-#`
2. **查表**对应的主题变量
3. **替换**为 UnoCSS 类
4. **跑门禁** `bun run check:dead-css`（别把新写法写成上文四个死写法）
5. **测试**明暗主题切换

## 💡 迁移示例

### Before (硬编码):

```tsx
<div className='bg-#EFF0F6 hover:bg-#E5E7F0'>
  <span className='text-#1D2129'>文本</span>
  <div className='border border-#E5E6EB'></div>
</div>
```

### After (主题变量):

```tsx
<div className='bg-aou-1 hover:bg-aou-2'>
  <span className='text-t-primary'>文本</span>
  <div className='border border-[var(--border-base)]'></div>
</div>
```

### 常见模式:

```tsx
// ❌ 不推荐
<div className="bg-#F7F8FA text-#86909C border-#E5E6EB">

// ✅ 推荐
<div className="bg-1 text-t-secondary border-[var(--border-base)]">
```

## 🎯 快速参考

- **背景**: `bg-base`, `bg-1`, `bg-2`, `bg-3`
- **文字**: `text-t-primary`, `text-t-secondary`, `text-t-disabled`
- **边框**: `border-[var(--border-base)]`, `border-arco-1` ~ `border-arco-4`, `border-3`
  （❌ 不是 `border-b-base` / `border-b-light` / `border-` + `border-N`）
- **品牌**: `bg-brand`, `bg-brand-light`, `bg-brand-hover`
- **状态**: `bg-primary`, `bg-success`, `bg-warning`, `bg-danger`
- **状态色阶**: `text-danger-6`, `bg-primary-1` ~ `bg-primary-9`（❌ 不带 `/NN` 透明度后缀）
  （❌ 不是 `text-[rgb(var(--danger-6))]`）
- **AOU色板**: `bg-aou-1` ~ `bg-aou-10`
