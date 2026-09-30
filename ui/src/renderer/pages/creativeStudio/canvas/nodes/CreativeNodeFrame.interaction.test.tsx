/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';

import type { CreativeCanvasNode } from '../../domain/schema';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import { CreativeTextNode } from './CreativeNodeViews';

const textNode = (
  locked = false
): Extract<CreativeCanvasNode, { type: 'text' }> => ({
  id: 'text-node',
  type: 'text',
  position: { x: 0, y: 0 },
  size: { width: 320, height: 180 },
  groupId: null,
  zIndex: 1,
  locked,
  data: {
    text: '第一行',
    format: 'plain',
    fontSize: 18,
    textAlign: 'left',
  },
});

afterEach(cleanup);

describe('CreativeNodeFrame title editing', () => {
  test('renames from the shared title on double click without opening the node', () => {
    const names: string[] = [];
    let opens = 0;
    const view = render(withCanvasTestI18n(
      <CreativeTextNode
        node={textNode()}
        title='文本1'
        placement='contained'
        onOpen={() => { opens += 1; }}
        onRename={(_, name) => names.push(name)}
      />
    ));
    const title = view.container.querySelector<HTMLElement>('[data-node-title]');
    if (!title) throw new Error('node title fixture missing');

    fireEvent.doubleClick(title);
    const input = view.container.querySelector<HTMLInputElement>('[data-node-title-input]');
    if (!input) throw new Error('node title input missing');
    expect(document.activeElement).toBe(input);
    expect(opens).toBe(0);

    fireEvent.change(input, { target: { value: '  旁白文案  ' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    expect(names).toEqual(['旁白文案']);
    expect(view.container.querySelector('[data-node-title-input]')).toBeNull();
  });

  test('cancels blank or escaped edits and keeps locked titles read-only', () => {
    const names: string[] = [];
    const view = render(withCanvasTestI18n(
      <CreativeTextNode
        node={textNode()}
        title='文本1'
        placement='contained'
        onRename={(_, name) => names.push(name)}
      />
    ));
    const title = view.container.querySelector<HTMLElement>('[data-node-title]');
    if (!title) throw new Error('node title fixture missing');

    fireEvent.doubleClick(title);
    let input = view.container.querySelector<HTMLInputElement>('[data-node-title-input]');
    if (!input) throw new Error('node title input missing');
    fireEvent.change(input, { target: { value: '   ' } });
    fireEvent.blur(input);
    expect(names).toEqual([]);

    fireEvent.doubleClick(title);
    input = view.container.querySelector<HTMLInputElement>('[data-node-title-input]');
    if (!input) throw new Error('node title input missing');
    fireEvent.change(input, { target: { value: '不保存' } });
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(names).toEqual([]);

    view.rerender(withCanvasTestI18n(
      <CreativeTextNode
        node={textNode(true)}
        title='文本1'
        placement='contained'
        onRename={(_, name) => names.push(name)}
      />
    ));
    const lockedTitle = view.container.querySelector<HTMLElement>('[data-node-title]');
    if (!lockedTitle) throw new Error('locked node title fixture missing');
    fireEvent.doubleClick(lockedTitle);
    expect(view.container.querySelector('[data-node-title-input]')).toBeNull();
  });
});
