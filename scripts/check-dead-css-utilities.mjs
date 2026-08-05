#!/usr/bin/env node
/**
 * 死 CSS 工具类禁令 / Dead CSS utility-class prohibition
 *
 * 七种写法在本仓库编译不出（或编译错）CSS。全部用真实 UnoCSS 生成器实测过，
 * 不是推测。All seven forms below were measured with the real UnoCSS generator
 * (ui/uno.config.ts), not guessed.
 *
 * 这个检查曾是「棘轮」：存量 95 个文件 / 276 处记在 BASELINE 里被放行。存量已
 * 全部清理完毕，BASELINE 随之删除——现在是一刀切的禁令，任何一处都会失败。
 * This used to be a ratchet with a 95-file / 276-occurrence BASELINE. The sweep
 * is done and the baseline is gone: any occurrence anywhere now fails.
 *
 * 1) ramp —— `{text,bg,border}-[rgb(var(--RAMP-N))]`。UnoCSS 把中括号里的值当颜色
 *    处理并注入 slash-alpha：
 *      color: rgb(var(--danger-6) / var(--un-text-opacity))
 *    而这些 ramp 变量是「逗号分隔三元组」（Arco `--red-6: 245,63,63`，预设写
 *    `--primary-6: 232, 23, 74;`），于是 `rgb(245,63,63 / 1)` 无法解析，浏览器
 *    整条声明作废——元素静默保留继承来的颜色。
 *    ✅ primary/success/warning/danger 改用项目自带规则：`text-danger-6` /
 *       `bg-primary-6` / `border-success-5`（uno.config.ts 的
 *       ^(bg|text|border)-(primary|success|warning|danger)-([1-9])$ 规则输出合法的
 *       `color: rgb(var(--danger-6))`）。
 *    ✅ 其它色板（link / arcoblue / purple / orange / cyan / gray …）没有项目规则，
 *       写成自带 alpha 的 `text-[rgba(var(--arcoblue-6),1)]`，UnoCSS 就不再注入。
 *    ⚠️ 显式 `rgba(var(--x), 0.12)` 自带 alpha，是合法写法，本检查不拦。
 *    ⚠️ 注意本条也覆盖 `bg-[rgb(var(--success-6))]/12` 这种「中括号 + 斜杠透明度」
 *       的变体——它同样产出非法的 `rgb(... / 0.12)`。
 *
 * 2) deadBorder —— `border-border-N`。theme 里没有名为 `border` 的颜色，
 *    完全不产出 CSS。✅ 改用 `border-arco-N`（Arco --color-border-N）或
 *    `border-N`（--bg-N 色阶）或 `border-[var(--border-base)]`。
 *
 * 3) dirNamedBorder —— `border-b-base` / `border-t-light` 之类。UnoCSS 先把 `-b-`
 *    解析成 bottom 方向，再拿剩下的键查 theme，所以产出的是基于 --bg-* 的
 *    `border-bottom-color`，而不是本意的基础边框色。四个方向都会犯同一个错：
 *    `border-t-base` 曾从棘轮时代一路漏到 ExecutionAdjustBox.tsx。
 *    ✅ 四边：`border border-solid border-[var(--border-base)]`；
 *       仅下边框：`border-b border-b-solid border-b-[var(--border-base)]`。
 *
 * 4) rampSlash —— `{bg,text,border}-(primary|success|warning|danger)-N/NN`。项目规则
 *    以 `$` 结尾锚定，末尾的 `/NN` 让整个类名一条规则都匹配不上，产出 0 条 CSS。
 *    这正是「把写法 1 机械 sed 成项目规则类」时最容易踩的坑（`bg-success-6/12`）。
 *    ✅ 需要透明度就写 `bg-[rgba(var(--success-6),0.12)]`。
 *
 * 5) doublePrefix —— `bg-bg-N` / `text-text-*`（前缀写两遍）。theme 里没有名为
 *    `bg` / `text` 的颜色，`bg-bg-1` 查的是一个叫「bg-1」的颜色，查不到，产出 0 条
 *    CSS——元素完全透明，明暗主题都一样。`border-border-*` 由写法 2 覆盖。
 *    ✅ 去掉重复前缀：`bg-1` / `bg-base` / `text-t-primary`。
 *    ⚠️ `bg-0` 也是死的（theme 没有 `0` 键），要主背景写 `bg-base`。
 *
 * 6) dirNumBorder —— `border-b-2` / `border-t-4` 这类「带方向的数字」。作者想写的是
 *    「下边框 2px」，UnoCSS 却先吃掉 `-b-` 当方向、再拿 `2` 去 theme.colors 里查到
 *    `backgroundColors[2]`，产出 `border-bottom-color: var(--bg-2)`：没有宽度、没有
 *    样式，还会把同级的 `border-primary` 覆盖掉。选中态下划线因此整个不存在。
 *    ✅ 宽度写 `border-b-2px`，样式写 `border-b-solid`，颜色单独写。
 *    ⚠️ 不带方向的 `border-2` 同样是颜色（`border-color: var(--bg-2)`），但它是本仓库
 *       **文档化过的合法颜色写法**（MIGRATION.md 推荐 `border border-3`），所以本条只
 *       拦带方向的形式。`border-b-0` 之类的 0 值是真的宽度（`border-bottom-width:0px`），
 *       不在禁令内。
 *
 * 7) borderNoStyle —— 同一个 class 列表里既有边框**宽度**又有边框**颜色**，却没有任何
 *    边框**样式**。本仓库唯一的 preflight 是 `* { color: inherit }`，没有引入
 *    `@unocss/reset/tailwind.css`，所以 `border-style` 保持 CSS 初始值 `none`——
 *    `border-b border-arco-2` 一个像素都画不出来。
 *    ✅ 无方向：`border border-solid border-arco-2`；
 *       带方向：`border-b border-b-solid border-arco-2`（**必须**用同方向的
 *       `border-b-solid`：`border-solid` 会给四边都上样式，而另外三边没有宽度类，
 *       会回落到 CSS 初始值 `medium`≈3px，凭空多出三条边）。
 *
 * 扫描方式 / How the scan works：
 *   - 写法 1-4 按「整行正则」匹配，沿用历史行为。
 *   - 写法 5-7 需要知道「哪些类写在同一个 class 列表里」，所以先用一个小词法器把源码
 *     里的**字符串字面量**取出来（模板字符串按 `${}` 切段），再逐串分析。注释被跳过：
 *     注释里的类名不会落到任何元素上，`styles/colors.ts` 与 `WorkspaceFolderSelect.tsx`
 *     的头注就是拿死写法当反例讲解的。
 *   - 写法 7 只在**同一串**里判断，故意留了两类漏网：class 列表被拆进数组/多个字面量时
 *     （`['... border', isX ? 'border-primary' : '...']`），以及样式来自组件自己的 CSS 时。
 *     宁可漏，不可误报。Rule 7 is deliberately single-string-scoped: cross-string class
 *     lists and styles supplied by component CSS are false negatives we accept.
 *
 * 扫描范围：ui/src 下的 .ts/.tsx/.css，排除 *.test.ts(x) 与 *.d.ts —— 测试文件里
 * 这些字符串是「断言源码里不含/曾含某写法」的字面量，不会渲染成 CSS。.md 同理不扫，
 * 迁移指南 ui/src/renderer/styles/MIGRATION.md 需要把错误写法作为反例展示。
 *
 * 用法 / Usage:
 *   bun scripts/check-dead-css-utilities.mjs             # 校验，发现违规 exit 1
 *   bun scripts/check-dead-css-utilities.mjs --self-test # 校验器自测
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const SCAN_DIR = join(ROOT, 'ui', 'src');
const SELF = 'scripts/check-dead-css-utilities.mjs';

/** UnoCSS 认得的边框方向 / Border directions UnoCSS parses before the theme lookup */
const DIR = '(?:t|r|b|l|x|y)';
/** 边框样式关键字 / border-style keywords */
const BORDER_STYLE = new RegExp(`^border(?:-${DIR})?-(?:solid|dashed|dotted|double|hidden|none|groove|ridge|inset|outset)$`);
/** 真正产出宽度的写法；`-0` 单独归类，它是「重置」而不是「想画一条边」 */
const BORDER_WIDTH = new RegExp(`^border(?:-${DIR})?(?:-(?:\\d*\\.?\\d+(?:px|rem|em)|\\[[^\\]]*\\]))?$`);
const BORDER_ZERO = new RegExp(`^border(?:-${DIR})?-0$`);
/**
 * 已知会产出 border-color 的写法。故意用白名单而不是「除宽度/样式之外都算颜色」：
 * 未知写法（比如 theme 里根本没有的 `border-line`）算不出颜色，写法 7 就不会误报。
 * Deliberately a whitelist — unknown tokens are not treated as colours, so rule 7
 * stays quiet instead of guessing.
 */
