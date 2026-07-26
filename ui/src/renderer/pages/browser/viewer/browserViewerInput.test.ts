/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  browserViewerKeyUsesTextInput,
  BrowserViewerPointerController,
  BrowserViewerTextInputController,
  isBrowserViewerReadOnlyIdentity,
  MAX_BROWSER_VIEWER_TEXT_BYTES,
  sendBrowserViewerCommand,
  splitBrowserViewerText,
  type BrowserViewerPointerCaptureTarget,
  type BrowserViewerPointerEventLike,
} from './browserViewerInput';

const textFromMessage = (message: Record<string, unknown>): string | undefined => {
  const input = message.input;
  if (!input || typeof input !== 'object') return undefined;
  const value = (input as Record<string, unknown>).text;
  return typeof value === 'string' ? value : undefined;
};

describe('browser viewer text input', () => {
  test('sends committed CJK/emoji composition exactly once and never via keydown', () => {
    const messages: Array<Record<string, unknown>> = [];
    const deferred: Array<() => void> = [];
    const input = new BrowserViewerTextInputController(
      (message) => {
        messages.push(message);
        return true;
      },
      { deferClear: (callback) => deferred.push(callback) }
    );
    const committed = '中文输入🙂';

    expect(
      browserViewerKeyUsesTextInput({
        key: 'Process',
        code: 'KeyA',
        keyCode: 229,
        isComposing: true,
      })
    ).toBe(true);
    input.compositionStart();
    expect(
      input.beforeInput({
        inputType: 'insertCompositionText',
        data: 'zhong',
        isComposing: true,
      })
    ).toEqual({ preventDefault: false, sent: false });
    expect(input.compositionEnd(committed)).toBe(true);
    expect(
      input.beforeInput({
        inputType: 'insertFromComposition',
        data: committed,
        isComposing: false,
      })
    ).toEqual({ preventDefault: true, sent: false });
    deferred.forEach((callback) => callback());

    expect(messages).toEqual([
      {
        type: 'input',
        input: { kind: 'text', text: committed },
      },
    ]);
  });

  test('sends beforeinput text and paste once, including browsers that fire them in either order', () => {
    const messages: Array<Record<string, unknown>> = [];
    const input = new BrowserViewerTextInputController(
      (message) => {
        messages.push(message);
        return true;
      },
      { deferClear: () => undefined }
    );

    expect(
      input.beforeInput({
        inputType: 'insertText',
        data: '漢',
      })
    ).toEqual({ preventDefault: true, sent: true });

    const paste = '粘贴🙂\n第二行';
    expect(input.paste(paste)).toBe(true);
    expect(
      input.beforeInput({
        inputType: 'insertFromPaste',
        data: paste,
      })
    ).toEqual({ preventDefault: true, sent: false });

    const beforePaste = 'beforeinput-first';
    expect(
      input.beforeInput({
        inputType: 'insertFromPaste',
        data: beforePaste,
      })
    ).toEqual({ preventDefault: true, sent: true });
    expect(input.paste(beforePaste)).toBe(false);

    expect(messages.map(textFromMessage)).toEqual(['漢', paste, beforePaste]);
  });

  test('chunks long CJK and emoji on UTF-8 boundaries with every command at most 256 bytes', () => {
    const text = `${'漢'.repeat(171)}${'🙂'.repeat(131)}tail`;
    const chunks = splitBrowserViewerText(text);
    const encoder = new TextEncoder();
    const decoder = new TextDecoder('utf-8', { fatal: true });

    expect(chunks.length).toBeGreaterThan(2);
    expect(chunks.join('')).toBe(text);
    for (const chunk of chunks) {
      const bytes = encoder.encode(chunk);
      expect(bytes.byteLength).toBeLessThanOrEqual(MAX_BROWSER_VIEWER_TEXT_BYTES);
      expect(decoder.decode(bytes)).toBe(chunk);
      expect(chunk.includes('\uFFFD')).toBe(false);
    }
  });

  test('keeps printable and paste keydowns on the text-event path but sends navigation keys separately', () => {
    expect(browserViewerKeyUsesTextInput({ key: 'a', code: 'KeyA' })).toBe(true);
    expect(browserViewerKeyUsesTextInput({ key: '🙂', code: 'Unidentified' })).toBe(true);
    expect(
      browserViewerKeyUsesTextInput({
        key: 'v',
        code: 'KeyV',
        ctrlKey: true,
      })
    ).toBe(true);
    expect(
      browserViewerKeyUsesTextInput({
        key: 'ArrowLeft',
        code: 'ArrowLeft',
      })
    ).toBe(false);
    expect(
      browserViewerKeyUsesTextInput({
        key: 'c',
        code: 'KeyC',
        ctrlKey: true,
      })
    ).toBe(false);
  });
});

describe('authenticated replica viewer input policy', () => {
  test('keeps observe available while navigation, tab, takeover, key, text, and pointer commands never send', () => {
    const messages: Array<Record<string, unknown>> = [];
    const rawSend = (message: Record<string, unknown>): boolean => {
      messages.push(message);
      return true;
    };
    const readOnly = isBrowserViewerReadOnlyIdentity('authenticated_replica');
    const send = (message: Record<string, unknown>): boolean =>
      sendBrowserViewerCommand(rawSend, readOnly, message);

    expect(readOnly).toBe(true);
    expect(send({ type: 'observe', lane_id: 'replica' })).toBe(true);
    for (const command of [
      { type: 'navigate', url: 'https://example.test' },
      { type: 'back' },
      { type: 'forward' },
      { type: 'reload' },
      { type: 'select_tab', tab_id: 'tab-2' },
      { type: 'takeover' },
      { type: 'input', input: { kind: 'key', action: 'down', key: 'a', code: 'KeyA' } },
      { type: 'input', input: { kind: 'text', text: 'blocked' } },
      {
        type: 'input',
        input: { kind: 'pointer', action: 'down', x: 1, y: 1, button: 0, buttons: 1 },
      },
    ]) {
      expect(send(command)).toBe(false);
    }

    expect(messages).toEqual([{ type: 'observe', lane_id: 'replica' }]);
  });
});

