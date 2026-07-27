/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { BrowserViewerSend } from './browserViewerActions';
import {
  mapBrowserViewerPoint,
  type BrowserFrameSize,
  type BrowserViewerPoint,
} from './browserViewerProtocol';

export const MAX_BROWSER_VIEWER_TEXT_BYTES = 256;

export interface BrowserViewerModifiers {
  alt: boolean;
  ctrl: boolean;
  meta: boolean;
  shift: boolean;
}

interface ModifierEventLike {
  altKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
}

export const browserViewerModifiersFor = (
  event: ModifierEventLike
): BrowserViewerModifiers => ({
  alt: Boolean(event.altKey),
  ctrl: Boolean(event.ctrlKey),
  meta: Boolean(event.metaKey),
  shift: Boolean(event.shiftKey),
});

export const isBrowserViewerReadOnlyIdentity = (
  identityMode?: string | null
): boolean => identityMode !== 'primary' && identityMode !== 'isolated';

/**
 * Anonymous and authenticated-replica lanes may keep the observation socket
 * open, but no command capable of changing browser state is allowed to reach
 * them. Only the Primary live identity and an explicitly isolated identity
 * permit user interaction.
 */
export const sendBrowserViewerCommand = (
  send: BrowserViewerSend,
  readOnly: boolean,
  message: Record<string, unknown>,
  connectionReady = true
): boolean => {
  if (readOnly && message.type !== 'observe') return false;
  if (!connectionReady && message.type !== 'observe') return false;
  return send(message);
};

/**
 * Splits text without ever cutting a UTF-8 sequence. Iterating a JavaScript
 * string by code point also keeps surrogate pairs (for example emoji) intact.
 */
export const splitBrowserViewerText = (
  text: string,
  maxBytes = MAX_BROWSER_VIEWER_TEXT_BYTES
): string[] => {
  if (!Number.isSafeInteger(maxBytes) || maxBytes < 4) {
    throw new RangeError('Browser viewer text chunks must allow one UTF-8 code point.');
  }

  const encoder = new TextEncoder();
  const chunks: string[] = [];
  let chunk = '';
  let chunkBytes = 0;

  for (const codePoint of text) {
    const codePointBytes = encoder.encode(codePoint).byteLength;
    if (chunk && chunkBytes + codePointBytes > maxBytes) {
      chunks.push(chunk);
      chunk = '';
      chunkBytes = 0;
    }
    chunk += codePoint;
    chunkBytes += codePointBytes;
  }

  if (chunk) chunks.push(chunk);
  return chunks;
};

export const sendBrowserViewerText = (
  send: BrowserViewerSend,
  text: string
): boolean => {
  const chunks = splitBrowserViewerText(text);
  if (chunks.length === 0) return false;

  for (const chunk of chunks) {
    if (
      !send({
        type: 'input',
        input: {
          kind: 'text',
          text: chunk,
        },
      })
    ) {
      return false;
    }
  }
  return true;
};

export interface BrowserViewerBeforeInput {
  data?: string | null;
  inputType?: string;
  isComposing?: boolean;
}

export interface BrowserViewerTextEventResult {
  preventDefault: boolean;
  sent: boolean;
}

type PendingTextEchoSource = 'composition' | 'paste';

interface PendingTextEcho {
  source: PendingTextEchoSource;
  text: string;
  token: number;
}

export interface BrowserViewerTextInputControllerOptions {
  deferClear?: (callback: () => void) => void;
}

/**
 * Coalesces the overlapping DOM text event families into exactly one remote
 * text insertion:
 * - intermediate IME beforeinput events stay local;
 * - compositionend sends the committed text;
 * - paste sends clipboard text;
 * - a matching follow-up beforeinput is consumed instead of duplicated.
 */
export class BrowserViewerTextInputController {
  private composing = false;
  private pendingEcho: PendingTextEcho | null = null;
  private echoToken = 0;
  private readonly deferClear: (callback: () => void) => void;

  constructor(
    private readonly send: BrowserViewerSend,
    options: BrowserViewerTextInputControllerOptions = {}
  ) {
    this.deferClear =
      options.deferClear ??
      ((callback) => {
        setTimeout(callback, 0);
      });
  }

  compositionStart(): void {
    this.composing = true;
    this.clearPendingEcho();
  }

  compositionEnd(text: string): boolean {
    this.composing = false;
    if (!text) return false;
    if (
      this.pendingEcho?.source === 'composition' &&
      this.pendingEcho.text === text
    ) {
      this.clearPendingEcho();
      return false;
    }
    const sent = sendBrowserViewerText(this.send, text);
    if (sent) this.rememberEcho('composition', text);
    return sent;
  }

