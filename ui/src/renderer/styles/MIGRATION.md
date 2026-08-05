# Theme Color Migration Guide

> 门禁 / Gate: `bun run check:dead-css`（`scripts/check-dead-css-utilities.mjs`）
> 分两层：**七条正则**拦住本文「⚠️ 七个死写法」一节里的任何一处使用；**生成器兜底**
> 再把源码里所有「长得像颜色/装饰工具类」的 token 喂给真实 UnoCSS 生成器，产出 0 条
> CSS 就失败——所以还没被命名的死写法也拦得住（见文末「生成器兜底」一节）。
> 存量已清零，棘轮基线已删除，现在是一刀切禁令。
> Two layers: seven regexes for the named forms, plus a generator backstop that
> fails any utility-shaped token which compiles to nothing. The sweep is done and
> the ratchet baseline is gone: these forms are now simply banned, anywhere under
> `ui/src`.

## ⚠️ 七个死写法 / Seven dead forms (measured, not guessed)

下面七种写法在本仓库**编译不出 CSS，或编译成错的属性**。都用真实 UnoCSS
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

**四个方向都会犯同一个错。** 门禁最初只拦 `-b-`，于是 `ExecutionAdjustBox.tsx` 里一处
「`border-t-` + `base`」的顶部分隔线漏了过去（实测产出 `border-top-color: var(--bg-base)`，
分隔线和背景同色）。现在规则按 `border-[trblxy]-(base|light)` 匹配。

```tsx
// ❌ 落到 --bg-* 上，不是基础边框色
<div className='border border-b-base' />

// ✅ 四边基础边框（注意必须带 border-solid，本仓库没有全局 border-style reset）
<div className='border border-solid border-[var(--border-base)]' />
// ✅ 只要下边框（宽度/样式/颜色分开写）
<div className='border-b border-b-solid border-b-[var(--border-base)]' />
```

同一个陷阱的**数字键**变体现在单列为第 6 条：`border-b-2` 不是「下边框 2px」，而是
`border-bottom-color: var(--bg-2)`。

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

### 5. `bg-bg-N` / `text-text-*` —— 前缀写两遍，产出 0 条 CSS

`uno.config.ts` 把 `backgroundColors` 合进 `theme.colors` 时用的是**数字键** `1..10`
（`3: 'var(--bg-3)'`）。所以 `bg-3` 命中键 `3`，而 `bg-bg-3` 查的是一个叫「`bg-3`」的
颜色——theme 里没有名为 `bg` 或 `text` 的颜色，重复前缀索引到空：

```css
/* 真实生成器实测 / measured with the real generator */
/* 输入 bg-bg-0 bg-bg-1 bg-bg-2 bg-bg-3 bg-bg-4 hover:bg-bg-3 dark:bg-bg-2 */
(ZERO CSS)

/* 输入 bg-1 bg-2 bg-3 bg-4 bg-base hover:bg-3 */
.bg-1 { background-color: var(--bg-1); }
.bg-2 { background-color: var(--bg-2); }
.bg-3 { background-color: var(--bg-3); }
.bg-4 { background-color: var(--bg-4); }
.bg-base { background-color: var(--bg-base); }
.hover\:bg-3:hover { background-color: var(--bg-3); }
```

```tsx
// ❌ 背景根本不画（浏览器里量到 background-color: rgba(0, 0, 0, 0)，明暗主题都一样）
<div className='bg-bg-2' />
// ✅ 去掉重复前缀即可，语义完全对应
<div className='bg-2' />        // → var(--bg-2)
<div className='text-t-primary' />
```

⚠️ `bg-0` **也是死的**（theme 没有 `0` 键，实测 0 条 CSS）。要主背景写 `bg-base`。
根因已经铲掉：`ui/src/renderer/styles/colors.ts` 的头注**曾把这三种写法当作推荐写法**
（连 `var(--color-bg-0)` 这个变量也不存在），现已改为正确写法并注明为什么它们是死的。

### 6. `border-b-2` / `border-t-4` —— 带方向的数字是**颜色**，不是宽度

第 3 条的数字键变体，单列出来是因为它造成的是**可见的产品缺陷**：选中态 Tab 的下划线
整条不存在。UnoCSS 先吃掉方向，再拿数字去 `theme.colors` 查到 `backgroundColors[N]`：

```css
/* 输入 border-b-1 border-b-2 border-b-3 border-b-4 */
.border-b-1 { border-bottom-color: var(--bg-1); }
.border-b-2 { border-bottom-color: var(--bg-2); }
.border-b-3 { border-bottom-color: var(--bg-3); }
.border-b-4 { border-bottom-color: var(--bg-4); }

/* 输入 border-b-0 border-b-2px border-b-4px —— 这三个才是宽度 */
.border-b-0 { border-bottom-width: 0px; }
.border-b-2px { border-bottom-width: 2px; }
.border-b-4px { border-bottom-width: 4px; }
```

