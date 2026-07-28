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
  // A removed auto-inject Skill is no longer present in the live catalog, but
  // its persisted opt-out still proves which side of the configuration it
  // belongs to and must remain reversible from the missing-state row.
  if (autoNames.has(name) || disabledAuto.has(name)) {
    if (checked) disabledAuto.delete(name);
    else disabledAuto.add(name);
  } else if (checked) {
    enabled.add(name);
  } else {
    enabled.delete(name);
  }
  return {
    enabled: [...enabled].sort(),
    disabled_auto: [...disabledAuto].sort(),
  };
}