  beforeInput(event: BrowserViewerBeforeInput): BrowserViewerTextEventResult {
    const data = event.data ?? '';
    const inputType = event.inputType ?? '';
    const isInsertion = inputType === '' || inputType.startsWith('insert');
    if (!isInsertion || !data) {
      return { preventDefault: false, sent: false };
    }

    if (this.consumeEcho(data, inputType)) {
      return { preventDefault: true, sent: false };
    }

    if (
      this.composing ||
      event.isComposing === true ||
      inputType === 'insertCompositionText'
    ) {
      return { preventDefault: false, sent: false };
    }

    const sent = sendBrowserViewerText(this.send, data);
    if (sent && inputType === 'insertFromPaste') {
      this.rememberEcho('paste', data);
    } else if (sent && inputType === 'insertFromComposition') {
      this.rememberEcho('composition', data);
    }
    return { preventDefault: true, sent };
  }

  paste(text: string): boolean {
    if (!text) return false;
    if (
      this.pendingEcho?.source === 'paste' &&
      this.pendingEcho.text === text
    ) {
      this.clearPendingEcho();
      return false;
    }
    const sent = sendBrowserViewerText(this.send, text);
    if (sent) this.rememberEcho('paste', text);
    return sent;
  }

  reset(): void {
    this.composing = false;
    this.clearPendingEcho();
  }

  private rememberEcho(source: PendingTextEchoSource, text: string): void {
    const token = ++this.echoToken;
    this.pendingEcho = { source, text, token };
    this.deferClear(() => {
      if (this.pendingEcho?.token === token) this.pendingEcho = null;
    });
  }

  private consumeEcho(text: string, inputType: string): boolean {
    const pending = this.pendingEcho;
    if (!pending || pending.text !== text) return false;

    const matchesSource =
      pending.source === 'paste'
        ? inputType === 'insertFromPaste'
        : inputType === 'insertFromComposition' ||
          inputType === 'insertCompositionText' ||
          inputType === 'insertText' ||
          inputType === '';
    if (!matchesSource) return false;

    this.clearPendingEcho();
    return true;
  }

  private clearPendingEcho(): void {
    this.echoToken++;
    this.pendingEcho = null;
  }
}

export interface BrowserViewerKeyboardEventLike extends ModifierEventLike {
  key: string;
  code?: string;
  keyCode?: number;
  isComposing?: boolean;
  getModifierState?: (key: string) => boolean;
}

const isPasteShortcut = (event: BrowserViewerKeyboardEventLike): boolean => {
  const key = event.key.toLowerCase();
  const commandPaste =
    key === 'v' &&
    Boolean(event.ctrlKey || event.metaKey) &&
    !event.altKey;
  const insertPaste =
    event.key === 'Insert' &&
    Boolean(event.shiftKey) &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.altKey;
  return commandPaste || insertPaste;
};

/**
 * These key events must be left to the hidden textarea so beforeinput,
 * composition, or paste can produce the sole text command.
 */
export const browserViewerKeyUsesTextInput = (
  event: BrowserViewerKeyboardEventLike
): boolean => {
  if (
    event.isComposing ||
    event.keyCode === 229 ||
    event.key === 'Process' ||
    event.key === 'Dead' ||
    event.key === 'Unidentified' ||
    isPasteShortcut(event)
  ) {
    return true;
  }

  const isAltGraph = event.getModifierState?.('AltGraph') === true;
  const isSingleCodePoint = [...event.key].length === 1;
  const hasCommandModifier =
    Boolean(event.ctrlKey || event.metaKey || event.altKey) && !isAltGraph;
  return isSingleCodePoint && !hasCommandModifier;
};

export type BrowserViewerKeyAction = 'down' | 'up';

export interface BrowserViewerKeyInput {
  [key: string]: unknown;
  kind: 'key';
  action: BrowserViewerKeyAction;
  key: string;
  code: string;
  repeat?: boolean;
  modifiers: BrowserViewerModifiers;
}

export interface BrowserViewerKeyboardEventResult {
  preventDefault: boolean;
  sent: boolean;
}

/**
 * Converts only non-text key events into remote key commands. Printable,
 * composition, dead-key, AltGraph, and paste keydowns stay on the DOM text
 * event path so they cannot be emitted twice.
 */
export const handleBrowserViewerKeyboardEvent = (
  sendInput: (input: BrowserViewerKeyInput) => boolean,
  action: BrowserViewerKeyAction,
  event: BrowserViewerKeyboardEventLike & { repeat?: boolean }
): BrowserViewerKeyboardEventResult => {
  if (browserViewerKeyUsesTextInput(event)) {
    return { preventDefault: false, sent: false };
  }

  const sent = sendInput({
    kind: 'key',
    action,
    key: event.key,
    code: event.code ?? '',
    ...(action === 'down' ? { repeat: Boolean(event.repeat) } : {}),
    modifiers: browserViewerModifiersFor(event),
  });
  return { preventDefault: true, sent };
};

export interface BrowserViewerPointerInput {
  [key: string]: unknown;
  kind: 'pointer';
  action: 'move' | 'down' | 'up';
  x: number;
  y: number;
  button: number;
  buttons: number;
  modifiers: BrowserViewerModifiers;
}

export interface BrowserViewerPointerCaptureTarget {
  getBoundingClientRect(): Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>;
  setPointerCapture(pointerId: number): void;
  releasePointerCapture(pointerId: number): void;
  hasPointerCapture?(pointerId: number): boolean;
}