所以 `border-b-4 border-brand` 是**两个颜色类**：没有宽度、没有样式，而且 `border-b-4`
的 `border-bottom-color` 还会盖掉 `border-brand`。下划线三个属性一个都不成立。

```tsx
// ❌ 选中态下划线完全不存在
<div className='text-brand border-b-4 border-brand' />
// ✅ 宽度 / 样式 / 颜色分三条写
<div className='text-brand border-b-4px border-b-solid border-brand' />
```

⚠️ **不带方向**的 `border-2` 同样是颜色（`border-color: var(--bg-2)`），但它在本仓库是
**文档化过的合法写法**（本文推荐 `border border-3` 当分隔色），所以门禁只拦带方向的形式。
`border-[trbl]-0` 是真的宽度（`border-bottom-width: 0px`），也不在禁令内。

### 7. 有边框宽度 + 有边框颜色，却没有 `border-style`

**本仓库没有全局 border reset。** 唯一的 preflight 是 `* { color: inherit }`，
`@unocss/reset/tailwind.css` **没有**被引入。于是 `border-style` 保持 CSS 初始值 `none`：

```css
/* 输入 border-b border-arco-2 —— 宽度和颜色都有，但没有 style */
.border-b { border-bottom-width: 1px; }
.border-arco-2 { border-color: var(--color-border-2); }
/* → border-style 仍是初始值 none，一个像素都不画 */

/* 输入 border-solid / border-b-solid */
.border-solid { border-style: solid; }      /* 四边 */
.border-b-solid { border-bottom-style: solid; } /* 只有下边 */
```

**带方向的宽度必须配同方向的样式类。** 只有下边框宽度却写了 `border-solid`，另外三边
拿不到宽度类，会回落到 CSS 初始值 `border-width: medium`（≈3px）并**凭空画出三条边**；
这正是仓库里那套 `border-t border-solid border-[...] border-l-0 border-r-0 border-b-0`
显式操作的来历。最简写法是全方向：

```tsx
// ❌ 一个像素都不画
<div className='border border-arco-2' />
<div className='border-b border-[var(--border-base)]' />

// ✅ 四边
<div className='border border-solid border-arco-2' />
// ✅ 只要下边框（宽度/样式同方向，颜色可以不带方向——另外三边没有宽度，不会画出来）
<div className='border-b border-b-solid border-arco-2' />
// ✅ 颜色也带方向（FileChangeList.tsx 里一直是这么写的）
<div className='border-b border-b-solid border-b-[var(--border-base)]' />
```

#### 为什么不加全局 border reset / Why no global border reset

Tailwind 的 preflight 会写 `* { border-width: 0; border-style: solid; }`，加上它这七条里的
第 7 条就自动消失。**这个方案评估过，明确否决：**

- 影响面无法界定。这条 `*` 规则会同时**抹掉原生表单控件的默认边框**（`input` /
  `select` / `textarea` / `button` / `fieldset` / `table`），而本仓库还大量使用 Arco
  Design 组件与原生控件混排，1000+ 个文件都在这条规则的作用域里，没法逐一目测回归。
- `border-width: 0` 会让所有**只写了颜色、宽度靠 CSS 初始值 `medium` 撑着**的元素
  （本文第 6 条那 20 多处 `border-2 border-solid border-...`）从「3px 边框」变成「无边框」，
  是另一批静默视觉回归。
- 收益可以用更小的代价拿到：逐站点补同方向的样式类 + 一条门禁规则。影响面有界、可验证、
  出问题能定位到具体那一行。
- A global `*` reset would strip default borders from native form controls across
  1000+ files — an unbounded blast radius. Per-site directional classes plus a gate
  rule is bounded and verifiable, so that is what we did.

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

## ✅ 第 5-7 条的存量清理 / Sweep for forms 5-7

这三条是「重复前缀」与「边框三属性」两族，和上面四条一起清完了，门禁同步补上。

| 写法                                                  | 文件数 | 出现次数 |
| ----------------------------------------------------- | ------ | -------- |
| `bg-bg-N`（写法 5）                                   | 31     | 87       |
| `border-[trbl]-N` 当宽度用（写法 6）                  | 3      | 6        |
| 有宽度 + 有颜色 + 没 `border-style`（写法 7）         | 30     | 63       |

`bg-bg-N` 的分布：`bg-bg-3` 32、`bg-bg-1` 30、`bg-bg-2` 23、`bg-bg-4` 2，集中在
`pages/conversation/Preview/**`、`pages/browser/**` 与几个 settings 面板。
（第 88 处 `bg-bg-0` 在 `styles/colors.ts` 的头注里，是讲解反例，不渲染，故保留。）

