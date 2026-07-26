/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  BrowserViewerPressedKeyTracker,
  canSendEmbeddedBrowserViewerCommand,
  isEmbeddedBrowserViewerInputEnabled,
  isEmbeddedBrowserViewerInteractionEnabled,
  resolveBrowserViewerStreamPolicy,
} from './EmbeddedBrowserViewer';

describe('embedded browser viewer control lease', () => {
  test('enables all mutation surfaces only for a streaming user-controlled lane', () => {
    expect(isEmbeddedBrowserViewerInteractionEnabled(false, 'streaming', 'user')).toBe(
      true
    );
    expect(isEmbeddedBrowserViewerInteractionEnabled(false, 'streaming', 'agent')).toBe(
      false
    );
    expect(isEmbeddedBrowserViewerInteractionEnabled(false, 'streaming', 'idle')).toBe(
      false
    );
    expect(isEmbeddedBrowserViewerInteractionEnabled(false, 'connecting', 'user')).toBe(
      false
    );
    expect(isEmbeddedBrowserViewerInteractionEnabled(true, 'streaming', 'user')).toBe(
      false
    );
  });

  test('allows the first input to acquire control without enabling toolbar mutations', () => {
    for (const state of ['agent', 'idle'] as const) {
      expect(isEmbeddedBrowserViewerInputEnabled(false, 'streaming', state)).toBe(true);
      expect(isEmbeddedBrowserViewerInteractionEnabled(false, 'streaming', state)).toBe(false);
      expect(canSendEmbeddedBrowserViewerCommand('input', state)).toBe(true);
      expect(canSendEmbeddedBrowserViewerCommand('navigate', state)).toBe(false);
      expect(canSendEmbeddedBrowserViewerCommand('select_tab', state)).toBe(false);
    }
    expect(isEmbeddedBrowserViewerInputEnabled(true, 'streaming', 'agent')).toBe(false);
    expect(isEmbeddedBrowserViewerInputEnabled(false, 'connecting', 'agent')).toBe(false);
  });

  test('keeps headless streams on demand and external mode non-embedded', () => {
    expect(resolveBrowserViewerStreamPolicy('embedded', 'running', false)).toBe('automatic');
    expect(resolveBrowserViewerStreamPolicy('headless', 'running', false)).toBe('on_demand');
    expect(resolveBrowserViewerStreamPolicy('headless', 'running', true)).toBe('automatic');
    expect(resolveBrowserViewerStreamPolicy('external', 'running', true)).toBe('external');
    expect(resolveBrowserViewerStreamPolicy('embedded', 'queued', true)).toBe('unavailable');
  });

  test('releases every locally pressed key exactly once when a lease is lost', () => {
    const tracker = new BrowserViewerPressedKeyTracker();
    tracker.observe({
      kind: 'key',
      action: 'down',
      key: 'Shift',
      code: 'ShiftLeft',
      modifiers: { alt: false, ctrl: false, meta: false, shift: true },
    });
    tracker.observe({
      kind: 'key',
      action: 'down',
      key: 'ArrowLeft',
      code: 'ArrowLeft',
      modifiers: { alt: false, ctrl: false, meta: false, shift: true },
    });
    // A normal keyup is no longer pending when the lease cleanup runs.
    tracker.observe({
      kind: 'key',
      action: 'up',
      key: 'ArrowLeft',
      code: 'ArrowLeft',
      modifiers: { alt: false, ctrl: false, meta: false, shift: true },
    });

    const releases: Array<Record<string, unknown>> = [];
    expect(
      tracker.releaseAll((input) => {
        releases.push(input);
        return true;
      })
    ).toBe(1);
    expect(releases).toEqual([
      {
        kind: 'key',
        action: 'up',
        key: 'Shift',
        code: 'ShiftLeft',
        modifiers: { alt: false, ctrl: false, meta: false, shift: true },
      },
    ]);
    expect(tracker.pressedCount).toBe(0);
    expect(tracker.releaseAll(() => true)).toBe(0);
  });
});
