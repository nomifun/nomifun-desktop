/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 自定义 CSS 处理工具
 * 统一处理自定义 CSS 的 !important 添加和格式化
 */

/**
 * 自动为所有 CSS 属性添加 !important
 * @param css - 原始 CSS 字符串
 * @returns 处理后的 CSS 字符串（所有属性都带 !important）
 */
const CSS_PROPERTY_NAME = /^(?:--)?-?[a-zA-Z_][a-zA-Z0-9_-]*$/;
const CSS_COMMENTS = /\/\*[\s\S]*?\*\//g;
const TRAILING_CSS_TRIVIA = /(?:\s|\/\*[\s\S]*?\*\/)*$/;

/**
 * Add `!important` at declaration boundaries instead of splitting on every
 * semicolon. CSS values may legally contain semicolons inside strings and
 * functions (most visibly data URLs), so a regular expression cannot safely
 * identify the end of a declaration.
 */
const addImportantToDeclarations = (css: string): string => {
  let result = '';
  let emittedUntil = 0;
  let declarationValueStart: number | null = null;
  let declarationBoundary = 0;
  let quote: '"' | "'" | null = null;
  let escaped = false;
  let inComment = false;
  let parenthesisDepth = 0;
  let bracketDepth = 0;

  const addImportantBefore = (declarationEnd: number) => {
    if (declarationValueStart === null) return;

    const rawValue = css.slice(declarationValueStart, declarationEnd);
    const trailingTrivia = rawValue.match(TRAILING_CSS_TRIVIA)?.[0] ?? '';
    const valueWithoutTrailingTrivia = rawValue.slice(0, rawValue.length - trailingTrivia.length);
    if (!valueWithoutTrailingTrivia.trim() || /!\s*important\s*$/i.test(valueWithoutTrailingTrivia)) {
      return;
    }

    const insertionPoint = declarationEnd - trailingTrivia.length;
    result += `${css.slice(emittedUntil, insertionPoint)} !important`;
    emittedUntil = insertionPoint;
  };

  for (let index = 0; index < css.length; index += 1) {
    const char = css[index];
    const next = css[index + 1];

    if (inComment) {
      if (char === '*' && next === '/') {
        inComment = false;
        index += 1;
      }
      continue;
    }

    if (quote) {
      if (escaped) {
        escaped = false;
      } else if (char === '\\') {
        escaped = true;
      } else if (char === quote) {
        quote = null;
      }
      continue;
    }

    if (char === '/' && next === '*') {
      inComment = true;
      index += 1;
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (char === '(') {
      parenthesisDepth += 1;
      continue;
    }
    if (char === ')') {
      parenthesisDepth = Math.max(0, parenthesisDepth - 1);
      continue;
    }
    if (char === '[') {
      bracketDepth += 1;
      continue;
    }
    if (char === ']') {
      bracketDepth = Math.max(0, bracketDepth - 1);
      continue;
    }
    if (parenthesisDepth > 0 || bracketDepth > 0) continue;

    if (char === ':' && declarationValueStart === null) {
      const property = css.slice(declarationBoundary, index).replace(CSS_COMMENTS, '').trim();
      if (CSS_PROPERTY_NAME.test(property)) declarationValueStart = index + 1;
      continue;
    }

    if (char === ';') {
      addImportantBefore(index);
      declarationValueStart = null;
      declarationBoundary = index + 1;
      continue;
    }

    if (char === '{') {
      // A colon followed by an opening brace belonged to a selector or at-rule,
      // not a declaration. The new block starts a fresh declaration boundary.
      declarationValueStart = null;
      declarationBoundary = index + 1;
      continue;
    }

    if (char === '}') {
      // The final declaration in a block may omit its semicolon.
      addImportantBefore(index);
      declarationValueStart = null;
      declarationBoundary = index + 1;
    }
  }

  addImportantBefore(css.length);
  result += css.slice(emittedUntil);
  return result;
};

export const addImportantToAll = (css: string): string => {
  if (!css || !css.trim()) {
    return '';
  }

  // 注释区间：注释里出现的 "@keyframes" 或大括号不能影响解析
  // Comment ranges: "@keyframes" or braces inside comments must not affect parsing.
  const commentRanges: Array<[number, number]> = [];
  const commentRe = /\/\*[\s\S]*?\*\//g;
  let commentMatch: RegExpExecArray | null;
  while ((commentMatch = commentRe.exec(css)) !== null) {
    commentRanges.push([commentMatch.index, commentMatch.index + commentMatch[0].length]);
  }
  const inComment = (index: number) => commentRanges.some(([start, end]) => index >= start && index < end);

  // CSS 规定 @keyframes 内带 !important 的声明会被整条忽略，
  // 所以关键帧块必须原样保留，只处理块外的声明。
  // Declarations with !important inside @keyframes are ignored per spec,
  // so keyframes blocks must pass through untouched.
  const keyframesRe = /@(?:-webkit-)?keyframes\b/g;
  let result = '';
  let cursor = 0;
  let match: RegExpExecArray | null;

  while ((match = keyframesRe.exec(css)) !== null) {
    if (inComment(match.index)) continue;
    const blockStart = css.indexOf('{', match.index);
    if (blockStart === -1) break;
    // 括号配平找到关键帧块结尾（跳过注释内的括号）
    // Balance braces to find the end of the block, ignoring braces inside comments
    let depth = 1;
    let blockEnd = blockStart + 1;
    while (blockEnd < css.length && depth > 0) {
      const ch = css[blockEnd];
      if ((ch === '{' || ch === '}') && !inComment(blockEnd)) {
        depth += ch === '{' ? 1 : -1;
      }
      blockEnd++;
    }
    result += addImportantToDeclarations(css.slice(cursor, match.index));
    result += css.slice(match.index, blockEnd);
    cursor = blockEnd;
    keyframesRe.lastIndex = blockEnd;
  }
  result += addImportantToDeclarations(css.slice(cursor));

  return result;
};

/**
 * 包装自定义 CSS，添加注释说明
 * @param css - 处理后的 CSS 字符串
 * @returns 带注释的 CSS 字符串
 */
export const wrapCustomCss = (css: string): string => {
  if (!css || !css.trim()) {
    return '';
  }

  return `
/* 用户自定义样式 - 自动添加 !important 提升优先级 */
/* User Custom Styles - Auto !important for highest priority */
${css}
  `.trim();
};

/**
 * 完整处理自定义 CSS
 * @param css - 原始 CSS 字符串
 * @returns 处理后并包装的 CSS 字符串
 */
export const processCustomCss = (css: string): string => {
  const processed = addImportantToAll(css);
  return wrapCustomCss(processed);
};
