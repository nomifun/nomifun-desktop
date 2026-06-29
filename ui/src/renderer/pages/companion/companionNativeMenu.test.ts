/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'vitest';
import { buildCompanionMenuEntries } from './companionNativeMenu';

describe('buildCompanionMenuEntries', () => {
  it('keeps the desktop companion context menu actions in order', () => {
    const entries = buildCompanionMenuEntries({
      name: '团团',
      t: (key, params) => {
        if (key === 'nomi.companion.menuOpenConfig') return `打开 ${params?.name} 的设置`;
        return key;
      },
    });

    expect(entries).toEqual([
      { action: 'open-chat', text: 'nomi.companion.menuOpenChat' },
      { action: 'open-memories', text: 'nomi.companion.menuOpenMemories' },
      { action: 'open-config', text: '打开 团团 的设置' },
      { action: 'clear-unread', text: 'nomi.companion.menuClearUnread' },
      { action: 'hide', text: 'nomi.companion.menuHide' },
    ]);
  });
});
