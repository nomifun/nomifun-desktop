/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type CompanionMenuAction = 'open-chat' | 'open-config' | 'clear-unread' | 'hide';

export interface CompanionMenuEntry {
  action: CompanionMenuAction;
  text: string;
}

type Translate = (key: string, params?: Record<string, string>) => string;

export function buildCompanionMenuEntries(opts: { name: string; t: Translate }): CompanionMenuEntry[] {
  return [
    { action: 'open-chat', text: opts.t('nomi.companion.menuOpenChat') },
    { action: 'open-config', text: opts.t('nomi.companion.menuOpenConfig', { name: opts.name }) },
    { action: 'clear-unread', text: opts.t('nomi.companion.menuClearUnread') },
    { action: 'hide', text: opts.t('nomi.companion.menuHide') },
  ];
}
