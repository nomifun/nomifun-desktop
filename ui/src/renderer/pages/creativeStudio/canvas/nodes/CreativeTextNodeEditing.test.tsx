/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import React, { useState } from 'react';

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

afterEach(() => cleanup());

describe('CreativeTextNode inline editing', () => {
  test('edits canonical text and isolates input gestures from the canvas', () => {
    let parentPointerDown = 0;
    let parentWheel = 0;

    const Harness: React.FC = () => {
      const [node, setNode] = useState(textNode());
      const [editing, setEditing] = useState(true);
      return (
        <div
          onPointerDown={() => {
            parentPointerDown += 1;
          }}
          onWheel={() => {
            parentWheel += 1;
          }}
        >
          <CreativeTextNode
            node={node}
            placement='contained'
            editing={editing}
            onTextChange={(text) =>
              setNode((current) => ({
                ...current,
                data: { ...current.data, text },
              }))
            }
            onFinishEditing={() => setEditing(false)}
          />
        </div>
      );
    };

    const { container, getByRole, queryByRole } = render(
      withCanvasTestI18n(<Harness />)
    );
    const editor = getByRole('textbox') as HTMLTextAreaElement;

    expect(editor.hasAttribute('data-node-text-editor')).toBe(true);
    expect(document.activeElement).toBe(editor);
    expect(editor.maxLength).toBe(1_000_000);
    fireEvent.pointerDown(editor);
    fireEvent.wheel(editor, { deltaY: 120 });
    expect(parentPointerDown).toBe(0);
    expect(parentWheel).toBe(0);

    fireEvent.change(editor, { target: { value: '第一行\n第二行' } });
    expect(editor.value).toBe('第一行\n第二行');
    fireEvent.keyDown(editor, { key: 'Enter' });
    expect(queryByRole('textbox')).not.toBeNull();

    fireEvent.keyDown(editor, { key: 'Enter', ctrlKey: true });
    expect(queryByRole('textbox')).toBeNull();
    expect(
      container.querySelector('[data-node-text-format]')?.textContent
    ).toBe('第一行\n第二行');
  });

  test('keeps IME composition active and refuses to edit a locked node', () => {
    let finished = 0;
    const { getByRole, queryByRole, rerender } = render(
      withCanvasTestI18n(
        <CreativeTextNode
          node={textNode()}
          placement='contained'
          editing
          onTextChange={() => undefined}
          onFinishEditing={() => {
            finished += 1;
          }}
        />
      )
    );
    const editor = getByRole('textbox');

    fireEvent.keyDown(editor, {
      key: 'Escape',
      isComposing: true,
      keyCode: 229,
    });
    expect(finished).toBe(0);

    rerender(
      withCanvasTestI18n(
        <CreativeTextNode
          node={textNode(true)}
          placement='contained'
          editing
          onTextChange={() => undefined}
          onFinishEditing={() => undefined}
        />
      )
    );
    expect(queryByRole('textbox')).toBeNull();
  });

  test('keeps the editor inset and the displayed text close to the top', () => {
    const css = readFileSync(
      new URL('./CreativeNodeViews.module.css', import.meta.url),
      'utf8'
    );

    expect(css.includes('padding: 32px 56px 16px 16px')).toBe(false);
    expect(css.includes('padding: 12px 16px')).toBe(true);
    expect(css.includes('inset: 6px')).toBe(true);
    expect(css.includes('height: calc(100% - 12px)')).toBe(true);
    expect(css.includes('padding: 8px 10px')).toBe(true);
  });
});