describe('anonymous viewer input policy', () => {
  test('is read-only just like an authenticated replica', () => {
    const messages: Array<Record<string, unknown>> = [];
    const rawSend = (message: Record<string, unknown>): boolean => {
      messages.push(message);
      return true;
    };
    const readOnly = isBrowserViewerReadOnlyIdentity('anonymous');

    expect(readOnly).toBe(true);
    expect(
      sendBrowserViewerCommand(rawSend, readOnly, { type: 'observe' })
    ).toBe(true);
    expect(
      sendBrowserViewerCommand(rawSend, readOnly, { type: 'takeover' })
    ).toBe(false);
    expect(messages).toEqual([{ type: 'observe' }]);
  });
});

describe('browser viewer identity interaction allowlist', () => {
  test('only primary and isolated identities are writable', () => {
    expect(isBrowserViewerReadOnlyIdentity('primary')).toBe(false);
    expect(isBrowserViewerReadOnlyIdentity('isolated')).toBe(false);
    expect(isBrowserViewerReadOnlyIdentity('anonymous')).toBe(true);
    expect(isBrowserViewerReadOnlyIdentity('authenticated_replica')).toBe(true);
    expect(isBrowserViewerReadOnlyIdentity(undefined)).toBe(true);
    expect(isBrowserViewerReadOnlyIdentity(null)).toBe(true);
  });
});

class FakePointerTarget implements BrowserViewerPointerCaptureTarget {
  readonly captured = new Set<number>();
  readonly setCalls: number[] = [];
  readonly releaseCalls: number[] = [];

  getBoundingClientRect(): Pick<DOMRect, 'left' | 'top' | 'width' | 'height'> {
    return { left: 0, top: 0, width: 200, height: 100 };
  }

  setPointerCapture(pointerId: number): void {
    this.setCalls.push(pointerId);
    this.captured.add(pointerId);
  }

  releasePointerCapture(pointerId: number): void {
    this.releaseCalls.push(pointerId);
    this.captured.delete(pointerId);
  }

  hasPointerCapture(pointerId: number): boolean {
    return this.captured.has(pointerId);
  }
}

const pointerEvent = (
  target: FakePointerTarget,
  overrides: Partial<BrowserViewerPointerEventLike> = {}
): BrowserViewerPointerEventLike => ({
  pointerId: 7,
  clientX: 20,
  clientY: 25,
  button: 0,
  buttons: 1,
  isPrimary: true,
  currentTarget: target,
  ...overrides,
});

describe('browser viewer pointer lifecycle', () => {
  test('captures a drag and releases it at the last valid point when pointerup happens outside', () => {
    const inputs: Array<Record<string, unknown>> = [];
    const target = new FakePointerTarget();
    const pointer = new BrowserViewerPointerController({
      getFrame: () => ({ width: 200, height: 100 }),
      sendInput: (input) => {
        inputs.push(input);
        return true;
      },
    });

    expect(pointer.pointerDown(pointerEvent(target))).toBe(true);
    expect(
      pointer.pointerMove(
        pointerEvent(target, {
          clientX: 150,
          clientY: 80,
          buttons: 1,
        })
      )
    ).toBe(true);
    expect(
      pointer.pointerUp(
        pointerEvent(target, {
          clientX: 300,
          clientY: 180,
          buttons: 0,
        })
      )
    ).toBe(true);

    expect(target.setCalls).toEqual([7]);
    expect(target.releaseCalls).toEqual([7]);
    expect(pointer.pressedCount).toBe(0);
    expect(inputs.map((input) => input.action)).toEqual(['down', 'move', 'up']);
    expect(inputs[2]).toMatchObject({
      action: 'up',
      x: 150,
      y: 80,
      button: 0,
      buttons: 0,
    });
  });

  test('pointercancel and unmount cleanup both send a final up so pressed state cannot stick', () => {
    const inputs: Array<Record<string, unknown>> = [];
    const target = new FakePointerTarget();
    const pointer = new BrowserViewerPointerController({
      getFrame: () => ({ width: 200, height: 100 }),
      sendInput: (input) => {
        inputs.push(input);
        return true;
      },
    });

    pointer.pointerDown(pointerEvent(target, { pointerId: 11, clientX: 40, clientY: 30 }));
    expect(
      pointer.pointerCancel(
        pointerEvent(target, {
          pointerId: 11,
          clientX: 500,
          clientY: 500,
          buttons: 0,
        })
      )
    ).toBe(true);
    expect(inputs.at(-1)).toMatchObject({
      action: 'up',
      x: 40,
      y: 30,
      buttons: 0,
    });

    pointer.pointerDown(pointerEvent(target, { pointerId: 12, clientX: 60, clientY: 50 }));
    expect(pointer.releaseAll()).toBe(1);
    expect(pointer.pressedCount).toBe(0);
    expect(inputs.at(-1)).toMatchObject({
      action: 'up',
      x: 60,
      y: 50,
      buttons: 0,
    });
    expect(target.releaseCalls).toEqual([11, 12]);
  });
});
