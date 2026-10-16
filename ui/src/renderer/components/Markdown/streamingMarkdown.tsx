/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { type ReactElement, type ReactNode } from 'react';
import ReactMarkdown, { type Components, type ExtraProps } from 'react-markdown';

type MarkdownOptions = Parameters<typeof ReactMarkdown>[0];
type Position = { start: { offset?: number }; end: { offset?: number } };
type SyntaxNode = {
  type: string;
  tagName?: string;
  value?: string;
  properties?: Record<string, unknown>;
  position?: Position;
  children?: SyntaxNode[];
};
type Range = { start: number; end: number };
type RevealPlan = (offset: number) => ReactNode;

/** Transparent renderers ask react-markdown to retain the HAST node on each
 * React element. They add no DOM wrapper and use the same URL/security pass. */
const SourceElement = ({ node, ...props }: Record<string, unknown> & ExtraProps) =>
  React.createElement(node?.tagName ?? 'span', props);

const sourceRange = (node: SyntaxNode | undefined): Range | undefined => {
  const start = node?.position?.start.offset;
  const end = node?.position?.end.offset;
  return typeof start === 'number' && typeof end === 'number' ? { start, end } : undefined;
};

const mathScope = (node: SyntaxNode): boolean => {
  if (node.tagName === 'pre') return node.children?.some(mathScope) === true;
  if (node.tagName !== 'code') return false;
  const classes = node.properties?.className;
  return Array.isArray(classes) && classes.some(value =>
    value === 'language-math' || value === 'math-inline' || value === 'math-display');
};

/** KaTeX replaces source nodes with HTML whose positions describe generated
 * HTML. Temporarily wrap its source, then unwrap and restore the real range. */
const mathPositions = () => {
  const wrappers = new WeakMap<SyntaxNode, { position?: Position; block: boolean }>();
  const atomic = new WeakSet<SyntaxNode>();
  const blocks = new WeakSet<SyntaxNode>();
  const before = () => (tree: SyntaxNode) => {
    const visit = (parent: SyntaxNode) => {
      parent.children = parent.children?.map(child => {
        if (mathScope(child)) {
          const wrapper: SyntaxNode = { type: 'element', tagName: 'span', properties: {}, children: [child] };
          wrappers.set(wrapper, { position: child.position, block: child.tagName === 'pre'
            || (Array.isArray(child.properties?.className) && child.properties.className.includes('math-display')) });
          return wrapper;
        }
        visit(child);
        return child;
      });
    };
    visit(tree);
  };
  const after = (tree: SyntaxNode) => {
    const visit = (parent: SyntaxNode) => {
      parent.children = parent.children?.flatMap(child => {
        if (wrappers.has(child)) {
          return (child.children ?? []).map(rendered => {
            const source = wrappers.get(child)!;
            rendered.position = source.position;
            atomic.add(rendered);
            if (source.block) blocks.add(rendered);
            return rendered;
          });
        }
        visit(child);
        return [child];
      });
    };
    visit(tree);
  };
  return { before, after, atomic, blocks };
};

/** Normalize only the cursor, never the canonical source. LaTeX delimiter
 * conversion changes \( / \) from two code units to one dollar sign. */
const normalizedCursor = (source: string, normalized: string) => {
  if (source === normalized) return (offset: number) => offset;
  const boundaries = [0];
  let original = 0;
  let converted = 0;
  while (original < source.length && converted < normalized.length) {
    if (source[original] === normalized[converted]) {
      original += 1;
      converted += 1;
      boundaries.push(original);
    } else if (/^\\[()]/.test(source.slice(original, original + 2)) && normalized[converted] === '$') {
      original += 2;
      converted += 1;
      boundaries.push(original);
    } else if (/^\\[\[\]]/.test(source.slice(original, original + 2)) && normalized.slice(converted, converted + 2) === '$$') {
      boundaries.push(original + 1, original + 2);
      original += 2;
      converted += 2;
    } else {
      // Unknown future transformations remain atomic instead of exposing text
      // whose source position cannot be established.
      return (offset: number) => offset >= source.length ? normalized.length : 0;
    }
  }
  return (offset: number) => {
    let low = 0;
    let high = boundaries.length - 1;
    while (low < high) {
      const middle = Math.ceil((low + high) / 2);
      if (boundaries[middle] <= offset) low = middle;
      else high = middle - 1;
    }
    return low;
  };
};

/** Decode source escapes/entities once, retaining the point at which each
 * displayed character has actually been revealed. Ambiguous nodes stay atomic. */