export interface BrowserViewerPointerEventLike extends ModifierEventLike {
  pointerId: number;
  clientX: number;
  clientY: number;
  button: number;
  buttons: number;
  isPrimary?: boolean;
  currentTarget: BrowserViewerPointerCaptureTarget;
}

interface PressedBrowserViewerPointer {
  pointerId: number;
  button: number;
  buttons: number;
  point: BrowserViewerPoint;
  modifiers: BrowserViewerModifiers;
  target: BrowserViewerPointerCaptureTarget;
}

export interface BrowserViewerPointerControllerOptions {
  sendInput: (input: BrowserViewerPointerInput) => boolean;
  getFrame: () => BrowserFrameSize | null;
  onEngage?: () => void;
}

/**
 * Owns the complete pressed-pointer lifecycle. Pointer capture keeps drag
 * events routed to the frame, while the last valid in-frame point lets an
 * outside pointerup/cancel still release the remote mouse button.
 */
export class BrowserViewerPointerController {
  private readonly pressed = new Map<number, PressedBrowserViewerPointer>();

  constructor(private readonly options: BrowserViewerPointerControllerOptions) {}

  pointerDown(event: BrowserViewerPointerEventLike): boolean {
    if (event.isPrimary === false || event.button < 0 || event.button > 4) return false;
    const point = this.pointFor(event);
    if (!point) return false;

    const modifiers = browserViewerModifiersFor(event);
    const sent = this.options.sendInput({
      kind: 'pointer',
      action: 'down',
      x: point.x,
      y: point.y,
      button: event.button,
      buttons: event.buttons,
      modifiers,
    });
    if (!sent) return false;

    this.pressed.set(event.pointerId, {
      pointerId: event.pointerId,
      button: event.button,
      buttons: event.buttons,
      point,
      modifiers,
      target: event.currentTarget,
    });
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // The tracked pointer is still released on pointerup/cancel/unmount.
    }
    this.options.onEngage?.();
    return true;
  }

  pointerMove(event: BrowserViewerPointerEventLike): boolean {
    const pressed = this.pressed.get(event.pointerId);
    if (!pressed) return false;
    const point = this.pointFor(event);
    if (!point) return true;

    pressed.point = point;
    pressed.buttons = event.buttons;
    pressed.modifiers = browserViewerModifiersFor(event);
    this.options.sendInput({
      kind: 'pointer',
      action: 'move',
      x: point.x,
      y: point.y,
      button: pressed.button,
      buttons: event.buttons,
      modifiers: pressed.modifiers,
    });
    return true;
  }

  pointerUp(event: BrowserViewerPointerEventLike): boolean {
    return this.finishPointer(event.pointerId, event, event.buttons);
  }

  pointerCancel(event: BrowserViewerPointerEventLike): boolean {
    return this.finishPointer(event.pointerId, event, 0);
  }

  lostPointerCapture(pointerId: number): boolean {
    return this.finishPointer(pointerId, undefined, 0);
  }

  releaseAll(): number {
    const pointers = [...this.pressed.values()];
    this.pressed.clear();
    let released = 0;
    for (const pointer of pointers) {
      this.releaseCapture(pointer);
      if (
        this.options.sendInput({
          kind: 'pointer',
          action: 'up',
          x: pointer.point.x,
          y: pointer.point.y,
          button: pointer.button,
          buttons: 0,
          modifiers: pointer.modifiers,
        })
      ) {
        released++;
      }
    }
    return released;
  }

  get pressedCount(): number {
    return this.pressed.size;
  }

  private finishPointer(
    pointerId: number,
    event?: BrowserViewerPointerEventLike,
    buttons = 0
  ): boolean {
    const pressed = this.pressed.get(pointerId);
    if (!pressed) return false;
    this.pressed.delete(pointerId);

    const point = event ? this.pointFor(event) ?? pressed.point : pressed.point;
    const modifiers = event
      ? browserViewerModifiersFor(event)
      : pressed.modifiers;
    this.releaseCapture(pressed);
    this.options.sendInput({
      kind: 'pointer',
      action: 'up',
      x: point.x,
      y: point.y,
      button: pressed.button,
      buttons,
      modifiers,
    });
    return true;
  }

  private pointFor(event: BrowserViewerPointerEventLike): BrowserViewerPoint | null {
    const frame = this.options.getFrame();
    if (!frame) return null;
    return mapBrowserViewerPoint(
      event.currentTarget.getBoundingClientRect(),
      frame,
      event.clientX,
      event.clientY
    );
  }

  private releaseCapture(pointer: PressedBrowserViewerPointer): void {
    try {
      if (
        !pointer.target.hasPointerCapture ||
        pointer.target.hasPointerCapture(pointer.pointerId)
      ) {
        pointer.target.releasePointerCapture(pointer.pointerId);
      }
    } catch {
      // The element may already be detached; the local pressed state is clear.
    }
  }
}
