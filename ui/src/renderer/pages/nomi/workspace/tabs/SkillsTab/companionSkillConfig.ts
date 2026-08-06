/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ICompanionSkillConfig } from '@/common/adapter/ipcBridge';

/** Apply one catalog checkbox change to a companion's persisted Skill intent. */
export function toggleCompanionSkill(
  config: ICompanionSkillConfig,
  autoNames: ReadonlySet<string>,
  name: string,
  checked: boolean
): ICompanionSkillConfig {
  const enabled = new Set(config.enabled);
  const disabledAuto = new Set(config.disabled_auto);
  if (checked) {
    // Clearing an opt-out only restores Skills the live catalog still
    // auto-injects. A stale opt-out (the auto Skill was demoted to a regular
    // catalog entry or uninstalled) must also become an explicit opt-in —
    // the backend effective set is (auto ∪ enabled) \ disabled_auto, so
    // deleting the opt-out alone would make the click a silent no-op.
    disabledAuto.delete(name);
    if (!autoNames.has(name)) enabled.add(name);
  } else if (autoNames.has(name)) {
    disabledAuto.add(name);
  } else {
    enabled.delete(name);
  }
  return {
    enabled: [...enabled].sort(),
    disabled_auto: [...disabledAuto].sort(),
  };
}
