/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type React from 'react';
import { describe, expect, test } from 'bun:test';
import {
  BrowserViewerPointerController,
  BrowserViewerTextInputController,
  MAX_BROWSER_VIEWER_TEXT_BYTES,
  type BrowserViewerPointerCaptureTarget,
} from './browserViewerInput';
import { createBrowserViewerInteractionHandlers } from './browserViewerHandlers';

const inputFromMessage = (
  message: Record<string, unknown>
): Record<string, unknown> | undefined =>
  message.input && typeof message.input === 'object'
    ? (message.input as Record<string, unknown>)
    : undefined;

class HandlerPointerTarget implements BrowserViewerPointerCaptureTarget {
  value = '';
  readonly captured = new Set<number>();
  readonly releases: number[] = [];

  getBoundingClientRect(): Pick<DOMRect, 'left' | 'top' | 'width' | 'height'> {
    return { left: 0, top: 0, width: 200, height: 100 };
  }

  setPointerCapture(pointerId: number): void {
    this.captured.add(pointerId);
  }

  releasePointerCapture(pointerId: number): void {
    this.releases.push(pointerId);
    this.captured.delete(pointerId);
  }

  hasPointerCapture(pointerId: number): boolean {
    return this.captured.has(pointerId);
  }
}

const harness = (readOnly = false, interactionEnabled = !readOnly) => {
  const messages: Array<Record<string, unknown>> = [];
  let currentReadOnly = readOnly;
  let currentInteractionEnabled = interactionEnabled;
  const send = (message: Record<string, unknown>): boolean => {
    messages.push(message);
    return true;
  };
  const textInput = new BrowserViewerTextInputController(send, {
    deferClear: () => undefined,
  });
  const pointerInput = new BrowserViewerPointerController({
    getFrame: () => ({ width: 200, height: 100 }),
    sendInput: (input) =>
      send({
        type: 'input',
        input,
      }),
  });
  const handlers = createBrowserViewerInteractionHandlers({
    readOnly: () => currentReadOnly,
    interactionEnabled: () => currentInteractionEnabled,
    sendInput: (input) =>
      send({
        type: 'input',
        input,
      }),
    textInput,
    pointerInput,
  });
  return {
    handlers,
    messages,
    pointerInput,
    setReadOnly: (next: boolean) => {
      currentReadOnly = next;
    },
    setInteractionEnabled: (next: boolean) => {
      currentInteractionEnabled = next;
    },
  };
};

const formEvent = (
  target: HandlerPointerTarget,
  nativeEvent: Partial<InputEvent>
): React.FormEvent<HTMLTextAreaElement> => {
  let prevented = false;
  return {
    currentTarget: target,
    nativeEvent,
    preventDefault: () => {
      prevented = true;
    },
    get defaultPrevented() {
      return prevented;
    },
  } as unknown as React.FormEvent<HTMLTextAreaElement>;
};

const compositionEvent = (
  target: HandlerPointerTarget,
  data: string
): React.CompositionEvent<HTMLTextAreaElement> => {
  let prevented = false;
  return {
    currentTarget: target,
    data,
    preventDefault: () => {
      prevented = true;
    },
    get defaultPrevented() {
      return prevented;
    },
  } as unknown as React.CompositionEvent<HTMLTextAreaElement>;
};

const pasteEvent = (
  target: HandlerPointerTarget,
  text: string
): React.ClipboardEvent<HTMLTextAreaElement> => {
  let prevented = false;
  return {
    currentTarget: target,
    clipboardData: {
      getData: (format: string) => (format === 'text/plain' ? text : ''),
    },
    preventDefault: () => {
      prevented = true;
    },
    get defaultPrevented() {
      return prevented;
    },
  } as unknown as React.ClipboardEvent<HTMLTextAreaElement>;
};

const keyEvent = (
  target: HandlerPointerTarget,
  key: string,
  code: string,
  overrides: Partial<React.KeyboardEvent<HTMLTextAreaElement>> = {}
): React.KeyboardEvent<HTMLTextAreaElement> => {
  let prevented = false;
  return {
    currentTarget: target,
    key,
    code,
    keyCode: 0,
    repeat: false,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    nativeEvent: { isComposing: false },
    getModifierState: () => false,
    preventDefault: () => {
      prevented = true;
    },
    get defaultPrevented() {
      return prevented;
    },
    ...overrides,
  } as unknown as React.KeyboardEvent<HTMLTextAreaElement>;
};