清理时定下的判断，后续改动请沿用：

- **`bg-bg-N` → `bg-N`**，逐个确认过语义：命中的全是「面板底色 / 行悬停底色 / 代码块底色」，
  正好对应 `--bg-N` 色阶，所以只删掉重复的前缀，没有改任何一处的色号。没有改成
  `bg-fill-N`：`--color-fill-*` 是半透明 rgba，嵌套时会叠色变浑，这些站点都是不透明面板。
- **写法 7 一律补同方向的样式类**，不动作者选的颜色：`border` → 加 `border-solid`，
  `border-b` → 加 `border-b-solid`。**不要**给带方向的宽度配无方向的 `border-solid`。
- **颜色本身就指错的，一并换掉**（这几处的「颜色」落到的是背景变量或自己的底色）：
  - `border-base` → `border-[var(--border-base)]`（`base` 键指向 `--bg-base`，是主背景）
  - 「`border-t-` + `base`」→ `border-t-[var(--border-base)]`（同一个陷阱的方向版）
  - `border border-2` + `bg-2` → `border border-solid border-arco-2`
    （`border-2` 是 `--bg-2`，和这张卡片自己的底色同色，等于没有边框）
  - `border-3 border-fill-3` 的环形 spinner（UpdateModal）→
    `border-3px border-solid border-[var(--color-fill-3)]`：`border-3` 是颜色、
    `border-fill-3` 在 theme 里根本不存在，两层圆环一条边都没画出来。
- **variant 前缀要跟着走**：`[&_.arco-collapse-item-content]:!border-t` 补的是
  `[&_.arco-collapse-item-content]:!border-t-solid`，不是裸的 `border-t-solid`
  ——后者落在外层元素上，descendant 依然没有样式。

### 门禁覆盖不到的边界 / What the gate cannot see

写法 7 的检测器**只在同一个字符串字面量里判断**（这是为了不误报：样式可能来自
兄弟 variant group、同串别处的 `border-solid`、或组件自己的 CSS）。因此这两类会漏：

1. **class 列表被拆进数组/多个字面量**：
   `['... rounded-8px border', isSelected ? '!border-primary-6' : 'border-[...]']`
   宽度在第一段、颜色在第二段，检测器看不到它们在同一个元素上。
   `CreateStudio/TypeRail.tsx` 的图标框就是这样漏的（已手工修掉）。
2. **样式来自组件自己的 CSS**（`.foo { border-style: solid }`）——这种其实是对的，
   漏报正是我们想要的行为。

`.css` 文件里的 `@apply` 同理不在扫描范围内；当前 `ui/src` 下**没有任何** `@apply`
（实测 grep 为 0），所以今天没有实际漏洞。

### 还留在树上的两个近亲问题 —— 已清 / Two related issues, now cleared

这两族曾经被「量到但故意没改」，理由是需要视觉决策而非机械替换。后来都做完了：

- **不带方向的 `border-N` 被当宽度用**：实清 **34 处**（远多于当初量到的 23 处）。
  判据不是「一律禁掉 `border-N`」，而是**同一串里有没有别的 token 提供真实宽度**：
  `border border-solid border-3` 与 `border-3 b b-solid` 是**合法**的（`border` / `b`
  给了 1px），`border-1 border-solid border-[var(--color-border-2)]` 是**坏的**（两个颜色 +
  样式、零宽度，渲染成 `medium`≈3px）。修法是补显式单位：`border-1px` / `b-1px` / `border-2px`。
  顺带发现两件事：`.border-2` 的规则排在 `.border-[var(...)]` **之后**，所以
  `CharacterPicker` 的选中描边在两种状态下都不可见、6 个转圈的轨道颜色被静默改写；
  以及 `ConversationTerminalPanel` 的 `border-0 border-t border-solid` 其实是**对的**——
  `.border-0` 先出、`.border-t` 后出，只有上边画。
- **theme 里不存在的边框色名**：`border-line` / `border-line-2` / `border-fill-3` /
  `b-color-border-2` / `b-border-2` / `bg-border-N` / `color-text-N` / `text-error` /
  `text-t-error` 共 **27 处**全部换成能编译的写法（并补上缺失的 `border-style`）。

### 生成器兜底 / The generator backstop — 为什么不再加第八条正则

上面七条正则各对着一个**已知**的死写法族。问题是这些族全是机械扫描才找出来的，
有几个已经活了好几个月：`border-line` 11 处、`divide-border-2` 7 处、`text-error`、
`bg-border-2`、`b-color-border-2`、`color-text-3`、`bg-fill-1/60` ——**没有一条能被当时的
七条正则看见**。再加第八、第九条只会继续落后于下一个拼错的颜色名。