const BORDER_COLOR_NAMES =
  '(?:arco-[1-4]|(?:primary|success|warning|danger|info)(?:-(?:[1-9]|light-[1-4]))?|brand(?:-(?:light|hover))?|base|hover|active|transparent|current|white|black|aou-(?:[1-9]|10)|(?:red|green|blue|yellow|gray|grey|orange|purple|cyan|pink|indigo|teal|amber|lime|emerald|sky|violet|fuchsia|rose|slate|zinc|neutral|stone)-\\d{2,3})';
const BORDER_COLOR = new RegExp(
  `^border(?:-${DIR})?-(?:${BORDER_COLOR_NAMES}|[1-9]|10|\\[[^\\]]*(?:var\\(|rgba?\\(|#|color:|color-mix)[^\\]]*\\])(?:/\\d+)?$`,
);

/** 七种死写法及其有效替代 / The seven dead forms and their valid replacements */
const FORMS = {
  ramp: {
    scan: 'line',
    // 只匹配整个中括号值恰好是 rgb(var(--name-N)) 的情形；rgba(...) 与
    // shadow-[...rgb(var(--x))...] 都不匹配。
    //   * 色板名不限于四个语义 ramp：arcoblue / purple / orange / cyan / gray 等
    //     Arco 色板同样是逗号三元组，犯的是同一个错。
    //   * 前缀也不限于 text/bg/border：ring- / outline- / border-t- / border-b- /
    //     divide- … 任何走 UnoCSS 颜色管道的工具类都会被注入 slash-alpha。
    //     实测证据：`ring-[rgb(var(--primary-6))]` 产出
    //     `--un-ring-color: rgb(var(--primary-6) / var(--un-ring-opacity))`。
    re: /(?<![\w-])[a-z]+(?:-[a-z])?-\[rgb\(var\(--[a-z]+-\d\)\)\]/g,
    label: '<任意颜色工具类>-[rgb(var(--RAMP-N))] 注入 slash-alpha，整条声明被浏览器丢弃',
    fix: 'primary/success/warning/danger 的 text/bg/border 改用 text-danger-6 / bg-primary-6 这类项目规则类；其它色板、或 ring-/outline-/border-t- 这些规则覆盖不到的前缀，写自带 alpha 的 ring-[rgba(var(--primary-6),1)]；需要透明度写 rgba(var(--primary-6), 0.12)',
  },
  deadBorder: {
    scan: 'line',
    // 后缀不限于数字：theme 里根本没有名为 `border` 的颜色，所以 `border-border-base`
    // 与 `border-border-2` 一样产出 0 条 CSS。这条规则最初只写了 `\d`，于是
    // HTMLViewer.tsx 里 3 处 `border-border-base` 从棘轮时代一路漏到禁令时代。
    // The suffix is NOT limited to digits: there is no colour named `border` in the
    // theme at all, so `border-border-base` emits zero CSS exactly like
    // `border-border-2`. This regex used to be `\d`-anchored, which is how 3 live
    // `border-border-base` sites survived both the ratchet and the sweep.
    re: /\bborder-border-[a-z0-9]+\b/g,
    label: 'border-border-*：theme 无 border 颜色，产出 0 条 CSS',
    fix: '改用 border-arco-N（--color-border-N）或 border-N（--bg-N）或 border-[var(--border-base)]；注意本仓库没有全局 border-style 重置，宽度类要配 border-solid 才画得出来',
  },
  dirNamedBorder: {
    scan: 'line',
    // 四个方向一起拦：`-t-`/`-r-`/`-l-` 与 `-b-` 走的是同一段方向解析代码，
    // 只拦 `-b-` 的话 `border-t-base` 就会漏（实测漏过一次）。
    re: /\bborder-[trblxy]-(?:base|light)\b/g,
    label: 'border-{t,r,b,l}-base / -light：方向先被吃掉，剩下的键落到 --bg-* 上',
    fix: '四边写 border border-solid border-[var(--border-base)]；仅下边框写 border-b border-b-solid border-b-[var(--border-base)]',
  },
  rampSlash: {
    scan: 'line',
    re: /\b(?:bg|text|border)-(?:primary|success|warning|danger)-[1-9]\/\d+\b/g,
    label: '{bg,text,border}-RAMP-N/NN：项目规则以 $ 锚定，斜杠透明度让它一条规则都匹配不上，产出 0 条 CSS',
    fix: '需要透明度改写成 bg-[rgba(var(--success-6),0.12)]',
  },
  doublePrefix: {
    scan: 'token',
    // `bg-bg-3` 查的是一个叫「bg-3」的颜色，theme 里只有键 `3`，所以 0 条 CSS。
    // 也拦 `bg-bg-base` / `text-text-primary` 这些命名后缀。
    token: /^(?:bg-bg|text-text)-[a-z0-9]+$/,
    label: 'bg-bg-* / text-text-*：前缀写两遍，索引到不存在的颜色，产出 0 条 CSS',
    fix: '去掉重复的前缀：bg-bg-3 → bg-3、bg-bg-0 → bg-base（theme 没有 0 键）、text-text-primary → text-t-primary',
  },
  dirNumBorder: {
    scan: 'token',
    token: new RegExp(`^border-${DIR}-(?:[1-9]|10)$`),
    label: 'border-{t,r,b,l}-N：这是「方向 + --bg-N 颜色」，不是 N px 宽度，没有宽度也没有样式',
    fix: '宽度写 border-b-2px，样式写 border-b-solid，颜色另写一条（如 border-primary）；border-b-0 是真的宽度，不受此限',
  },
  borderNoStyle: {
    scan: 'list',
    list: (tokens) => {
      const width = tokens.filter((t) => !BORDER_ZERO.test(t) && !BORDER_COLOR.test(t) && BORDER_WIDTH.test(t));
      const color = tokens.filter((t) => !BORDER_ZERO.test(t) && BORDER_COLOR.test(t));
      const style = tokens.filter((t) => BORDER_STYLE.test(t));
      if (!width.length || !color.length || style.length) return null;
      return `${width[0]} ${color[0]}`;
    },
    label: '同一串里有边框宽度 + 边框颜色但没有 border-style：本仓库没有全局 border reset，一个像素都不画',
    fix: '无方向补 border-solid；带方向必须补同方向的 border-b-solid / border-t-solid（border-solid 会让没有宽度类的另外三边回落到 medium≈3px）',
  },
};

