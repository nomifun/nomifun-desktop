/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const read = (url: URL) => readFileSync(url, 'utf8');

describe('desktop companion product boundary', () => {
  const nomi = read(new URL('../index.tsx', import.meta.url));
  const sessions = read(new URL('../../conversation/SessionList/index.tsx', import.meta.url));
  const guid = read(new URL('../../guid/GuidPage.tsx', import.meta.url));
  const guidShowcase = read(new URL('../../guid/components/GuidCompanionShowcase.tsx', import.meta.url));
  const companionWindow = read(new URL('../../companion/index.tsx', import.meta.url));
  const chat = read(new URL('../../conversation/components/ChatConversation.tsx', import.meta.url));

  test('the companion workspace owns cohabit and management modes', () => {
    expect(nomi.includes("modeParam === 'manage'")).toBe(true);
    expect(nomi.includes('<CompanionCohabitView')).toBe(true);
    expect(nomi.includes('styles.manageLayout')).toBe(true);
  });

  test('work conversations exclude companion sessions while the Guid keeps its creative showcase', () => {
    expect(sessions.includes('<CompanionSessionGroup')).toBe(false);
    expect(guid.includes('<GuidCompanionShowcase')).toBe(true);
    expect(guidShowcase.includes('&mode=cohabit')).toBe(true);
    expect(guidShowcase.includes('ensureCompanionSession')).toBe(false);
  });

  test('the collapsed desktop surface exposes direct companion switching', () => {
    expect(companionWindow.includes("className='nomi-companion-switcher'")).toBe(true);
    expect(companionWindow.includes('item.companion_id !== companionId')).toBe(true);
    expect(companionWindow.includes('activateCompanionWindow(item.companion_id)')).toBe(true);
    expect(companionWindow.includes('if (profileRef.current?.model)')).toBe(true);
  });

  test('the expanded quick window owns secondary actions and a persistent drag surface', () => {
    expect(companionWindow.includes("role='menu'")).toBe(true);
    expect(companionWindow.includes('nomi-companion-quick__footer')).toBe(false);
    expect(companionWindow.includes('<CloseSmall')).toBe(false);
    expect(companionWindow.includes('onMouseDown={startQuickPanelDrag}')).toBe(true);
    expect(companionWindow.includes('translateAnchorAfterDrag')).toBe(true);
  });

  test('legacy companion conversation URLs redirect into the cohabit mode', () => {
    expect(chat.includes('CompanionConversationRedirect')).toBe(true);
    expect(chat.includes('&mode=cohabit')).toBe(true);
  });
});