所以门禁多了一层不枚举错误写法的检查：把源码里**长得像颜色/装饰工具类**的 token
喂给真实 UnoCSS 生成器，产出 0 条 CSS 就失败。这一下覆盖「编译出零 CSS」整类，
包括还没被写出来的（实测连随手编的 `ring-nope-9` 都会被抓住）。

判别轴是**首段是不是颜色/装饰前缀**，不是「有没有在样式表里定义过」。后者试过，
不收敛：`nomi-input` / `katex-display` / `markdown-shadow` / BEM 钩子名同样产出 0 条 CSS
（它们的样式来自手写 CSS 或 CSS module），逐条加白名单意味着项目每加一个语义类名
门禁就红一次。而前缀集合是封闭的（取自 `uno.config.ts` 里真正走颜色/装饰管道的规则），
`border-line` 命中，`nomi-input` 永远不会。

两条踩过的坑，都写进了自检：

- **数值后缀不能排除**，裸的和带单位的都不行。裸数字 `text-3` / `bg-1` 是本仓库自有的
  颜色规则（数字键映射到 `--bg-N`），排掉它们等于让真颜色类到不了生成器面前；带单位的
  `ring-2px` / `border-1.5px` 是宽度类，而宽度类**本来就会编译通过**，排除它们不解决任何
  误报，只白送一块盲区。
- **非 class 字符串要挡在门外**。class 列表提取器是按属性名/函数名抓字符串字面量的，
  所以 MIME 类型（`text/plain;charset=utf-8`）、CSS 属性名（`border-color`、
  `stroke-dashoffset`）、`box-sizing` 的值（`border-box`）、SVG data-URI 碎片、以及恰好以
  `text-` 开头的英文散文（`text-only.`）都会混进 token 流。这 12 个就是兜底层的**全部**
  误报——判别轴是对的，是这些 token 不该进来。

`--self-test` 因此分三段：七条正则 68 例、兜底层的形状筛选 33 例、兜底层的产出判定
16 例（后者用真实生成器确认已知死写法真的产出 0 条 CSS、已知活写法真的产出 CSS，
防止检测器自身失效后静默通过）。全量 186 个 token 一次 `generate` 调用判完，实测 24ms，
所以这一层不会让 `bun run check` 变慢。

改完跑 `bun run check` + `bun test --cwd ui` + `bun run build:ui`，并在明暗两套主题下目测：
这些修复会让**原本不存在的边框和背景开始出现**，要留意分隔线扎堆、或原本「无框」的卡片
突然有框之类的观感问题。

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
// ⚠️ 边框要画出来必须三件齐：宽度 + 样式 + 颜色。本仓库没有全局 border-style reset。
<div className="border border-solid border-[var(--border-base)]">  // 基础边框 (#E5E6EB)
<div className="border border-solid border-arco-2">                // Arco 边框 token
<div className="border border-solid border-3">                     // --bg-3 分隔色
<div className="border-b border-b-solid border-arco-2">            // 只要下边框（同方向）

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
  <div className='border border-solid border-[var(--border-base)]'></div>
</div>
```

### 常见模式:

```tsx
// ❌ 不推荐
<div className="bg-#F7F8FA text-#86909C border-#E5E6EB">

// ✅ 推荐
<div className="bg-1 text-t-secondary border border-solid border-[var(--border-base)]">
```

## 🎯 快速参考

- **背景**: `bg-base`, `bg-1`, `bg-2`, `bg-3`（❌ 不是 `bg-bg-N`，也不是 `bg-0`）
- **文字**: `text-t-primary`, `text-t-secondary`, `text-t-disabled`
- **边框**: 宽度 + 样式 + 颜色三件齐 —— `border border-solid border-[var(--border-base)]`,
  `border-arco-1` ~ `border-arco-4`, `border-3`；只要下边框写
  `border-b border-b-solid ...`
  （❌ 不是 `border-b-base` / `border-b-light` / `border-` + `border-N`；
  ❌ `border-b-2` 是下边框**颜色**不是 2px，宽度写 `border-b-2px`）
- **品牌**: `bg-brand`, `bg-brand-light`, `bg-brand-hover`
- **状态**: `bg-primary`, `bg-success`, `bg-warning`, `bg-danger`
- **状态色阶**: `text-danger-6`, `bg-primary-1` ~ `bg-primary-9`（❌ 不带 `/NN` 透明度后缀）
  （❌ 不是 `text-[rgb(var(--danger-6))]`）
- **AOU色板**: `bg-aou-1` ~ `bg-aou-10`