function* walk(dir) {
  for (const name of readdirSync(dir).sort()) {
    if (name === 'node_modules' || name === 'dist' || name.startsWith('.')) continue;
    const full = join(dir, name);
    if (statSync(full).isDirectory()) yield* walk(full);
    else if (/\.(tsx?|css)$/.test(name) && !/\.test\.tsx?$/.test(name) && !/\.d\.ts$/.test(name)) yield full;
  }
}

/**
 * 取出源码里的字符串字面量（模板字符串按 `${}` 切段），跳过注释。
 * Pull string literals out of the source (template literals split at `${}`),
 * skipping comments — a class name in a comment styles nothing.
 */
function extractClassLists(source) {
  const out = [];
  const n = source.length;
  const tplBraces = [];
  let mode = 'code';
  let line = 1;
  let bufLine = 1;
  let buf = '';
  let i = 0;
  const flush = () => {
    if (buf.trim()) out.push({ line: bufLine, text: buf });
    buf = '';
  };
  while (i < n) {
    const c = source[i];
    if (mode === 'code') {
      if (c === '\n') {
        line += 1;
        i += 1;
      } else if (c === '/' && source[i + 1] === '/') {
        while (i < n && source[i] !== '\n') i += 1;
      } else if (c === '/' && source[i + 1] === '*') {
        i += 2;
        while (i < n && !(source[i] === '*' && source[i + 1] === '/')) {
          if (source[i] === '\n') line += 1;
          i += 1;
        }
        i += 2;
      } else if (c === "'" || c === '"' || c === '`') {
        if (c === '`') tplBraces.push(0);
        mode = c;
        bufLine = line;
        i += 1;
      } else if (c === '{' && tplBraces.length) {
        tplBraces[tplBraces.length - 1] += 1;
        i += 1;
      } else if (c === '}' && tplBraces.length) {
        // 插值结束，回到模板字符串 / the interpolation ended, resume the template
        if (tplBraces[tplBraces.length - 1] === 0) {
          mode = '`';
          bufLine = line;
        } else {
          tplBraces[tplBraces.length - 1] -= 1;
        }
        i += 1;
      } else {
        i += 1;
      }
      continue;
    }
    // 字符串内部 / inside a string literal
    if (c === '\\') {
      buf += ' ';
      i += 2;
    } else if (c === '\n') {
      // 单/双引号字符串不能跨行：走到这里说明前面那个引号其实不是字符串起点
      // （最常见的是 JSX 正文里的英文撇号），就地收尾，避免把整份文件当成一串。
      // A `'`/`"` literal cannot span lines — bail out instead of swallowing the file.
      line += 1;
      if (mode === '`') {
        buf += ' ';
      } else {
        flush();
        mode = 'code';
      }
      i += 1;
    } else if (mode === '`' && c === '$' && source[i + 1] === '{') {
      flush();
      mode = 'code';
      i += 2;
    } else if (c === mode) {
      flush();
      if (mode === '`') tplBraces.pop();
      mode = 'code';
      i += 1;
    } else {
      buf += c;
      i += 1;
    }
  }
  flush();
  return out;
}

