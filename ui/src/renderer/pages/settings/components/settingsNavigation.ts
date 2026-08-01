/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Shared settings-navigation assembly: inserts extension settings tabs into an
 * ordered list of builtin nav items honoring their before/after anchors, with
 * legacy anchor remapping and an unanchored fallback (end of the Application
 * group, i.e. before "about"). Used by both `SettingsSider` (desktop rail) and
 * `SettingsPageWrapper` (mobile top nav); each caller keeps its own item shape
 * and icon construction via `toItem`.
 */

import { type IExtensionSettingsTab } from '@/common/adapter/ipcBridge';

/**
 * Legacy anchor IDs that have been merged into other tabs.
 * When an extension anchors to one of these, it is redirected to the new host.
 * This keeps older extensions working without requiring them to update.
 */
export const LEGACY_ANCHOR_REMAP: Record<string, string> = {
  agent: 'execution-engines',
  'agent-runtime': 'execution-engines',
};

export function buildSettingsNavItems<T extends { id: string }>(
  builtins: T[],
  extensionTabs: IExtensionSettingsTab[],
  toItem: (tab: IExtensionSettingsTab) => T
): {
  items: T[];
  /**
   * Number of extension tabs inserted with placement='before' per anchor id.
   * Lets callers place group headers above such tabs rather than between them
   * and their anchor builtin.
   */
  beforeCounts: Map<string, number>;
} {
  const result = [...builtins];

  const beforeMap = new Map<string, IExtensionSettingsTab[]>();
  const afterMap = new Map<string, IExtensionSettingsTab[]>();
  const unanchored: IExtensionSettingsTab[] = [];

  for (const tab of extensionTabs) {
    if (!tab.position) {
      unanchored.push(tab);
      continue;
    }
    const { relative_to: rawAnchor, placement } = tab.position;
    const anchor = LEGACY_ANCHOR_REMAP[rawAnchor] ?? rawAnchor;
    if (!result.some((item) => item.id === anchor)) {
      unanchored.push(tab);
      continue;
    }
    const map = placement === 'before' ? beforeMap : afterMap;
    let list = map.get(anchor);
    if (!list) {
      list = [];
      map.set(anchor, list);
    }
    list.push(tab);
  }

  // Insert anchored tabs (reverse iteration to preserve indices)
  for (let i = result.length - 1; i >= 0; i--) {
    const builtinId = result[i].id;
    const afters = afterMap.get(builtinId);
    if (afters) {
      result.splice(i + 1, 0, ...afters.map(toItem));
    }
    const befores = beforeMap.get(builtinId);
    if (befores) {
      result.splice(i, 0, ...befores.map(toItem));
    }
  }

  // Append unanchored at the end of the "Application" group (before "about", the
  // first "Other"-group item). Anchoring to the group's last builtin keeps these
  // tabs inside the Application group regardless of where "system" sits in the order.
  if (unanchored.length > 0) {
    const aboutIdx = result.findIndex((item) => item.id === 'about');
    const insertIdx = aboutIdx >= 0 ? aboutIdx : result.length;
    result.splice(insertIdx, 0, ...unanchored.map(toItem));
  }

  const beforeCounts = new Map<string, number>();
  for (const [anchor, tabs] of beforeMap) {
    beforeCounts.set(anchor, tabs.length);
  }

  return { items: result, beforeCounts };
}
