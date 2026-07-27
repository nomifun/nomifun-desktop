/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type React from 'react';
import type { BrowserViewerSend } from './browserViewerActions';
import {
  BrowserViewerPointerController,
  BrowserViewerTextInputController,
  handleBrowserViewerKeyboardEvent,
} from './browserViewerInput';

export interface BrowserViewerInteractionHandlersOptions {
  readOnly: () => boolean;
  /**
   * Input surfaces may be enabled before a lease exists: the first event is
   * sent to the server's automatic takeover path. Toolbar commands do not use
   * these handlers and remain user-lease gated in the viewer component.
   */
  interactionEnabled?: () => boolean;
  sendInput: BrowserViewerSend;
  textInput: BrowserViewerTextInputController;
  pointerInput: BrowserViewerPointerController;
}

export interface BrowserViewerInteractionHandlers {
  onCompositionStart: React.CompositionEventHandler<HTMLTextAreaElement>;
  onCompositionEnd: React.CompositionEventHandler<HTMLTextAreaElement>;
  onBeforeInput: React.FormEventHandler<HTMLTextAreaElement>;
  onPaste: React.ClipboardEventHandler<HTMLTextAreaElement>;
  onKeyDown: React.KeyboardEventHandler<HTMLTextAreaElement>;
  onKeyUp: React.KeyboardEventHandler<HTMLTextAreaElement>;
  onPointerMove: React.PointerEventHandler<HTMLImageElement>;
  onPointerDown: React.PointerEventHandler<HTMLImageElement>;
  onPointerUp: React.PointerEventHandler<HTMLImageElement>;
  onPointerCancel: React.PointerEventHandler<HTMLImageElement>;
  onLostPointerCapture: React.PointerEventHandler<HTMLImageElement>;
}

const keyboardEventLike = (
  event: React.KeyboardEvent<HTMLTextAreaElement>
) => ({
  key: event.key,
  code: event.code,
  keyCode: event.keyCode,
  repeat: event.repeat,
  isComposing: event.nativeEvent.isComposing,
  altKey: event.altKey,
  ctrlKey: event.ctrlKey,
  metaKey: event.metaKey,
  shiftKey: event.shiftKey,
  getModifierState: (key: string) =>
    event.getModifierState(key as React.ModifierKey),
});

/**
 * Creates the exact handlers bound by EmbeddedBrowserViewer. Keeping this
 * adapter outside the component lets behavior tests invoke the same handlers
 * React receives rather than merely testing the underlying controllers.
 */
export const createBrowserViewerInteractionHandlers = (
  options: BrowserViewerInteractionHandlersOptions
): BrowserViewerInteractionHandlers => {
  const inputEnabled = (): boolean =>
    options.interactionEnabled?.() ?? !options.readOnly();

  return {
  onCompositionStart: () => {
    if (inputEnabled()) options.textInput.compositionStart();
  },

  onCompositionEnd: (event) => {
    if (!inputEnabled()) return;
    event.preventDefault();
    options.textInput.compositionEnd(event.data);
    event.currentTarget.value = '';
  },

  onBeforeInput: (event) => {
    if (!inputEnabled()) return;
    const nativeEvent = event.nativeEvent as InputEvent;
    const result = options.textInput.beforeInput({
      data: nativeEvent.data,
      inputType: nativeEvent.inputType,
      isComposing: nativeEvent.isComposing,
    });
    if (result.preventDefault) event.preventDefault();
    event.currentTarget.value = '';
  },

  onPaste: (event) => {
    if (!inputEnabled()) return;
    const text = event.clipboardData.getData('text/plain');
    if (!text) return;
    event.preventDefault();
    options.textInput.paste(text);
    event.currentTarget.value = '';
  },

  onKeyDown: (event) => {
    if (!inputEnabled()) return;
    const result = handleBrowserViewerKeyboardEvent(
      options.sendInput,
      'down',
      keyboardEventLike(event)
    );
    if (result.preventDefault) event.preventDefault();
  },

  onKeyUp: (event) => {
    if (!inputEnabled()) return;
    const result = handleBrowserViewerKeyboardEvent(
      options.sendInput,
      'up',
      keyboardEventLike(event)
    );
    if (result.preventDefault) event.preventDefault();
  },

  onPointerMove: (event) => {
    if (!inputEnabled()) return;
    options.pointerInput.pointerMove(event);
  },

  onPointerDown: (event) => {
    if (!inputEnabled()) return;
    event.preventDefault();
    options.pointerInput.pointerDown(event);
  },

  onPointerUp: (event) => {
    if (!inputEnabled()) return;
    options.pointerInput.pointerUp(event);
  },

  onPointerCancel: (event) => {
    if (!inputEnabled()) return;
    options.pointerInput.pointerCancel(event);
  },

  onLostPointerCapture: (event) => {
    options.pointerInput.lostPointerCapture(event.pointerId);
  },
  };
};
