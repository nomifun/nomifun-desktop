/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Render smoke tests for the shell primitives.
 *
 * The seven tabs each have their own tests, but the pieces that hold them
 * together — the portal-based aside host and the workspace header — had none.
 * Server-rendering them catches the class of mistake that `tsc` cannot: a
 * component that throws on first render, a hook used conditionally, or an aside
 * that silently renders nowhere.
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import ContentAside from '@/renderer/components/layout/ContentAside';
import SegmentedTabs from '@/renderer/components/base/SegmentedTabs';
import { AsideHost, useAsidePortal } from './AsideHost';

describe('ContentAside', () => {
  test('renders its title, subtitle and body', () => {
    const html = renderToStaticMarkup(
      React.createElement(ContentAside, {
        title: '记忆详情',
        subtitle: '2026-08-04',
        onClose: () => {},
        storageKey: 'test:aside',
        children: React.createElement('p', null, '这是正文'),
      })
    );
    expect(html.includes('记忆详情')).toBe(true);
    expect(html.includes('2026-08-04')).toBe(true);
    expect(html.includes('这是正文')).toBe(true);
  });

  test('offers a close affordance that is reachable without a mouse', () => {
    const html = renderToStaticMarkup(
      React.createElement(ContentAside, {
        title: 't',
        onClose: () => {},
        storageKey: 'test:aside',
        children: '内容',
      })
    );
    expect(html.includes('role="button"')).toBe(true);
    expect(html.includes('tabindex="0"')).toBe(true);
    expect(html.includes('aria-label="close"')).toBe(true);
  });

  test('honours the caller-supplied minimum width', () => {
    const html = renderToStaticMarkup(
      React.createElement(ContentAside, {
        title: 't',
        onClose: () => {},
        storageKey: 'test:aside-min',
        defaultWidth: 400,
        minWidth: 320,
        children: '内容',
      })
    );
    expect(html.includes('min-width:320px')).toBe(true);
  });
});

describe('AsideHost', () => {
  /** A tab that portals an aside, mirroring how the real tabs use the host. */
  const TabWithAside: React.FC = () => {
    const aside = useAsidePortal(React.createElement('aside', { id: 'portalled' }, 'detail'));
    return React.createElement(React.Fragment, null, React.createElement('div', null, 'body'), aside);
  };

  test('the landing slot uses display:contents so a portalled pane becomes a flex sibling', () => {
    // If this wrapper were a normal block, the aside would be boxed inside it and
    // would not participate in the three-column row.
    const html = renderToStaticMarkup(
      React.createElement(AsideHost, { children: React.createElement('div', null, 'workspace') })
    );
    expect(html.includes('display:contents')).toBe(true);
  });

  test('a tab renders its body even before the host element exists', () => {
    // On the server (and on the very first client render) the host ref is null, so
    // useAsidePortal returns null. The tab must still render — returning the portal
    // unconditionally must not blow up.
    const html = renderToStaticMarkup(
      React.createElement(AsideHost, { children: React.createElement(TabWithAside, null) })
    );
    expect(html.includes('body')).toBe(true);
    // The portal cannot resolve server-side, so the aside is absent rather than
    // misplaced — the important part is that nothing threw.
    expect(html.includes('id="portalled"')).toBe(false);
  });
});

describe('SegmentedTabs attention dot', () => {
  const items = [
    { key: 'a', label: '总览' },
    { key: 'b', label: '技能', dot: true },
  ];

  test('marks only the segment that asked for it', () => {
    const html = renderToStaticMarkup(
      React.createElement(SegmentedTabs, { items, activeKey: 'a', onChange: () => {}, size: 'sm' })
    );
    // One dot, on the second segment.
    expect(html.split('rd-full bg-primary-6').length - 1).toBe(1);
    expect(html.indexOf('技能')).toBeLessThan(html.indexOf('rd-full bg-primary-6'));
  });

  test('the dot is decorative, not announced', () => {
    const html = renderToStaticMarkup(
      React.createElement(SegmentedTabs, { items, activeKey: 'a', onChange: () => {}, size: 'sm' })
    );
    expect(html.includes('aria-hidden="true"')).toBe(true);
  });

  test('tabs carry the roles a tablist needs', () => {
    const html = renderToStaticMarkup(
      React.createElement(SegmentedTabs, { items, activeKey: 'a', onChange: () => {} })
    );
    expect(html.includes('role="tablist"')).toBe(true);
    expect(html.includes('role="tab"')).toBe(true);
    expect(html.includes('aria-selected="true"')).toBe(true);
  });
});