/**
 * 把一个 class 列表切成「去掉 variant 前缀」的 token。
 * `hover:(border-b border-arco-2)` → ['border-b', 'border-arco-2']
 * variant 前缀里可能带中括号（`[&_.x]:!border-t`），所以要在括号深度为 0 时才认冒号。
 */
function tokenize(list) {
  const tokens = [];
  for (const raw of list.split(/\s+/)) {
    if (!raw) continue;
    let t = raw.replace(/^[!<~]+/, '');
    let depth = 0;
    let cut = -1;
    for (let k = 0; k < t.length; k += 1) {
      const ch = t[k];
      if (ch === '[' || ch === '(') depth += 1;
      else if (ch === ']' || ch === ')') depth -= 1;
      else if (ch === ':' && depth === 0) cut = k;
    }
    if (cut >= 0) t = t.slice(cut + 1);
    t = t.replace(/^[!(]+/, '');
    const open = (t.match(/\(/g) || []).length;
    let close = (t.match(/\)/g) || []).length;
    while (close > open && t.endsWith(')')) {
      t = t.slice(0, -1);
      close -= 1;
    }
    if (t) tokens.push(t);
  }
  return tokens;
}

/** 逐行定位，便于报错时给出坐标 / Locate matches with 1-based line numbers */
function scanSource(source) {
  const hits = [];
  const lines = source.split('\n');
  for (const [form, { scan, re }] of Object.entries(FORMS)) {
    if (scan !== 'line') continue;
    lines.forEach((line, i) => {
      for (const m of line.matchAll(re)) hits.push({ form, line: i + 1, snippet: m[0] });
    });
  }
  for (const { line, text } of extractClassLists(source)) {
    const tokens = tokenize(text);
    for (const [form, { scan, token, list }] of Object.entries(FORMS)) {
      if (scan === 'token') {
        for (const t of tokens) if (token.test(t)) hits.push({ form, line, snippet: t });
      } else if (scan === 'list') {
        const snippet = list(tokens);
        if (snippet) hits.push({ form, line, snippet });
      }
    }
  }
  // 两种扫描方式产出的命中混在一起，按行号排一下再报，坐标才读得顺。
  // The two scan modes interleave; sort so the report reads top-to-bottom.
  return hits.sort((a, b) => a.line - b.line);
}

function selfTest() {
  const cases = [
    // 干净写法 / clean forms — 一条都不许命中
    { src: "<div className='text-danger-6 bg-primary-6 border-success-5' />", bad: 0 },
    { src: "<div className='border-arco-2 border-3 border-[var(--border-base)]' />", bad: 0 },
    { src: "<div className='border-b border-b-solid border-b-[var(--border-base)]' />", bad: 0 },
    // 显式 alpha 自带透明度，合法 / explicit alpha carries its own opacity
    { src: "<div className='bg-[rgba(var(--primary-6),0.12)]' />", bad: 0 },
    { src: "<div className='text-[rgba(var(--arcoblue-6),1)]' />", bad: 0 },
    { src: "<div className='shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.22)]' />", bad: 0 },
    // 非颜色属性里的 rgb() 不走颜色管道，合法 / rgb() in a shadow is fine
    { src: "<div className='shadow-[0_0_8px_rgb(var(--warning-6))]' />", bad: 0 },
    { src: "<div className='shadow-[0_4px_14px_rgba(var(--primary-6),0.32)]' />", bad: 0 },
    // 违规写法 / violations
    { src: "<div className='text-[rgb(var(--danger-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='bg-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='focus-visible:border-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='text-[rgb(var(--link-6))] bg-[rgb(var(--warning-5))]' />", bad: 2, form: 'ramp' },
    // 非语义色板同样是逗号三元组，同一个错 / Arco palettes break identically
    { src: "<div className='!text-[rgb(var(--arcoblue-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='bg-[rgb(var(--gray-2))]' />", bad: 1, form: 'ramp' },
    // text/bg/border 之外的颜色前缀也一样被注入 / other colour prefixes too
    { src: "<div className='ring-2 ring-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='focus-visible:outline-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='border-t-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    { src: "<div className='!border-b-[rgb(var(--primary-6))]' />", bad: 1, form: 'ramp' },
    // ✅ 这些前缀没有项目规则，自带 alpha 才是正解 / explicit alpha is the fix there
    { src: "<div className='ring-[rgba(var(--primary-6),1)] border-t-[rgba(var(--primary-6),1)]' />", bad: 0 },
    // 中括号 + 斜杠透明度，同样产出非法的 rgb(... / .12)
    { src: "<div className='bg-[rgb(var(--success-6))]/12' />", bad: 1, form: 'ramp' },
    { src: "<div className='border-b border-b-solid border-border-1' />", bad: 1, form: 'deadBorder' },
    { src: "<div className='border border-dashed border-border-2' />", bad: 1, form: 'deadBorder' },
    // 非数字后缀同样死：theme 里没有 `border` 这个颜色，后面跟什么都产出 0 条 CSS
    // Non-numeric suffixes are dead too — there is no `border` colour to index into.
    { src: "<div className='border-b border-b-solid border-border-base' />", bad: 1, form: 'deadBorder' },
    { src: "<div className='border border-solid border-border-light' />", bad: 1, form: 'deadBorder' },
    // ✅ 正解：颜色用 arbitrary value，并且补上 border-style
    { src: "<div className='border-b border-b-solid border-b-[var(--border-base)]' />", bad: 0 },
    { src: "<div className='border-r border-r-solid border-r-[var(--border-base)]' />", bad: 0 },
    { src: "<div className='border-b-solid border-b-base' />", bad: 1, form: 'dirNamedBorder' },
    { src: "<div className='border-t border-t-solid border-b-light' />", bad: 1, form: 'dirNamedBorder' },
    // 方向陷阱四个方向都要拦：只拦 -b- 时 border-t-base 漏过一次
    { src: "<div className='border-t border-t-solid border-t-base' />", bad: 1, form: 'dirNamedBorder' },
    // 项目规则类 + 斜杠透明度 = 匹配不上任何规则，产出 0 条 CSS
    { src: "<div className='bg-success-6/12' />", bad: 1, form: 'rampSlash' },
    { src: "<div className='hover:!text-danger-6/50' />", bad: 1, form: 'rampSlash' },
    // ── 5) 前缀写两遍 / doubled prefix ──────────────────────────────────────
    { src: "<div className='bg-bg-1' />", bad: 1, form: 'doublePrefix' },
    { src: "<div className='px-8px bg-bg-3 text-12px' />", bad: 1, form: 'doublePrefix' },
    { src: "<div className='hover:bg-bg-3 dark:bg-bg-2' />", bad: 2, form: 'doublePrefix' },
    { src: "<div className='bg-bg-base text-text-primary' />", bad: 2, form: 'doublePrefix' },
    { src: "<div className={`px-4px ${on ? 'bg-bg-2' : 'bg-1'}`} />", bad: 1, form: 'doublePrefix' },
    // variant group 里的也要抓到 / variant groups are expanded before matching
    { src: "<div className='hover:(bg-bg-3 text-t-primary)' />", bad: 1, form: 'doublePrefix' },
    // ✅ 去掉重复前缀 / the fix
    { src: "<div className='bg-1 hover:bg-3 dark:bg-2 text-t-primary' />", bad: 0 },
    // 前缀只写一次的正常类名不许误报 / single-prefix classes must stay clean
    { src: "<div className='bg-base bg-fill-2 bg-hover bg-brand-light text-t-secondary' />", bad: 0 },
    // 注释里的死写法是反例讲解，不是渲染出来的样式 / dead forms in comments are docs
    { src: '// 旧写法 bg-bg-2 是死的 / the old bg-bg-2 emitted nothing\nconst x = 1;', bad: 0 },
    { src: '/** `bg-bg-0` / `text-text` 都产出 0 条 CSS */\nconst y = 2;', bad: 0 },
    // ── 6) 带方向的数字 = 颜色，不是宽度 / directional numeric is a colour ──
    { src: "<div className='border-b-2 border-primary' />", bad: 1, form: 'dirNumBorder' },
    { src: "<div className='text-brand border-b-4 border-brand' />", bad: 1, form: 'dirNumBorder' },
    { src: "<div className='border-t-1 border-r-10' />", bad: 2, form: 'dirNumBorder' },
    // ✅ 宽度用 px 后缀，样式单独写 / px suffix is a real width
    { src: "<div className='border-b-2px border-b-solid border-primary' />", bad: 0 },
    { src: "<div className='border-b-4px border-b-solid border-brand' />", bad: 0 },
    // 0 值是真的宽度重置，不许误报 / `-0` really is a width reset
    { src: "<div className='border-b-0 border-l-0 border-r-0 border-t-0' />", bad: 0 },
    { src: "<div className='border-b border-b-solid border-arco-2 last:border-b-0' />", bad: 0 },
    // 不带方向的 border-N 是文档化的合法颜色写法 / non-directional border-N is documented
    { src: "<div className='border border-solid border-3' />", bad: 0 },
    // ── 7) 有宽度 + 有颜色 + 没样式 / width + colour, no style ─────────────
    { src: "<div className='border border-arco-2' />", bad: 1, form: 'borderNoStyle' },
    { src: "<div className='border-b border-arco-2' />", bad: 1, form: 'borderNoStyle' },
    { src: "<div className='rd-12px border border-[var(--color-border-2)] bg-fill-0' />", bad: 1, form: 'borderNoStyle' },
    { src: "<div className='border-t border-t-[var(--color-border-2)] pt-10px' />", bad: 1, form: 'borderNoStyle' },
    { src: "<div className='border border-[rgba(var(--primary-6),0.4)]' />", bad: 1, form: 'borderNoStyle' },
    { src: "<div className='border-2px border-primary-6' />", bad: 1, form: 'borderNoStyle' },
    // 一串里只报一次，不按颜色个数刷数量 / one hit per class list, not per colour
    { src: "<div className='border border-arco-2 hover:border-arco-3' />", bad: 1, form: 'borderNoStyle' },
    // ✅ 补上同方向的样式 / the fix
    { src: "<div className='border border-solid border-arco-2' />", bad: 0 },
    { src: "<div className='border-b border-b-solid border-arco-2' />", bad: 0 },
    { src: "<div className='border-b border-b-solid border-[var(--color-border-2)]' />", bad: 0 },
    { src: "<div className='border border-dashed border-primary-6' />", bad: 0 },
    { src: "<div className='border border-none border-arco-2' />", bad: 0 },
    // 只有宽度、或只有颜色，都不判定（样式可能来自组件自己的 CSS）
    // Width-only / colour-only lists are left alone: the style may come from CSS.
    { src: "<div className='border-t pt-8px' />", bad: 0 },
    { src: "<div className='hover:border-primary-6' />", bad: 0 },
    { src: "<div className='border-b-0 border-arco-2' />", bad: 0 },
    // 白名单之外的 token 不算颜色，宁可漏报 / unknown tokens are not colours
    { src: "<div className='border border-line bg-fill-1' />", bad: 0 },
    // variant group 里的宽度 + 颜色同样判定 / variant groups too
    { src: "<div className='hover:(border-b border-arco-2)' />", bad: 1, form: 'borderNoStyle' },
    // 混合 / mixed
    { src: "<div className='border-border-3 text-[rgb(var(--success-6))] border-b-base' />", bad: 3 },
    { src: "<div className='bg-bg-2 border-b-2 border-primary' />", bad: 2 },
  ];
  let failed = 0;
  cases.forEach(({ src, bad, form }, i) => {
    const hits = scanSource(src);
    if (hits.length !== bad) {
      failed += 1;
      console.error(`self-test case ${i} failed: expected ${bad} violation(s), got ${hits.length}\n  ${src}`);
      return;
    }
    if (form && hits.some((h) => h.form !== form)) {
      failed += 1;
      console.error(`self-test case ${i} failed: expected form "${form}", got ${[...new Set(hits.map((h) => h.form))]}`);
    }
  });

  const total = cases.length;
  if (failed > 0) {
    console.error(`❌ check-dead-css-utilities self-test: ${failed}/${total} case(s) failed`);
    process.exit(1);
  }
  console.log(`✅ check-dead-css-utilities self-test: ${total}/${total} cases pass`);
}

if (process.argv.includes('--self-test')) {
  selfTest();
  process.exit(0);
}

const errors = [];
let scanned = 0;
for (const abs of walk(SCAN_DIR)) {
  const file = relative(ROOT, abs).split('\\').join('/');
  scanned += 1;
  const hits = scanSource(readFileSync(abs, 'utf8'));
  if (hits.length) {
    errors.push([`${file} 使用了死 CSS 工具类:`, ...hits.map((h) => `    ${file}:${h.line}  ${h.snippet}`)].join('\n'));
  }
}

if (errors.length) {
  console.error(`\n❌ 死 CSS 工具类检查未通过（详见 ${SELF} 头注 / ui/src/renderer/styles/MIGRATION.md）:\n`);
  for (const e of errors) console.error(`  ${e}`);
  console.error('\n  有效替代 / Valid replacements:');
  for (const [form, { label, fix }] of Object.entries(FORMS)) {
    console.error(`    [${form}] ${label}\n        → ${fix}`);
  }
  process.exit(1);
}

console.log(`✅ dead CSS utilities clean (${scanned} file(s) scanned, ${Object.keys(FORMS).length} banned form(s), no baseline)`);
