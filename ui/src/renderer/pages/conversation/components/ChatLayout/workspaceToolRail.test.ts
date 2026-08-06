/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const stylesheet = readFileSync(new URL('./chat-layout.css', import.meta.url), 'utf8');
const componentSource = readFileSync(new URL('./WorkspaceToolRail.tsx', import.meta.url), 'utf8');
const workspaceRailBodySource = readFileSync(new URL('../../Workspace/WorkspaceRailBody.tsx', import.meta.url), 'utf8');
const workspaceEventsSource = readFileSync(new URL('../../Workspace/hooks/useWorkspaceEvents.ts', import.meta.url), 'utf8');
// The rail's tooltip metrics live in the app-wide Arco override rather than in
// this layout's stylesheet, so the compact look is guarded there.
const arcoOverrides = readFileSync(new URL('../../../../styles/arco-override.css', import.meta.url), 'utf8');

const ruleIn = (sheet: string, selector: string) => {
  const match = sheet.match(new RegExp(`${selector}\\s*\\{([\\s\\S]*?)\\n\\}`, 'm'));
  expect(match).not.toBeNull();
  return match?.[1] ?? '';
};

const rule = (selector: string) => ruleIn(stylesheet, selector);

describe('workspace tool rail dimensions', () => {
  test('uses a text-free red dot when workspace changes are pending', () => {
    const badge = rule('\\.workspace-tool-rail__badge');

    expect(componentSource.includes("changeCount > 0 ? <span className='workspace-tool-rail__badge' /> : undefined")).toBe(true);
    expect(componentSource.includes("changeCount > 99 ? '99+' : changeCount")).toBe(false);
    expect(badge.includes('width: 7px;')).toBe(true);
    expect(badge.includes('height: 7px;')).toBe(true);
    expect(badge.includes('background: rgb(var(--danger-6));')).toBe(true);
  });

  test('refreshes the change count from the existing agent workspace refresh signal', () => {
    expect(workspaceRailBodySource.includes('refreshChanges: fileChangesHook.refreshChanges,')).toBe(true);
    expect(workspaceEventsSource.includes('refreshChangesRef.current();')).toBe(true);
  });

  test('uses compact square desktop controls', () => {
    const rail = rule('\\.workspace-tool-rail');
    const item = rule('\\.workspace-tool-rail__item');
    const collapse = rule('\\.workspace-tool-rail__item--collapse');

    expect(rail.includes('flex: 0 0 32px;')).toBe(true);
    expect(rail.includes('width: 32px;')).toBe(true);
    expect(rail.includes('min-width: 32px;')).toBe(true);
    expect(item.includes('width: 28px;')).toBe(true);
    expect(item.includes('height: 28px;')).toBe(true);
    expect(item.includes('aspect-ratio: 1 / 1;')).toBe(true);
    expect(collapse.includes('height: 28px;')).toBe(true);
  });

  test('uses the same readable icon color for the collapse control as the toolbar controls', () => {
    const item = rule('\\.workspace-tool-rail__item');
    const collapse = rule('\\.workspace-tool-rail__item--collapse');

    expect(item.includes('color: var(--text-secondary);')).toBe(true);
    expect(collapse.includes('color: var(--text-secondary);')).toBe(true);
  });

  test('sets an explicit theme-aware color for the workspace panel title', () => {
    const title = rule('\\.workspace-panel-header__title');

    expect(title.includes('color: var(--text-primary);')).toBe(true);
  });

  test('does not change the mobile workspace trigger dimensions', () => {
    const trigger = rule('\\.workspace-tool-rail-mobile-trigger');

    expect(trigger.includes('width: 24px;')).toBe(true);
    expect(trigger.includes('height: 70px;')).toBe(true);
  });

  test('keeps labels accessible but visually hidden beneath icon-only controls', () => {
    const label = rule('\\.workspace-tool-rail__label');

    expect(componentSource.includes("className='workspace-tool-rail__label'")).toBe(true);
    expect(label.includes('position: absolute;')).toBe(true);
    expect(label.includes('width: 1px;')).toBe(true);
    expect(label.includes('height: 1px;')).toBe(true);
    expect(label.includes('overflow: hidden;')).toBe(true);
  });

  test('uses a compact scoped tooltip and removes the active vertical bar', () => {
    // The rail still opts into Arco's `mini` tooltip and keeps its own scoping
    // class, so per-rail tweaks stay possible.
    expect(componentSource.includes("mini className='workspace-tool-rail__tooltip'")).toBe(true);
    // The active item is marked by colour alone; no vertical indicator bar.
    expect(stylesheet.includes('.workspace-tool-rail__item--active::before')).toBe(false);

    // The compact metrics are no longer duplicated per-rail: this layout must
    // not re-declare them, and the global contract must supply them.
    expect(stylesheet.includes('.workspace-tool-rail__tooltip .arco-tooltip-content')).toBe(false);

    const tooltipVars = ruleIn(arcoOverrides, ':root');
    expect(tooltipVars.includes('--nomi-tooltip-font-size: 12px;')).toBe(true);
    expect(tooltipVars.includes('--nomi-tooltip-line-height: 16px;')).toBe(true);
    expect(tooltipVars.includes('--nomi-tooltip-padding-block: 3px;')).toBe(true);
    expect(tooltipVars.includes('--nomi-tooltip-padding-inline: 7px;')).toBe(true);

    const tooltip = ruleIn(arcoOverrides, '\\.arco-tooltip-content');
    expect(tooltip.includes('font-size: var(--nomi-tooltip-font-size) !important;')).toBe(true);
    expect(tooltip.includes('line-height: var(--nomi-tooltip-line-height) !important;')).toBe(true);
    expect(
      tooltip.includes(
        'padding: var(--nomi-tooltip-padding-block) var(--nomi-tooltip-padding-inline) !important;'
      )
    ).toBe(true);
  });
});