const textBoundaries = (raw: string, value: string): number[] | undefined => {
  const result = [0];
  let source = 0;
  let displayed = 0;
  while (displayed < value.length) {
    if (raw[source] === '\\' && /[!"#$%&'()*+,\-./:;<=>?@[\]\\^_`{|}~]/.test(raw[source + 1] ?? '')
      && raw[source + 1] === value[displayed]) {
      source += 2;
      displayed += 1;
      result.push(source);
      continue;
    }
    if (raw.slice(source, source + 2) === '\r\n' && value[displayed] === '\n') {
      source += 2;
      displayed += 1;
      result.push(source);
      continue;
    }
    const entity = /^&(?:#[xX][\da-fA-F]{1,8}|#\d{1,10}|[A-Za-z][A-Za-z\d]{1,31});/.exec(raw.slice(source))?.[0];
    if (entity) {
      const decoder = document.createElement('textarea');
      decoder.innerHTML = entity;
      const decoded = decoder.value;
      if (decoded !== entity && value.startsWith(decoded, displayed)) {
        source += entity.length;
        displayed += decoded.length;
        for (let index = 0; index < decoded.length; index += 1) result.push(source);
        continue;
      }
    }
    if (raw[source] === value[displayed]) {
      result.push(++source);
      displayed += 1;
      continue;
    }
    return undefined;
  }
  return result;
};

const safeOffset = (text: string, offset: number): number => {
  const cursor = Math.max(0, Math.min(text.length, Math.floor(Number.isFinite(offset) ? offset : 0)));
  const previous = text.charCodeAt(cursor - 1);
  const next = text.charCodeAt(cursor);
  return previous >= 0xd800 && previous <= 0xdbff && next >= 0xdc00 && next <= 0xdfff ? cursor - 1 : cursor;
};

/** One react-markdown parse per actual source revision. Reveal frames only
 * visit the active branch; completed elements retain their original identity.
 * Full-source parsing preserves reference definitions, lists, tables and fences. */
export function createStreamingMarkdown(source: string, normalized: string, options: Omit<MarkdownOptions, 'children'>) {
  const components = { ...options.components } as Record<string, unknown>;
  const math = mathPositions();
  const retainPositions = () => (tree: SyntaxNode) => {
    math.after(tree);
    const visit = (node: SyntaxNode) => {
      if (node.type === 'element' && node.tagName && !components[node.tagName]) components[node.tagName] = SourceElement;
      node.children?.forEach(visit);
    };
    visit(tree);
  };
  // react-markdown's synchronous entry point is a pure parser/render-tree
  // builder. The React components in that tree still render through React.
  const parsed = ReactMarkdown({ ...options, children: normalized, components: components as Components,
    rehypePlugins: [math.before, ...(options.rehypePlugins ?? []), retainPositions] });
  const compile = (value: ReactNode, syntax: SyntaxNode | undefined, parent: Range): RevealPlan => {
    if (typeof value === 'string') {
      const range = sourceRange(syntax);
      if (!range) return !value.trim() ? () => value : offset => offset >= parent.end ? value : null;
      const boundaries = textBoundaries(normalized.slice(range.start, range.end), value);
      return offset => {
        if (offset >= range.end) return value;
        if (offset <= range.start || !boundaries) return null;
        let low = 0;
        let high = boundaries.length - 1;
        while (low < high) {
          const middle = Math.ceil((low + high) / 2);
          if (range.start + boundaries[middle] <= offset) low = middle;
          else high = middle - 1;
        }
        return value.slice(0, safeOffset(value, low));
      };
    }
    if (Array.isArray(value)) {
      const plans = value.map(child => compile(child, undefined, parent));
      return offset => plans.map(plan => plan(offset));
    }
    if (!React.isValidElement(value)) return () => value;
    const element = value as ReactElement<{ children?: ReactNode; node?: SyntaxNode }>;
    const node = element.props.node ?? syntax;
    const range = sourceRange(node) ?? parent;
    const children = element.props.children;
    const renderableNodes = node?.children?.filter(child => child.type === 'element' || child.type === 'text');
    const childValues = Array.isArray(children) ? children : children === undefined ? [] : [children];
    const plans = childValues.map((child, index) => compile(child, renderableNodes?.[index], range));
    const atomic = Boolean(node && math.atomic.has(node))
      || ['img', 'pre', 'code', 'hr', 'br', 'input'].includes(node?.tagName ?? '');
    return offset => {
      // Rich blocks keep their existing received-source widget behavior. Once
      // preceding prose reaches the opener, render the entire received block;
      // appending to an EOF-terminated fence/math block must not hide it again.
      if (node?.tagName === 'pre' || (node && math.blocks.has(node))) return offset >= range.start ? element : null;
      if (offset >= range.end) return element;
      if ((node ? offset <= range.start : offset < range.start) || atomic) return null;
      const visibleChildren = plans.map(plan => plan(offset));
      // The label (including a long autolink URL) can stream as ordinary text.
      // Its destination and click handler become available only when the whole
      // link syntax has been revealed. No future href leaks into the DOM.
      if (node?.tagName === 'a') return React.createElement('span', { key: element.key }, visibleChildren);
      return React.cloneElement(element, { children: visibleChildren });
    };
  };
  const reveal = compile(parsed, undefined, { start: 0, end: normalized.length });
  const cursor = normalizedCursor(source, normalized);
  return {
    render(offset: number): ReactNode {
      if (offset >= source.length) return parsed;
      return reveal(cursor(safeOffset(source, offset)));
    },
  };
}
