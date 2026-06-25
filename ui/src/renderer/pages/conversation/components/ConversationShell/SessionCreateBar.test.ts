/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('session sidebar toolbar tooltips', () => {
  test('uses each action tooltip as its accessible label', () => {
    const actionsSource = readSource(
      new URL('../../../../components/layout/Sider/SiderNav/ConversationSiderActions.tsx', import.meta.url)
    );

    expect(actionsSource.includes('aria-label={tooltip}')).toBe(true);
    expect(actionsSource.includes('title={tooltip}')).toBe(false);
  });

  test('uses the local instant hover tooltip instead of Arco Tooltip for toolbar actions', () => {
    const createBarSource = readSource(new URL('./SessionCreateBar.tsx', import.meta.url));
    const displaySettingsSource = readSource(new URL('./SessionDisplaySettingsPopover.tsx', import.meta.url));
    const actionsSource = readSource(
      new URL('../../../../components/layout/Sider/SiderNav/ConversationSiderActions.tsx', import.meta.url)
    );

    expect(createBarSource.includes('<InstantHoverTooltip')).toBe(true);
    expect(displaySettingsSource.includes('<InstantHoverTooltip')).toBe(true);
    expect(actionsSource.includes('<InstantHoverTooltip')).toBe(true);
    expect(createBarSource.includes('<Tooltip')).toBe(false);
    expect(displaySettingsSource.includes('<Tooltip')).toBe(false);
    expect(actionsSource.includes('<Tooltip')).toBe(false);
  });

  test('does not use native title fallbacks for toolbar buttons', () => {
    const createBarSource = readSource(new URL('./SessionCreateBar.tsx', import.meta.url));
    const displaySettingsSource = readSource(new URL('./SessionDisplaySettingsPopover.tsx', import.meta.url));

    expect(createBarSource.includes("title={t('sessionList.collapseList')}")).toBe(false);
    expect(displaySettingsSource.includes("title={t('sessionList.displaySettings')}")).toBe(false);
  });
});
