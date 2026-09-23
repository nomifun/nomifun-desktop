/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CompanionId } from '@/common/types/ids';

export interface CompanionSwitchRosterItem {
  companion_id: CompanionId;
  enabled: boolean;
}

export interface CompanionSwitchWindow {
  show(): Promise<void>;
  setFocus(): Promise<void>;
}

export interface CompanionSwitchPosition {
  x: number;
  y: number;
}

interface CompanionWindowSwitchDeps<TWindow extends CompanionSwitchWindow> {
  currentId: CompanionId;
  targetId: CompanionId;
  roster: CompanionSwitchRosterItem[];
  getCurrentPosition(): Promise<CompanionSwitchPosition | null>;
  getWindow(id: CompanionId): Promise<TWindow | null>;
  enableTarget(id: CompanionId, position: CompanionSwitchPosition | null): Promise<void>;
  disableCurrent(id: CompanionId): Promise<void>;
  syncWindows(specs: CompanionSwitchRosterItem[]): Promise<void>;
  placeTarget(window: TWindow, position: CompanionSwitchPosition): Promise<void>;
  wait(ms: number): Promise<void>;
}

export type CompanionWindowSwitchResult = 'focused' | 'replaced' | 'missing';

/**
 * Bring an already-visible companion forward, or replace the current desktop
 * companion in-place when the target did not previously own a visible window.
 * The current companion is disabled only after the target has been created,
 * positioned, shown and focused, so a failed switch never leaves an empty desk.
 */
export async function switchCompanionDesktopWindow<TWindow extends CompanionSwitchWindow>(
  deps: CompanionWindowSwitchDeps<TWindow>
): Promise<CompanionWindowSwitchResult> {
  const targetProfile = deps.roster.find((item) => item.companion_id === deps.targetId);
  if (!targetProfile) return 'missing';

  const replacingCurrent = !targetProfile.enabled;
  const position = replacingCurrent ? await deps.getCurrentPosition() : null;
  let target = await deps.getWindow(deps.targetId);

  if (replacingCurrent) {
    await deps.enableTarget(deps.targetId, position);
  }

  if (replacingCurrent || !target) {
    await deps.syncWindows(
      deps.roster.map((item) => ({
        ...item,
        enabled: item.enabled || item.companion_id === deps.targetId,
      }))
    );
  }

  for (let attempt = 0; !target && attempt < 10; attempt += 1) {
    if (attempt > 0) await deps.wait(100);
    target = await deps.getWindow(deps.targetId);
  }
  if (!target) return 'missing';

  if (replacingCurrent && position) await deps.placeTarget(target, position);
  await target.show();
  await target.setFocus();

  if (replacingCurrent) await deps.disableCurrent(deps.currentId);
  return replacingCurrent ? 'replaced' : 'focused';
}