const pointerEvent = (
  target: HandlerPointerTarget,
  overrides: Partial<React.PointerEvent<HTMLImageElement>> = {}
): React.PointerEvent<HTMLImageElement> => {
  let prevented = false;
  return {
    currentTarget: target,
    pointerId: 8,
    clientX: 20,
    clientY: 25,
    button: 0,
    buttons: 1,
    isPrimary: true,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    preventDefault: () => {
      prevented = true;
    },
    get defaultPrevented() {
      return prevented;
    },
    ...overrides,
  } as unknown as React.PointerEvent<HTMLImageElement>;
};

describe('browser viewer component interaction handlers', () => {
  test('composition, beforeinput, and paste handlers emit one UTF-8-safe text stream', () => {
    const { handlers, messages } = harness();
    const target = new HandlerPointerTarget();
    const composition = `${'中文🙂'.repeat(80)}结束`;

    handlers.onCompositionStart(
      compositionEvent(target, '') as React.CompositionEvent<HTMLTextAreaElement>
    );
    const intermediate = formEvent(target, {
      data: 'zhong',
      inputType: 'insertCompositionText',
      isComposing: true,
    });
    handlers.onBeforeInput(intermediate);
    expect(intermediate.defaultPrevented).toBe(false);

    target.value = composition;
    const committed = compositionEvent(target, composition);
    handlers.onCompositionEnd(committed);
    expect(committed.defaultPrevented).toBe(true);
    expect(target.value).toBe('');

    const duplicate = formEvent(target, {
      data: composition,
      inputType: 'insertFromComposition',
      isComposing: false,
    });
    handlers.onBeforeInput(duplicate);
    expect(duplicate.defaultPrevented).toBe(true);

    const paste = `${'粘贴🙂'.repeat(90)}\n第二行`;
    const pasted = pasteEvent(target, paste);
    handlers.onPaste(pasted);
    expect(pasted.defaultPrevented).toBe(true);
    const pasteEcho = formEvent(target, {
      data: paste,
      inputType: 'insertFromPaste',
      isComposing: false,
    });
    handlers.onBeforeInput(pasteEcho);
    expect(pasteEcho.defaultPrevented).toBe(true);

    const textInputs = messages
      .map(inputFromMessage)
      .filter((input): input is Record<string, unknown> => input?.kind === 'text');
    expect(textInputs.map((input) => input.text).join('')).toBe(composition + paste);
    for (const input of textInputs) {
      expect(new TextEncoder().encode(String(input.text)).byteLength).toBeLessThanOrEqual(
        MAX_BROWSER_VIEWER_TEXT_BYTES
      );
    }
  });

  test('printable keydown stays on beforeinput while navigation keydown/up send once', () => {
    const { handlers, messages } = harness();
    const target = new HandlerPointerTarget();

    const printableDown = keyEvent(target, 'a', 'KeyA');
    const printableUp = keyEvent(target, 'a', 'KeyA');
    handlers.onKeyDown(printableDown);
    handlers.onKeyUp(printableUp);
    expect(printableDown.defaultPrevented).toBe(false);
    expect(printableUp.defaultPrevented).toBe(false);

    const inserted = formEvent(target, {
      data: 'a',
      inputType: 'insertText',
      isComposing: false,
    });
    handlers.onBeforeInput(inserted);
    expect(inserted.defaultPrevented).toBe(true);

    const arrowDown = keyEvent(target, 'ArrowLeft', 'ArrowLeft');
    const arrowUp = keyEvent(target, 'ArrowLeft', 'ArrowLeft');
    handlers.onKeyDown(arrowDown);
    handlers.onKeyUp(arrowUp);
    expect(arrowDown.defaultPrevented).toBe(true);
    expect(arrowUp.defaultPrevented).toBe(true);

    expect(messages.map(inputFromMessage)).toEqual([
      { kind: 'text', text: 'a' },
      {
        kind: 'key',
        action: 'down',
        key: 'ArrowLeft',
        code: 'ArrowLeft',
        repeat: false,
        modifiers: { alt: false, ctrl: false, meta: false, shift: false },
      },
      {
        kind: 'key',
        action: 'up',
        key: 'ArrowLeft',
        code: 'ArrowLeft',
        modifiers: { alt: false, ctrl: false, meta: false, shift: false },
      },
    ]);
  });

  test('authenticated_replica handlers remain observable but never mutate or send input', () => {
    const { handlers, messages } = harness(true);
    const target = new HandlerPointerTarget();
    target.value = 'visible-local-value';

    const beforeInput = formEvent(target, {
      data: 'blocked',
      inputType: 'insertText',
      isComposing: false,
    });
    const paste = pasteEvent(target, 'blocked paste');
    const key = keyEvent(target, 'ArrowLeft', 'ArrowLeft');
    const pointer = pointerEvent(target);

    handlers.onCompositionStart(compositionEvent(target, ''));
    handlers.onCompositionEnd(compositionEvent(target, 'blocked'));
    handlers.onBeforeInput(beforeInput);
    handlers.onPaste(paste);
    handlers.onKeyDown(key);
    handlers.onKeyUp(key);
    handlers.onPointerDown(pointer);
    handlers.onPointerMove(pointer);
    handlers.onPointerUp(pointer);
    handlers.onPointerCancel(pointer);
    handlers.onLostPointerCapture(pointer);

    expect(messages).toEqual([]);
    expect(target.value).toBe('visible-local-value');
    expect(beforeInput.defaultPrevented).toBe(false);
    expect(paste.defaultPrevented).toBe(false);
    expect(key.defaultPrevented).toBe(false);
    expect(pointer.defaultPrevented).toBe(false);
    expect(target.captured.size).toBe(0);
  });

  test('agent and idle control states block every text, key, and pointer input path', () => {
    const { handlers, messages, setInteractionEnabled } = harness(false, false);
    const target = new HandlerPointerTarget();
    target.value = 'local draft';

    for (const enabled of [false, false]) {
      setInteractionEnabled(enabled);
      const beforeInput = formEvent(target, {
        data: 'blocked',
        inputType: 'insertText',
        isComposing: false,
      });
      const paste = pasteEvent(target, 'blocked paste');
      const key = keyEvent(target, 'ArrowLeft', 'ArrowLeft');
      const pointer = pointerEvent(target);
      handlers.onCompositionStart(compositionEvent(target, ''));
      handlers.onCompositionEnd(compositionEvent(target, 'blocked'));
      handlers.onBeforeInput(beforeInput);
      handlers.onPaste(paste);
      handlers.onKeyDown(key);
      handlers.onKeyUp(key);
      handlers.onPointerDown(pointer);
      handlers.onPointerMove(pointer);
      handlers.onPointerUp(pointer);
      handlers.onPointerCancel(pointer);

      expect(beforeInput.defaultPrevented).toBe(false);
      expect(paste.defaultPrevented).toBe(false);
      expect(key.defaultPrevented).toBe(false);
      expect(pointer.defaultPrevented).toBe(false);
    }

    expect(messages).toEqual([]);
    expect(target.value).toBe('local draft');
    expect(target.captured.size).toBe(0);
  });

  test('bound pointer handlers capture, release on up/cancel, and support unmount releaseAll', () => {
    const { handlers, messages, pointerInput } = harness();
    const target = new HandlerPointerTarget();

    const down = pointerEvent(target);
    handlers.onPointerDown(down);
    expect(down.defaultPrevented).toBe(true);
    expect(target.captured.has(8)).toBe(true);

    handlers.onPointerMove(
      pointerEvent(target, { clientX: 150, clientY: 80, buttons: 1 })
    );
    handlers.onPointerUp(
      pointerEvent(target, { clientX: 400, clientY: 400, buttons: 0 })
    );
    expect(target.releases).toEqual([8]);
    expect(pointerInput.pressedCount).toBe(0);

    handlers.onPointerDown(pointerEvent(target, { pointerId: 9, clientX: 60, clientY: 40 }));
    handlers.onPointerCancel(
      pointerEvent(target, {
        pointerId: 9,
        clientX: 500,
        clientY: 500,
        buttons: 0,
      })
    );
    expect(target.releases).toEqual([8, 9]);

    handlers.onPointerDown(pointerEvent(target, { pointerId: 10, clientX: 80, clientY: 50 }));
    expect(pointerInput.releaseAll()).toBe(1);
    expect(target.releases).toEqual([8, 9, 10]);
    expect(
      messages
        .map(inputFromMessage)
        .filter((input) => input?.kind === 'pointer')
        .map((input) => input?.action)
    ).toEqual(['down', 'move', 'up', 'down', 'up', 'down', 'up']);
  });
});
