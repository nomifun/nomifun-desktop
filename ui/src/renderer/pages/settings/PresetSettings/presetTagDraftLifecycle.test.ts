/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createPresetTagDraftLifecycle } from './presetTagDraftLifecycle';

describe('preset tag draft lifecycle', () => {
  test('resets both drafts before saving without changing drawer visibility', () => {
    const calls: string[] = [];
    const lifecycle = createPresetTagDraftLifecycle(
      { current: { resetPendingTag: () => calls.push('audience') } },
      { current: { resetPendingTag: () => calls.push('scenario') } },
      (visible) => calls.push(`visible:${visible}`),
      () => calls.push('save')
    );

    lifecycle.handleDrawerSave();

    expect(calls).toEqual(['audience', 'scenario', 'save']);
  });

  test('resets both drafts before closing the drawer', () => {
    const calls: string[] = [];
    const lifecycle = createPresetTagDraftLifecycle(
      { current: { resetPendingTag: () => calls.push('audience') } },
      { current: { resetPendingTag: () => calls.push('scenario') } },
      (visible) => calls.push(`visible:${visible}`),
      () => calls.push('save')
    );

    lifecycle.closeDrawer();

    expect(calls).toEqual(['audience', 'scenario', 'visible:false']);
  });

  test('allows the drawer to be hidden before either picker mounts', () => {
    const lifecycle = createPresetTagDraftLifecycle(
      { current: null },
      { current: null },
      () => {},
      () => {}
    );

    lifecycle.resetPendingTagDrafts();
  });
});
